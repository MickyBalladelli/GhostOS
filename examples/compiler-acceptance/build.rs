use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo supplies OUT_DIR"))
        .join("build-script-output.rs");
    fs::write(output, "pub const BUILD_SCRIPT_RAN: bool = true;\n")
        .expect("acceptance build script output is writable");
}
