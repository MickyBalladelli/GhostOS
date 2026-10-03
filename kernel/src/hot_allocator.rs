//! Bounded pools for objects that are created and reclaimed on hot paths.
//!
//! The allocator never touches the global heap. Each NUMA node has one pool
//! per CPU and per object kind. Allocation checks the current CPU, then other
//! CPUs on the same node, and finally a configured number of remote nodes.
//! This makes the remote-memory cost explicit and measurable.

use ghostos_observability::{field, EventField, EventKind};

const BITMAP_WORDS: usize = 4;
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
#[repr(C)]
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

#[repr(C)]
#[derive(Clone, Copy)]
struct Pool {
    capacity: u16,
    used: u16,
    free: [u64; BITMAP_WORDS],
}

impl Pool {
    const EMPTY: Self = Self { capacity: 0, used: 0, free: [0; BITMAP_WORDS] };
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PoolSet<const CPUS: usize, const NODES: usize> {
    pools: [[Pool; CPUS]; NODES],
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
        match unsafe {
            ghostos_hot_validate(cpu_to_node.as_ptr(), CPUS, NODES, node_count,
                per_cpu_capacity, max_cross_node_fallbacks)
        } {
            0 => {},
            1 => return Err(HotAllocatorConfigError::NoCpus),
            2 => return Err(HotAllocatorConfigError::NoNodes),
            3 => return Err(HotAllocatorConfigError::InvalidNode),
            4 => return Err(HotAllocatorConfigError::InvalidCapacity),
            5 => return Err(HotAllocatorConfigError::InvalidFallbackLimit),
            _ => unreachable!("invalid native hot allocator configuration result"),
        }
        let mut pool = Pool::EMPTY;
        unsafe { ghostos_hot_pool_init(&mut pool, per_cpu_capacity) };
        let set = PoolSet { pools: [[pool; CPUS]; NODES] };
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
        let mut allocation = NativeAllocation::EMPTY;
        let mut view = self.native_view();
        match unsafe { ghostos_hot_view_allocate(&mut view, kind.index() as u32, cpu, &mut allocation) } {
            0 => Ok(allocation.to_public(kind)),
            1 => Err(HotAllocationError::InvalidCpu),
            2 => Err(HotAllocationError::Exhausted),
            3 => panic!("attempt to calculate the remainder with a divisor of zero"),
            _ => unreachable!("invalid native hot allocation result"),
        }
    }

    pub fn reclaim(&mut self, allocation: HotAllocation) -> Result<(), HotReclaimError> {
        self.reclaim_on(allocation.cpu as usize, allocation)
    }

    pub fn reclaim_on(
        &mut self,
        current_cpu: usize,
        allocation: HotAllocation,
    ) -> Result<(), HotReclaimError> {
        let mut view = self.native_view();
        match unsafe {
            ghostos_hot_view_reclaim_on(&mut view, current_cpu, NativeAllocation::from(allocation))
        } {
            0 => Ok(()),
            1 => Err(HotReclaimError::InvalidCpu),
            2 => Err(HotReclaimError::InvalidNode),
            3 => Err(HotReclaimError::InvalidSlot),
            4 => Err(HotReclaimError::AlreadyFree),
            _ => unreachable!("invalid native hot reclaim result"),
        }
    }

    /// Account bytes touched through a remote-node allocation. Object pools
    /// do not know the payload size, so callers report the exact transfer or
    /// object size at the ownership boundary.
    pub fn record_remote_memory(&mut self, kind: HotObjectKind, bytes: u64) {
        if bytes == 0 {
            return
        }
        let mut view = self.native_view();
        unsafe { ghostos_hot_view_record_remote_memory(&mut view, kind.index() as u32, bytes) };
        ghostos_observability::trace!(
            EventKind::RemoteMemory,
            EventField::unsigned(field::NUMA_KIND, 2),
            EventField::unsigned(field::REMOTE_MEMORY_BYTES, bytes),
        );
    }

    pub fn report(&self, kind: HotObjectKind) -> HotAllocatorReport {
        let view = NativeView {
            cpu_count: CPUS, node_capacity: NODES, cpu_stride: CPUS, node_stride: NODES,
            node_count: self.node_count, max_cross_node_fallbacks: self.max_cross_node_fallbacks,
            cpu_to_node: self.cpu_to_node.as_ptr(),
            pools: self.pools.as_ptr().cast::<Pool>().cast_mut(),
            stats: self.stats.as_ptr().cast_mut(), checked: cfg!(debug_assertions),
        };
        let mut report = NativeReport::EMPTY;
        if !unsafe { ghostos_hot_view_report(&view, kind.index() as u32, &mut report) } {
            panic!("attempt to add with overflow")
        }
        HotAllocatorReport {
            kind, capacity: report.capacity, in_use: report.in_use, free: report.free,
            largest_free_run: report.largest_free_run,
            fragmentation_per_mille: report.fragmentation_per_mille, stats: report.stats,
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

    fn native_view(&mut self) -> NativeView {
        NativeView {
            cpu_count: CPUS, node_capacity: NODES, cpu_stride: CPUS, node_stride: NODES,
            node_count: self.node_count, max_cross_node_fallbacks: self.max_cross_node_fallbacks,
            cpu_to_node: self.cpu_to_node.as_ptr(),
            pools: self.pools.as_mut_ptr().cast(), stats: self.stats.as_mut_ptr(),
            checked: cfg!(debug_assertions),
        }
    }
}

#[repr(C)]
struct NativeView {
    cpu_count: usize,
    node_capacity: usize,
    cpu_stride: usize,
    node_stride: usize,
    node_count: u8,
    max_cross_node_fallbacks: u8,
    cpu_to_node: *const u8,
    pools: *mut Pool,
    stats: *mut HotAllocatorStats,
    checked: bool,
}

#[repr(C)]
struct NativeAllocation {
    kind: u32,
    node: u8,
    cpu: u16,
    slot: u16,
    placement: u32,
}

impl NativeAllocation {
    const EMPTY: Self = Self { kind: 0, node: 0, cpu: 0, slot: 0, placement: 0 };

    fn to_public(self, kind: HotObjectKind) -> HotAllocation {
        HotAllocation {
            kind, node: self.node, cpu: self.cpu, slot: self.slot,
            placement: match self.placement {
                0 => HotAllocationPlacement::LocalCpu,
                1 => HotAllocationPlacement::LocalNode,
                2 => HotAllocationPlacement::RemoteNode,
                _ => unreachable!("invalid native hot allocation placement"),
            },
        }
    }
}

impl From<HotAllocation> for NativeAllocation {
    fn from(allocation: HotAllocation) -> Self {
        Self {
            kind: allocation.kind.index() as u32, node: allocation.node,
            cpu: allocation.cpu, slot: allocation.slot,
            placement: match allocation.placement {
                HotAllocationPlacement::LocalCpu => 0,
                HotAllocationPlacement::LocalNode => 1,
                HotAllocationPlacement::RemoteNode => 2,
            },
        }
    }
}

#[repr(C)]
struct NativeReport {
    kind: u32,
    capacity: u32,
    in_use: u32,
    free: u32,
    largest_free_run: u32,
    fragmentation_per_mille: u16,
    stats: HotAllocatorStats,
}

impl NativeReport {
    const EMPTY: Self = Self {
        kind: 0, capacity: 0, in_use: 0, free: 0, largest_free_run: 0,
        fragmentation_per_mille: 0, stats: HotAllocatorStats::new(),
    };
}

const _: () = {
    assert!(core::mem::size_of::<Pool>() == 40);
    assert!(core::mem::offset_of!(Pool, free) == 8);
    assert!(core::mem::size_of::<HotAllocatorStats>() == 152);
    assert!(core::mem::offset_of!(HotAllocatorStats, probe_buckets) == 64);
    assert!(core::mem::size_of::<NativeAllocation>() == 16);
    assert!(core::mem::size_of::<NativeReport>() == 176);
    assert!(core::mem::size_of::<NativeView>() == 72);
};

unsafe extern "C" {
    fn ghostos_hot_pool_init(pool: *mut Pool, capacity: usize);
    fn ghostos_hot_validate(map: *const u8, cpus: usize, nodes: usize,
        node_count: usize, capacity: usize, fallbacks: usize) -> u32;
    fn ghostos_hot_view_allocate(view: *mut NativeView, kind: u32,
        cpu: usize, allocation: *mut NativeAllocation) -> u32;
    fn ghostos_hot_view_reclaim_on(view: *mut NativeView, cpu: usize,
        allocation: NativeAllocation) -> u32;
    fn ghostos_hot_view_record_remote_memory(view: *mut NativeView, kind: u32, bytes: u64);
    fn ghostos_hot_view_report(view: *const NativeView, kind: u32, report: *mut NativeReport) -> bool;
}
