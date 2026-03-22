use std::{env, fs, path::{Path, PathBuf}, process::Command};

fn probe(cmd: &str) -> bool {
    Command::new(cmd)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn copy_dll_to_bin_dir(dll: &Path, out_dir: &Path) {
    let Some(bin_dir) = out_dir.ancestors().nth(3) else {
        eprintln!("cargo:warning=Could not determine bin dir from OUT_DIR");
        return;
    };
    let dst = bin_dir.join("dinput8.dll");
    match fs::copy(dll, &dst) {
        Ok(_)  => eprintln!("cargo:warning=dinput8.dll -> {}", dst.display()),
        Err(e) => eprintln!("cargo:warning=Failed to copy dinput8.dll: {e}"),
    }
}

fn build_mingw(cc: &str, sources: &[PathBuf], def: &Path,
               include: &Path, dll_out: &Path, out_dir: &Path) {
    let mut cmd = Command::new(cc);
    cmd.args(["-std=c++17", "-shared", "-O2"])
       .arg("-o").arg(dll_out)
       .args(sources)
       .arg(def)
       .args(["-Wl,-Bstatic,-lz,-Bdynamic","-static-libgcc","-static-libstdc++","-Wl,--kill-at"])
       .arg("-I").arg(include);

    eprintln!("cargo:warning=Building dinput8.dll with {cc}");
    match cmd.status() {
        Ok(s) if s.success() => copy_dll_to_bin_dir(dll_out, out_dir),
        Ok(s) => eprintln!(
            "cargo:warning=DLL build failed (exit {:?}); check libz-mingw-w64-dev",
            s.code()
        ),
        Err(e) => eprintln!("cargo:warning=Failed to run {cc}: {e}"),
    }
}

fn build_msvc(sources: &[PathBuf], def: &Path,
              include: &Path, dll_out: &Path, out_dir: &Path) {
    let mut cmd = Command::new("cl");
    cmd.args(["/nologo", "/LD", "/MD", "/std:c++17", "/EHsc", "/O2"])
       .arg(format!("/Fe:{}", dll_out.display()))
       .arg("/I").arg(include)
       .args(sources)
       .arg("/link")
       .arg(format!("/DEF:{}", def.display()))
       .args(["zlibstatic.lib", "/NODEFAULTLIB:LIBCMT"]);

    eprintln!("cargo:warning=Building dinput8.dll with cl.exe (MSVC)");
    match cmd.status() {
        Ok(s) if s.success() => copy_dll_to_bin_dir(dll_out, out_dir),
        Ok(s) => eprintln!(
            "cargo:warning=MSVC DLL build failed (exit {:?}); ensure zlib.lib is in LIB",
            s.code()
        ),
        Err(e) => eprintln!("cargo:warning=Failed to run cl.exe: {e}"),
    }
}

fn main() {
    let host    = env::var("HOST").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let wrapper_src = manifest.join("wrapper").join("src");
    let def_file    = manifest.join("wrapper").join("dinput8.def");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", def_file.display());

    let sources: Vec<PathBuf> = match fs::read_dir(&wrapper_src) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map(|x| x == "cpp").unwrap_or(false))
            .inspect(|p| println!("cargo:rerun-if-changed={}", p.display()))
            .collect(),
        Err(e) => {
            eprintln!("cargo:warning=Cannot read {}: {e}", wrapper_src.display());
            return;
        }
    };

    if sources.is_empty() {
        eprintln!("cargo:warning=No .cpp sources found in {}, skipping DLL build",
                  wrapper_src.display());
        return;
    }

    let dll_out = out_dir.join("dinput8.dll");

    if host.contains("linux") {
        let cc = ["x86_64-w64-mingw32-g++", "i686-w64-mingw32-g++"]
            .iter().copied()
            .find(|&c| probe(c));
        match cc {
            Some(c) => build_mingw(c, &sources, &def_file, &wrapper_src, &dll_out, &out_dir),
            None    => eprintln!(
                "cargo:warning=mingw not found — install mingw-w64 to build dinput8.dll"
            ),
        }
    } else if host.contains("windows") {
        if probe("cl") {
            build_msvc(&sources, &def_file, &wrapper_src, &dll_out, &out_dir);
        } else if probe("x86_64-w64-mingw32-g++") {
            build_mingw("x86_64-w64-mingw32-g++", &sources, &def_file,
                        &wrapper_src, &dll_out, &out_dir);
        } else if probe("g++") {
            build_mingw("g++", &sources, &def_file,
                        &wrapper_src, &dll_out, &out_dir);
        } else {
            eprintln!("cargo:warning=No C++ compiler found on Windows, skipping DLL build");
        }
    } else {
        eprintln!("cargo:warning=Host {host} unsupported for DLL build, skipping");
    }
}
