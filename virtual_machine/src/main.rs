use std::fs::{self, File, OpenOptions};
use std::io::{IsTerminal, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(not(unix))]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
#[cfg(unix)]
use std::os::unix::net::UnixListener;

use synos_vm::{
    run_synos_integration, DiskController, DiskFormat, DiskImage, DiskManager, DiskPersistence,
    DiskRole,
    DiskSpec, FirmwareMode, SystemDiskCreateOptions, SystemDiskInstall, SystemDiskProvisioner,
    GuestInputMode, HardwareAcceleration, TerminalExit, TerminalSession, Vm, VmConfig,
    migration_checkpoint_tag, validate_migration_checkpoint, snapshot_digest, SnapshotAuthKey,
    SnapshotFeatures, SnapshotSchema, MAX_MIGRATION_ALLOCATION_BYTES, COM1_PORT, COM2_PORT,
};

mod control;

use control::{
    MonitorAuthenticator, MonitorCommand, MonitorPermissions, MonitorRequestBuffer,
    MonitorRequestFrame, MAX_MONITOR_CLIENTS, MIGRATION_PROTOCOL_VERSION,
    MONITOR_CONNECTION_LIFETIME_SECS,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Cli {
    command: Command,
    config: VmConfig,
    memory_explicit: bool,
    efi_path: Option<PathBuf>,
    integration: bool,
    terminal: Option<bool>,
    input_mode: GuestInputMode,
    disk_options: DiskOptions,
    snapshot_save: Option<PathBuf>,
    snapshot_restore: Option<PathBuf>,
    snapshot_key: Option<PathBuf>,
    monitor_path: Option<PathBuf>,
    monitor_auth_key: Option<PathBuf>,
    monitor_permissions: MonitorPermissions,
    replay_record: Option<PathBuf>,
    replay_path: Option<PathBuf>,
    verbose: bool,
}

enum Command {
    Run,
    ListDisks,
}

enum MigrateCommand {
    Send {
        snapshot: PathBuf,
        address: String,
        security: MigrationSecurity,
    },
    Receive {
        address: String,
        snapshot: PathBuf,
        security: MigrationSecurity,
    },
}

struct MigrationSecurity {
    key: PathBuf,
    authorized_peer_key_id: [u8; 16],
    audit_log: PathBuf,
}

impl MigrateCommand {
    fn security(&self) -> &MigrationSecurity {
        match self {
            Self::Send { security, .. } | Self::Receive { security, .. } => security,
        }
    }

    fn direction(&self) -> &'static str {
        match self {
            Self::Send { .. } => "send",
            Self::Receive { .. } => "receive",
        }
    }

    fn address(&self) -> &str {
        match self {
            Self::Send { address, .. } | Self::Receive { address, .. } => address,
        }
    }
}

enum DiskCommand {
    List(DiskOptions),
    Inspect { path: PathBuf, json: bool },
    Validate { path: PathBuf, json: bool },
    Repair { path: PathBuf, json: bool },
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
        json: bool,
    },
    Lock { path: PathBuf, verbose: bool, json: bool },
    RecoverLock { path: PathBuf, verbose: bool, json: bool },
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
    json: bool,
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
            json: false,
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
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| {
            arg.into_string()
                .map_err(|_| "command argument is not valid UTF-8".to_string())
        })
        .collect::<Result<Vec<_>, _>>();
    match args.and_then(parse_args) {
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
    let mut input_mode = GuestInputMode::Serial;
    let mut command = Command::Run;
    let mut disk_options = DiskOptions::default();
    let mut snapshot_save = None;
    let mut snapshot_restore = None;
    let mut snapshot_key = None;
    let mut monitor_path = None;
    let mut monitor_auth_key = None;
    let mut monitor_permissions = MonitorPermissions::status_only();
    let mut monitor_permissions_explicit = false;
    let mut replay_record = None;
    let mut replay_path = None;
    let mut verbose = false;
    let mut args = values.into_iter().peekable();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(ParseResult::Help),
            "-V" | "--version" => return Ok(ParseResult::Version),
            "-v" | "--v" | "-verbose" | "--verbose" => verbose = true,
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
            "--accel" | "--hardware-acceleration" => {
                let value = next_value(&mut args, "--accel")?;
                config.hardware_acceleration = parse_hardware_acceleration(&value)?;
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
                    "serial" => GuestInputMode::Serial,
                    "ps2" | "keyboard" => GuestInputMode::Ps2,
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
            "--snapshot-key" => {
                snapshot_key = Some(PathBuf::from(next_value(&mut args, "--snapshot-key")?));
            }
            "--monitor" => {
                monitor_path = Some(PathBuf::from(next_value(&mut args, "--monitor")?));
            }
            "--monitor-auth-key" => {
                monitor_auth_key = Some(PathBuf::from(next_value(&mut args, "--monitor-auth-key")?));
            }
            "--monitor-allow" => {
                let parsed = MonitorPermissions::parse(&next_value(&mut args, "--monitor-allow")?)?;
                if monitor_permissions_explicit {
                    return Err("--monitor-allow may only be supplied once".to_string())
                }
                monitor_permissions = parsed;
                monitor_permissions_explicit = true;
            }
            "--replay-record" => {
                replay_record = Some(PathBuf::from(next_value(&mut args, "--replay-record")?));
            }
            "--replay" => {
                replay_path = Some(PathBuf::from(next_value(&mut args, "--replay")?));
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
        && input_mode == GuestInputMode::Serial
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
    if integration && (replay_record.is_some() || replay_path.is_some()) {
        return Err("replay options cannot be combined with --integration".to_string());
    }
    if replay_record.is_some() && replay_path.is_some() {
        return Err("--replay-record and --replay cannot be combined".to_string());
    }
    if (snapshot_save.is_some() || snapshot_restore.is_some()) && snapshot_key.is_none()
    {
        return Err(
            "snapshot save and restore require --snapshot-key <PATH>".to_string(),
        );
    }
    if monitor_path.is_some() && monitor_auth_key.is_none() {
        return Err("--monitor requires --monitor-auth-key <PATH>".to_string())
    }
    if monitor_path.is_none() && (monitor_auth_key.is_some() || monitor_permissions_explicit) {
        return Err("--monitor-auth-key and --monitor-allow require --monitor <SOCKET>".to_string())
    }
    if monitor_path.is_some()
        && monitor_permissions.allows(control::MonitorPermission::Save)
        && snapshot_key.is_none()
    {
        return Err("monitor `save` permission requires --snapshot-key <PATH>".to_string())
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
        snapshot_key,
        monitor_path,
        monitor_auth_key,
        monitor_permissions,
        replay_record,
        replay_path,
        verbose,
    }))
}

fn parse_migrate_command(values: &[String]) -> Result<ParseResult, String> {
    match values.first().map(String::as_str) {
        Some("send") if values.len() >= 3 => {
            Ok(ParseResult::Migrate(MigrateCommand::Send {
                snapshot: PathBuf::from(&values[1]),
                address: values[2].clone(),
                security: parse_migration_security(&values[3..])?,
            }))
        }
        Some("receive") if values.len() >= 3 => {
            Ok(ParseResult::Migrate(MigrateCommand::Receive {
                address: values[1].clone(),
                snapshot: PathBuf::from(&values[2]),
                security: parse_migration_security(&values[3..])?,
            }))
        }
        Some("help" | "--help" | "-h") if values.len() == 1 => {
            Ok(ParseResult::Help)
        }
        None => Err("migrate needs `send SNAPSHOT ADDRESS` or `receive ADDRESS SNAPSHOT` plus security options".to_string()),
        _ => Err(
            "usage: synos-vm migrate send SNAPSHOT ADDRESS|receive ADDRESS SNAPSHOT --key KEY --peer-key-id ID --audit-log PATH --secure-transport"
                .to_string(),
        ),
    }
}

fn parse_migration_security(values: &[String]) -> Result<MigrationSecurity, String> {
    let mut key = None;
    let mut authorized_peer_key_id = None;
    let mut audit_log = None;
    let mut secure_transport = false;
    let mut args = values.iter().peekable();
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--key" => key = Some(PathBuf::from(next_ref(&mut args, "--key")?)),
            "--peer-key-id" => {
                authorized_peer_key_id = Some(parse_migration_key_id(next_ref(
                    &mut args,
                    "--peer-key-id",
                )?)?)
            }
            "--audit-log" => {
                audit_log = Some(PathBuf::from(next_ref(&mut args, "--audit-log")?))
            }
            "--secure-transport" => secure_transport = true,
            value => return Err(format!("unknown migration security option `{value}`")),
        }
    }
    if !secure_transport {
        return Err("migration requires --secure-transport to confirm TLS, VPN, or SSH tunnel protection".to_string())
    }
    Ok(MigrationSecurity {
        key: key.ok_or_else(|| "migration requires --key KEY".to_string())?,
        authorized_peer_key_id: authorized_peer_key_id
            .ok_or_else(|| "migration requires --peer-key-id ID".to_string())?,
        audit_log: audit_log
            .ok_or_else(|| "migration requires --audit-log PATH".to_string())?,
    })
}

fn parse_migration_key_id(value: &str) -> Result<[u8; 16], String> {
    if value.len() != 32 {
        return Err("migration peer key ID must contain 32 hexadecimal characters".to_string())
    }
    let mut output = [0; 16];
    for (index, byte) in output.iter_mut().enumerate() {
        let high = migration_hex_value(value.as_bytes()[index * 2]);
        let low = migration_hex_value(value.as_bytes()[index * 2 + 1]);
        let (Some(high), Some(low)) = (high, low) else {
            return Err("migration peer key ID is not hexadecimal".to_string())
        };
        *byte = (high << 4) | low;
    }
    Ok(output)
}

fn migration_hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn parse_disk_command(values: &[String]) -> Result<ParseResult, String> {
    let subcommand = values
        .first()
        .map(String::as_str)
        .ok_or_else(|| "disk needs a command: list, inspect, validate, repair, provision, lock, or recover-lock".to_string())?;
    match subcommand {
        "list" => Ok(ParseResult::Disk(DiskCommand::List(parse_disk_options(&values[1..])?))),
        "inspect" => {
            let (path, json) = command_path(values, "inspect")?;
            Ok(ParseResult::Disk(DiskCommand::Inspect { path, json }))
        }
        "validate" => {
            let (path, json) = command_path(values, "validate")?;
            Ok(ParseResult::Disk(DiskCommand::Validate { path, json }))
        }
        "repair" => {
            let (path, json) = command_path(values, "repair")?;
            Ok(ParseResult::Disk(DiskCommand::Repair { path, json }))
        }
        "lock" => parse_lock_command(values),
        "recover-lock" => parse_recover_lock_command(values),
        "provision" => parse_provision_command(&values[1..]),
        "help" | "--help" | "-h" => Ok(ParseResult::Help),
        value => Err(format!("unknown disk command `{value}`")),
    }
}

fn parse_lock_command(values: &[String]) -> Result<ParseResult, String> {
    let mut path = None;
    let mut verbose = false;
    let mut json = false;
    for value in &values[1..] {
        match value.as_str() {
            "-v" | "--v" | "-verbose" | "--verbose" => verbose = true,
            "--json" => json = true,
            _ if path.is_none() => path = Some(PathBuf::from(value)),
            _ => return Err("disk lock needs exactly one PATH".to_string()),
        }
    }
    let path = path.ok_or_else(|| "disk lock needs exactly one PATH".to_string())?;
    Ok(ParseResult::Disk(DiskCommand::Lock { path, verbose, json }))
}

fn parse_recover_lock_command(values: &[String]) -> Result<ParseResult, String> {
    let mut path = None;
    let mut verbose = false;
    let mut json = false;
    for value in &values[1..] {
        match value.as_str() {
            "-v" | "--v" | "-verbose" | "--verbose" => verbose = true,
            "--json" => json = true,
            _ if path.is_none() => path = Some(PathBuf::from(value)),
            _ => return Err("disk recover-lock needs exactly one PATH".to_string()),
        }
    }
    let path = path.ok_or_else(|| "disk recover-lock needs exactly one PATH".to_string())?;
    Ok(ParseResult::Disk(DiskCommand::RecoverLock { path, verbose, json }))
}

fn command_path(values: &[String], command: &str) -> Result<(PathBuf, bool), String> {
    if values.len() < 2 || values.len() > 3 || (values.len() == 3 && values[2] != "--json") {
        return Err(format!("disk {command} needs exactly one PATH"));
    }
    Ok((PathBuf::from(&values[1]), values.len() == 3))
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
            "--json" => options.json = true,
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
    let mut json = false;
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
            "--json" => json = true,
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
        json,
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

fn parse_hardware_acceleration(value: &str) -> Result<HardwareAcceleration, String> {
    match value.to_ascii_lowercase().as_str() {
        "software" | "none" => Ok(HardwareAcceleration::Software),
        "auto" => Ok(HardwareAcceleration::Auto),
        "kvm" => Ok(HardwareAcceleration::Kvm),
        "haxm" => Ok(HardwareAcceleration::Haxm),
        "hvf" => Ok(HardwareAcceleration::Hvf),
        "whpx" => Ok(HardwareAcceleration::Whpx),
        _ => Err(format!(
            "invalid accelerator `{value}`; use software, auto, kvm, haxm, hvf, or whpx"
        )),
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

fn load_monitor_auth_key(path: &Path) -> Result<SnapshotAuthKey, String> {
    #[cfg(unix)]
    {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err(format!("{} must be a regular file, not a symlink", path.display()))
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(format!("{} must not be accessible by group or other users", path.display()))
        }
    }
    SnapshotAuthKey::from_file(path).map_err(|error| error.to_string())
}

fn run(mut cli: Cli) -> Result<(), String> {
    let list_disks = matches!(&cli.command, Command::ListDisks);
    cli.config.disks = prepare_disk_specs(&cli.disk_options)?;
    if list_disks {
        return print_disk_inventory(&cli.config.disks, false);
    }

    let snapshot_key = cli
        .snapshot_key
        .as_ref()
        .map(SnapshotAuthKey::from_file)
        .transpose()
        .map_err(|error| format!("cannot load snapshot authentication key: {error}"))?;
    let monitor_auth_key = cli
        .monitor_auth_key
        .as_ref()
        .map(|path| load_monitor_auth_key(path))
        .transpose()
        .map_err(|error| format!("cannot load monitor authentication key: {error}"))?;
    if snapshot_key.is_some()
        && monitor_auth_key.map(SnapshotAuthKey::key_id) == snapshot_key.map(SnapshotAuthKey::key_id)
    {
        return Err("monitor and snapshot authentication keys must be different".to_string())
    }
    let auth_key = snapshot_key.as_ref();
    let restore_snapshot = cli
        .snapshot_restore
        .as_ref()
        .map(|path| {
            let key = auth_key.ok_or_else(|| "snapshot restore needs an authentication key".to_string())?;
            Vm::load_authenticated_snapshot(path, *key)
                .map_err(|error| format!("cannot load authenticated snapshot: {error}"))
        })
        .transpose()?;
    let verbose = cli.verbose;
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
    let monitor_permissions = cli.monitor_permissions;
    let replay_record = cli.replay_record;
    let replay_path = cli.replay_path;
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
        .map_err(|error| format!("VM configuration error: {}", vm_error_message(&error, verbose)))?;
    synos_vm::host_println(format_args!(
        "Hardware acceleration: {}",
        vm.hardware_acceleration()
    ));
    if let Some(image) = efi_image {
        vm.set_efi_application(image);
    }
    if let Some(snapshot) = restore_snapshot.as_ref() {
        let restore_report = vm
            .restore_snapshot_with_report(snapshot)
            .map_err(|error| format!("cannot restore snapshot: {error}"))?;
        println!("Restored VM snapshot (checksum=0x{:016x})", snapshot.checksum());
        println!(
            "Snapshot restored: {}",
            restore_report.restored_state().join(", ")
        );
        for state in restore_report.rebuild_required_state() {
            eprintln!("Snapshot restore needs host state rebuilt: {state}");
        }
        if restore_report.has_excluded_state() {
            for state in restore_report.excluded_state() {
                eprintln!("Snapshot restore excluded host state: {state}");
            }
        }
    }
    if let Some(path) = replay_path.as_ref() {
        let trace = Vm::load_replay(path)
            .map_err(|error| format!("cannot load replay trace {}: {error}", path.display()))?;
        vm.begin_replay(trace)
            .map_err(|error| format!("cannot start replay: {error}"))?;
        println!("Replaying VM input trace from {}", path.display());
    } else if replay_record.is_some() {
        vm.begin_replay_recording();
    }

    let terminal_mode = if monitor_path.is_some() && terminal_mode.is_none() {
        Some(false)
    } else {
        terminal_mode
    };
    let mut monitor = monitor_path
        .map(|path| {
            let key = monitor_auth_key
                .ok_or_else(|| "monitor authentication key is missing".to_string())?;
            MonitorSession::bind(path, key, monitor_permissions)
        })
        .transpose()?;
    let mut poll_monitor = |vm: &mut Vm| {
        monitor
            .as_mut()
            .map(|session| session.poll(vm, auth_key))
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
        synos_vm::host_println(format_args!("Starting CPU emulation..."));
        let terminal = TerminalSession::new(if replay_path.is_some() {
            Some(false)
        } else {
            terminal_mode
        })
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
        let key = auth_key.ok_or_else(|| "snapshot save needs an authentication key".to_string())?;
        vm.save_authenticated_snapshot(&path, *key)
            .map_err(|error| format!("cannot save authenticated snapshot {}: {error}", path.display()))?;
        println!("Saved VM snapshot to {}", path.display());
    }

    if let Some(path) = replay_record {
        vm.save_replay(&path)
            .map_err(|error| format!("cannot save replay trace {}: {error}", path.display()))?;
        println!("Saved VM replay trace to {}", path.display());
    }

    Ok(())
}

const MIGRATION_MAGIC: &[u8; 8] = b"SYNOMIG3";
const MIGRATION_AUTH_DOMAIN: &[u8] = b"SYNOS-MIGRATION-HMAC-SHA256-V3";
const MIGRATION_NONCE_BYTES: usize = 32;
const MAX_CHECKPOINT_AGE_SECS: u64 = 24 * 60 * 60;
const MAX_CLOCK_SKEW_SECS: u64 = 5 * 60;
const REPLAY_RECORD_BYTES: usize = 56;
const MAX_REPLAY_RECORDS: usize = 4096;
const MAX_REPLAY_LEDGER_BYTES: u64 = (REPLAY_RECORD_BYTES * MAX_REPLAY_RECORDS) as u64;
#[cfg(not(unix))]
static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn authorize_migration_key(
    key: SnapshotAuthKey,
    authorized_peer_key_id: [u8; 16],
) -> Result<(), String> {
    if key.key_id() != authorized_peer_key_id {
        return Err(
            "configured peer key ID does not match the migration authentication key".to_string(),
        )
    }
    Ok(())
}

fn append_migration_audit(
    path: &Path,
    event: &str,
    direction: &str,
    address: &str,
    peer_key_id: [u8; 16],
) -> Result<(), String> {
    let timestamp = now_seconds()?;
    let peer_key_id = encode_migration_key_id(peer_key_id);
    let record = format!(
        "{{\"timestamp\":{timestamp},\"event\":{},\"direction\":{},\"peer\":{},\"peer_key_id\":{},\"protocol_version\":{},\"secure_transport_required\":true}}\n",
        control::json_string(event),
        control::json_string(direction),
        control::json_string(address),
        control::json_string(&peer_key_id),
        MIGRATION_PROTOCOL_VERSION,
    );
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("cannot open migration audit log {}: {error}", path.display()))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect migration audit log: {error}"))?;
    if !metadata.is_file() {
        return Err("migration audit log must be a regular file".to_string())
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err("migration audit log must not be accessible by group or other users".to_string())
    }
    file.write_all(record.as_bytes())
        .and_then(|_| file.sync_data())
        .map_err(|error| format!("cannot append migration audit event: {error}"))
}

fn encode_migration_key_id(value: [u8; 16]) -> String {
    let mut output = String::with_capacity(32);
    for byte in value {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn run_migrate_command(command: MigrateCommand) -> Result<(), String> {
    let audit_log = command.security().audit_log.clone();
    let direction = command.direction();
    let address = command.address().to_string();
    let peer_key_id = command.security().authorized_peer_key_id;
    append_migration_audit(
        &audit_log,
        "started",
        direction,
        &address,
        peer_key_id,
    )?;
    let result = run_migrate_operation(command);
    let outcome = if result.is_ok() { "succeeded" } else { "failed" };
    if let Err(audit_error) = append_migration_audit(
        &audit_log,
        outcome,
        direction,
        &address,
        peer_key_id,
    ) {
        return match result {
            Ok(()) => Err(format!("migration completed but final audit write failed: {audit_error}")),
            Err(error) => Err(format!("{error}; final audit write failed: {audit_error}")),
        }
    }
    result
}

fn run_migrate_operation(command: MigrateCommand) -> Result<(), String> {
    match command {
        MigrateCommand::Send {
            snapshot,
            address,
            security,
        } => {
            let key = SnapshotAuthKey::from_file(&security.key)
                .map_err(|error| format!("cannot load migration authentication key: {error}"))?;
            authorize_migration_key(key, security.authorized_peer_key_id)?;
            let bytes = read_bounded_migration_file(&snapshot)?;
            let snapshot_value = synos_vm::VmSnapshot::from_authenticated_bytes(&bytes, key)
                .map_err(|error| format!("cannot validate authenticated snapshot {}: {error}", snapshot.display()))?;
            let issued_at = checkpoint_timestamp(&snapshot)?;
            validate_checkpoint_time(issued_at)?;
            let sender_nonce = migration_nonce()?;
            let sender_schema = SnapshotSchema::local();
            let sender_auth = migration_sender_handshake_tag(key, &sender_nonce, sender_schema);
            let mut stream = connect_migration(&address)?;
            configure_migration_stream(&stream)?;
            stream
                .write_all(MIGRATION_MAGIC)
                .and_then(|_| write_migration_u32(&mut stream, MIGRATION_PROTOCOL_VERSION))
                .and_then(|_| write_migration_schema(&mut stream, sender_schema))
                .and_then(|_| stream.write_all(&key.key_id()))
                .and_then(|_| stream.write_all(&sender_nonce))
                .and_then(|_| stream.write_all(&sender_auth))
                .map_err(|error| format!("migration schema handshake failed: {error}"))?;
            let (negotiated, receiver_nonce) =
                read_migration_response(
                    &mut stream,
                    key,
                    security.authorized_peer_key_id,
                    &sender_nonce,
                )?;
            let wire_snapshot = snapshot_value
                .convert_to_schema(negotiated)
                .map_err(|error| format!("migration schema conversion failed: {error}"))?;
            let bytes = wire_snapshot
                .to_authenticated_bytes(key)
                .map_err(|error| format!("migration snapshot encoding failed: {error}"))?;
            let length = u64::try_from(bytes.len())
                .map_err(|_| "migration payload does not fit in the wire length".to_string())?;
            let checkpoint_id = snapshot_digest(&bytes);
            let tag = migration_auth_tag(
                key,
                negotiated,
                &sender_nonce,
                &receiver_nonce,
                issued_at,
                &checkpoint_id,
                length,
                &bytes,
            );
            stream
                .write_all(&issued_at.to_le_bytes())
                .and_then(|_| stream.write_all(&checkpoint_id))
                .and_then(|_| stream.write_all(&length.to_le_bytes()))
                .and_then(|_| stream.write_all(&bytes))
                .and_then(|_| stream.write_all(&tag))
                .map_err(|error| format!("migration send failed: {error}"))?;
            println!("sent VM checkpoint {} to {address}", snapshot.display());
            Ok(())
        }
        MigrateCommand::Receive {
            address,
            snapshot,
            security,
        } => {
            let key = SnapshotAuthKey::from_file(&security.key)
                .map_err(|error| format!("cannot load migration authentication key: {error}"))?;
            authorize_migration_key(key, security.authorized_peer_key_id)?;
            let listener = TcpListener::bind(&address)
                .map_err(|error| format!("cannot listen for migration on {address}: {error}"))?;
            println!("waiting for VM checkpoint on {address}");
            let (mut stream, peer) = listener
                .accept()
                .map_err(|error| format!("migration accept failed: {error}"))?;
            configure_migration_stream(&stream)?;
            let mut magic = [0u8; 8];
            stream
                .read_exact(&mut magic)
                .map_err(|error| format!("migration header read failed: {error}"))?;
            if &magic != MIGRATION_MAGIC {
                if &magic == b"SYNOMIG1" || &magic == b"SYNOMIG2" {
                    return Err("unauthenticated migration protocol is no longer accepted".to_string());
                }
                return Err("migration stream has an invalid header".to_string());
            }
            let protocol = read_migration_u32(&mut stream)?;
            if protocol != MIGRATION_PROTOCOL_VERSION {
                return Err(format!("unsupported migration protocol {protocol}"));
            }
            let peer_schema = read_migration_schema(&mut stream)?;
            let mut peer_key_id = [0u8; 16];
            stream
                .read_exact(&mut peer_key_id)
                .map_err(|error| format!("migration key identity read failed: {error}"))?;
            if peer_key_id != security.authorized_peer_key_id {
                return Err("migration peer is not in the configured authorization policy".to_string());
            }
            let mut sender_nonce = [0u8; MIGRATION_NONCE_BYTES];
            stream
                .read_exact(&mut sender_nonce)
                .map_err(|error| format!("migration sender challenge read failed: {error}"))?;
            let mut sender_auth = [0u8; 32];
            stream
                .read_exact(&mut sender_auth)
                .map_err(|error| format!("migration peer authentication read failed: {error}"))?;
            let expected_sender_auth = migration_sender_handshake_tag(key, &sender_nonce, peer_schema);
            key.verify_tag(&expected_sender_auth, &sender_auth)
                .map_err(|error| format!("migration peer authentication failed: {error}"))?;
            let receiver_nonce = migration_nonce()?;
            let negotiated = SnapshotSchema::negotiate_with(SnapshotSchema::local(), peer_schema)
                .map_err(|error| format!("migration schema negotiation failed: {error}"))?;
            let response_tag = migration_handshake_tag(
                key,
                &sender_nonce,
                &receiver_nonce,
                negotiated,
            );
            stream
                .write_all(&[0])
                .and_then(|_| write_migration_schema(&mut stream, negotiated))
                .and_then(|_| stream.write_all(&key.key_id()))
                .and_then(|_| stream.write_all(&receiver_nonce))
                .and_then(|_| stream.write_all(&response_tag))
                .map_err(|error| format!("migration schema response failed: {error}"))?;
            let frame = read_migration_checkpoint(
                &mut stream,
                key,
                negotiated,
                &sender_nonce,
                &receiver_nonce,
                validate_checkpoint_time,
            );
            let frame = frame?;
            reserve_replay(
                &replay_ledger_path(&snapshot),
                key.key_id(),
                frame.checkpoint_id,
                frame.issued_at,
            )?;
            let ledger = replay_ledger_path(&snapshot);
            if let Err(error) = publish_received_checkpoint(&snapshot, &frame.bytes, key, &frame.snapshot) {
                if let Err(release_error) = release_replay(
                    &ledger,
                    key.key_id(),
                    frame.checkpoint_id,
                ) {
                    return Err(format!(
                        "{error}; cannot release migration replay reservation: {release_error}"
                    ));
                }
                return Err(error)
            }
            println!("received VM checkpoint from {peer} at {}", snapshot.display());
            Ok(())
        }
    }
}

fn read_bounded_migration_file(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("cannot inspect snapshot {}: {error}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("migration snapshot {} is not a regular file", path.display()))
    }
    if metadata.len() == 0 || metadata.len() > MAX_MIGRATION_ALLOCATION_BYTES {
        return Err(format!(
            "migration snapshot is too large: {} bytes",
            metadata.len()
        ))
    }
    let file = File::open(path)
        .map_err(|error| format!("cannot read snapshot {}: {error}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(MAX_MIGRATION_ALLOCATION_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read snapshot {}: {error}", path.display()))?;
    if bytes.len() as u64 > MAX_MIGRATION_ALLOCATION_BYTES {
        return Err("migration snapshot grew beyond its byte limit".to_string())
    }
    if bytes.len() as u64 != metadata.len() {
        return Err("migration snapshot changed while it was being read".to_string())
    }
    Ok(bytes)
}

fn read_migration_checkpoint<R, F>(
    reader: &mut R,
    key: SnapshotAuthKey,
    negotiated: SnapshotSchema,
    sender_nonce: &[u8; MIGRATION_NONCE_BYTES],
    receiver_nonce: &[u8; MIGRATION_NONCE_BYTES],
    validate_timestamp: F,
) -> Result<synos_vm::MigrationCheckpointFrame, String>
where
    R: Read,
    F: Fn(u64) -> Result<(), String>,
{
    let mut issued_at = [0u8; 8];
    reader
        .read_exact(&mut issued_at)
        .map_err(|error| format!("migration checkpoint time read failed: {error}"))?;
    let issued_at = u64::from_le_bytes(issued_at);
    validate_timestamp(issued_at)?;

    let mut checkpoint_id = [0u8; 32];
    reader
        .read_exact(&mut checkpoint_id)
        .map_err(|error| format!("migration checkpoint identity read failed: {error}"))?;
    let mut length = [0u8; 8];
    reader
        .read_exact(&mut length)
        .map_err(|error| format!("migration length read failed: {error}"))?;
    let length = u64::from_le_bytes(length);
    if length == 0 || length > MAX_MIGRATION_ALLOCATION_BYTES {
        return Err(format!("migration payload is too large: {length} bytes"));
    }
    let payload_length = usize::try_from(length)
        .map_err(|_| "migration payload does not fit in host memory".to_string())?;
    let mut bytes = vec![0u8; payload_length];
    reader
        .read_exact(&mut bytes)
        .map_err(|error| format!("migration payload read failed: {error}"))?;
    let mut received_tag = [0u8; 32];
    reader
        .read_exact(&mut received_tag)
        .map_err(|error| format!("migration authentication tag read failed: {error}"))?;
    validate_migration_checkpoint(
        key,
        negotiated,
        sender_nonce,
        receiver_nonce,
        issued_at,
        checkpoint_id,
        bytes,
        received_tag,
    )
    .map_err(|error| error.to_string())
}

fn publish_received_checkpoint(
    target: &Path,
    bytes: &[u8],
    key: SnapshotAuthKey,
    expected: &synos_vm::VmSnapshot,
) -> Result<(), String> {
    let parent = target
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let (partial, mut file) = create_migration_temp(parent, target)?;
    let result = match file.write_all(bytes).and_then(|_| file.sync_all()) {
        Ok(()) => {
            drop(file);
            verify_received_checkpoint(&partial, key, expected, "temporary")
                .and_then(|_| publish_checkpoint_file(&partial, target, parent, key, expected))
        }
        Err(error) => {
            drop(file);
            Err(format!("cannot write migration checkpoint: {error}"))
        }
    };
    if result.is_err() {
        let _ = fs::remove_file(&partial);
        let _ = sync_migration_directory(parent);
    }
    result
}

fn create_migration_temp(parent: &Path, target: &Path) -> Result<(PathBuf, File), String> {
    let name = target
        .file_name()
        .ok_or_else(|| "migration target must name a file".to_string())?
        .to_string_lossy();
    for attempt in 0..100u32 {
        let partial = parent.join(format!(
            ".{name}.synos-migration-{}-{attempt}.partial",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        match options.open(&partial)
        {
            Ok(file) => return Ok((partial, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("cannot create migration checkpoint temporary file: {error}")),
        }
    }
    Err("cannot create a unique migration checkpoint temporary file".to_string())
}

fn verify_received_checkpoint(
    path: &Path,
    key: SnapshotAuthKey,
    expected: &synos_vm::VmSnapshot,
    stage: &str,
) -> Result<(), String> {
    let reopened = synos_vm::VmSnapshot::load_authenticated(path, key)
        .map_err(|error| format!("cannot verify {stage} migration checkpoint after reopen: {error}"))?;
    if &reopened != expected {
        return Err(format!("{stage} migration checkpoint changed during publication"));
    }
    Ok(())
}

fn publish_checkpoint_file(
    partial: &Path,
    target: &Path,
    parent: &Path,
    key: SnapshotAuthKey,
    expected: &synos_vm::VmSnapshot,
) -> Result<(), String> {
    let had_old_target = match fs::symlink_metadata(target) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("cannot inspect migration target: {error}")),
    };
    let backup = if had_old_target {
        Some(create_migration_backup(parent, target)?)
    } else {
        None
    };
    if had_old_target {
        if let Err(error) = sync_migration_directory(parent) {
            if let Some(backup) = backup.as_ref() {
                if let Err(restore_error) = restore_migration_target(target, Some(backup), parent) {
                    return Err(format!(
                        "{error}; cannot preserve old migration target: {restore_error}"
                    ));
                }
            }
            return Err(error);
        }
    }

    let mut published = false;
    let result = (|| {
        fs::rename(partial, target)
            .map_err(|error| format!("cannot publish migration checkpoint: {error}"))?;
        published = true;
        sync_migration_directory(parent)?;
        verify_received_checkpoint(target, key, expected, "published")
    })();

    if let Err(error) = result {
        if published {
            if let Err(restore_error) = restore_migration_target(target, backup.as_deref(), parent) {
                return Err(format!(
                    "{error}; cannot preserve old migration target: {restore_error}"
                ));
            }
        } else if let Some(backup) = backup.as_deref() {
            if let Err(restore_error) = restore_migration_target(target, Some(backup), parent) {
                return Err(format!(
                    "{error}; cannot preserve old migration target: {restore_error}"
                ));
            }
        }
        return Err(error);
    }

    if let Some(backup) = backup {
        fs::remove_file(backup)
            .map_err(|error| format!("cannot remove migration checkpoint backup: {error}"))?;
        sync_migration_directory(parent)?;
    }
    Ok(())
}

fn create_migration_backup(parent: &Path, target: &Path) -> Result<PathBuf, String> {
    let name = target
        .file_name()
        .ok_or_else(|| "migration target must name a file".to_string())?
        .to_string_lossy();
    for attempt in 0..100u32 {
        let backup = parent.join(format!(
            ".{name}.synos-migration-{}-{attempt}.backup",
            std::process::id()
        ));
        match create_migration_backup_entry(target, &backup) {
            Ok(()) => return Ok(backup),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("cannot preserve old migration target: {error}")),
        }
    }
    Err("cannot create a unique migration checkpoint backup file".to_string())
}

#[cfg(unix)]
fn create_migration_backup_entry(target: &Path, backup: &Path) -> std::io::Result<()> {
    fs::hard_link(target, backup)
}

#[cfg(not(unix))]
fn create_migration_backup_entry(target: &Path, backup: &Path) -> std::io::Result<()> {
    fs::rename(target, backup)
}

fn restore_migration_target(
    target: &Path,
    backup: Option<&Path>,
    parent: &Path,
) -> Result<(), String> {
    match backup {
        Some(backup) => {
            #[cfg(unix)]
            fs::rename(backup, target)
                .map_err(|error| format!("cannot restore old migration target: {error}"))?;

            #[cfg(not(unix))]
            {
                let _ = fs::remove_file(target);
                fs::rename(backup, target)
                    .map_err(|error| format!("cannot restore old migration target: {error}"))?;
            }
        }
        None => match fs::remove_file(target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot remove failed migration target: {error}")),
        },
    }
    sync_migration_directory(parent)
}

fn sync_migration_directory(parent: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("cannot sync migration target directory: {error}"))?;
    }
    #[cfg(not(unix))]
    let _ = parent;
    Ok(())
}

fn configure_migration_stream(stream: &TcpStream) -> Result<(), String> {
    let timeout = Some(Duration::from_secs(10));
    stream
        .set_read_timeout(timeout)
        .map_err(|error| format!("cannot set migration read timeout: {error}"))?;
    stream
        .set_write_timeout(timeout)
        .map_err(|error| format!("cannot set migration write timeout: {error}"))?;
    Ok(())
}

fn connect_migration(address: &str) -> Result<TcpStream, String> {
    let socket = address
        .to_socket_addrs()
        .map_err(|error| format!("cannot resolve migration target {address}: {error}"))?
        .next()
        .ok_or_else(|| format!("migration target {address} has no socket address"))?;
    TcpStream::connect_timeout(&socket, Duration::from_secs(10))
        .map_err(|error| format!("cannot connect to migration target {address}: {error}"))
}

fn now_seconds() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| format!("host clock is before the Unix epoch: {error}"))
}

fn checkpoint_timestamp(path: &Path) -> Result<u64, String> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| format!("cannot read checkpoint timestamp: {error}"))?
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| format!("checkpoint timestamp is before the Unix epoch: {error}"))
}

fn validate_checkpoint_time(issued_at: u64) -> Result<(), String> {
    let now = now_seconds()?;
    if issued_at > now.saturating_add(MAX_CLOCK_SKEW_SECS) {
        return Err("checkpoint timestamp is too far in the future".to_string());
    }
    if now.saturating_sub(issued_at) > MAX_CHECKPOINT_AGE_SECS {
        return Err("checkpoint is stale; save a fresh checkpoint".to_string());
    }
    Ok(())
}

fn migration_nonce() -> Result<[u8; MIGRATION_NONCE_BYTES], String> {
    #[cfg(unix)]
    {
        let mut nonce = [0u8; MIGRATION_NONCE_BYTES];
        let mut random = std::fs::File::open("/dev/urandom")
            .map_err(|error| format!("cannot open host randomness source: {error}"))?;
        random
            .read_exact(&mut nonce)
            .map_err(|error| format!("cannot read host randomness source: {error}"))?;
        return Ok(nonce)
    }

    #[cfg(not(unix))]
    {
        let counter = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("host clock is before the Unix epoch: {error}"))?;
        let mut seed = [0u8; MIGRATION_NONCE_BYTES];
        seed[..8].copy_from_slice(&counter.to_le_bytes());
        seed[8..16].copy_from_slice(&u64::from(std::process::id()).to_le_bytes());
        seed[16..24].copy_from_slice(&now.as_secs().to_le_bytes());
        seed[24..].copy_from_slice(&u64::from(now.subsec_nanos()).to_le_bytes());
        let fallback_key = SnapshotAuthKey::new(seed);
        return Ok(fallback_key.authenticate_parts(&[b"migration nonce", &seed]))
    }
}

fn replay_ledger_path(snapshot: &Path) -> PathBuf {
    let parent = snapshot
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    parent.join(".synos-vm-migration-replay")
}

fn reserve_replay(
    ledger: &Path,
    key_id: [u8; 16],
    checkpoint_id: [u8; 32],
    issued_at: u64,
) -> Result<(), String> {
    let now = now_seconds()?;
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut ledger_file = options
        .open(ledger)
        .map_err(|error| format!("cannot open migration replay ledger: {error}"))?;
    let metadata = ledger_file
        .metadata()
        .map_err(|error| format!("cannot inspect migration replay ledger: {error}"))?;
    if !metadata.is_file() {
        return Err("migration replay ledger must be a regular file".to_string())
    }
    #[cfg(unix)]
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err("migration replay ledger must not be accessible by group or other users".to_string())
    }
    #[cfg(unix)]
    if unsafe { libc::flock(ledger_file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(format!(
            "cannot lock migration replay ledger: {}",
            std::io::Error::last_os_error()
        ))
    }
    let ledger_length = ledger_file
        .metadata()
        .map_err(|error| format!("cannot inspect migration replay ledger: {error}"))?
        .len();
    if ledger_length > MAX_REPLAY_LEDGER_BYTES {
        return Err("migration replay ledger exceeds its byte limit".to_string())
    }
    let mut existing = Vec::new();
    ledger_file
        .read_to_end(&mut existing)
        .map_err(|error| format!("cannot read migration replay ledger: {error}"))?;
    if existing.len() % REPLAY_RECORD_BYTES != 0 {
        return Err("migration replay ledger is corrupt".to_string());
    }
    let mut next = Vec::with_capacity(existing.len() + REPLAY_RECORD_BYTES);
    for record in existing.chunks_exact(REPLAY_RECORD_BYTES) {
        let record_time = u64::from_le_bytes(
            record[..8]
                .try_into()
                .map_err(|_| "migration replay ledger is corrupt".to_string())?,
        );
        if now.saturating_sub(record_time) > MAX_CHECKPOINT_AGE_SECS {
            continue
        }
        if record[8..24] == key_id && record[24..] == checkpoint_id {
            return Err("checkpoint replay detected".to_string());
        }
        next.extend_from_slice(record);
    }
    if next.len() / REPLAY_RECORD_BYTES >= MAX_REPLAY_RECORDS {
        return Err("migration replay ledger is full".to_string());
    }
    next.extend_from_slice(&issued_at.to_le_bytes());
    next.extend_from_slice(&key_id);
    next.extend_from_slice(&checkpoint_id);
    ledger_file
        .seek(SeekFrom::Start(0))
        .and_then(|_| ledger_file.set_len(0))
        .and_then(|_| ledger_file.write_all(&next))
        .and_then(|_| ledger_file.sync_all())
        .map_err(|error| format!("cannot update migration replay ledger: {error}"))
}

fn release_replay(
    ledger: &Path,
    key_id: [u8; 16],
    checkpoint_id: [u8; 32],
) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.create(false).read(true).write(true);
    #[cfg(unix)]
    {
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let mut ledger_file = options
        .open(ledger)
        .map_err(|error| format!("cannot open migration replay ledger for rollback: {error}"))?;
    #[cfg(unix)]
    if unsafe { libc::flock(ledger_file.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(format!(
            "cannot lock migration replay ledger for rollback: {}",
            std::io::Error::last_os_error()
        ))
    }
    let metadata = ledger_file
        .metadata()
        .map_err(|error| format!("cannot inspect migration replay ledger for rollback: {error}"))?;
    if !metadata.is_file() || metadata.len() > MAX_REPLAY_LEDGER_BYTES {
        return Err("migration replay ledger is invalid during rollback".to_string())
    }
    let mut existing = Vec::new();
    ledger_file
        .read_to_end(&mut existing)
        .map_err(|error| format!("cannot read migration replay ledger for rollback: {error}"))?;
    if existing.len() % REPLAY_RECORD_BYTES != 0 {
        return Err("migration replay ledger is corrupt during rollback".to_string())
    }
    let mut next = Vec::with_capacity(existing.len());
    for record in existing.chunks_exact(REPLAY_RECORD_BYTES) {
        if record[8..24] == key_id && record[24..] == checkpoint_id {
            continue
        }
        next.extend_from_slice(record);
    }
    ledger_file
        .seek(SeekFrom::Start(0))
        .and_then(|_| ledger_file.set_len(0))
        .and_then(|_| ledger_file.write_all(&next))
        .and_then(|_| ledger_file.sync_all())
        .map_err(|error| format!("cannot roll back migration replay ledger: {error}"))
}

fn migration_auth_tag(
    key: SnapshotAuthKey,
    schema: SnapshotSchema,
    sender_nonce: &[u8; MIGRATION_NONCE_BYTES],
    receiver_nonce: &[u8; MIGRATION_NONCE_BYTES],
    issued_at: u64,
    checkpoint_id: &[u8; 32],
    length: u64,
    payload: &[u8],
) -> [u8; 32] {
    migration_checkpoint_tag(
        key,
        schema,
        sender_nonce,
        receiver_nonce,
        issued_at,
        checkpoint_id,
        length,
        payload,
    )
}

fn migration_handshake_tag(
    key: SnapshotAuthKey,
    sender_nonce: &[u8; MIGRATION_NONCE_BYTES],
    receiver_nonce: &[u8; MIGRATION_NONCE_BYTES],
    schema: SnapshotSchema,
) -> [u8; 32] {
    let min_version = schema.min_version.to_le_bytes();
    let max_version = schema.max_version.to_le_bytes();
    let features = schema.features.bits().to_le_bytes();
    key.authenticate_parts(&[
        MIGRATION_AUTH_DOMAIN,
        b"handshake",
        &key.key_id(),
        sender_nonce,
        receiver_nonce,
        &min_version,
        &max_version,
        &features,
    ])
}

fn migration_sender_handshake_tag(
    key: SnapshotAuthKey,
    sender_nonce: &[u8; MIGRATION_NONCE_BYTES],
    schema: SnapshotSchema,
) -> [u8; 32] {
    let min_version = schema.min_version.to_le_bytes();
    let max_version = schema.max_version.to_le_bytes();
    let features = schema.features.bits().to_le_bytes();
    key.authenticate_parts(&[
        MIGRATION_AUTH_DOMAIN,
        b"sender-handshake",
        &key.key_id(),
        sender_nonce,
        &min_version,
        &max_version,
        &features,
    ])
}

fn write_migration_u32(stream: &mut TcpStream, value: u32) -> std::io::Result<()> {
    stream.write_all(&value.to_le_bytes())
}

fn read_migration_u32(stream: &mut TcpStream) -> Result<u32, String> {
    let mut bytes = [0u8; 4];
    stream
        .read_exact(&mut bytes)
        .map_err(|error| format!("migration schema read failed: {error}"))?;
    Ok(u32::from_le_bytes(bytes))
}

fn write_migration_schema(stream: &mut TcpStream, schema: SnapshotSchema) -> std::io::Result<()> {
    write_migration_u32(stream, schema.min_version)?;
    write_migration_u32(stream, schema.max_version)?;
    stream.write_all(&schema.features.bits().to_le_bytes())
}

fn read_migration_schema(stream: &mut TcpStream) -> Result<SnapshotSchema, String> {
    let min_version = read_migration_u32(stream)?;
    let max_version = read_migration_u32(stream)?;
    let mut features = [0u8; 8];
    stream
        .read_exact(&mut features)
        .map_err(|error| format!("migration feature read failed: {error}"))?;
    let raw_features = u64::from_le_bytes(features);
    let schema = SnapshotSchema {
        min_version,
        max_version,
        features: SnapshotFeatures::from_bits_retain(raw_features),
    };
    schema
        .validate()
        .map_err(|error| format!("migration peer advertised an invalid schema: {error}"))?;
    Ok(schema)
}

fn read_migration_response(
    stream: &mut TcpStream,
    key: SnapshotAuthKey,
    authorized_peer_key_id: [u8; 16],
    sender_nonce: &[u8; MIGRATION_NONCE_BYTES],
) -> Result<(SnapshotSchema, [u8; MIGRATION_NONCE_BYTES]), String> {
    let mut status = [0u8; 1];
    stream
        .read_exact(&mut status)
        .map_err(|error| format!("migration schema response read failed: {error}"))?;
    if status[0] != 0 {
        return Err("migration target rejected the snapshot schema".to_string());
    }
    let schema = read_migration_schema(stream)?;
    if schema.min_version != schema.max_version {
        return Err("migration target returned a non-negotiated schema range".to_string());
    }
    let mut key_id = [0u8; 16];
    stream
        .read_exact(&mut key_id)
        .map_err(|error| format!("migration key identity response read failed: {error}"))?;
    if key_id != authorized_peer_key_id {
        return Err("migration target is not in the configured authorization policy".to_string());
    }
    let mut receiver_nonce = [0u8; MIGRATION_NONCE_BYTES];
    stream
        .read_exact(&mut receiver_nonce)
        .map_err(|error| format!("migration receiver challenge read failed: {error}"))?;
    let mut response_tag = [0u8; 32];
    stream
        .read_exact(&mut response_tag)
        .map_err(|error| format!("migration handshake tag read failed: {error}"))?;
    let expected_tag = migration_handshake_tag(key, sender_nonce, &receiver_nonce, schema);
    key.verify_tag(&expected_tag, &response_tag)
        .map_err(|error| format!("migration peer authentication failed: {error}"))?;
    Ok((schema, receiver_nonce))
}

struct MonitorSession {
    #[cfg(unix)]
    listener: UnixListener,
    path: PathBuf,
    authenticator: MonitorAuthenticator,
    #[cfg(unix)]
    clients: Vec<MonitorClient>,
}

#[cfg(unix)]
struct MonitorClient {
    stream: std::os::unix::net::UnixStream,
    accepted_at: Instant,
    request: MonitorRequestBuffer,
    response: Option<Vec<u8>>,
    written: usize,
}

#[cfg(unix)]
impl MonitorClient {
    fn new(stream: std::os::unix::net::UnixStream) -> Self {
        Self {
            stream,
            accepted_at: Instant::now(),
            request: MonitorRequestBuffer::new(),
            response: None,
            written: 0,
        }
    }

    fn set_response(&mut self, response: String) {
        self.response = Some(control::bounded_response(response));
        self.written = 0
    }

    fn flush_response(&mut self) -> bool {
        let Some(response) = self.response.as_ref() else {
            return false
        };
        while self.written < response.len() {
            match self.stream.write(&response[self.written..]) {
                Ok(0) => return true,
                Ok(count) => self.written += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return false,
                Err(_) => return true,
            }
        }
        true
    }
}

impl MonitorSession {
    fn bind(
        path: PathBuf,
        auth_key: SnapshotAuthKey,
        permissions: MonitorPermissions,
    ) -> Result<Self, String> {
        #[cfg(unix)]
        {
            if path.exists() {
                return Err(format!("monitor socket {} already exists", path.display()));
            }
            let listener = UnixListener::bind(&path)
                .map_err(|error| format!("cannot create monitor socket {}: {error}", path.display()))?;
            if let Err(error) = std::fs::set_permissions(
                &path,
                std::fs::Permissions::from_mode(0o600),
            ) {
                let _ = std::fs::remove_file(&path);
                return Err(format!("cannot restrict monitor socket permissions: {error}"))
            }
            listener
                .set_nonblocking(true)
                .map_err(|error| format!("cannot configure monitor socket: {error}"))?;
            println!("monitor console listening at {}", path.display());
            Ok(Self {
                listener,
                path,
                authenticator: MonitorAuthenticator::new(auth_key, permissions),
                clients: Vec::new(),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = (path, auth_key, permissions);
            Err("monitor console is only supported on Unix hosts".to_string())
        }
    }

    fn poll(
        &mut self,
        vm: &mut Vm,
        auth_key: Option<&SnapshotAuthKey>,
    ) -> Result<bool, String> {
        #[cfg(unix)]
        {
            self.accept_clients()?;
            let mut keep_running = true;
            let mut index = 0;
            while index < self.clients.len() {
                if self.clients[index].accepted_at.elapsed()
                    >= Duration::from_secs(MONITOR_CONNECTION_LIFETIME_SECS)
                {
                    if self.clients[index].response.is_none() {
                        self.clients[index].set_response(control::failure_response(
                            None,
                            "connection-timeout",
                            "monitor connection exceeded its lifetime",
                        ));
                    }
                    self.clients[index].flush_response();
                    self.clients.swap_remove(index);
                    continue
                }

                if self.clients[index].response.is_some() {
                    if self.clients[index].flush_response() {
                        self.clients.swap_remove(index);
                    } else {
                        index += 1;
                    }
                    continue
                }

                let frame = read_monitor_frame(&mut self.clients[index]);
                let Some(frame) = frame else {
                    index += 1;
                    continue
                };
                let response = match frame {
                    MonitorRequestFrame::Pending => {
                        index += 1;
                        continue
                    }
                    MonitorRequestFrame::Rejected { code, message } => {
                        control::failure_response(None, code, &message)
                    }
                    MonitorRequestFrame::Complete(request) => {
                        match self.authenticator.authenticate(&request) {
                            Ok(command) => {
                                let (command_keep_running, response) =
                                    monitor_command(vm, command, auth_key)?;
                                keep_running &= command_keep_running;
                                response
                            }
                            Err(error) => control::failure_response(
                                error.command.as_deref(),
                                error.code,
                                &error.message,
                            ),
                        }
                    }
                };
                self.clients[index].set_response(response);
                if self.clients[index].flush_response() {
                    self.clients.swap_remove(index);
                } else {
                    index += 1;
                }
            }
            return Ok(keep_running)
        }
        #[cfg(not(unix))]
        {
            let _ = (vm, auth_key);
            Ok(true)
        }
    }

    #[cfg(unix)]
    fn accept_clients(&mut self) -> Result<(), String> {
        while self.clients.len() < MAX_MONITOR_CLIENTS {
            let (stream, _) = match self.listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(format!("monitor accept failed: {error}")),
            };
            stream
                .set_nonblocking(true)
                .map_err(|error| format!("cannot configure monitor client: {error}"))?;
            self.clients.push(MonitorClient::new(stream));
        }
        Ok(())
    }
}

#[cfg(unix)]
fn read_monitor_frame(client: &mut MonitorClient) -> Option<MonitorRequestFrame> {
    let mut buffer = [0u8; 1024];
    loop {
        match client.stream.read(&mut buffer) {
            Ok(0) => return Some(client.request.end_of_stream()),
            Ok(count) => match client.request.push(&buffer[..count]) {
                MonitorRequestFrame::Pending => continue,
                frame => return Some(frame),
            },
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return None,
            Err(error) => return Some(MonitorRequestFrame::Rejected {
                code: "connection-read-failed",
                message: format!("monitor read failed: {error}"),
            }),
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

fn monitor_command(
    vm: &mut Vm,
    command: MonitorCommand,
    auth_key: Option<&SnapshotAuthKey>,
) -> Result<(bool, String), String> {
    match command {
        MonitorCommand::Help => Ok((true, control::help_response())),
        MonitorCommand::Info {
            topic,
            disclose_sensitive,
        } => {
            Ok((true, control::info_response(vm, topic, disclose_sensitive)))
        }
        MonitorCommand::Quit => Ok((false, control::action_response("quit", "stop"))),
        MonitorCommand::SaveSnapshot(path) => {
            let Some(key) = auth_key else {
                return Ok((true, control::failure_response(
                    Some("snapshot-save"),
                    "authentication-required",
                    "monitor snapshot save needs an authentication key",
                )))
            };
            if let Err(_error) = vm.save_authenticated_snapshot(&path, *key) {
                return Ok((true, control::failure_response(
                    Some("snapshot-save"),
                    "snapshot-save-failed",
                    "authenticated snapshot save failed; details redacted",
                )))
            }
            Ok((true, control::action_response("snapshot-save", "save")))
        }
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
            print_disk_inventory(&specs, options.json)
        }
        DiskCommand::Inspect { path, json } => inspect_disk(&path, json),
        DiskCommand::Validate { path, json } => {
            let manifest = SystemDiskProvisioner::validate(&path)
                .map_err(|error| format!("disk validation failed for {}: {error}", path.display()))?;
            if json {
                println!(
                    "{{\"status\":\"valid\",\"path\":{},\"format\":{},\"capacity-bytes\":{},\"generation\":{},\"kernel-bytes\":{},\"initrd-bytes\":{}}}",
                    control::json_string(&canonical_display(&path)?),
                    control::json_string(format_name(manifest.format)),
                    manifest.disk_size,
                    manifest.generation,
                    manifest.layout.kernel_size,
                    manifest.layout.initrd_size,
                );
            } else {
                println!(
                    "valid system disk: path={} format={} capacity={} generation={} kernel={} bytes initrd={} bytes",
                    canonical_display(&path)?,
                    format_name(manifest.format),
                    format_bytes(manifest.disk_size),
                    manifest.generation,
                    manifest.layout.kernel_size,
                    manifest.layout.initrd_size,
                );
            }
            Ok(())
        }
        DiskCommand::Repair { path, json } => repair_disk(&path, json),
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
            json,
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
            if json {
                println!(
                    "{{\"status\":\"provisioned\",\"path\":{},\"format\":{},\"capacity-bytes\":{},\"generation\":{}}}",
                    control::json_string(&canonical_display(&path)?),
                    control::json_string(format_name(manifest.format)),
                    manifest.disk_size,
                    manifest.generation,
                );
            } else {
                println!(
                    "provisioned system disk: path={} format={} capacity={} generation={}",
                    canonical_display(&path)?,
                    format_name(manifest.format),
                    format_bytes(manifest.disk_size),
                    manifest.generation,
                );
            }
            Ok(())
        }
        DiskCommand::Lock { path, verbose, json } => print_lock_status(&path, verbose, json),
        DiskCommand::RecoverLock { path, verbose, json } => {
            let info = DiskImage::recover_stale_lock(&path)
                .map_err(|error| format!("cannot recover lock for {}: {error}", path.display()))?;
            if json {
                println!(
                    "{{\"status\":\"recovered\",\"path\":{}}}",
                    control::json_string(&info.path.display().to_string()),
                );
                return Ok(())
            }
            println!("recovered stale disk lock {}", info.path.display());
            if verbose {
                println!();
                println!("owner:");
                for line in info.owner.lines() {
                    println!("  {line}");
                }
            }
            Ok(())
        }
    }
}

fn vm_error_message(error: &synos_vm::VmError, verbose: bool) -> String {
    match error {
        synos_vm::VmError::Disk(message) if !verbose => {
            let summary = message.lines().next().unwrap_or(message);
            let action = message.lines().find(|line| line.starts_with("  action:"));
            match action {
                Some(action) => format!("{summary}\n\n{}", action.trim_start()),
                None => summary.to_string(),
            }
        }
        synos_vm::VmError::Disk(message) => message.clone(),
        other => format!("{other:?}"),
    }
}

fn print_disk_inventory(specs: &[DiskSpec], json: bool) -> Result<(), String> {
    if json {
        print!("[");
    }
    for (index, spec) in specs.iter().enumerate() {
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
                if json {
                    if index != 0 { print!(","); }
                    print!(
                        "{{\"id\":{},\"role\":{},\"controller\":{},\"format\":{},\"capacity-bytes\":{},\"persistence\":{},\"read-only\":{},\"health\":{},\"guest-id\":{},\"path\":{},\"lock\":{}}}",
                        info.id,
                        control::json_string(role_name(info.role)),
                        control::json_string(controller_name(info.controller)),
                        control::json_string(format_name(info.format)),
                        info.capacity,
                        control::json_string(persistence_name(info.persistence)),
                        info.read_only,
                        control::json_string(&health),
                        info.guest_id,
                        control::json_string(&info.image_path.display().to_string()),
                        control::json_string(&lock_state),
                    );
                } else {
                    println!(
                        "id={} role={} controller={} format={} capacity={} persistence={} read-only={} health={} guest-id={} path={} lock={}",
                        info.id, role_name(info.role), controller_name(info.controller),
                        format_name(info.format), format_bytes(info.capacity),
                        persistence_name(info.persistence), info.read_only, health,
                        info.guest_id, info.image_path.display(), lock_state,
                    );
                }
            }
            Err(error) if json => {
                if index != 0 { print!(","); }
                print!(
                    "{{\"id\":{},\"role\":{},\"path\":{},\"health\":\"invalid\",\"error\":{}}}",
                    spec.id, control::json_string(role_name(spec.role)),
                    control::json_string(&spec.image_path.display().to_string()),
                    control::json_string(&error.to_string()),
                );
            }
            Err(error) => println!("id={} role={} path={} health=invalid ({error})",
                spec.id, role_name(spec.role), spec.image_path.display()),
        }
    }
    if json { println!("]"); }
    Ok(())
}

fn inspect_disk(path: &PathBuf, json: bool) -> Result<(), String> {
    let report = DiskImage::inspect_report(path);
    if json {
        print_inspection_json(&report);
        println!();
    } else {
        print_inspection_report(&report);
    }
    if report.is_healthy() {
        Ok(())
    } else {
        Err(format!("disk inspection found errors in {}", path.display()))
    }
}

fn repair_disk(path: &PathBuf, json: bool) -> Result<(), String> {
    let result = DiskImage::repair(path)
        .map(|report| (report.path, report.actions))
        .or_else(|_| {
            SystemDiskProvisioner::repair(path).map(|report| (report.path, report.actions))
        });

    match result {
        Ok((path, actions)) => {
            if !json {
                if actions.is_empty() {
                    println!("repair: no changes needed path={}", path.display());
                } else {
                    for action in &actions {
                        println!("repair: path={} action={action}", path.display());
                    }
                }
            }
            let report = DiskImage::inspect_report(path);
            if json {
                print!("{{\"status\":\"repaired\",\"actions\":[");
                for (index, action) in actions.iter().enumerate() {
                    if index != 0 { print!(","); }
                    print!("{}", control::json_string(action));
                }
                print!("],\"report\":");
                print_inspection_json(&report);
                println!("}}");
            } else {
                print_inspection_report(&report);
            }
            if report.is_healthy() {
                Ok(())
            } else {
                Err("repair completed but inspection still reports errors".to_string())
            }
        }
        Err(error) => Err(format!("disk repair failed for {}: {error}", path.display())),
    }
}

fn print_inspection_report(report: &synos_vm::DiskInspectionReport) {
    let format = report
        .format
        .map(format_name)
        .unwrap_or("unknown");
    let capacity = report
        .capacity
        .map(format_bytes)
        .unwrap_or_else(|| "unknown".to_string());
    let file_size = report
        .file_size
        .map(format_bytes)
        .unwrap_or_else(|| "unknown".to_string());
    let lock = report
        .lock
        .as_ref()
        .map(|value| format!("present(stale={})", value.stale))
        .unwrap_or_else(|| "absent".to_string());
    println!(
        "path={} format={} capacity={} file-size={} status={} lock={}",
        report.path.display(),
        format,
        capacity,
        file_size,
        if report.is_healthy() { "healthy" } else { "error" },
        lock,
    );
    for finding in &report.findings {
        println!(
            "finding={} severity={} repairable={} message={}",
            finding.code,
            finding_severity_name(finding.severity),
            finding.repairable,
            finding.message.replace('\n', "; "),
        );
    }
    if report.format.is_some() && report.capacity.is_some() {
        let system = match SystemDiskProvisioner::validate(&report.path) {
            Ok(manifest) => format!("installed(generation={})", manifest.generation),
            Err(_) => "not-installed-or-invalid".to_string(),
        };
        println!("system-disk={system}");
    }
}

fn print_inspection_json(report: &synos_vm::DiskInspectionReport) {
    let format = report.format.map(format_name).unwrap_or("unknown");
    let capacity = report.capacity.map_or_else(|| "null".to_string(), |value| value.to_string());
    let file_size = report.file_size.map_or_else(|| "null".to_string(), |value| value.to_string());
    let lock = report.lock.as_ref().map_or_else(|| "null".to_string(), |value| format!(
        "{{\"stale\":{},\"path\":{}}}",
        value.stale,
        control::json_string(&value.path.display().to_string()),
    ));
    print!(
        "{{\"path\":{},\"format\":{},\"capacity-bytes\":{},\"file-size-bytes\":{},\"status\":{},\"lock\":{},\"findings\":[",
        control::json_string(&report.path.display().to_string()),
        control::json_string(format),
        capacity,
        file_size,
        control::json_string(if report.is_healthy() { "healthy" } else { "error" }),
        lock,
    );
    for (index, finding) in report.findings.iter().enumerate() {
        if index != 0 { print!(","); }
        print!(
            "{{\"code\":{},\"severity\":{},\"repairable\":{},\"message\":{}}}",
            control::json_string(&finding.code),
            control::json_string(finding_severity_name(finding.severity)),
            finding.repairable,
            control::json_string(&finding.message),
        );
    }
    print!("]}}");
}

fn print_lock_status(path: &PathBuf, verbose: bool, json: bool) -> Result<(), String> {
    match DiskImage::inspect_lock(path)
        .map_err(|error| format!("cannot inspect lock for {}: {error}", path.display()))?
    {
        Some(info) => {
            if json {
                println!(
                    "{{\"state\":{},\"lock\":{},\"image\":{},\"owner\":{},\"pid\":{},\"started\":{},\"host\":{},\"format\":{}}}",
                    control::json_string(if info.stale { "stale" } else { "active" }),
                    control::json_string(&info.path.display().to_string()),
                    control::json_string(&info.image_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "unknown".into())),
                    control::json_string(info.owner_identity.as_deref().unwrap_or("unknown")),
                    info.pid.map_or_else(|| "null".into(), |pid| pid.to_string()),
                    control::json_string(info.start_time.as_deref().unwrap_or("unknown")),
                    control::json_string(info.host_identity.as_deref().unwrap_or("unknown")),
                    control::json_string(info.format.map(format_name).unwrap_or("unknown")),
                );
                return Ok(())
            }
            if !verbose {
                if info.stale {
                    println!("disk lock: stale");
                    if let Some(image) = info.image_path.as_ref() {
                        println!();
                        println!(
                            "action: ./target/release/synos-vm disk recover-lock {}",
                            image.display(),
                        );
                    }
                } else {
                    println!("disk lock: active");
                    if let Some(pid) = info.pid {
                        println!();
                        println!("action: stop process {pid} before starting another VM");
                    }
                }
                return Ok(());
            }
            println!("disk lock");
            if info.stale {
                println!("  state: stale (owner process is gone)");
            } else {
                println!("  state: active");
            }
            println!("  lock: {}", info.path.display());
            println!(
                "  image: {}",
                info.image_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "unknown".to_string()),
            );
            println!(
                "  owner: {}",
                info.owner_identity.as_deref().unwrap_or("unknown"),
            );
            println!(
                "  pid: {}",
                info.pid.map(|pid| pid.to_string()).unwrap_or_else(|| "unknown".to_string()),
            );
            println!(
                "  started: {}",
                info.start_time.as_deref().unwrap_or("unknown"),
            );
            println!(
                "  host: {}",
                info.host_identity.as_deref().unwrap_or("unknown"),
            );
            println!("  format: {}", info.format.map(format_name).unwrap_or("unknown"));
            println!();
            if info.stale {
                if let Some(image) = info.image_path.as_ref() {
                    println!(
                        "  action: ./target/release/synos-vm disk recover-lock {}",
                        image.display(),
                    );
                }
            } else if let Some(pid) = info.pid {
                println!("  action: stop process {pid} before starting another VM");
            }
        }
        None => {
            if json {
                println!(
                    "{{\"state\":\"absent\",\"lock\":{}}}",
                    control::json_string(&DiskImage::lock_path(path).display().to_string()),
                );
                return Ok(())
            }
            println!("disk lock");
            println!("  state: absent");
            println!("  lock: {}", DiskImage::lock_path(path).display());
        }
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

fn finding_severity_name(severity: synos_vm::DiskFindingSeverity) -> &'static str {
    match severity {
        synos_vm::DiskFindingSeverity::Info => "info",
        synos_vm::DiskFindingSeverity::Warning => "warning",
        synos_vm::DiskFindingSeverity::Error => "error",
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
      --accel <BACKEND>     Host acceleration: software (default), auto, kvm, haxm, hvf, or whpx
      --serial              Enable COM1 serial output (default)
      --no-serial            Disable COM1 serial output
      --serial-port <PORT>   Serial port: com1, com2, or a hex I/O base
      --interactive          Force raw interactive terminal mode
      --non-interactive      Disable raw mode; keep pipe input usable
  -v, --v, -verbose         Show verbose error details
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
      --snapshot-key <PATH>  Raw 32-byte or 64-hex-byte auth key
      --monitor <SOCKET>     Expose a local monitor console socket
      --monitor-auth-key <PATH>
                              Private monitor HMAC key (required with --monitor)
      --monitor-allow <LIST> Allow status, device, disk, migration, save, quit,
                              sensitive, or all (default: status)
      --replay-record <PATH> Record deterministic VM inputs to a trace
      --replay <PATH>        Replay a deterministic VM input trace

Commands:
      --integration          Run SynOS integration checks
  synos-vm disk list [--json] List disk health and guest identities
  synos-vm disk inspect PATH [--json]
                              Inspect an image without modifying it
  synos-vm disk validate PATH [--json]
                              Validate an installed system disk
  synos-vm disk repair PATH [--json]
                              Repair supported redundant image metadata
  synos-vm disk provision PATH --kernel PATH [OPTIONS]
                              Create/install a system disk without booting
  synos-vm disk lock PATH [--json]
                              Diagnose an ownership lock
  synos-vm disk recover-lock PATH [--json]
                              Recover a lock only when its owner is stale
  synos-vm migrate send SNAPSHOT ADDRESS [SECURITY OPTIONS]
                              Send an authenticated checkpoint
  synos-vm migrate receive ADDRESS SNAPSHOT [SECURITY OPTIONS]
                              Receive an authenticated checkpoint

Migration security options:
      --key <PATH>           Shared 32-byte migration authentication key
      --peer-key-id <ID>     Authorized peer key ID (32 hexadecimal digits)
      --audit-log <PATH>     Private durable JSONL security audit log
      --secure-transport     Confirm TCP runs inside TLS, VPN, or SSH protection

Other options:
  -h, --help                Show this help
  -V, --version             Show the version"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::atomic::{AtomicU64, Ordering};

    static MIGRATION_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn migration_test_path(label: &str) -> PathBuf {
        let id = MIGRATION_TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "synos-vm-migration-{label}-{}-{id}.bin",
            std::process::id()
        ))
    }

    fn migration_fixture() -> (
        SnapshotAuthKey,
        SnapshotSchema,
        [u8; MIGRATION_NONCE_BYTES],
        [u8; MIGRATION_NONCE_BYTES],
        synos_vm::VmSnapshot,
        Vec<u8>,
    ) {
        let key = SnapshotAuthKey::new([0x42; 32]);
        let schema = SnapshotSchema::for_version(2);
        let sender_nonce = [0x11; MIGRATION_NONCE_BYTES];
        let receiver_nonce = [0x22; MIGRATION_NONCE_BYTES];
        let mut vm = Vm::with_config(VmConfig {
            memory_size: 4 * 1024 * 1024,
            ..VmConfig::default()
        });
        vm.mmu_mut().write_byte(0x2000, 0xA5).expect("fixture memory");
        let snapshot = vm.snapshot();
        let bytes = snapshot
            .to_authenticated_bytes(key)
            .expect("authenticate fixture");
        (key, schema, sender_nonce, receiver_nonce, snapshot, bytes)
    }

    fn migration_wire(
        key: SnapshotAuthKey,
        schema: SnapshotSchema,
        sender_nonce: &[u8; MIGRATION_NONCE_BYTES],
        receiver_nonce: &[u8; MIGRATION_NONCE_BYTES],
        issued_at: u64,
        checkpoint_id: [u8; 32],
        length: u64,
        bytes: &[u8],
    ) -> Vec<u8> {
        let tag = migration_auth_tag(
            key,
            schema,
            sender_nonce,
            receiver_nonce,
            issued_at,
            &checkpoint_id,
            length,
            bytes,
        );
        let mut frame = Vec::new();
        frame.extend_from_slice(&issued_at.to_le_bytes());
        frame.extend_from_slice(&checkpoint_id);
        frame.extend_from_slice(&length.to_le_bytes());
        frame.extend_from_slice(bytes);
        frame.extend_from_slice(&tag);
        frame
    }

    fn read_test_migration_frame(
        wire: &[u8],
        key: SnapshotAuthKey,
        schema: SnapshotSchema,
        sender_nonce: &[u8; MIGRATION_NONCE_BYTES],
        receiver_nonce: &[u8; MIGRATION_NONCE_BYTES],
    ) -> Result<synos_vm::MigrationCheckpointFrame, String> {
        read_migration_checkpoint(
            &mut Cursor::new(wire),
            key,
            schema,
            sender_nonce,
            receiver_nonce,
            |_| Ok(()),
        )
    }

    #[test]
    fn migration_interrupted_transfer_is_rejected() {
        let (key, schema, sender_nonce, receiver_nonce, _snapshot, bytes) = migration_fixture();
        let checkpoint_id = snapshot_digest(&bytes);
        let mut wire = migration_wire(
            key,
            schema,
            &sender_nonce,
            &receiver_nonce,
            1,
            checkpoint_id,
            bytes.len() as u64,
            &bytes,
        );
        wire.truncate(48 + bytes.len() / 2);
        let error = read_test_migration_frame(
            &wire,
            key,
            schema,
            &sender_nonce,
            &receiver_nonce,
        )
        .expect_err("truncated migration must fail");
        assert!(error.contains("migration payload read failed"));
    }

    #[test]
    fn migration_malicious_length_is_rejected_before_allocation() {
        let (key, schema, sender_nonce, receiver_nonce, _snapshot, _bytes) = migration_fixture();
        let wire = migration_wire(
            key,
            schema,
            &sender_nonce,
            &receiver_nonce,
            1,
            [0; 32],
            MAX_MIGRATION_ALLOCATION_BYTES + 1,
            &[],
        );
        let error = read_test_migration_frame(
            &wire,
            key,
            schema,
            &sender_nonce,
            &receiver_nonce,
        )
        .expect_err("oversized migration must fail");
        assert!(error.contains("migration payload is too large"));
    }

    #[test]
    fn migration_corrupt_pages_are_rejected_by_checkpoint_identity() {
        let (key, schema, sender_nonce, receiver_nonce, _snapshot, bytes) = migration_fixture();
        let mut corrupted = bytes.clone();
        corrupted[412] ^= 1;
        let wire = migration_wire(
            key,
            schema,
            &sender_nonce,
            &receiver_nonce,
            1,
            snapshot_digest(&bytes),
            corrupted.len() as u64,
            &corrupted,
        );
        let error = read_test_migration_frame(
            &wire,
            key,
            schema,
            &sender_nonce,
            &receiver_nonce,
        )
        .expect_err("corrupt migration pages must fail");
        assert!(error.contains("identity does not match"));
    }

    #[test]
    fn migration_wrong_vm_topology_is_rejected() {
        let (_key, _schema, _sender_nonce, _receiver_nonce, snapshot, _bytes) =
            migration_fixture();
        let mut incompatible = Vm::with_config(VmConfig {
            memory_size: 8 * 1024 * 1024,
            ..VmConfig::default()
        });
        assert!(matches!(
            snapshot.restore_into(&mut incompatible),
            Err(synos_vm::snapshot::SnapshotError::IncompatibleMemory { .. })
        ));
    }

    #[test]
    fn migration_incompatible_device_state_is_rejected() {
        let incompatible = SnapshotSchema {
            min_version: 2,
            max_version: 2,
            features: SnapshotFeatures::ALL - SnapshotFeatures::APIC_STATE,
        };
        assert!(matches!(
            SnapshotSchema::negotiate(incompatible),
            Err(synos_vm::snapshot::SnapshotError::MissingFeatures { missing })
                if missing & SnapshotFeatures::APIC_STATE.bits() != 0
        ));
    }

    #[test]
    fn migration_duplicate_delivery_is_rejected() {
        let key = SnapshotAuthKey::new([0x24; 32]);
        let ledger = migration_test_path("replay");
        let checkpoint_id = [0xA7; 32];
        let issued_at = u64::MAX;
        reserve_replay(&ledger, key.key_id(), checkpoint_id, issued_at)
            .expect("first delivery is accepted");
        let error = reserve_replay(&ledger, key.key_id(), checkpoint_id, issued_at)
            .expect_err("duplicate delivery must fail");
        assert!(error.contains("checkpoint replay detected"));
        let _ = fs::remove_file(ledger);
    }

    #[test]
    fn migration_destination_crash_keeps_old_target() {
        let (key, _schema, _sender_nonce, _receiver_nonce, new_snapshot, new_bytes) =
            migration_fixture();
        let mut old_vm = Vm::with_config(VmConfig {
            memory_size: 4 * 1024 * 1024,
            ..VmConfig::default()
        });
        old_vm.mmu_mut().write_byte(0x2000, 0x5A).expect("old memory");
        let old_snapshot = old_vm.snapshot();
        let old_bytes = old_snapshot
            .to_authenticated_bytes(key)
            .expect("authenticate old fixture");
        let target = migration_test_path("target");
        let partial = migration_test_path("partial");
        fs::write(&target, &old_bytes).expect("write old target");
        fs::write(&partial, &new_bytes).expect("write staged checkpoint");

        let error = publish_checkpoint_file(
            &partial,
            &target,
            target.parent().expect("target parent"),
            key,
            &old_snapshot,
        )
        .expect_err("post-publish verification must fail for mismatched state");
        assert!(error.contains("changed during publication"));
        assert_eq!(fs::read(&target).expect("read restored target"), old_bytes);
        assert_ne!(new_snapshot, old_snapshot);
        let _ = fs::remove_file(target);
        let _ = fs::remove_file(partial);
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
        assert_eq!(cli.input_mode, GuestInputMode::Ps2);
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
