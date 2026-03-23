use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

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
        Ok(_) => eprintln!("cargo:warning=dinput8.dll -> {}", dst.display()),
        Err(e) => eprintln!("cargo:warning=Failed to copy dinput8.dll: {e}"),
    }
}

fn build_cmake(
    wrapper_dir: &Path,
    out_dir: &Path,
    toolchain_file: Option<&Path>,
) -> Option<PathBuf> {
    let build_dir = out_dir.join("cmake_build");
    fs::create_dir_all(&build_dir).ok();

    let mut cfg = Command::new("cmake");
    cfg.arg(wrapper_dir)
        .arg("-B")
        .arg(&build_dir)
        .arg("-DCMAKE_BUILD_TYPE=Release");

    if let Some(tc) = toolchain_file {
        cfg.arg(format!("-DCMAKE_TOOLCHAIN_FILE={}", tc.display()));
    }

    cfg.arg("-DCMAKE_INSTALL_PREFIX=UNUSED");

    eprintln!("cargo:warning=cmake configure: {wrapper_dir:?}");
    let st = cfg.status();
    match st {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("cargo:warning=cmake configure failed (exit {:?})", s.code());
            return None;
        }
        Err(e) => {
            eprintln!("cargo:warning=cmake not found: {e}");
            return None;
        }
    }

    let nproc = std::thread::available_parallelism()
        .map(|n| n.get().to_string())
        .unwrap_or_else(|_| "4".to_string());

    let st = Command::new("cmake")
        .arg("--build")
        .arg(&build_dir)
        .arg("--config")
        .arg("Release")
        .arg("--parallel")
        .arg(&nproc)
        .status();
    match st {
        Ok(s) if s.success() => {}
        Ok(s) => {
            eprintln!("cargo:warning=cmake build failed (exit {:?})", s.code());
            return None;
        }
        Err(e) => {
            eprintln!("cargo:warning=cmake --build failed: {e}");
            return None;
        }
    }

    for sub in &["", "Release", "RelWithDebInfo"] {
        let candidate = if sub.is_empty() {
            build_dir.join("dinput8.dll")
        } else {
            build_dir.join(sub).join("dinput8.dll")
        };
        if candidate.exists() {
            return Some(candidate);
        }
    }

    eprintln!("cargo:warning=cmake build succeeded but dinput8.dll not found in {build_dir:?}");
    None
}

fn write_mingw_toolchain(out_dir: &Path, triple: &str) -> PathBuf {
    let path = out_dir.join("mingw-toolchain.cmake");
    let content = format!(
        r#"set(CMAKE_SYSTEM_NAME Windows)
set(CMAKE_SYSTEM_PROCESSOR x86_64)

set(CMAKE_C_COMPILER   {triple}-gcc)
set(CMAKE_CXX_COMPILER {triple}-g++)
set(CMAKE_RC_COMPILER  {triple}-windres)

set(CMAKE_FIND_ROOT_PATH /usr/{triple})
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
"#,
        triple = triple
    );
    fs::write(&path, content).expect("failed to write toolchain file");
    path
}

fn main() {
    let host = env::var("HOST").unwrap_or_default();
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

    let wrapper_dir = manifest.join("wrapper");

    println!("cargo:rerun-if-changed=build.rs");
    for glob_root in [wrapper_dir.join("src"), wrapper_dir.join("third_party")] {
        println!("cargo:rerun-if-changed={}", glob_root.display());
    }
    println!(
        "cargo:rerun-if-changed={}",
        wrapper_dir.join("CMakeLists.txt").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        wrapper_dir.join("dinput8.def").display()
    );

    if !probe("cmake") {
        eprintln!("cargo:warning=cmake not found — install cmake to build dinput8.dll");
        return;
    }

    let dll = if host.contains("linux") {
        let triple = ["x86_64-w64-mingw32", "i686-w64-mingw32"]
            .iter()
            .copied()
            .find(|t| probe(&format!("{t}-gcc")));

        match triple {
            Some(t) => {
                let tc = write_mingw_toolchain(&out_dir, t);
                eprintln!("cargo:warning=Cross-compiling with {t} via CMake");
                build_cmake(&wrapper_dir, &out_dir, Some(&tc))
            }
            None => {
                eprintln!(
                    "cargo:warning=No MinGW cross-compiler found. \
                     Install mingw-w64 (x86_64-w64-mingw32-gcc)."
                );
                None
            }
        }
    } else if host.contains("windows") {
        eprintln!("cargo:warning=Building dinput8.dll via CMake (native Windows)");
        build_cmake(&wrapper_dir, &out_dir, None)
    } else {
        eprintln!("cargo:warning=Host {host} not supported for DLL build");
        None
    };

    if let Some(dll_path) = dll {
        copy_dll_to_bin_dir(&dll_path, &out_dir);
    }
}
