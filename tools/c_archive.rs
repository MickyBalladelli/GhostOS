//! Archive and compiler target selection for the temporary Rust adapters to freestanding C ports.

use std::env;
use std::path::PathBuf;
use std::process::Command;

pub fn command(target: &str) -> Command {
    println!("cargo:rerun-if-env-changed=AR");
    if let Some(archiver) = env::var_os("AR") {
        return Command::new(archiver)
    }
    if target.contains("apple") {
        return Command::new("ar")
    }
    // The host's BSD ar can make ELF archives whose symbol index rustc cannot
    // bundle. Use the LLVM tools installed alongside rustc, with an explicit
    // format so rebuilding an existing BSD archive also repairs its index.
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(rustc).args(["--print", "target-libdir"])
        .output().expect("query host Rust tool directory for C archiver");
    assert!(output.status.success(), "host Rust tool directory query failed");
    let library = PathBuf::from(String::from_utf8(output.stdout)
        .expect("Rust tool directory is UTF-8").trim());
    let executable = if cfg!(windows) { "llvm-ar.exe" } else { "llvm-ar" };
    let bundled = library.parent().expect("Rust library directory has a parent")
        .join("bin").join(executable);
    let mut command = if bundled.is_file() {
        Command::new(bundled)
    } else {
        Command::new(executable)
    };
    command.arg(if target.ends_with("-msvc") { "--format=coff" } else { "--format=gnu" });
    command
}

pub fn configure_compiler(command: &mut Command, target: &str) {
    println!("cargo:rerun-if-env-changed=CLANG");
    if target == "riscv64gc-unknown-none-elf" {
        // Rust's target specification uses RV64IMAFDC and the lp64d ABI. Clang
        // takes the base architecture in its triple and the ISA in -march.
        command.args(["--target=riscv64-unknown-none-elf", "-march=rv64gc", "-mabi=lp64d"]);
    } else {
        command.arg(format!("--target={target}"));
    }
}
