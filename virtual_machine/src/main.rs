use std::path::PathBuf;

use synos_vm::{run_synos_integration, FirmwareMode, Vm, VmConfig};

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Cli {
    config: VmConfig,
    efi_path: Option<PathBuf>,
    integration: bool,
}

enum ParseResult {
    Run(Cli),
    Help,
    Version,
}

fn main() {
    match parse_args(std::env::args().skip(1)) {
        Ok(ParseResult::Help) => {
            print_help();
        }
        Ok(ParseResult::Version) => {
            println!("synos-vm {VERSION}");
        }
        Ok(ParseResult::Run(cli)) => {
            if let Err(error) = run(cli) {
                eprintln!("synos-vm: {error}");
                std::process::exit(1);
            }
        }
        Err(error) => {
            eprintln!("synos-vm: {error}");
            eprintln!("Try `synos-vm --help` for usage.");
            std::process::exit(2);
        }
    }
}

fn parse_args<I>(args: I) -> Result<ParseResult, String>
where
    I: IntoIterator<Item = String>,
{
    let mut config = VmConfig::default();
    let mut efi_path = None;
    let mut integration = false;
    let mut args = args.into_iter().peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(ParseResult::Help),
            "-V" | "--version" => return Ok(ParseResult::Version),
            "-f" | "--firmware" => {
                let value = next_value(&mut args, "--firmware")?;
                config.firmware = parse_firmware(&value)?;
            }
            "--efi" => {
                efi_path = Some(PathBuf::from(next_value(&mut args, "--efi")?));
            }
            "-m" | "--memory" => {
                let value = next_value(&mut args, "--memory")?;
                config.memory_size =
                    parse_memory(&value).ok_or_else(|| format!("invalid memory size `{value}`"))?;
            }
            "-k" | "--kernel" => {
                config.kernel_path = Some(PathBuf::from(next_value(&mut args, "--kernel")?));
            }
            "-i" | "--initrd" => {
                config.initrd_path = Some(PathBuf::from(next_value(&mut args, "--initrd")?));
            }
            "-a" | "--append" => {
                config.boot_args = next_value(&mut args, "--append")?;
            }
            "-c" | "--cpus" => {
                let value = next_value(&mut args, "--cpus")?;
                config.smp_cores = parse_positive_usize(&value, "CPU count")?;
            }
            "--steps" => {
                let value = next_value(&mut args, "--steps")?;
                config.max_steps = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| format!("invalid step count `{value}`"))?,
                );
            }
            "--serial" => config.enable_serial = true,
            "--no-serial" => config.enable_serial = false,
            "--integration" => integration = true,
            value if value.starts_with('-') => {
                return Err(format!("unknown option `{value}`"));
            }
            value => return Err(format!("unexpected argument `{value}`")),
        }
    }

    if efi_path.is_some() && config.firmware != FirmwareMode::Uefi {
        return Err("--efi requires --firmware uefi".to_string());
    }
    if integration && config.kernel_path.is_none() {
        return Err("--integration requires --kernel <path>".to_string());
    }

    Ok(ParseResult::Run(Cli {
        config,
        efi_path,
        integration,
    }))
}

fn next_value<I>(args: &mut std::iter::Peekable<I>, option: &str) -> Result<String, String>
where
    I: Iterator<Item = String>,
{
    let value = args
        .next()
        .ok_or_else(|| format!("{option} needs a value"))?;
    if value.starts_with('-') {
        return Err(format!("{option} needs a value"));
    }
    Ok(value)
}

fn parse_firmware(value: &str) -> Result<FirmwareMode, String> {
    match value.to_ascii_lowercase().as_str() {
        "bios" => Ok(FirmwareMode::Bios),
        "uefi" => Ok(FirmwareMode::Uefi),
        _ => Err(format!("invalid firmware `{value}`; use `bios` or `uefi`")),
    }
}

fn parse_positive_usize(value: &str, name: &str) -> Result<usize, String> {
    let parsed = value
        .parse::<usize>()
        .map_err(|_| format!("invalid {name} `{value}`"))?;
    if parsed == 0 {
        return Err(format!("{name} must be greater than zero"));
    }
    Ok(parsed)
}

fn parse_memory(value: &str) -> Option<usize> {
    let value = value.trim().to_ascii_lowercase();
    let suffixes = [
        ("gib", 1024usize * 1024 * 1024),
        ("gb", 1024usize * 1024 * 1024),
        ("g", 1024usize * 1024 * 1024),
        ("mib", 1024usize * 1024),
        ("mb", 1024usize * 1024),
        ("m", 1024usize * 1024),
        ("kib", 1024usize),
        ("kb", 1024usize),
        ("k", 1024usize),
        ("b", 1usize),
    ];
    let (number, multiplier) = suffixes
        .iter()
        .find_map(|(suffix, multiplier)| {
            value
                .strip_suffix(suffix)
                .map(|number| (number, *multiplier))
        })
        .unwrap_or((value.as_str(), 1));

    number
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|number| number.checked_mul(multiplier))
        .filter(|size| *size > 0)
}

fn run(cli: Cli) -> Result<(), String> {
    println!("SynOS Virtual Machine");
    println!("=====================");

    let config = cli.config;
    let efi_image = cli
        .efi_path
        .map(|path| {
            std::fs::read(&path)
                .map_err(|error| format!("cannot read EFI image {}: {error}", path.display()))
        })
        .transpose()?;

    if cli.integration {
        let steps = config.max_steps.unwrap_or(10_000_000);
        let report = run_synos_integration(config, steps)
            .map_err(|error| format!("integration failed: {error:?}"))?;
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
            return Err("integration checks failed".to_string());
        }
        return Ok(());
    }

    let mut vm = Vm::with_config(config);
    if let Some(image) = efi_image {
        vm.set_efi_application(image);
    }

    let result = if let Some(steps) = vm.config().max_steps {
        let report = vm
            .run_for_steps(steps)
            .map_err(|error| format!("VM error: {error:?}"))?;
        println!(
            "VM stopped after {} steps at RIP 0x{:016x} (halted={})",
            report.steps, report.rip, report.halted
        );
        Ok(())
    } else {
        vm.run().map_err(|error| format!("VM error: {error:?}"))
    };

    result
}

fn print_help() {
    println!(
        "{}",
        r#"SynOS virtual machine

Usage:
  synos-vm [OPTIONS]

Boot options:
  -k, --kernel <PATH>       Load a SynOS kernel image
  -i, --initrd <PATH>       Load an initrd image
  -a, --append <ARGS>       Pass kernel command-line arguments
  -f, --firmware <MODE>     Firmware: bios (default) or uefi
      --efi <PATH>          Load a UEFI application

Machine options:
  -m, --memory <SIZE>       Guest RAM, such as 128M, 1GiB, or 4096K
  -c, --cpus <COUNT>        Number of guest CPUs
      --serial              Enable COM1 serial output (default)
      --no-serial            Disable COM1 serial output
      --steps <COUNT>       Run a bounded number of instructions

Commands:
      --integration          Run SynOS integration checks

Other options:
  -h, --help                Show this help
  -V, --version             Show the version"#
    );
}
