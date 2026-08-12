//! Services started by the first user-space boot sequence.

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, Ordering};

use synos_fsd::{Daemon, ProcessRights};
use synos_init::{
    LifecycleEvent, ProcessId, RestartPolicy, ServiceId, ServiceKind, ServiceName, ServiceSpec,
    SpawnRequest, Supervisor, SupervisorRuntime,
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
const FILESYSTEM_BLOCKS: usize = 64;

type FilesystemDaemon = Daemon<FILESYSTEM_BLOCKS>;

static mut FILESYSTEM_DAEMON: MaybeUninit<FilesystemDaemon> = MaybeUninit::uninit();
static FILESYSTEM_READY: AtomicBool = AtomicBool::new(false);
static STORAGE_READY: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
pub struct BootServices {
    pub filesystem_process: ProcessId,
    pub storage_process: ProcessId,
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
        Err(StartError::Process)
    }

    fn fence_process(&mut self, process: ProcessId) -> Result<(), Self::Error> {
        (process == self.filesystem_process || process == self.storage_process)
            .then_some(())
            .ok_or(StartError::Process)
    }
}

pub fn start() -> Result<BootServices, StartError> {
    if FILESYSTEM_READY.load(Ordering::Acquire) && STORAGE_READY.load(Ordering::Acquire) {
        return Ok(BootServices {
            filesystem_process: filesystem_process_id().ok_or(StartError::Process)?,
            storage_process: storage_process_id().ok_or(StartError::Process)?,
        })
    }

    let filesystem_process = filesystem_process_id().ok_or(StartError::Process)?;
    let storage_process = storage_process_id().ok_or(StartError::Process)?;
    let mut daemon = Daemon::new(SynFs::<FILESYSTEM_BLOCKS>::new())
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
    let mut supervisor = Supervisor::<2>::new();
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

    let mut runtime = BootRuntime {
        filesystem_process,
        storage_process,
    };
    let trace = supervisor
        .boot(&mut runtime)
        .map_err(|_| StartError::Supervisor)?;
    let started_services = trace
        .events()
        .filter(|event| matches!(event, LifecycleEvent::ServiceStarted { .. }))
        .count();
    if started_services != 2 {
        return Err(StartError::Supervisor)
    }

    // The daemon owns its namespace and capabilities for the lifetime of the
    // service. Publish it only after all startup checks have succeeded.
    unsafe {
        (&mut *core::ptr::addr_of_mut!(FILESYSTEM_DAEMON)).write(daemon);
    }
    FILESYSTEM_READY.store(true, Ordering::Release);
    STORAGE_READY.store(true, Ordering::Release);
    Ok(BootServices {
        filesystem_process,
        storage_process,
    })
}

pub const fn filesystem_service_id() -> ServiceId {
    ServiceId::new(FILESYSTEM_SERVICE_ID).unwrap()
}

pub const fn storage_service_id() -> ServiceId {
    ServiceId::new(STORAGE_SERVICE_ID).unwrap()
}

const fn filesystem_process_id() -> Option<ProcessId> {
    ProcessId::new(FILESYSTEM_PROCESS_ID)
}

const fn storage_process_id() -> Option<ProcessId> {
    ProcessId::new(STORAGE_PROCESS_ID)
}
