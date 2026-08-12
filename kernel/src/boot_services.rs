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
const FILESYSTEM_BLOCKS: usize = 64;

type FilesystemDaemon = Daemon<FILESYSTEM_BLOCKS>;

static mut FILESYSTEM_DAEMON: MaybeUninit<FilesystemDaemon> = MaybeUninit::uninit();
static FILESYSTEM_READY: AtomicBool = AtomicBool::new(false);

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
    process: ProcessId,
}

impl SupervisorRuntime for BootRuntime {
    type Error = StartError;

    fn spawn(&mut self, request: SpawnRequest) -> Result<ProcessId, Self::Error> {
        if request.service != service_id() {
            return Err(StartError::Process)
        }
        Ok(self.process)
    }

    fn fence_process(&mut self, process: ProcessId) -> Result<(), Self::Error> {
        (process == self.process)
            .then_some(())
            .ok_or(StartError::Process)
    }
}

pub fn start() -> Result<ProcessId, StartError> {
    if FILESYSTEM_READY.load(Ordering::Acquire) {
        return process_id().ok_or(StartError::Process)
    }

    let process = process_id().ok_or(StartError::Process)?;
    let mut daemon = Daemon::new(SynFs::<FILESYSTEM_BLOCKS>::new())
        .map_err(|_| StartError::Daemon)?;
    daemon
        .register_process(
            synos_fsd::ProcessId::new(process.raw()).ok_or(StartError::Process)?,
            ProcessRights::from_bits(
                ProcessRights::READ.bits()
                    | ProcessRights::WRITE.bits()
                    | ProcessRights::DELETE.bits()
                    | ProcessRights::ADMIN.bits(),
            ),
        )
        .map_err(|_| StartError::Daemon)?;

    let name = ServiceName::new("synos-fsd").map_err(|_| StartError::Supervisor)?;
    let mut supervisor = Supervisor::<1>::new();
    supervisor
        .register(ServiceSpec {
            id: service_id(),
            name,
            kind: ServiceKind::System,
            image_id: FILESYSTEM_IMAGE_ID,
            capability_profile: FILESYSTEM_CAPABILITY_PROFILE,
            restart: RestartPolicy::on_failure(3, 60_000_000, 100_000, 5_000_000)
                .map_err(|_| StartError::Supervisor)?,
        })
        .map_err(|_| StartError::Supervisor)?;

    let mut runtime = BootRuntime { process };
    let event = supervisor
        .start(service_id(), &mut runtime)
        .map_err(|_| StartError::Supervisor)?;
    if !matches!(event, SupervisorEvent::Started { .. }) {
        return Err(StartError::Supervisor)
    }

    // The daemon owns its namespace and capabilities for the lifetime of the
    // service. Publish it only after all startup checks have succeeded.
    unsafe {
        (&mut *core::ptr::addr_of_mut!(FILESYSTEM_DAEMON)).write(daemon);
    }
    FILESYSTEM_READY.store(true, Ordering::Release);
    Ok(process)
}

pub const fn service_id() -> ServiceId {
    ServiceId::new(FILESYSTEM_SERVICE_ID).unwrap()
}

const fn process_id() -> Option<ProcessId> {
    ProcessId::new(FILESYSTEM_PROCESS_ID)
}
