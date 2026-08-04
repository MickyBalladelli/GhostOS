use std::io::{IsTerminal, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::net::UnixListener;

use synos_vm::{
    run_synos_integration, DiskController, DiskFormat, DiskImage, DiskManager, DiskPersistence,
    DiskRole,
    DiskSpec, FirmwareMode, SystemDiskCreateOptions, SystemDiskInstall, SystemDiskProvisioner,
    TerminalExit, TerminalInputMode, TerminalSession, Vm, VmConfig, COM1_PORT, COM2_PORT,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Cli {
    command: Command,
    config: VmConfig,
    memory_explicit: bool,
    efi_path: Option<PathBuf>,
    integration: bool,
    terminal: Option<bool>,
    input_mode: TerminalInputMode,
    disk_options: DiskOptions,
    snapshot_save: Option<PathBuf>,
    snapshot_restore: Option<PathBuf>,
    monitor_path: Option<PathBuf>,
}

enum Command {
    Run,
    ListDisks,
}

enum MigrateCommand {
    Send { snapshot: PathBuf, address: String },
    Receive { address: String, snapshot: PathBuf },
}

enum DiskCommand {
    List(DiskOptions),
    Inspect(PathBuf),
    Validate(PathBuf),
    Provision {
        path: PathBuf,
        kernel: PathBuf,
        initrd: Option<PathBuf>,
        size: Option<u64>,
        format: DiskFormat,
        replace: bool,
        boot_args: String,
        machine_identity: String,
        network_identity: String,
    },
    Lock(PathBuf),
    RecoverLock(PathBuf),
}

struct DiskOptions {
    disks: Vec<PathBuf>,
    system_disk: Option<PathBuf>,
    controller: DiskController,
    format: Option<DiskFormat>,
    size: Option<u64>,
    read_only: bool,
    persistence: DiskPersistence,
    create_if_missing: bool,
}

impl Default for DiskOptions {
    fn default() -> Self {
        Self {
            disks: Vec::new(),
            system_disk: None,
            controller: DiskController::VirtioBlk,
            format: None,
            size: None,
            read_only: false,
            persistence: DiskPersistence::Persistent,
            create_if_missing: false,
        }
    }
}

enum ParseResult {
    Run(Cli),
    Help,
    Version,
    Disk(DiskCommand),
    Migrate(MigrateCommand),
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
        Ok(ParseResult::Disk(command)) => {
            if let Err(error) = run_disk_command(command) {
                eprintln!("synos-vm: {error}");
                std::process::exit(1);
            }
        }
        Ok(ParseResult::Migrate(command)) => {
            if let Err(error) = run_migrate_command(command) {
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
    let values: Vec<String> = args.into_iter().collect();
    if values.first().map(String::as_str) == Some("disk") {
        return parse_disk_command(&values[1..]);
    }
    if values.first().map(String::as_str) == Some("migrate") {
        return parse_migrate_command(&values[1..]);
    }

    let mut config = VmConfig::default();
    let mut memory_explicit = false;
    let mut efi_path = None;
    let mut integration = false;
    let mut terminal = None;
    let mut input_mode = TerminalInputMode::Serial;
    let mut command = Command::Run;
    let mut disk_options = DiskOptions::default();
    let mut snapshot_save = None;
    let mut snapshot_restore = None;
    let mut monitor_path = None;
    let mut args = values.into_iter().peekable();

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
                memory_explicit = true;
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
            "--serial-port" => {
                let value = next_value(&mut args, "--serial-port")?;
                config.serial_port = parse_serial_port(&value)?;
            }
            "--interactive" | "--terminal" => terminal = Some(true),
            "--non-interactive" | "--no-terminal" => terminal = Some(false),
            "--input" => {
                input_mode = match next_value(&mut args, "--input")?.to_ascii_lowercase().as_str() {
                    "serial" => TerminalInputMode::Serial,
                    "ps2" | "keyboard" => TerminalInputMode::Ps2,
                    value => return Err(format!("invalid input mode `{value}`; use `serial` or `ps2`")),
                };
            }
            "--integration" => integration = true,
            "--list-disks" => command = Command::ListDisks,
            "--disk" => disk_options.disks.push(PathBuf::from(next_value(&mut args, "--disk")?)),
            "--system-disk" => {
                if disk_options.system_disk.is_some() {
                    return Err("--system-disk may only be supplied once".to_string());
                }
                disk_options.system_disk = Some(PathBuf::from(next_value(&mut args, "--system-disk")?));
            }
            "--disk-controller" => {
                disk_options.controller = parse_disk_controller(&next_value(&mut args, "--disk-controller")?)?;
            }
            "--disk-format" => {
                disk_options.format = Some(parse_disk_format(&next_value(&mut args, "--disk-format")?)?);
            }
            "--disk-size" => {
                let value = next_value(&mut args, "--disk-size")?;
                disk_options.size = Some(parse_size_bytes(&value, "disk size")?);
            }
            "--read-only" => disk_options.read_only = true,
            "--read-write" => disk_options.read_only = false,
            "--copy-on-write" => disk_options.persistence = DiskPersistence::CopyOnWrite,
            "--disposable" => disk_options.persistence = DiskPersistence::Disposable,
            "--create-if-missing" => disk_options.create_if_missing = true,
            "--snapshot-save" => {
                snapshot_save = Some(PathBuf::from(next_value(&mut args, "--snapshot-save")?));
            }
            "--snapshot-restore" => {
                snapshot_restore =
                    Some(PathBuf::from(next_value(&mut args, "--snapshot-restore")?));
            }
            "--monitor" => {
                monitor_path = Some(PathBuf::from(next_value(&mut args, "--monitor")?));
            }
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
    if terminal == Some(true)
        && input_mode == TerminalInputMode::Serial
        && !config.enable_serial
    {
        return Err("--interactive requires serial output".to_string());
    }
    if terminal == Some(true) && config.max_steps.is_some() {
        return Err("--interactive cannot be combined with --steps".to_string());
    }
    if integration && (snapshot_save.is_some() || snapshot_restore.is_some()) {
        return Err("snapshot options cannot be combined with --integration".to_string());
    }
    if terminal == Some(true) && monitor_path.is_some() {
        return Err("--monitor cannot be combined with --interactive".to_string());
    }

    Ok(ParseResult::Run(Cli {
        command,
        config,
        memory_explicit,
        efi_path,
        integration,
        terminal,
        input_mode,
        disk_options,
        snapshot_save,
        snapshot_restore,
        monitor_path,
    }))
}

fn parse_migrate_command(values: &[String]) -> Result<ParseResult, String> {
    match values {
        [command, snapshot, address] if command == "send" => {
            Ok(ParseResult::Migrate(MigrateCommand::Send {
                snapshot: PathBuf::from(snapshot),
                address: address.clone(),
            }))
        }
        [command, address, snapshot] if command == "receive" => {
            Ok(ParseResult::Migrate(MigrateCommand::Receive {
                address: address.clone(),
                snapshot: PathBuf::from(snapshot),
            }))
        }
        [command] if command == "help" || command == "--help" || command == "-h" => {
            Ok(ParseResult::Help)
        }
        [] => Err("migrate needs `send SNAPSHOT ADDRESS` or `receive ADDRESS SNAPSHOT`".to_string()),
        _ => Err("usage: synos-vm migrate send SNAPSHOT ADDRESS | receive ADDRESS SNAPSHOT".to_string()),
    }
}

fn parse_disk_command(values: &[String]) -> Result<ParseResult, String> {
    let subcommand = values
        .first()
        .map(String::as_str)
        .ok_or_else(|| "disk needs a command: list, inspect, validate, provision, lock, or recover-lock".to_string())?;
    match subcommand {
        "list" => Ok(ParseResult::Disk(DiskCommand::List(parse_disk_options(&values[1..])?))),
        "inspect" => Ok(ParseResult::Disk(DiskCommand::Inspect(command_path(values, "inspect")?))),
        "validate" => Ok(ParseResult::Disk(DiskCommand::Validate(command_path(values, "validate")?))),
        "lock" => Ok(ParseResult::Disk(DiskCommand::Lock(command_path(values, "lock")?))),
        "recover-lock" => Ok(ParseResult::Disk(DiskCommand::RecoverLock(command_path(values, "recover-lock")?))),
        "provision" => parse_provision_command(&values[1..]),
        "help" | "--help" | "-h" => Ok(ParseResult::Help),
        value => Err(format!("unknown disk command `{value}`")),
    }
}

fn command_path(values: &[String], command: &str) -> Result<PathBuf, String> {
    if values.len() != 2 {
        return Err(format!("disk {command} needs exactly one PATH"));
    }
    Ok(PathBuf::from(&values[1]))
}

fn parse_disk_options(values: &[String]) -> Result<DiskOptions, String> {
    let mut options = DiskOptions::default();
    let mut args = values.iter().peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--disk" => options.disks.push(PathBuf::from(next_ref(&mut args, "--disk")?)),
            "--system-disk" => {
                if options.system_disk.is_some() {
                    return Err("--system-disk may only be supplied once".to_string());
                }
                options.system_disk = Some(PathBuf::from(next_ref(&mut args, "--system-disk")?));
            }
            "--disk-controller" => {
                options.controller = parse_disk_controller(next_ref(&mut args, "--disk-controller")?)?;
            }
            "--disk-format" => {
                options.format = Some(parse_disk_format(next_ref(&mut args, "--disk-format")?)?);
            }
            "--disk-size" => {
                let value = next_ref(&mut args, "--disk-size")?;
                options.size = Some(parse_size_bytes(value, "disk size")?);
            }
            "--read-only" => options.read_only = true,
            "--read-write" => options.read_only = false,
            "--copy-on-write" => options.persistence = DiskPersistence::CopyOnWrite,
            "--disposable" => options.persistence = DiskPersistence::Disposable,
            "--create-if-missing" => options.create_if_missing = true,
            "--help" | "-h" => return Ok(options),
            value => return Err(format!("unknown disk list option `{value}`")),
        }
    }
    Ok(options)
}

fn parse_provision_command(values: &[String]) -> Result<ParseResult, String> {
    let mut args = values.iter().peekable();
    let path = PathBuf::from(next_ref(&mut args, "disk provision PATH")?);
    let mut kernel = None;
    let mut initrd = None;
    let mut size = None;
    let mut format = DiskFormat::Raw;
    let mut replace = false;
    let mut boot_args = String::new();
    let mut machine_identity = String::new();
    let mut network_identity = String::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--kernel" => kernel = Some(PathBuf::from(next_ref(&mut args, "--kernel")?)),
            "--initrd" => initrd = Some(PathBuf::from(next_ref(&mut args, "--initrd")?)),
            "--size" | "--disk-size" => {
                size = Some(parse_size_bytes(next_ref(&mut args, "--size")?, "disk size")?);
            }
            "--format" | "--disk-format" => {
                format = parse_disk_format(next_ref(&mut args, "--format")?)?;
            }
            "--replace" => replace = true,
            "--boot-args" | "--append" => boot_args = next_ref(&mut args, "--boot-args")?.to_string(),
            "--machine-id" => machine_identity = next_ref(&mut args, "--machine-id")?.to_string(),
            "--network-id" => network_identity = next_ref(&mut args, "--network-id")?.to_string(),
            value => return Err(format!("unknown disk provision option `{value}`")),
        }
    }
    let kernel = kernel.ok_or_else(|| "disk provision needs --kernel PATH".to_string())?;
    Ok(ParseResult::Disk(DiskCommand::Provision {
        path,
        kernel,
        initrd,
        size,
        format,
        replace,
        boot_args,
        machine_identity,
        network_identity,
    }))
}

fn next_ref<'a, I>(args: &mut std::iter::Peekable<I>, option: &str) -> Result<&'a str, String>
where
    I: Iterator<Item = &'a String>,
{
    let value = args.next().ok_or_else(|| format!("{option} needs a value"))?;
    if value.starts_with('-') {
        return Err(format!("{option} needs a value"));
    }
    Ok(value)
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

fn parse_size_bytes(value: &str, name: &str) -> Result<u64, String> {
    parse_memory(value)
        .map(|size| size as u64)
        .ok_or_else(|| format!("invalid {name} `{value}`"))
}

fn parse_disk_controller(value: &str) -> Result<DiskController, String> {
    match value.to_ascii_lowercase().as_str() {
        "ahci" | "sata" => Ok(DiskController::Ahci),
        "nvme" => Ok(DiskController::Nvme),
        "virtio" | "virtio-blk" => Ok(DiskController::VirtioBlk),
        _ => Err(format!(
            "invalid disk controller `{value}`; use ahci, nvme, or virtio-blk"
        )),
    }
}

fn parse_disk_format(value: &str) -> Result<DiskFormat, String> {
    match value.to_ascii_lowercase().as_str() {
        "raw" | "img" => Ok(DiskFormat::Raw),
        "vhd" => Ok(DiskFormat::Vhd),
        "qcow2" | "qcow" => Ok(DiskFormat::Qcow2),
        _ => Err(format!(
            "invalid disk format `{value}`; use raw, vhd, or qcow2"
        )),
    }
}

fn parse_serial_port(value: &str) -> Result<u16, String> {
    match value.to_ascii_lowercase().as_str() {
        "com1" => Ok(COM1_PORT),
        "com2" => Ok(COM2_PORT),
        value => {
            let value = value.strip_prefix("0x").unwrap_or(value);
            u16::from_str_radix(value, 16)
                .map_err(|_| format!("invalid serial port `{value}`"))
        }
    }
}

fn run(mut cli: Cli) -> Result<(), String> {
    let list_disks = matches!(&cli.command, Command::ListDisks);
    cli.config.disks = prepare_disk_specs(&cli.disk_options)?;
    if list_disks {
        return print_disk_inventory(&cli.config.disks);
    }

    let restore_snapshot = cli
        .snapshot_restore
        .as_ref()
        .map(Vm::load_snapshot)
        .transpose()
        .map_err(|error| format!("cannot load snapshot: {error}"))?;
    let mut config = cli.config;
    if let Some(snapshot) = restore_snapshot.as_ref() {
        if !cli.memory_explicit {
            config.memory_size = snapshot.memory_size;
        }
        if config.memory_size != snapshot.memory_size {
            return Err(format!(
                "snapshot needs {} bytes of RAM; pass matching --memory or omit it",
                snapshot.memory_size
            ));
        }
    }
    if std::io::stdout().is_terminal() {
        print!(
            "\x1b]0;SynOS | {}\x07",
            format_memory(config.memory_size)
        );
    }
    let terminal_mode = cli.terminal;
    let input_mode = cli.input_mode;
    let snapshot_save = cli.snapshot_save;
    let monitor_path = cli.monitor_path;
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

    let mut vm = Vm::try_with_config(config)
        .map_err(|error| format!("VM configuration error: {error:?}"))?;
    if let Some(image) = efi_image {
        vm.set_efi_application(image);
    }
    if let Some(snapshot) = restore_snapshot.as_ref() {
        vm.restore_snapshot(snapshot)
            .map_err(|error| format!("cannot restore snapshot: {error}"))?;
        println!("Restored VM snapshot (checksum=0x{:016x})", snapshot.checksum());
    }

    let terminal_mode = if monitor_path.is_some() && terminal_mode.is_none() {
        Some(false)
    } else {
        terminal_mode
    };
    let mut monitor = monitor_path
        .map(|path| MonitorSession::bind(path))
        .transpose()?;
    let mut poll_monitor = |vm: &mut Vm| {
        monitor
            .as_mut()
            .map(|session| session.poll(vm))
            .transpose()
            .map(|value| value.unwrap_or(true))
            .map_err(|_| synos_vm::VmError::IoError)
    };

    if let Some(steps) = vm.config().max_steps {
        let report = vm
            .run_for_steps_with_monitor(steps, &mut poll_monitor)
            .map_err(|error| format!("VM error: {error:?}"))?;
        println!(
            "VM stopped after {} steps at RIP 0x{:016x} (halted={})",
            report.steps, report.rip, report.halted
        );
    } else {
        println!("Starting CPU emulation...");
        let terminal = TerminalSession::new(terminal_mode)
            .map_err(|error| format!("terminal error: {error}"))?;
        let exit = vm
            .run_with_terminal_mode_and_monitor(&terminal, input_mode, &mut poll_monitor)
            .map_err(|error| format!("VM error: {error:?}"))?;
        drop(terminal);
        match exit {
            TerminalExit::GuestShutdown => println!("\nGuest powered off"),
        }
    }

    if let Some(path) = snapshot_save {
        vm.save_snapshot(&path)
            .map_err(|error| format!("cannot save snapshot {}: {error}", path.display()))?;
        println!("Saved VM snapshot to {}", path.display());
    }

    Ok(())
}

const MIGRATION_MAGIC: &[u8; 8] = b"SYNOMIG1";
const MAX_MIGRATION_BYTES: u64 = 64 * 1024 * 1024 * 1024;

fn run_migrate_command(command: MigrateCommand) -> Result<(), String> {
    match command {
        MigrateCommand::Send { snapshot, address } => {
            let bytes = std::fs::read(&snapshot)
                .map_err(|error| format!("cannot read snapshot {}: {error}", snapshot.display()))?;
            let snapshot_value = synos_vm::VmSnapshot::from_bytes(&bytes)
                .map_err(|error| format!("cannot validate snapshot {}: {error}", snapshot.display()))?;
            let bytes = snapshot_value.to_bytes();
            let mut stream = TcpStream::connect(&address)
                .map_err(|error| format!("cannot connect to migration target {address}: {error}"))?;
            stream
                .write_all(MIGRATION_MAGIC)
                .and_then(|_| stream.write_all(&(bytes.len() as u64).to_le_bytes()))
                .and_then(|_| stream.write_all(&bytes))
                .map_err(|error| format!("migration send failed: {error}"))?;
            println!("sent VM checkpoint {} to {address}", snapshot.display());
            Ok(())
        }
        MigrateCommand::Receive { address, snapshot } => {
            if snapshot.exists() {
                return Err(format!(
                    "migration target {} already exists; choose a new path",
                    snapshot.display()
                ));
            }
            let listener = TcpListener::bind(&address)
                .map_err(|error| format!("cannot listen for migration on {address}: {error}"))?;
            println!("waiting for VM checkpoint on {address}");
            let (mut stream, peer) = listener
                .accept()
                .map_err(|error| format!("migration accept failed: {error}"))?;
            let mut magic = [0u8; 8];
            stream
                .read_exact(&mut magic)
                .map_err(|error| format!("migration header read failed: {error}"))?;
            if &magic != MIGRATION_MAGIC {
                return Err("migration stream has an invalid header".to_string());
            }
            let mut length = [0u8; 8];
            stream
                .read_exact(&mut length)
                .map_err(|error| format!("migration length read failed: {error}"))?;
            let length = u64::from_le_bytes(length);
            if length == 0 || length > MAX_MIGRATION_BYTES {
                return Err(format!("migration payload is too large: {length} bytes"));
            }
            let length = usize::try_from(length)
                .map_err(|_| "migration payload does not fit in host memory".to_string())?;
            let mut bytes = vec![0u8; length];
            stream
                .read_exact(&mut bytes)
                .map_err(|error| format!("migration payload read failed: {error}"))?;
            synos_vm::VmSnapshot::from_bytes(&bytes)
                .map_err(|error| format!("received invalid VM checkpoint: {error}"))?;
            let partial = snapshot.with_extension("synos-migration-partial");
            std::fs::write(&partial, bytes)
                .map_err(|error| format!("cannot write migration checkpoint: {error}"))?;
            std::fs::rename(&partial, &snapshot)
                .map_err(|error| format!("cannot publish migration checkpoint: {error}"))?;
            println!("received VM checkpoint from {peer} at {}", snapshot.display());
            Ok(())
        }
    }
}

struct MonitorSession {
    #[cfg(unix)]
    listener: UnixListener,
    path: PathBuf,
}

impl MonitorSession {
    fn bind(path: PathBuf) -> Result<Self, String> {
        #[cfg(unix)]
        {
            if path.exists() {
                return Err(format!("monitor socket {} already exists", path.display()));
            }
            let listener = UnixListener::bind(&path)
                .map_err(|error| format!("cannot create monitor socket {}: {error}", path.display()))?;
            listener
                .set_nonblocking(true)
                .map_err(|error| format!("cannot configure monitor socket: {error}"))?;
            println!("monitor console listening at {}", path.display());
            Ok(Self { listener, path })
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err("monitor console is only supported on Unix hosts".to_string())
        }
    }

    fn poll(&mut self, vm: &mut Vm) -> Result<bool, String> {
        #[cfg(unix)]
        {
            let (mut stream, _) = match self.listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(true),
                Err(error) => return Err(format!("monitor accept failed: {error}")),
            };
            stream
                .set_nonblocking(true)
                .map_err(|error| format!("cannot configure monitor client: {error}"))?;
            let mut buffer = [0u8; 4096];
            let count = match stream.read(&mut buffer) {
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(true),
                Err(error) => return Err(format!("monitor read failed: {error}")),
            };
            let command = String::from_utf8_lossy(&buffer[..count]).trim().to_string();
            let (keep_running, response) = monitor_command(vm, &command)?;
            stream
                .write_all(response.as_bytes())
                .map_err(|error| format!("monitor write failed: {error}"))?;
            return Ok(keep_running)
        }
        #[cfg(not(unix))]
        {
            let _ = vm;
            Ok(true)
        }
    }
}

impl Drop for MonitorSession {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn monitor_command(vm: &mut Vm, command: &str) -> Result<(bool, String), String> {
    match command {
        "help" | "?" => Ok((true, "help\ninfo registers\ninfo disks\ninfo status\nsave PATH\nquit\n".to_string())),
        "info registers" => Ok((
            true,
            format!(
                "rip=0x{:016x} rax=0x{:016x} rbx=0x{:016x} rcx=0x{:016x} rdx=0x{:016x} rflags=0x{:016x} halted={}\n",
                vm.cpu().state.rip,
                vm.cpu().state.rax,
                vm.cpu().state.rbx,
                vm.cpu().state.rcx,
                vm.cpu().state.rdx,
                vm.cpu().state.rflags,
                vm.cpu().state.halted,
            ),
        )),
        "info disks" => {
            let mut response = String::new();
            for disk in vm.disks() {
                response.push_str(&format!(
                    "id={} role={:?} controller={:?} capacity={} read-only={} path={}\n",
                    disk.id,
                    disk.role,
                    disk.controller,
                    disk.capacity,
                    disk.read_only,
                    disk.image_path.display(),
                ));
            }
            if response.is_empty() {
                response.push_str("no disks attached\n");
            }
            Ok((true, response))
        }
        "info status" => Ok((true, format!("power={:?}\n", vm.power_state()))),
        "quit" | "exit" => Ok((false, "stopping VM\n".to_string())),
        command if command.strip_prefix("save ").is_some() => {
            let path = command.strip_prefix("save ").unwrap().trim();
            if path.is_empty() {
                return Err("save needs a snapshot PATH".to_string());
            }
            let path = PathBuf::from(path);
            vm.save_snapshot(&path)
                .map_err(|error| format!("cannot save snapshot: {error}"))?;
            Ok((true, format!("saved {}\n", path.display())))
        }
        _ => Ok((true, "unknown monitor command; try help\n".to_string())),
    }
}

fn prepare_disk_specs(options: &DiskOptions) -> Result<Vec<DiskSpec>, String> {
    let mut specs = Vec::new();
    if options.system_disk.is_none() && options.disks.is_empty() {
        return Ok(specs);
    }
    if let Some(path) = options.system_disk.as_ref() {
        specs.push(make_disk_spec("system", DiskRole::System, path, options)?);
    }
    for (index, path) in options.disks.iter().enumerate() {
        specs.push(make_disk_spec(
            &format!("disk{index}"),
            DiskRole::Data,
            path,
            options,
        )?);
    }
    Ok(specs)
}

fn make_disk_spec(
    id: &str,
    role: DiskRole,
    path: &PathBuf,
    options: &DiskOptions,
) -> Result<DiskSpec, String> {
    if !path.exists() {
        if !options.create_if_missing {
            return Err(format!(
                "disk path {} does not exist; pass --create-if-missing explicitly",
                path.display()
            ));
        }
        let size = options.size.ok_or_else(|| {
            format!(
                "disk path {} is missing; --disk-size is required with --create-if-missing",
                path.display()
            )
        })?;
        let create_options = SystemDiskCreateOptions::new(size)
            .with_format(options.format.unwrap_or(DiskFormat::Raw));
        SystemDiskProvisioner::create(path, create_options)
            .map_err(|error| format!("cannot create disk {}: {error}", path.display()))?;
    }
    let canonical = std::fs::canonicalize(path)
        .map_err(|error| format!("cannot canonicalize disk {}: {error}", path.display()))?;
    let mut spec = DiskSpec::new(id, canonical)
        .with_controller(options.controller)
        .with_persistence(options.persistence)
        .read_only(options.read_only);
    spec.role = role;
    if let Some(format) = options.format {
        spec = spec.with_format(format);
    }
    if let Some(size) = options.size {
        spec = spec.with_capacity(size);
    }
    Ok(spec)
}

fn run_disk_command(command: DiskCommand) -> Result<(), String> {
    match command {
        DiskCommand::List(options) => {
            let specs = prepare_disk_specs(&options)?;
            if specs.is_empty() {
                return Err("disk list needs --disk PATH or --system-disk PATH".to_string());
            }
            print_disk_inventory(&specs)
        }
        DiskCommand::Inspect(path) => inspect_disk(&path),
        DiskCommand::Validate(path) => {
            let manifest = SystemDiskProvisioner::validate(&path)
                .map_err(|error| format!("disk validation failed for {}: {error}", path.display()))?;
            println!(
                "valid system disk: path={} format={} capacity={} generation={} kernel={} bytes initrd={} bytes",
                canonical_display(&path)?,
                format_name(manifest.format),
                format_bytes(manifest.disk_size),
                manifest.generation,
                manifest.layout.kernel_size,
                manifest.layout.initrd_size,
            );
            Ok(())
        }
        DiskCommand::Provision {
            path,
            kernel,
            initrd,
            size,
            format,
            replace,
            boot_args,
            machine_identity,
            network_identity,
        } => {
            let mut install = SystemDiskInstall::new(kernel)
                .with_boot_args(boot_args)
                .with_machine_identity(machine_identity)
                .with_network_identity(network_identity);
            if let Some(initrd) = initrd {
                install = install.with_initrd(initrd);
            }
            let manifest = if let Some(size) = size {
                SystemDiskProvisioner::provision_with_options(
                    &path,
                    SystemDiskCreateOptions::new(size)
                        .with_format(format)
                        .replace_existing(replace),
                    &install,
                )
                .map_err(|error| format!("disk provisioning failed: {error}"))?
            } else if replace {
                return Err("disk provision --replace needs --size".to_string());
            } else {
                SystemDiskProvisioner::provision(&path, &install)
                    .map_err(|error| format!("disk provisioning failed: {error}"))?
            };
            println!(
                "provisioned system disk: path={} format={} capacity={} generation={}",
                canonical_display(&path)?,
                format_name(manifest.format),
                format_bytes(manifest.disk_size),
                manifest.generation,
            );
            Ok(())
        }
        DiskCommand::Lock(path) => print_lock_status(&path),
        DiskCommand::RecoverLock(path) => {
            let info = DiskImage::recover_stale_lock(&path)
                .map_err(|error| format!("cannot recover lock for {}: {error}", path.display()))?;
            println!("recovered stale disk lock {} (owner: {})", info.path.display(), info.owner.trim());
            Ok(())
        }
    }
}

fn print_disk_inventory(specs: &[DiskSpec]) -> Result<(), String> {
    for spec in specs {
        let info = DiskManager::inspect(spec);
        match info {
            Ok(info) => {
                let lock = DiskImage::inspect_lock(&info.image_path)
                    .map_err(|error| format!("cannot inspect disk lock: {error}"))?;
                let health = if spec.role == DiskRole::System {
                    match SystemDiskProvisioner::validate(&info.image_path) {
                        Ok(_) => "healthy".to_string(),
                        Err(_) => "uninstalled or invalid".to_string(),
                    }
                } else {
                    "healthy".to_string()
                };
                let lock_state = lock
                    .map(|lock| format!("locked(stale={})", lock.stale))
                    .unwrap_or_else(|| "unlocked".to_string());
                println!(
                    "id={} role={} controller={} format={} capacity={} persistence={} read-only={} health={} guest-id={} path={} lock={}",
                    info.id,
                    role_name(info.role),
                    controller_name(info.controller),
                    format_name(info.format),
                    format_bytes(info.capacity),
                    persistence_name(info.persistence),
                    info.read_only,
                    health,
                    info.guest_id,
                    info.image_path.display(),
                    lock_state,
                );
            }
            Err(error) => println!(
                "id={} role={} path={} health=invalid ({error})",
                spec.id,
                role_name(spec.role),
                spec.image_path.display(),
            ),
        }
    }
    Ok(())
}

fn inspect_disk(path: &PathBuf) -> Result<(), String> {
    let image = DiskImage::open_with_access(path, false)
        .map_err(|error| format!("cannot inspect disk {}: {error}", path.display()))?;
    let lock = DiskImage::inspect_lock(path)
        .map_err(|error| format!("cannot inspect disk lock: {error}"))?;
    let health = match SystemDiskProvisioner::validate(path) {
        Ok(_) => "healthy system installation".to_string(),
        Err(_) => "valid image, no installed system".to_string(),
    };
    println!(
        "path={} format={} capacity={} health={} lock={}",
        canonical_display(path)?,
        format_name(image.format()),
        format_bytes(image.size()),
        health,
        lock.map(|value| format!("present(stale={})", value.stale))
            .unwrap_or_else(|| "absent".to_string()),
    );
    Ok(())
}

fn print_lock_status(path: &PathBuf) -> Result<(), String> {
    match DiskImage::inspect_lock(path)
        .map_err(|error| format!("cannot inspect lock for {}: {error}", path.display()))?
    {
        Some(info) => println!(
            "lock={} stale={} pid={} image={} owner={}",
            info.path.display(),
            info.stale,
            info.pid.map(|pid| pid.to_string()).unwrap_or_else(|| "unknown".to_string()),
            info.image_path
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            info.owner.trim().replace('\n', "; "),
        ),
        None => println!("lock={} state=absent", DiskImage::lock_path(path).display()),
    }
    Ok(())
}

fn canonical_display(path: &PathBuf) -> Result<String, String> {
    std::fs::canonicalize(path)
        .map(|value| value.display().to_string())
        .map_err(|error| format!("cannot canonicalize {}: {error}", path.display()))
}

fn format_bytes(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * MIB;
    if bytes % GIB == 0 {
        format!("{} GiB", bytes / GIB)
    } else if bytes % MIB == 0 {
        format!("{} MiB", bytes / MIB)
    } else {
        format!("{} bytes", bytes)
    }
}

fn format_name(format: DiskFormat) -> &'static str {
    match format {
        DiskFormat::Raw => "raw",
        DiskFormat::Vhd => "vhd",
        DiskFormat::Qcow2 => "qcow2",
    }
}

fn controller_name(controller: DiskController) -> &'static str {
    match controller {
        DiskController::Ahci => "ahci",
        DiskController::Nvme => "nvme",
        DiskController::VirtioBlk => "virtio-blk",
    }
}

fn role_name(role: DiskRole) -> &'static str {
    match role {
        DiskRole::System => "system",
        DiskRole::Data => "data",
    }
}

fn persistence_name(persistence: synos_vm::DiskPersistence) -> &'static str {
    match persistence {
        synos_vm::DiskPersistence::Persistent => "persistent",
        synos_vm::DiskPersistence::CopyOnWrite => "copy-on-write",
        synos_vm::DiskPersistence::Disposable => "disposable",
    }
}

fn format_memory(bytes: usize) -> String {
    const KIB: usize = 1024;
    const MIB: usize = KIB * 1024;
    const GIB: usize = MIB * 1024;

    if bytes % GIB == 0 {
        format!("{} GiB", bytes / GIB)
    } else if bytes % MIB == 0 {
        format!("{} MiB", bytes / MIB)
    } else if bytes % KIB == 0 {
        format!("{} KiB", bytes / KIB)
    } else {
        format!("{} bytes", bytes)
    }
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
      --serial-port <PORT>   Serial port: com1, com2, or a hex I/O base
      --interactive          Force raw interactive terminal mode
      --non-interactive      Disable raw mode; keep pipe input usable
      --input <MODE>         Host input path: serial (default) or ps2
      --steps <COUNT>        Run a bounded number of instructions

Disk options:
      --disk <PATH>          Attach a data disk; repeat for more disks
      --system-disk <PATH>   Attach the SynOS system disk
      --disk-controller <C>  Controller: ahci, nvme, or virtio-blk
      --disk-format <F>      Image format: raw, vhd, or qcow2
      --disk-size <SIZE>     Require this capacity, or use it when creating
      --read-only            Open disks without writable ownership
      --copy-on-write        Use a temporary clone for VM writes
      --disposable            Use a temporary clone and discard VM writes
      --create-if-missing    Explicitly create a missing disk image
      --list-disks           Inspect configured disks and do not boot

State and management:
      --snapshot-save <PATH> Save a checkpoint when the VM stops
      --snapshot-restore <PATH>
                              Restore a checkpoint before running
      --monitor <SOCKET>     Expose a local monitor console socket

Commands:
      --integration          Run SynOS integration checks
  synos-vm disk list          List disk health and guest identities
  synos-vm disk inspect PATH  Inspect an image without modifying it
  synos-vm disk validate PATH Validate an installed system disk
  synos-vm disk provision PATH --kernel PATH [OPTIONS]
                              Create/install a system disk without booting
  synos-vm disk lock PATH     Diagnose an ownership lock
  synos-vm disk recover-lock PATH
                              Recover a lock only when its owner is stale
  synos-vm migrate send SNAPSHOT ADDRESS
                              Send a checkpoint to another VM host
  synos-vm migrate receive ADDRESS SNAPSHOT
                              Receive a checkpoint for later restore

Other options:
  -h, --help                Show this help
  -V, --version             Show the version"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn cli_parses_bios_machine_and_terminal_options() {
        let parsed = parse_args(args(&[
            "--firmware",
            "bios",
            "--memory",
            "64M",
            "--cpus",
            "2",
            "--serial-port",
            "com2",
            "--input",
            "ps2",
            "--non-interactive",
            "--steps",
            "12",
        ]))
        .expect("parse CLI");
        let ParseResult::Run(cli) = parsed else {
            panic!("expected run configuration")
        };
        assert_eq!(cli.config.memory_size, 64 * 1024 * 1024);
        assert_eq!(cli.config.smp_cores, 2);
        assert_eq!(cli.config.serial_port, COM2_PORT);
        assert_eq!(cli.input_mode, TerminalInputMode::Ps2);
        assert_eq!(cli.terminal, Some(false));
        assert_eq!(cli.config.max_steps, Some(12));
    }

    #[test]
    fn cli_rejects_invalid_combinations() {
        assert!(parse_args(args(&["--efi", "app.efi"])).is_err());
        assert!(parse_args(args(&["--integration"])).is_err());
        assert!(parse_args(args(&["--interactive", "--steps", "1"])).is_err());
        assert!(parse_args(args(&["--cpus", "0"])).is_err());
        assert!(parse_args(args(&["--unknown"])).is_err());
    }

    #[test]
    fn cli_supports_help_and_version_commands() {
        assert!(matches!(parse_args(args(&["--help"])), Ok(ParseResult::Help)));
        assert!(matches!(parse_args(args(&["--version"])), Ok(ParseResult::Version)));
    }

    #[test]
    fn cli_parses_hex_ports_and_memory_suffixes() {
        assert_eq!(parse_serial_port("0x3f8").expect("COM1 port"), COM1_PORT);
        assert_eq!(parse_serial_port("2f8").expect("COM2 port"), COM2_PORT);
        assert_eq!(parse_memory("1GiB"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_memory("4096K"), Some(4096 * 1024));
        assert_eq!(parse_memory("0"), None);
    }
}
