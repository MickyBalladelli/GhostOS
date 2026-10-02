use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("output directory"));
    let header = manifest.join("../c/include");
    let archive = output.join("libghostos_vm_boot.a");
    let target = env::var("TARGET").expect("target triple");
    let clang = env::var_os("CLANG").unwrap_or_else(|| "clang".into());
    let target_flag = format!("--target={target}");
    let mut objects = Vec::new();
    for module in ["vm_boot", "vm_clock", "vm_apic", "vm_hpet", "vm_pit", "vm_input", "vm_mac", "vm_packet", "vm_power", "vm_interrupt_controller", "vm_virtio_queue", "vm_serial", "vm_net", "vm_dhcp"] {
        let source = manifest.join(format!("../c/src/{module}.c"));
        let object = output.join(format!("ghostos-{module}.o"));
        let status = Command::new(&clang)
            .args([target_flag.as_str(), "-std=c11", "-O2", "-ffreestanding", "-fno-builtin", "-Wall", "-Wextra", "-Werror", "-c"])
            .arg("-I")
            .arg(&header)
            .arg(&source)
            .arg("-o")
            .arg(&object)
            .status()
            .unwrap_or_else(|error| panic!("start C compiler for {module}: {error}"));
        assert!(status.success(), "VM C module {module} compilation failed");
        println!("cargo:rerun-if-changed={}", source.display());
        println!("cargo:rerun-if-changed={}", manifest.join(format!("../c/include/ghostos/{module}.h")).display());
        objects.push(object);
    }
    let ar = env::var_os("AR").unwrap_or_else(|| "ar".into());
    let status = Command::new(ar)
        .arg("rcs")
        .arg(&archive)
        .args(&objects)
        .status()
        .expect("start archiver for VM boot C module");
    assert!(status.success(), "VM boot C module archive failed");
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=static=ghostos_vm_boot");
}
