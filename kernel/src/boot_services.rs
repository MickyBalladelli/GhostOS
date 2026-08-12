//! Services started by the first user-space boot sequence.

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, Ordering};

use synos_fsd::{Daemon, ProcessRights};
use synos_init::{
    ProcessId, RestartPolicy, ServiceId, ServiceKind, ServiceName, ServiceSpec, SpawnRequest,
    Supervisor, SupervisorEvent, SupervisorRuntime,
};
use synos_status::Status;
use synos_synfs::SynFs;

const FILESYSTEM_SERVICE_ID: u32 = 0x4653_4444;
const FILESYSTEM_PROCESS_ID: u64 = 2;
const FILESYSTEM_IMAGE_ID: u128 = 0x5359_4e4f_5346_5344_0000_0000_0000_0001;
const FILESYSTEM_CAPABILITY_PROFILE: u64 = 0x5346_5344_5f52_4f4f;
const STORAGE_SERVICE_ID: u32 = 0x5354_4f52;
const STORAGE_PROCESS_ID: u64 = 3;
const STORAGE_IMAGE_ID: u128 = 0x5359_4e4f_5354_4f52_0000_0000_0000_0001;
const STORAGE_CAPABILITY_PROFILE: u64 = 0x5354_4f52_5f52_4f4f;
const NETWORK_SERVICE_ID: u32 = 0x4e45_5444;
const NETWORK_PROCESS_ID: u64 = 4;
const NETWORK_IMAGE_ID: u128 = 0x5359_4e4f_4e45_5444_0000_0000_0000_0001;
const NETWORK_CAPABILITY_PROFILE: u64 = 0x4e45_5444_5f52_4f4f;
const LOGGING_SERVICE_ID: u32 = 0x4c4f_4744;
const LOGGING_PROCESS_ID: u64 = 5;
const LOGGING_IMAGE_ID: u128 = 0x5359_4e4f_4c4f_4744_0000_0000_0000_0001;
const LOGGING_CAPABILITY_PROFILE: u64 = 0x4c4f_4744_5f52_4f4f;
const AUDIT_SERVICE_ID: u32 = 0x4155_4454;
const AUDIT_PROCESS_ID: u64 = 6;
const AUDIT_IMAGE_ID: u128 = 0x5359_4e4f_4155_4454_0000_0000_0000_0001;
const AUDIT_CAPABILITY_PROFILE: u64 = 0x4155_4454_5f52_4f4f;
const AUTHENTICATION_SERVICE_ID: u32 = 0x4155_5448;
const AUTHENTICATION_PROCESS_ID: u64 = 7;
const AUTHENTICATION_IMAGE_ID: u128 = 0x5359_4e4f_5341_5554_4800_0000_0000_0001;
const AUTHENTICATION_CAPABILITY_PROFILE: u64 = 0x4155_5448_5f52_4f4f;
const PACKAGE_SERVICE_ID: u32 = 0x504b_4744;
const PACKAGE_PROCESS_ID: u64 = 8;
const PACKAGE_IMAGE_ID: u128 = 0x5359_4e4f_5350_4b47_4400_0000_0000_0001;
const PACKAGE_CAPABILITY_PROFILE: u64 = 0x504b_4744_5f52_4f4f;
const SHELL_SERVICE_ID: u32 = 0x5348_454c;
const SHELL_PROCESS_ID: u64 = 9;
const SHELL_IMAGE_ID: u128 = 0x5359_4e4f_5353_4845_4c4c_0000_0000_0001;
const SHELL_CAPABILITY_PROFILE: u64 = 0x5348_454c_4c5f_524f;
const PCI_SERVICE_ID: u32 = 0x5043_4944;
const PCI_PROCESS_ID: u64 = 10;
const PCI_IMAGE_ID: u128 = 0x5359_4e4f_5043_4944_0000_0000_0000_0001;
const PCI_CAPABILITY_PROFILE: u64 = 0x5043_4944_5f52_4f4f;
const AHCI_SERVICE_ID: u32 = 0x4148_4349;
const AHCI_PROCESS_ID: u64 = 11;
const AHCI_IMAGE_ID: u128 = 0x5359_4e4f_4148_4349_0000_0000_0000_0001;
const AHCI_CAPABILITY_PROFILE: u64 = 0x4148_4349_5f52_4f4f;
const NVME_SERVICE_ID: u32 = 0x4e56_4d45;
const NVME_PROCESS_ID: u64 = 12;
const NVME_IMAGE_ID: u128 = 0x5359_4e4f_4e56_4d45_0000_0000_0000_0001;
const NVME_CAPABILITY_PROFILE: u64 = 0x4e56_4d45_5f52_4f4f;
const ETHERNET_SERVICE_ID: u32 = 0x4554_4844;
const ETHERNET_PROCESS_ID: u64 = 13;
const ETHERNET_IMAGE_ID: u128 = 0x5359_4e4f_4554_4844_0000_0000_0000_0001;
const ETHERNET_CAPABILITY_PROFILE: u64 = 0x4554_4844_5f52_4f4f;
const FILESYSTEM_BLOCKS: usize = synos_synfs::SYSTEM_VOLUME_BLOCKS;

type FilesystemDaemon = Daemon<FILESYSTEM_BLOCKS>;

static mut FILESYSTEM_DAEMON: MaybeUninit<FilesystemDaemon> = MaybeUninit::uninit();
static FILESYSTEM_READY: AtomicBool = AtomicBool::new(false);
static STORAGE_READY: AtomicBool = AtomicBool::new(false);
static NETWORK_READY: AtomicBool = AtomicBool::new(false);
static LOGGING_READY: AtomicBool = AtomicBool::new(false);
static AUDIT_READY: AtomicBool = AtomicBool::new(false);
static AUTHENTICATION_READY: AtomicBool = AtomicBool::new(false);
static PACKAGE_READY: AtomicBool = AtomicBool::new(false);
static SHELL_READY: AtomicBool = AtomicBool::new(false);
static PCI_READY: AtomicBool = AtomicBool::new(false);
static AHCI_READY: AtomicBool = AtomicBool::new(false);
static NVME_READY: AtomicBool = AtomicBool::new(false);
static ETHERNET_READY: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
pub struct BootServices {
    pub filesystem_process: ProcessId,
    pub storage_process: ProcessId,
    pub network_process: ProcessId,
    pub logging_process: ProcessId,
    pub audit_process: ProcessId,
    pub authentication_process: ProcessId,
    pub package_process: ProcessId,
    pub shell_process: ProcessId,
    pub pci_process: ProcessId,
    pub ahci_process: ProcessId,
    pub nvme_process: ProcessId,
    pub ethernet_process: ProcessId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartError {
    Daemon,
    Process,
    Supervisor,
}

impl StartError {
    pub const fn status(self) -> Status {
        match self {
            Self::Daemon => Status::CORRUPT,
            Self::Process => Status::NO_SPACE,
            Self::Supervisor => Status::BUSY,
        }
    }
}

struct BootRuntime {
    filesystem_process: ProcessId,
    storage_process: ProcessId,
    network_process: ProcessId,
    logging_process: ProcessId,
    audit_process: ProcessId,
    authentication_process: ProcessId,
    package_process: ProcessId,
    shell_process: ProcessId,
    pci_process: ProcessId,
    ahci_process: ProcessId,
    nvme_process: ProcessId,
    ethernet_process: ProcessId,
}

impl SupervisorRuntime for BootRuntime {
    type Error = StartError;

    fn spawn(&mut self, request: SpawnRequest) -> Result<ProcessId, Self::Error> {
        if request.service == filesystem_service_id() {
            return Ok(self.filesystem_process)
        }
        if request.service == storage_service_id() {
            return Ok(self.storage_process)
        }
        if request.service == network_service_id() {
            return Ok(self.network_process)
        }
        if request.service == logging_service_id() {
            return Ok(self.logging_process)
        }
        if request.service == audit_service_id() {
            return Ok(self.audit_process)
        }
        if request.service == authentication_service_id() {
            return Ok(self.authentication_process)
        }
        if request.service == package_service_id() {
            return Ok(self.package_process)
        }
        if request.service == shell_service_id() {
            return Ok(self.shell_process)
        }
        if request.service == pci_service_id() {
            return Ok(self.pci_process)
        }
        if request.service == ahci_service_id() {
            return Ok(self.ahci_process)
        }
        if request.service == nvme_service_id() {
            return Ok(self.nvme_process)
        }
        if request.service == ethernet_service_id() {
            return Ok(self.ethernet_process)
        }
        Err(StartError::Process)
    }

    fn fence_process(&mut self, process: ProcessId) -> Result<(), Self::Error> {
        (process == self.filesystem_process
            || process == self.storage_process
            || process == self.network_process
            || process == self.logging_process
            || process == self.audit_process
            || process == self.authentication_process
            || process == self.package_process
            || process == self.shell_process
            || process == self.pci_process
            || process == self.ahci_process
            || process == self.nvme_process
            || process == self.ethernet_process)
            .then_some(())
            .ok_or(StartError::Process)
    }
}

pub fn start(physical_filesystem: Option<SynFs<FILESYSTEM_BLOCKS>>) -> Result<BootServices, StartError> {
    if FILESYSTEM_READY.load(Ordering::Acquire)
        && STORAGE_READY.load(Ordering::Acquire)
        && NETWORK_READY.load(Ordering::Acquire)
        && LOGGING_READY.load(Ordering::Acquire)
        && AUDIT_READY.load(Ordering::Acquire)
        && AUTHENTICATION_READY.load(Ordering::Acquire)
        && PACKAGE_READY.load(Ordering::Acquire)
        && SHELL_READY.load(Ordering::Acquire)
        && PCI_READY.load(Ordering::Acquire)
        && AHCI_READY.load(Ordering::Acquire)
        && NVME_READY.load(Ordering::Acquire)
        && ETHERNET_READY.load(Ordering::Acquire)
    {
        return Ok(BootServices {
            filesystem_process: filesystem_process_id().ok_or(StartError::Process)?,
            storage_process: storage_process_id().ok_or(StartError::Process)?,
            network_process: network_process_id().ok_or(StartError::Process)?,
            logging_process: logging_process_id().ok_or(StartError::Process)?,
            audit_process: audit_process_id().ok_or(StartError::Process)?,
            authentication_process: authentication_process_id().ok_or(StartError::Process)?,
            package_process: package_process_id().ok_or(StartError::Process)?,
            shell_process: shell_process_id().ok_or(StartError::Process)?,
            pci_process: pci_process_id().ok_or(StartError::Process)?,
            ahci_process: ahci_process_id().ok_or(StartError::Process)?,
            nvme_process: nvme_process_id().ok_or(StartError::Process)?,
            ethernet_process: ethernet_process_id().ok_or(StartError::Process)?,
        })
    }

    let filesystem_process = filesystem_process_id().ok_or(StartError::Process)?;
    let storage_process = storage_process_id().ok_or(StartError::Process)?;
    let network_process = network_process_id().ok_or(StartError::Process)?;
    let logging_process = logging_process_id().ok_or(StartError::Process)?;
    let audit_process = audit_process_id().ok_or(StartError::Process)?;
    let authentication_process = authentication_process_id().ok_or(StartError::Process)?;
    let package_process = package_process_id().ok_or(StartError::Process)?;
    let shell_process = shell_process_id().ok_or(StartError::Process)?;
    let pci_process = pci_process_id().ok_or(StartError::Process)?;
    let ahci_process = ahci_process_id().ok_or(StartError::Process)?;
    let nvme_process = nvme_process_id().ok_or(StartError::Process)?;
    let ethernet_process = ethernet_process_id().ok_or(StartError::Process)?;
    let mut daemon = Daemon::new(
        physical_filesystem.unwrap_or_else(SynFs::<FILESYSTEM_BLOCKS>::new),
    )
        .map_err(|_| StartError::Daemon)?;
    daemon
        .register_process(
            synos_fsd::ProcessId::new(filesystem_process.raw()).ok_or(StartError::Process)?,
            ProcessRights::from_bits(
                ProcessRights::READ.bits()
                    | ProcessRights::WRITE.bits()
                    | ProcessRights::DELETE.bits()
                    | ProcessRights::ADMIN.bits(),
            ),
        )
        .map_err(|_| StartError::Daemon)?;

    let filesystem_name = ServiceName::new("synos-fsd").map_err(|_| StartError::Supervisor)?;
    let storage_name = ServiceName::new("synos-storaged").map_err(|_| StartError::Supervisor)?;
    let network_name = ServiceName::new("synos-netd").map_err(|_| StartError::Supervisor)?;
    let logging_name = ServiceName::new("synos-logd").map_err(|_| StartError::Supervisor)?;
    let audit_name = ServiceName::new("synos-auditd").map_err(|_| StartError::Supervisor)?;
    let authentication_name =
        ServiceName::new("synos-authd").map_err(|_| StartError::Supervisor)?;
    let package_name = ServiceName::new("synos-pkgd").map_err(|_| StartError::Supervisor)?;
    let shell_name = ServiceName::new("synos-shell").map_err(|_| StartError::Supervisor)?;
    let pci_name = ServiceName::new("synos-pcid").map_err(|_| StartError::Supervisor)?;
    let ahci_name = ServiceName::new("synos-ahcid").map_err(|_| StartError::Supervisor)?;
    let nvme_name = ServiceName::new("synos-nvmed").map_err(|_| StartError::Supervisor)?;
    let ethernet_name =
        ServiceName::new("synos-ethernetd").map_err(|_| StartError::Supervisor)?;
    let mut supervisor = Supervisor::<12>::new();
    supervisor
        .register(ServiceSpec {
            id: filesystem_service_id(),
            name: filesystem_name,
            kind: ServiceKind::System,
            image_id: FILESYSTEM_IMAGE_ID,
            capability_profile: FILESYSTEM_CAPABILITY_PROFILE,
            restart: RestartPolicy::on_failure(3, 60_000_000, 100_000, 5_000_000)
                .map_err(|_| StartError::Supervisor)?,
        })
        .map_err(|_| StartError::Supervisor)?;
    supervisor
        .register(ServiceSpec {
            id: storage_service_id(),
            name: storage_name,
            kind: ServiceKind::StorageDriver,
            image_id: STORAGE_IMAGE_ID,
            capability_profile: STORAGE_CAPABILITY_PROFILE,
            restart: RestartPolicy::on_failure(3, 60_000_000, 100_000, 5_000_000)
                .map_err(|_| StartError::Supervisor)?,
        })
        .map_err(|_| StartError::Supervisor)?;
    supervisor
        .register(ServiceSpec {
            id: network_service_id(),
            name: network_name,
            kind: ServiceKind::NetworkDriver,
            image_id: NETWORK_IMAGE_ID,
            capability_profile: NETWORK_CAPABILITY_PROFILE,
            restart: RestartPolicy::on_failure(3, 60_000_000, 100_000, 5_000_000)
                .map_err(|_| StartError::Supervisor)?,
        })
        .map_err(|_| StartError::Supervisor)?;
    supervisor
        .register(ServiceSpec {
            id: logging_service_id(),
            name: logging_name,
            kind: ServiceKind::System,
            image_id: LOGGING_IMAGE_ID,
            capability_profile: LOGGING_CAPABILITY_PROFILE,
            restart: RestartPolicy::on_failure(3, 60_000_000, 100_000, 5_000_000)
                .map_err(|_| StartError::Supervisor)?,
        })
        .map_err(|_| StartError::Supervisor)?;
    supervisor
        .register(ServiceSpec {
            id: audit_service_id(),
            name: audit_name,
            kind: ServiceKind::System,
            image_id: AUDIT_IMAGE_ID,
            capability_profile: AUDIT_CAPABILITY_PROFILE,
            restart: RestartPolicy::on_failure(3, 60_000_000, 100_000, 5_000_000)
                .map_err(|_| StartError::Supervisor)?,
        })
        .map_err(|_| StartError::Supervisor)?;
    supervisor
        .register(ServiceSpec {
            id: authentication_service_id(),
            name: authentication_name,
            kind: ServiceKind::System,
            image_id: AUTHENTICATION_IMAGE_ID,
            capability_profile: AUTHENTICATION_CAPABILITY_PROFILE,
            restart: RestartPolicy::on_failure(3, 60_000_000, 100_000, 5_000_000)
                .map_err(|_| StartError::Supervisor)?,
        })
        .map_err(|_| StartError::Supervisor)?;
    supervisor
        .register(ServiceSpec {
            id: package_service_id(),
            name: package_name,
            kind: ServiceKind::System,
            image_id: PACKAGE_IMAGE_ID,
            capability_profile: PACKAGE_CAPABILITY_PROFILE,
            restart: RestartPolicy::on_failure(3, 60_000_000, 100_000, 5_000_000)
                .map_err(|_| StartError::Supervisor)?,
        })
        .map_err(|_| StartError::Supervisor)?;
    supervisor
        .register(ServiceSpec {
            id: shell_service_id(),
            name: shell_name,
            kind: ServiceKind::System,
            image_id: SHELL_IMAGE_ID,
            capability_profile: SHELL_CAPABILITY_PROFILE,
            restart: RestartPolicy::on_failure(3, 60_000_000, 100_000, 5_000_000)
                .map_err(|_| StartError::Supervisor)?,
        })
        .map_err(|_| StartError::Supervisor)?;
    supervisor
        .register(ServiceSpec {
            id: pci_service_id(),
            name: pci_name,
            kind: ServiceKind::System,
            image_id: PCI_IMAGE_ID,
            capability_profile: PCI_CAPABILITY_PROFILE,
            restart: RestartPolicy::on_failure(3, 60_000_000, 100_000, 5_000_000)
                .map_err(|_| StartError::Supervisor)?,
        })
        .map_err(|_| StartError::Supervisor)?;
    supervisor
        .register(ServiceSpec {
            id: ahci_service_id(),
            name: ahci_name,
            kind: ServiceKind::StorageDriver,
            image_id: AHCI_IMAGE_ID,
            capability_profile: AHCI_CAPABILITY_PROFILE,
            restart: RestartPolicy::on_failure(3, 60_000_000, 100_000, 5_000_000)
                .map_err(|_| StartError::Supervisor)?,
        })
        .map_err(|_| StartError::Supervisor)?;
    supervisor
        .register(ServiceSpec {
            id: nvme_service_id(),
            name: nvme_name,
            kind: ServiceKind::StorageDriver,
            image_id: NVME_IMAGE_ID,
            capability_profile: NVME_CAPABILITY_PROFILE,
            restart: RestartPolicy::on_failure(3, 60_000_000, 100_000, 5_000_000)
                .map_err(|_| StartError::Supervisor)?,
        })
        .map_err(|_| StartError::Supervisor)?;
    supervisor
        .register(ServiceSpec {
            id: ethernet_service_id(),
            name: ethernet_name,
            kind: ServiceKind::NetworkDriver,
            image_id: ETHERNET_IMAGE_ID,
            capability_profile: ETHERNET_CAPABILITY_PROFILE,
            restart: RestartPolicy::on_failure(3, 60_000_000, 100_000, 5_000_000)
                .map_err(|_| StartError::Supervisor)?,
        })
        .map_err(|_| StartError::Supervisor)?;

    let mut runtime = BootRuntime {
        filesystem_process,
        storage_process,
        network_process,
        logging_process,
        audit_process,
        authentication_process,
        package_process,
        shell_process,
        pci_process,
        ahci_process,
        nvme_process,
        ethernet_process,
    };
    for service in [
        filesystem_service_id(),
        storage_service_id(),
        network_service_id(),
        logging_service_id(),
        audit_service_id(),
        authentication_service_id(),
        package_service_id(),
        shell_service_id(),
        pci_service_id(),
        ahci_service_id(),
        nvme_service_id(),
        ethernet_service_id(),
    ] {
        let event = supervisor
            .start(service, &mut runtime)
            .map_err(|_| StartError::Supervisor)?;
        if !matches!(event, SupervisorEvent::Started { .. }) {
            return Err(StartError::Supervisor)
        }
    }

    // The daemon owns its namespace and capabilities for the lifetime of the
    // service. Publish it only after all startup checks have succeeded.
    unsafe {
        (&mut *core::ptr::addr_of_mut!(FILESYSTEM_DAEMON)).write(daemon);
    }
    FILESYSTEM_READY.store(true, Ordering::Release);
    STORAGE_READY.store(true, Ordering::Release);
    NETWORK_READY.store(true, Ordering::Release);
    LOGGING_READY.store(true, Ordering::Release);
    AUDIT_READY.store(true, Ordering::Release);
    AUTHENTICATION_READY.store(true, Ordering::Release);
    PACKAGE_READY.store(true, Ordering::Release);
    SHELL_READY.store(true, Ordering::Release);
    PCI_READY.store(true, Ordering::Release);
    AHCI_READY.store(true, Ordering::Release);
    NVME_READY.store(true, Ordering::Release);
    ETHERNET_READY.store(true, Ordering::Release);
    Ok(BootServices {
        filesystem_process,
        storage_process,
        network_process,
        logging_process,
        audit_process,
        authentication_process,
        package_process,
        shell_process,
        pci_process,
        ahci_process,
        nvme_process,
        ethernet_process,
    })
}

pub const fn filesystem_service_id() -> ServiceId {
    ServiceId::new(FILESYSTEM_SERVICE_ID).unwrap()
}

pub const fn storage_service_id() -> ServiceId {
    ServiceId::new(STORAGE_SERVICE_ID).unwrap()
}

pub const fn network_service_id() -> ServiceId {
    ServiceId::new(NETWORK_SERVICE_ID).unwrap()
}

pub const fn logging_service_id() -> ServiceId {
    ServiceId::new(LOGGING_SERVICE_ID).unwrap()
}

pub const fn audit_service_id() -> ServiceId {
    ServiceId::new(AUDIT_SERVICE_ID).unwrap()
}

pub const fn authentication_service_id() -> ServiceId {
    ServiceId::new(AUTHENTICATION_SERVICE_ID).unwrap()
}

pub const fn package_service_id() -> ServiceId {
    ServiceId::new(PACKAGE_SERVICE_ID).unwrap()
}

pub const fn shell_service_id() -> ServiceId {
    ServiceId::new(SHELL_SERVICE_ID).unwrap()
}

pub const fn pci_service_id() -> ServiceId {
    ServiceId::new(PCI_SERVICE_ID).unwrap()
}

pub const fn ahci_service_id() -> ServiceId {
    ServiceId::new(AHCI_SERVICE_ID).unwrap()
}

pub const fn nvme_service_id() -> ServiceId {
    ServiceId::new(NVME_SERVICE_ID).unwrap()
}

pub const fn ethernet_service_id() -> ServiceId {
    ServiceId::new(ETHERNET_SERVICE_ID).unwrap()
}

const fn filesystem_process_id() -> Option<ProcessId> {
    ProcessId::new(FILESYSTEM_PROCESS_ID)
}

const fn storage_process_id() -> Option<ProcessId> {
    ProcessId::new(STORAGE_PROCESS_ID)
}

const fn network_process_id() -> Option<ProcessId> {
    ProcessId::new(NETWORK_PROCESS_ID)
}

const fn logging_process_id() -> Option<ProcessId> {
    ProcessId::new(LOGGING_PROCESS_ID)
}

const fn audit_process_id() -> Option<ProcessId> {
    ProcessId::new(AUDIT_PROCESS_ID)
}

const fn authentication_process_id() -> Option<ProcessId> {
    ProcessId::new(AUTHENTICATION_PROCESS_ID)
}

const fn package_process_id() -> Option<ProcessId> {
    ProcessId::new(PACKAGE_PROCESS_ID)
}

const fn shell_process_id() -> Option<ProcessId> {
    ProcessId::new(SHELL_PROCESS_ID)
}

const fn pci_process_id() -> Option<ProcessId> {
    ProcessId::new(PCI_PROCESS_ID)
}

const fn ahci_process_id() -> Option<ProcessId> {
    ProcessId::new(AHCI_PROCESS_ID)
}

const fn nvme_process_id() -> Option<ProcessId> {
    ProcessId::new(NVME_PROCESS_ID)
}

const fn ethernet_process_id() -> Option<ProcessId> {
    ProcessId::new(ETHERNET_PROCESS_ID)
}
