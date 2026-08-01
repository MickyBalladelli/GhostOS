use std::path::PathBuf;
use synos_vm::{run_synos_integration, FirmwareMode, Vm, VmConfig};

fn main() {
    println!("SynOS Virtual Machine");
    println!("=====================");

    let mut config = VmConfig::default();
    let mut efi_image: Option<Vec<u8>> = None;
    let mut integration = false;

    let args: Vec<String> = std::env::args().collect();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--firmware" | "-f" => {
                if let Some(mode) = args.get(i + 1) {
                    config.firmware = match mode.as_str() {
                        "uefi" | "UEFI" => FirmwareMode::Uefi,
                        _ => FirmwareMode::Bios,
                    };
                    i += 1;
                }
            }
            "--efi" => {
                if let Some(path) = args.get(i + 1) {
                    match std::fs::read(path) {
                        Ok(img) => efi_image = Some(img),
                        Err(e) => {
                            eprintln!("Failed to read EFI application {}: {}", path, e);
                            std::process::exit(1);
                        }
                    }
                    i += 1;
                }
            }
            "--memory" => {
                if let Some(size) = args.get(i + 1) {
                    config.memory_size = parse_memory(size).unwrap_or(config.memory_size);
                    i += 1;
                }
            }
            "--kernel" => {
                if let Some(path) = args.get(i + 1) {
                    config.kernel_path = Some(PathBuf::from(path));
                    i += 1;
                }
            }
            "--initrd" => {
                if let Some(path) = args.get(i + 1) {
                    config.initrd_path = Some(PathBuf::from(path));
                    i += 1;
                }
            }
            "--append" => {
                if let Some(args) = args.get(i + 1) {
                    config.boot_args = args.clone();
                    i += 1;
                }
            }
            "--steps" => {
                if let Some(steps) = args.get(i + 1) {
                    config.max_steps = steps.parse().ok();
                    i += 1;
                }
            }
            "--integration" => integration = true,
            _ => {}
        }
        i += 1;
    }

    if integration {
        let steps = config.max_steps.unwrap_or(10_000_000);
        match run_synos_integration(config, steps) {
            Ok(report) => {
                println!(
                    "SynOS integration: boot={} paging={} scheduler={} capabilities={} ipc={} ({} steps)",
                    report.kernel_booted,
                    report.paging_ready,
                    report.scheduler_ready,
                    report.capabilities_ready,
                    report.ipc_ready,
                    report.vm.steps,
                );
                if !report.kernel_booted
                    || !report.paging_ready
                    || !report.scheduler_ready
                    || !report.capabilities_ready
                    || !report.ipc_ready
                {
                    std::process::exit(1);
                }
                return;
            }
            Err(error) => {
                eprintln!("SynOS integration error: {:?}", error);
                std::process::exit(1);
            }
        }
    }

    let mut vm = Vm::with_config(config);
    if let Some(img) = efi_image {
        vm.set_efi_application(img);
    }

    let result = if let Some(steps) = vm.config().max_steps {
        match vm.run_for_steps(steps) {
            Ok(report) => {
                println!(
                    "VM stopped after {} steps at RIP 0x{:016x} (halted={})",
                    report.steps, report.rip, report.halted
                );
                Ok(())
            }
            Err(error) => Err(error),
        }
    } else {
        vm.run()
    };
    if let Err(e) = result {
        eprintln!("VM Error: {:?}", e);
    }
}

fn parse_memory(s: &str) -> Option<usize> {
    let s = s.trim().to_ascii_lowercase();
    let (num, mult) = if let Some(v) = s.strip_suffix("g") {
        (v, 1024 * 1024 * 1024usize)
    } else if let Some(v) = s.strip_suffix("m") {
        (v, 1024 * 1024usize)
    } else if let Some(v) = s.strip_suffix("k") {
        (v, 1024usize)
    } else {
        (s.as_str(), 1)
    };
    num.parse::<usize>().ok().map(|n| n * mult)
}
