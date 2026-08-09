#![no_std]
#![forbid(unsafe_code)]

use synos_status::{IntoStatus, Severity, Status, facility};

pub const DEFAULT_SERVICE_CAPACITY: usize = 32;
pub const MAX_SERVICE_NAME_BYTES: usize = 48;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ServiceId(u32);

impl ServiceId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ProcessId(u64);

impl ProcessId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceName {
    bytes: [u8; MAX_SERVICE_NAME_BYTES],
    len: u8,
}

impl ServiceName {
    pub fn new(name: &str) -> Result<Self, SupervisorError> {
        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes.len() > MAX_SERVICE_NAME_BYTES || bytes.contains(&0) {
            return Err(SupervisorError::InvalidService);
        }
        let mut stored = [0; MAX_SERVICE_NAME_BYTES];
        stored[..bytes.len()].copy_from_slice(bytes);
        Ok(Self {
            bytes: stored,
            len: bytes.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceKind {
    StorageDriver,
    NetworkDriver,
    AcceleratorDriver,
    Compiler,
    System,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestartPolicy {
    pub max_restarts: u16,
    pub window_us: u64,
    pub initial_backoff_us: u64,
    pub max_backoff_us: u64,
}

impl RestartPolicy {
    pub const NEVER: Self = Self {
        max_restarts: 0,
        window_us: 1,
        initial_backoff_us: 0,
        max_backoff_us: 0,
    };

    pub const fn on_failure(
        max_restarts: u16,
        window_us: u64,
        initial_backoff_us: u64,
        max_backoff_us: u64,
    ) -> Result<Self, SupervisorError> {
        if max_restarts == 0
            || window_us == 0
            || initial_backoff_us == 0
            || max_backoff_us < initial_backoff_us
        {
            Err(SupervisorError::InvalidPolicy)
        } else {
            Ok(Self {
                max_restarts,
                window_us,
                initial_backoff_us,
                max_backoff_us,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceSpec {
    pub id: ServiceId,
    pub name: ServiceName,
    pub kind: ServiceKind,
    pub image_id: u128,
    pub capability_profile: u64,
    pub restart: RestartPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpawnRequest {
    pub service: ServiceId,
    pub image_id: u128,
    pub capability_profile: u64,
    pub generation: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CrashReason {
    Panic,
    ProtectionFault,
    IllegalInstruction,
    Watchdog,
    UnexpectedExit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitReason {
    Clean,
    Crash(CrashReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceState {
    Stopped,
    Running,
    Backoff,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceStatus {
    pub spec: ServiceSpec,
    pub state: ServiceState,
    pub process: Option<ProcessId>,
    pub generation: u32,
    pub restart_count: u16,
    pub restart_at_us: u64,
    pub last_exit: Option<ExitReason>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorEvent {
    Started {
        service: ServiceId,
        process: ProcessId,
        generation: u32,
    },
    RestartScheduled {
        service: ServiceId,
        at_us: u64,
        reason: CrashReason,
    },
    Stopped {
        service: ServiceId,
    },
    Failed {
        service: ServiceId,
        reason: CrashReason,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorError {
    AlreadyRegistered,
    Capacity,
    FenceFailed,
    InvalidPolicy,
    InvalidService,
    NotFound,
    SpawnFailed,
    StaleExit,
}

impl IntoStatus for SupervisorError {
    fn status(self) -> Status {
        match self {
            Self::AlreadyRegistered | Self::InvalidPolicy | Self::InvalidService => {
                Status::INVALID_ARGUMENT
            }
            Self::Capacity => Status::NO_SPACE,
            Self::NotFound | Self::StaleExit => Status::NOT_FOUND,
            Self::FenceFailed | Self::SpawnFailed => {
                Status::new(Severity::Error, facility::DRIVER, 1, 0)
                    .unwrap_or(Status::INVALID_ARGUMENT)
            }
        }
    }
}

/// Ring 0 operations used by the Ring 3 supervisor.
///
/// `fence_process` revokes the dead process' capabilities, IPC endpoints, DMA
/// mappings, and interrupts before its replacement is spawned.
pub trait SupervisorRuntime {
    type Error;

    fn spawn(&mut self, request: SpawnRequest) -> Result<ProcessId, Self::Error>;
    fn fence_process(&mut self, process: ProcessId) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy)]
struct ServiceSlot {
    occupied: bool,
    spec: ServiceSpec,
    state: ServiceState,
    process: Option<ProcessId>,
    generation: u32,
    restart_count: u16,
    window_started_us: u64,
    restart_at_us: u64,
    last_exit: Option<ExitReason>,
}

impl ServiceSlot {
    const EMPTY: Self = Self {
        occupied: false,
        spec: ServiceSpec {
            id: ServiceId(0),
            name: ServiceName {
                bytes: [0; MAX_SERVICE_NAME_BYTES],
                len: 0,
            },
            kind: ServiceKind::System,
            image_id: 0,
            capability_profile: 0,
            restart: RestartPolicy::NEVER,
        },
        state: ServiceState::Stopped,
        process: None,
        generation: 0,
        restart_count: 0,
        window_started_us: 0,
        restart_at_us: 0,
        last_exit: None,
    };
}

/// Heap-free supervisor for isolated Ring 3 drivers and system services.
///
/// A crash only changes its owning slot. Other services keep their process,
/// capabilities, and generation.
pub struct Supervisor<const CAPACITY: usize = DEFAULT_SERVICE_CAPACITY> {
    services: [ServiceSlot; CAPACITY],
}

impl<const CAPACITY: usize> Supervisor<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            services: [ServiceSlot::EMPTY; CAPACITY],
        }
    }

    pub fn register(&mut self, spec: ServiceSpec) -> Result<(), SupervisorError> {
        if spec.id.raw() == 0
            || spec.image_id == 0
            || spec.name.len == 0
            || self
                .services
                .iter()
                .any(|slot| slot.occupied && slot.spec.id == spec.id)
        {
            return Err(
                if self
                    .services
                    .iter()
                    .any(|slot| slot.occupied && slot.spec.id == spec.id)
                {
                    SupervisorError::AlreadyRegistered
                } else {
                    SupervisorError::InvalidService
                },
            );
        }
        validate_policy(spec.restart)?;
        let slot = self
            .services
            .iter_mut()
            .find(|slot| !slot.occupied)
            .ok_or(SupervisorError::Capacity)?;
        *slot = ServiceSlot {
            occupied: true,
            spec,
            ..ServiceSlot::EMPTY
        };
        Ok(())
    }

    pub fn start<R: SupervisorRuntime>(
        &mut self,
        id: ServiceId,
        runtime: &mut R,
    ) -> Result<SupervisorEvent, SupervisorError> {
        let slot = self.slot_mut(id)?;
        if slot.state == ServiceState::Running {
            return Err(SupervisorError::AlreadyRegistered);
        }
        spawn(slot, runtime)
    }

    pub fn report_exit<R: SupervisorRuntime>(
        &mut self,
        process: ProcessId,
        reason: ExitReason,
        now_us: u64,
        runtime: &mut R,
    ) -> Result<SupervisorEvent, SupervisorError> {
        let slot = self
            .services
            .iter_mut()
            .find(|slot| slot.occupied && slot.process == Some(process))
            .ok_or(SupervisorError::StaleExit)?;

        runtime
            .fence_process(process)
            .map_err(|_| SupervisorError::FenceFailed)?;
        slot.process = None;
        slot.last_exit = Some(reason);

        let ExitReason::Crash(crash) = reason else {
            slot.state = ServiceState::Stopped;
            return Ok(SupervisorEvent::Stopped {
                service: slot.spec.id,
            });
        };

        if slot.spec.restart.max_restarts == 0 {
            slot.state = ServiceState::Failed;
            return Ok(SupervisorEvent::Failed {
                service: slot.spec.id,
                reason: crash,
            });
        }

        if now_us.saturating_sub(slot.window_started_us) >= slot.spec.restart.window_us {
            slot.window_started_us = now_us;
            slot.restart_count = 0
        }
        if slot.restart_count >= slot.spec.restart.max_restarts {
            slot.state = ServiceState::Failed;
            return Ok(SupervisorEvent::Failed {
                service: slot.spec.id,
                reason: crash,
            });
        }

        let shift = core::cmp::min(slot.restart_count as u32, 63);
        let backoff = slot
            .spec
            .restart
            .initial_backoff_us
            .saturating_mul(1u64 << shift)
            .min(slot.spec.restart.max_backoff_us);
        slot.restart_count += 1;
        slot.restart_at_us = now_us.saturating_add(backoff);
        slot.state = ServiceState::Backoff;
        Ok(SupervisorEvent::RestartScheduled {
            service: slot.spec.id,
            at_us: slot.restart_at_us,
            reason: crash,
        })
    }

    /// Starts one service whose recovery delay has elapsed.
    pub fn tick<R: SupervisorRuntime>(
        &mut self,
        now_us: u64,
        runtime: &mut R,
    ) -> Result<Option<SupervisorEvent>, SupervisorError> {
        let Some(slot) = self.services.iter_mut().find(|slot| {
            slot.occupied && slot.state == ServiceState::Backoff && now_us >= slot.restart_at_us
        }) else {
            return Ok(None);
        };
        spawn(slot, runtime).map(Some)
    }

    pub fn status(&self, id: ServiceId) -> Result<ServiceStatus, SupervisorError> {
        let slot = self
            .services
            .iter()
            .find(|slot| slot.occupied && slot.spec.id == id)
            .ok_or(SupervisorError::NotFound)?;
        Ok(ServiceStatus {
            spec: slot.spec,
            state: slot.state,
            process: slot.process,
            generation: slot.generation,
            restart_count: slot.restart_count,
            restart_at_us: slot.restart_at_us,
            last_exit: slot.last_exit,
        })
    }

    pub fn services(&self) -> impl Iterator<Item = ServiceStatus> + '_ {
        self.services
            .iter()
            .filter(|slot| slot.occupied)
            .map(|slot| ServiceStatus {
                spec: slot.spec,
                state: slot.state,
                process: slot.process,
                generation: slot.generation,
                restart_count: slot.restart_count,
                restart_at_us: slot.restart_at_us,
                last_exit: slot.last_exit,
            })
    }

    fn slot_mut(&mut self, id: ServiceId) -> Result<&mut ServiceSlot, SupervisorError> {
        self.services
            .iter_mut()
            .find(|slot| slot.occupied && slot.spec.id == id)
            .ok_or(SupervisorError::NotFound)
    }
}

impl<const CAPACITY: usize> Default for Supervisor<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_policy(policy: RestartPolicy) -> Result<(), SupervisorError> {
    if policy == RestartPolicy::NEVER
        || (policy.max_restarts != 0
            && policy.window_us != 0
            && policy.initial_backoff_us != 0
            && policy.max_backoff_us >= policy.initial_backoff_us)
    {
        Ok(())
    } else {
        Err(SupervisorError::InvalidPolicy)
    }
}

fn spawn<R: SupervisorRuntime>(
    slot: &mut ServiceSlot,
    runtime: &mut R,
) -> Result<SupervisorEvent, SupervisorError> {
    let generation = slot.generation.wrapping_add(1).max(1);
    let process = runtime
        .spawn(SpawnRequest {
            service: slot.spec.id,
            image_id: slot.spec.image_id,
            capability_profile: slot.spec.capability_profile,
            generation,
        })
        .map_err(|_| SupervisorError::SpawnFailed)?;
    slot.generation = generation;
    slot.process = Some(process);
    slot.state = ServiceState::Running;
    Ok(SupervisorEvent::Started {
        service: slot.spec.id,
        process,
        generation,
    })
}
