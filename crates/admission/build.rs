use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("output directory"));
    let source = manifest.join("../../c/src/admission.c");
    let headers = manifest.join("../../c/include");
    let object = output.join("admission.o");
    let archive = output.join("libghostos_admission.a");
    let target = env::var("TARGET").expect("target triple");
    let compiler = env::var_os("CLANG").unwrap_or_else(|| "clang".into());
    let status = Command::new(compiler)
        .arg(format!("--target={target}"))
        .args(["-std=c11", "-O2", "-ffreestanding", "-fno-builtin", "-Wall", "-Wextra", "-Werror", "-c"])
        .arg("-I").arg(&headers).arg(&source).arg("-o").arg(&object)
        .status().expect("start admission C compiler");
    assert!(status.success(), "admission C compilation failed");
    let archiver = env::var_os("AR").unwrap_or_else(|| "ar".into());
    let status = Command::new(archiver).arg("rcs").arg(&archive).arg(&object)
        .status().expect("start admission archiver");
    assert!(status.success(), "admission archive failed");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", headers.join("ghostos/admission.h").display());
    println!("cargo:rerun-if-changed={}", headers.join("ghostos/memory.h").display());
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=static=ghostos_admission");
}
