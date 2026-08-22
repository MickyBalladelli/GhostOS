#![forbid(unsafe_code)]

//! Coordinated, bounded maintenance drain.
//!
//! The coordinator stops admission, requests handoff, waits on a monotonic
//! deadline, and then force-cleans every resource class. Concrete supervisors,
//! network daemons, storage managers, terminal services, and cluster managers
//! implement [`DrainRuntime`]. A drain completes only when no resource, live
//! lock, or partial publication remains.

pub const DRAIN_RESOURCE_COUNT: usize = 6;
pub const DEFAULT_DRAIN_TIMEOUT_MS: u64 = 30_000;
pub const DEFAULT_FORCE_TIMEOUT_MS: u64 = 5_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DrainResource {
    Process = 0,
    Socket = 1,
    Queue = 2,
    StorageLease = 3,
    TerminalSession = 4,
    ClusterOwnership = 5,
}

impl DrainResource {
    pub const ALL: [Self; DRAIN_RESOURCE_COUNT] = [
        Self::Process,
        Self::Socket,
        Self::Queue,
        Self::StorageLease,
        Self::TerminalSession,
        Self::ClusterOwnership,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrainTarget {
    pub owner: u64,
    pub generation: u64,
}

impl DrainTarget {
    pub const fn new(owner: u64, generation: u64) -> Option<Self> {
        if owner == 0 || generation == 0 {
            None
        } else {
            Some(Self { owner, generation })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrainPlan {
    pub drain_timeout_ms: u64,
    pub force_timeout_ms: u64,
}

impl DrainPlan {
    pub const fn new(drain_timeout_ms: u64, force_timeout_ms: u64) -> Option<Self> {
        if drain_timeout_ms == 0 || force_timeout_ms == 0 {
            None
        } else {
            Some(Self {
                drain_timeout_ms,
                force_timeout_ms,
            })
        }
    }

    pub const fn defaults() -> Self {
        Self {
            drain_timeout_ms: DEFAULT_DRAIN_TIMEOUT_MS,
            force_timeout_ms: DEFAULT_FORCE_TIMEOUT_MS,
        }
    }
}

impl Default for DrainPlan {
    fn default() -> Self {
        Self::defaults()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct DrainObservation {
    pub processes: u32,
    pub sockets: u32,
    pub queues: u32,
    pub storage_leases: u32,
    pub terminal_sessions: u32,
    pub cluster_ownership: u32,
    pub live_locks: bool,
    pub partial_publications: bool,
}

impl DrainObservation {
    pub const fn resources_empty(self) -> bool {
        self.processes == 0
            && self.sockets == 0
            && self.queues == 0
            && self.storage_leases == 0
            && self.terminal_sessions == 0
            && self.cluster_ownership == 0
    }

    pub const fn clean(self) -> bool {
        self.resources_empty() && !self.live_locks && !self.partial_publications
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ForcedCleanup {
    pub stale_capabilities_fenced: bool,
    pub ownership_released: bool,
    pub live_locks: bool,
    pub partial_publications: bool,
}

impl ForcedCleanup {
    pub const CLEAN: Self = Self {
        stale_capabilities_fenced: true,
        ownership_released: true,
        live_locks: false,
        partial_publications: false,
    };

    pub const fn safe(self) -> bool {
        self.stale_capabilities_fenced
            && self.ownership_released
            && !self.live_locks
            && !self.partial_publications
    }

    pub const fn merge(self, other: Self) -> Self {
        Self {
            stale_capabilities_fenced: self.stale_capabilities_fenced
                && other.stale_capabilities_fenced,
            ownership_released: self.ownership_released && other.ownership_released,
            live_locks: self.live_locks || other.live_locks,
            partial_publications: self.partial_publications || other.partial_publications,
        }
    }
}

pub trait DrainRuntime {
    type Error;

    fn quiesce(
        &mut self,
        target: DrainTarget,
        resource: DrainResource,
    ) -> Result<(), Self::Error>;

    fn request_handoff(
        &mut self,
        target: DrainTarget,
        resource: DrainResource,
    ) -> Result<(), Self::Error>;

    fn observe(&mut self, target: DrainTarget) -> Result<DrainObservation, Self::Error>;

    fn force_cleanup(
        &mut self,
        target: DrainTarget,
        resource: DrainResource,
    ) -> Result<ForcedCleanup, Self::Error>;

    fn record_audit(&mut self, event: DrainAuditEvent) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrainPhase {
    Started,
    Quiesced,
    HandoffRequested,
    ForceStarted,
    Completed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrainAuditEvent {
    pub target: DrainTarget,
    pub phase: DrainPhase,
    pub observed: DrainObservation,
    pub forced: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrainReceipt {
    pub target: DrainTarget,
    pub started_at_ms: u64,
    pub completed_at_ms: u64,
    pub forced: bool,
    pub resources: usize,
    pub stale_capabilities_fenced: bool,
    pub ownership_released: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrainProgress {
    Waiting {
        observation: DrainObservation,
        remaining_ms: u64,
    },
    Forced {
        observation: DrainObservation,
        remaining_ms: u64,
    },
    Complete(DrainReceipt),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DrainState {
    Idle,
    Waiting,
    Forced,
    Complete,
    Failed,
}

#[derive(Debug)]
pub enum DrainError<E> {
    Busy,
    NotStarted,
    InvalidPlan,
    ClockReversed,
    Runtime(E),
    ResidualResources(DrainObservation),
    ForceCleanupIncomplete(ForcedCleanup),
}

pub struct DrainCoordinator {
    state: DrainState,
    target: Option<DrainTarget>,
    started_at_ms: u64,
    last_now_ms: u64,
    drain_deadline_ms: u64,
    force_deadline_ms: u64,
    forced_cleanup: ForcedCleanup,
    forced: bool,
}

impl DrainCoordinator {
    pub const fn new() -> Self {
        Self {
            state: DrainState::Idle,
            target: None,
            started_at_ms: 0,
            last_now_ms: 0,
            drain_deadline_ms: 0,
            force_deadline_ms: 0,
            forced_cleanup: ForcedCleanup::CLEAN,
            forced: false,
        }
    }

    pub const fn state(&self) -> DrainState {
        self.state
    }

    pub const fn forced_cleanup(&self) -> ForcedCleanup {
        self.forced_cleanup
    }

    pub fn begin<R: DrainRuntime>(
        &mut self,
        runtime: &mut R,
        target: DrainTarget,
        plan: DrainPlan,
        now_ms: u64,
    ) -> Result<(), DrainError<R::Error>> {
        if self.state != DrainState::Idle {
            return Err(DrainError::Busy)
        }
        if plan.drain_timeout_ms == 0 || plan.force_timeout_ms == 0 {
            return Err(DrainError::InvalidPlan)
        }

        self.started_at_ms = now_ms;
        self.last_now_ms = now_ms;
        self.drain_deadline_ms = now_ms.saturating_add(plan.drain_timeout_ms);
        self.force_deadline_ms = self
            .drain_deadline_ms
            .saturating_add(plan.force_timeout_ms);
        self.target = Some(target);
        self.forced_cleanup = ForcedCleanup::CLEAN;
        self.forced = false;

        self.audit(runtime, DrainPhase::Started, DrainObservation::default(), false)?;
        for resource in DrainResource::ALL {
            runtime
                .quiesce(target, resource)
                .map_err(DrainError::Runtime)?;
        }
        self.audit(runtime, DrainPhase::Quiesced, DrainObservation::default(), false)?;
        for resource in DrainResource::ALL {
            runtime
                .request_handoff(target, resource)
                .map_err(DrainError::Runtime)?;
        }
        self.audit(
            runtime,
            DrainPhase::HandoffRequested,
            DrainObservation::default(),
            false,
        )?;
        self.state = DrainState::Waiting;
        Ok(())
    }

    pub fn step<R: DrainRuntime>(
        &mut self,
        runtime: &mut R,
        now_ms: u64,
    ) -> Result<DrainProgress, DrainError<R::Error>> {
        if self.state == DrainState::Idle {
            return Err(DrainError::NotStarted)
        }
        if self.state == DrainState::Complete {
            return Err(DrainError::Busy)
        }
        if now_ms < self.last_now_ms {
            self.state = DrainState::Failed;
            return Err(DrainError::ClockReversed)
        }
        self.last_now_ms = now_ms;
        let target = self.target.ok_or(DrainError::NotStarted)?;
        let observation = runtime.observe(target).map_err(DrainError::Runtime)?;

        if observation.clean() {
            return self.complete(runtime, now_ms, observation)
        }

        if self.state == DrainState::Waiting && now_ms < self.drain_deadline_ms {
            return Ok(DrainProgress::Waiting {
                observation,
                remaining_ms: self.drain_deadline_ms - now_ms,
            })
        }

        if self.state == DrainState::Waiting {
            self.state = DrainState::Forced;
            self.forced = true;
            self.audit(runtime, DrainPhase::ForceStarted, observation, true)?;
            self.forced_cleanup = ForcedCleanup::CLEAN;
            for resource in DrainResource::ALL {
                let cleanup = runtime
                    .force_cleanup(target, resource)
                    .map_err(DrainError::Runtime)?;
                self.forced_cleanup = self.forced_cleanup.merge(cleanup);
            }
            if !self.forced_cleanup.safe() {
                self.state = DrainState::Failed;
                return Err(DrainError::ForceCleanupIncomplete(self.forced_cleanup))
            }
        }

        if observation.clean() {
            return self.complete(runtime, now_ms, observation)
        }
        if now_ms >= self.force_deadline_ms {
            self.state = DrainState::Failed;
            return Err(DrainError::ResidualResources(observation))
        }
        Ok(DrainProgress::Forced {
            observation,
            remaining_ms: self.force_deadline_ms - now_ms,
        })
    }

    fn complete<R: DrainRuntime>(
        &mut self,
        runtime: &mut R,
        completed_at_ms: u64,
        observation: DrainObservation,
    ) -> Result<DrainProgress, DrainError<R::Error>> {
        let target = self.target.ok_or(DrainError::NotStarted)?;
        let cleanup = self.forced_cleanup;
        if self.forced && !cleanup.safe() {
            self.state = DrainState::Failed;
            return Err(DrainError::ForceCleanupIncomplete(cleanup))
        }
        let receipt = DrainReceipt {
            target,
            started_at_ms: self.started_at_ms,
            completed_at_ms,
            forced: self.forced,
            resources: DRAIN_RESOURCE_COUNT,
            stale_capabilities_fenced: !self.forced || cleanup.stale_capabilities_fenced,
            ownership_released: !self.forced || cleanup.ownership_released,
        };
        self.audit(runtime, DrainPhase::Completed, observation, self.forced)?;
        self.state = DrainState::Complete;
        Ok(DrainProgress::Complete(receipt))
    }

    fn audit<R: DrainRuntime>(
        &self,
        runtime: &mut R,
        phase: DrainPhase,
        observed: DrainObservation,
        forced: bool,
    ) -> Result<(), DrainError<R::Error>> {
        let target = self.target.ok_or(DrainError::NotStarted)?;
        runtime
            .record_audit(DrainAuditEvent {
                target,
                phase,
                observed,
                forced,
            })
            .map_err(DrainError::Runtime)
    }
}

impl Default for DrainCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

