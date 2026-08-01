use synos_vm::{FirmwareMode, Vm, VmConfig};

fn main() {
    println!("SynOS Virtual Machine");
    println!("=====================");

    let mut config = VmConfig::default();
    let mut efi_image: Option<Vec<u8>> = None;

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
            _ => {}
        }
        i += 1;
    }

    let mut vm = Vm::with_config(config);
    if let Some(img) = efi_image {
        vm.set_efi_application(img);
    }

    if let Err(e) = vm.run() {
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