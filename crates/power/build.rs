#[path = "../../tools/c_archive.rs"]
mod c_archive;

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("output directory"));
    let headers = manifest.join("../../c/include");
    let archive = output.join("libghostos_thermal.a");
    let target = env::var("TARGET").expect("target triple");
    let compiler = env::var_os("CLANG").unwrap_or_else(|| "clang".into());
    let mut objects = Vec::new();
    for name in ["thermal", "power_policy"] {
        let source = manifest.join(format!("../../c/src/{name}.c"));
        let object = output.join(format!("{name}.o"));
        let mut compile = Command::new(&compiler);
        c_archive::configure_compiler(&mut compile, &target);
        let status = compile
            .args(["-std=c11", "-O2", "-ffreestanding", "-fno-builtin", "-Wall", "-Wextra", "-Werror", "-c"])
            .arg("-I").arg(&headers).arg(&source).arg("-o").arg(&object)
            .status().expect("start power C compiler");
        assert!(status.success(), "power C compilation failed");
        objects.push(object);
        println!("cargo:rerun-if-changed={}", source.display());
        println!("cargo:rerun-if-changed={}", headers.join(format!("ghostos/{name}.h")).display());
    }
    let status = c_archive::command(&target).arg("rcs").arg(&archive).args(&objects)
        .status().expect("start power archiver");
    assert!(status.success(), "power archive failed");
    println!("cargo:rerun-if-changed={}", manifest.join("../../tools/c_archive.rs").display());
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=static=ghostos_thermal");
}
