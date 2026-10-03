#[path = "../../tools/c_archive.rs"]
mod c_archive;

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("output directory"));
    let source = manifest.join("../../c/src/api_compat.c");
    let headers = manifest.join("../../c/include");
    let object = output.join("api_compat.o");
    let archive = output.join("libghostos_api_compat.a");
    let target = env::var("TARGET").expect("target triple");
    let compiler = env::var_os("CLANG").unwrap_or_else(|| "clang".into());
    let status = Command::new(compiler)
        .arg(format!("--target={target}"))
        .args(["-std=c11", "-O2", "-ffreestanding", "-fno-builtin", "-Wall", "-Wextra", "-Werror", "-c"])
        .arg("-I").arg(&headers).arg(&source).arg("-o").arg(&object)
        .status().expect("start api_compat C compiler");
    assert!(status.success(), "api_compat C compilation failed");
    let status = c_archive::command(&target).arg("rcs").arg(&archive).arg(&object)
        .status().expect("start api_compat archiver");
    assert!(status.success(), "api_compat archive failed");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", headers.join("ghostos/api_compat.h").display());
    println!("cargo:rerun-if-changed={}", manifest.join("../../tools/c_archive.rs").display());
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=static=ghostos_api_compat");
}
