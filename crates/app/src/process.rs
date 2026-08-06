//! Bounded native process lifecycle and resource policy.

use synos_init::{CrashReason, ExitReason, ProcessId};
use synos_status::{IntoStatus, Status};
use synos_system_model::ContentId;

use crate::loader::{ImageArchitecture, ProcessArguments};

pub const DEFAULT_PROCESS_CAPACITY: usize = 64;
pub const DEFAULT_CANCEL_GRACE_US: u64 = 5_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessLimits {
    pub memory_bytes: u64,
    pub cpu_time_us: u64,
    pub deadline_us: u64,
    pub cancel_grace_us: u64,
}

impl ProcessLimits {
    pub const DEFAULT: Self = Self {
        memory_bytes: 256 * 1024 * 1024,
        cpu_time_us: 10 * 60 * 1_000_000,
        deadline_us: 10 * 60 * 1_000_000,
        cancel_grace_us: DEFAULT_CANCEL_GRACE_US,
    };

    pub fn validate(self) -> Result<(), ProcessError> {
        if self.memory_bytes == 0
            || self.cpu_time_us == 0
            || self.deadline_us == 0
            || self.cancel_grace_us == 0
        {
            return Err(ProcessError::InvalidLimits);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessUsage {
    pub memory_bytes: u64,
    pub cpu_time_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeSpawnRequest<'a> {
    pub image: &'a [u8],
    pub architecture: ImageArchitecture,
    pub expected_payload: Option<ContentId>,
    pub heap_bytes: u64,
    pub arguments: ProcessArguments<'a>,
    pub limits: ProcessLimits,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeExecRequest<'a> {
    pub image: &'a [u8],
    pub architecture: ImageArchitecture,
    pub expected_payload: Option<ContentId>,
    pub heap_bytes: u64,
    pub arguments: ProcessArguments<'a>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessExit {
    pub status: i32,
    pub reason: ExitReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessState {
    Running,
    Cancelling { requested_at_us: u64 },
    Exited,
    Fenced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessStatus {
    pub process: ProcessId,
    pub state: ProcessState,
    pub limits: ProcessLimits,
    pub usage: ProcessUsage,
    pub exit: Option<ProcessExit>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessError {
    Capacity,
    InvalidLimits,
    InvalidArguments,
    NotFound,
    InvalidTransition,
    Backend,
    DeadlineExpired,
    ResourceLimit,
    Cancelled,
}

impl IntoStatus for ProcessError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::InvalidLimits | Self::InvalidArguments | Self::InvalidTransition => {
                Status::INVALID_ARGUMENT
            }
            Self::NotFound => Status::NOT_FOUND,
            Self::Backend | Self::DeadlineExpired | Self::ResourceLimit => Status::BUSY,
            Self::Cancelled => Status::CANCELLED,
        }
    }
}

pub trait ProcessBackend {
    type Error;

    /// The backend loads the ELF image, maps segments, applies relocations,
    /// creates a non-executable stack/TLS, installs argv/env/TLS, and returns
    /// only after the process has a valid entry context.
    fn spawn(&mut self, request: NativeSpawnRequest<'_>) -> Result<ProcessId, Self::Error>;
    fn exec(
        &mut self,
        process: ProcessId,
        request: NativeExecRequest<'_>,
    ) -> Result<(), Self::Error>;
    fn wait(&mut self, process: ProcessId) -> Result<Option<ProcessExit>, Self::Error>;
    fn usage(&mut self, process: ProcessId) -> Result<ProcessUsage, Self::Error>;
    fn request_cancel(&mut self, process: ProcessId) -> Result<(), Self::Error>;
    /// Fence revokes mappings, capabilities, IPC, DMA, and execution before
    /// the process slot can be reused.
    fn fence(&mut self, process: ProcessId) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy)]
struct ProcessSlot {
    process: Option<ProcessId>,
    state: ProcessState,
    limits: ProcessLimits,
    usage: ProcessUsage,
    started_at_us: u64,
    exit: Option<ProcessExit>,
}

impl ProcessSlot {
    const EMPTY: Self = Self {
        process: None,
        state: ProcessState::Exited,
        limits: ProcessLimits::DEFAULT,
        usage: ProcessUsage {
            memory_bytes: 0,
            cpu_time_us: 0,
        },
        started_at_us: 0,
        exit: None,
    };
}

pub struct ProcessSupervisor<const CAPACITY: usize = DEFAULT_PROCESS_CAPACITY> {
    slots: [ProcessSlot; CAPACITY],
}

impl<const CAPACITY: usize> ProcessSupervisor<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            slots: [ProcessSlot::EMPTY; CAPACITY],
        }
    }

    pub fn spawn<B: ProcessBackend>(
        &mut self,
        request: NativeSpawnRequest<'_>,
        now_us: u64,
        backend: &mut B,
    ) -> Result<ProcessId, ProcessError> {
        request
            .limits
            .validate()
            .map_err(|_| ProcessError::InvalidLimits)?;
        request
            .arguments
            .validate()
            .map_err(|_| ProcessError::InvalidArguments)?;
        if self.slots.iter().all(|slot| slot.process.is_some()) {
            return Err(ProcessError::Capacity);
        }
        let process = backend.spawn(request).map_err(|_| ProcessError::Backend)?;
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.process.is_none())
            .ok_or(ProcessError::Capacity)?;
        *slot = ProcessSlot {
            process: Some(process),
            state: ProcessState::Running,
            limits: request.limits,
            usage: ProcessUsage {
                memory_bytes: 0,
                cpu_time_us: 0,
            },
            started_at_us: now_us,
            exit: None,
        };
        Ok(process)
    }

    pub fn exec<B: ProcessBackend>(
        &mut self,
        process: ProcessId,
        request: NativeExecRequest<'_>,
        backend: &mut B,
    ) -> Result<(), ProcessError> {
        let slot = self.slot_mut(process)?;
        if slot.state != ProcessState::Running {
            return Err(ProcessError::InvalidTransition);
        }
        request
            .arguments
            .validate()
            .map_err(|_| ProcessError::InvalidArguments)?;
        backend
            .exec(process, request)
            .map_err(|_| ProcessError::Backend)
    }

    pub fn wait<B: ProcessBackend>(
        &mut self,
        process: ProcessId,
        backend: &mut B,
    ) -> Result<Option<ProcessExit>, ProcessError> {
        let slot = self.slot_mut(process)?;
        if matches!(slot.state, ProcessState::Exited | ProcessState::Fenced) {
            return Ok(slot.exit);
        }
        let exit = backend.wait(process).map_err(|_| ProcessError::Backend)?;
        if let Some(exit) = exit {
            slot.state = ProcessState::Exited;
            slot.exit = Some(exit);
        }
        Ok(exit)
    }

    pub fn cancel<B: ProcessBackend>(
        &mut self,
        process: ProcessId,
        now_us: u64,
        backend: &mut B,
    ) -> Result<(), ProcessError> {
        let slot = self.slot_mut(process)?;
        match slot.state {
            ProcessState::Running => {
                backend
                    .request_cancel(process)
                    .map_err(|_| ProcessError::Backend)?;
                slot.state = ProcessState::Cancelling {
                    requested_at_us: now_us,
                };
                Ok(())
            }
            ProcessState::Cancelling { .. } => Ok(()),
            ProcessState::Exited | ProcessState::Fenced => Err(ProcessError::InvalidTransition),
        }
    }

    pub fn fence<B: ProcessBackend>(
        &mut self,
        process: ProcessId,
        backend: &mut B,
    ) -> Result<(), ProcessError> {
        let slot = self.slot_mut(process)?;
        if slot.state == ProcessState::Fenced {
            return Ok(());
        }
        backend.fence(process).map_err(|_| ProcessError::Backend)?;
        slot.state = ProcessState::Fenced;
        slot.exit = Some(ProcessExit {
            status: -1,
            reason: ExitReason::Crash(CrashReason::Watchdog),
        });
        Ok(())
    }

    /// Refresh usage, enforce memory/CPU/deadline limits, and fence processes
    /// that ignore cooperative cancellation.
    pub fn tick<B: ProcessBackend>(
        &mut self,
        now_us: u64,
        backend: &mut B,
    ) -> Result<(), ProcessError> {
        for index in 0..self.slots.len() {
            let Some(process) = self.slots[index].process else {
                continue;
            };
            if matches!(
                self.slots[index].state,
                ProcessState::Exited | ProcessState::Fenced
            ) {
                continue;
            }
            let usage = backend.usage(process).map_err(|_| ProcessError::Backend)?;
            self.slots[index].usage = usage;
            let elapsed = now_us.saturating_sub(self.slots[index].started_at_us);
            let limits = self.slots[index].limits;
            let over_limit = usage.memory_bytes > limits.memory_bytes
                || usage.cpu_time_us > limits.cpu_time_us
                || elapsed > limits.deadline_us;
            if over_limit && self.slots[index].state == ProcessState::Running {
                self.cancel(process, now_us, backend)?;
            }
            if let ProcessState::Cancelling { requested_at_us } = self.slots[index].state {
                if now_us.saturating_sub(requested_at_us) >= limits.cancel_grace_us {
                    self.fence(process, backend)?;
                }
            }
        }
        Ok(())
    }

    pub fn status(&self, process: ProcessId) -> Result<ProcessStatus, ProcessError> {
        let slot = self
            .slots
            .iter()
            .find(|slot| slot.process == Some(process))
            .ok_or(ProcessError::NotFound)?;
        Ok(ProcessStatus {
            process,
            state: slot.state,
            limits: slot.limits,
            usage: slot.usage,
            exit: slot.exit,
        })
    }

    fn slot_mut(&mut self, process: ProcessId) -> Result<&mut ProcessSlot, ProcessError> {
        self.slots
            .iter_mut()
            .find(|slot| slot.process == Some(process))
            .ok_or(ProcessError::NotFound)
    }
}

impl<const CAPACITY: usize> Default for ProcessSupervisor<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
