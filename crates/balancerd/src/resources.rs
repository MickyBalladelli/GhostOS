use synos_fabric::NodeId;
use synos_status::{IntoStatus, Status};

pub const MAX_RESOURCE_LABELS: usize = 8;
pub const MAX_RESOURCE_TAINTS: usize = 4;
pub const MAX_WORKLOAD_AFFINITY: usize = 4;
pub const MAX_PREEMPTED_WORKLOADS: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub struct ResourceVector {
    pub cpu_millis: u64,
    pub ram_bytes: u64,
    pub vram_bytes: u64,
    pub cxl_bytes: u64,
    pub storage_bytes: u64,
    pub network_mbps: u64,
    pub accelerator_units: u64,
    pub lease_slots: u64,
}

impl ResourceVector {
    pub const fn new(
        cpu_millis: u64,
        ram_bytes: u64,
        vram_bytes: u64,
        cxl_bytes: u64,
        storage_bytes: u64,
        network_mbps: u64,
        accelerator_units: u64,
        lease_slots: u64,
    ) -> Self {
        Self {
            cpu_millis,
            ram_bytes,
            vram_bytes,
            cxl_bytes,
            storage_bytes,
            network_mbps,
            accelerator_units,
            lease_slots,
        }
    }

    pub const fn is_zero(self) -> bool {
        self.cpu_millis == 0
            && self.ram_bytes == 0
            && self.vram_bytes == 0
            && self.cxl_bytes == 0
            && self.storage_bytes == 0
            && self.network_mbps == 0
            && self.accelerator_units == 0
            && self.lease_slots == 0
    }

    pub const fn fits(self, request: Self) -> bool {
        self.cpu_millis >= request.cpu_millis
            && self.ram_bytes >= request.ram_bytes
            && self.vram_bytes >= request.vram_bytes
            && self.cxl_bytes >= request.cxl_bytes
            && self.storage_bytes >= request.storage_bytes
            && self.network_mbps >= request.network_mbps
            && self.accelerator_units >= request.accelerator_units
            && self.lease_slots >= request.lease_slots
    }

    pub fn checked_add(self, other: Self) -> Option<Self> {
        Some(Self {
            cpu_millis: self.cpu_millis.checked_add(other.cpu_millis)?,
            ram_bytes: self.ram_bytes.checked_add(other.ram_bytes)?,
            vram_bytes: self.vram_bytes.checked_add(other.vram_bytes)?,
            cxl_bytes: self.cxl_bytes.checked_add(other.cxl_bytes)?,
            storage_bytes: self.storage_bytes.checked_add(other.storage_bytes)?,
            network_mbps: self.network_mbps.checked_add(other.network_mbps)?,
            accelerator_units: self.accelerator_units.checked_add(other.accelerator_units)?,
            lease_slots: self.lease_slots.checked_add(other.lease_slots)?,
        })
    }

    pub fn checked_sub(self, other: Self) -> Option<Self> {
        Some(Self {
            cpu_millis: self.cpu_millis.checked_sub(other.cpu_millis)?,
            ram_bytes: self.ram_bytes.checked_sub(other.ram_bytes)?,
            vram_bytes: self.vram_bytes.checked_sub(other.vram_bytes)?,
            cxl_bytes: self.cxl_bytes.checked_sub(other.cxl_bytes)?,
            storage_bytes: self.storage_bytes.checked_sub(other.storage_bytes)?,
            network_mbps: self.network_mbps.checked_sub(other.network_mbps)?,
            accelerator_units: self.accelerator_units.checked_sub(other.accelerator_units)?,
            lease_slots: self.lease_slots.checked_sub(other.lease_slots)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceLabel {
    pub key: u64,
    pub value: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceTaint {
    pub key: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeAvailability {
    Active,
    Draining,
    Degraded,
    Fenced,
    Incompatible,
    Failed,
}

impl NodeAvailability {
    pub const fn accepts_work(self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeResourceSpec {
    pub node: NodeId,
    pub node_epoch: u64,
    pub capacity: ResourceVector,
    pub tenant_boundary: u64,
    pub labels: [Option<ResourceLabel>; MAX_RESOURCE_LABELS],
    pub taints: [Option<ResourceTaint>; MAX_RESOURCE_TAINTS],
}

impl NodeResourceSpec {
    pub const fn new(node: NodeId, node_epoch: u64, capacity: ResourceVector) -> Self {
        Self {
            node,
            node_epoch,
            capacity,
            tenant_boundary: 0,
            labels: [None; MAX_RESOURCE_LABELS],
            taints: [None; MAX_RESOURCE_TAINTS],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkloadKind {
    Actor,
    Job,
    Service,
    RemoteSession,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkloadTarget {
    ActiveCluster,
    Member(NodeId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlacementPolicy {
    pub tenant: u64,
    pub target: WorkloadTarget,
    pub priority: u8,
    pub preemptible: bool,
    pub required_labels: [Option<ResourceLabel>; MAX_RESOURCE_LABELS],
    pub affinity: [Option<NodeId>; MAX_WORKLOAD_AFFINITY],
    pub anti_affinity: [Option<NodeId>; MAX_WORKLOAD_AFFINITY],
    pub tolerations: [Option<u64>; MAX_RESOURCE_TAINTS],
}

impl PlacementPolicy {
    pub const fn new(tenant: u64) -> Self {
        Self {
            tenant,
            target: WorkloadTarget::ActiveCluster,
            priority: 1,
            preemptible: true,
            required_labels: [None; MAX_RESOURCE_LABELS],
            affinity: [None; MAX_WORKLOAD_AFFINITY],
            anti_affinity: [None; MAX_WORKLOAD_AFFINITY],
            tolerations: [None; MAX_RESOURCE_TAINTS],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkloadSpec {
    pub kind: WorkloadKind,
    pub resources: ResourceVector,
    pub placement: PlacementPolicy,
    pub lease_duration_us: u64,
}

impl WorkloadSpec {
    pub const fn new(
        kind: WorkloadKind,
        resources: ResourceVector,
        placement: PlacementPolicy,
        lease_duration_us: u64,
    ) -> Self {
        Self {
            kind,
            resources,
            placement,
            lease_duration_us,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct WorkloadId(u64);

impl WorkloadId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkloadState {
    Queued,
    Running,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkloadRecord {
    pub id: WorkloadId,
    pub spec: WorkloadSpec,
    pub state: WorkloadState,
    pub node: Option<NodeId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkloadAdmission {
    pub workload: WorkloadRecord,
    pub preempted: [Option<WorkloadId>; MAX_PREEMPTED_WORKLOADS],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TenantQuota {
    pub tenant: u64,
    pub capacity: ResourceVector,
    pub max_workloads: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeResourceReport {
    pub node: NodeId,
    pub state: NodeAvailability,
    pub capacity: ResourceVector,
    pub available: ResourceVector,
    pub running_workloads: u32,
    pub reserved_workloads: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterResourceReport<const NODES: usize = DEFAULT_RESOURCE_NODE_CAPACITY> {
    pub generation: u64,
    pub capacity: ResourceVector,
    pub available: ResourceVector,
    pub active_nodes: u32,
    pub running_workloads: u32,
    pub queued_workloads: u32,
    pub nodes: [Option<NodeResourceReport>; NODES],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlacementFailure {
    NoEligibleNode,
    SelectedNodeUnavailable,
    LabelsNotMatched,
    AffinityNotMatched,
    AntiAffinityConflict,
    TaintNotTolerated,
    Capacity,
    Quota,
    WorkloadCapacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceError {
    InvalidRequest,
    DuplicateNode,
    NodeNotFound,
    NodeBusy,
    DuplicateTenant,
    TenantNotFound,
    Placement(PlacementFailure),
    WorkloadNotFound,
    Overflow,
}

impl IntoStatus for ResourceError {
    fn status(self) -> Status {
        match self {
            Self::Placement(PlacementFailure::Capacity)
            | Self::Placement(PlacementFailure::Quota)
            | Self::Placement(PlacementFailure::WorkloadCapacity)
            | Self::Overflow => Status::NO_SPACE,
            Self::Placement(PlacementFailure::SelectedNodeUnavailable)
            | Self::Placement(PlacementFailure::NoEligibleNode)
            | Self::Placement(PlacementFailure::LabelsNotMatched)
            | Self::Placement(PlacementFailure::AffinityNotMatched)
            | Self::Placement(PlacementFailure::AntiAffinityConflict)
            | Self::Placement(PlacementFailure::TaintNotTolerated)
            | Self::NodeBusy => Status::BUSY,
            Self::NodeNotFound | Self::TenantNotFound | Self::WorkloadNotFound => Status::NOT_FOUND,
            Self::DuplicateNode | Self::DuplicateTenant => Status::ALREADY_EXISTS,
            Self::InvalidRequest => Status::INVALID_ARGUMENT,
        }
    }
}

pub const DEFAULT_RESOURCE_NODE_CAPACITY: usize = 32;
pub const DEFAULT_RESOURCE_WORKLOAD_CAPACITY: usize = 128;
pub const DEFAULT_TENANT_CAPACITY: usize = 32;

#[derive(Clone, Copy)]
struct NodeSlot {
    spec: NodeResourceSpec,
    state: NodeAvailability,
    used: ResourceVector,
    running_workloads: u32,
}

#[derive(Clone, Copy)]
struct WorkloadSlot {
    record: Option<WorkloadRecord>,
}

#[derive(Clone, Copy)]
struct TenantSlot {
    quota: TenantQuota,
    used: ResourceVector,
    workloads: u32,
}

pub struct ResourceScheduler<
    const NODES: usize = DEFAULT_RESOURCE_NODE_CAPACITY,
    const WORKLOADS: usize = DEFAULT_RESOURCE_WORKLOAD_CAPACITY,
    const TENANTS: usize = DEFAULT_TENANT_CAPACITY,
> {
    nodes: [Option<NodeSlot>; NODES],
    workloads: [WorkloadSlot; WORKLOADS],
    tenants: [Option<TenantSlot>; TENANTS],
    next_workload: u64,
    generation: u64,
}

impl<const NODES: usize, const WORKLOADS: usize, const TENANTS: usize>
    ResourceScheduler<NODES, WORKLOADS, TENANTS>
{
    pub const fn new() -> Self {
        Self {
            nodes: [None; NODES],
            workloads: [WorkloadSlot { record: None }; WORKLOADS],
            tenants: [None; TENANTS],
            next_workload: 0,
            generation: 0,
        }
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn register_tenant(&mut self, quota: TenantQuota) -> Result<(), ResourceError> {
        if quota.tenant == 0 || quota.capacity.is_zero() || quota.max_workloads == 0 {
            return Err(ResourceError::InvalidRequest)
        }
        if let Some(entry) = self
            .tenants
            .iter_mut()
            .flatten()
            .find(|entry| entry.quota.tenant == quota.tenant)
        {
            if !quota.capacity.fits(entry.used) || quota.max_workloads < entry.workloads {
                return Err(ResourceError::NodeBusy)
            }
            entry.quota = quota;
            self.bump_generation();
            return Ok(())
        }
        let slot = self
            .tenants
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(ResourceError::Placement(PlacementFailure::WorkloadCapacity))?;
        *slot = Some(TenantSlot {
            quota,
            used: ResourceVector::default(),
            workloads: 0,
        });
        self.bump_generation();
        Ok(())
    }

    pub fn register_node(&mut self, spec: NodeResourceSpec) -> Result<(), ResourceError> {
        if spec.node_epoch == 0 || spec.capacity.is_zero() {
            return Err(ResourceError::InvalidRequest)
        }
        if let Some(entry) = self
            .nodes
            .iter_mut()
            .flatten()
            .find(|entry| entry.spec.node == spec.node)
        {
            if !spec.capacity.fits(entry.used) {
                return Err(ResourceError::NodeBusy)
            }
            entry.spec = spec;
            entry.state = NodeAvailability::Active;
            self.bump_generation();
            return Ok(())
        }
        let slot = self
            .nodes
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(ResourceError::Placement(PlacementFailure::NoEligibleNode))?;
        *slot = Some(NodeSlot {
            spec,
            state: NodeAvailability::Active,
            used: ResourceVector::default(),
            running_workloads: 0,
        });
        self.bump_generation();
        Ok(())
    }

    pub fn set_node_state(
        &mut self,
        node: NodeId,
        state: NodeAvailability,
    ) -> Result<(), ResourceError> {
        let entry = self.node_slot_mut(node)?;
        entry.state = state;
        self.bump_generation();
        Ok(())
    }

    pub fn drain_node(&mut self, node: NodeId) -> Result<(), ResourceError> {
        self.set_node_state(node, NodeAvailability::Draining)
    }

    pub fn fence_node(&mut self, node: NodeId) -> Result<(), ResourceError> {
        self.set_node_state(node, NodeAvailability::Fenced)
    }

    pub fn unfence_node(&mut self, node: NodeId) -> Result<(), ResourceError> {
        self.set_node_state(node, NodeAvailability::Active)
    }

    pub fn admit(&mut self, spec: WorkloadSpec) -> Result<WorkloadAdmission, ResourceError> {
        self.validate_spec(spec)?;
        let tenant = self.tenant_slot(spec.placement.tenant)?;
        if !tenant.quota.capacity.fits(
            tenant
                .used
                .checked_add(spec.resources)
                .ok_or(ResourceError::Overflow)?,
        ) || tenant.workloads >= tenant.quota.max_workloads
        {
            return Err(ResourceError::Placement(PlacementFailure::Quota))
        }
        let slot = self
            .workloads
            .iter()
            .position(|entry| entry.record.is_none())
            .ok_or(ResourceError::Placement(PlacementFailure::WorkloadCapacity))?;
        let id = self
            .next_workload
            .checked_add(1)
            .and_then(WorkloadId::new)
            .ok_or(ResourceError::Overflow)?;
        self.next_workload = id.raw();

        let mut preempted = [None; MAX_PREEMPTED_WORKLOADS];
        let node = self.find_node(spec);
        if node.is_none() && self.has_eligible_node(spec) {
            for item in &mut preempted {
                let Some(victim) = self.find_preemption_victim(spec) else { break };
                self.release(victim)?;
                *item = Some(victim);
                if self.find_node(spec).is_some() { break }
            }
        }
        let node = self.find_node(spec);
        let (state, assigned_node) = match node {
            Some(index) => {
                let node = self.nodes[index].expect("selected resource node").spec.node;
                self.nodes[index].as_mut().expect("selected resource node").used = self.nodes[index]
                    .expect("selected resource node")
                    .used
                    .checked_add(spec.resources)
                    .ok_or(ResourceError::Overflow)?;
                self.nodes[index]
                    .as_mut()
                    .expect("selected resource node")
                    .running_workloads += 1;
                (WorkloadState::Running, Some(node))
            }
            None if self.has_eligible_node(spec) => (WorkloadState::Queued, None),
            None => return Err(ResourceError::Placement(self.explain_failure(spec))),
        };
        self.charge_tenant(spec.placement.tenant, spec.resources)?;
        let record = WorkloadRecord { id, spec, state, node: assigned_node };
        self.workloads[slot].record = Some(record);
        self.bump_generation();
        Ok(WorkloadAdmission { workload: record, preempted })
    }

    pub fn schedule_queued(&mut self) -> u32 {
        let mut scheduled = 0;
        loop {
            let Some(slot) = self.next_queued_slot() else { break };
            let record = self.workloads[slot].record.expect("queued workload");
            let Some(node_index) = self.find_node(record.spec) else { break };
            let node = self.nodes[node_index].expect("selected resource node").spec.node;
            let entry = self.nodes[node_index].as_mut().expect("selected resource node");
            entry.used = entry.used.checked_add(record.spec.resources).expect("validated resources");
            entry.running_workloads += 1;
            self.workloads[slot].record = Some(WorkloadRecord {
                state: WorkloadState::Running,
                node: Some(node),
                ..record
            });
            scheduled += 1;
            self.bump_generation();
        }
        scheduled
    }

    pub fn cancel(&mut self, workload: WorkloadId) -> Result<WorkloadRecord, ResourceError> {
        let slot = self.workload_slot(workload)?;
        let record = self.workloads[slot].record.take().expect("located workload");
        if let Some(node) = record.node {
            let entry = self.node_slot_mut(node)?;
            entry.used = entry.used.checked_sub(record.spec.resources).expect("usage invariant");
            entry.running_workloads = entry.running_workloads.saturating_sub(1);
        }
        self.release_tenant(record.spec.placement.tenant, record.spec.resources)?;
        self.bump_generation();
        Ok(record)
    }

    pub fn migrate(&mut self, workload: WorkloadId, target: NodeId) -> Result<(), ResourceError> {
        let slot = self.workload_slot(workload)?;
        let record = self.workloads[slot].record.expect("located workload");
        if record.state != WorkloadState::Running || record.node == Some(target) {
            return Err(ResourceError::InvalidRequest)
        }
        let mut spec = record.spec;
        spec.placement.target = WorkloadTarget::Member(target);
        let target_slot = self
            .find_node(spec)
            .ok_or(ResourceError::Placement(self.explain_failure(spec)))?;
        let old = record.node.ok_or(ResourceError::InvalidRequest)?;
        let old_entry = self.node_slot_mut(old)?;
        old_entry.used = old_entry.used.checked_sub(record.spec.resources).expect("usage invariant");
        old_entry.running_workloads = old_entry.running_workloads.saturating_sub(1);
        let new_entry = self.nodes[target_slot].as_mut().expect("selected resource node");
        new_entry.used = new_entry.used.checked_add(record.spec.resources).expect("validated resources");
        new_entry.running_workloads += 1;
        self.workloads[slot].record = Some(WorkloadRecord { node: Some(target), ..record });
        self.bump_generation();
        Ok(())
    }

    pub fn failover_node(&mut self, node: NodeId) -> Result<u32, ResourceError> {
        self.set_node_state(node, NodeAvailability::Failed)?;
        let mut moved = 0;
        for slot in 0..WORKLOADS {
            let Some(record) = self.workloads[slot].record else { continue };
            if record.node != Some(node) { continue }
            let old = self.node_slot_mut(node)?;
            old.used = old.used.checked_sub(record.spec.resources).expect("usage invariant");
            old.running_workloads = old.running_workloads.saturating_sub(1);
            let mut replacement = record.spec;
            replacement.placement.target = WorkloadTarget::ActiveCluster;
            if let Some(target_slot) = self.find_node(replacement) {
                let target = self.nodes[target_slot].as_mut().expect("replacement node");
                target.used = target.used.checked_add(record.spec.resources).expect("validated resources");
                target.running_workloads += 1;
                self.workloads[slot].record = Some(WorkloadRecord {
                    node: Some(target.spec.node),
                    ..record
                });
                moved += 1;
            } else {
                self.workloads[slot].record = Some(WorkloadRecord {
                    state: WorkloadState::Queued,
                    node: None,
                    ..record
                });
            }
        }
        self.bump_generation();
        Ok(moved)
    }

    pub fn workload(&self, workload: WorkloadId) -> Result<WorkloadRecord, ResourceError> {
        Ok(self.workloads[self.workload_slot(workload)?].record.expect("located workload"))
    }

    pub fn report(&self) -> ClusterResourceReport<NODES> {
        let mut report = ClusterResourceReport {
            generation: self.generation,
            capacity: ResourceVector::default(),
            available: ResourceVector::default(),
            active_nodes: 0,
            running_workloads: 0,
            queued_workloads: 0,
            nodes: [None; NODES],
        };
        for (index, entry) in self.nodes.iter().flatten().enumerate() {
            let available = entry.spec.capacity.checked_sub(entry.used).expect("usage invariant");
            report.capacity = report.capacity.checked_add(entry.spec.capacity).expect("capacity invariant");
            if entry.state.accepts_work() {
                report.available = report.available.checked_add(available).expect("capacity invariant");
                report.active_nodes += 1;
            }
            report.running_workloads += entry.running_workloads;
            report.nodes[index] = Some(NodeResourceReport {
                node: entry.spec.node,
                state: entry.state,
                capacity: entry.spec.capacity,
                available,
                running_workloads: entry.running_workloads,
                reserved_workloads: self
                    .workloads
                    .iter()
                    .filter_map(|workload| workload.record)
                    .filter(|workload| workload.node == Some(entry.spec.node))
                    .count() as u32,
            });
        }
        report.queued_workloads = self
            .workloads
            .iter()
            .filter(|workload| workload.record.is_some_and(|record| record.state == WorkloadState::Queued))
            .count() as u32;
        report
    }

    fn validate_spec(&self, spec: WorkloadSpec) -> Result<(), ResourceError> {
        if spec.placement.tenant == 0
            || spec.placement.priority == 0
            || spec.resources.is_zero()
            || spec.lease_duration_us == 0
        {
            return Err(ResourceError::InvalidRequest)
        }
        let tenant = self.tenant_slot(spec.placement.tenant)?;
        if tenant.quota.capacity.checked_add(spec.resources).is_none() {
            return Err(ResourceError::Overflow)
        }
        Ok(())
    }

    fn find_node(&self, spec: WorkloadSpec) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let entry = (*entry)?;
                if !self.matches_placement(&entry, spec) {
                    return None
                }
                let available = entry.spec.capacity.checked_sub(entry.used)?;
                if !available.fits(spec.resources) {
                    return None
                }
                Some((index, available.cpu_millis, entry.spec.node.raw()))
            })
            .max_by_key(|candidate| (candidate.1, core::cmp::Reverse(candidate.2)))
            .map(|candidate| candidate.0)
    }

    fn has_eligible_node(&self, spec: WorkloadSpec) -> bool {
        self.nodes
            .iter()
            .flatten()
            .any(|entry| self.matches_placement(entry, spec))
    }

    fn matches_placement(&self, entry: &NodeSlot, spec: WorkloadSpec) -> bool {
        if !entry.state.accepts_work() {
            return false
        }
        if let WorkloadTarget::Member(target) = spec.placement.target
            && entry.spec.node != target
        {
            return false
        }
        if entry.spec.tenant_boundary != 0 && entry.spec.tenant_boundary != spec.placement.tenant {
            return false
        }
        self.labels_match(entry, spec)
            && self.affinity_match(entry, spec)
            && self.anti_affinity_match(entry, spec)
            && self.taints_tolerated(entry, spec)
    }

    fn explain_failure(&self, spec: WorkloadSpec) -> PlacementFailure {
        if let WorkloadTarget::Member(node) = spec.placement.target
            && self.nodes.iter().flatten().all(|entry| entry.spec.node != node)
        {
            return PlacementFailure::SelectedNodeUnavailable
        }
        if !self.has_eligible_node(spec) {
            if self.nodes.iter().flatten().any(|entry| {
                entry.state.accepts_work()
                    && matches!(spec.placement.target, WorkloadTarget::ActiveCluster)
                    || matches!(spec.placement.target, WorkloadTarget::Member(node) if entry.spec.node == node)
            }) {
                if self.nodes.iter().flatten().any(|entry| {
                    entry.state.accepts_work()
                        && !self.labels_match(entry, spec)
                }) {
                    return PlacementFailure::LabelsNotMatched
                }
                if self.nodes.iter().flatten().any(|entry| {
                    entry.state.accepts_work()
                        && self.labels_match(entry, spec)
                        && !self.affinity_match(entry, spec)
                }) {
                    return PlacementFailure::AffinityNotMatched
                }
                if self.nodes.iter().flatten().any(|entry| {
                    entry.state.accepts_work()
                        && self.labels_match(entry, spec)
                        && self.affinity_match(entry, spec)
                        && !self.anti_affinity_match(entry, spec)
                }) {
                    return PlacementFailure::AntiAffinityConflict
                }
                return PlacementFailure::TaintNotTolerated
            }
            if self.nodes.iter().flatten().any(|entry| !entry.state.accepts_work()) {
                return PlacementFailure::SelectedNodeUnavailable
            }
            return PlacementFailure::NoEligibleNode
        }
        PlacementFailure::Capacity
    }

    fn find_preemption_victim(&self, spec: WorkloadSpec) -> Option<WorkloadId> {
        self.workloads
            .iter()
            .filter_map(|entry| entry.record)
            .filter(|record| {
                record.state == WorkloadState::Running
                    && record.spec.placement.tenant == spec.placement.tenant
                    && record.spec.placement.preemptible
                    && record.spec.placement.priority < spec.placement.priority
                    && record.node.is_some_and(|node| {
                        self.nodes
                            .iter()
                            .flatten()
                            .find(|entry| entry.spec.node == node)
                            .is_some_and(|entry| self.matches_placement(entry, spec))
                    })
            })
            .min_by_key(|record| (record.spec.placement.priority, record.id.raw()))
            .map(|record| record.id)
    }

    fn next_queued_slot(&self) -> Option<usize> {
        self.workloads
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                let record = entry.record?;
                (record.state == WorkloadState::Queued).then_some((index, record.spec.placement.priority, record.id.raw()))
            })
            .max_by_key(|entry| (entry.1, core::cmp::Reverse(entry.2)))
            .map(|entry| entry.0)
    }

    fn charge_tenant(&mut self, tenant: u64, resources: ResourceVector) -> Result<(), ResourceError> {
        let entry = self.tenant_slot_mut(tenant)?;
        entry.used = entry.used.checked_add(resources).ok_or(ResourceError::Overflow)?;
        entry.workloads = entry.workloads.checked_add(1).ok_or(ResourceError::Overflow)?;
        Ok(())
    }

    fn release_tenant(&mut self, tenant: u64, resources: ResourceVector) -> Result<(), ResourceError> {
        let entry = self.tenant_slot_mut(tenant)?;
        entry.used = entry.used.checked_sub(resources).expect("tenant usage invariant");
        entry.workloads = entry.workloads.saturating_sub(1);
        Ok(())
    }

    fn release(&mut self, workload: WorkloadId) -> Result<(), ResourceError> {
        self.cancel(workload).map(|_| ())
    }

    fn labels_match(&self, entry: &NodeSlot, spec: WorkloadSpec) -> bool {
        spec.placement.required_labels.iter().flatten().all(|label| {
            entry.spec.labels.iter().flatten().any(|candidate| candidate == label)
        })
    }

    fn affinity_match(&self, entry: &NodeSlot, spec: WorkloadSpec) -> bool {
        spec.placement
            .affinity
            .iter()
            .flatten()
            .next()
            .is_none_or(|_| {
                spec.placement
                    .affinity
                    .iter()
                    .flatten()
                    .any(|node| *node == entry.spec.node)
            })
    }

    fn anti_affinity_match(&self, entry: &NodeSlot, spec: WorkloadSpec) -> bool {
        !spec
            .placement
            .anti_affinity
            .iter()
            .flatten()
            .any(|node| *node == entry.spec.node)
    }

    fn taints_tolerated(&self, entry: &NodeSlot, spec: WorkloadSpec) -> bool {
        entry.spec.taints.iter().flatten().all(|taint| {
            spec.placement
                .tolerations
                .iter()
                .flatten()
                .any(|key| *key == taint.key)
        })
    }

    fn node_slot_mut(&mut self, node: NodeId) -> Result<&mut NodeSlot, ResourceError> {
        self.nodes
            .iter_mut()
            .flatten()
            .find(|entry| entry.spec.node == node)
            .ok_or(ResourceError::NodeNotFound)
    }

    fn tenant_slot(&self, tenant: u64) -> Result<&TenantSlot, ResourceError> {
        self.tenants
            .iter()
            .flatten()
            .find(|entry| entry.quota.tenant == tenant)
            .ok_or(ResourceError::TenantNotFound)
    }

    fn tenant_slot_mut(&mut self, tenant: u64) -> Result<&mut TenantSlot, ResourceError> {
        self.tenants
            .iter_mut()
            .flatten()
            .find(|entry| entry.quota.tenant == tenant)
            .ok_or(ResourceError::TenantNotFound)
    }

    fn workload_slot(&self, workload: WorkloadId) -> Result<usize, ResourceError> {
        self.workloads
            .iter()
            .position(|entry| entry.record.is_some_and(|record| record.id == workload))
            .ok_or(ResourceError::WorkloadNotFound)
    }

    fn bump_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1)
    }
}

impl<const NODES: usize, const WORKLOADS: usize, const TENANTS: usize> Default
    for ResourceScheduler<NODES, WORKLOADS, TENANTS>
{
    fn default() -> Self {
        Self::new()
    }
}
