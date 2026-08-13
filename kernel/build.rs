use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const SERVICE_CODE_BYTES: u64 = 4 * 4096;

fn run(command: &mut Command, description: &str) {
    let status = command.status().unwrap_or_else(|error| {
        panic!("could not start {description}: {error}")
    });
    assert!(status.success(), "{description} failed with {status}")
}

fn build_user_image(source: &Path, linker: &Path, output: &Path, tools: &Path) {
    let object = output.with_extension("o");
    let elf = output.with_extension("elf");
    run(
        Command::new("clang")
            .args([
                "--target=x86_64-unknown-none-elf",
                "-std=c11",
                "-O2",
                "-ffreestanding",
                "-fno-builtin",
                "-fno-pic",
                "-fno-pie",
                "-fstack-protector-strong",
                "-mstack-protector-guard=global",
                "-fcf-protection=branch",
                "-mno-red-zone",
                "-mcmodel=large",
                "-mno-mmx",
                "-mno-sse",
                "-mno-sse2",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-c",
            ])
            .arg(source)
            .arg("-o")
            .arg(&object),
        "Ring 3 service compilation",
    );
    run(
        Command::new(tools.join("rust-lld"))
            .args(["-flavor", "gnu", "-m", "elf_x86_64", "-static", "-nostdlib", "-T"])
            .arg(linker)
            .arg(&object)
            .arg("-o")
            .arg(&elf),
        "Ring 3 service link",
    );
    run(
        Command::new(tools.join("llvm-objcopy"))
            .args(["-O", "binary"])
            .arg(&elf)
            .arg(output),
        "Ring 3 service image conversion",
    );
    let length = std::fs::metadata(output)
        .unwrap_or_else(|error| panic!("could not inspect {}: {error}", output.display()))
        .len();
    assert!(
        length != 0 && length <= SERVICE_CODE_BYTES,
        "{} is {length} bytes; maximum is {SERVICE_CODE_BYTES}",
        output.display(),
    )
}

fn rust_tools() -> PathBuf {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(rustc)
        .args(["--print", "target-libdir"])
        .output()
        .expect("could not query Rust target library directory");
    assert!(output.status.success(), "rustc --print target-libdir failed");
    let target_libdir = PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("Rust target library directory was not UTF-8")
            .trim(),
    );
    target_libdir
        .parent()
        .expect("Rust target library directory has no parent")
        .join("bin")
}

fn main() {
    println!("cargo:rerun-if-changed=linker/x86_64.ld");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("none") {
        println!("cargo:rustc-link-arg=-Tkernel/linker/x86_64.ld");
    }

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_arch != "x86_64" || !matches!(target_os.as_str(), "none" | "uefi") {
        return
    }

    let kernel = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let sources = kernel.join("../userspace/boot-services");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
    let tools = rust_tools();
    let service = output.join("synos-service.bin");
    let login = output.join("synos-login.bin");
    let shell = output.join("synos-shell.bin");
    let linker = sources.join("linker.ld");

    for source in ["service.c", "login.c", "shell.c", "linker.ld"] {
        println!("cargo:rerun-if-changed={}", sources.join(source).display());
    }
    build_user_image(&sources.join("service.c"), &linker, &service, &tools);
    build_user_image(&sources.join("login.c"), &linker, &login, &tools);
    build_user_image(&sources.join("shell.c"), &linker, &shell, &tools);
    println!("cargo:rustc-env=SYNOS_SERVICE_IMAGE={}", service.display());
    println!("cargo:rustc-env=SYNOS_LOGIN_IMAGE={}", login.display());
    println!("cargo:rustc-env=SYNOS_SHELL_IMAGE={}", shell.display());
}
