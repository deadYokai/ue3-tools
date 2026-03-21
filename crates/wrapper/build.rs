use std::{env, fs, path::PathBuf};

fn main() {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "windows" {
        panic!(
            "dinput8_proxy must be compiled for a Windows target.\n\
             Set --target x86_64-pc-windows-gnu (or -msvc)."
        );
    }

    let target = env::var("TARGET").unwrap_or_default();
    let out    = PathBuf::from(env::var("OUT_DIR").unwrap());

    let def_content = "\
LIBRARY dinput8
EXPORTS
    DirectInput8Create @1
";
    let def_path = out.join("dinput8.def");
    fs::write(&def_path, def_content)
        .expect("failed to write dinput8.def");

    if target.contains("msvc") {
        println!(
            "cargo:rustc-cdylib-link-arg=/DEF:{}",
            def_path.display()
        );
    } else {
        println!(
            "cargo:rustc-cdylib-link-arg={}",
            def_path.display()
        );
        println!(
            "cargo:rustc-cdylib-link-arg=-Wl,--out-implib,{}/dinput8.lib",
            out.display()
        );
    }

    println!("cargo:rerun-if-changed=build.rs");
}
