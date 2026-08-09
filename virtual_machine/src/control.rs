use std::path::PathBuf;

use synos_vm::{
    DiskController, DiskPersistence, DiskRole, PowerState, SnapshotFeatures, SnapshotSchema, Vm,
};

pub const MIGRATION_PROTOCOL_VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonitorCommand {
    Help,
    Info(MonitorTopic),
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
        match input {
            "help" | "?" => Ok(Self::Help),
            "quit" | "exit" => Ok(Self::Quit),
            "status" | "info status" => Ok(Self::Info(MonitorTopic::Status)),
            "devices" | "info devices" => Ok(Self::Info(MonitorTopic::Devices)),
            "disks" | "info disks" => Ok(Self::Info(MonitorTopic::Disks)),
            "snapshots" | "info snapshots" => Ok(Self::Info(MonitorTopic::Snapshots)),
            "migration" | "info migration" => Ok(Self::Info(MonitorTopic::Migration)),
            "registers" | "info registers" => Ok(Self::Info(MonitorTopic::Registers)),
            _ => {
                if let Some(path) = input.strip_prefix("save ").map(str::trim) {
                    if path.is_empty() {
                        return Err("save needs a snapshot PATH".to_string())
                    }
                    return Ok(Self::SaveSnapshot(PathBuf::from(path)))
                }
                Err(format!("unknown monitor command `{input}`"))
            }
        }
    }
}

pub fn help_response() -> String {
    let commands = [
        "help",
        "info status",
        "info devices",
        "info disks",
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

pub fn error_response(command: Option<&str>, message: &str) -> String {
    failure_response(command, "invalid-command", message)
}

pub fn failure_response(command: Option<&str>, code: &str, message: &str) -> String {
    format!(
        "{{\"ok\":false,\"command\":{},\"error\":{{\"code\":{},\"message\":{}}}}}\n",
        command.map(json_string).unwrap_or_else(|| "null".to_string()),
        json_string(code),
        json_string(message),
    )
}

pub fn action_response(command: &str, action: &str, path: Option<&str>) -> String {
    let path = path
        .map(|value| format!(",\"path\":{}", json_string(value)))
        .unwrap_or_default();
    envelope(
        command,
        &format!("{{\"action\":{},\"completed\":true{path}}}", json_string(action)),
    )
}

pub fn info_response(vm: &Vm, topic: MonitorTopic) -> String {
    match topic {
        MonitorTopic::Status => status_response(vm),
        MonitorTopic::Devices => devices_response(vm),
        MonitorTopic::Disks => disks_response(vm),
        MonitorTopic::Snapshots => snapshots_response(),
        MonitorTopic::Migration => migration_response(),
        MonitorTopic::Registers => registers_response(vm),
    }
}

fn status_response(vm: &Vm) -> String {
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
    let data = format!(
        "{{\"power\":{},\"hardware_acceleration\":{{\"requested\":{},\"host\":{},\"execution\":{},\"fallback\":{},\"supported_features\":[{}],\"limitations\":[{}]}},\"cpus\":{},\"memory_bytes\":{},\"rip\":{}}}",
        json_string(power),
        json_string(&acceleration.requested.to_string()),
        json_string(&acceleration.active.to_string()),
        json_string(&acceleration.execution_backend.to_string()),
        json_string(&acceleration.fallback_behavior.to_string()),
        features,
        limitations,
        vm.config().smp_cores,
        vm.config().memory_size,
        vm.cpu().state.rip,
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

fn disks_response(vm: &Vm) -> String {
    let disks = vm
        .disks()
        .into_iter()
        .map(|disk| {
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

fn json_string(value: &str) -> String {
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

fn format_name(format: synos_vm::DiskFormat) -> &'static str {
    match format {
        synos_vm::DiskFormat::Raw => "raw",
        synos_vm::DiskFormat::Vhd => "vhd",
        synos_vm::DiskFormat::Qcow2 => "qcow2",
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
