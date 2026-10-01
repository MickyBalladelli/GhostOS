use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("output directory"));
    let source = manifest.join("../c/src/vm_boot.c");
    let header = manifest.join("../c/include");
    let object = output.join("ghostos-vm-boot.o");
    let archive = output.join("libghostos_vm_boot.a");
    let target = env::var("TARGET").expect("target triple");
    let clang = env::var_os("CLANG").unwrap_or_else(|| "clang".into());
    let target_flag = format!("--target={target}");
    let status = Command::new(clang)
        .args([target_flag.as_str(), "-std=c11", "-O2", "-ffreestanding", "-fno-builtin", "-Wall", "-Wextra", "-Werror", "-c"])
        .arg("-I")
        .arg(header)
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .status()
        .expect("start C compiler for VM boot module");
    assert!(status.success(), "VM boot C module compilation failed");
    let ar = env::var_os("AR").unwrap_or_else(|| "ar".into());
    let status = Command::new(ar)
        .arg("rcs")
        .arg(&archive)
        .arg(&object)
        .status()
        .expect("start archiver for VM boot C module");
    assert!(status.success(), "VM boot C module archive failed");
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=static=ghostos_vm_boot");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", manifest.join("../c/include/ghostos/vm_boot.h").display());
}
