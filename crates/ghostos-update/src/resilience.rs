#![forbid(unsafe_code)]

//! Boot rollback and rescue operations that do not depend on normal services.

pub const DEFAULT_BOOT_HEALTH_ATTEMPTS: u8 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootHealthState {
    Idle,
    Pending,
    Healthy,
    RollbackRequired,
    RolledBack,
    Recovery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootHealthRecord {
    pub candidate_revision: u64,
    pub rollback_revision: u64,
    pub attempts: u8,
    pub max_attempts: u8,
    pub state: BootHealthState,
}

impl BootHealthRecord {
    pub const fn new(
        candidate_revision: u64,
        rollback_revision: u64,
        max_attempts: u8,
    ) -> Result<Self, BootRollbackError> {
        if candidate_revision == 0 || max_attempts == 0 {
            return Err(BootRollbackError::InvalidRecord)
        }
        Ok(Self {
            candidate_revision,
            rollback_revision,
            attempts: 0,
            max_attempts,
            state: BootHealthState::Pending,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootDecision {
    Continue,
    Retry,
    Rollback,
    EnterRecovery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootRollbackError {
    InvalidRecord,
    NotPending,
    RuntimeFailed,
}

pub trait BootRollbackRuntime {
    type Error;

    fn rollback_to(
        &mut self,
        candidate_revision: u64,
        rollback_revision: u64,
    ) -> Result<(), Self::Error>;
    fn enter_recovery(&mut self) -> Result<(), Self::Error>;
}

/// Persistent boot-health gate. The returned record can be stored beside the
/// boot slot so a power loss does not reset the failed-boot counter.
pub struct BootRollbackController {
    record: BootHealthRecord,
}

impl BootRollbackController {
    pub fn arm(
        candidate_revision: u64,
        rollback_revision: u64,
        max_attempts: u8,
    ) -> Result<Self, BootRollbackError> {
        Ok(Self {
            record: BootHealthRecord::new(candidate_revision, rollback_revision, max_attempts)?,
        })
    }

    pub const fn from_record(record: BootHealthRecord) -> Result<Self, BootRollbackError> {
        if record.candidate_revision == 0
            || record.max_attempts == 0
            || record.attempts > record.max_attempts
        {
            return Err(BootRollbackError::InvalidRecord)
        }
        Ok(Self { record })
    }

    pub const fn record(&self) -> BootHealthRecord {
        self.record
    }

    pub const fn state(&self) -> BootHealthState {
        self.record.state
    }

    pub fn boot_attempt(&mut self) -> Result<BootDecision, BootRollbackError> {
        if self.record.state != BootHealthState::Pending {
            return Err(BootRollbackError::NotPending)
        }
        self.record.attempts = self.record.attempts.saturating_add(1);
        if self.record.attempts > self.record.max_attempts {
            self.record.state = BootHealthState::RollbackRequired;
            return Ok(BootDecision::Rollback)
        }
        Ok(BootDecision::Continue)
    }

    pub fn health_passed(&mut self) -> Result<BootDecision, BootRollbackError> {
        if self.record.state != BootHealthState::Pending {
            return Err(BootRollbackError::NotPending)
        }
        self.record.state = BootHealthState::Healthy;
        Ok(BootDecision::Continue)
    }

    pub fn health_failed(&mut self) -> Result<BootDecision, BootRollbackError> {
        if self.record.state != BootHealthState::Pending {
            return Err(BootRollbackError::NotPending)
        }
        if self.record.attempts >= self.record.max_attempts {
            self.record.state = BootHealthState::RollbackRequired;
            Ok(BootDecision::Rollback)
        } else {
            Ok(BootDecision::Retry)
        }
    }

    pub fn rollback<R: BootRollbackRuntime>(
        &mut self,
        runtime: &mut R,
    ) -> Result<BootDecision, BootRollbackError> {
        if self.record.state != BootHealthState::RollbackRequired {
            return Err(BootRollbackError::NotPending)
        }
        runtime
            .rollback_to(self.record.candidate_revision, self.record.rollback_revision)
            .map_err(|_| BootRollbackError::RuntimeFailed)?;
        self.record.state = BootHealthState::RolledBack;
        Ok(BootDecision::Rollback)
    }

    pub fn enter_recovery<R: BootRollbackRuntime>(
        &mut self,
        runtime: &mut R,
    ) -> Result<BootDecision, BootRollbackError> {
        runtime
            .enter_recovery()
            .map_err(|_| BootRollbackError::RuntimeFailed)?;
        self.record.state = BootHealthState::Recovery;
        Ok(BootDecision::EnterRecovery)
    }
}

impl Default for BootRollbackController {
    fn default() -> Self {
        Self::arm(1, 0, DEFAULT_BOOT_HEALTH_ATTEMPTS).unwrap_or_else(|_| unreachable!())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryStatus {
    pub normal_services_available: bool,
    pub active_revision: u64,
    pub rollback_revision: u64,
    pub backup_available: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryCommand {
    Status,
    Rollback,
    Restore { backup_id: u64 },
    Reboot,
    Continue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryResult {
    Status(RecoveryStatus),
    RollbackStarted,
    RestoreStarted { backup_id: u64 },
    RebootStarted,
    ContinueStarted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryShellError {
    InvalidCommand,
    RuntimeFailed,
}

pub trait RecoveryShellRuntime {
    type Error;

    fn status(&mut self) -> Result<RecoveryStatus, Self::Error>;
    fn rollback(&mut self) -> Result<(), Self::Error>;
    fn restore(&mut self, backup_id: u64) -> Result<(), Self::Error>;
    fn reboot(&mut self) -> Result<(), Self::Error>;
    fn continue_boot(&mut self) -> Result<(), Self::Error>;
}

/// Minimal command surface for a boot where init, storage, or networking is
/// unavailable. The adapter must use boot media, not normal service RPC.
pub struct RecoveryShell {
    command_sequence: u64,
}

impl RecoveryShell {
    pub const fn new() -> Self {
        Self {
            command_sequence: 0,
        }
    }

    pub const fn command_sequence(&self) -> u64 {
        self.command_sequence
    }

    pub fn execute<R: RecoveryShellRuntime>(
        &mut self,
        runtime: &mut R,
        command: RecoveryCommand,
    ) -> Result<RecoveryResult, RecoveryShellError> {
        self.command_sequence = self.command_sequence.saturating_add(1);
        match command {
            RecoveryCommand::Status => runtime
                .status()
                .map(RecoveryResult::Status)
                .map_err(|_| RecoveryShellError::RuntimeFailed),
            RecoveryCommand::Rollback => runtime
                .rollback()
                .map(|_| RecoveryResult::RollbackStarted)
                .map_err(|_| RecoveryShellError::RuntimeFailed),
            RecoveryCommand::Restore { backup_id } if backup_id != 0 => runtime
                .restore(backup_id)
                .map(|_| RecoveryResult::RestoreStarted { backup_id })
                .map_err(|_| RecoveryShellError::RuntimeFailed),
            RecoveryCommand::Restore { .. } => Err(RecoveryShellError::InvalidCommand),
            RecoveryCommand::Reboot => runtime
                .reboot()
                .map(|_| RecoveryResult::RebootStarted)
                .map_err(|_| RecoveryShellError::RuntimeFailed),
            RecoveryCommand::Continue => runtime
                .continue_boot()
                .map(|_| RecoveryResult::ContinueStarted)
                .map_err(|_| RecoveryShellError::RuntimeFailed),
        }
    }
}

impl Default for RecoveryShell {
    fn default() -> Self {
        Self::new()
    }
}
