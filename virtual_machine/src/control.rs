use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ghostos_vm::{
    DiskController, DiskPersistence, DiskRole, PowerState, SnapshotFeatures, SnapshotSchema, Vm,
};

pub const MIGRATION_PROTOCOL_VERSION: u32 = 3;
pub const MONITOR_AUTH_DOMAIN: &[u8] = b"SYNOS-MONITOR-HMAC-SHA256-V1";
pub const MAX_MONITOR_COMMAND_BYTES: usize = 2048;
pub const MAX_MONITOR_REQUEST_BYTES: usize = 4096;
pub const MAX_MONITOR_RESPONSE_BYTES: usize = 64 * 1024;
pub const MAX_MONITOR_CLIENTS: usize = 8;
pub const MONITOR_CONNECTION_LIFETIME_SECS: u64 = 10;
const MONITOR_AUTH_WINDOW_SECS: u64 = 5 * 60;
const MONITOR_NONCE_BYTES: usize = 32;
const MONITOR_TAG_BYTES: usize = 32;
const MAX_SEEN_NONCES: usize = 1024;

pub enum MonitorRequestFrame {
    Pending,
    Complete(String),
    Rejected {
        code: &'static str,
        message: String,
    },
}

pub struct MonitorRequestBuffer {
    bytes: Vec<u8>,
}

impl MonitorRequestBuffer {
    pub fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub fn push(&mut self, bytes: &[u8]) -> MonitorRequestFrame {
        if self.bytes.len().saturating_add(bytes.len()) > MAX_MONITOR_REQUEST_BYTES {
            return MonitorRequestFrame::Rejected {
                code: "request-too-large",
                message: format!(
                    "monitor request exceeds the {MAX_MONITOR_REQUEST_BYTES} byte limit"
                ),
            }
        }
        self.bytes.extend_from_slice(bytes);
        self.frame(false)
    }

    pub fn end_of_stream(&self) -> MonitorRequestFrame {
        self.frame(true)
    }

    fn frame(&self, eof: bool) -> MonitorRequestFrame {
        let Some(newline) = self.bytes.iter().position(|byte| *byte == b'\n') else {
            return if eof {
                MonitorRequestFrame::Rejected {
                    code: "partial-command",
                    message: "monitor connection closed before a complete command line".to_string(),
                }
            } else {
                MonitorRequestFrame::Pending
            }
        };
        if self.bytes[newline + 1..]
            .iter()
            .any(|byte| !byte.is_ascii_whitespace())
        {
            return MonitorRequestFrame::Rejected {
                code: "multiple-commands",
                message: "monitor connection accepts exactly one command".to_string(),
            }
        }
        let mut command = &self.bytes[..newline];
        if command.last() == Some(&b'\r') {
            command = &command[..command.len() - 1]
        }
        match std::str::from_utf8(command) {
            Ok(command) => MonitorRequestFrame::Complete(command.to_string()),
            Err(_) => MonitorRequestFrame::Rejected {
                code: "invalid-encoding",
                message: "monitor command is not valid UTF-8".to_string(),
            },
        }
    }
}

pub fn bounded_response(response: String) -> Vec<u8> {
    if response.len() <= MAX_MONITOR_RESPONSE_BYTES {
        return response.into_bytes()
    }
    failure_response(
        None,
        "response-too-large",
        &format!("monitor response exceeds the {MAX_MONITOR_RESPONSE_BYTES} byte limit"),
    )
    .into_bytes()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorPermission {
    Status,
    Device,
    Disk,
    Migration,
    Save,
    Quit,
    Sensitive,
}

impl MonitorPermission {
    const fn bit(self) -> u8 {
        match self {
            Self::Status => 1 << 0,
            Self::Device => 1 << 1,
            Self::Disk => 1 << 2,
            Self::Migration => 1 << 3,
            Self::Save => 1 << 4,
            Self::Quit => 1 << 5,
            Self::Sensitive => 1 << 6,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Device => "device",
            Self::Disk => "disk",
            Self::Migration => "migration",
            Self::Save => "save",
            Self::Quit => "quit",
            Self::Sensitive => "sensitive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorPermissions(u8);

impl MonitorPermissions {
    pub const fn status_only() -> Self {
        Self(MonitorPermission::Status.bit())
    }

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn allows(self, permission: MonitorPermission) -> bool {
        self.0 & permission.bit() != 0
    }

    pub fn insert(&mut self, permission: MonitorPermission) {
        self.0 |= permission.bit()
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        let mut permissions = Self::empty();
        for name in value.split(',').map(str::trim).filter(|name| !name.is_empty()) {
            if name == "all" {
                return Ok(Self(u8::MAX))
            }
            let permission = match name {
                "status" => MonitorPermission::Status,
                "device" | "devices" => MonitorPermission::Device,
                "disk" | "disks" => MonitorPermission::Disk,
                "migration" => MonitorPermission::Migration,
                "save" | "snapshot-save" => MonitorPermission::Save,
                "quit" | "stop" => MonitorPermission::Quit,
                "sensitive" | "diagnostics" => MonitorPermission::Sensitive,
                _ => return Err(format!(
                    "invalid monitor permission `{name}`; use status, device, disk, migration, save, quit, sensitive, or all"
                )),
            };
            permissions.insert(permission)
        }
        if permissions == Self::empty() {
            return Err("monitor permission list cannot be empty".to_string())
        }
        Ok(permissions)
    }
}

pub struct MonitorRequestError {
    pub code: &'static str,
    pub command: Option<String>,
    pub message: String,
}

pub struct MonitorAuthenticator {
    key: ghostos_vm::SnapshotAuthKey,
    permissions: MonitorPermissions,
    seen_nonces: VecDeque<[u8; MONITOR_NONCE_BYTES]>,
}

impl MonitorAuthenticator {
    pub fn new(key: ghostos_vm::SnapshotAuthKey, permissions: MonitorPermissions) -> Self {
        Self {
            key,
            permissions,
            seen_nonces: VecDeque::new(),
        }
    }

    pub fn authenticate(&mut self, request: &str) -> Result<MonitorCommand, MonitorRequestError> {
        let (scheme, rest) = take_word(request).ok_or_else(|| auth_error("authentication-required", "monitor request needs authentication"))?;
        if scheme != "auth" {
            return Err(auth_error("authentication-required", "monitor request must start with `auth`"))
        }
        let (timestamp, rest) = take_word(rest).ok_or_else(|| auth_error("invalid-authentication", "authenticated request needs a timestamp"))?;
        let (nonce, rest) = take_word(rest).ok_or_else(|| auth_error("invalid-authentication", "authenticated request needs a nonce"))?;
        let (tag, command) = take_word(rest).ok_or_else(|| auth_error("invalid-authentication", "authenticated request needs a tag and command"))?;
        let command = command.trim();
        if command.is_empty() {
            return Err(auth_error("invalid-authentication", "authenticated request needs a command"))
        }

        let timestamp = timestamp.parse::<u64>()
            .map_err(|_| auth_error("invalid-authentication", "monitor timestamp is invalid"))?;
        validate_timestamp(timestamp)?;
        let nonce = decode_hex::<MONITOR_NONCE_BYTES>(nonce, "monitor nonce")?;
        let tag = decode_hex::<MONITOR_TAG_BYTES>(tag, "monitor authentication tag")?;
        if self.seen_nonces.contains(&nonce) {
            return Err(auth_error("authentication-replay", "monitor nonce was already used"))
        }
        let timestamp_bytes = timestamp.to_le_bytes();
        self.key
            .verify_parts(&[MONITOR_AUTH_DOMAIN, &timestamp_bytes, &nonce, command.as_bytes()], &tag)
            .map_err(|_| auth_error("authentication-failed", "monitor authentication failed"))?;

        let parsed = MonitorCommand::parse(command).map_err(|message| MonitorRequestError {
            code: "invalid-command",
            command: Some(command.to_string()),
            message,
        })?;
        let permission = parsed.permission();
        if !self.permissions.allows(permission) {
            return Err(MonitorRequestError {
                code: "permission-denied",
                command: Some(parsed.name().to_string()),
                message: format!("monitor client lacks `{}` permission", permission.name()),
            })
        }
        if parsed.requests_sensitive_data()
            && !self.permissions.allows(MonitorPermission::Sensitive)
        {
            return Err(MonitorRequestError {
                code: "permission-denied",
                command: Some(parsed.name().to_string()),
                message: "monitor client lacks `sensitive` permission".to_string(),
            })
        }
        if self.seen_nonces.len() == MAX_SEEN_NONCES {
            self.seen_nonces.pop_front();
        }
        self.seen_nonces.push_back(nonce);
        Ok(parsed)
    }

}

fn validate_timestamp(timestamp: u64) -> Result<(), MonitorRequestError> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| auth_error("authentication-failed", "host clock is before the Unix epoch"))?
        .as_secs();
    if now.abs_diff(timestamp) > MONITOR_AUTH_WINDOW_SECS {
        return Err(auth_error("authentication-expired", "monitor request timestamp is outside the five-minute window"))
    }
    Ok(())
}

fn take_word(value: &str) -> Option<(&str, &str)> {
    let value = value.trim_start();
    if value.is_empty() {
        return None
    }
    match value.find(char::is_whitespace) {
        Some(index) => Some((&value[..index], &value[index..])),
        None => Some((value, "")),
    }
}

fn decode_hex<const N: usize>(value: &str, name: &str) -> Result<[u8; N], MonitorRequestError> {
    if value.len() != N * 2 {
        return Err(auth_error("invalid-authentication", &format!("{name} must be {} hexadecimal characters", N * 2)))
    }
    let mut output = [0; N];
    for (index, byte) in output.iter_mut().enumerate() {
        let high = hex_value(value.as_bytes()[index * 2]);
        let low = hex_value(value.as_bytes()[index * 2 + 1]);
        let (Some(high), Some(low)) = (high, low) else {
            return Err(auth_error("invalid-authentication", &format!("{name} is not hexadecimal")))
        };
        *byte = (high << 4) | low;
    }
    Ok(output)
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn auth_error(code: &'static str, message: &str) -> MonitorRequestError {
    MonitorRequestError {
        code,
        command: None,
        message: message.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitorCommand {
    Help,
    Info {
        topic: MonitorTopic,
        disclose_sensitive: bool,
    },
    SaveSnapshot(PathBuf),
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorTopic {
    Status,
    Devices,
    Disks,
    Snapshots,
    Migration,
    Registers,
}

impl MonitorCommand {
    pub fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();
        if input.len() > MAX_MONITOR_COMMAND_BYTES {
            return Err(format!(
                "monitor command exceeds the {MAX_MONITOR_COMMAND_BYTES} byte limit"
            ))
        }
        match input {
            "help" | "?" => Ok(Self::Help),
            "quit" | "exit" => Ok(Self::Quit),
            "status" | "info status" => Ok(Self::info(MonitorTopic::Status, false)),
            "info status --show-sensitive" => Ok(Self::info(MonitorTopic::Status, true)),
            "devices" | "info devices" => Ok(Self::info(MonitorTopic::Devices, false)),
            "disks" | "info disks" => Ok(Self::info(MonitorTopic::Disks, false)),
            "info disks --show-sensitive" => Ok(Self::info(MonitorTopic::Disks, true)),
            "snapshots" | "info snapshots" => Ok(Self::info(MonitorTopic::Snapshots, false)),
            "migration" | "info migration" => Ok(Self::info(MonitorTopic::Migration, false)),
            "registers" | "info registers" => Ok(Self::info(MonitorTopic::Registers, true)),
            _ => {
                if let Some(path) = input.strip_prefix("save ").map(str::trim) {
                    if path.is_empty() {
                        return Err("save needs a snapshot PATH".to_string())
                    }
                    return Ok(Self::SaveSnapshot(PathBuf::from(path)))
                }
                Err("unknown monitor command".to_string())
            }
        }
    }

    const fn info(topic: MonitorTopic, disclose_sensitive: bool) -> Self {
        Self::Info {
            topic,
            disclose_sensitive,
        }
    }

    pub const fn permission(&self) -> MonitorPermission {
        match self {
            Self::Help
            | Self::Info {
                topic: MonitorTopic::Status | MonitorTopic::Snapshots | MonitorTopic::Registers,
                ..
            } => MonitorPermission::Status,
            Self::Info { topic: MonitorTopic::Devices, .. } => MonitorPermission::Device,
            Self::Info { topic: MonitorTopic::Disks, .. } => MonitorPermission::Disk,
            Self::Info { topic: MonitorTopic::Migration, .. } => MonitorPermission::Migration,
            Self::SaveSnapshot(_) => MonitorPermission::Save,
            Self::Quit => MonitorPermission::Quit,
        }
    }

    pub const fn requests_sensitive_data(&self) -> bool {
        match self {
            Self::Info {
                disclose_sensitive,
                ..
            } => *disclose_sensitive,
            _ => false,
        }
    }

    pub const fn name(&self) -> &'static str {
        match self {
            Self::Help => "help",
            Self::Info { topic: MonitorTopic::Status, .. } => "status",
            Self::Info { topic: MonitorTopic::Devices, .. } => "devices",
            Self::Info { topic: MonitorTopic::Disks, .. } => "disks",
            Self::Info { topic: MonitorTopic::Snapshots, .. } => "snapshots",
            Self::Info { topic: MonitorTopic::Migration, .. } => "migration",
            Self::Info { topic: MonitorTopic::Registers, .. } => "registers",
            Self::SaveSnapshot(_) => "snapshot-save",
            Self::Quit => "quit",
        }
    }
}

pub fn help_response() -> String {
    let commands = [
        "help",
        "info status",
        "info status --show-sensitive",
        "info devices",
        "info disks",
        "info disks --show-sensitive",
        "info snapshots",
        "info migration",
        "info registers",
        "save PATH",
        "quit",
    ];
    let values = commands
        .iter()
        .map(|command| json_string(command))
        .collect::<Vec<_>>()
        .join(",");
    envelope("help", &format!("{{\"commands\":[{values}]}}"))
}

pub fn failure_response(command: Option<&str>, code: &str, message: &str) -> String {
    format!(
        "{{\"ok\":false,\"command\":{},\"error\":{{\"code\":{},\"message\":{}}}}}\n",
        command.map(json_string).unwrap_or_else(|| "null".to_string()),
        json_string(code),
        json_string(message),
    )
}

pub fn action_response(command: &str, action: &str) -> String {
    envelope(
        command,
        &format!("{{\"action\":{},\"completed\":true}}", json_string(action)),
    )
}

pub fn info_response(vm: &Vm, topic: MonitorTopic, disclose_sensitive: bool) -> String {
    match topic {
        MonitorTopic::Status => status_response(vm, disclose_sensitive),
        MonitorTopic::Devices => devices_response(vm),
        MonitorTopic::Disks => disks_response(vm, disclose_sensitive),
        MonitorTopic::Snapshots => snapshots_response(),
        MonitorTopic::Migration => migration_response(),
        MonitorTopic::Registers => registers_response(vm),
    }
}

fn status_response(vm: &Vm, disclose_sensitive: bool) -> String {
    let power = match vm.power_state() {
        PowerState::Running => "running",
        PowerState::Shutdown => "shutdown",
        PowerState::Reboot => "reboot",
    };
    let acceleration = vm.hardware_acceleration();
    let features = acceleration
        .supported_features
        .iter()
        .map(|feature| json_string(&feature.to_string()))
        .collect::<Vec<_>>()
        .join(",");
    let limitations = acceleration
        .limitations
        .iter()
        .map(|limitation| json_string(&limitation.to_string()))
        .collect::<Vec<_>>()
        .join(",");
    let driver_capabilities = vm
        .driver_capabilities()
        .iter()
        .map(|capability| {
            format!(
                "{{\"kind\":{},\"feature\":{},\"available\":{},\"selected\":{},\"fallback\":{},\"semantics\":{}}}",
                json_string(&capability.kind.to_string()),
                json_string(capability.feature),
                capability.available,
                json_string(capability.selected),
                json_string(capability.fallback),
                json_string(capability.semantics),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let guest_data = if disclose_sensitive {
        format!(",\"rip\":{}", vm.cpu().state.rip)
    } else {
        ",\"guest_data_redacted\":true".to_string()
    };
    let data = format!(
        "{{\"power\":{},\"hardware_acceleration\":{{\"requested\":{},\"host\":{},\"execution\":{},\"fallback\":{},\"supported_features\":[{}],\"limitations\":[{}]}},\"driver_capabilities\":[{}],\"cpus\":{},\"memory_bytes\":{}{guest_data}}}",
        json_string(power),
        json_string(&acceleration.requested.to_string()),
        json_string(&acceleration.active.to_string()),
        json_string(&acceleration.execution_backend.to_string()),
        json_string(&acceleration.fallback_behavior.to_string()),
        features,
        limitations,
        driver_capabilities,
        vm.config().smp_cores,
        vm.config().memory_size,
    );
    envelope("status", &data)
}

fn devices_response(vm: &Vm) -> String {
    let mut devices = vec![
        device("pic", "interrupt-controller", "io", "0x20,0xa0", true),
        device("apic", "interrupt-controller", "mmio", "0xfee00000", true),
        device("pit", "timer", "io", "0x40", true),
        device("hpet", "timer", "mmio", "0xfed00000", true),
        device("power", "power", "io", "0x604", true),
        device("ps2", "input", "io", "0x60", true),
        device("pci-host", "bus", "mmio", "0xe0000000", true),
        device("display", "display", "mmio", "0xb8000,0xf0000000", true),
        device("pv-clock", "clock", "msr", "0x4b564d00,0x4b564d01", true),
        device("persistence", "storage", "io", "0x5400", true),
        device("ahci", "storage", "pci", "0000:00:04.0", true),
        device("nvme", "storage", "pci", "0000:00:05.0", true),
        device("e1000", "network", "pci", "0000:00:06.0", true),
        device("virtio-net", "network", "pci", "0000:00:07.0", true),
        device("virtio-blk", "storage", "pci", "0000:00:08.0", true),
        device("virtio-console", "console", "pci", "0000:00:09.0", true),
        device("virtio-rng", "rng", "pci", "0000:00:0a.0", true),
        device("guest-agent", "guest-integration", "mmio", "0xfebf0000", true),
        device("memory-hotplug", "memory", "mmio", "0xfebe0000", true),
    ];
    devices.push(device(
        "serial",
        "console",
        "io",
        &format!("0x{:x}", vm.config().serial_port),
        vm.config().enable_serial,
    ));
    envelope(
        "devices",
        &format!(
            "{{\"count\":{},\"devices\":[{}]}}",
            devices.len(),
            devices.join(",")
        ),
    )
}

fn device(name: &str, kind: &str, bus: &str, address: &str, enabled: bool) -> String {
    format!(
        "{{\"name\":{},\"kind\":{},\"bus\":{},\"address\":{},\"enabled\":{enabled}}}",
        json_string(name),
        json_string(kind),
        json_string(bus),
        json_string(address),
    )
}

fn disks_response(vm: &Vm, disclose_sensitive: bool) -> String {
    let disks = vm
        .disks()
        .into_iter()
        .map(|disk| {
            if disclose_sensitive {
                format!(
                    "{{\"id\":{},\"role\":{},\"controller\":{},\"bus\":{},\"slot\":{},\"format\":{},\"capacity_bytes\":{},\"read_only\":{},\"persistence\":{},\"guest_id\":{},\"path\":{}}}",
                    json_string(&disk.id),
                    json_string(role_name(disk.role)),
                    json_string(controller_name(disk.controller)),
                    disk.bus,
                    disk.slot,
                    json_string(format_name(disk.format)),
                    disk.capacity,
                    disk.read_only,
                    json_string(persistence_name(disk.persistence)),
                    json_string(&disk.guest_id),
                    json_string(&disk.image_path.to_string_lossy()),
                )
            } else {
                format!(
                    "{{\"id\":{},\"role\":{},\"controller\":{},\"read_only\":{},\"image_metadata_redacted\":true,\"host_path_redacted\":true}}",
                    json_string(&disk.id),
                    json_string(role_name(disk.role)),
                    json_string(controller_name(disk.controller)),
                    disk.read_only,
                )
            }
        })
        .collect::<Vec<_>>();
    envelope(
        "disks",
        &format!("{{\"count\":{},\"disks\":[{}]}}", disks.len(), disks.join(",")),
    )
}

fn snapshots_response() -> String {
    let schema = SnapshotSchema::local();
    let data = format!(
        "{{\"schema\":{{\"min_version\":{},\"max_version\":{},\"feature_bits\":{}}},\"authentication_required\":true}}",
        schema.min_version,
        schema.max_version,
        schema.features.bits(),
    );
    envelope("snapshots", &data)
}

fn migration_response() -> String {
    let schema = SnapshotSchema::local();
    let data = format!(
        "{{\"state\":\"idle\",\"protocol_version\":{},\"authenticated\":true,\"schema\":{{\"min_version\":{},\"max_version\":{},\"feature_bits\":{}}},\"supported_feature_bits\":{}}}",
        MIGRATION_PROTOCOL_VERSION,
        schema.min_version,
        schema.max_version,
        schema.features.bits(),
        SnapshotFeatures::ALL.bits(),
    );
    envelope("migration", &data)
}

fn registers_response(vm: &Vm) -> String {
    let state = &vm.cpu().state;
    let data = format!(
        "{{\"rip\":{},\"rax\":{},\"rbx\":{},\"rcx\":{},\"rdx\":{},\"rflags\":{},\"halted\":{}}}",
        state.rip, state.rax, state.rbx, state.rcx, state.rdx, state.rflags, state.halted,
    );
    envelope("registers", &data)
}

fn envelope(command: &str, data: &str) -> String {
    format!(
        "{{\"ok\":true,\"command\":{},\"data\":{data}}}\n",
        json_string(command),
    )
}

pub(crate) fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            value if value <= '\u{1f}' => output.push_str(&format!("\\u{:04x}", value as u32)),
            value => output.push(value),
        }
    }
    output.push('"');
    output
}

fn format_name(format: ghostos_vm::DiskFormat) -> &'static str {
    match format {
        ghostos_vm::DiskFormat::Raw => "raw",
        ghostos_vm::DiskFormat::Vhd => "vhd",
        ghostos_vm::DiskFormat::Qcow2 => "qcow2",
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

fn persistence_name(persistence: DiskPersistence) -> &'static str {
    match persistence {
        DiskPersistence::Persistent => "persistent",
        DiskPersistence::CopyOnWrite => "copy-on-write",
        DiskPersistence::Disposable => "disposable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_monitor_command_waits_for_newline() {
        let mut request = MonitorRequestBuffer::new();
        assert!(matches!(
            request.push(b"auth 1 0000"),
            MonitorRequestFrame::Pending
        ));
        assert!(matches!(
            request.push(b" status\n"),
            MonitorRequestFrame::Complete(command) if command == "auth 1 0000 status"
        ));
    }

    #[test]
    fn closed_partial_monitor_command_is_rejected() {
        let mut request = MonitorRequestBuffer::new();
        assert!(matches!(request.push(b"auth incomplete"), MonitorRequestFrame::Pending));
        assert!(matches!(
            request.end_of_stream(),
            MonitorRequestFrame::Rejected { code: "partial-command", .. }
        ));
    }

    #[test]
    fn oversized_monitor_request_is_rejected() {
        let mut request = MonitorRequestBuffer::new();
        let oversized = vec![b'a'; MAX_MONITOR_REQUEST_BYTES + 1];
        assert!(matches!(
            request.push(&oversized),
            MonitorRequestFrame::Rejected { code: "request-too-large", .. }
        ));
    }

    #[test]
    fn malformed_monitor_framing_and_encoding_are_rejected() {
        let mut multiple = MonitorRequestBuffer::new();
        assert!(matches!(
            multiple.push(b"first\nsecond\n"),
            MonitorRequestFrame::Rejected { code: "multiple-commands", .. }
        ));

        let mut invalid_utf8 = MonitorRequestBuffer::new();
        assert!(matches!(
            invalid_utf8.push(&[0xff, b'\n']),
            MonitorRequestFrame::Rejected { code: "invalid-encoding", .. }
        ));
    }

    #[test]
    fn command_and_response_limits_are_enforced() {
        let command = "x".repeat(MAX_MONITOR_COMMAND_BYTES + 1);
        assert!(MonitorCommand::parse(&command)
            .expect_err("oversized command must fail")
            .contains("byte limit"));

        let response = bounded_response("x".repeat(MAX_MONITOR_RESPONSE_BYTES + 1));
        assert!(response.len() <= MAX_MONITOR_RESPONSE_BYTES);
        assert!(std::str::from_utf8(&response)
            .expect("bounded response is UTF-8")
            .contains("response-too-large"));
    }

    #[test]
    fn diagnostics_snapshots_and_migration_responses_redact_sensitive_state() {
        let mut vm = Vm::with_config(crate::VmConfig::default());
        vm.cpu_mut().set_rip(0xfeed_cafe);

        let status = info_response(&vm, MonitorTopic::Status, false);
        assert!(status.contains("guest_data_redacted"));
        assert!(!status.contains("feedcafe"));
        assert!(!status.contains("\"rip\""));

        let disks = info_response(&vm, MonitorTopic::Disks, false);
        assert!(!disks.contains("host_path"));
        assert!(!disks.contains("guest_id"));

        let snapshots = info_response(&vm, MonitorTopic::Snapshots, false);
        let migration = info_response(&vm, MonitorTopic::Migration, false);
        assert!(snapshots.contains("authentication_required"));
        assert!(migration.contains("authenticated"));
        assert!(!snapshots.contains("secret"));
        assert!(!migration.contains("secret"));
    }

    #[test]
    fn authorized_diagnostics_can_disclose_only_explicit_fields() {
        let mut vm = Vm::with_config(crate::VmConfig::default());
        vm.cpu_mut().set_rip(0xfeed_cafe);
        let status = info_response(&vm, MonitorTopic::Status, true);

        assert!(status.contains("\"rip\":4276996862"));
    }
}
