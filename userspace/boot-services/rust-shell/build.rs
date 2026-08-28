use std::path::PathBuf;

fn main() {
    let linker = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("..")
        .join("linker.ld")
        .canonicalize()
        .unwrap();
    println!("cargo:rerun-if-changed={}", linker.display());
    println!("cargo:rustc-link-arg=-T{}", linker.display());
}
