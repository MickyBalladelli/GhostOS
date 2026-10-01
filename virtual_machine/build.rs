use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("output directory"));
    let source = manifest.join("../c/src/vm_boot.c");
    let header = manifest.join("../c/include");
    let object = output.join("ghostos-vm-boot.o");
    let target = env::var("TARGET").expect("target triple");
    let clang = env::var_os("CLANG").unwrap_or_else(|| "clang".into());
    let status = Command::new(clang)
        .args(["--target", &target, "-std=c11", "-O2", "-ffreestanding", "-fno-builtin", "-Wall", "-Wextra", "-Werror", "-c"])
        .arg("-I")
        .arg(header)
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .status()
        .expect("start C compiler for VM boot module");
    assert!(status.success(), "VM boot C module compilation failed");
    println!("cargo:rustc-link-arg={}", object.display());
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", manifest.join("../c/include/ghostos/vm_boot.h").display());
}
