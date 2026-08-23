//! Services started by the first user-space boot sequence.

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, Ordering};
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
use core::sync::atomic::{AtomicU8, AtomicU64};

use ghostos_fsd::{Daemon, ProcessRights};
use ghostos_init::{
    ProcessId, RestartPolicy, ServiceId, ServiceKind, ServiceName, ServiceReadiness, ServiceSpec,
    ServiceState, SpawnRequest, StartupDiagnostic, Supervisor, SupervisorRuntime,
};
use ghostos_status::Status;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
use ghostos_status::{facility, IntoStatus, Severity};
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
use ghostos_observability::{audit_event, field, EventField, Level};
use ghostos_ghostfs::SynFs;

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
const LOGIN_SERVICE_ID: u32 = 0x4c4f_4749;
const LOGIN_PROCESS_ID: u64 = 14;
const LOGIN_IMAGE_ID: u128 = 0x5359_4e4f_4c4f_4749_4e00_0000_0000_0001;
const LOGIN_CAPABILITY_PROFILE: u64 = 0x4c4f_4749_4e5f_524f;
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
const FILESYSTEM_BLOCKS: usize = ghostos_ghostfs::SYSTEM_VOLUME_BLOCKS;
pub const AUTHORIZATION_DATABASE_PATH: &str = "/system/security/authorization";
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub const FIRST_ADMIN_USERNAME_PATH: &str = "/system/security/first-admin-username";
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub const FIRST_ADMIN_CREDENTIAL_PATH: &str = "/system/security/first-admin-credential";
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const FIRST_ADMIN_USERNAME_CAPACITY: usize = 32;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const FIRST_ADMIN_CREDENTIAL_CAPACITY: usize = 96;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const FIRST_ADMIN_AUTHORIZATION_RECORD_VERSION: u8 = 1;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const FIRST_ADMIN_AUTHORIZATION_RECORD_CAPACITY: usize =
    2 + FIRST_ADMIN_USERNAME_CAPACITY + 2 + FIRST_ADMIN_CREDENTIAL_CAPACITY;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) const LOCAL_PASSKEY_MAX_KEYS: usize = 4;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) const LOCAL_PASSKEY_MAX_KEY_BYTES: usize = FIRST_ADMIN_CREDENTIAL_CAPACITY;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOCAL_PASSKEY_COUNTERS_PATH: &str = "/system/security/passkey-counters";
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOCAL_PASSKEY_COUNTERS_HEADER_BYTES: usize = 6;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOCAL_PASSKEY_COUNTER_ENTRY_BYTES: usize = 37;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOCAL_ACCOUNT_RECORD_HEADER_BYTES: usize = 36;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const LOCAL_ACCOUNT_USERNAME_CAPACITY: usize = 32;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const AUDIT_FIRST_ADMIN_USERNAME: u64 = 0x1001;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const AUDIT_FIRST_ADMIN_CREDENTIAL: u64 = 0x1002;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const AUDIT_FIRST_ADMIN_COMMIT: u64 = 0x1003;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const AUDIT_FIRST_ADMIN_RECOVERY_STATUS: u64 = 0x1004;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const AUDIT_FIRST_ADMIN_RECOVERY_RETRY: u64 = 0x1005;
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
const AUDIT_FIRST_ADMIN_RECOVERY_RESET: u64 = 0x1006;
const SERVICE_COUNT: usize = 13;

type FilesystemDaemon = Daemon<FILESYSTEM_BLOCKS>;

static mut FILESYSTEM_DAEMON: MaybeUninit<FilesystemDaemon> = MaybeUninit::uninit();
static FILESYSTEM_READY: AtomicBool = AtomicBool::new(false);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static SHELL_FILESYSTEM_AUTHORITY: AtomicU64 = AtomicU64::new(0);
static STORAGE_READY: AtomicBool = AtomicBool::new(false);
static NETWORK_READY: AtomicBool = AtomicBool::new(false);
static LOGGING_READY: AtomicBool = AtomicBool::new(false);
static AUDIT_READY: AtomicBool = AtomicBool::new(false);
static AUTHENTICATION_READY: AtomicBool = AtomicBool::new(false);
static PACKAGE_READY: AtomicBool = AtomicBool::new(false);
static SHELL_READY: AtomicBool = AtomicBool::new(false);
static LOGIN_READY: AtomicBool = AtomicBool::new(false);
static PCI_READY: AtomicBool = AtomicBool::new(false);
static AHCI_READY: AtomicBool = AtomicBool::new(false);
static NVME_READY: AtomicBool = AtomicBool::new(false);
static ETHERNET_READY: AtomicBool = AtomicBool::new(false);
static PROVISIONING_REQUIRED: AtomicBool = AtomicBool::new(true);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static FIRST_ADMIN_SYNC_PENDING: AtomicBool = AtomicBool::new(false);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static FIRST_ADMIN_RECOVERY_SYNC_PENDING: AtomicBool = AtomicBool::new(false);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static FIRST_ADMIN_USERNAME_LENGTH: AtomicU8 = AtomicU8::new(0);
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
static FIRST_ADMIN_USERNAME: [AtomicU8; FIRST_ADMIN_USERNAME_CAPACITY] =
    [const { AtomicU8::new(0) }; FIRST_ADMIN_USERNAME_CAPACITY];
static STARTUP_DIAGNOSTICS_READY: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
pub struct BootStartupDiagnostic {
    pub startup_order: usize,
    pub state: ServiceState,
    pub readiness: ServiceReadiness,
    pub dependency_count: usize,
    pub blocked_on: Option<ServiceId>,
}

const EMPTY_STARTUP_DIAGNOSTIC: BootStartupDiagnostic = BootStartupDiagnostic {
    startup_order: 0,
    state: ServiceState::Stopped,
    readiness: ServiceReadiness::Waiting,
    dependency_count: 0,
    blocked_on: None,
};

static mut STARTUP_DIAGNOSTICS: [BootStartupDiagnostic; SERVICE_COUNT] =
    [EMPTY_STARTUP_DIAGNOSTIC; SERVICE_COUNT];

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
    pub login_process: ProcessId,
    pub pci_process: ProcessId,
    pub ahci_process: ProcessId,
    pub nvme_process: ProcessId,
    pub ethernet_process: ProcessId,
    #[allow(dead_code)]
    pub provisioning_required: bool,
}

fn service_ids() -> [ServiceId; SERVICE_COUNT] {
    [
        filesystem_service_id(),
        storage_service_id(),
        network_service_id(),
        logging_service_id(),
        audit_service_id(),
        authentication_service_id(),
        package_service_id(),
        shell_service_id(),
        login_service_id(),
        pci_service_id(),
        ahci_service_id(),
        nvme_service_id(),
        ethernet_service_id(),
    ]
}

pub(crate) fn startup_diagnostic(service: ServiceId) -> Option<BootStartupDiagnostic> {
    if !STARTUP_DIAGNOSTICS_READY.load(Ordering::Acquire) {
        return None
    }
    let index = service_ids().iter().position(|id| *id == service)?;
    unsafe { Some(STARTUP_DIAGNOSTICS[index]) }
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
    login_process: ProcessId,
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
        if request.service == login_service_id() {
            return Ok(self.login_process)
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
            || process == self.login_process
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
        && LOGIN_READY.load(Ordering::Acquire)
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
            login_process: login_process_id().ok_or(StartError::Process)?,
            pci_process: pci_process_id().ok_or(StartError::Process)?,
            ahci_process: ahci_process_id().ok_or(StartError::Process)?,
            nvme_process: nvme_process_id().ok_or(StartError::Process)?,
            ethernet_process: ethernet_process_id().ok_or(StartError::Process)?,
            provisioning_required: PROVISIONING_REQUIRED.load(Ordering::Acquire),
        })
    }

    let provisioning_required = physical_filesystem.as_ref().is_none_or(|filesystem| {
        matches!(
            filesystem.lookup(AUTHORIZATION_DATABASE_PATH),
            Err(ghostos_ghostfs::Error::NotFound)
        )
    });
    #[cfg(all(
        target_arch = "x86_64",
        any(target_os = "none", target_os = "uefi")
    ))]
    load_first_admin_username(physical_filesystem.as_ref());
    #[cfg(all(
        target_arch = "x86_64",
        any(target_os = "none", target_os = "uefi")
    ))]
    match physical_filesystem.as_ref() {
        Some(filesystem) => {
            crate::println!(
                "[fsprobe] boot: provisioning_required={} used_blocks={} free_bytes={}",
                provisioning_required,
                filesystem.used_blocks(),
                filesystem.free_bytes()
            );
            match filesystem.check_consistency() {
                Ok(()) => crate::println!("[fsprobe] boot: consistency ok"),
                Err(error) => crate::println!("[fsprobe] boot: consistency {error:?}"),
            }
        }
        None => {
            crate::println!("[fsprobe] boot: AHCI MOUNT FAILED - running on transient bootstrap filesystem");
        }
    }

    let filesystem_process = filesystem_process_id().ok_or(StartError::Process)?;
    let storage_process = storage_process_id().ok_or(StartError::Process)?;
    let network_process = network_process_id().ok_or(StartError::Process)?;
    let logging_process = logging_process_id().ok_or(StartError::Process)?;
    let audit_process = audit_process_id().ok_or(StartError::Process)?;
    let authentication_process = authentication_process_id().ok_or(StartError::Process)?;
    let package_process = package_process_id().ok_or(StartError::Process)?;
    let shell_process = shell_process_id().ok_or(StartError::Process)?;
    let login_process = login_process_id().ok_or(StartError::Process)?;
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
            ghostos_fsd::ProcessId::new(filesystem_process.raw()).ok_or(StartError::Process)?,
            ProcessRights::from_bits(
                ProcessRights::READ.bits()
                    | ProcessRights::WRITE.bits()
                    | ProcessRights::DELETE.bits()
                    | ProcessRights::ADMIN.bits(),
            ),
        )
        .map_err(|_| StartError::Daemon)?;
    daemon
        .register_process(
            ghostos_fsd::ProcessId::new(shell_process.raw()).ok_or(StartError::Process)?,
            ProcessRights::NONE,
        )
        .map_err(|_| StartError::Daemon)?;

    let filesystem_name = ServiceName::new("ghostos-fsd").map_err(|_| StartError::Supervisor)?;
    let storage_name = ServiceName::new("ghostos-storaged").map_err(|_| StartError::Supervisor)?;
    let network_name = ServiceName::new("ghostos-netd").map_err(|_| StartError::Supervisor)?;
    let logging_name = ServiceName::new("ghostos-logd").map_err(|_| StartError::Supervisor)?;
    let audit_name = ServiceName::new("ghostos-auditd").map_err(|_| StartError::Supervisor)?;
    let authentication_name =
        ServiceName::new("ghostos-authd").map_err(|_| StartError::Supervisor)?;
    let package_name = ServiceName::new("ghostos-pkgd").map_err(|_| StartError::Supervisor)?;
    let shell_name = ServiceName::new("ghostos-shell").map_err(|_| StartError::Supervisor)?;
    let login_name = ServiceName::new("ghostos-logind").map_err(|_| StartError::Supervisor)?;
    let pci_name = ServiceName::new("ghostos-pcid").map_err(|_| StartError::Supervisor)?;
    let ahci_name = ServiceName::new("ghostos-ahcid").map_err(|_| StartError::Supervisor)?;
    let nvme_name = ServiceName::new("ghostos-nvmed").map_err(|_| StartError::Supervisor)?;
    let ethernet_name =
        ServiceName::new("ghostos-ethernetd").map_err(|_| StartError::Supervisor)?;
    let mut supervisor = Supervisor::<SERVICE_COUNT>::new();
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
            id: login_service_id(),
            name: login_name,
            kind: ServiceKind::System,
            image_id: LOGIN_IMAGE_ID,
            capability_profile: LOGIN_CAPABILITY_PROFILE,
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

    for (service, dependency) in [
        (storage_service_id(), pci_service_id()),
        (filesystem_service_id(), storage_service_id()),
        (network_service_id(), ethernet_service_id()),
        (audit_service_id(), logging_service_id()),
        (authentication_service_id(), filesystem_service_id()),
        (authentication_service_id(), audit_service_id()),
        (package_service_id(), filesystem_service_id()),
        (package_service_id(), logging_service_id()),
        (shell_service_id(), authentication_service_id()),
        (login_service_id(), authentication_service_id()),
        (shell_service_id(), login_service_id()),
        (shell_service_id(), package_service_id()),
        (shell_service_id(), network_service_id()),
        (shell_service_id(), logging_service_id()),
        (ahci_service_id(), pci_service_id()),
        (nvme_service_id(), pci_service_id()),
        (ethernet_service_id(), pci_service_id()),
    ] {
        supervisor
            .add_dependency(service, dependency)
            .map_err(|_| StartError::Supervisor)?;
    }

    let mut runtime = BootRuntime {
        filesystem_process,
        storage_process,
        network_process,
        logging_process,
        audit_process,
        authentication_process,
        package_process,
        shell_process,
        login_process,
        pci_process,
        ahci_process,
        nvme_process,
        ethernet_process,
    };
    let startup_trace = match supervisor.boot(&mut runtime) {
        Ok(trace) => trace,
        Err(error) => {
            crate::println!("service startup failed: {:?}", error);
            return Err(StartError::Supervisor)
        }
    };
    let mut startup_orders = [0usize; SERVICE_COUNT];
    let mut startup_order = 1usize;
    crate::println!("service startup dependency order:");
    for event in startup_trace.events() {
        if let ghostos_init::LifecycleEvent::ServiceStarted { service, generation } = event {
            if let Some(index) = service_ids().iter().position(|id| *id == service) {
                startup_orders[index] = startup_order;
            }
            let diagnostic = supervisor
                .startup_diagnostic(service)
                .map_err(|_| StartError::Supervisor)?;
            crate::println!(
                "  service={} generation={} dependencies={} readiness={}",
                service.raw(),
                generation,
                diagnostic.status.dependency_count,
                matches!(diagnostic.status.readiness, ServiceReadiness::Ready),
            );
            startup_order += 1;
        }
    }
    for (index, service) in service_ids().iter().copied().enumerate() {
        let diagnostic: StartupDiagnostic = supervisor
            .startup_diagnostic(service)
            .map_err(|_| StartError::Supervisor)?;
        unsafe {
            STARTUP_DIAGNOSTICS[index] = BootStartupDiagnostic {
                startup_order: startup_orders[index],
                state: diagnostic.status.state,
                readiness: diagnostic.status.readiness,
                dependency_count: diagnostic.status.dependency_count,
                blocked_on: diagnostic.blocked_on,
            };
        }
    }
    STARTUP_DIAGNOSTICS_READY.store(true, Ordering::Release);

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
    LOGIN_READY.store(true, Ordering::Release);
    PCI_READY.store(true, Ordering::Release);
    AHCI_READY.store(true, Ordering::Release);
    NVME_READY.store(true, Ordering::Release);
    ETHERNET_READY.store(true, Ordering::Release);
    PROVISIONING_REQUIRED.store(provisioning_required, Ordering::Release);
    Ok(BootServices {
        filesystem_process,
        storage_process,
        network_process,
        logging_process,
        audit_process,
        authentication_process,
        package_process,
        shell_process,
        login_process,
        pci_process,
        ahci_process,
        nvme_process,
        ethernet_process,
        provisioning_required,
    })
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn valid_first_admin_username(username: &[u8]) -> bool {
    if username.is_empty()
        || username.len() > FIRST_ADMIN_USERNAME_CAPACITY
        || !username
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-$".contains(byte))
    {
        return false
    }
    let mut normalized = [0; FIRST_ADMIN_USERNAME_CAPACITY];
    for (slot, byte) in normalized.iter_mut().zip(username.iter().copied()) {
        *slot = byte.to_ascii_lowercase();
    }
    !is_reserved_first_admin_username(&normalized[..username.len()])
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn is_reserved_first_admin_username(username: &[u8]) -> bool {
    [
        b".".as_slice(),
        b"..".as_slice(),
        b"account".as_slice(),
        b"anonymous".as_slice(),
        b"daemon".as_slice(),
        b"guest".as_slice(),
        b"kernel".as_slice(),
        b"nobody".as_slice(),
        b"operator".as_slice(),
        b"root".as_slice(),
        b"service".as_slice(),
        b"system".as_slice(),
    ]
    .iter()
    .any(|reserved| *reserved == username)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn normalized_first_admin_username(username: &[u8]) -> Option<[u8; FIRST_ADMIN_USERNAME_CAPACITY]> {
    if !valid_first_admin_username(username) {
        return None
    }
    let mut normalized = [0; FIRST_ADMIN_USERNAME_CAPACITY];
    for (slot, byte) in normalized.iter_mut().zip(username.iter().copied()) {
        *slot = byte.to_ascii_lowercase();
    }
    Some(normalized)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) fn local_passkey_keys(
    username: &[u8],
    keys: &mut [[u8; LOCAL_PASSKEY_MAX_KEY_BYTES]; LOCAL_PASSKEY_MAX_KEYS],
    lengths: &mut [u8; LOCAL_PASSKEY_MAX_KEYS],
) -> Result<usize, Status> {
    if !valid_first_admin_username(username) {
        return Err(Status::INVALID_ARGUMENT)
    }
    let daemon = unsafe {
        (&mut *core::ptr::addr_of_mut!(FILESYSTEM_DAEMON)).assume_init_mut()
    };
    let metadata = daemon
        .filesystem()
        .lookup(AUTHORIZATION_DATABASE_PATH)
        .map_err(|error| error.status())?;
    let database_size = metadata.size as usize;
    if metadata.file_type != ghostos_ghostfs::FileType::Regular
        || database_size == 0
        || database_size > 4096
    {
        return Err(Status::CORRUPT)
    }
    let mut database = [0; 4096];
    let read = daemon
        .filesystem()
        .read_version(AUTHORIZATION_DATABASE_PATH, metadata.version, &mut database)
        .map_err(|error| error.status())?;
    if read.bytes_read != database_size {
        return Err(Status::CORRUPT)
    }

    let mut latest_offset = None;
    let mut offset = 0;
    while offset < database_size {
        if database_size - offset < LOCAL_ACCOUNT_RECORD_HEADER_BYTES {
            return Err(Status::CORRUPT)
        }
        let record = &database[offset..];
        let record_bytes = LOCAL_ACCOUNT_RECORD_HEADER_BYTES
            + record[LOCAL_ACCOUNT_RECORD_HEADER_BYTES - 1] as usize;
        if record_bytes > database_size - offset
            || !matches!(record[0], 1 | 2)
            || record[1] == 0
            || record[1] as usize > LOCAL_ACCOUNT_USERNAME_CAPACITY
        {
            return Err(Status::CORRUPT)
        }
        if record[2..2 + record[1] as usize]
            .iter()
            .zip(username.iter())
            .all(|(stored, requested)| stored.eq_ignore_ascii_case(requested))
            && record[1] as usize == username.len()
        {
            latest_offset = Some(offset)
        }
        offset += record_bytes
    }
    let Some(record_offset) = latest_offset else {
        return Ok(0)
    };
    let record = &database[record_offset..];
    let state = record[LOCAL_ACCOUNT_USERNAME_CAPACITY + 2];
    if !(1..=3).contains(&state) {
        return Ok(0)
    }
    let mut count = 0;
    if record[0] == 1 {
        let length = record[LOCAL_ACCOUNT_RECORD_HEADER_BYTES - 1] as usize;
        if length <= LOCAL_PASSKEY_MAX_KEY_BYTES && state == 1 {
            keys[0][..length].copy_from_slice(
                &record[LOCAL_ACCOUNT_RECORD_HEADER_BYTES
                    ..LOCAL_ACCOUNT_RECORD_HEADER_BYTES + length],
            );
            lengths[0] = length as u8;
            return Ok(1)
        }
        return Ok(0)
    }
    let credential_count = record[LOCAL_ACCOUNT_RECORD_HEADER_BYTES] as usize;
    if credential_count == 0 || credential_count > LOCAL_PASSKEY_MAX_KEYS {
        return Err(Status::CORRUPT)
    }
    let mut credential_offset = LOCAL_ACCOUNT_RECORD_HEADER_BYTES + 1;
    for _ in 0..credential_count {
        if credential_offset + 3 > record_bytes(record) {
            return Err(Status::CORRUPT)
        }
        let kind = record[credential_offset + 1];
        let length = record[credential_offset + 2] as usize;
        let material_start = credential_offset + 3;
        if length == 0 || material_start + length > record_bytes(record) {
            return Err(Status::CORRUPT)
        }
        if kind == 1 && count < LOCAL_PASSKEY_MAX_KEYS && length <= LOCAL_PASSKEY_MAX_KEY_BYTES {
            keys[count][..length].copy_from_slice(&record[material_start..material_start + length]);
            lengths[count] = length as u8;
            count += 1;
        }
        credential_offset = material_start + length;
    }
    if credential_offset != record_bytes(record) {
        return Err(Status::CORRUPT)
    }
    Ok(count)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) fn local_passkey_sign_count(
    username: &[u8],
    key_fingerprint: &[u8; 32],
) -> Result<u32, Status> {
    let mut database = [0; 4096];
    let database_size = load_local_passkey_counters(&mut database)?;
    let mut offset = LOCAL_PASSKEY_COUNTERS_HEADER_BYTES;
    for _ in 0..database[5] {
        let username_length = database[offset] as usize;
        if offset + LOCAL_PASSKEY_COUNTER_ENTRY_BYTES + username_length > database_size {
            return Err(Status::CORRUPT)
        }
        if database[offset + 5..offset + 37] == *key_fingerprint
            && database[offset + 37..offset + 37 + username_length]
                .eq_ignore_ascii_case(username)
        {
            return Ok(u32::from_be_bytes([
                database[offset + 1],
                database[offset + 2],
                database[offset + 3],
                database[offset + 4],
            ]))
        }
        offset += LOCAL_PASSKEY_COUNTER_ENTRY_BYTES + username_length;
    }
    Ok(0)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) fn record_local_passkey_sign_count(
    username: &[u8],
    key_fingerprint: &[u8; 32],
    sign_count: u32,
) -> Result<(), Status> {
    if username.is_empty() || username.len() > 32 {
        return Err(Status::INVALID_ARGUMENT)
    }
    let mut database = [0; 4096];
    let mut database_size = load_local_passkey_counters(&mut database)?;
    let mut offset = LOCAL_PASSKEY_COUNTERS_HEADER_BYTES;
    for _ in 0..database[5] {
        let username_length = database[offset] as usize;
        if offset + LOCAL_PASSKEY_COUNTER_ENTRY_BYTES + username_length > database_size {
            return Err(Status::CORRUPT)
        }
        if database[offset + 5..offset + 37] == *key_fingerprint
            && database[offset + 37..offset + 37 + username_length]
                .eq_ignore_ascii_case(username)
        {
            database[offset + 1..offset + 5].copy_from_slice(&sign_count.to_be_bytes());
            return store_local_passkey_counters(&database[..database_size])
        }
        offset += LOCAL_PASSKEY_COUNTER_ENTRY_BYTES + username_length;
    }
    if database[5] == u8::MAX
        || database_size + LOCAL_PASSKEY_COUNTER_ENTRY_BYTES + username.len() > database.len()
    {
        return Err(Status::NO_SPACE)
    }
    database[offset] = username.len() as u8;
    database[offset + 1..offset + 5].copy_from_slice(&sign_count.to_be_bytes());
    database[offset + 5..offset + 37].copy_from_slice(key_fingerprint);
    database[offset + 37..offset + 37 + username.len()].copy_from_slice(username);
    database[5] += 1;
    database_size += LOCAL_PASSKEY_COUNTER_ENTRY_BYTES + username.len();
    store_local_passkey_counters(&database[..database_size])
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn load_local_passkey_counters(database: &mut [u8; 4096]) -> Result<usize, Status> {
    let daemon = unsafe {
        (&mut *core::ptr::addr_of_mut!(FILESYSTEM_DAEMON)).assume_init_mut()
    };
    let metadata = match daemon.filesystem().lookup(LOCAL_PASSKEY_COUNTERS_PATH) {
        Ok(metadata) => metadata,
        Err(ghostos_ghostfs::Error::NotFound) => {
            database[..4].copy_from_slice(b"SYPC");
            database[4] = 2;
            database[5] = 0;
            return Ok(LOCAL_PASSKEY_COUNTERS_HEADER_BYTES)
        }
        Err(error) => return Err(error.status()),
    };
    let size = metadata.size as usize;
    if metadata.file_type != ghostos_ghostfs::FileType::Regular
        || !(LOCAL_PASSKEY_COUNTERS_HEADER_BYTES..=database.len()).contains(&size)
    {
        return Err(Status::CORRUPT)
    }
    let read = daemon
        .filesystem()
        .read_version(LOCAL_PASSKEY_COUNTERS_PATH, metadata.version, database)
        .map_err(|error| error.status())?;
    if read.bytes_read != size
        || database[..4] != *b"SYPC"
        || database[4] != 2
    {
        return Err(Status::CORRUPT)
    }
    let mut offset = LOCAL_PASSKEY_COUNTERS_HEADER_BYTES;
    for _ in 0..database[5] {
        if offset + LOCAL_PASSKEY_COUNTER_ENTRY_BYTES > size {
            return Err(Status::CORRUPT)
        }
        let username_length = database[offset] as usize;
        if username_length == 0
            || username_length > 32
            || offset + LOCAL_PASSKEY_COUNTER_ENTRY_BYTES + username_length > size
        {
            return Err(Status::CORRUPT)
        }
        offset += LOCAL_PASSKEY_COUNTER_ENTRY_BYTES + username_length;
    }
    (offset == size).then_some(size).ok_or(Status::CORRUPT)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn store_local_passkey_counters(database: &[u8]) -> Result<(), Status> {
    let daemon = unsafe {
        (&mut *core::ptr::addr_of_mut!(FILESYSTEM_DAEMON)).assume_init_mut()
    };
    let mut transaction = daemon.filesystem_mut().transaction();
    transaction
        .write(LOCAL_PASSKEY_COUNTERS_PATH, database)
        .map_err(|error| error.status())?;
    transaction.commit().map_err(|error| error.status())?;
    crate::physical_storage::sync(daemon.filesystem_mut()).map_err(|_| Status::INTERNAL)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn record_bytes(record: &[u8]) -> usize {
    LOCAL_ACCOUNT_RECORD_HEADER_BYTES + record[LOCAL_ACCOUNT_RECORD_HEADER_BYTES - 1] as usize
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn audit_first_admin(action: u64, status: Status) {
    audit_event!(
        if status.is_success() { Level::Info } else { Level::Warn },
        EventField::unsigned(field::OPERATION, action),
        EventField::unsigned(field::CALLER, SHELL_PROCESS_ID as u64),
        EventField::status(status),
    );
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn load_first_admin_username(filesystem: Option<&SynFs<FILESYSTEM_BLOCKS>>) {
    FIRST_ADMIN_USERNAME_LENGTH.store(0, Ordering::Release);
    let Some(filesystem) = filesystem else {
        return
    };
    let Ok(metadata) = filesystem.lookup(FIRST_ADMIN_USERNAME_PATH) else {
        return
    };
    if metadata.file_type != ghostos_ghostfs::FileType::Regular
        || metadata.size == 0
        || metadata.size as usize > FIRST_ADMIN_USERNAME_CAPACITY
    {
        return
    }
    let mut username = [0; FIRST_ADMIN_USERNAME_CAPACITY];
    let Ok(read) = filesystem.read_version(
        FIRST_ADMIN_USERNAME_PATH,
        metadata.version,
        &mut username,
    ) else {
        return
    };
    let Some(normalized) = normalized_first_admin_username(&username[..read.bytes_read]) else {
        return
    };
    for (slot, byte) in FIRST_ADMIN_USERNAME
        .iter()
        .zip(normalized.iter().copied())
        .take(read.bytes_read)
    {
        slot.store(byte, Ordering::Relaxed)
    }
    FIRST_ADMIN_USERNAME_LENGTH.store(read.bytes_read as u8, Ordering::Release);
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) fn create_first_admin_username(username: &[u8]) -> Result<(), Status> {
    let result = create_first_admin_username_inner(username);
    audit_first_admin(
        AUDIT_FIRST_ADMIN_USERNAME,
        result.as_ref().map(|_| Status::NORMAL).unwrap_or_else(|error| *error),
    );
    result
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn create_first_admin_username_inner(username: &[u8]) -> Result<(), Status> {
    let normalized = normalized_first_admin_username(username)
        .ok_or(Status::INVALID_ARGUMENT)?;
    let username = &normalized[..username.len()];
    if !PROVISIONING_REQUIRED.load(Ordering::Acquire) {
        return Err(Status::ALREADY_EXISTS)
    }
    let daemon = unsafe {
        (&mut *core::ptr::addr_of_mut!(FILESYSTEM_DAEMON)).assume_init_mut()
    };
    match daemon.filesystem().lookup(AUTHORIZATION_DATABASE_PATH) {
        Ok(_) => return Err(Status::ALREADY_EXISTS),
        Err(ghostos_ghostfs::Error::NotFound) => {}
        Err(error) => return Err(error.status()),
    }
    match daemon.filesystem().lookup(FIRST_ADMIN_USERNAME_PATH) {
        Ok(metadata) => {
            if metadata.file_type != ghostos_ghostfs::FileType::Regular
                || metadata.size as usize != username.len()
            {
                return Err(Status::ALREADY_EXISTS)
            }
            let mut existing = [0; FIRST_ADMIN_USERNAME_CAPACITY];
            let read = daemon
                .filesystem()
                .read_version(FIRST_ADMIN_USERNAME_PATH, metadata.version, &mut existing)
                .map_err(|error| error.status())?;
            if read.bytes_read != username.len()
                || existing[..read.bytes_read] != *username
            {
                return Err(Status::ALREADY_EXISTS)
            }
            return Ok(())
        }
        Err(ghostos_ghostfs::Error::NotFound) => {}
        Err(error) => return Err(error.status()),
    }
    let mut transaction = daemon.filesystem_mut().transaction();
    match transaction.lookup("/system/security") {
        Ok(metadata) if metadata.file_type == ghostos_ghostfs::FileType::Directory => {}
        Ok(_) => return Err(Status::INVALID_PATH),
        Err(ghostos_ghostfs::Error::NotFound) => {
            transaction
                .create_directory("/system/security", true)
                .map_err(|error| error.status())?;
        }
        Err(error) => return Err(error.status()),
    }
    transaction
        .write(FIRST_ADMIN_USERNAME_PATH, username)
        .map_err(|error| error.status())?;
    transaction
        .commit()
        .map_err(|error| error.status())?;
    #[cfg(debug_assertions)]
    {
        let mut probe = [0u8; FIRST_ADMIN_USERNAME_CAPACITY];
        let read_back = daemon.filesystem().read_version(
            FIRST_ADMIN_USERNAME_PATH,
            1,
            &mut probe,
        );
        crate::println!(
            "[fsprobe] username saved: len={} readback={:?} used_blocks={}",
            username.len(),
            read_back.as_ref().map(|read| read.bytes_read),
            daemon.filesystem().used_blocks()
        );
        match daemon.filesystem().debug_data_probe(FIRST_ADMIN_USERNAME_PATH) {
            Ok((size, slot, kind, checksum, head)) => crate::println!(
                "[fsprobe] username data: size={} slot={} kind={} checksum={:#x} head={:#x}",
                size, slot, kind, checksum, head
            ),
            Err(error) => crate::println!("[fsprobe] username data: probe failed {error:?}"),
        }
        print_arena(daemon.filesystem_mut());
        if let Ok(read) = read_back {
            crate::println!("[fsprobe] username bytes: {:02x?}", &probe[..read.bytes_read]);
        }
    }
    for (slot, byte) in FIRST_ADMIN_USERNAME
        .iter()
        .zip(username.iter().copied())
        .take(username.len())
    {
        slot.store(byte, Ordering::Relaxed)
    }
    FIRST_ADMIN_USERNAME_LENGTH.store(username.len() as u8, Ordering::Release);
    Ok(())
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) fn create_first_admin_credential(
    kind: u8,
    public_material: &[u8],
) -> Result<(), Status> {
    let result = create_first_admin_credential_inner(kind, public_material);
    audit_first_admin(
        AUDIT_FIRST_ADMIN_CREDENTIAL,
        result.as_ref().map(|_| Status::NORMAL).unwrap_or_else(|error| *error),
    );
    result
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn create_first_admin_credential_inner(
    kind: u8,
    public_material: &[u8],
) -> Result<(), Status> {
    if !(1..=3).contains(&kind)
        || public_material.is_empty()
        || public_material.len() > FIRST_ADMIN_CREDENTIAL_CAPACITY
        || (kind == 1 && !crate::webauthn::valid_cose_es256_public_key(public_material))
    {
        return Err(Status::INVALID_ARGUMENT)
    }
    if !PROVISIONING_REQUIRED.load(Ordering::Acquire) {
        return Err(Status::ACCESS_DENIED)
    }
    let daemon = unsafe {
        (&mut *core::ptr::addr_of_mut!(FILESYSTEM_DAEMON)).assume_init_mut()
    };
    match daemon.filesystem().lookup(AUTHORIZATION_DATABASE_PATH) {
        Ok(_) => return Err(Status::ALREADY_EXISTS),
        Err(ghostos_ghostfs::Error::NotFound) => {}
        Err(error) => return Err(error.status()),
    }
    match daemon.filesystem().lookup(FIRST_ADMIN_USERNAME_PATH) {
        Ok(metadata) if metadata.file_type == ghostos_ghostfs::FileType::Regular => {}
        Ok(_) => return Err(Status::CORRUPT),
        Err(ghostos_ghostfs::Error::NotFound) => return Err(Status::ACCESS_DENIED),
        Err(error) => return Err(error.status()),
    }
    match daemon.filesystem().lookup(FIRST_ADMIN_CREDENTIAL_PATH) {
        Ok(metadata) => {
            let size = metadata.size as usize;
            if metadata.file_type != ghostos_ghostfs::FileType::Regular
                || size < 3
                || size > FIRST_ADMIN_CREDENTIAL_CAPACITY + 2
            {
                return Err(Status::CORRUPT)
            }
            let mut existing = [0; FIRST_ADMIN_CREDENTIAL_CAPACITY + 2];
            let read = daemon
                .filesystem()
                .read_version(FIRST_ADMIN_CREDENTIAL_PATH, metadata.version, &mut existing)
                .map_err(|error| error.status())?;
            let existing_length = existing[1] as usize;
            if read.bytes_read != size
                || !(1..=3).contains(&existing[0])
                || existing_length == 0
                || existing_length > FIRST_ADMIN_CREDENTIAL_CAPACITY
                || size != existing_length + 2
            {
                return Err(Status::CORRUPT)
            }
            if existing[0] == kind
                && existing_length == public_material.len()
                && existing[2..2 + existing_length] == *public_material
            {
                return Ok(())
            }
        }
        Err(ghostos_ghostfs::Error::NotFound) => {}
        Err(error) => return Err(error.status()),
    }
    let mut transaction = daemon.filesystem_mut().transaction();
    let mut record = [0; FIRST_ADMIN_CREDENTIAL_CAPACITY + 2];
    record[0] = kind;
    record[1] = public_material.len() as u8;
    record[2..2 + public_material.len()].copy_from_slice(public_material);
    transaction
        .write(FIRST_ADMIN_CREDENTIAL_PATH, &record[..2 + public_material.len()])
        .map_err(|error| error.status())?;
    transaction
        .commit()
        .map_err(|error| error.status())?;
    #[cfg(debug_assertions)]
    {
        let mut probe = [0u8; FIRST_ADMIN_USERNAME_CAPACITY];
        let username_back = daemon.filesystem().read_version(
            FIRST_ADMIN_USERNAME_PATH,
            1,
            &mut probe,
        );
        crate::println!(
            "[fsprobe] credential saved: kind={} material_len={} username_readback={:?} used_blocks={}",
            kind,
            public_material.len(),
            username_back.as_ref().map(|read| read.bytes_read),
            daemon.filesystem().used_blocks()
        );
        for (label, path) in [
            ("username", FIRST_ADMIN_USERNAME_PATH),
            ("credential", FIRST_ADMIN_CREDENTIAL_PATH),
        ] {
            match daemon.filesystem().debug_data_probe(path) {
                Ok((size, slot, block_kind, checksum, head)) => crate::println!(
                    "[fsprobe] {} data: size={} slot={} kind={} checksum={:#x} head={:#x}",
                    label, size, slot, block_kind, checksum, head
                ),
                Err(error) => {
                    crate::println!("[fsprobe] {label} data: probe failed {error:?}")
                }
            }
        }
        print_arena(daemon.filesystem_mut());
        if let Ok(read) = username_back {
            crate::println!("[fsprobe] username bytes now: {:02x?}", &probe[..read.bytes_read]);
        }
    }
    Ok(())
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) fn commit_first_admin() -> Result<(), Status> {
    let result = commit_first_admin_inner();
    audit_first_admin(
        AUDIT_FIRST_ADMIN_COMMIT,
        result.as_ref().map(|_| Status::NORMAL).unwrap_or_else(|error| *error),
    );
    result
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn first_admin_corrupt(site: u8) -> Status {
    Status::new(Severity::Fatal, facility::SYSTEM, 6, site).unwrap_or(Status::CORRUPT)
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
#[cfg(debug_assertions)]
fn print_arena(filesystem: &mut SynFs<FILESYSTEM_BLOCKS>) {
    let (words, root, generation) = filesystem.debug_arena_dump();
    crate::println!(
        "[fsprobe] arena: used={} root={} gen={} daemon@{:#x}",
        filesystem.used_blocks(),
        root,
        generation,
        (&raw const FILESYSTEM_DAEMON) as usize
    );
    for (index, word) in words.iter().enumerate() {
        crate::println!("[fsprobe] arena[{:02}]: {:#018x}", index * 16, word);
    }
    for (index, (kind, first, second)) in filesystem.debug_event_slice().iter().enumerate() {
        crate::println!("[fsprobe] ev[{index:02}] kind={kind} a={first} b={second}");
    }
    filesystem.debug_reset_events();
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn commit_first_admin_inner() -> Result<(), Status> {
    if !PROVISIONING_REQUIRED.load(Ordering::Acquire) {
        return Err(Status::ALREADY_EXISTS)
    }
    let daemon = unsafe {
        (&mut *core::ptr::addr_of_mut!(FILESYSTEM_DAEMON)).assume_init_mut()
    };
    if FIRST_ADMIN_SYNC_PENDING.load(Ordering::Acquire) {
        crate::physical_storage::sync(daemon.filesystem_mut())
            .map_err(|_| Status::INTERNAL)?;
        FIRST_ADMIN_SYNC_PENDING.store(false, Ordering::Release);
        FIRST_ADMIN_USERNAME_LENGTH.store(0, Ordering::Release);
        PROVISIONING_REQUIRED.store(false, Ordering::Release);
        return Ok(())
    }
    match daemon.filesystem().lookup(AUTHORIZATION_DATABASE_PATH) {
        Ok(_) => return Err(Status::ALREADY_EXISTS),
        Err(ghostos_ghostfs::Error::NotFound) => {}
        Err(ghostos_ghostfs::Error::Corrupt) => return Err(first_admin_corrupt(6)),
        Err(error) => return Err(error.status()),
    }

    let username_metadata = daemon
        .filesystem()
        .lookup(FIRST_ADMIN_USERNAME_PATH)
        .map_err(|error| match error {
            ghostos_ghostfs::Error::NotFound => Status::CONFIRMATION_REQUIRED,
            other => other.status(),
        })?;
    #[cfg(debug_assertions)]
    {
        crate::println!(
            "[fsprobe] confirm: username type={:?} size={} used_blocks={}",
            username_metadata.file_type,
            username_metadata.size,
            daemon.filesystem().used_blocks()
        );
        match daemon.filesystem().debug_data_probe(FIRST_ADMIN_USERNAME_PATH) {
            Ok((size, slot, kind, checksum, head)) => crate::println!(
                "[fsprobe] confirm: username data size={} slot={} kind={} checksum={:#x} head={:#x}",
                size, slot, kind, checksum, head
            ),
            Err(error) => crate::println!("[fsprobe] confirm: username probe failed {error:?}"),
        }
        print_arena(daemon.filesystem_mut());
    }
    if username_metadata.file_type != ghostos_ghostfs::FileType::Regular
        || username_metadata.size == 0
        || username_metadata.size as usize > FIRST_ADMIN_USERNAME_CAPACITY
    {
        return Err(first_admin_corrupt(1))
    }
    let mut username = [0; FIRST_ADMIN_USERNAME_CAPACITY];
    let username_read = daemon
        .filesystem()
        .read_version(
            FIRST_ADMIN_USERNAME_PATH,
            username_metadata.version,
            &mut username,
        )
        .map_err(|error| {
            crate::println!(
                "[fsprobe] confirm: USERNAME READ FAILED: {error:?} used_blocks={}",
                daemon.filesystem().used_blocks()
            );
            match error {
                ghostos_ghostfs::Error::Corrupt => first_admin_corrupt(7),
                other => other.status(),
            }
        })?;
    if username_read.bytes_read != username_metadata.size as usize
        || !valid_first_admin_username(&username[..username_read.bytes_read])
    {
        return Err(first_admin_corrupt(2))
    }

    let credential_metadata = daemon
        .filesystem()
        .lookup(FIRST_ADMIN_CREDENTIAL_PATH)
        .map_err(|error| match error {
            ghostos_ghostfs::Error::NotFound => Status::CONFIRMATION_REQUIRED,
            other => other.status(),
        })?;
    let credential_size = credential_metadata.size as usize;
    if credential_metadata.file_type != ghostos_ghostfs::FileType::Regular
        || credential_size < 3
        || credential_size > FIRST_ADMIN_CREDENTIAL_CAPACITY + 2
    {
        return Err(first_admin_corrupt(3))
    }
    let mut credential = [0; FIRST_ADMIN_CREDENTIAL_CAPACITY + 2];
    let credential_read = daemon
        .filesystem()
        .read_version(
            FIRST_ADMIN_CREDENTIAL_PATH,
            credential_metadata.version,
            &mut credential,
        )
        .map_err(|error| match error {
            ghostos_ghostfs::Error::Corrupt => first_admin_corrupt(8),
            other => other.status(),
        })?;
    let credential_length = credential[1] as usize;
    if credential_read.bytes_read != credential_size
        || !(1..=3).contains(&credential[0])
        || credential_length == 0
        || credential_length > FIRST_ADMIN_CREDENTIAL_CAPACITY
        || credential_size != credential_length + 2
    {
        return Err(first_admin_corrupt(4))
    }
    if credential[0] == 1
        && !crate::webauthn::valid_cose_es256_public_key(
            &credential[2..2 + credential_length],
        )
    {
        return Err(Status::INVALID_ARGUMENT)
    }

    let mut record = [0; FIRST_ADMIN_AUTHORIZATION_RECORD_CAPACITY];
    record[0] = FIRST_ADMIN_AUTHORIZATION_RECORD_VERSION;
    let normalized_username = normalized_first_admin_username(&username[..username_read.bytes_read])
        .ok_or_else(|| first_admin_corrupt(5))?;
    record[1] = username_read.bytes_read as u8;
    record[2..2 + username_read.bytes_read]
        .copy_from_slice(&normalized_username[..username_read.bytes_read]);
    record[2 + FIRST_ADMIN_USERNAME_CAPACITY] = credential[0];
    record[3 + FIRST_ADMIN_USERNAME_CAPACITY] = credential_length as u8;
    let material_start = 4 + FIRST_ADMIN_USERNAME_CAPACITY;
    record[material_start..material_start + credential_length]
        .copy_from_slice(&credential[2..2 + credential_length]);

    let mut transaction = daemon.filesystem_mut().transaction();
    match transaction.lookup(AUTHORIZATION_DATABASE_PATH) {
        Ok(_) => return Err(Status::ALREADY_EXISTS),
        Err(ghostos_ghostfs::Error::NotFound) => {}
        Err(ghostos_ghostfs::Error::Corrupt) => return Err(first_admin_corrupt(9)),
        Err(error) => return Err(error.status()),
    }
    let transaction_error_status = |error: ghostos_ghostfs::Error| match error {
        ghostos_ghostfs::Error::Corrupt => first_admin_corrupt(10),
        other => other.status(),
    };
    transaction
        .write(
            AUTHORIZATION_DATABASE_PATH,
            &record[..material_start + credential_length],
        )
        .map_err(transaction_error_status)?;
    transaction
        .delete(FIRST_ADMIN_USERNAME_PATH)
        .map_err(transaction_error_status)?;
    transaction
        .delete(FIRST_ADMIN_CREDENTIAL_PATH)
        .map_err(transaction_error_status)?;
    transaction
        .commit()
        .map_err(transaction_error_status)?;
    if crate::physical_storage::sync(daemon.filesystem_mut()).is_err() {
        FIRST_ADMIN_SYNC_PENDING.store(true, Ordering::Release);
        return Err(Status::INTERNAL)
    }
    FIRST_ADMIN_USERNAME_LENGTH.store(0, Ordering::Release);
    PROVISIONING_REQUIRED.store(false, Ordering::Release);
    Ok(())
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) fn first_admin_recovery(action: u64) -> Result<[u64; 4], Status> {
    let audit_action = match action {
        1 => AUDIT_FIRST_ADMIN_RECOVERY_STATUS,
        2 => AUDIT_FIRST_ADMIN_RECOVERY_RESET,
        3 => AUDIT_FIRST_ADMIN_RECOVERY_RETRY,
        _ => AUDIT_FIRST_ADMIN_RECOVERY_STATUS,
    };
    let result = first_admin_recovery_inner(action);
    audit_first_admin(
        audit_action,
        result.as_ref().map(|_| Status::NORMAL).unwrap_or_else(|error| *error),
    );
    result
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn first_admin_recovery_inner(action: u64) -> Result<[u64; 4], Status> {
    if !PROVISIONING_REQUIRED.load(Ordering::Acquire) {
        return Err(Status::ALREADY_EXISTS)
    }
    let daemon = unsafe {
        (&mut *core::ptr::addr_of_mut!(FILESYSTEM_DAEMON)).assume_init_mut()
    };
    if action == 1 {
        let username = match daemon.filesystem().lookup(FIRST_ADMIN_USERNAME_PATH) {
            Ok(metadata) => u64::from(
                metadata.file_type == ghostos_ghostfs::FileType::Regular
                    && metadata.size != 0,
            ),
            Err(ghostos_ghostfs::Error::NotFound) => 0,
            Err(_) => 2,
        };
        let credential = match daemon.filesystem().lookup(FIRST_ADMIN_CREDENTIAL_PATH) {
            Ok(metadata) => u64::from(
                metadata.file_type == ghostos_ghostfs::FileType::Regular
                    && metadata.size >= 3,
            ),
            Err(ghostos_ghostfs::Error::NotFound) => 0,
            Err(_) => 2,
        };
        return Ok([
            username,
            credential,
            u64::from(FIRST_ADMIN_SYNC_PENDING.load(Ordering::Acquire)),
            1,
        ])
    }
    if action == 3 {
        commit_first_admin()?;
        return Ok([1, 0, 0, 0])
    }
    if action != 2 {
        return Err(Status::INVALID_ARGUMENT)
    }
    if FIRST_ADMIN_RECOVERY_SYNC_PENDING.load(Ordering::Acquire) {
        crate::physical_storage::sync(daemon.filesystem_mut())
            .map_err(|_| Status::INTERNAL)?;
        FIRST_ADMIN_RECOVERY_SYNC_PENDING.store(false, Ordering::Release);
    }
    match daemon.filesystem().lookup(AUTHORIZATION_DATABASE_PATH) {
        Ok(_) => return Err(Status::ALREADY_EXISTS),
        Err(ghostos_ghostfs::Error::NotFound) => {}
        Err(error) => return Err(error.status()),
    }
    let mut transaction = daemon.filesystem_mut().transaction();
    for path in [FIRST_ADMIN_USERNAME_PATH, FIRST_ADMIN_CREDENTIAL_PATH] {
        match transaction.lookup(path) {
            Ok(metadata) if metadata.file_type == ghostos_ghostfs::FileType::Regular => {
                transaction
                    .delete(path)
                    .map_err(|error| error.status())?;
            }
            Ok(_) => return Err(Status::CORRUPT),
            Err(ghostos_ghostfs::Error::NotFound) => {}
            Err(error) => return Err(error.status()),
        }
    }
    transaction
        .commit()
        .map_err(|error| error.status())?;
    if crate::physical_storage::sync(daemon.filesystem_mut()).is_err() {
        FIRST_ADMIN_RECOVERY_SYNC_PENDING.store(true, Ordering::Release);
        return Err(Status::INTERNAL)
    }
    FIRST_ADMIN_USERNAME_LENGTH.store(0, Ordering::Release);
    Ok([0, 0, 0, 0])
}

/// Dispatch a filesystem request issued by the Ring 3 shell.
///
/// The shell owns command parsing and presentation. Ring 0 only validates the
/// user buffer and forwards this already-shaped request to the filesystem
/// daemon with the shell's attenuated authority.
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) fn dispatch_shell_filesystem(
    operation: ghostos_runtime::Operation,
    flags: u16,
    capability: u64,
    offset: u64,
    length: u64,
    buffer: Option<&mut [u8]>,
) -> ghostos_runtime::Response {
    let Some(process) = ghostos_fsd::ProcessId::new(SHELL_PROCESS_ID as u64) else {
        return ghostos_runtime::Response {
            status: Status::ACCESS_DENIED.raw(),
            flags: 0,
            values: [0; 4],
        }
    };
    let Some(fs_operation) = (match operation {
        ghostos_runtime::Operation::SynFsOpen => Some(ghostos_fsd::Operation::Open),
        ghostos_runtime::Operation::SynFsClose => Some(ghostos_fsd::Operation::Close),
        ghostos_runtime::Operation::SynFsRead => Some(ghostos_fsd::Operation::Read),
        ghostos_runtime::Operation::SynFsWrite => Some(ghostos_fsd::Operation::Write),
        ghostos_runtime::Operation::SynFsMap => Some(ghostos_fsd::Operation::Map),
        ghostos_runtime::Operation::SynFsUnmap => Some(ghostos_fsd::Operation::Unmap),
        ghostos_runtime::Operation::SynFsMkdir => Some(ghostos_fsd::Operation::Mkdir),
        ghostos_runtime::Operation::SynFsRmdir => Some(ghostos_fsd::Operation::Rmdir),
        ghostos_runtime::Operation::SynFsList => Some(ghostos_fsd::Operation::List),
        ghostos_runtime::Operation::SynFsDelete => Some(ghostos_fsd::Operation::Delete),
        _ => None,
    }) else {
        return ghostos_runtime::Response {
            status: Status::INVALID_ARGUMENT.raw(),
            flags: 0,
            values: [0; 4],
        }
    };
    let authority = ghostos_fsd::Capability::from_raw(
        SHELL_FILESYSTEM_AUTHORITY.load(Ordering::Acquire),
    );
    let Some(authority) = authority else {
        return ghostos_runtime::Response {
            status: Status::BUSY.raw(),
            flags: 0,
            values: [0; 4],
        }
    };
    let request_capability = match fs_operation {
        ghostos_fsd::Operation::Open
        | ghostos_fsd::Operation::Mkdir
        | ghostos_fsd::Operation::Rmdir
        | ghostos_fsd::Operation::List
        | ghostos_fsd::Operation::Delete => authority,
        _ => match ghostos_fsd::Capability::from_raw(capability) {
            Some(capability) => capability,
            None => authority,
        },
    };
    let request = ghostos_fsd::Request::new(fs_operation, process)
        .with_flags(ghostos_fsd::Flags::from_bits(flags))
        .with_capability(request_capability)
        .with_offset(offset)
        .with_length(length);
    let response = unsafe {
        (&mut *core::ptr::addr_of_mut!(FILESYSTEM_DAEMON)).assume_init_mut()
    }
    .dispatch(request, buffer);
    ghostos_runtime::Response {
        status: response.status.raw(),
        flags: 0,
        values: response.values,
    }
}

/// Bind filesystem authority to the current authenticated shell session.
///
/// Unregistering first closes old shell handles and rotates the authority, so
/// a capability from an expired session cannot work after the next login.
#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
pub(crate) fn set_shell_filesystem_rights(rights: ProcessRights) -> Result<(), Status> {
    let process = ghostos_fsd::ProcessId::new(SHELL_PROCESS_ID).ok_or(Status::INVALID_ARGUMENT)?;
    let previous = SHELL_FILESYSTEM_AUTHORITY.swap(0, Ordering::AcqRel);
    if previous != 0 {
        let daemon = unsafe {
            (&mut *core::ptr::addr_of_mut!(FILESYSTEM_DAEMON)).assume_init_mut()
        };
        daemon
            .unregister_process(process)
            .map_err(|_| Status::ACCESS_DENIED)?;
    }
    if rights.bits() == 0 {
        return Ok(())
    }
    let daemon = unsafe {
        (&mut *core::ptr::addr_of_mut!(FILESYSTEM_DAEMON)).assume_init_mut()
    };
    let authority = daemon
        .register_process(process, rights)
        .map_err(|_| Status::NO_SPACE)?;
    SHELL_FILESYSTEM_AUTHORITY.store(authority.raw(), Ordering::Release);
    Ok(())
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

pub const fn login_service_id() -> ServiceId {
    ServiceId::new(LOGIN_SERVICE_ID).unwrap()
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

const fn login_process_id() -> Option<ProcessId> {
    ProcessId::new(LOGIN_PROCESS_ID)
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
