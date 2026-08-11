//! Bounded pools for objects that are created and reclaimed on hot paths.
//!
//! The allocator never touches the global heap. Each NUMA node has one pool
//! per CPU and per object kind. Allocation checks the current CPU, then other
//! CPUs on the same node, and finally a configured number of remote nodes.
//! This makes the remote-memory cost explicit and measurable.

use synos_observability::{field, EventField, EventKind};

const BITMAP_WORDS: usize = 4;
const MAX_POOL_SLOTS: usize = BITMAP_WORDS * u64::BITS as usize;
const PROBE_BUCKETS: usize = 8;
const KIND_COUNT: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotObjectKind {
    Ipc,
    Packet,
    Timer,
    Scheduler,
}

impl HotObjectKind {
    const fn index(self) -> usize {
        match self {
            Self::Ipc => 0,
            Self::Packet => 1,
            Self::Timer => 2,
            Self::Scheduler => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotAllocatorConfigError {
    NoCpus,
    NoNodes,
    InvalidNode,
    InvalidCapacity,
    InvalidFallbackLimit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotAllocationError {
    InvalidCpu,
    Exhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotReclaimError {
    InvalidCpu,
    InvalidNode,
    InvalidSlot,
    AlreadyFree,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotAllocationPlacement {
    LocalCpu,
    LocalNode,
    RemoteNode,
}

/// Opaque ownership token returned by [`HotObjectAllocator::allocate`].
///
/// The token records its pool, so an object can be reclaimed by a different
/// CPU without silently returning it to the wrong NUMA node.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotAllocation {
    kind: HotObjectKind,
    node: u8,
    cpu: u16,
    slot: u16,
    placement: HotAllocationPlacement,
}

impl HotAllocation {
    pub const fn kind(self) -> HotObjectKind {
        self.kind
    }

    pub const fn node(self) -> u8 {
        self.node
    }

    pub const fn cpu(self) -> u16 {
        self.cpu
    }

    pub const fn slot(self) -> u16 {
        self.slot
    }

    pub const fn placement(self) -> HotAllocationPlacement {
        self.placement
    }
}

/// Counters needed to compare locality, allocator tail work, and reclamation
/// behavior under a mixed workload.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotAllocatorStats {
    pub allocations: u64,
    pub allocation_failures: u64,
    pub local_cpu_allocations: u64,
    pub local_node_allocations: u64,
    pub remote_node_allocations: u64,
    pub remote_memory_bytes: u64,
    pub total_probe_steps: u64,
    pub max_probe_steps: u16,
    pub probe_buckets: [u64; PROBE_BUCKETS],
    pub reclaims: u64,
    pub remote_reclaims: u64,
    pub invalid_reclaims: u64,
}

impl HotAllocatorStats {
    pub const fn new() -> Self {
        Self {
            allocations: 0,
            allocation_failures: 0,
            local_cpu_allocations: 0,
            local_node_allocations: 0,
            remote_node_allocations: 0,
            remote_memory_bytes: 0,
            total_probe_steps: 0,
            max_probe_steps: 0,
            probe_buckets: [0; PROBE_BUCKETS],
            reclaims: 0,
            remote_reclaims: 0,
            invalid_reclaims: 0,
        }
    }

    pub const fn average_probe_steps(self) -> u64 {
        if self.allocations == 0 {
            0
        } else {
            self.total_probe_steps / self.allocations
        }
    }
}

impl Default for HotAllocatorStats {
    fn default() -> Self {
        Self::new()
    }
}

/// A point-in-time report for one object kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotAllocatorReport {
    pub kind: HotObjectKind,
    pub capacity: u32,
    pub in_use: u32,
    pub free: u32,
    pub largest_free_run: u32,
    /// Free space not in the largest free run, in per-mille.
    pub fragmentation_per_mille: u16,
    pub stats: HotAllocatorStats,
}

#[derive(Clone, Copy)]
struct Pool {
    capacity: u16,
    used: u16,
    free: [u64; BITMAP_WORDS],
}

impl Pool {
    fn new(capacity: usize) -> Self {
        let mut free = [0; BITMAP_WORDS];
        let full_words = capacity / u64::BITS as usize;
        let remainder = capacity % u64::BITS as usize;
        let mut word = 0;
        while word < full_words {
            free[word] = u64::MAX;
            word += 1;
        }
        if remainder != 0 {
            free[full_words] = (1u64 << remainder) - 1;
        }
        Self {
            capacity: capacity as u16,
            used: 0,
            free,
        }
    }

    fn take(&mut self) -> Option<u16> {
        let mut word = 0;
        while word < BITMAP_WORDS {
            let available = self.free[word];
            if available != 0 {
                let bit = available.trailing_zeros() as usize;
                self.free[word] &= !(1u64 << bit);
                self.used += 1;
                return Some((word * u64::BITS as usize + bit) as u16)
            }
            word += 1;
        }
        None
    }

    fn release(&mut self, slot: u16) -> Result<(), HotReclaimError> {
        if slot >= self.capacity || slot as usize >= MAX_POOL_SLOTS {
            return Err(HotReclaimError::InvalidSlot)
        }
        let word = slot as usize / u64::BITS as usize;
        let bit = slot as usize % u64::BITS as usize;
        let mask = 1u64 << bit;
        if self.free[word] & mask != 0 {
            return Err(HotReclaimError::AlreadyFree)
        }
        self.free[word] |= mask;
        self.used -= 1;
        Ok(())
    }

    fn free_count(self) -> u16 {
        self.capacity - self.used
    }

    fn largest_free_run(self) -> u16 {
        let mut largest = 0;
        let mut current = 0;
        let mut slot = 0;
        while slot < self.capacity as usize {
            let word = slot / u64::BITS as usize;
            let bit = slot % u64::BITS as usize;
            if self.free[word] & (1u64 << bit) != 0 {
                current += 1;
                largest = largest.max(current);
            } else {
                current = 0;
            }
            slot += 1;
        }
        largest
    }
}

#[derive(Clone, Copy)]
struct PoolSet<const CPUS: usize, const NODES: usize> {
    pools: [[Pool; CPUS]; NODES],
}

impl<const CPUS: usize, const NODES: usize> PoolSet<CPUS, NODES> {
    fn new(capacity: usize) -> Self {
        let pool = Pool::new(capacity);
        Self {
            pools: [[pool; CPUS]; NODES],
        }
    }
}

/// Per-CPU, NUMA-aware object pools with bounded remote fallback.
///
/// `CPUS` and `NODES` are compile-time bounds. Every CPU receives a pool on
/// its assigned node for each hot object kind. Allocation is deterministic:
/// current CPU, same-node CPUs in round-robin order, then at most
/// `max_cross_node_fallbacks` remote nodes.
pub struct HotObjectAllocator<const CPUS: usize, const NODES: usize> {
    cpu_to_node: [u8; CPUS],
    node_count: u8,
    max_cross_node_fallbacks: u8,
    pools: [PoolSet<CPUS, NODES>; KIND_COUNT],
    stats: [HotAllocatorStats; KIND_COUNT],
}

impl<const CPUS: usize, const NODES: usize> HotObjectAllocator<CPUS, NODES> {
    pub fn new(
        cpu_to_node: [u8; CPUS],
        node_count: usize,
        per_cpu_capacity: usize,
        max_cross_node_fallbacks: usize,
    ) -> Result<Self, HotAllocatorConfigError> {
        if CPUS == 0 {
            return Err(HotAllocatorConfigError::NoCpus)
        }
        if node_count == 0 || NODES == 0 {
            return Err(HotAllocatorConfigError::NoNodes)
        }
        if node_count > NODES {
            return Err(HotAllocatorConfigError::InvalidNode)
        }
        if per_cpu_capacity == 0 || per_cpu_capacity > MAX_POOL_SLOTS {
            return Err(HotAllocatorConfigError::InvalidCapacity)
        }
        if max_cross_node_fallbacks > node_count - 1 {
            return Err(HotAllocatorConfigError::InvalidFallbackLimit)
        }
        if cpu_to_node
            .iter()
            .any(|node| *node as usize >= node_count)
        {
            return Err(HotAllocatorConfigError::InvalidNode)
        }

        let set = PoolSet::new(per_cpu_capacity);
        Ok(Self {
            cpu_to_node,
            node_count: node_count as u8,
            max_cross_node_fallbacks: max_cross_node_fallbacks as u8,
            pools: [set; KIND_COUNT],
            stats: [HotAllocatorStats::new(); KIND_COUNT],
        })
    }

    pub fn allocate(
        &mut self,
        kind: HotObjectKind,
        cpu: usize,
    ) -> Result<HotAllocation, HotAllocationError> {
        if cpu >= CPUS {
            return Err(HotAllocationError::InvalidCpu)
        }
        let node = self.cpu_to_node[cpu] as usize;
        let kind_index = kind.index();
        let mut probes = 1u16;

        if let Some(slot) = self.pools[kind_index].pools[node][cpu].take() {
            self.record_allocation(kind_index, HotAllocationPlacement::LocalCpu, probes);
            return Ok(HotAllocation {
                kind,
                node: node as u8,
                cpu: cpu as u16,
                slot,
                placement: HotAllocationPlacement::LocalCpu,
            })
        }

        let mut distance = 1;
        while distance < CPUS {
            let candidate = (cpu + distance) % CPUS;
            probes = probes.saturating_add(1);
            if self.cpu_to_node[candidate] as usize == node {
                if let Some(slot) = self.pools[kind_index].pools[node][candidate].take() {
                    self.record_allocation(
                        kind_index,
                        HotAllocationPlacement::LocalNode,
                        probes,
                    );
                    return Ok(HotAllocation {
                        kind,
                        node: node as u8,
                        cpu: candidate as u16,
                        slot,
                        placement: HotAllocationPlacement::LocalNode,
                    })
                }
            }
            distance += 1;
        }

        let mut fallback = 0;
        while fallback < self.max_cross_node_fallbacks as usize {
            let remote_node = (node + fallback + 1) % self.node_count as usize;
            let mut remote_cpu = 0;
            while remote_cpu < CPUS {
                probes = probes.saturating_add(1);
                if self.cpu_to_node[remote_cpu] as usize == remote_node {
                    if let Some(slot) =
                        self.pools[kind_index].pools[remote_node][remote_cpu].take()
                    {
                        self.record_allocation(
                            kind_index,
                            HotAllocationPlacement::RemoteNode,
                            probes,
                        );
                        return Ok(HotAllocation {
                            kind,
                            node: remote_node as u8,
                            cpu: remote_cpu as u16,
                            slot,
                            placement: HotAllocationPlacement::RemoteNode,
                        })
                    }
                }
                remote_cpu += 1;
            }
            fallback += 1;
        }

        self.stats[kind_index].allocation_failures =
            self.stats[kind_index].allocation_failures.saturating_add(1);
        Err(HotAllocationError::Exhausted)
    }

    pub fn reclaim(&mut self, allocation: HotAllocation) -> Result<(), HotReclaimError> {
        self.reclaim_on(allocation.cpu as usize, allocation)
    }

    pub fn reclaim_on(
        &mut self,
        current_cpu: usize,
        allocation: HotAllocation,
    ) -> Result<(), HotReclaimError> {
        if current_cpu >= CPUS {
            return Err(HotReclaimError::InvalidCpu)
        }
        let owner_cpu = allocation.cpu as usize;
        let owner_node = allocation.node as usize;
        if owner_cpu >= CPUS {
            return self.record_reclaim_error(allocation.kind, HotReclaimError::InvalidCpu)
        }
        if owner_node >= self.node_count as usize
            || self.cpu_to_node[owner_cpu] as usize != owner_node
        {
            return self.record_reclaim_error(allocation.kind, HotReclaimError::InvalidNode)
        }
        let result = self.pools[allocation.kind.index()].pools[owner_node][owner_cpu]
            .release(allocation.slot);
        if let Err(error) = result {
            return self.record_reclaim_error(allocation.kind, error)
        }
        let stats = &mut self.stats[allocation.kind.index()];
        stats.reclaims = stats.reclaims.saturating_add(1);
        if current_cpu != owner_cpu {
            stats.remote_reclaims = stats.remote_reclaims.saturating_add(1);
        }
        Ok(())
    }

    /// Account bytes touched through a remote-node allocation. Object pools
    /// do not know the payload size, so callers report the exact transfer or
    /// object size at the ownership boundary.
    pub fn record_remote_memory(&mut self, kind: HotObjectKind, bytes: u64) {
        if bytes == 0 {
            return
        }
        self.stats[kind.index()].remote_memory_bytes = self.stats[kind.index()]
            .remote_memory_bytes
            .saturating_add(bytes);
        synos_observability::trace!(
            EventKind::RemoteMemory,
            EventField::unsigned(field::NUMA_KIND, 2),
            EventField::unsigned(field::REMOTE_MEMORY_BYTES, bytes),
        );
    }

    pub fn report(&self, kind: HotObjectKind) -> HotAllocatorReport {
        let mut capacity = 0u32;
        let mut in_use = 0u32;
        let mut free = 0u32;
        let mut largest_free_run = 0u32;
        let pools = &self.pools[kind.index()].pools;
        let mut node = 0;
        while node < NODES {
            let mut cpu = 0;
            while cpu < CPUS {
                if self.cpu_to_node[cpu] as usize == node {
                    let pool = pools[node][cpu];
                    capacity += pool.capacity as u32;
                    in_use += pool.used as u32;
                    free += pool.free_count() as u32;
                    largest_free_run += pool.largest_free_run() as u32;
                }
                cpu += 1;
            }
            node += 1;
        }
        let fragmented = free.saturating_sub(largest_free_run);
        let fragmentation_per_mille = if free == 0 {
            0
        } else {
            ((fragmented as u64 * 1_000) / free as u64) as u16
        };
        HotAllocatorReport {
            kind,
            capacity,
            in_use,
            free,
            largest_free_run,
            fragmentation_per_mille,
            stats: self.stats[kind.index()],
        }
    }

    pub const fn cpu_node(&self, cpu: usize) -> Option<u8> {
        if cpu < CPUS {
            Some(self.cpu_to_node[cpu])
        } else {
            None
        }
    }

    pub const fn node_count(&self) -> usize {
        self.node_count as usize
    }

    pub const fn max_cross_node_fallbacks(&self) -> usize {
        self.max_cross_node_fallbacks as usize
    }

    fn record_allocation(
        &mut self,
        kind: usize,
        placement: HotAllocationPlacement,
        probes: u16,
    ) {
        let stats = &mut self.stats[kind];
        stats.allocations = stats.allocations.saturating_add(1);
        stats.total_probe_steps = stats.total_probe_steps.saturating_add(probes as u64);
        stats.max_probe_steps = stats.max_probe_steps.max(probes);
        let bucket = probes.saturating_sub(1).min((PROBE_BUCKETS - 1) as u16) as usize;
        stats.probe_buckets[bucket] = stats.probe_buckets[bucket].saturating_add(1);
        match placement {
            HotAllocationPlacement::LocalCpu => {
                stats.local_cpu_allocations = stats.local_cpu_allocations.saturating_add(1)
            }
            HotAllocationPlacement::LocalNode => {
                stats.local_node_allocations = stats.local_node_allocations.saturating_add(1)
            }
            HotAllocationPlacement::RemoteNode => {
                stats.remote_node_allocations = stats.remote_node_allocations.saturating_add(1)
            }
        }
    }

    fn record_reclaim_error(
        &mut self,
        kind: HotObjectKind,
        error: HotReclaimError,
    ) -> Result<(), HotReclaimError> {
        self.stats[kind.index()].invalid_reclaims =
            self.stats[kind.index()].invalid_reclaims.saturating_add(1);
        Err(error)
    }
}
