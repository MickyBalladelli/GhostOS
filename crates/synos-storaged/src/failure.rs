use synos_fabric::NodeId;
use synos_mesh::{CowDelta, DeltaApplyReceipt, DeltaMode};
use synos_status::{IntoStatus, Status};
use synos_synfs::SynFs;
use synos_time_sync::MonotonicClock;

pub const MAX_FAILURE_EVENTS: usize = 32;
pub const MAX_RECOVERY_NODES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FailureKind {
    NodeFailure = 1,
    ClusterDegraded = 2,
    Partition = 3,
    QuorumLoss = 4,
    ClockSkew = 5,
    ProtocolMismatch = 6,
    StaleState = 7,
}

impl FailureKind {
    pub const fn status(self) -> Status {
        match self {
            Self::NodeFailure => Status::NODE_UNSAFE,
            Self::ClusterDegraded => Status::CLUSTER_DEGRADED,
            Self::Partition => Status::PARTITIONED,
            Self::QuorumLoss => Status::QUORUM_LOST,
            Self::ClockSkew => Status::CLOCK_SKEW,
            Self::ProtocolMismatch => Status::PROTOCOL_MISMATCH,
            Self::StaleState => Status::STALE_STATE,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ClusterHealth {
    Healthy = 1,
    Degraded = 2,
    Partitioned = 3,
    QuorumLost = 4,
}

impl ClusterHealth {
    pub const fn status(self) -> Status {
        match self {
            Self::Healthy => Status::NORMAL,
            Self::Degraded => Status::CLUSTER_DEGRADED,
            Self::Partitioned => Status::PARTITIONED,
            Self::QuorumLost => Status::QUORUM_LOST,
        }
    }

    pub const fn is_available(self) -> bool {
        matches!(self, Self::Healthy | Self::Degraded)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterSample {
    pub generation: u64,
    pub membership_epoch: u64,
    pub voting_members: u16,
    pub available_votes: u16,
    pub required_votes: u16,
    pub partitioned: bool,
    pub sampled_at_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeHealthSample {
    pub node: NodeId,
    pub reachable: bool,
    pub heartbeat_age_us: u64,
    pub heartbeat_timeout_us: u64,
    pub clock_offset_us: i64,
    pub maximum_clock_skew_us: u64,
    pub protocol_version: u16,
    pub minimum_protocol_version: u16,
    pub maximum_protocol_version: u16,
    pub generation: u64,
    pub membership_epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FailureEvent {
    pub kind: FailureKind,
    pub node: Option<NodeId>,
    pub at_us: u64,
    pub generation: u64,
    pub membership_epoch: u64,
    pub status: Status,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FailureReport<const EVENTS: usize = MAX_FAILURE_EVENTS> {
    pub health: ClusterHealth,
    pub generation: u64,
    pub membership_epoch: u64,
    pub sampled_at_us: u64,
    pub available_votes: u16,
    pub required_votes: u16,
    pub events: [Option<FailureEvent>; EVENTS],
    pub event_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RecoveryState {
    Healthy = 1,
    Suspect = 2,
    Failed = 3,
    Draining = 4,
    Fenced = 5,
    RecoveryRequired = 6,
    Reconciled = 7,
    Abandoned = 8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionRequest {
    pub action: OperatorAction,
    pub node: Option<NodeId>,
    pub confirm: bool,
    pub force: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum OperatorAction {
    Retry = 1,
    Resync = 2,
    Drain = 3,
    Recover = 4,
    Fence = 5,
    Unfence = 6,
    Rollback = 7,
    Abandon = 8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryReceipt {
    pub action: OperatorAction,
    pub node: Option<NodeId>,
    pub state: RecoveryState,
    pub health: ClusterHealth,
    pub generation: u64,
    pub membership_epoch: u64,
    pub fenced_before_release: bool,
    pub resources_released: bool,
    pub reconciled: bool,
    pub availability_preserved: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureError {
    Capacity,
    UnknownNode,
    ConfirmationRequired,
    QuorumUnavailable,
    Partitioned,
    NodeUnsafe,
    InvalidState,
    ReconciliationRequired,
    RollbackUnavailable,
    Hook(Status),
}

impl IntoStatus for FailureError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::UnknownNode => Status::NOT_FOUND,
            Self::ConfirmationRequired => Status::CONFIRMATION_REQUIRED,
            Self::QuorumUnavailable => Status::QUORUM_LOST,
            Self::Partitioned => Status::PARTITIONED,
            Self::NodeUnsafe => Status::NODE_UNSAFE,
            Self::InvalidState => Status::RECOVERY_STATE_INVALID,
            Self::ReconciliationRequired => Status::RECONCILIATION_REQUIRED,
            Self::RollbackUnavailable => Status::ROLLBACK_UNAVAILABLE,
            Self::Hook(status) => status,
        }
    }
}

/// Side effects are injected so fencing remains the first operation before
/// any shared resource is released or reassigned.
pub trait RecoveryHooks {
    fn retry(&mut self, node: NodeId) -> Result<(), Status>;
    fn drain(&mut self, node: NodeId) -> Result<(), Status>;
    fn fence(&mut self, node: NodeId) -> Result<(), Status>;
    fn unfence(&mut self, node: NodeId) -> Result<(), Status>;
    fn release_shared_memory(&mut self, node: NodeId) -> Result<(), Status>;
    fn release_storage(&mut self, node: NodeId) -> Result<(), Status>;
    fn release_jobs(&mut self, node: NodeId) -> Result<(), Status>;
    fn revoke_capabilities(&mut self, node: NodeId) -> Result<(), Status>;
    fn release_dlm_leases(&mut self, node: NodeId) -> Result<(), Status>;
    fn reconcile_membership(&mut self, node: NodeId) -> Result<(), Status>;
    fn reconcile_synfs_cow(&mut self, node: NodeId) -> Result<(), Status>;
    fn reconcile_logs(&mut self, node: NodeId) -> Result<(), Status>;
    fn reconcile_reservations(&mut self, node: NodeId) -> Result<(), Status>;
    fn reconcile_workloads(&mut self, node: NodeId) -> Result<(), Status>;
    fn rollback(&mut self) -> Result<(), Status>;
    fn abandon(&mut self, node: NodeId) -> Result<(), Status>;
}

#[derive(Clone, Copy)]
struct NodeRecord {
    node: Option<NodeId>,
    state: RecoveryState,
}

impl NodeRecord {
    const EMPTY: Self = Self {
        node: None,
        state: RecoveryState::Healthy,
    };
}

pub struct FailureController<
    const NODES: usize = MAX_RECOVERY_NODES,
    const EVENTS: usize = MAX_FAILURE_EVENTS,
> {
    health: ClusterHealth,
    generation: u64,
    membership_epoch: u64,
    available_votes: u16,
    required_votes: u16,
    nodes: [NodeRecord; NODES],
    events: [Option<FailureEvent>; EVENTS],
    event_count: usize,
}

impl<const NODES: usize, const EVENTS: usize> FailureController<NODES, EVENTS> {
    pub const fn new() -> Self {
        Self {
            health: ClusterHealth::Healthy,
            generation: 0,
            membership_epoch: 0,
            available_votes: 0,
            required_votes: 0,
            nodes: [NodeRecord::EMPTY; NODES],
            events: [None; EVENTS],
            event_count: 0,
        }
    }

    pub const fn health(&self) -> ClusterHealth {
        self.health
    }

    pub fn state(&self, node: NodeId) -> Option<RecoveryState> {
        self.nodes
            .iter()
            .find(|record| record.node == Some(node))
            .map(|record| record.state)
    }

    pub fn events(&self) -> impl Iterator<Item = FailureEvent> + '_ {
        self.events[..self.event_count].iter().flatten().copied()
    }

    pub fn observe<const SAMPLES: usize>(
        &mut self,
        cluster: ClusterSample,
        samples: &[NodeHealthSample; SAMPLES],
    ) -> FailureReport<EVENTS> {
        self.generation = cluster.generation;
        self.membership_epoch = cluster.membership_epoch;
        self.available_votes = cluster.available_votes;
        self.required_votes = cluster.required_votes;
        self.event_count = 0;
        self.events.fill(None);

        let mut report = FailureReport {
            health: ClusterHealth::Healthy,
            generation: cluster.generation,
            membership_epoch: cluster.membership_epoch,
            sampled_at_us: cluster.sampled_at_us,
            available_votes: cluster.available_votes,
            required_votes: cluster.required_votes,
            events: [None; EVENTS],
            event_count: 0,
        };

        for sample in samples {
            let mut state = RecoveryState::Healthy;
            let mut failure = None;
            if !sample.reachable || sample.heartbeat_age_us >= sample.heartbeat_timeout_us {
                state = RecoveryState::Failed;
                failure = Some(FailureKind::NodeFailure);
            } else if sample.clock_offset_us.unsigned_abs() > sample.maximum_clock_skew_us {
                state = RecoveryState::Failed;
                failure = Some(FailureKind::ClockSkew);
            } else if sample.protocol_version < sample.minimum_protocol_version
                || sample.protocol_version > sample.maximum_protocol_version
            {
                state = RecoveryState::Failed;
                failure = Some(FailureKind::ProtocolMismatch);
            } else if sample.generation != cluster.generation
                || sample.membership_epoch != cluster.membership_epoch
            {
                state = RecoveryState::Failed;
                failure = Some(FailureKind::StaleState);
            }
            self.upsert_node(sample.node, state);
            if let Some(kind) = failure {
                self.push_event(
                    FailureEvent {
                        kind,
                        node: Some(sample.node),
                        at_us: cluster.sampled_at_us,
                        generation: cluster.generation,
                        membership_epoch: cluster.membership_epoch,
                        status: kind.status(),
                    },
                    &mut report,
                );
            }
        }

        let health = if cluster.partitioned {
            self.push_event(
                FailureEvent {
                    kind: FailureKind::Partition,
                    node: None,
                    at_us: cluster.sampled_at_us,
                    generation: cluster.generation,
                    membership_epoch: cluster.membership_epoch,
                    status: Status::PARTITIONED,
                },
                &mut report,
            );
            ClusterHealth::Partitioned
        } else if cluster.available_votes < cluster.required_votes {
            self.push_event(
                FailureEvent {
                    kind: FailureKind::QuorumLoss,
                    node: None,
                    at_us: cluster.sampled_at_us,
                    generation: cluster.generation,
                    membership_epoch: cluster.membership_epoch,
                    status: Status::QUORUM_LOST,
                },
                &mut report,
            );
            ClusterHealth::QuorumLost
        } else if cluster.available_votes < cluster.voting_members || report.event_count != 0 {
            self.push_event(
                FailureEvent {
                    kind: FailureKind::ClusterDegraded,
                    node: None,
                    at_us: cluster.sampled_at_us,
                    generation: cluster.generation,
                    membership_epoch: cluster.membership_epoch,
                    status: Status::CLUSTER_DEGRADED,
                },
                &mut report,
            );
            ClusterHealth::Degraded
        } else {
            ClusterHealth::Healthy
        };
        self.health = health;
        report.health = health;
        report
    }

    pub fn observe_with_clock<C: MonotonicClock, const SAMPLES: usize>(
        &mut self,
        mut cluster: ClusterSample,
        samples: &[NodeHealthSample; SAMPLES],
        clock: &C,
    ) -> FailureReport<EVENTS> {
        cluster.sampled_at_us = clock.now_us();
        self.observe(cluster, samples)
    }

    pub fn apply<H: RecoveryHooks>(
        &mut self,
        request: ActionRequest,
        hooks: &mut H,
    ) -> Result<RecoveryReceipt, FailureError> {
        if matches!(
            request.action,
            OperatorAction::Fence
                | OperatorAction::Unfence
                | OperatorAction::Rollback
                | OperatorAction::Abandon
        ) && !request.confirm
        {
            return Err(FailureError::ConfirmationRequired);
        }
        if matches!(request.action, OperatorAction::Rollback) {
            hooks.rollback().map_err(FailureError::Hook)?;
            return Ok(self.receipt(
                request.action,
                None,
                RecoveryState::Reconciled,
                false,
                false,
                true,
            ));
        }
        let node = request.node.ok_or(FailureError::UnknownNode)?;
        let state = self.state(node).ok_or(FailureError::UnknownNode)?;
        match request.action {
            OperatorAction::Retry => {
                hooks.retry(node).map_err(FailureError::Hook)?;
                self.set_state(node, RecoveryState::Suspect)?;
                Ok(self.receipt(
                    request.action,
                    Some(node),
                    RecoveryState::Suspect,
                    false,
                    false,
                    false,
                ))
            }
            OperatorAction::Resync => {
                if !matches!(
                    state,
                    RecoveryState::Fenced | RecoveryState::RecoveryRequired
                ) {
                    return Err(FailureError::ReconciliationRequired);
                }
                self.reconcile(node, hooks)?;
                Ok(self.receipt(
                    request.action,
                    Some(node),
                    RecoveryState::Reconciled,
                    true,
                    true,
                    true,
                ))
            }
            OperatorAction::Drain => {
                hooks.drain(node).map_err(FailureError::Hook)?;
                self.set_state(node, RecoveryState::Draining)?;
                Ok(self.receipt(
                    request.action,
                    Some(node),
                    RecoveryState::Draining,
                    false,
                    false,
                    true,
                ))
            }
            OperatorAction::Recover => {
                if !matches!(
                    state,
                    RecoveryState::Fenced | RecoveryState::RecoveryRequired
                ) {
                    return Err(FailureError::ReconciliationRequired);
                }
                self.reconcile(node, hooks)?;
                Ok(self.receipt(
                    request.action,
                    Some(node),
                    RecoveryState::Reconciled,
                    true,
                    true,
                    true,
                ))
            }
            OperatorAction::Fence => {
                if matches!(state, RecoveryState::Healthy | RecoveryState::Reconciled)
                    && !request.force
                {
                    return Err(FailureError::NodeUnsafe);
                }
                hooks.fence(node).map_err(FailureError::Hook)?;
                self.set_state(node, RecoveryState::Fenced)?;
                Ok(self.receipt(
                    request.action,
                    Some(node),
                    RecoveryState::Fenced,
                    true,
                    false,
                    self.health.is_available(),
                ))
            }
            OperatorAction::Unfence => {
                if state != RecoveryState::Reconciled {
                    return Err(FailureError::ReconciliationRequired);
                }
                if !self.health.is_available() {
                    return Err(FailureError::QuorumUnavailable);
                }
                hooks.unfence(node).map_err(FailureError::Hook)?;
                self.set_state(node, RecoveryState::Healthy)?;
                Ok(self.receipt(
                    request.action,
                    Some(node),
                    RecoveryState::Healthy,
                    true,
                    true,
                    true,
                ))
            }
            OperatorAction::Abandon => {
                if !matches!(
                    state,
                    RecoveryState::Fenced | RecoveryState::RecoveryRequired
                ) || !request.force
                {
                    return Err(FailureError::InvalidState);
                }
                hooks.abandon(node).map_err(FailureError::Hook)?;
                self.set_state(node, RecoveryState::Abandoned)?;
                Ok(self.receipt(
                    request.action,
                    Some(node),
                    RecoveryState::Abandoned,
                    true,
                    false,
                    self.health.is_available(),
                ))
            }
            OperatorAction::Rollback => unreachable!(),
        }
    }

    /// Apply a bounded SynFS CoW delta only after the source node is fenced.
    /// The caller can then run the remaining log, reservation, and workload
    /// hooks through `RESYNC NODE` or `RECOVER NODE`.
    pub fn apply_cow_delta<const BLOCKS: usize, const OPS: usize, const MAX_BYTES: usize>(
        &self,
        node: NodeId,
        filesystem: &mut SynFs<BLOCKS>,
        delta: &CowDelta<OPS, MAX_BYTES>,
        mode: DeltaMode,
    ) -> Result<DeltaApplyReceipt, FailureError> {
        if !matches!(
            self.state(node),
            Some(
                RecoveryState::Fenced | RecoveryState::RecoveryRequired | RecoveryState::Reconciled
            )
        ) {
            return Err(FailureError::ReconciliationRequired);
        }
        delta
            .apply(filesystem, mode)
            .map_err(|error| FailureError::Hook(error.status()))
    }

    fn reconcile<H: RecoveryHooks>(
        &mut self,
        node: NodeId,
        hooks: &mut H,
    ) -> Result<(), FailureError> {
        self.set_state(node, RecoveryState::RecoveryRequired)?;
        let result = (|| {
            hooks
                .release_shared_memory(node)
                .map_err(FailureError::Hook)?;
            hooks.release_storage(node).map_err(FailureError::Hook)?;
            hooks.release_jobs(node).map_err(FailureError::Hook)?;
            hooks
                .revoke_capabilities(node)
                .map_err(FailureError::Hook)?;
            hooks.release_dlm_leases(node).map_err(FailureError::Hook)?;
            hooks
                .reconcile_membership(node)
                .map_err(FailureError::Hook)?;
            hooks
                .reconcile_synfs_cow(node)
                .map_err(FailureError::Hook)?;
            hooks.reconcile_logs(node).map_err(FailureError::Hook)?;
            hooks
                .reconcile_reservations(node)
                .map_err(FailureError::Hook)?;
            hooks.reconcile_workloads(node).map_err(FailureError::Hook)
        })();
        if result.is_err() {
            self.set_state(node, RecoveryState::RecoveryRequired)?;
        }
        result
    }

    fn upsert_node(&mut self, node: NodeId, state: RecoveryState) {
        if let Some(record) = self
            .nodes
            .iter_mut()
            .find(|record| record.node == Some(node))
        {
            if matches!(
                record.state,
                RecoveryState::Fenced | RecoveryState::RecoveryRequired | RecoveryState::Reconciled
            ) && state == RecoveryState::Healthy
            {
                return;
            }
            record.state = state;
            return;
        }
        if let Some(record) = self.nodes.iter_mut().find(|record| record.node.is_none()) {
            record.node = Some(node);
            record.state = state;
        }
    }

    fn set_state(&mut self, node: NodeId, state: RecoveryState) -> Result<(), FailureError> {
        let record = self
            .nodes
            .iter_mut()
            .find(|record| record.node == Some(node))
            .ok_or(FailureError::UnknownNode)?;
        record.state = state;
        Ok(())
    }

    fn push_event<const REPORT_EVENTS: usize>(
        &mut self,
        event: FailureEvent,
        report: &mut FailureReport<REPORT_EVENTS>,
    ) {
        if self.event_count < EVENTS {
            self.events[self.event_count] = Some(event);
            self.event_count += 1;
        } else if EVENTS != 0 {
            self.events[0] = Some(event);
        }
        if report.event_count < REPORT_EVENTS {
            report.events[report.event_count] = Some(event);
            report.event_count += 1;
        }
    }

    fn receipt(
        &self,
        action: OperatorAction,
        node: Option<NodeId>,
        state: RecoveryState,
        fenced_before_release: bool,
        resources_released: bool,
        reconciled: bool,
    ) -> RecoveryReceipt {
        RecoveryReceipt {
            action,
            node,
            state,
            health: self.health,
            generation: self.generation,
            membership_epoch: self.membership_epoch,
            fenced_before_release,
            resources_released,
            reconciled,
            availability_preserved: self.health.is_available(),
        }
    }
}

impl<const NODES: usize, const EVENTS: usize> Default for FailureController<NODES, EVENTS> {
    fn default() -> Self {
        Self::new()
    }
}
