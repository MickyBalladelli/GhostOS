use ghostos_fabric::NodeId;
use ghostos_status::{IntoStatus, Status};

pub const MAX_SHARD_REPLICAS: usize = 3;
pub const MAX_SHARD_EVENTS: usize = 128;

/// The consistency promise carried by an existing service contract. Sharding
/// changes ownership, never this value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ConsistencyContract {
    Linearizable = 1,
    Snapshot = 2,
    AppendOnly = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ShardNamespace {
    ClusterMetadata = 1,
    CapabilityIndex = 2,
    PackageCatalog = 3,
    AuditStream = 4,
    PlacementDecision = 5,
}

impl ShardNamespace {
    pub const fn consistency(self) -> ConsistencyContract {
        match self {
            Self::ClusterMetadata | Self::CapabilityIndex | Self::PlacementDecision => {
                ConsistencyContract::Linearizable
            }
            Self::PackageCatalog => ConsistencyContract::Snapshot,
            Self::AuditStream => ConsistencyContract::AppendOnly,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ShardId(u16);

impl ShardId {
    pub const fn new(raw: u16) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ShardState {
    Stable = 1,
    Moving = 2,
    Degraded = 3,
    Recovering = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShardRoute {
    pub shard: ShardId,
    pub namespace: ShardNamespace,
    pub primary: NodeId,
    pub generation: u64,
    pub contract: ConsistencyContract,
    pub state: ShardState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShardRecord {
    pub id: ShardId,
    pub namespace: ShardNamespace,
    pub generation: u64,
    pub primary: NodeId,
    pub replicas: [Option<NodeId>; MAX_SHARD_REPLICAS],
    pub committed_index: u64,
    pub state: ShardState,
    pub moving_to: Option<NodeId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShardWriteReceipt {
    pub shard: ShardId,
    pub index: u64,
    pub term: u64,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryEvidence {
    pub term: u64,
    pub epoch: u64,
    pub event_count: u16,
    pub shard_moves: u16,
    pub rebalances: u16,
    pub split_brain_rejections: u16,
    pub node_losses: u16,
    pub recovered_nodes: u16,
    pub stable_shards: u16,
    pub degraded_shards: u16,
    pub fingerprint: u64,
}

impl RecoveryEvidence {
    pub const fn is_stable(self) -> bool {
        self.degraded_shards == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShardError {
    Capacity,
    Duplicate,
    InvalidConfiguration,
    InvalidNode,
    InsufficientReplicas,
    NotFound,
    NotLeader,
    NotMoving,
    QuorumUnavailable,
    SplitBrain,
    StaleGeneration,
    StaleTerm,
    StaleNodeEpoch,
    TargetAlreadyReplica,
    TargetUnavailable,
    WriteConflict,
}

impl IntoStatus for ShardError {
    fn status(self) -> Status {
        match self {
            Self::Capacity | Self::InsufficientReplicas => Status::NO_SPACE,
            Self::Duplicate | Self::SplitBrain | Self::StaleGeneration | Self::WriteConflict => {
                Status::CONFLICT
            }
            Self::NotFound => Status::NOT_FOUND,
            Self::QuorumUnavailable | Self::NotLeader | Self::TargetUnavailable => Status::BUSY,
            Self::InvalidConfiguration
            | Self::InvalidNode
            | Self::NotMoving
            | Self::StaleTerm
            | Self::StaleNodeEpoch
            | Self::TargetAlreadyReplica => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct NodeState {
    node: NodeId,
    epoch: u64,
    capacity: u64,
    load: u16,
    alive: bool,
    fenced: bool,
}

impl NodeState {
    const EMPTY: Self = Self {
        node: NodeId::from_valid_raw(0),
        epoch: 0,
        capacity: 0,
        load: 0,
        alive: false,
        fenced: true,
    };

    const fn available(self) -> bool {
        self.alive && !self.fenced
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShardSlot {
    record: ShardRecord,
    move_acknowledged: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EventKind {
    NodeLoss = 1,
    NodeRecovery = 2,
    MoveCommitted = 3,
    RebalanceCommitted = 4,
    SplitBrainRejected = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShardEvent {
    sequence: u64,
    kind: EventKind,
    shard: Option<ShardId>,
    node: NodeId,
    term: u64,
    generation: u64,
}

/// Bounded shard ownership and recovery state for the cluster control plane.
///
/// Existing writers still supply their service-level generation or append
/// index. This coordinator adds ownership fencing and quorum checks around
/// those operations; it does not turn snapshot or append-only consumers into
/// linearizable consumers.
pub struct ClusterShardCoordinator<
    const SHARDS: usize = 32,
    const NODES: usize = 16,
    const EVENTS: usize = MAX_SHARD_EVENTS,
> {
    replication_factor: usize,
    epoch: u64,
    term: u64,
    leader: Option<NodeId>,
    next_shard: u16,
    nodes: [NodeState; NODES],
    shards: [Option<ShardSlot>; SHARDS],
    events: [Option<ShardEvent>; EVENTS],
    event_count: usize,
    moves: u16,
    rebalances: u16,
    split_brain_rejections: u16,
    node_losses: u16,
    recovered_nodes: u16,
}

impl<const SHARDS: usize, const NODES: usize, const EVENTS: usize>
    ClusterShardCoordinator<SHARDS, NODES, EVENTS>
{
    pub fn new(replication_factor: usize) -> Result<Self, ShardError> {
        if !(1..=MAX_SHARD_REPLICAS).contains(&replication_factor)
            || SHARDS == 0
            || NODES == 0
            || EVENTS == 0
        {
            return Err(ShardError::InvalidConfiguration)
        }
        Ok(Self {
            replication_factor,
            epoch: 1,
            term: 0,
            leader: None,
            next_shard: 1,
            nodes: [NodeState::EMPTY; NODES],
            shards: [None; SHARDS],
            events: [None; EVENTS],
            event_count: 0,
            moves: 0,
            rebalances: 0,
            split_brain_rejections: 0,
            node_losses: 0,
            recovered_nodes: 0,
        })
    }

    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    pub const fn term(&self) -> u64 {
        self.term
    }

    pub const fn leader(&self) -> Option<NodeId> {
        self.leader
    }

    pub fn node_epoch(&self, node: NodeId) -> Option<u64> {
        self.node(node).map(|state| state.epoch)
    }

    pub fn add_node(&mut self, node: NodeId, capacity: u64) -> Result<u64, ShardError> {
        if node.raw() == 0 || capacity == 0 {
            return Err(ShardError::InvalidNode)
        }
        if self.node(node).is_some() {
            return Err(ShardError::Duplicate)
        }
        let slot = self
            .nodes
            .iter_mut()
            .find(|entry| !entry.alive && entry.epoch == 0)
            .ok_or(ShardError::Capacity)?;
        *slot = NodeState {
            node,
            epoch: 1,
            capacity,
            load: 0,
            alive: true,
            fenced: false,
        };
        Ok(1)
    }

    pub fn elect_leader(&mut self, candidate: NodeId, term: u64) -> Result<(), ShardError> {
        let node = self.node(candidate).ok_or(ShardError::NotFound)?;
        if !node.available() {
            return Err(ShardError::TargetUnavailable)
        }
        if term < self.term {
            return Err(ShardError::StaleTerm)
        }
        if term == self.term {
            if self.leader == Some(candidate) {
                return Ok(())
            }
            if self.leader.is_some() {
                self.split_brain_rejections = self.split_brain_rejections.saturating_add(1);
                self.record_event(EventKind::SplitBrainRejected, None, candidate, term, 0);
                return Err(ShardError::SplitBrain)
            }
        }
        self.term = term;
        self.leader = Some(candidate);
        self.epoch = self.epoch.saturating_add(1);
        Ok(())
    }

    pub fn create_shard(
        &mut self,
        namespace: ShardNamespace,
    ) -> Result<ShardId, ShardError> {
        let id = ShardId::new(self.next_shard).ok_or(ShardError::Capacity)?;
        self.next_shard = self.next_shard.checked_add(1).ok_or(ShardError::Capacity)?;
        self.create_shard_with_id(id, namespace)
    }

    pub fn create_shard_with_id(
        &mut self,
        id: ShardId,
        namespace: ShardNamespace,
    ) -> Result<ShardId, ShardError> {
        if self.shard(id).is_some() {
            return Err(ShardError::Duplicate)
        }
        let replicas = self.choose_replicas(None)?;
        let primary = replicas[0].ok_or(ShardError::InsufficientReplicas)?;
        let slot = self
            .shards
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(ShardError::Capacity)?;
        *slot = Some(ShardSlot {
            record: ShardRecord {
                id,
                namespace,
                generation: 1,
                primary,
                replicas,
                committed_index: 0,
                state: ShardState::Stable,
                moving_to: None,
            },
            move_acknowledged: false,
        });
        self.recalculate_loads();
        Ok(id)
    }

    pub fn shard(&self, id: ShardId) -> Option<ShardRecord> {
        self.shards
            .iter()
            .flatten()
            .find(|slot| slot.record.id == id)
            .map(|slot| slot.record)
    }

    pub fn route(&self, namespace: ShardNamespace, key: u64) -> Result<ShardRoute, ShardError> {
        let mut matching = [None; SHARDS];
        let mut count = 0usize;
        for slot in self.shards.iter().flatten() {
            if slot.record.namespace == namespace {
                if count < SHARDS {
                    matching[count] = Some(slot.record.id);
                    count += 1;
                }
            }
        }
        if count == 0 {
            return Err(ShardError::NotFound)
        }
        let id = matching[(key as usize) % count].ok_or(ShardError::NotFound)?;
        let record = self.shard(id).ok_or(ShardError::NotFound)?;
        Ok(ShardRoute {
            shard: id,
            namespace,
            primary: record.primary,
            generation: record.generation,
            contract: namespace.consistency(),
            state: record.state,
        })
    }

    pub fn commit_write(
        &mut self,
        id: ShardId,
        writer: NodeId,
        node_epoch: u64,
        term: u64,
        expected_index: u64,
    ) -> Result<ShardWriteReceipt, ShardError> {
        self.validate_term(term)?;
        let record = self.shard(id).ok_or(ShardError::NotFound)?;
        if record.primary != writer {
            return Err(ShardError::NotLeader)
        }
        let current_epoch = self
            .node(writer)
            .ok_or(ShardError::NotFound)?
            .epoch;
        if current_epoch != node_epoch {
            return Err(ShardError::StaleNodeEpoch)
        }
        if !self.has_quorum(record) {
            return Err(ShardError::QuorumUnavailable)
        }
        if record.committed_index != expected_index {
            return Err(ShardError::WriteConflict)
        }
        let slot = self.shard_slot_mut(id)?;
        slot.record.committed_index = slot.record.committed_index.saturating_add(1);
        Ok(ShardWriteReceipt {
            shard: id,
            index: slot.record.committed_index,
            term,
            generation: slot.record.generation,
        })
    }

    pub fn begin_move(
        &mut self,
        id: ShardId,
        target: NodeId,
        expected_generation: u64,
        leader: NodeId,
        term: u64,
    ) -> Result<(), ShardError> {
        self.validate_leader(leader, term)?;
        let target_state = self.node(target).ok_or(ShardError::NotFound)?;
        if !target_state.available() {
            return Err(ShardError::TargetUnavailable)
        }
        let record = self.shard(id).ok_or(ShardError::NotFound)?;
        if record.generation != expected_generation {
            return Err(ShardError::StaleGeneration)
        }
        if record.moving_to.is_some() {
            return Err(ShardError::NotMoving)
        }
        if record.replicas.iter().flatten().any(|node| *node == target) {
            return Err(ShardError::TargetAlreadyReplica)
        }
        if !self.has_quorum(record) {
            return Err(ShardError::QuorumUnavailable)
        }
        let slot = self.shard_slot_mut(id)?;
        slot.record.state = ShardState::Moving;
        slot.record.moving_to = Some(target);
        slot.move_acknowledged = false;
        Ok(())
    }

    pub fn acknowledge_move(
        &mut self,
        id: ShardId,
        target: NodeId,
        term: u64,
    ) -> Result<(), ShardError> {
        if term != self.term {
            return Err(ShardError::StaleTerm)
        }
        let target_state = self.node(target).ok_or(ShardError::NotFound)?;
        if !target_state.available() {
            return Err(ShardError::TargetUnavailable)
        }
        let slot = self.shard_slot_mut(id)?;
        if slot.record.moving_to != Some(target) {
            return Err(ShardError::NotMoving)
        }
        slot.move_acknowledged = true;
        Ok(())
    }

    pub fn commit_move(
        &mut self,
        id: ShardId,
        leader: NodeId,
        term: u64,
    ) -> Result<(), ShardError> {
        self.validate_leader(leader, term)?;
        let record = self.shard(id).ok_or(ShardError::NotFound)?;
        if !self.has_quorum(record) {
            return Err(ShardError::QuorumUnavailable)
        }
        let target = record.moving_to.ok_or(ShardError::NotMoving)?;
        let mut replicas = [None; MAX_SHARD_REPLICAS];
        replicas[0] = Some(target);
        let mut cursor = 1usize;
        for replica in record.replicas.iter().flatten() {
            if *replica != target && cursor < record.replicas.iter().flatten().count().min(MAX_SHARD_REPLICAS) {
                replicas[cursor] = Some(*replica);
                cursor += 1;
            }
        }
        if cursor < record.replicas.iter().flatten().count().min(MAX_SHARD_REPLICAS) {
            replicas[cursor] = Some(record.primary);
        }
        let next_generation = {
            let slot = self.shard_slot_mut(id)?;
            if !slot.move_acknowledged {
                return Err(ShardError::QuorumUnavailable)
            }
            slot.record.primary = target;
            slot.record.replicas = replicas;
            slot.record.generation = slot.record.generation.saturating_add(1);
            slot.record.state = ShardState::Stable;
            slot.record.moving_to = None;
            slot.move_acknowledged = false;
            slot.record.generation
        };
        self.moves = self.moves.saturating_add(1);
        self.record_event(
            EventKind::MoveCommitted,
            Some(id),
            target,
            term,
            next_generation,
        );
        self.recalculate_loads();
        Ok(())
    }

    pub fn rebalance(&mut self, leader: NodeId, term: u64) -> Result<Option<ShardId>, ShardError> {
        self.validate_leader(leader, term)?;
        self.recalculate_loads();
        let mut source = None;
        let mut source_load = 0u16;
        for slot in self.shards.iter().flatten() {
            let load = self.node(slot.record.primary).map_or(0, |node| node.load);
            if slot.record.state == ShardState::Stable && load > source_load {
                source = Some(slot.record);
                source_load = load;
            }
        }
        let source = source.ok_or(ShardError::NotFound)?;
        let mut target = None;
        for node in self.nodes.iter().copied().filter(|node| node.available()) {
            if source.replicas.iter().flatten().any(|replica| *replica == node.node) {
                continue
            }
            if node.load < source_load && target.is_none_or(|current: NodeState| {
                (node.load, node.node.raw()) < (current.load, current.node.raw())
            }) {
                target = Some(node);
            }
        }
        let target = target.ok_or(ShardError::NotFound)?.node;
        self.begin_move(source.id, target, source.generation, leader, term)?;
        self.acknowledge_move(source.id, target, term)?;
        self.commit_move(source.id, leader, term)?;
        self.rebalances = self.rebalances.saturating_add(1);
        if let Some(event) = self.events.iter_mut().rev().flatten().next() {
            event.kind = EventKind::RebalanceCommitted;
        }
        Ok(Some(source.id))
    }

    pub fn fail_node(&mut self, node: NodeId) -> Result<(), ShardError> {
        let state = self.node_mut(node)?;
        if !state.alive {
            return Ok(())
        }
        state.alive = false;
        state.fenced = true;
        self.epoch = self.epoch.saturating_add(1);
        self.node_losses = self.node_losses.saturating_add(1);
        if self.leader == Some(node) {
            self.leader = None;
            self.term = self.term.saturating_add(1);
        }
        for index in 0..SHARDS {
            let Some(record) = self.shards[index].map(|slot| slot.record) else {
                continue
            };
            if !record.replicas.iter().flatten().any(|replica| *replica == node) {
                continue
            }
            let replacement = record
                .replicas
                .iter()
                .flatten()
                .copied()
                .find(|replica| *replica != node && self.node(*replica).is_some_and(NodeState::available));
            let slot = self.shards[index].as_mut().expect("shard slot exists");
            if slot.record.primary == node {
                if let Some(replacement) = replacement {
                    slot.record.primary = replacement;
                    slot.record.generation = slot.record.generation.saturating_add(1);
                }
            }
            slot.record.state = ShardState::Degraded;
            slot.record.moving_to = None;
        }
        self.record_event(EventKind::NodeLoss, None, node, self.term, 0);
        Ok(())
    }

    pub fn recover_node(&mut self, node: NodeId) -> Result<u64, ShardError> {
        let next_epoch = {
            let state = self.node_mut(node)?;
            if state.alive {
                return Err(ShardError::Duplicate)
            }
            state.alive = true;
            state.fenced = true;
            state.epoch = state.epoch.saturating_add(1);
            state.epoch
        };
        self.epoch = self.epoch.saturating_add(1);
        self.recovered_nodes = self.recovered_nodes.saturating_add(1);
        self.record_event(EventKind::NodeRecovery, None, node, self.term, 0);
        Ok(next_epoch)
    }

    pub fn reconcile_node(
        &mut self,
        node: NodeId,
        leader: NodeId,
        term: u64,
    ) -> Result<(), ShardError> {
        self.validate_leader(leader, term)?;
        let state = self.node(node).ok_or(ShardError::NotFound)?;
        if !state.alive {
            return Err(ShardError::TargetUnavailable)
        }
        let state = self.node_mut(node)?;
        state.fenced = false;
        for index in 0..SHARDS {
            let Some(record) = self.shards[index].map(|slot| slot.record) else {
                continue
            };
            if !record.replicas.iter().flatten().any(|replica| *replica == node) {
                continue
            }
            let has_quorum = self.has_quorum(record);
            let fully_available = record
                .replicas
                .iter()
                .flatten()
                .all(|replica| self.node(*replica).is_some_and(NodeState::available));
            let slot = self.shards[index].as_mut().expect("shard slot exists");
            if has_quorum && fully_available {
                slot.record.state = ShardState::Stable;
                slot.record.generation = slot.record.generation.saturating_add(1);
            } else {
                slot.record.state = ShardState::Recovering;
            }
        }
        self.recalculate_loads();
        Ok(())
    }

    pub fn recovery_evidence(&self) -> RecoveryEvidence {
        let mut stable_shards = 0u16;
        let mut degraded_shards = 0u16;
        let mut fingerprint = 0xcbf29ce484222325;
        for slot in self.shards.iter().flatten() {
            if slot.record.state == ShardState::Stable {
                stable_shards = stable_shards.saturating_add(1);
            } else {
                degraded_shards = degraded_shards.saturating_add(1);
            }
            fingerprint = mix(fingerprint, slot.record.id.raw() as u64);
            fingerprint = mix(fingerprint, slot.record.primary.raw() as u64);
            fingerprint = mix(fingerprint, slot.record.generation);
            fingerprint = mix(fingerprint, slot.record.committed_index);
        }
        for event in self.events.iter().flatten() {
            fingerprint = mix(fingerprint, event.sequence);
            fingerprint = mix(fingerprint, event.kind as u64);
            fingerprint = mix(fingerprint, event.shard.map_or(0, |shard| shard.raw() as u64));
            fingerprint = mix(fingerprint, event.node.raw() as u64);
            fingerprint = mix(fingerprint, event.term);
            fingerprint = mix(fingerprint, event.generation);
        }
        RecoveryEvidence {
            term: self.term,
            epoch: self.epoch,
            event_count: self.event_count.min(u16::MAX as usize) as u16,
            shard_moves: self.moves,
            rebalances: self.rebalances,
            split_brain_rejections: self.split_brain_rejections,
            node_losses: self.node_losses,
            recovered_nodes: self.recovered_nodes,
            stable_shards,
            degraded_shards,
            fingerprint,
        }
    }

    fn validate_leader(&mut self, leader: NodeId, term: u64) -> Result<(), ShardError> {
        if self.leader != Some(leader) {
            return Err(ShardError::NotLeader)
        }
        self.validate_term(term)
    }

    fn validate_term(&self, term: u64) -> Result<(), ShardError> {
        if self.leader.is_none() {
            return Err(ShardError::NotLeader)
        }
        if term != self.term {
            return Err(ShardError::StaleTerm)
        }
        if !self
            .leader
            .is_some_and(|leader| self.node(leader).is_some_and(NodeState::available))
        {
            return Err(ShardError::TargetUnavailable)
        }
        Ok(())
    }

    fn node(&self, node: NodeId) -> Option<NodeState> {
        self.nodes.iter().copied().find(|entry| entry.epoch != 0 && entry.node == node)
    }

    fn node_mut(&mut self, node: NodeId) -> Result<&mut NodeState, ShardError> {
        self.nodes
            .iter_mut()
            .find(|entry| entry.epoch != 0 && entry.node == node)
            .ok_or(ShardError::NotFound)
    }

    fn shard_slot_mut(&mut self, id: ShardId) -> Result<&mut ShardSlot, ShardError> {
        self.shards
            .iter_mut()
            .flatten()
            .find(|slot| slot.record.id == id)
            .ok_or(ShardError::NotFound)
    }

    fn choose_replicas(
        &self,
        excluded: Option<NodeId>,
    ) -> Result<[Option<NodeId>; MAX_SHARD_REPLICAS], ShardError> {
        let mut result = [None; MAX_SHARD_REPLICAS];
        let mut count = 0usize;
        while count < self.replication_factor {
            let mut best = None;
            for node in self.nodes.iter().copied().filter(|node| node.available()) {
                if Some(node.node) == excluded || result.iter().flatten().any(|used| *used == node.node) {
                    continue
                }
                if best.is_none_or(|current: NodeState| {
                    (node.load, node.node.raw()) < (current.load, current.node.raw())
                }) {
                    best = Some(node);
                }
            }
            let node = best.ok_or(ShardError::InsufficientReplicas)?;
            result[count] = Some(node.node);
            count += 1;
        }
        Ok(result)
    }

    fn has_quorum(&self, record: ShardRecord) -> bool {
        let available = record
            .replicas
            .iter()
            .flatten()
            .filter(|replica| self.node(**replica).is_some_and(NodeState::available))
            .count();
        available >= self.replication_factor / 2 + 1
    }

    fn recalculate_loads(&mut self) {
        for node in &mut self.nodes {
            if node.epoch != 0 {
                node.load = 0;
            }
        }
        for slot in self.shards.iter().flatten() {
            for replica in slot.record.replicas.iter().flatten() {
                if let Some(node) = self.nodes.iter_mut().find(|entry| entry.node == *replica) {
                    node.load = node.load.saturating_add(1);
                }
            }
        }
    }

    fn record_event(
        &mut self,
        kind: EventKind,
        shard: Option<ShardId>,
        node: NodeId,
        term: u64,
        generation: u64,
    ) {
        let event = ShardEvent {
            sequence: self.event_count as u64 + 1,
            kind,
            shard,
            node,
            term,
            generation,
        };
        let index = self.event_count % EVENTS;
        self.events[index] = Some(event);
        self.event_count = self.event_count.saturating_add(1);
    }
}

impl<const SHARDS: usize, const NODES: usize, const EVENTS: usize> Default
    for ClusterShardCoordinator<SHARDS, NODES, EVENTS>
{
    fn default() -> Self {
        Self::new(3).expect("valid default shard configuration")
    }
}

fn mix(mut hash: u64, value: u64) -> u64 {
    hash ^= value;
    hash = hash.wrapping_mul(0x100000001b3);
    hash.rotate_left(13)
}
