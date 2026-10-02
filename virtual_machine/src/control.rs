#[path = "control_native.rs"]
mod native;

use std::path::PathBuf;

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

pub enum MonitorRequestFrame {
    Pending,
    Complete(String),
    Rejected {
        code: &'static str,
        message: String,
    },
}

pub struct MonitorRequestBuffer {
    native: native::Buffer,
}

impl MonitorRequestBuffer {
    pub fn new() -> Self { Self { native: native::Buffer::new() } }
    pub fn push(&mut self, bytes: &[u8]) -> MonitorRequestFrame { self.native.push(bytes) }
    pub fn end_of_stream(&self) -> MonitorRequestFrame { self.native.end_of_stream() }
}

pub fn bounded_response(response: String) -> Vec<u8> {
    if native::response_fits(response.len()) {
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
        native::permissions(value)
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
    seen_nonces: native::Nonces,
}

impl MonitorAuthenticator {
    pub fn new(key: ghostos_vm::SnapshotAuthKey, permissions: MonitorPermissions) -> Self {
        Self {
            key,
            permissions,
            seen_nonces: native::Nonces::new(),
        }
    }

    pub fn authenticate(&mut self, request: &str) -> Result<MonitorCommand, MonitorRequestError> {
        self.seen_nonces.authenticate(self.key, self.permissions, request)
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
        native::parse(input)
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

pub fn help_response() -> String { native::help() }

pub fn failure_response(command: Option<&str>, code: &str, message: &str) -> String {
    native::failure(command, code, message)
}

pub fn action_response(command: &str, action: &str) -> String { native::action(command, action) }

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
    native::envelope(command, data)
}

pub(crate) fn json_string(value: &str) -> String {
    native::json_string(value)
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
