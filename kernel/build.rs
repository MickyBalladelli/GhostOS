use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

const SERVICE_CODE_BYTES: u64 = 19 * 4096;

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
            .arg("-I")
            .arg(source.parent().expect("service source directory").join("../../c/include"))
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
            .args(["-O", "binary", "--remove-section=.stack_guard"])
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

fn build_crash_port(kernel: &Path, output: &Path, target_arch: &str) {
    let target = match target_arch {
        "x86_64" => "x86_64-unknown-none-elf",
        "aarch64" => "aarch64-unknown-none-elf",
        "riscv64" => "riscv64-unknown-none-elf",
        _ => panic!("unsupported kernel architecture for C crash port: {target_arch}"),
    };
    let clang = env::var_os("CLANG").unwrap_or_else(|| "clang".into());
    let target_flag = format!("--target={target}");
    for source in ["crash", "dlm", "capability", "contention", "dma", "driver_capabilities", "hot_allocator", "invariants", "ipc", "keyboard", "keyboard_stub", "kernel", "litmus", "main", "micro_silo", "monitor", "mouse", "mouse_stub", "page_fault"] {
        let object = output.join(format!("ghostos-{source}.o"));
        let mut compile = Command::new(&clang);
        compile.args([
            target_flag.as_str(), "-std=c11", "-O2", "-ffreestanding", "-fno-builtin",
            "-fno-pic", "-fno-pie", "-Wall", "-Wextra", "-Werror", "-c",
        ]);
        if source == "main" {
            compile.arg("-DGHOSTOS_KERNEL_IMAGE=1");
        }
        if target_arch == "x86_64" {
            compile.arg("-mno-red-zone");
        }
        let source_path = kernel.join(format!("../c/src/{source}.c"));
        run(
            compile.arg("-I").arg(kernel.join("../c/include"))
                .arg(&source_path).arg("-o").arg(&object),
            "kernel C foundation module compilation",
        );
        println!("cargo:rustc-link-arg={}", object.display());
        println!("cargo:rerun-if-changed={}", source_path.display());
    }
    for header in ["crash.h", "dlm.h", "capability.h", "contention.h", "dma.h", "driver_capabilities.h", "hot_allocator.h", "invariants.h", "ipc.h", "keyboard.h", "keyboard_stub.h", "kernel.h", "litmus.h", "main.h", "micro_silo.h", "monitor.h", "mouse.h", "mouse_stub.h", "page_fault.h"] {
        println!("cargo:rerun-if-changed={}", kernel.join(format!("../c/include/ghostos/{header}")).display());
    }
}

fn build_rust_shell_image(manifest: &Path, output: &Path, tools: &Path) {
    let target_directory = output
        .parent()
        .expect("Rust shell image has no output directory")
        .join("rust-shell-target");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let rustflags = format!(
        "-C linker={} -C code-model=kernel -C no-redzone=yes -C relocation-model=pic -C panic=abort",
        tools.join("rust-lld").display()
    );
    let mut build = Command::new(cargo);
    build
        .arg("build")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--target")
        .arg("x86_64-unknown-none")
        .arg("--release")
        .env("CARGO_TARGET_DIR", &target_directory)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env("RUSTFLAGS", rustflags);
    run(&mut build, "Rust Ring 3 shell compilation");

    let binary = target_directory
        .join("x86_64-unknown-none")
        .join("release")
        .join("ghostos-boot-shell");
    run(
        Command::new(tools.join("llvm-objcopy"))
            .args(["-O", "binary", "--remove-section=.stack_guard"])
            .arg(binary)
            .arg(output),
        "Rust Ring 3 shell image conversion",
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
    if matches!(target_os.as_str(), "none" | "uefi") {
        let kernel = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
        let output = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
        build_crash_port(&kernel, &output, &target_arch);
    }
    if target_arch != "x86_64" || !matches!(target_os.as_str(), "none" | "uefi") {
        return
    }

    let kernel = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let sources = kernel.join("../userspace/boot-services");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("build output directory"));
    let tools = rust_tools();
    let service = output.join("ghostos-service.bin");
    let login = output.join("ghostos-login.bin");
    let shell = output.join("ghostos-shell.bin");
    let linker = sources.join("linker.ld");

    for source in ["service.c", "login.c", "shell.c", "linker.ld"] {
        println!("cargo:rerun-if-changed={}", sources.join(source).display());
    }
    println!(
        "cargo:rerun-if-changed={}",
        kernel.join("../c/include/ghostos").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        sources.join("rust-shell").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        kernel.join("../crates").display()
    );
    build_user_image(&sources.join("service.c"), &linker, &service, &tools);
    build_user_image(&sources.join("login.c"), &linker, &login, &tools);
    build_rust_shell_image(
        &sources.join("rust-shell/Cargo.toml"),
        &shell,
        &tools,
    );
    let pinned = kernel.join("../build/kernel-ring3");
    std::fs::create_dir_all(&pinned).unwrap_or_else(|error| {
        panic!("could not create {}: {error}", pinned.display())
    });
    for (from, name) in [
        (&service, "ghostos-service.bin"),
        (&login, "ghostos-login.bin"),
        (&shell, "ghostos-shell.bin"),
    ] {
        let to = pinned.join(name);
        std::fs::copy(from, &to).unwrap_or_else(|error| {
            panic!("could not pin {}: {error}", to.display())
        });
    }
    println!("cargo:rustc-env=GHOSTOS_SERVICE_IMAGE={}", service.display());
    println!("cargo:rustc-env=GHOSTOS_LOGIN_IMAGE={}", login.display());
    println!("cargo:rustc-env=GHOSTOS_SHELL_IMAGE={}", shell.display());
}
