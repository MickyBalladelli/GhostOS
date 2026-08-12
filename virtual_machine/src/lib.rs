pub mod cpu;
pub mod memory;
pub mod devices;
pub mod firmware;
pub mod boot;
pub mod cluster;
pub mod net;
pub mod integration;
pub mod execution;
pub mod hardware_acceleration;
pub mod snapshot;
pub mod terminal;
pub mod input;
pub mod clock;
pub mod replay;
pub mod migration;

pub const GUEST_ABI_SCHEMA_VERSION: u16 = synos_abi::ABI_SCHEMA_VERSION;
const INTERACTIVE_STEP_BUDGET: usize = 4_096;

pub use cpu::{Cpu, CpuState, CpuMode, PrivilegeLevel, CpuError};
pub use memory::{LargePageSize, MemoryError, MemoryStats, Mmu, PageFlags, PAGE_SIZE};
pub use net::{
    DhcpConfigError, DhcpLeaseInfo, DhcpReservation, DhcpServerConfig, DeterministicDhcpServer,
    DeterministicVmNetwork,
    DeterministicPort, DeterministicSegment, HostNetworkBackend, LoopbackHub, LoopbackPort,
    MacAddress, NetBackend, NetError, NetQueueState, NetworkBackendConfig, PacketQueue,
    DHCP_SERVER_MAC,
};
pub use devices::{
    Ahci, ApicTrigger, Device, DiskController, DiskFindingSeverity, DiskFormat, DiskImage,
    DiskInfo, DiskInspectionFinding, DiskInspectionReport, DiskLockInfo, DiskManager,
    DiskRepairReport,
    DiskMode, DiskPersistence, DiskRole, DiskSpec, DisplayState, E1000, E1000_MMIO_SIZE,
    GopMode,
    GopPixelFormat, Hpet, InterruptController, LegacyPic, LocalApic, Nvme, PciDeviceId, PciHostBridge, Pit,
    PortBus, PortDevice, PowerControl, PowerNotification, PowerState, Ps2Controller, Serial16550, UefiGop, VesaFbDevice, VgaPorts,
    VgaTextDevice, VideoMode, PS2_DATA_PORT, PS2_PORT_COUNT, PS2_STATUS_PORT,
    GuestAgent, GuestEvent, MemoryHotplugDevice, PvClock,
    GUEST_AGENT_FEATURE_AGENT, GUEST_AGENT_FEATURE_MEMORY_HOTPLUG, GUEST_AGENT_FEATURE_POWER_EVENTS,
    GUEST_AGENT_FEATURE_PV_CLOCK, GUEST_AGENT_IRQ_VECTOR, GUEST_AGENT_MAGIC, GUEST_AGENT_MMIO_BASE,
    GUEST_AGENT_MMIO_SIZE, GUEST_AGENT_VERSION, KVM_SYSTEM_TIME_NEW, KVM_WALL_CLOCK_NEW,
    MEMORY_HOTPLUG_IRQ_VECTOR, MEMORY_HOTPLUG_MMIO_BASE, MEMORY_HOTPLUG_MMIO_SIZE,
    StorageError, VirtioBlk, VirtioConsole, VirtioNet, VirtioRng,
    SynosPersistencePort,
    SystemDiskBootArtifacts, SystemDiskCreateOptions, SystemDiskInstall, SystemDiskLayout,
    SystemDiskManifest, SystemDiskProvisioner, SystemDiskRepairReport, SystemSetting,
    SYSTEM_DISK_ALIGNMENT,
    SYSTEM_DISK_FORMAT_VERSION,
    SYSTEM_DISK_MANIFEST_SIZE, SYSTEM_DISK_MIN_SIZE, SYSTEM_DISK_PAYLOAD_OFFSET,
    SYSTEM_DISK_SETTINGS_SIZE, SYNFS_SYSTEM_BLOCKS, SYNFS_SYSTEM_VOLUME_SIZE,
    AHCI_ABAR_SIZE, AHCI_CLASS, AHCI_DEVICE_ID, AHCI_PROG_IF, AHCI_SUBCLASS, AHCI_VENDOR_ID,
    APIC_BASE_DEFAULT, APIC_SIZE, HPET_BASE_DEFAULT, HPET_SIZE, IA32_APIC_BASE_MSR,
    NVME_BAR0_SIZE, NVME_CLASS, NVME_DEVICE_ID, NVME_PROG_IF, NVME_SUBCLASS, NVME_VENDOR_ID,
    PCIE_ECAM_BASE_DEFAULT, PIT_CH0_PORT, PIT_PORT_COUNT, VBE_MODES, VESA_FB_SIZE, VESA_LFB_BASE,
    VGA_PORT_BASE, VGA_PORT_COUNT, VGA_TEXT_BASE, VGA_TEXT_SIZE, VIRTIO_BLK_CLASS,
    POWER_CONTROL_PORT,
    VIRTIO_BLK_DEVICE_ID, VIRTIO_BLK_PROG_IF, VIRTIO_BLK_SUBCLASS, VIRTIO_CONSOLE_CLASS,
    VIRTIO_CONSOLE_DEVICE_ID, VIRTIO_CONSOLE_PROG_IF, VIRTIO_CONSOLE_SUBCLASS,
    VIRTIO_PCI_BAR0_SIZE, VIRTIO_PCI_VENDOR_ID, VIRTIO_RNG_CLASS, VIRTIO_RNG_DEVICE_ID,
    VIRTIO_RNG_PROG_IF, VIRTIO_RNG_SUBCLASS,
};
pub use firmware::bios::{Bios, BiosContext};
pub use firmware::uefi::{
    UefiContext, UefiError, UefiState, UEFI_CALL_VECTOR, UEFI_CHILD_IMAGE_BASE, UEFI_IMAGE_BASE,
    UEFI_MEMORY_MAP_BASE, UEFI_MEMORY_MAP_SIZE, UEFI_STACK_TOP, UEFI_TABLES_BASE,
};
pub use firmware::FirmwareMode;
pub use boot::Loader;
pub use boot::{framebuffer_info, LoaderError, KERNEL_LOAD_ADDR};
pub use cluster::{
    ClusterError, ClusterFault, ClusterNode, ClusterNodeId, ClusterNodeState, ClusterPacket,
    ClusterEvidence, ClusterFaultRecord, ClusterHeartbeat, ClusterNetwork, ClusterNetworkConfig,
    ClusterNetworkOutcome, ClusterNetworkTrace, ClusterScaleEvidence, ClusterStatus,
    ClusterWorkload,
    CxlFabricFixture, SharedMemoryDevice, SharedMemoryFixture, SharedMemoryMapping, VmCluster,
};
pub use integration::{run_synos_integration, IntegrationError, SynosIntegrationReport};
pub use execution::{
    BlockProfile, ExecutionEngine, ExecutionEngineConfig, ExecutionStats,
};
pub use hardware_acceleration::{
    HardwareAcceleration, HardwareAccelerationAttempt, HardwareAccelerationError,
    HardwareAccelerationFallback, HardwareAccelerationFeature, HardwareAccelerationHandle,
    HardwareAccelerationLimitation, HardwareAccelerationSession, HardwareAccelerationStatus,
};
pub use snapshot::{
    SnapshotChain, SnapshotDiff, SnapshotError, SnapshotFeatures, SnapshotId, SnapshotPage,
    snapshot_digest, SnapshotAuthKey, SnapshotRestoreReport, SnapshotSchema, VmSnapshot,
    MAX_SNAPSHOT_BYTES,
    MAX_SNAPSHOT_MEMORY_BYTES, SNAPSHOT_AUTH_FORMAT_VERSION, SNAPSHOT_AUTH_KEY_BYTES,
    SNAPSHOT_AUTH_TAG_BYTES, SNAPSHOT_FORMAT_VERSION,
    SNAPSHOT_MIN_FORMAT_VERSION,
};
pub use terminal::{
    translate_input_bytes, TerminalError, TerminalExit, TerminalFailure, TerminalInput,
    TerminalOperation, TerminalResize, TerminalSession, TerminalSessionDiagnostics,
    TerminalTranscript, TerminalTranscriptEvent,
};
pub use input::{
    ascii_to_scancodes, serial_resize_sequence, strip_terminal_resize_responses, GuestInputMode,
};
pub use clock::{HostMonotonicClock, ManualMonotonicClock, MonotonicClock, SharedMonotonicClock};
pub use replay::{
    shared_replay, ReplayDmaWrite, ReplayError, ReplayEvent, ReplayEventKind, ReplayHostInput,
    ReplayMode, ReplaySession, ReplayTrace, SharedReplay,
};
pub use migration::{
    decode_migration_checkpoint, migration_checkpoint_tag, validate_migration_checkpoint,
    MigrationCheckpointFrame, MigrationFrameError, MIGRATION_NONCE_BYTES,
    MAX_MIGRATION_ALLOCATION_BYTES,
};

/// Compatibility alias. Guest routing is no longer part of terminal policy.
pub type TerminalInputMode = GuestInputMode;

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fmt;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
use replay::ReplayMode as VmReplayMode;
use synos_boot_protocol::{
    BootMethod, FramebufferInfo, SYNOS_PERSISTENCE_PORT, SYNOS_PERSISTENCE_PORT_SIZE,
};

pub const COM1_PORT: u16 = 0x3F8;
pub const COM2_PORT: u16 = 0x2F8;

/// Write one host VM log line with the same CRLF convention as guest serial.
pub fn host_println(args: fmt::Arguments<'_>) {
    let mut stdout = std::io::stdout().lock();
    let _ = stdout.write_fmt(args);
    let _ = stdout.write_all(b"\r\n");
    let _ = stdout.flush();
}

pub const PCI_CONFIG_PORT: u16 = 0xCF8;
pub const PCI_CONFIG_PORT_SIZE: u16 = 8;

/// Size of the ECAM (MMCONFIG) aperture: 256 buses * 32 devices * 8 funcs * 4 KiB.
pub const PCIE_ECAM_SIZE: u64 = 256 * 32 * 8 * 4096;

/// AHCI controller ABAR (BAR5) aperture base.
pub const AHCI_MMIO_BASE: u64 = 0xF100_0000;
/// NVMe controller BAR0 aperture base.
pub const NVME_MMIO_BASE: u64 = 0xF110_0000;
pub const E1000_MMIO_BASE: u64 = 0xF120_0000;
pub const VIRTIO_NET_IO_BASE: u16 = 0x5000;
pub const VIRTIO_BLK_IO_BASE: u16 = 0x5100;
pub const VIRTIO_CONSOLE_IO_BASE: u16 = 0x5200;
pub const VIRTIO_RNG_IO_BASE: u16 = 0x5300;

const HOST_INPUT_SERIAL: u64 = 1;
const HOST_INPUT_KEYBOARD: u64 = 2;
const HOST_INPUT_MOUSE: u64 = 3;
const HOST_INPUT_TERMINAL_PS2: u64 = 4;

#[derive(Clone)]
pub struct VmConfig {
    pub memory_size: usize,
    pub max_memory_size: usize,
    pub kernel_path: Option<PathBuf>,
    pub initrd_path: Option<PathBuf>,
    pub boot_args: String,
    pub smp_cores: usize,
    pub enable_serial: bool,
    pub serial_port: u16,
    pub firmware: FirmwareMode,
    pub max_steps: Option<u64>,
    pub hardware_acceleration: HardwareAcceleration,
    pub disks: Vec<DiskSpec>,
    pub network: NetworkBackendConfig,
    pub dhcp_server: Option<DhcpServerConfig>,
}

impl Default for VmConfig {
    fn default() -> Self {
        Self {
            memory_size: 128 * 1024 * 1024,
            max_memory_size: 1024 * 1024 * 1024,
            kernel_path: None,
            initrd_path: None,
            boot_args: String::new(),
            smp_cores: 1,
            enable_serial: true,
            serial_port: COM1_PORT,
            firmware: FirmwareMode::Bios,
            max_steps: None,
            hardware_acceleration: HardwareAcceleration::Software,
            disks: Vec::new(),
            network: NetworkBackendConfig::default(),
            dhcp_server: Some(DhcpServerConfig::default()),
        }
    }
}

pub struct Vm {
    cpu: Cpu,
    mmu: Mmu,
    interrupt_controller: InterruptController,
    ports: PortBus,
    serial: Option<Rc<RefCell<Serial16550>>>,
    power_state: Rc<RefCell<PowerState>>,
    power_notifications: Rc<RefCell<VecDeque<PowerNotification>>>,
    guest_power_notifications: Rc<RefCell<VecDeque<PowerNotification>>>,
    guest_agent: Rc<RefCell<GuestAgent>>,
    pv_clock: Rc<RefCell<PvClock>>,
    memory_hotplug: Rc<RefCell<MemoryHotplugDevice>>,
    ps2: Rc<RefCell<Ps2Controller>>,
    pci: Rc<RefCell<PciHostBridge>>,
    apic: Rc<RefCell<LocalApic>>,
    pit: Rc<RefCell<Pit>>,
    hpet: Rc<RefCell<Hpet>>,
    ahci: Rc<RefCell<Ahci>>,
    nvme: Rc<RefCell<Nvme>>,
    e1000: Rc<RefCell<E1000>>,
    virtio_net: Rc<RefCell<VirtioNet>>,
    last_network_error: Option<NetError>,
    dhcp_network: Option<Rc<RefCell<DeterministicVmNetwork>>>,
    virtio_blk: Rc<RefCell<VirtioBlk>>,
    virtio_console: Rc<RefCell<VirtioConsole>>,
    virtio_rng: Rc<RefCell<VirtioRng>>,
    persistence: Rc<RefCell<SynosPersistencePort>>,
    display: Rc<RefCell<DisplayState>>,
    bios: Bios,
    execution: ExecutionEngine,
    hardware_acceleration: HardwareAccelerationSession,
    disk_manager: DiskManager,
    booted_system_disk: Option<SystemDiskBootArtifacts>,
    config: VmConfig,
    clock: SharedMonotonicClock,
    replay: SharedReplay,
    initialized: bool,
}

impl Vm {
    pub fn new() -> Self {
        Self::with_config(VmConfig::default())
    }

    /// Create a default VM driven by `clock`.
    pub fn with_clock(clock: SharedMonotonicClock) -> Self {
        Self::with_config_and_clock(VmConfig::default(), clock)
    }

    pub fn with_config(config: VmConfig) -> Self {
        Self::with_config_and_clock(config, Rc::new(HostMonotonicClock::new()))
    }

    /// Create a VM driven by `clock` instead of the host clock.
    pub fn with_config_and_clock(config: VmConfig, clock: SharedMonotonicClock) -> Self {
        Self::try_with_config_and_clock(config, clock)
            .unwrap_or_else(|error| panic!("cannot create VM: {error:?}"))
    }

    /// Create a VM and attach every configured disk before firmware starts.
    pub fn try_with_config(config: VmConfig) -> Result<Self, VmError> {
        Self::try_with_config_and_clock(config, Rc::new(HostMonotonicClock::new()))
    }

    /// Fallible variant of [`Vm::with_config_and_clock`].
    pub fn try_with_config_and_clock(
        mut config: VmConfig,
        clock: SharedMonotonicClock,
    ) -> Result<Self, VmError> {
        config.max_memory_size = config.max_memory_size.max(config.memory_size);
        DiskManager::validate_specs(&config.disks).map_err(disk_error_to_vm)?;
        let hardware_acceleration = HardwareAccelerationSession::open(config.hardware_acceleration)
            .map_err(|error| VmError::HardwareAcceleration(error.to_string()))?;
        let mut vm = Self::build_with_config(config, clock)?;
        vm.hardware_acceleration = hardware_acceleration;
        vm.attach_configured_disks()?;
        if let Err(error) = vm.configure_persistence() {
            let _ = vm.close_disks();
            return Err(error)
        }
        Ok(vm)
    }

    fn build_with_config(config: VmConfig, clock: SharedMonotonicClock) -> Result<Self, VmError> {
        let mut mmu = Mmu::new(config.memory_size);
        let replay = shared_replay();
        mmu.attach_replay(replay.clone());

        // One shared PCI Express host bridge exposed through both the legacy
        // 0xCF8/0xCFC config ports and the ECAM (MMCONFIG) memory aperture.
        let pci: Rc<RefCell<PciHostBridge>> = Rc::new(RefCell::new(PciHostBridge::new()));

        let apic: Rc<RefCell<LocalApic>> = Rc::new(RefCell::new(LocalApic::new(0)));
        let mut ports = PortBus::new();
        ports.attach_replay(replay.clone());
        ports.attach(0x20, 2, Box::new(LegacyPic::new(apic.clone())));
        ports.attach(0xa0, 2, Box::new(LegacyPic::new(apic.clone())));
        let power_state = Rc::new(RefCell::new(PowerState::Running));
        let power_notifications = Rc::new(RefCell::new(VecDeque::new()));
        let guest_power_notifications = Rc::new(RefCell::new(VecDeque::new()));
        let guest_agent = Rc::new(RefCell::new(GuestAgent::new()));
        guest_agent.borrow().attach_apic(apic.clone());
        let pv_clock = Rc::new(RefCell::new(PvClock::new()));
        pv_clock.borrow_mut().attach_replay(replay.clone());
        let memory_hotplug = Rc::new(RefCell::new(MemoryHotplugDevice::new(
            config.memory_size as u64,
            config.max_memory_size.max(config.memory_size) as u64,
        )));
        memory_hotplug.borrow().attach_apic(apic.clone());
        ports.attach(
            POWER_CONTROL_PORT,
            4,
            Box::new({
                let mut power = PowerControl::new(power_state.clone());
                power.attach_notifications(power_notifications.clone());
                power.attach_notifications(guest_power_notifications.clone());
                power
            }),
        );
        let serial = if config.enable_serial {
            let serial = Rc::new(RefCell::new(Serial16550::new(config.serial_port)));
            serial.borrow_mut().attach_apic(apic.clone());
            serial.borrow_mut().set_irq_vector(0x24);
            ports.attach(config.serial_port, 8, Box::new(serial.clone()));
            Some(serial)
        } else {
            None
        };
        let ps2 = Rc::new(RefCell::new(Ps2Controller::new()));
        ps2.borrow_mut().attach_apic(apic.clone());
        ports.attach(PS2_DATA_PORT, 5, Box::new(ps2.clone()));
        ports.attach(PCI_CONFIG_PORT, PCI_CONFIG_PORT_SIZE, Box::new(pci.clone()));
        let persistence = Rc::new(RefCell::new(SynosPersistencePort::new()));
        ports.attach(
            SYNOS_PERSISTENCE_PORT,
            SYNOS_PERSISTENCE_PORT_SIZE,
            Box::new(persistence.clone()),
        );

        // ECAM aperture is memory-mapped; route it through the MMU so guest
        // loads/stores to the configuration space hit the same bridge.
        mmu.attach_mmio(PCIE_ECAM_BASE_DEFAULT, PCIE_ECAM_SIZE, Box::new(pci.clone()));

        // One local APIC (BSP id 0) shared between the CPU (for the
        // IA32_APIC_BASE MSR path) and the MMU (for the xAPIC MMIO path).
        mmu.attach_mmio(APIC_BASE_DEFAULT, APIC_SIZE, Box::new(apic.clone()));

        // 8254 PIT on the legacy I/O ports 0x40..0x43. Channel 0 maps to
        // ISA IRQ0 which the I/O APIC redirects to APIC vector 0x20 in the
        // default PC-compatible interrupt table.
        let pit: Rc<RefCell<Pit>> = Rc::new(RefCell::new(Pit::new()));
        pit.borrow_mut().attach_apic(apic.clone());
        pit.borrow_mut().set_irq0_vector(0x20);
        ports.attach(PIT_CH0_PORT, PIT_PORT_COUNT, Box::new(pit.clone()));

        // HPET at the ACPI base 0xFED0_0000. Legacy-replacement interrupts
        // from timers 0/1 are routed to APIC vector 0x20 (IRQ0) and 0x21
        // (IRQ1) matching the IA-PC compatibility table.
        let hpet: Rc<RefCell<Hpet>> = Rc::new(RefCell::new(Hpet::new()));
        hpet.borrow_mut().attach_apic(apic.clone());
        hpet.borrow_mut().set_legacy_vector(0x20);
        mmu.attach_mmio(HPET_BASE_DEFAULT, HPET_SIZE, Box::new(hpet.clone()));
        mmu.attach_mmio(
            GUEST_AGENT_MMIO_BASE,
            GUEST_AGENT_MMIO_SIZE,
            Box::new(guest_agent.clone()),
        );
        mmu.attach_mmio(
            MEMORY_HOTPLUG_MMIO_BASE,
            MEMORY_HOTPLUG_MMIO_SIZE,
            Box::new(memory_hotplug.clone()),
        );

        // -------------------------------------------------------------------
        // Storage controllers
        // -------------------------------------------------------------------

        // AHCI SATA HBA at device 4, function 0. BAR5 (ABAR) is memory
        // mapped at AHCI_MMIO_BASE (4 KiB) and interrupts to vector 0x2B
        // (ISA IRQ11 remapped by the I/O APIC).
        let ahci: Rc<RefCell<Ahci>> = Rc::new(RefCell::new(Ahci::new()));
        ahci.borrow_mut().attach_apic(apic.clone());
        ahci.borrow_mut().set_irq_vector(0x2B);
        mmu.attach_mmio(AHCI_MMIO_BASE, AHCI_ABAR_SIZE, Box::new(ahci.clone()));
        pci.borrow_mut().add_device(
            0,
            4,
            0,
            PciDeviceId {
                vendor: AHCI_VENDOR_ID,
                device: AHCI_DEVICE_ID,
                revision: 0x01,
                prog_if: AHCI_PROG_IF,
                subclass: AHCI_SUBCLASS,
                class: AHCI_CLASS,
            },
        );
        pci.borrow_mut().set_bar_size(0, 4, 0, 5, AHCI_ABAR_SIZE as u32).ok();
        pci.borrow_mut().write_config(0, 4, 0, 0x10 + 5 * 4, AHCI_MMIO_BASE as u32);

        // NVMe controller at device 5, function 0. BAR0 is memory mapped at
        // NVME_MMIO_BASE (8 KiB) and interrupts to vector 0x31 (IRQ17).
        let nvme: Rc<RefCell<Nvme>> = Rc::new(RefCell::new(Nvme::new()));
        nvme.borrow_mut().attach_apic(apic.clone());
        nvme.borrow_mut().set_irq_vector(0x31);
        mmu.attach_mmio(NVME_MMIO_BASE, NVME_BAR0_SIZE, Box::new(nvme.clone()));
        pci.borrow_mut().add_device(
            0,
            5,
            0,
            PciDeviceId {
                vendor: NVME_VENDOR_ID,
                device: NVME_DEVICE_ID,
                revision: 0x01,
                prog_if: NVME_PROG_IF,
                subclass: NVME_SUBCLASS,
                class: NVME_CLASS,
            },
        );
        pci.borrow_mut().set_bar_size(0, 5, 0, 0, NVME_BAR0_SIZE as u32).ok();
        pci.borrow_mut().write_config(0, 5, 0, 0x10, NVME_MMIO_BASE as u32);

        // Networking: deterministic segment by default, or a real host
        // transport selected by VmConfig.
        let (
            e1000_mac,
            virtio_mac,
            e1000_backend,
            virtio_backend,
            dhcp_network,
        ) = create_network_backends(
            &config.network,
            config.dhcp_server.as_ref(),
        )?;
        let e1000 =
            Rc::new(RefCell::new(E1000::new(e1000_mac)));
        e1000.borrow_mut().attach_apic(apic.clone());
        e1000.borrow_mut().set_irq_vector(0x2D);
        e1000.borrow_mut().attach_backend(e1000_backend);
        mmu.attach_mmio(E1000_MMIO_BASE, E1000_MMIO_SIZE, Box::new(e1000.clone()));
        pci.borrow_mut().add_device(
            0, 6, 0,
            PciDeviceId {
                vendor: 0x8086, device: 0x100E, revision: 0x03,
                prog_if: 0x00, subclass: 0x00, class: 0x02,
            },
        );
        pci.borrow_mut()
            .set_bar_size(0, 6, 0, 0, E1000_MMIO_SIZE as u32)
            .ok();
        pci.borrow_mut()
            .write_config(0, 6, 0, 0x10, E1000_MMIO_BASE as u32);

        let virtio_net = Rc::new(RefCell::new(VirtioNet::new(virtio_mac)));
        virtio_net.borrow_mut().attach_apic(apic.clone());
        virtio_net.borrow_mut().set_irq_vector(0x2E);
        virtio_net.borrow_mut().attach_backend(virtio_backend);
        ports.attach(
            VIRTIO_NET_IO_BASE,
            VIRTIO_PCI_BAR0_SIZE as u16,
            Box::new(virtio_net.clone()),
        );
        pci.borrow_mut().add_device(
            0, 7, 0,
            PciDeviceId {
                vendor: 0x1AF4, device: 0x1000, revision: 0x01,
                prog_if: 0x00, subclass: 0x00, class: 0x02,
            },
        );
        pci.borrow_mut().set_bar_size(0, 7, 0, 0, 0x100).ok();
        pci.borrow_mut()
            .write_config(0, 7, 0, 0x10, VIRTIO_NET_IO_BASE as u32 | 0x1);

        // The remaining legacy Virtio devices share the same port BAR layout.
        // Each receives its own PCI function and interrupt vector so a guest
        // can use block, console, and entropy services independently.
        let virtio_blk = Rc::new(RefCell::new(VirtioBlk::new()));
        virtio_blk.borrow_mut().attach_apic(apic.clone());
        virtio_blk.borrow_mut().set_irq_vector(0x32);
        ports.attach(
            VIRTIO_BLK_IO_BASE,
            VIRTIO_PCI_BAR0_SIZE as u16,
            Box::new(virtio_blk.clone()),
        );
        pci.borrow_mut().add_device(
            0,
            8,
            0,
            PciDeviceId {
                vendor: VIRTIO_PCI_VENDOR_ID,
                device: VIRTIO_BLK_DEVICE_ID,
                revision: 0x01,
                prog_if: VIRTIO_BLK_PROG_IF,
                subclass: VIRTIO_BLK_SUBCLASS,
                class: VIRTIO_BLK_CLASS,
            },
        );
        pci.borrow_mut()
            .set_bar_size(0, 8, 0, 0, VIRTIO_PCI_BAR0_SIZE as u32)
            .ok();
        pci.borrow_mut()
            .write_config(0, 8, 0, 0x10, VIRTIO_BLK_IO_BASE as u32 | 0x1);

        let virtio_console = Rc::new(RefCell::new(VirtioConsole::new()));
        virtio_console.borrow_mut().attach_apic(apic.clone());
        virtio_console.borrow_mut().set_irq_vector(0x33);
        ports.attach(
            VIRTIO_CONSOLE_IO_BASE,
            VIRTIO_PCI_BAR0_SIZE as u16,
            Box::new(virtio_console.clone()),
        );
        pci.borrow_mut().add_device(
            0,
            9,
            0,
            PciDeviceId {
                vendor: VIRTIO_PCI_VENDOR_ID,
                device: VIRTIO_CONSOLE_DEVICE_ID,
                revision: 0x01,
                prog_if: VIRTIO_CONSOLE_PROG_IF,
                subclass: VIRTIO_CONSOLE_SUBCLASS,
                class: VIRTIO_CONSOLE_CLASS,
            },
        );
        pci.borrow_mut()
            .set_bar_size(0, 9, 0, 0, VIRTIO_PCI_BAR0_SIZE as u32)
            .ok();
        pci.borrow_mut()
            .write_config(0, 9, 0, 0x10, VIRTIO_CONSOLE_IO_BASE as u32 | 0x1);

        let virtio_rng = Rc::new(RefCell::new(VirtioRng::new()));
        virtio_rng.borrow_mut().attach_apic(apic.clone());
        virtio_rng.borrow_mut().set_irq_vector(0x34);
        ports.attach(
            VIRTIO_RNG_IO_BASE,
            VIRTIO_PCI_BAR0_SIZE as u16,
            Box::new(virtio_rng.clone()),
        );
        pci.borrow_mut().add_device(
            0,
            10,
            0,
            PciDeviceId {
                vendor: VIRTIO_PCI_VENDOR_ID,
                device: VIRTIO_RNG_DEVICE_ID,
                revision: 0x01,
                prog_if: VIRTIO_RNG_PROG_IF,
                subclass: VIRTIO_RNG_SUBCLASS,
                class: VIRTIO_RNG_CLASS,
            },
        );
        pci.borrow_mut()
            .set_bar_size(0, 10, 0, 0, VIRTIO_PCI_BAR0_SIZE as u32)
            .ok();
        pci.borrow_mut()
            .write_config(0, 10, 0, 0x10, VIRTIO_RNG_IO_BASE as u32 | 0x1);

        let mut cpu = Cpu::new();
        cpu.attach_apic(apic.clone());
        cpu.attach_pv_clock(pv_clock.clone());

        // Display: shared VGA text buffer + VESA LFB + VGA controller ports.
        let display: Rc<RefCell<DisplayState>> = Rc::new(RefCell::new(DisplayState::new()));
        mmu.attach_mmio(
            VGA_TEXT_BASE,
            VGA_TEXT_SIZE as u64,
            Box::new(VgaTextDevice::new(display.clone())),
        );
        mmu.attach_mmio(
            VESA_LFB_BASE,
            VESA_FB_SIZE as u64,
            Box::new(VesaFbDevice::new(display.clone())),
        );
        ports.attach(
            VGA_PORT_BASE,
            VGA_PORT_COUNT,
            Box::new(VgaPorts::new(display.clone())),
        );

        let mut bios = Bios::new();
        bios.context.set_display(display.clone());
        bios.set_memory_size(config.memory_size);

        // When UEFI firmware is selected, construct a shared UEFI context
        // reachable from both the VM (for init/EFI image config) and the CPU
        // executor (via the INT 0xE0 firmware-call vector routed through the
        // BIOS context).
        if config.firmware == FirmwareMode::Uefi {
            let mut uefi = UefiContext::new();
            uefi.set_display(display.clone());
            uefi.set_memory_size(config.memory_size);
            bios.context.uefi = Some(uefi);
        }
        bios.context.attach_replay(replay.clone());

        Ok(Self {
            cpu,
            mmu,
            interrupt_controller: InterruptController::new(),
            ports,
            serial,
            power_state,
            power_notifications,
            guest_power_notifications,
            guest_agent,
            pv_clock,
            memory_hotplug,
            ps2,
            pci,
            apic,
            pit,
            hpet,
            ahci,
            nvme,
            e1000,
            virtio_net,
            last_network_error: None,
            dhcp_network,
            virtio_blk,
            virtio_console,
            virtio_rng,
            persistence,
            display,
            bios,
            execution: ExecutionEngine::new(),
            hardware_acceleration: HardwareAccelerationSession::open(HardwareAcceleration::Software)
                .expect("software acceleration session cannot fail"),
            disk_manager: DiskManager::new(),
            booted_system_disk: None,
            config,
            clock,
            replay,
            initialized: false,
        })
    }

    /// Set the firmware mode (BIOS or UEFI) before calling `run`.
    pub fn set_firmware_mode(&mut self, mode: FirmwareMode) {
        self.config.firmware = mode;
        if mode == FirmwareMode::Uefi {
            if self.bios.context.uefi.is_none() {
                let mut uefi = UefiContext::new();
                uefi.set_display(self.display.clone());
                uefi.set_memory_size(self.config.memory_size);
                uefi.attach_replay(self.replay.clone());
                self.bios.context.uefi = Some(uefi);
            }
        } else {
            self.bios.context.uefi = None;
        }
    }

    /// Provide an EFI application image (PE32+) for UEFI boot.
    pub fn set_efi_application(&mut self, image: Vec<u8>) {
        if let Some(uefi) = self.bios.context.uefi.as_mut() {
            uefi.set_efi_application(image);
        }
    }

    fn attach_configured_disks(&mut self) -> Result<(), VmError> {
        let mut specs = self.config.disks.clone();
        specs.sort_by(|left, right| left.id.cmp(&right.id));
        for spec in specs {
            self.attach_disk_spec(spec)?;
        }
        Ok(())
    }

    fn configure_persistence(&mut self) -> Result<(), VmError> {
        let disk = self
            .disk_manager
            .list()
            .into_iter()
            .filter(|disk| {
                disk.persistence == DiskPersistence::Persistent && !disk.read_only
            })
            .min_by_key(|disk| if disk.role == DiskRole::Data { 0 } else { 1 });
        let Some(disk) = disk else {
            return Ok(())
        };
        let image = DiskImage::open_with_access(&disk.image_path, true).map_err(disk_error_to_vm)?;
        self.persistence
            .borrow_mut()
            .attach_image(image)
            .map_err(disk_error_to_vm)
    }

    fn attach_disk_spec(&mut self, spec: DiskSpec) -> Result<(), VmError> {
        if self.disk_manager.get(&spec.id).is_some() {
            return Err(VmError::Disk(format!("duplicate disk ID `{}`", spec.id)));
        }
        if self
            .disk_manager
            .list()
            .iter()
            .any(|disk| disk.controller == spec.controller)
        {
            return Err(VmError::Disk(format!(
                "{} already has its only disk slot attached",
                disk_controller_name(spec.controller)
            )));
        }

        let (image, info) = DiskManager::open(&spec).map_err(disk_error_to_vm)?;
        let previous = match spec.controller {
            DiskController::Ahci => self.ahci.borrow_mut().attach_disk(image),
            DiskController::Nvme => self.nvme.borrow_mut().attach_namespace(image),
            DiskController::VirtioBlk => self.virtio_blk.borrow_mut().attach_disk(image),
        };
        if let Some(previous) = previous {
            let _ = previous.close();
            return Err(VmError::Disk(format!(
                "{} disk slot is already occupied",
                disk_controller_name(spec.controller)
            )));
        }
        self.disk_manager.insert(info);
        Ok(())
    }

    /// Attach a disk after VM creation. The disk is immediately visible to
    /// the selected guest controller.
    pub fn attach_disk(&mut self, spec: DiskSpec) -> Result<(), VmError> {
        DiskManager::validate_specs(&[spec.clone()]).map_err(disk_error_to_vm)?;
        if spec.role == DiskRole::System
            && self.disk_manager.list().iter().any(|disk| disk.role == DiskRole::System)
        {
            return Err(VmError::Disk("only one system disk may be attached".into()));
        }
        self.attach_disk_spec(spec.clone())?;
        if !self.persistence.borrow().has_image() {
            self.configure_persistence()?;
        }
        self.config.disks.push(spec);
        Ok(())
    }

    /// Return a deterministic inventory of configured and attached disks.
    pub fn disks(&self) -> Vec<DiskInfo> {
        self.disk_manager.list()
    }

    pub fn flush_disks(&mut self) -> Result<(), VmError> {
        for disk in self.disk_manager.list() {
            let result = match disk.controller {
                DiskController::Ahci => self.ahci.borrow_mut().flush_disk(),
                DiskController::Nvme => self.nvme.borrow_mut().flush_namespace(),
                DiskController::VirtioBlk => self.virtio_blk.borrow_mut().flush_disk(),
            };
            result.map_err(disk_error_to_vm)?;
        }
        self.persistence
            .borrow_mut()
            .sync()
            .map_err(disk_error_to_vm)?;
        Ok(())
    }

    pub fn sync_disks(&mut self) -> Result<(), VmError> {
        for disk in self.disk_manager.list() {
            let result = match disk.controller {
                DiskController::Ahci => self.ahci.borrow_mut().sync_disk(),
                DiskController::Nvme => self.nvme.borrow_mut().sync_namespace(),
                DiskController::VirtioBlk => self.virtio_blk.borrow_mut().sync_disk(),
            };
            result.map_err(disk_error_to_vm)?;
        }
        Ok(())
    }

    /// Flush, release, and close one attached disk.
    pub fn detach_disk(&mut self, id: &str) -> Result<(), VmError> {
        let info = self
            .disk_manager
            .get(id)
            .cloned()
            .ok_or_else(|| VmError::Disk(format!("disk `{id}` is not attached")))?;
        let image = match info.controller {
            DiskController::Ahci => self.ahci.borrow_mut().detach_disk(),
            DiskController::Nvme => self.nvme.borrow_mut().detach_namespace(),
            DiskController::VirtioBlk => self.virtio_blk.borrow_mut().detach_disk(),
        }
        .ok_or_else(|| VmError::Disk(format!("disk `{id}` has no backing image")))?;
        self.disk_manager.remove(id);
        self.config.disks.retain(|spec| spec.id != id);
        image.close().map_err(disk_error_to_vm)
    }

    pub fn close_disks(&mut self) -> Result<(), VmError> {
        self.persistence
            .borrow_mut()
            .sync()
            .map_err(disk_error_to_vm)?;
        let ids: Vec<String> = self.disks().into_iter().map(|disk| disk.id).collect();
        for id in ids {
            self.detach_disk(&id)?;
        }
        Ok(())
    }

    /// Mutable access to the UEFI firmware context (only valid in UEFI mode).
    pub fn uefi(&self) -> Option<&UefiContext> {
        self.bios.context.uefi.as_ref()
    }

    pub fn uefi_mut(&mut self) -> Option<&mut UefiContext> {
        self.bios.context.uefi.as_mut()
    }

    /// Return the validated system disk that supplied the current boot.
    pub fn booted_system_disk(&self) -> Option<&SystemDiskBootArtifacts> {
        self.booted_system_disk.as_ref()
    }

    fn initialize(&mut self) -> Result<(), VmError> {
        if self.initialized {
            return Ok(())
        }
        host_println(format_args!("Initializing VM..."));

        match self.config.firmware {
            FirmwareMode::Bios => {
                self.bios
                    .init(&mut self.mmu, &mut self.cpu.state)
                    .map_err(|_| VmError::BootFailure)?;
            }
            FirmwareMode::Uefi => {
                // UEFI init: build the SystemTable/boot/runtime services,
                // map RAM identity, take the CPU to 64-bit long mode, and
                // start the configured EFI application (if any).
                if let Some(uefi) = self.bios.context.uefi.as_mut() {
                    uefi.init(&mut self.mmu, &mut self.cpu.state)
                        .map_err(|_| VmError::BootFailure)?;
                } else {
                    return Err(VmError::InvalidConfiguration);
                }
            }
        }

        let uefi_application_selected = self
            .bios
            .context
            .uefi
            .as_ref()
            .and_then(|uefi| uefi.efi_application())
            .is_some();
        if !uefi_application_selected
            && (self.config.kernel_path.is_some() || self.configured_system_disk().is_some())
        {
            self.boot_kernel()?;
        }

        self.initialized = true;
        Ok(())
    }

    fn step_cpu(&mut self, max_instructions: usize) -> Result<usize, VmError> {
        self.inject_replay_host_inputs()?;
        if let Some(error) = self.replay.borrow_mut().take_error() {
            return Err(VmError::Replay(error))
        }
        let executed = match self.execution.execute(
            &mut self.cpu,
            &mut self.mmu,
            &mut self.interrupt_controller,
            &mut self.ports,
            &mut self.bios.context,
            max_instructions,
        ) {
            Ok(executed) => executed,
            Err(error) => {
                if error == CpuError::ReplayDivergence {
                    if let Some(replay_error) = self.replay.borrow_mut().take_error() {
                        return Err(VmError::Replay(replay_error))
                    }
                    return Err(VmError::Replay(ReplayError::Corrupt))
                }
                eprintln!("CPU error at RIP 0x{:016x}", self.cpu.state.rip);
                return Err(error.into());
            }
        };

        // Deferred DMA for storage and NICs issued during the step.
        self.poll_devices()?;

        let now_ns = self
            .replay
            .borrow_mut()
            .clock(self.clock.now_ns())
            .map_err(VmError::Replay)?;
        self.poll_guest_features();
        self.pv_clock.borrow_mut().update(&mut self.mmu, now_ns);
        self.poll_apic(now_ns)?;
        Ok(executed)
    }

    fn poll_guest_features(&mut self) {
        loop {
            let request = self.memory_hotplug.borrow().take_request();
            let Some(size) = request else {
                break
            };
            let _ = self.hotplug_memory(size as usize);
        }
        let notifications: Vec<_> = self
            .guest_power_notifications
            .borrow_mut()
            .drain(..)
            .collect();
        for notification in notifications {
            self.guest_agent.borrow().notify(match notification {
                PowerNotification::Shutdown => GuestEvent::Shutdown,
                PowerNotification::Reboot => GuestEvent::Reboot,
            });
        }
    }

    /// Run the VM until the host stops it. Guest HLT instructions wait for
    /// an interrupt while device and timer polling continues.
    pub fn run(&mut self) -> Result<(), VmError> {
        self.run_with_monitor(|_| Ok(true))
    }

    /// Run the VM while polling a host-side monitor between guest batches.
    /// Returning `false` cleanly stops the VM and releases attached disks.
    pub fn run_with_monitor<F>(&mut self, mut monitor: F) -> Result<(), VmError>
    where
        F: FnMut(&mut Self) -> Result<bool, VmError>,
    {
        self.initialize()?;

        host_println(format_args!("Starting CPU emulation..."));

        loop {
            if !monitor(self)? {
                self.close_disks()?;
                return Ok(())
            }
            self.step_cpu(usize::MAX)?;
            self.flush_serial_output();
            self.return_if_guest_panicked()?;

            match self.power_state() {
                PowerState::Running => {}
                PowerState::Shutdown => {
                    self.close_disks()?;
                    return Ok(())
                }
                PowerState::Reboot => {
                    self.sync_disks()?;
                    self.reset();
                    self.initialize()?;
                    continue
                }
            }

            if self.cpu.state.halted {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }

    /// Run with a host terminal attached to the configured serial port.
    /// Input is polled between guest execution batches, so a halted guest
    /// remains alive and can be woken by a serial or PS/2 interrupt.
    pub fn run_with_terminal(
        &mut self,
        terminal: &TerminalSession,
    ) -> Result<TerminalExit, VmError> {
        self.run_with_terminal_mode(terminal, GuestInputMode::Serial)
    }

    pub fn run_with_terminal_mode(
        &mut self,
        terminal: &TerminalSession,
        input_mode: GuestInputMode,
    ) -> Result<TerminalExit, VmError> {
        self.run_with_terminal_mode_and_monitor(terminal, input_mode, |_| Ok(true))
    }

    /// Run with terminal input and a host-side monitor polled between guest
    /// batches. A monitor request to stop is treated like a clean session end.
    pub fn run_with_terminal_mode_and_monitor<F>(
        &mut self,
        terminal: &TerminalSession,
        input_mode: GuestInputMode,
        mut monitor: F,
    ) -> Result<TerminalExit, VmError>
    where
        F: FnMut(&mut Self) -> Result<bool, VmError>,
    {
        self.initialize()?;

        loop {
            if self.replay_mode() == VmReplayMode::Replaying {
                self.inject_replay_host_inputs()?;
            } else {
                let input = terminal
                    .poll_at(self.clock.now_ns())
                    .map_err(|error| VmError::Terminal(error.diagnostic()))?;
                self.record_terminal_input(input, input_mode)?;
            }

            if !monitor(self)? {
                self.close_disks()?;
                return Ok(TerminalExit::GuestShutdown)
            }

            self.step_cpu(INTERACTIVE_STEP_BUDGET)?;
            self.flush_serial_output();
            self.return_if_guest_panicked()?;

            match self.power_state() {
                PowerState::Running => {}
                PowerState::Shutdown => {
                    self.close_disks()?;
                    return Ok(TerminalExit::GuestShutdown)
                }
                PowerState::Reboot => {
                    self.sync_disks()?;
                    self.reset();
                    self.initialize()?;
                    continue
                }
            }

            if self.cpu.state.halted {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }

    /// Run a finite number of guest instructions and return the resulting
    /// CPU position. This is the bounded bring-up entry point for SynOS
    /// integration checks; a kernel that waits for input can be inspected
    /// without leaving a host process running forever.
    pub fn run_for_steps(&mut self, max_steps: u64) -> Result<VmRunReport, VmError> {
        self.run_for_steps_with_monitor(max_steps, |_| Ok(true))
    }

    /// Run a bounded number of instructions while polling a host-side
    /// monitor between guest batches.
    pub fn run_for_steps_with_monitor<F>(
        &mut self,
        max_steps: u64,
        mut monitor: F,
    ) -> Result<VmRunReport, VmError>
    where
        F: FnMut(&mut Self) -> Result<bool, VmError>,
    {
        self.initialize()?;
        let mut steps = 0;
        while steps < max_steps {
            if !monitor(self)? {
                break
            }
            let remaining = (max_steps - steps).min(usize::MAX as u64) as usize;
            let executed = self.step_cpu(remaining)?;
            self.return_if_guest_panicked()?;
            if executed == 0 {
                if self.cpu.state.halted {
                    break;
                }
                continue;
            }
            steps += executed as u64;
        }
        self.sync_disks()?;
        Ok(VmRunReport {
            steps,
            halted: self.cpu.state.halted,
            rip: self.cpu.state.rip,
        })
    }

    fn boot_kernel(&mut self) -> Result<(), VmError> {
        let mut loader = Loader::new();
        let installed = if self.config.kernel_path.is_none() {
            let spec = self
                .configured_system_disk()
                .ok_or(VmError::InvalidConfiguration)?;
            Some(
                SystemDiskProvisioner::load_boot_artifacts(&spec.image_path)
                    .map_err(disk_error_to_vm)?,
            )
        } else {
            None
        };

        if let Some(kernel_path) = self.config.kernel_path.as_ref() {
            loader
                .load_kernel(kernel_path)
                .map_err(loader_error_to_vm)?;
        } else if let Some(artifacts) = installed.as_ref() {
            loader
                .load_kernel_bytes(artifacts.kernel.clone())
                .map_err(loader_error_to_vm)?;
        } else {
            return Err(VmError::InvalidConfiguration);
        }

        if let Some(initrd_path) = self.config.initrd_path.as_ref() {
            loader
                .load_initrd(initrd_path)
                .map_err(loader_error_to_vm)?;
        } else if let Some(initrd) = installed
            .as_ref()
            .and_then(|artifacts| artifacts.initrd.as_ref())
        {
            loader
                .load_initrd_bytes(initrd.clone())
                .map_err(loader_error_to_vm)?;
        }

        let boot_args = if !self.config.boot_args.is_empty() {
            self.config.boot_args.clone()
        } else {
            installed
                .as_ref()
                .and_then(|artifacts| artifacts.setting("boot_args"))
                .or_else(|| {
                    installed
                        .as_ref()
                        .map(|artifacts| artifacts.manifest.boot_args.as_str())
                })
                .unwrap_or_default()
                .to_string()
        };
        loader.set_cmdline(boot_args.clone());
        loader
            .load_to_memory(&mut self.mmu, KERNEL_LOAD_ADDR)
            .map_err(loader_error_to_vm)?;

        let framebuffer = if Self::serial_console_requested(&boot_args) {
            FramebufferInfo::EMPTY
        } else {
            let gop = self.display.borrow().gop();
            framebuffer_info(&gop)
        };
        let method = match self.config.firmware {
            FirmwareMode::Bios => BootMethod::Bios,
            FirmwareMode::Uefi => BootMethod::Uefi,
        };
        loader
            .install_boot_parameters(
                &mut self.mmu,
                self.config.memory_size,
                method,
                framebuffer,
            )
            .map_err(loader_error_to_vm)?;
        loader
            .handoff(&mut self.cpu, &mut self.mmu)
            .map_err(loader_error_to_vm)?;
        self.booted_system_disk = installed;
        Ok(())
    }

    fn configured_system_disk(&self) -> Option<&DiskSpec> {
        self.config
            .disks
            .iter()
            .find(|spec| spec.role == DiskRole::System)
    }

    fn serial_console_requested(boot_args: &str) -> bool {
        boot_args
            .split_ascii_whitespace()
            .any(|argument| matches!(argument, "console=serial0" | "console=ttyS0"))
    }

    /// Process deferred DMA for storage controllers and NICs issued during
    /// the last CPU step. Runs outside the executor's `&mut Mmu` borrow so
    /// devices can DMA directly into guest physical memory.
    fn poll_devices(&mut self) -> Result<(), VmError> {
        self.mmu.begin_dma_capture();
        self.ps2.borrow_mut().poll_interrupt();
        if self.ahci.borrow().has_pending() {
            self.ahci.borrow_mut().poll_dma(&mut self.mmu);
        }
        if self.nvme.borrow().has_pending() {
            self.nvme.borrow_mut().poll_dma(&mut self.mmu);
        }
        self.e1000.borrow_mut().poll(&mut self.mmu);
        let e1000_error = self.e1000.borrow_mut().take_network_error();
        if let Some(error) = e1000_error {
            self.record_network_error(error);
        }
        if self.virtio_net.borrow().has_pending() {
            self.virtio_net.borrow_mut().poll(&mut self.mmu);
        }
        let virtio_error = self.virtio_net.borrow_mut().take_network_error();
        if let Some(error) = virtio_error {
            self.record_network_error(error);
        }
        if let Some(network) = &self.dhcp_network {
            network
                .borrow_mut()
                .poll(self.clock.now_ns() / 1_000_000)
                .map_err(|error| VmError::Network(error.to_string()))?;
        }
        if self.virtio_blk.borrow().has_pending() {
            self.virtio_blk.borrow_mut().poll(&mut self.mmu);
        }
        if self.virtio_console.borrow().has_pending() {
            self.virtio_console.borrow_mut().poll(&mut self.mmu);
        }
        if self.virtio_rng.borrow().has_pending() {
            self.virtio_rng.borrow_mut().poll(&mut self.mmu);
        }
        let writes = self.mmu.take_dma_writes();
        let replay_writes = self
            .replay
            .borrow_mut()
            .device_completion(&writes)
            .map_err(VmError::Replay)?;
        if self.replay_mode() == VmReplayMode::Replaying {
            self.mmu
                .apply_dma_writes(&replay_writes)
                .map_err(|_| VmError::MemoryError)?;
        }
        Ok(())
    }

    /// Advance the local APIC timer plus the PIT/HPET timebase and deliver
    /// the highest-priority pending vector when the CPU can take an interrupt
    /// (IF set, outside the STI shadow window).
    fn poll_apic(&mut self, now_ns: u64) -> Result<(), VmError> {
        // Each timer is advanced against host time; expired counters signal
        // their APIC vector, which lands in the IRR priority queue.
        self.pit.borrow_mut().advance(now_ns);
        self.hpet.borrow_mut().advance(now_ns);
        self.apic.borrow_mut().advance(now_ns);

        let pending = self.apic.borrow_mut().pending_vector();
        self.replay
            .borrow_mut()
            .timer(now_ns, pending)
            .map_err(VmError::Replay)?;
        let Some(vector) = pending else {
            return Ok(());
        };

        // Maskable interrupts are only taken when the guest has sti'd and is
        // outside the one-instruction STI shadow window.
        if self.cpu.state.rflags & (1 << 9) == 0 || self.cpu.state.interrupt_shadow {
            return Ok(());
        }

        self.apic.borrow_mut().accept_pending(vector);
        self.replay
            .borrow_mut()
            .interrupt(vector)
            .map_err(VmError::Replay)?;
        self.cpu
            .handle_interrupt(
                vector,
                &mut self.mmu,
                &mut self.interrupt_controller,
            )?;
        Ok(())
    }

    fn record_terminal_input(
        &mut self,
        input: TerminalInput,
        input_mode: GuestInputMode,
    ) -> Result<(), VmError> {
        let channel = match input_mode {
            GuestInputMode::Serial => HOST_INPUT_SERIAL,
            GuestInputMode::Ps2 => HOST_INPUT_TERMINAL_PS2,
        };
        if let Some(resize) = input.resize {
            self.replay
                .borrow_mut()
                .host_input(channel, Some(resize.rows), Some(resize.columns), &[])
                .map_err(VmError::Replay)?;
            if input_mode == GuestInputMode::Serial {
                self.enqueue_serial_input(&serial_resize_sequence(resize));
            }
        }
        let bytes = if input_mode == GuestInputMode::Ps2 {
            strip_terminal_resize_responses(&input.bytes)
        } else {
            input.bytes
        };
        if !bytes.is_empty() {
            self.replay
                .borrow_mut()
                .host_input(channel, None, None, &bytes)
                .map_err(VmError::Replay)?;
            match input_mode {
                GuestInputMode::Serial => self.enqueue_serial_input(&bytes),
                GuestInputMode::Ps2 => {
                    for byte in bytes {
                        for scancode in ascii_to_scancodes(byte) {
                            self.enqueue_keyboard_scancode(scancode)
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn inject_replay_host_inputs(&mut self) -> Result<(), VmError> {
        if self.replay_mode() != VmReplayMode::Replaying {
            return Ok(())
        }
        loop {
            let input = self
                .replay
                .borrow_mut()
                .next_host_input()
                .map_err(VmError::Replay)?;
            let Some(input) = input else {
                return Ok(())
            };
            self.apply_replay_host_input(input)?;
        }
    }

    fn apply_replay_host_input(
        &mut self,
        input: crate::replay::ReplayHostInput,
    ) -> Result<(), VmError> {
        match input.channel {
            HOST_INPUT_SERIAL => {
                if let (Some(rows), Some(columns)) = (input.rows, input.columns) {
                    self.enqueue_serial_input(&serial_resize_sequence(TerminalResize {
                        rows,
                        columns,
                    }));
                }
                self.enqueue_serial_input(&input.bytes);
            }
            HOST_INPUT_TERMINAL_PS2 => {
                for byte in input.bytes {
                    for scancode in ascii_to_scancodes(byte) {
                        self.enqueue_keyboard_scancode(scancode)
                    }
                }
            }
            HOST_INPUT_KEYBOARD if input.rows.is_none() && input.bytes.len() == 1 => {
                self.enqueue_keyboard_scancode(input.bytes[0]);
            }
            HOST_INPUT_MOUSE if input.rows.is_none() && input.bytes.len() == 3 => {
                let packet = input
                    .bytes
                    .try_into()
                    .map_err(|_| VmError::Replay(ReplayError::Corrupt))?;
                self.enqueue_mouse_packet(packet);
            }
            _ => return Err(VmError::Replay(ReplayError::Corrupt)),
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        let _ = self.sync_disks();
        let virtio_disk = self.virtio_blk.borrow_mut().detach_disk();
        self.cpu.reset();
        self.execution.reset();
        self.mmu.reset();
        self.interrupt_controller.reset();
        self.ports.reset();
        self.pci.borrow_mut().reset();
        self.apic.borrow_mut().reset();
        self.pit.borrow_mut().reset();
        self.hpet.borrow_mut().reset();
        self.ahci.borrow_mut().reset();
        self.nvme.borrow_mut().reset();
        self.virtio_blk.borrow_mut().reset();
        self.virtio_console.borrow_mut().reset();
        self.virtio_rng.borrow_mut().reset();
        self.display.borrow_mut().reset();
        self.bios.reset();
        self.power_notifications.borrow_mut().clear();
        self.guest_power_notifications.borrow_mut().clear();
        self.guest_agent.borrow().clear();
        self.pv_clock.borrow_mut().reset();
        self.memory_hotplug.borrow().clear_pending();
        self.initialized = false;
        if let Some(disk) = virtio_disk {
            let _ = self.virtio_blk.borrow_mut().attach_disk(disk);
        }
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    pub fn cpu_mut(&mut self) -> &mut Cpu {
        &mut self.cpu
    }

    pub fn execution(&self) -> &ExecutionEngine {
        &self.execution
    }

    pub fn execution_mut(&mut self) -> &mut ExecutionEngine {
        &mut self.execution
    }

    /// Report host probing, actual execution, supported features, limitations,
    /// and any fallback. Native host handles do not change guest semantics
    /// until a native executor is integrated and equivalence-tested.
    pub fn hardware_acceleration(&self) -> HardwareAccelerationStatus {
        self.hardware_acceleration.status()
    }

    pub fn mmu(&self) -> &Mmu {
        &self.mmu
    }

    pub fn mmu_mut(&mut self) -> &mut Mmu {
        &mut self.mmu
    }

    /// Shared handle to the PCI Express host bridge (both the port-mapped
    /// config path and the ECAM/MMCONFIG path use the same bridge).
    pub fn pci(&self) -> Rc<RefCell<PciHostBridge>> {
        self.pci.clone()
    }

    /// Shared handle to the local APIC (MMIO and IA32_APIC_BASE MSR paths).
    pub fn apic(&self) -> Rc<RefCell<LocalApic>> {
        self.apic.clone()
    }

    /// Shared handle to the 8254 PIT (port-mapped at 0x40..0x43).
    pub fn pit(&self) -> Rc<RefCell<Pit>> {
        self.pit.clone()
    }

    /// Shared handle to the HPET (MMIO at 0xFED0_0000).
    pub fn hpet(&self) -> Rc<RefCell<Hpet>> {
        self.hpet.clone()
    }

    pub fn guest_agent(&self) -> Rc<RefCell<GuestAgent>> {
        self.guest_agent.clone()
    }

    pub fn pv_clock(&self) -> Rc<RefCell<PvClock>> {
        self.pv_clock.clone()
    }

    /// Monotonic time source shared by VM scheduling and timers.
    pub fn clock(&self) -> SharedMonotonicClock {
        self.clock.clone()
    }

    pub fn replay_session(&self) -> SharedReplay {
        self.replay.clone()
    }

    pub fn replay_mode(&self) -> ReplayMode {
        self.replay.borrow().mode()
    }

    pub fn begin_replay_recording(&mut self) {
        self.replay.borrow_mut().begin_recording()
    }

    pub fn begin_replay(&mut self, trace: ReplayTrace) -> Result<(), ReplayError> {
        self.replay.borrow_mut().begin_replay(trace)
    }

    pub fn stop_replay(&mut self) {
        self.replay.borrow_mut().stop()
    }

    pub fn replay_trace(&self) -> ReplayTrace {
        self.replay.borrow().trace()
    }

    pub fn save_replay(&self, path: impl AsRef<std::path::Path>) -> Result<(), ReplayError> {
        self.replay_trace().save(path)
    }

    pub fn load_replay(path: impl AsRef<std::path::Path>) -> Result<ReplayTrace, ReplayError> {
        ReplayTrace::load(path)
    }

    pub fn memory_hotplug(&self) -> Rc<RefCell<MemoryHotplugDevice>> {
        self.memory_hotplug.clone()
    }

    pub fn take_power_notifications(&mut self) -> Vec<PowerNotification> {
        self.power_notifications.borrow_mut().drain(..).collect()
    }

    pub fn request_shutdown(&mut self) {
        *self.power_state.borrow_mut() = PowerState::Shutdown;
        self.power_notifications
            .borrow_mut()
            .push_back(PowerNotification::Shutdown);
        self.guest_power_notifications
            .borrow_mut()
            .push_back(PowerNotification::Shutdown);
    }

    pub fn request_reboot(&mut self) {
        *self.power_state.borrow_mut() = PowerState::Reboot;
        self.power_notifications
            .borrow_mut()
            .push_back(PowerNotification::Reboot);
        self.guest_power_notifications
            .borrow_mut()
            .push_back(PowerNotification::Reboot);
    }

    /// Add physical RAM at the end of the existing address space and notify
    /// the guest through the hot-plug device and guest-agent event queue.
    pub fn hotplug_memory(&mut self, size: usize) -> Result<u64, VmError> {
        let new_size = self
            .mmu
            .ram_size()
            .checked_add(size)
            .ok_or(VmError::MemoryError)?;
        if new_size > self.config.max_memory_size {
            return Err(VmError::MemoryError)
        }
        let base = self
            .mmu
            .hotplug_memory(size)
            .map_err(|_| VmError::MemoryError)?;
        self.memory_hotplug
            .borrow()
            .add_region(base, size as u64);
        self.guest_agent
            .borrow()
            .notify(GuestEvent::MemoryAdded {
                base,
                size: size as u64,
            });
        Ok(base)
    }

    /// Shared handle to the AHCI SATA host controller.
    pub fn ahci(&self) -> Rc<RefCell<Ahci>> {
        self.ahci.clone()
    }

    /// Shared handle to the NVMe controller.
    pub fn nvme(&self) -> Rc<RefCell<Nvme>> {
        self.nvme.clone()
    }

    /// Shared handle to the legacy Virtio block device.
    pub fn virtio_blk(&self) -> Rc<RefCell<VirtioBlk>> {
        self.virtio_blk.clone()
    }

    /// Shared handle to the legacy Virtio console device.
    pub fn virtio_console(&self) -> Rc<RefCell<VirtioConsole>> {
        self.virtio_console.clone()
    }

    /// Shared handle to the legacy Virtio random-number generator.
    pub fn virtio_rng(&self) -> Rc<RefCell<VirtioRng>> {
        self.virtio_rng.clone()
    }

    /// Shared handle to the emulated COM1 UART, when serial output is enabled.
    pub fn serial(&self) -> Option<Rc<RefCell<Serial16550>>> {
        self.serial.clone()
    }

    /// Shared handle to the PS/2 keyboard and mouse controller.
    pub fn ps2(&self) -> Rc<RefCell<Ps2Controller>> {
        self.ps2.clone()
    }

    pub fn queue_serial_input(&mut self, bytes: &[u8]) {
        let _ = self
            .replay
            .borrow_mut()
            .host_input(HOST_INPUT_SERIAL, None, None, bytes);
        self.enqueue_serial_input(bytes)
    }

    fn enqueue_serial_input(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            self.cpu.state.halted = false
        }
        if let Some(serial) = &self.serial {
            serial.borrow_mut().push_input_lossless(bytes)
        }
    }

    pub fn flush_serial_output(&mut self) {
        if let Some(serial) = &self.serial {
            serial.borrow_mut().flush()
        }
    }

    pub fn power_state(&self) -> PowerState {
        *self.power_state.borrow()
    }

    fn return_if_guest_panicked(&self) -> Result<(), VmError> {
        let Some(serial) = &self.serial else {
            return Ok(())
        };
        let output = serial.borrow();
        if output.guest_panicked() {
            return Err(VmError::GuestPanic)
        }
        Ok(())
    }

    pub fn queue_keyboard_scancode(&mut self, scancode: u8) {
        let _ = self
            .replay
            .borrow_mut()
            .host_input(HOST_INPUT_KEYBOARD, None, None, &[scancode]);
        self.enqueue_keyboard_scancode(scancode)
    }

    fn enqueue_keyboard_scancode(&mut self, scancode: u8) {
        self.cpu.state.halted = false;
        self.ps2.borrow_mut().push_keyboard_scancode(scancode)
    }

    pub fn queue_mouse_packet(&mut self, packet: [u8; 3]) {
        let _ = self
            .replay
            .borrow_mut()
            .host_input(HOST_INPUT_MOUSE, None, None, &packet);
        self.enqueue_mouse_packet(packet)
    }

    fn enqueue_mouse_packet(&mut self, packet: [u8; 3]) {
        self.ps2.borrow_mut().push_mouse_packet(packet)
    }

    pub fn queue_mouse_motion(&mut self, dx: i16, dy: i16, buttons: u8) {
        let packet = [
            0x08 | (buttons & 0x07),
            dx.clamp(-127, 127) as i8 as u8,
            dy.clamp(-127, 127) as i8 as u8,
        ];
        let _ = self
            .replay
            .borrow_mut()
            .host_input(HOST_INPUT_MOUSE, None, None, &packet);
        self.ps2.borrow_mut().push_mouse_motion(dx, dy, buttons)
    }

    /// Shared handle to the display state (VGA text, VESA LFB, palette).
    pub fn display(&self) -> Rc<RefCell<DisplayState>> {
        self.display.clone()
    }

    pub fn config(&self) -> &VmConfig {
        &self.config
    }

    pub fn dhcp_network(&self) -> Option<Rc<RefCell<DeterministicVmNetwork>>> {
        self.dhcp_network.clone()
    }

    pub fn network_mac_addresses(&self) -> [MacAddress; 2] {
        [self.e1000.borrow().mac(), self.virtio_net.borrow().mac()]
    }

    pub fn set_network_admin_up(&mut self, up: bool) {
        self.e1000.borrow_mut().set_admin_up(up);
    }

    pub fn network_admin_up(&self) -> bool {
        self.e1000.borrow().admin_up()
    }

    pub fn network_link_up(&self) -> bool {
        self.e1000.borrow().carrier_up()
    }

    pub fn take_network_error(&mut self) -> Option<NetError> {
        self.last_network_error.take()
    }

    fn record_network_error(&mut self, error: NetError) {
        if self.last_network_error.is_none() {
            self.last_network_error = Some(error);
        }
    }

    pub fn transmit_network_frame(&mut self, packet: &[u8]) -> Result<(), NetError> {
        let result = self.e1000.borrow_mut().transmit_frame(packet);
        if let Err(error) = result {
            self.record_network_error(error);
        }
        result
    }

    pub fn receive_network_frame(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        let result = self.e1000.borrow_mut().receive_frame();
        if let Err(error) = result {
            self.record_network_error(error);
        }
        result
    }

    /// Capture a checkpoint of guest execution and memory state.
    pub fn snapshot(&self) -> VmSnapshot {
        VmSnapshot::capture(self)
    }

    /// Alias for [`Self::snapshot`] using checkpoint terminology.
    pub fn checkpoint(&self) -> VmSnapshot {
        self.snapshot()
    }

    /// Restore a checkpoint captured from a VM with the same RAM size.
    pub fn restore_snapshot(&mut self, snapshot: &VmSnapshot) -> Result<(), SnapshotError> {
        snapshot.restore_into(self)
    }

    /// Restore a checkpoint and report which host-owned state must be rebuilt
    /// or remains outside the checkpoint.
    pub fn restore_snapshot_with_report(
        &mut self,
        snapshot: &VmSnapshot,
    ) -> Result<snapshot::SnapshotRestoreReport, SnapshotError> {
        snapshot.restore_into_with_report(self)
    }

    /// Alias for [`Self::restore_snapshot`].
    pub fn restore(&mut self, snapshot: &VmSnapshot) -> Result<(), SnapshotError> {
        self.restore_snapshot(snapshot)
    }

    /// Save an unkeyed raw payload for offline format conversion only.
    /// Trusted checkpoint storage must use [`Self::save_authenticated_snapshot`].
    pub fn save_snapshot(&self, path: impl AsRef<std::path::Path>) -> Result<(), SnapshotError> {
        self.snapshot().save(path)
    }

    /// Save an authenticated checkpoint. Use this for persistent or remote
    /// snapshot state; the unkeyed method above is retained for raw format
    /// conversion and local test fixtures.
    pub fn save_authenticated_snapshot(
        &self,
        path: impl AsRef<std::path::Path>,
        key: snapshot::SnapshotAuthKey,
    ) -> Result<(), SnapshotError> {
        self.snapshot().save_authenticated(path, key)
    }

    /// Load an untrusted raw payload for offline format conversion only.
    pub fn load_snapshot(path: impl AsRef<std::path::Path>) -> Result<VmSnapshot, SnapshotError> {
        VmSnapshot::load(path)
    }

    pub fn load_authenticated_snapshot(
        path: impl AsRef<std::path::Path>,
        key: snapshot::SnapshotAuthKey,
    ) -> Result<VmSnapshot, SnapshotError> {
        VmSnapshot::load_authenticated(path, key)
    }
}

impl Default for Vm {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub enum VmError {
    CpuError(CpuError),
    GuestPanic,
    MemoryError,
    BiosError,
    IoError,
    Terminal(TerminalFailure),
    Replay(ReplayError),
    InvalidConfiguration,
    Disk(String),
    Network(String),
    KernelLoadError,
    BootFailure,
    HardwareAcceleration(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VmRunReport {
    pub steps: u64,
    pub halted: bool,
    pub rip: u64,
}

impl From<CpuError> for VmError {
    fn from(e: CpuError) -> Self {
        VmError::CpuError(e)
    }
}

fn loader_error_to_vm(error: crate::boot::LoaderError) -> VmError {
    match error {
        crate::boot::LoaderError::CpuError(error) => VmError::CpuError(error),
        _ => VmError::KernelLoadError,
    }
}

fn disk_error_to_vm(error: StorageError) -> VmError {
    VmError::Disk(error.to_string())
}

fn create_network_backends(
    config: &NetworkBackendConfig,
    dhcp_config: Option<&DhcpServerConfig>,
) -> Result<(
    MacAddress,
    MacAddress,
    Box<dyn NetBackend>,
    Box<dyn NetBackend>,
    Option<Rc<RefCell<DeterministicVmNetwork>>>,
), VmError> {
    match config {
        NetworkBackendConfig::Deterministic => {
            let network = DeterministicVmNetwork::new(dhcp_config.cloned())
                .map_err(|error| VmError::Network(error.to_string()))?;
            let (e1000_mac, e1000, virtio_mac, virtio) = network
                .borrow_mut()
                .attach_vm()
                .map_err(|error| VmError::Network(error.to_string()))?;
            Ok((
                e1000_mac,
                virtio_mac,
                Box::new(e1000),
                Box::new(virtio),
                Some(network),
            ))
        }
        NetworkBackendConfig::DeterministicShared { network } => {
            let (e1000_mac, e1000, virtio_mac, virtio) = network
                .borrow_mut()
                .attach_vm()
                .map_err(|error| VmError::Network(error.to_string()))?;
            Ok((
                e1000_mac,
                virtio_mac,
                Box::new(e1000),
                Box::new(virtio),
                Some(network.clone()),
            ))
        }
        NetworkBackendConfig::UserNat { bind, peer } => {
            let e1000_mac = MacAddress::synos_default(0x56);
            let virtio_mac = MacAddress::synos_default(0x57);
            let e1000 = HostNetworkBackend::user_nat(*bind, *peer, e1000_mac)
                .map_err(|error| VmError::Network(format!("cannot open user-mode network: {error}")))?;
            let virtio_bind = SocketAddr::new(bind.ip(), 0);
            let virtio = HostNetworkBackend::user_nat(virtio_bind, *peer, virtio_mac)
                .map_err(|error| VmError::Network(format!("cannot open user-mode network: {error}")))?;
            Ok((
                e1000_mac,
                virtio_mac,
                Box::new(e1000),
                Box::new(virtio),
                None,
            ))
        }
        NetworkBackendConfig::Bridged { interface } => {
            let e1000_mac = MacAddress::synos_default(0x56);
            let virtio_mac = MacAddress::synos_default(0x57);
            let e1000 = HostNetworkBackend::bridged(interface, e1000_mac)
                .map_err(|error| VmError::Network(format!("cannot open bridged network: {error}")))?;
            let virtio = HostNetworkBackend::bridged(interface, virtio_mac)
                .map_err(|error| VmError::Network(format!("cannot open bridged network: {error}")))?;
            Ok((
                e1000_mac,
                virtio_mac,
                Box::new(e1000),
                Box::new(virtio),
                None,
            ))
        }
    }
}

fn disk_controller_name(controller: DiskController) -> &'static str {
    match controller {
        DiskController::Ahci => "AHCI",
        DiskController::Nvme => "NVMe",
        DiskController::VirtioBlk => "virtio-blk",
    }
}
