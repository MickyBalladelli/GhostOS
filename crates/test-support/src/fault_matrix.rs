//! Deterministic fault-injection matrix and recovery evidence.
//!
//! Production workflows call [`FaultInjectionController::checkpoint`] from
//! their test adapter. The adapter records a [`RecoveryEvidence`] value after
//! the workflow reaches a stable normal or degraded state. The matrix rejects
//! observations that exceed the declared RTO or lose data, capabilities, audit
//! continuity, or duplicate-side-effect safety.

use std::fmt;

pub const WORKFLOW_COUNT: usize = 8;
pub const FAULT_COUNT: usize = 9;
pub const MATRIX_SIZE: usize = WORKFLOW_COUNT * FAULT_COUNT;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u8)]
pub enum Workflow {
    Boot = 0,
    Store = 1,
    Backup = 2,
    Package = 3,
    Rpc = 4,
    Cluster = 5,
    Ai = 6,
    Device = 7,
}

impl Workflow {
    pub const ALL: [Self; WORKFLOW_COUNT] = [
        Self::Boot,
        Self::Store,
        Self::Backup,
        Self::Package,
        Self::Rpc,
        Self::Cluster,
        Self::Ai,
        Self::Device,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Boot => "BOOT",
            Self::Store => "STORE",
            Self::Backup => "BACKUP",
            Self::Package => "PKG",
            Self::Rpc => "RPC",
            Self::Cluster => "CLUSTER",
            Self::Ai => "AI",
            Self::Device => "DEVICE",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u8)]
pub enum Fault {
    PowerLoss = 0,
    DiskFull = 1,
    DeviceReset = 2,
    PacketLoss = 3,
    Partition = 4,
    ClockJump = 5,
    ProcessHang = 6,
    CorruptInput = 7,
    DependencyOutage = 8,
}

impl Fault {
    pub const ALL: [Self; FAULT_COUNT] = [
        Self::PowerLoss,
        Self::DiskFull,
        Self::DeviceReset,
        Self::PacketLoss,
        Self::Partition,
        Self::ClockJump,
        Self::ProcessHang,
        Self::CorruptInput,
        Self::DependencyOutage,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::PowerLoss => "power-loss",
            Self::DiskFull => "disk-full",
            Self::DeviceReset => "device-reset",
            Self::PacketLoss => "packet-loss",
            Self::Partition => "partition",
            Self::ClockJump => "clock-jump",
            Self::ProcessHang => "process-hang",
            Self::CorruptInput => "corrupt-input",
            Self::DependencyOutage => "dependency-outage",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DegradedMode {
    Recovery,
    ReadOnly,
    Paused,
    Reconnect,
    QuorumReadOnly,
    Offline,
    FailClosed,
    Restart,
    Rejected,
    LocalOnly,
    Quarantined,
    Degraded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryAction {
    KeepPrior,
    ResumeCheckpoint,
    RejectNewWork,
    ReconnectIdempotent,
    FenceAndQuorumRead,
    ReenumerateOffline,
    PauseAndRetry,
    RetryBounded,
    RetainStaged,
    UseMirror,
    LocalOnly,
    MonotonicGuard,
    RestartGated,
    RejectInput,
    QuarantineInput,
    IsolateDependency,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DegradedBehavior {
    pub mode: DegradedMode,
    pub action: RecoveryAction,
    pub detail: &'static str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryCell {
    pub fault: Fault,
    pub workflow: Workflow,
    pub rto_budget_ms: u64,
    pub behavior: DegradedBehavior,
}

impl RecoveryCell {
    pub const fn for_pair(fault: Fault, workflow: Workflow) -> Self {
        Self {
            fault,
            workflow,
            rto_budget_ms: RTO_BUDGETS_MS[fault.index()][workflow.index()],
            behavior: behavior(fault, workflow),
        }
    }
}

pub const RTO_BUDGETS_MS: [[u64; WORKFLOW_COUNT]; FAULT_COUNT] = [
    [60_000, 30_000, 90_000, 60_000, 30_000, 120_000, 90_000, 60_000],
    [30_000, 5_000, 15_000, 15_000, 5_000, 30_000, 15_000, 30_000],
    [60_000, 30_000, 60_000, 30_000, 15_000, 120_000, 60_000, 30_000],
    [30_000, 30_000, 60_000, 60_000, 15_000, 120_000, 60_000, 30_000],
    [30_000, 30_000, 60_000, 60_000, 15_000, 120_000, 60_000, 30_000],
    [30_000, 10_000, 30_000, 30_000, 15_000, 120_000, 60_000, 30_000],
    [30_000, 30_000, 60_000, 60_000, 15_000, 120_000, 60_000, 30_000],
    [30_000, 5_000, 30_000, 30_000, 5_000, 15_000, 15_000, 15_000],
    [60_000, 30_000, 60_000, 60_000, 15_000, 120_000, 60_000, 30_000],
];

const fn behavior(fault: Fault, workflow: Workflow) -> DegradedBehavior {
    use DegradedMode::*;
    use RecoveryAction::*;

    match fault {
        Fault::PowerLoss => match workflow {
            Workflow::Boot => DegradedBehavior { mode: Recovery, action: KeepPrior, detail: "boot last valid image" },
            Workflow::Store => DegradedBehavior { mode: Recovery, action: KeepPrior, detail: "discard partial generation and keep prior" },
            Workflow::Backup => DegradedBehavior { mode: Recovery, action: ResumeCheckpoint, detail: "resume from checkpoint with source unchanged" },
            Workflow::Package => DegradedBehavior { mode: Recovery, action: KeepPrior, detail: "keep old activation" },
            Workflow::Rpc => DegradedBehavior { mode: Reconnect, action: ReconnectIdempotent, detail: "reconnect without duplicate effects" },
            Workflow::Cluster => DegradedBehavior { mode: QuorumReadOnly, action: FenceAndQuorumRead, detail: "fence stale leases and serve quorum reads" },
            Workflow::Ai => DegradedBehavior { mode: Recovery, action: ResumeCheckpoint, detail: "resume last committed checkpoint" },
            Workflow::Device => DegradedBehavior { mode: Offline, action: ReenumerateOffline, detail: "re-enumerate and mark missing device offline" },
        },
        Fault::DiskFull => match workflow {
            Workflow::Boot => DegradedBehavior { mode: Recovery, action: KeepPrior, detail: "reserve recovery space and open admin shell" },
            Workflow::Store => DegradedBehavior { mode: ReadOnly, action: RejectNewWork, detail: "reject writes and preserve reads and old generations" },
            Workflow::Backup => DegradedBehavior { mode: Paused, action: PauseAndRetry, detail: "pause before commit and retry after space is available" },
            Workflow::Package => DegradedBehavior { mode: ReadOnly, action: RejectNewWork, detail: "refuse activation and keep old package" },
            Workflow::Rpc => DegradedBehavior { mode: ReadOnly, action: RejectNewWork, detail: "reject uploads while reads continue" },
            Workflow::Cluster => DegradedBehavior { mode: QuorumReadOnly, action: FenceAndQuorumRead, detail: "block membership writes and serve quorum reads" },
            Workflow::Ai => DegradedBehavior { mode: Paused, action: RejectNewWork, detail: "stop new checkpoints and jobs; resume old checkpoint" },
            Workflow::Device => DegradedBehavior { mode: Degraded, action: RejectNewWork, detail: "reject new state and keep device inspectable" },
        },
        Fault::DeviceReset => match workflow {
            Workflow::Boot => DegradedBehavior { mode: Recovery, action: RetryBounded, detail: "retry boundedly and enter safe mode if needed" },
            Workflow::Store => DegradedBehavior { mode: Degraded, action: KeepPrior, detail: "fence device and use prior generation and degraded pool" },
            Workflow::Backup => DegradedBehavior { mode: Recovery, action: ResumeCheckpoint, detail: "fail stream safely and resume by chunk" },
            Workflow::Package => DegradedBehavior { mode: Recovery, action: RetainStaged, detail: "retain staged bytes and do not activate" },
            Workflow::Rpc => DegradedBehavior { mode: Reconnect, action: ReconnectIdempotent, detail: "reconnect transport with idempotent retry" },
            Workflow::Cluster => DegradedBehavior { mode: QuorumReadOnly, action: FenceAndQuorumRead, detail: "suspect node and fence ownership" },
            Workflow::Ai => DegradedBehavior { mode: Recovery, action: UseMirror, detail: "use mirror and resume checkpoint" },
            Workflow::Device => DegradedBehavior { mode: Offline, action: ReenumerateOffline, detail: "reinitialize and keep device offline on failure" },
        },
        Fault::PacketLoss => match workflow {
            Workflow::Boot => DegradedBehavior { mode: DegradedMode::LocalOnly, action: RecoveryAction::LocalOnly, detail: "local readiness does not wait on network" },
            Workflow::Store => DegradedBehavior { mode: Recovery, action: RetryBounded, detail: "publish only after durable acknowledgement and serve old generation" },
            Workflow::Backup => DegradedBehavior { mode: Paused, action: RetryBounded, detail: "retry bounded chunks and keep checkpoint" },
            Workflow::Package => DegradedBehavior { mode: Recovery, action: RetryBounded, detail: "retry manifest and keep old version" },
            Workflow::Rpc => DegradedBehavior { mode: Reconnect, action: ReconnectIdempotent, detail: "retry one idempotent request then timeout" },
            Workflow::Cluster => DegradedBehavior { mode: QuorumReadOnly, action: FenceAndQuorumRead, detail: "require quorum and prevent split brain" },
            Workflow::Ai => DegradedBehavior { mode: Paused, action: ResumeCheckpoint, detail: "wait for dual journal acknowledgement and resume last commit" },
            Workflow::Device => DegradedBehavior { mode: Degraded, action: RetryBounded, detail: "bounded control retry and mark link degraded" },
        },
        Fault::Partition => match workflow {
            Workflow::Boot => DegradedBehavior { mode: Degraded, action: RecoveryAction::LocalOnly, detail: "boot local services and show dependency degraded" },
            Workflow::Store => DegradedBehavior { mode: ReadOnly, action: RecoveryAction::LocalOnly, detail: "allow local durable writes and reject cluster-backed writes" },
            Workflow::Backup => DegradedBehavior { mode: Paused, action: PauseAndRetry, detail: "keep local snapshot and pause remote upload" },
            Workflow::Package => DegradedBehavior { mode: Recovery, action: KeepPrior, detail: "use cached signed artifacts and avoid cluster activation" },
            Workflow::Rpc => DegradedBehavior { mode: DegradedMode::LocalOnly, action: RecoveryAction::LocalOnly, detail: "serve local routes and report remote unavailable" },
            Workflow::Cluster => DegradedBehavior { mode: QuorumReadOnly, action: FenceAndQuorumRead, detail: "make minority read-only and fence for rejoin" },
            Workflow::Ai => DegradedBehavior { mode: DegradedMode::LocalOnly, action: UseMirror, detail: "continue with local mirror and stop without quorum" },
            Workflow::Device => DegradedBehavior { mode: Offline, action: IsolateDependency, detail: "isolate remote device while local devices continue" },
        },
        Fault::ClockJump => match workflow {
            Workflow::Boot => DegradedBehavior { mode: Degraded, action: MonotonicGuard, detail: "use monotonic time and delay network readiness" },
            Workflow::Store => DegradedBehavior { mode: Degraded, action: MonotonicGuard, detail: "use monotonic deadlines with unchanged commit semantics" },
            Workflow::Backup => DegradedBehavior { mode: Paused, action: MonotonicGuard, detail: "use monotonic schedule and pause expiry decisions" },
            Workflow::Package => DegradedBehavior { mode: FailClosed, action: MonotonicGuard, detail: "fail closed when wall time is untrusted" },
            Workflow::Rpc => DegradedBehavior { mode: FailClosed, action: MonotonicGuard, detail: "use monotonic timeout and fail closed on auth expiry" },
            Workflow::Cluster => DegradedBehavior { mode: QuorumReadOnly, action: MonotonicGuard, detail: "use bounded monotonic leases and stop unsafe renewal" },
            Workflow::Ai => DegradedBehavior { mode: Recovery, action: MonotonicGuard, detail: "use monotonic checkpoints and lose unsafe lease safely" },
            Workflow::Device => DegradedBehavior { mode: Degraded, action: MonotonicGuard, detail: "use monotonic debounce and never detach unsafely" },
        },
        Fault::ProcessHang => match workflow {
            Workflow::Boot => DegradedBehavior { mode: Restart, action: RestartGated, detail: "watchdog restart while readiness stays gated" },
            Workflow::Store => DegradedBehavior { mode: Recovery, action: KeepPrior, detail: "fence daemon and serve last valid root" },
            Workflow::Backup => DegradedBehavior { mode: Recovery, action: ResumeCheckpoint, detail: "stop worker while checkpoint remains usable" },
            Workflow::Package => DegradedBehavior { mode: Recovery, action: KeepPrior, detail: "abort activation and keep old package live" },
            Workflow::Rpc => DegradedBehavior { mode: Restart, action: RestartGated, detail: "restart handler and prevent request replay" },
            Workflow::Cluster => DegradedBehavior { mode: QuorumReadOnly, action: FenceAndQuorumRead, detail: "mark node suspect and fence leases" },
            Workflow::Ai => DegradedBehavior { mode: Recovery, action: ResumeCheckpoint, detail: "restart from checkpoint or snapshot" },
            Workflow::Device => DegradedBehavior { mode: Restart, action: RestartGated, detail: "restart driver and keep device offline if hung" },
        },
        Fault::CorruptInput => match workflow {
            Workflow::Boot => DegradedBehavior { mode: Recovery, action: RejectInput, detail: "reject image or config and enter recovery path" },
            Workflow::Store => DegradedBehavior { mode: Recovery, action: RejectInput, detail: "reject before publish and preserve old generation" },
            Workflow::Backup => DegradedBehavior { mode: Quarantined, action: QuarantineInput, detail: "quarantine bad chunk while source stays intact" },
            Workflow::Package => DegradedBehavior { mode: Rejected, action: RejectInput, detail: "reject digest or signature and keep old activation" },
            Workflow::Rpc => DegradedBehavior { mode: Rejected, action: RejectInput, detail: "return stable invalid-input error and keep connection" },
            Workflow::Cluster => DegradedBehavior { mode: Rejected, action: RejectInput, detail: "reject frame and leave peer state unchanged" },
            Workflow::Ai => DegradedBehavior { mode: Rejected, action: RejectInput, detail: "reject tensor, tool, or snapshot without mutation" },
            Workflow::Device => DegradedBehavior { mode: Rejected, action: RejectInput, detail: "reject descriptor and reset endpoint" },
        },
        Fault::DependencyOutage => match workflow {
            Workflow::Boot => DegradedBehavior { mode: Degraded, action: RecoveryAction::LocalOnly, detail: "start independent services and expose degraded state" },
            Workflow::Store => DegradedBehavior { mode: ReadOnly, action: RejectNewWork, detail: "be read-only on missing dependency and retain old root" },
            Workflow::Backup => DegradedBehavior { mode: Paused, action: RetainStaged, detail: "retain checkpoint and stage locally" },
            Workflow::Package => DegradedBehavior { mode: Paused, action: KeepPrior, detail: "use cached artifacts and defer activation" },
            Workflow::Rpc => DegradedBehavior { mode: Degraded, action: IsolateDependency, detail: "return unavailable while unrelated routes continue" },
            Workflow::Cluster => DegradedBehavior { mode: QuorumReadOnly, action: FenceAndQuorumRead, detail: "apply quorum rules and reject unsafe writes" },
            Workflow::Ai => DegradedBehavior { mode: Paused, action: RecoveryAction::LocalOnly, detail: "use local model and checkpoint; stop without journal" },
            Workflow::Device => DegradedBehavior { mode: Degraded, action: IsolateDependency, detail: "isolate failed dependency while unaffected devices run" },
        },
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FaultTarget {
    pub fault: Fault,
    pub workflow: Workflow,
}

impl FaultTarget {
    pub const fn new(fault: Fault, workflow: Workflow) -> Self {
        Self { fault, workflow }
    }

    pub const fn cell(self) -> RecoveryCell {
        RecoveryCell::for_pair(self.fault, self.workflow)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FaultInjected {
    pub target: FaultTarget,
    pub detected_at_ms: u64,
}

impl fmt::Display for FaultInjected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} injected in {} at {}ms",
            self.target.fault.name(),
            self.target.workflow.name(),
            self.detected_at_ms
        )
    }
}

impl std::error::Error for FaultInjected {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FaultInjectionController {
    target: FaultTarget,
    fired: bool,
}

impl FaultInjectionController {
    pub const fn new(target: FaultTarget) -> Self {
        Self {
            target,
            fired: false,
        }
    }

    pub const fn target(self) -> FaultTarget {
        self.target
    }

    pub const fn fired(self) -> bool {
        self.fired
    }

    pub fn checkpoint(
        &mut self,
        fault: Fault,
        workflow: Workflow,
        detected_at_ms: u64,
    ) -> Result<(), FaultInjected> {
        if !self.fired && self.target == FaultTarget::new(fault, workflow) {
            self.fired = true;
            return Err(FaultInjected {
                target: self.target,
                detected_at_ms,
            })
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryEvidence {
    pub target: FaultTarget,
    pub detected_at_ms: u64,
    pub stable_at_ms: u64,
    pub behavior: DegradedBehavior,
    pub stable: bool,
    pub data_continuity: bool,
    pub capability_continuity: bool,
    pub stale_capabilities_fenced: bool,
    pub audit_continuity: bool,
    pub duplicate_side_effects: bool,
}

impl RecoveryEvidence {
    pub const fn observed(
        target: FaultTarget,
        detected_at_ms: u64,
        stable_at_ms: u64,
        behavior: DegradedBehavior,
        stable: bool,
        data_continuity: bool,
        capability_continuity: bool,
        stale_capabilities_fenced: bool,
        audit_continuity: bool,
        duplicate_side_effects: bool,
    ) -> Self {
        Self {
            target,
            detected_at_ms,
            stable_at_ms,
            behavior,
            stable,
            data_continuity,
            capability_continuity,
            stale_capabilities_fenced,
            audit_continuity,
            duplicate_side_effects,
        }
    }

    pub const fn observed_rto_ms(self) -> u64 {
        self.stable_at_ms.saturating_sub(self.detected_at_ms)
    }

    pub const fn cell(self) -> RecoveryCell {
        self.target.cell()
    }

    pub fn validate(self) -> Result<(), MatrixError> {
        let cell = self.cell();
        if self.stable_at_ms < self.detected_at_ms {
            return Err(MatrixError::ClockReversed)
        }
        if !self.stable {
            return Err(MatrixError::NotStable)
        }
        if self.behavior != cell.behavior {
            return Err(MatrixError::BehaviorMismatch)
        }
        if self.observed_rto_ms() > cell.rto_budget_ms {
            return Err(MatrixError::RtoExceeded {
                observed_ms: self.observed_rto_ms(),
                budget_ms: cell.rto_budget_ms,
            })
        }
        if !self.data_continuity {
            return Err(MatrixError::DataContinuityLost)
        }
        if !self.capability_continuity || !self.stale_capabilities_fenced {
            return Err(MatrixError::CapabilityContinuityLost)
        }
        if !self.audit_continuity {
            return Err(MatrixError::AuditContinuityLost)
        }
        if self.duplicate_side_effects {
            return Err(MatrixError::DuplicateSideEffect)
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixError {
    ClockReversed,
    NotStable,
    BehaviorMismatch,
    RtoExceeded { observed_ms: u64, budget_ms: u64 },
    DataContinuityLost,
    CapabilityContinuityLost,
    AuditContinuityLost,
    DuplicateSideEffect,
    DuplicateEvidence,
    Capacity,
    Incomplete { recorded: usize, expected: usize },
}

impl fmt::Display for MatrixError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClockReversed => write!(formatter, "recovery clock moved backwards"),
            Self::NotStable => write!(formatter, "workflow did not reach a stable state"),
            Self::BehaviorMismatch => write!(formatter, "observed degraded behavior does not match the matrix"),
            Self::RtoExceeded { observed_ms, budget_ms } => write!(formatter, "recovery RTO {observed_ms}ms exceeded {budget_ms}ms"),
            Self::DataContinuityLost => write!(formatter, "data continuity was lost"),
            Self::CapabilityContinuityLost => write!(formatter, "capability continuity or stale fencing was lost"),
            Self::AuditContinuityLost => write!(formatter, "audit continuity was lost"),
            Self::DuplicateSideEffect => write!(formatter, "recovery duplicated a side effect"),
            Self::DuplicateEvidence => write!(formatter, "recovery evidence was recorded twice"),
            Self::Capacity => write!(formatter, "fault matrix evidence capacity is too small"),
            Self::Incomplete { recorded, expected } => write!(formatter, "fault matrix has {recorded} of {expected} cells"),
        }
    }
}

impl std::error::Error for MatrixError {}

#[derive(Clone, Copy, Debug)]
pub struct MatrixCells {
    next: usize,
}

impl Iterator for MatrixCells {
    type Item = RecoveryCell;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= MATRIX_SIZE {
            return None
        }
        let fault = Fault::ALL[self.next / WORKFLOW_COUNT];
        let workflow = Workflow::ALL[self.next % WORKFLOW_COUNT];
        self.next += 1;
        Some(RecoveryCell::for_pair(fault, workflow))
    }
}

pub fn matrix() -> MatrixCells {
    MatrixCells { next: 0 }
}

#[derive(Clone, Debug)]
pub struct FaultMatrix<const CAPACITY: usize = MATRIX_SIZE> {
    evidence: [Option<RecoveryEvidence>; CAPACITY],
    recorded: usize,
}

impl<const CAPACITY: usize> FaultMatrix<CAPACITY> {
    pub fn new() -> Self {
        Self {
            evidence: [None; CAPACITY],
            recorded: 0,
        }
    }

    pub fn record(&mut self, evidence: RecoveryEvidence) -> Result<(), MatrixError> {
        evidence.validate()?;
        let index = evidence.target.fault.index() * WORKFLOW_COUNT
            + evidence.target.workflow.index();
        let slot = self.evidence.get_mut(index).ok_or(MatrixError::Capacity)?;
        if slot.is_some() {
            return Err(MatrixError::DuplicateEvidence)
        }
        *slot = Some(evidence);
        self.recorded += 1;
        Ok(())
    }

    pub fn complete(&self) -> Result<(), MatrixError> {
        if self.recorded != MATRIX_SIZE {
            return Err(MatrixError::Incomplete {
                recorded: self.recorded,
                expected: MATRIX_SIZE,
            })
        }
        Ok(())
    }

    pub const fn recorded(&self) -> usize {
        self.recorded
    }

    pub fn evidence(&self) -> &[Option<RecoveryEvidence>] {
        &self.evidence
    }
}

impl<const CAPACITY: usize> Default for FaultMatrix<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
