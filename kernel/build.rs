fn main() {
    println!("cargo:rerun-if-changed=linker/x86_64.ld");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        println!("cargo:rustc-link-arg=-Tkernel/linker/x86_64.ld");
    }
}

