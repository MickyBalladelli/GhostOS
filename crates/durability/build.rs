#[path = "../../tools/c_archive.rs"]
mod c_archive;

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("output directory"));
    let source = manifest.join("../../c/src/durability.c");
    let headers = manifest.join("../../c/include");
    let object = output.join("durability.o");
    let archive = output.join("libghostos_durability.a");
    let target = env::var("TARGET").expect("target triple");
    let compiler = env::var_os("CLANG").unwrap_or_else(|| "clang".into());
    let mut compile = Command::new(compiler);
    c_archive::configure_compiler(&mut compile, &target);
    let status = compile
        .args(["-std=c11", "-O2", "-ffreestanding", "-fno-builtin", "-Wall", "-Wextra", "-Werror", "-c"])
        .arg("-I").arg(&headers).arg(&source).arg("-o").arg(&object)
        .status().expect("start durability C compiler");
    assert!(status.success(), "durability C compilation failed");
    let status = c_archive::command(&target).arg("rcs").arg(&archive).arg(&object)
        .status().expect("start durability archiver");
    assert!(status.success(), "durability archive failed");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed={}", headers.join("ghostos/durability.h").display());
    println!("cargo:rerun-if-changed={}", manifest.join("../../tools/c_archive.rs").display());
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=static=ghostos_durability");
}
