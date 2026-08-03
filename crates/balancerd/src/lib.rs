#![no_std]
#![forbid(unsafe_code)]

use synos_actors::{
    ActorError, ActorId, ActorRef, ActorRuntime, ActorSpawnRequest, ActorSystem,
    DEFAULT_ACTOR_CAPACITY,
};
use synos_fabric::{
    NodeId as ClusterNodeId,
    cluster::{
        Heartbeat, HeartbeatMonitor, NodeFailure, NodeState as HeartbeatNodeState,
        RecoverySummary,
        recover_failed_node,
    },
    dsm::CoherenceDirectory,
    memory::{GlobalAddressSpace, LeaseTable},
};
use synos_kernel::{
    AddressSpaceId, CapabilityHandle, CapabilitySpace, DistributedLockManager, LockError,
    LockGrant, LockHandle, LockMode, LockOwner, LockRange, NodeFenceTable, ResourceId,
    ResourceKind, ResourceName,
};
use synos_status::{IntoStatus, Severity, Status, facility};

pub const DEFAULT_NODE_CAPACITY: usize = 32;
pub const DEFAULT_JOB_CAPACITY: usize = 128;
pub const DEFAULT_THREAD_CAPACITY: usize = 16;
pub const DEFAULT_DLM_CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct JobId(u64);

impl JobId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeOffer {
    pub node: ClusterNodeId,
    pub node_epoch: u64,
    pub cpu_capacity: usize,
    pub cache_latency_ns: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobRequest {
    pub image: u128,
    pub capability_profile: u64,
    pub generation: u32,
    pub address_space: AddressSpaceId,
    pub threads: usize,
    pub resource: ResourceId,
    pub resource_name: ResourceName,
    pub cache_range: LockRange,
    pub lease_duration_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaseRecord {
    pub handle: LockHandle,
    pub owner: LockOwner,
    pub node_epoch: u64,
    pub lease_epoch: u64,
    pub expires_at_us: u64,
    pub range: LockRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlacementSnapshot<const THREADS: usize = DEFAULT_THREAD_CAPACITY> {
    pub job: JobId,
    pub actors: [Option<ActorId>; THREADS],
    pub leases: [Option<LeaseRecord>; THREADS],
}

#[derive(Debug, Eq, PartialEq)]
pub enum BalancerError<E> {
    Actor(ActorError<E>),
    Capacity,
    InvalidRequest,
    JobNotFound,
    Lease(LockError),
    MigrationFailed,
    NoNode,
    StalePlacement,
}

#[derive(Debug, Eq, PartialEq)]
pub enum FailoverError<E> {
    Fabric(synos_fabric::Error),
    Balancer(BalancerError<E>),
}

impl<E: IntoStatus> IntoStatus for FailoverError<E> {
    fn status(self) -> Status {
        match self {
            Self::Fabric(error) => error.status(),
            Self::Balancer(error) => error.status(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FailoverSummary {
    pub failure: NodeFailure,
    pub memory: RecoverySummary,
    pub threads_reassigned: usize,
}

impl<E: IntoStatus> IntoStatus for BalancerError<E> {
    fn status(self) -> Status {
        match self {
            Self::Actor(error) => error.status(),
            Self::Lease(error) => error.status(),
            Self::Capacity => Status::NO_SPACE,
            Self::NoNode | Self::JobNotFound => Status::NOT_FOUND,
            Self::InvalidRequest | Self::StalePlacement => Status::INVALID_ARGUMENT,
            Self::MigrationFailed => Status::new(Severity::Error, facility::FABRIC, 5, 0)
                .expect("valid balancer status"),
        }
    }
}

#[derive(Clone, Copy)]
struct NodeState {
    offer: NodeOffer,
    active_threads: usize,
    failed: bool,
}

#[derive(Clone, Copy)]
struct JobSlot<const THREADS: usize> {
    id: Option<JobId>,
    request: Option<JobRequest>,
    actors: [Option<ActorRef>; THREADS],
    leases: [Option<LeaseRecord>; THREADS],
}

impl<const THREADS: usize> JobSlot<THREADS> {
    const EMPTY: Self = Self {
        id: None,
        request: None,
        actors: [None; THREADS],
        leases: [None; THREADS],
    };
}

/// Active-active compute placement for actor threads.
///
/// The coordinator owns only bounded metadata. Actor execution remains in
/// `synos-actors`, while every cache slice is protected by a kernel DLM lease.
pub struct Balancer<
    const NODES: usize = DEFAULT_NODE_CAPACITY,
    const JOBS: usize = DEFAULT_JOB_CAPACITY,
    const THREADS: usize = DEFAULT_THREAD_CAPACITY,
> {
    actors: ActorSystem<DEFAULT_ACTOR_CAPACITY, NODES>,
    nodes: [Option<NodeState>; NODES],
    jobs: [JobSlot<THREADS>; JOBS],
    next_job: u64,
    next_actor_local: u64,
}

impl<const NODES: usize, const JOBS: usize, const THREADS: usize>
    Balancer<NODES, JOBS, THREADS>
{
    pub const fn new(local: ClusterNodeId) -> Self {
        Self {
            actors: ActorSystem::new(local),
            nodes: [None; NODES],
            jobs: [JobSlot::EMPTY; JOBS],
            next_job: 0,
            next_actor_local: 0,
        }
    }

    pub const fn actors(&self) -> &ActorSystem<DEFAULT_ACTOR_CAPACITY, NODES> {
        &self.actors
    }

    pub fn actors_mut(&mut self) -> &mut ActorSystem<DEFAULT_ACTOR_CAPACITY, NODES> {
        &mut self.actors
    }

    pub fn admit_node(&mut self, offer: NodeOffer) -> Result<(), BalancerError<core::convert::Infallible>> {
        if offer.node_epoch == 0 || offer.cpu_capacity == 0 {
            return Err(BalancerError::InvalidRequest)
        }
        if let Some(node) = self
            .nodes
            .iter_mut()
            .flatten()
            .find(|node| node.offer.node == offer.node)
        {
            if node.failed && node.active_threads != 0 {
                return Err(BalancerError::Capacity)
            }
            if offer.cpu_capacity < node.active_threads {
                return Err(BalancerError::Capacity)
            }
            node.offer = offer;
            node.failed = false;
            return Ok(())
        }
        let slot = self
            .nodes
            .iter_mut()
            .find(|node| node.is_none())
            .ok_or(BalancerError::Capacity)?;
        *slot = Some(NodeState {
            offer,
            active_threads: 0,
            failed: false,
        });
        Ok(())
    }

    pub fn remove_node(&mut self, node: ClusterNodeId) -> Result<(), BalancerError<core::convert::Infallible>> {
        let state = self
            .nodes
            .iter_mut()
            .flatten()
            .find(|state| state.offer.node == node)
            .ok_or(BalancerError::NoNode)?;
        if state.active_threads != 0 {
            return Err(BalancerError::Capacity)
        }
        let slot = self
            .nodes
            .iter()
            .position(|entry| entry.is_some_and(|entry| entry.offer.node == node))
            .expect("located node entry");
        self.nodes[slot] = None;
        Ok(())
    }

    pub fn submit<T, const CAPABILITIES: usize, const DLM: usize, const FENCES: usize>(
        &mut self,
        transport: &mut T,
        dlm: &mut DistributedLockManager<DLM>,
        capabilities: &CapabilitySpace<CAPABILITIES>,
        authority: CapabilityHandle,
        request: JobRequest,
        now_us: u64,
        fences: &NodeFenceTable<FENCES>,
    ) -> Result<JobId, BalancerError<T::Error>>
    where
        T: ActorRuntime,
    {
        self.validate_request(request)?;
        let slot = self
            .jobs
            .iter()
            .position(|job| job.id.is_none())
            .ok_or(BalancerError::Capacity)?;
        let job = self.allocate_job_id().ok_or(BalancerError::Capacity)?;
        let mut actors = [None; THREADS];
        let mut leases = [None; THREADS];
        let mut selected = [None; THREADS];
        let mut selected_count = [0usize; NODES];

        for thread in 0..request.threads {
            let Some(node_slot) = self.pick_node(&selected_count) else {
                self.rollback_submission(transport, dlm, &actors, &leases);
                return Err(BalancerError::NoNode)
            };
            selected[thread] = Some(node_slot);
            selected_count[node_slot] += 1;
            let state = self.nodes[node_slot].expect("selected node");
            let owner = LockOwner {
                node: kernel_node(state.offer.node),
                address_space: request.address_space,
            };
            let range = request
                .cache_range
                .thread_slice(thread, request.threads)
                .ok_or(BalancerError::InvalidRequest)
                .map_err(|error| {
                    self.rollback_submission(transport, dlm, &actors, &leases);
                    error
                })?;
            let handle = match dlm
                .acquire_node_range(
                    capabilities,
                    authority,
                    owner,
                    state.offer.node_epoch,
                    request.resource,
                    ResourceKind::SharedMemory,
                    request.resource_name,
                    range,
                    LockMode::Exclusive,
                    false,
                    now_us,
                    request.lease_duration_us,
                    fences,
                )
                .map_err(BalancerError::Lease)
            {
                Ok(LockGrant::Granted(handle)) => handle,
                Err(error) => {
                    self.rollback_submission(transport, dlm, &actors, &leases);
                    return Err(error)
                }
                Ok(LockGrant::Queued(_)) => {
                    self.rollback_submission(transport, dlm, &actors, &leases);
                    return Err(BalancerError::Lease(LockError::WouldBlock))
                }
            };
            let (lease_range, lease_epoch, expires_at_us) =
                match dlm.lease(handle).map_err(BalancerError::Lease) {
                    Ok(lease) => lease,
                    Err(error) => {
                        self.rollback_submission(transport, dlm, &actors, &leases);
                        return Err(error)
                    }
                };
            leases[thread] = Some(LeaseRecord {
                handle,
                owner,
                node_epoch: state.offer.node_epoch,
                lease_epoch,
                expires_at_us,
                range: lease_range,
            });
            let actor = ActorId::new(state.offer.node, self.next_actor_id())
                .ok_or(BalancerError::Capacity)
                .map_err(|error| {
                    self.rollback_submission(transport, dlm, &actors, &leases);
                    error
                })?;
            let actor_ref = self
                .actors
                .orchestrate(
                    transport,
                    ActorSpawnRequest {
                        actor,
                        image: request.image,
                        capability_profile: request.capability_profile,
                        generation: request.generation,
                    },
                )
                .map_err(|error| {
                    self.rollback_submission(transport, dlm, &actors, &leases);
                    BalancerError::Actor(error)
                })?;
            actors[thread] = Some(actor_ref);
        }

        for node_slot in selected.iter().flatten() {
            self.nodes[*node_slot]
                .as_mut()
                .expect("selected node")
                .active_threads += 1;
        }
        self.jobs[slot] = JobSlot {
            id: Some(job),
            request: Some(request),
            actors,
            leases,
        };
        Ok(job)
    }

    pub fn renew_job<const DLM: usize, const FENCES: usize>(
        &mut self,
        job: JobId,
        dlm: &mut DistributedLockManager<DLM>,
        now_us: u64,
        fences: &NodeFenceTable<FENCES>,
    ) -> Result<(), BalancerError<core::convert::Infallible>> {
        let slot = self.job_slot(job)?;
        let request = self.jobs[slot].request.expect("occupied job");
        for lease in self.jobs[slot].leases.iter_mut().flatten() {
            lease.lease_epoch = dlm
                .renew_node(
                    lease.owner,
                    lease.handle,
                    lease.node_epoch,
                    lease.lease_epoch,
                    now_us,
                    request.lease_duration_us,
                    fences,
                )
                .map_err(BalancerError::Lease)?;
            lease.expires_at_us = now_us.saturating_add(request.lease_duration_us);
        }
        Ok(())
    }

    /// Move one actor to a less loaded node through a lease handoff.
    ///
    /// The old actor is stopped before its cache slice is released. The new
    /// node cannot acquire that slice until the old lease is gone, so a move
    /// never runs two writers against the same cache range.
    pub fn migrate_thread<T, const CAPABILITIES: usize, const DLM: usize, const FENCES: usize>(
        &mut self,
        job: JobId,
        thread: usize,
        transport: &mut T,
        dlm: &mut DistributedLockManager<DLM>,
        capabilities: &CapabilitySpace<CAPABILITIES>,
        authority: CapabilityHandle,
        now_us: u64,
        fences: &NodeFenceTable<FENCES>,
    ) -> Result<ClusterNodeId, BalancerError<T::Error>>
    where
        T: ActorRuntime,
    {
        if thread >= THREADS {
            return Err(BalancerError::InvalidRequest)
        }
        let slot = self.job_slot(job).map_err(convert_error)?;
        let entry = self.jobs[slot];
        let request = entry.request.expect("occupied job");
        let old_actor = entry.actors[thread].ok_or(BalancerError::StalePlacement)?;
        let old_lease = entry.leases[thread].ok_or(BalancerError::StalePlacement)?;
        let target_slot = self
            .pick_migration_node(old_lease.owner.node)
            .ok_or(BalancerError::NoNode)?;
        let target = self.nodes[target_slot].expect("selected node");

        self.actors
            .stop(transport, old_actor)
            .map_err(BalancerError::Actor)?;
        dlm.release(old_lease.owner, old_lease.handle)
            .map_err(BalancerError::Lease)?;
        self.decrement_node(old_lease.owner.node);

        let new_owner = LockOwner {
            node: kernel_node(target.offer.node),
            address_space: request.address_space,
        };
        let new_lease = match self.acquire_lease(
            dlm,
            capabilities,
            authority,
            request,
            old_lease.range,
            new_owner,
            target.offer.node_epoch,
            now_us,
            fences,
        ) {
            Ok(lease) => lease,
            Err(error) => {
                return self.migration_failure(
                    slot,
                    thread,
                    transport,
                    dlm,
                    capabilities,
                    authority,
                    request,
                    old_lease,
                    old_actor,
                    now_us,
                    fences,
                    convert_error(error),
                )
            }
        };
        let new_actor_id = match ActorId::new(target.offer.node, self.next_actor_id()) {
            Some(actor) => actor,
            None => {
                let _ = dlm.release(new_lease.owner, new_lease.handle);
                return self.migration_failure(
                    slot,
                    thread,
                    transport,
                    dlm,
                    capabilities,
                    authority,
                    request,
                    old_lease,
                    old_actor,
                    now_us,
                    fences,
                    BalancerError::Capacity,
                )
            }
        };
        let new_actor = match self.actors.orchestrate(
            transport,
            ActorSpawnRequest {
                actor: new_actor_id,
                image: request.image,
                capability_profile: request.capability_profile,
                generation: request.generation.wrapping_add(1).max(1),
            },
        ) {
            Ok(actor) => actor,
            Err(error) => {
                let _ = dlm.release(new_lease.owner, new_lease.handle);
                return self.migration_failure(
                    slot,
                    thread,
                    transport,
                    dlm,
                    capabilities,
                    authority,
                    request,
                    old_lease,
                    old_actor,
                    now_us,
                    fences,
                    BalancerError::Actor(error),
                )
            }
        };
        self.nodes[target_slot]
            .as_mut()
            .expect("selected node")
            .active_threads += 1;
        self.jobs[slot].actors[thread] = Some(new_actor);
        self.jobs[slot].leases[thread] = Some(new_lease);
        Ok(target.offer.node)
    }

    /// Recreate every actor thread hosted by an isolated node.
    ///
    /// The old actor is forgotten without contacting its node. Its DLM leases
    /// are evicted only after the caller has confirmed hardware isolation, so
    /// the replacement can safely acquire the same cache slices.
    #[allow(clippy::too_many_arguments)]
    pub fn reassign_failed_node<
        T,
        const CAPABILITIES: usize,
        const DLM: usize,
        const FENCES: usize,
    >(
        &mut self,
        failed: ClusterNodeId,
        transport: &mut T,
        dlm: &mut DistributedLockManager<DLM>,
        capabilities: &CapabilitySpace<CAPABILITIES>,
        authority: CapabilityHandle,
        now_us: u64,
        fences: &NodeFenceTable<FENCES>,
    ) -> Result<usize, BalancerError<T::Error>>
    where
        T: ActorRuntime,
    {
        let failed_kernel = kernel_node(failed);
        let failed_slot = self
            .nodes
            .iter()
            .position(|state| state.is_some_and(|state| state.offer.node == failed))
            .ok_or(BalancerError::NoNode)?;
        dlm.evict_node(failed_kernel, fences)
            .map_err(BalancerError::Lease)?;
        self.nodes[failed_slot]
            .as_mut()
            .expect("located failed node")
            .failed = true;

        let mut reassigned = 0;
        for slot in 0..JOBS {
            for thread in 0..THREADS {
                let owned_by_failed = self.jobs[slot].leases[thread]
                    .is_some_and(|lease| lease.owner.node == failed_kernel);
                if owned_by_failed {
                    self.reassign_failed_thread(
                        slot,
                        thread,
                        transport,
                        dlm,
                        capabilities,
                        authority,
                        now_us,
                        fences,
                    )?;
                    reassigned += 1;
                }
            }
        }
        Ok(reassigned)
    }

    #[allow(clippy::too_many_arguments)]
    fn reassign_failed_thread<
        T,
        const CAPABILITIES: usize,
        const DLM: usize,
        const FENCES: usize,
    >(
        &mut self,
        slot: usize,
        thread: usize,
        transport: &mut T,
        dlm: &mut DistributedLockManager<DLM>,
        capabilities: &CapabilitySpace<CAPABILITIES>,
        authority: CapabilityHandle,
        now_us: u64,
        fences: &NodeFenceTable<FENCES>,
    ) -> Result<(), BalancerError<T::Error>>
    where
        T: ActorRuntime,
    {
        let entry = self.jobs[slot];
        let request = entry.request.ok_or(BalancerError::StalePlacement)?;
        let old_actor = entry.actors[thread].ok_or(BalancerError::StalePlacement)?;
        let old_lease = entry.leases[thread].ok_or(BalancerError::StalePlacement)?;
        let target_slot = self
            .pick_migration_node(old_lease.owner.node)
            .ok_or(BalancerError::NoNode)?;
        let target = self.nodes[target_slot].expect("selected replacement node");
        let new_owner = LockOwner {
            node: kernel_node(target.offer.node),
            address_space: request.address_space,
        };
        let new_lease = self.acquire_lease(
            dlm,
            capabilities,
            authority,
            request,
            old_lease.range,
            new_owner,
            target.offer.node_epoch,
            now_us,
            fences,
        )
        .map_err(convert_error)?;

        self.actors
            .forget(old_actor)
            .map_err(convert_actor_error)
            .map_err(BalancerError::Actor)?;
        let new_actor_id = ActorId::new(target.offer.node, self.next_actor_id())
            .ok_or(BalancerError::Capacity)?;
        let new_actor = match self.actors.orchestrate(
            transport,
            ActorSpawnRequest {
                actor: new_actor_id,
                image: request.image,
                capability_profile: request.capability_profile,
                generation: request.generation.wrapping_add(1).max(1),
            },
        ) {
            Ok(actor) => actor,
            Err(error) => {
                let _ = dlm.release(new_lease.owner, new_lease.handle);
                return Err(BalancerError::Actor(error))
            }
        };

        self.nodes[target_slot]
            .as_mut()
            .expect("selected replacement node")
            .active_threads += 1;
        if let Some(state) = self.nodes.iter_mut().flatten().find(|state| {
            kernel_node(state.offer.node) == old_lease.owner.node
        }) {
            state.active_threads = state.active_threads.saturating_sub(1)
        }
        self.jobs[slot].actors[thread] = Some(new_actor);
        self.jobs[slot].leases[thread] = Some(new_lease);
        Ok(())
    }

    pub fn snapshot(&self, job: JobId) -> Result<PlacementSnapshot<THREADS>, BalancerError<core::convert::Infallible>> {
        let slot = self.job_slot(job)?;
        let entry = &self.jobs[slot];
        let mut actors = [None; THREADS];
        for (target, actor) in actors.iter_mut().zip(entry.actors) {
            *target = actor.map(|actor| actor.id());
        }
        Ok(PlacementSnapshot {
            job,
            actors,
            leases: entry.leases,
        })
    }

    pub fn stop<T, const DLM: usize>(
        &mut self,
        transport: &mut T,
        dlm: &mut DistributedLockManager<DLM>,
        job: JobId,
    ) -> Result<(), BalancerError<T::Error>>
    where
        T: ActorRuntime,
    {
        let slot = self.job_slot(job).map_err(convert_error)?;
        let entry = self.jobs[slot];
        for actor in entry.actors.iter().flatten() {
            self.actors.stop(transport, *actor).map_err(BalancerError::Actor)?;
        }
        for lease in entry.leases.iter().flatten() {
            dlm.release(lease.owner, lease.handle)
                .map_err(BalancerError::Lease)?;
            self.decrement_node(lease.owner.node);
        }
        self.jobs[slot] = JobSlot::EMPTY;
        Ok(())
    }

    fn validate_request<E>(&self, request: JobRequest) -> Result<(), BalancerError<E>> {
        if request.image == 0
            || request.generation == 0
            || request.threads == 0
            || request.threads > THREADS
            || request.lease_duration_us == 0
            || request.resource_name.as_str().is_empty()
        {
            return Err(BalancerError::InvalidRequest)
        }
        if request
            .cache_range
            .thread_slice(request.threads.saturating_sub(1), request.threads)
            .is_none()
        {
            return Err(BalancerError::InvalidRequest)
        }
        Ok(())
    }

    fn pick_node(&self, selected: &[usize; NODES]) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, state)| {
                let state = (*state)?;
                if state.failed {
                    return None
                }
                let reserved = selected[index];
                if state.offer.cpu_capacity < state.active_threads + reserved {
                    return None
                }
                let projected = state.active_threads + reserved;
                let load = projected
                    .saturating_mul(1_000_000)
                    .checked_div(state.offer.cpu_capacity)
                    .unwrap_or(usize::MAX);
                Some((load, state.offer.cache_latency_ns, state.offer.node.raw(), index))
            })
            .min_by_key(|entry| (entry.0, entry.1, entry.2))
            .map(|entry| entry.3)
    }

    fn pick_migration_node(&self, excluded: synos_kernel::NodeId) -> Option<usize> {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, state)| {
                let state = (*state)?;
                if state.failed
                    || kernel_node(state.offer.node) == excluded
                    || state.active_threads >= state.offer.cpu_capacity
                {
                    return None
                }
                let load = state
                    .active_threads
                    .saturating_mul(1_000_000)
                    .checked_div(state.offer.cpu_capacity)
                    .unwrap_or(usize::MAX);
                Some((load, state.offer.cache_latency_ns, state.offer.node.raw(), index))
            })
            .min_by_key(|entry| (entry.0, entry.1, entry.2))
            .map(|entry| entry.3)
    }

    fn acquire_lease<const CAPABILITIES: usize, const DLM: usize, const FENCES: usize>(
        &self,
        dlm: &mut DistributedLockManager<DLM>,
        capabilities: &CapabilitySpace<CAPABILITIES>,
        authority: CapabilityHandle,
        request: JobRequest,
        range: LockRange,
        owner: LockOwner,
        node_epoch: u64,
        now_us: u64,
        fences: &NodeFenceTable<FENCES>,
    ) -> Result<LeaseRecord, BalancerError<core::convert::Infallible>> {
        let handle = match dlm
            .acquire_node_range(
                capabilities,
                authority,
                owner,
                node_epoch,
                request.resource,
                ResourceKind::SharedMemory,
                request.resource_name,
                range,
                LockMode::Exclusive,
                false,
                now_us,
                request.lease_duration_us,
                fences,
            )
            .map_err(BalancerError::Lease)?
        {
            LockGrant::Granted(handle) => handle,
            LockGrant::Queued(_) => return Err(BalancerError::Lease(LockError::WouldBlock)),
        };
        let (range, lease_epoch, expires_at_us) = dlm.lease(handle).map_err(BalancerError::Lease)?;
        Ok(LeaseRecord {
            handle,
            owner,
            node_epoch,
            lease_epoch,
            expires_at_us,
            range,
        })
    }

    fn restore_actor<T, const CAPABILITIES: usize, const DLM: usize, const FENCES: usize>(
        &mut self,
        transport: &mut T,
        dlm: &mut DistributedLockManager<DLM>,
        capabilities: &CapabilitySpace<CAPABILITIES>,
        authority: CapabilityHandle,
        request: JobRequest,
        old_lease: LeaseRecord,
        now_us: u64,
        fences: &NodeFenceTable<FENCES>,
        old_actor: ActorId,
    ) -> Option<(ActorRef, LeaseRecord)> where
        T: ActorRuntime,
    {
        let Ok(lease) = self.acquire_lease(
            dlm,
            capabilities,
            authority,
            request,
            old_lease.range,
            old_lease.owner,
            old_lease.node_epoch,
            now_us,
            fences,
        ) else {
            return None
        };
        let Ok(actor) = self.actors.orchestrate(
            transport,
            ActorSpawnRequest {
                actor: old_actor,
                image: request.image,
                capability_profile: request.capability_profile,
                generation: request.generation,
            },
        ) else {
            let _ = dlm.release(lease.owner, lease.handle);
            return None
        };
        Some((actor, lease))
    }

    fn migration_failure<T, const CAPABILITIES: usize, const DLM: usize, const FENCES: usize>(
        &mut self,
        slot: usize,
        thread: usize,
        transport: &mut T,
        dlm: &mut DistributedLockManager<DLM>,
        capabilities: &CapabilitySpace<CAPABILITIES>,
        authority: CapabilityHandle,
        request: JobRequest,
        old_lease: LeaseRecord,
        old_actor: ActorRef,
        now_us: u64,
        fences: &NodeFenceTable<FENCES>,
        error: BalancerError<T::Error>,
    ) -> Result<ClusterNodeId, BalancerError<T::Error>>
    where
        T: ActorRuntime,
    {
        let Some((actor, lease)) = self.restore_actor(
            transport,
            dlm,
            capabilities,
            authority,
            request,
            old_lease,
            now_us,
            fences,
            old_actor.id(),
        ) else {
            return Err(BalancerError::MigrationFailed)
        };
        self.nodes
            .iter_mut()
            .flatten()
            .find(|state| state.offer.node == actor.id().node)
            .expect("restored node")
            .active_threads += 1;
        self.jobs[slot].actors[thread] = Some(actor);
        self.jobs[slot].leases[thread] = Some(lease);
        Err(error)
    }

    fn rollback_submission<T, const DLM: usize>(
        &mut self,
        transport: &mut T,
        dlm: &mut DistributedLockManager<DLM>,
        actors: &[Option<ActorRef>; THREADS],
        leases: &[Option<LeaseRecord>; THREADS],
    ) where
        T: ActorRuntime,
    {
        for actor in actors.iter().flatten() {
            let _ = self.actors.stop(transport, *actor);
        }
        for lease in leases.iter().flatten() {
            let _ = dlm.release(lease.owner, lease.handle);
        }
    }

    fn allocate_job_id(&mut self) -> Option<JobId> {
        self.next_job = self.next_job.wrapping_add(1).max(1);
        JobId::new(self.next_job)
    }

    fn next_actor_id(&mut self) -> u64 {
        self.next_actor_local = self.next_actor_local.wrapping_add(1).max(1);
        self.next_actor_local
    }

    fn job_slot<E>(&self, job: JobId) -> Result<usize, BalancerError<E>> {
        self.jobs
            .iter()
            .position(|entry| entry.id == Some(job))
            .ok_or(BalancerError::JobNotFound)
    }

    fn decrement_node(&mut self, node: synos_kernel::NodeId) {
        if let Some(state) = self.nodes.iter_mut().flatten().find(|state| {
            kernel_node(state.offer.node) == node
        }) {
            state.active_threads = state.active_threads.saturating_sub(1)
        }
    }
}

impl<const NODES: usize, const JOBS: usize, const THREADS: usize> Default
    for Balancer<NODES, JOBS, THREADS>
{
    fn default() -> Self {
        Self::new(ClusterNodeId::LOCAL)
    }
}

/// Drives hardware heartbeats and turns one failure decision into a complete
/// active-active recovery transaction.
pub struct FailoverCoordinator<const HEARTBEATS: usize = DEFAULT_NODE_CAPACITY> {
    monitor: HeartbeatMonitor<HEARTBEATS>,
}

impl<const HEARTBEATS: usize> FailoverCoordinator<HEARTBEATS> {
    pub fn new(
        local: ClusterNodeId,
        period_us: u32,
        missed_limit: u8,
        now_us: u64,
    ) -> Result<Self, synos_fabric::Error> {
        Ok(Self {
            monitor: HeartbeatMonitor::new(local, period_us, missed_limit, now_us)?,
        })
    }

    pub fn add_node(
        &mut self,
        node: ClusterNodeId,
        now_us: u64,
    ) -> Result<(), synos_fabric::Error> {
        self.monitor.add_node(node, now_us)
    }

    pub fn due(&mut self, now_us: u64) -> Option<Heartbeat> {
        self.monitor.due(now_us)
    }

    pub fn observe(
        &mut self,
        heartbeat: Heartbeat,
        received_at_us: u64,
    ) -> Result<(), synos_fabric::Error> {
        self.monitor.observe(heartbeat, received_at_us)
    }

    pub fn state(&self, node: ClusterNodeId) -> Option<HeartbeatNodeState> {
        self.monitor.state(node)
    }

    /// Detect at most one node per timer tick and recover it before returning.
    /// Calling this from the NIC timer keeps detection and redirection bounded
    /// while allowing the caller to emit one completion record per failure.
    #[allow(clippy::too_many_arguments)]
    pub fn detect_and_recover<
        I,
        T,
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
        const PAGES: usize,
        const CAPABILITIES: usize,
        const DLM: usize,
        const FENCES: usize,
        const NODES: usize,
        const JOBS: usize,
        const THREADS: usize,
    >(
        &mut self,
        now_us: u64,
        isolation: &I,
        space: &mut GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &mut LeaseTable<LEASES>,
        coherence: &mut CoherenceDirectory<PAGES>,
        balancer: &mut Balancer<NODES, JOBS, THREADS>,
        transport: &mut T,
        dlm: &mut DistributedLockManager<DLM>,
        capabilities: &CapabilitySpace<CAPABILITIES>,
        authority: CapabilityHandle,
        fences: &NodeFenceTable<FENCES>,
    ) -> Result<Option<FailoverSummary>, FailoverError<T::Error>>
    where
        I: synos_fabric::cluster::NodeIsolation,
        T: ActorRuntime,
    {
        let Some(failure) = self.monitor.detect(now_us) else {
            return Ok(None)
        };
        let memory = recover_failed_node(failure, isolation, space, leases, coherence)
            .map_err(FailoverError::Fabric)?;
        let threads_reassigned = balancer
            .reassign_failed_node(
                failure.node,
                transport,
                dlm,
                capabilities,
                authority,
                now_us,
                fences,
            )
            .map_err(FailoverError::Balancer)?;
        Ok(Some(FailoverSummary {
            failure,
            memory,
            threads_reassigned,
        }))
    }
}

trait ThreadSlice {
    fn thread_slice(self, index: usize, threads: usize) -> Option<LockRange>;
}

impl ThreadSlice for LockRange {
    fn thread_slice(self, index: usize, threads: usize) -> Option<LockRange> {
        if threads == 0 || index >= threads {
            return None
        }
        match self {
            LockRange::Bytes { start, length } => {
                let offset = length.checked_mul(index as u64)?;
                let total = length.checked_mul(threads as u64)?;
                let end = start.checked_add(total)?;
                if end == start {
                    None
                } else {
                    Some(LockRange::bytes(start.checked_add(offset)?, length).ok()?)
                }
            }
            LockRange::WholeObject if threads == 1 => Some(LockRange::WholeObject),
            LockRange::WholeObject => None,
        }
    }
}

fn kernel_node(node: ClusterNodeId) -> synos_kernel::NodeId {
    synos_kernel::NodeId::new(node.raw()).expect("cluster node invariant")
}

fn convert_error<E>(error: BalancerError<core::convert::Infallible>) -> BalancerError<E> {
    match error {
        BalancerError::Actor(error) => BalancerError::Actor(match error {
            ActorError::Transport(never) => match never {},
            ActorError::ActorFailed(status) => ActorError::ActorFailed(status),
            ActorError::AlreadyRegistered => ActorError::AlreadyRegistered,
            ActorError::Capacity => ActorError::Capacity,
            ActorError::CorruptEnvelope => ActorError::CorruptEnvelope,
            ActorError::InvalidActor => ActorError::InvalidActor,
            ActorError::InvalidEndpoint => ActorError::InvalidEndpoint,
            ActorError::InvalidMailbox => ActorError::InvalidMailbox,
            ActorError::NodeRouteMissing => ActorError::NodeRouteMissing,
            ActorError::NotFound => ActorError::NotFound,
        }),
        BalancerError::Capacity => BalancerError::Capacity,
        BalancerError::InvalidRequest => BalancerError::InvalidRequest,
        BalancerError::JobNotFound => BalancerError::JobNotFound,
        BalancerError::Lease(error) => BalancerError::Lease(error),
        BalancerError::MigrationFailed => BalancerError::MigrationFailed,
        BalancerError::NoNode => BalancerError::NoNode,
        BalancerError::StalePlacement => BalancerError::StalePlacement,
    }
}

fn convert_actor_error<E>(error: ActorError<core::convert::Infallible>) -> ActorError<E> {
    match error {
        ActorError::ActorFailed(status) => ActorError::ActorFailed(status),
        ActorError::AlreadyRegistered => ActorError::AlreadyRegistered,
        ActorError::Capacity => ActorError::Capacity,
        ActorError::CorruptEnvelope => ActorError::CorruptEnvelope,
        ActorError::InvalidActor => ActorError::InvalidActor,
        ActorError::InvalidEndpoint => ActorError::InvalidEndpoint,
        ActorError::InvalidMailbox => ActorError::InvalidMailbox,
        ActorError::NodeRouteMissing => ActorError::NodeRouteMissing,
        ActorError::NotFound => ActorError::NotFound,
        ActorError::Transport(never) => match never {},
    }
}
