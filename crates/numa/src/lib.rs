#![no_std]

pub const MAX_NUMA_NODES: usize = 64;
pub const MAX_NUMA_CPUS: usize = 128;
pub const NUMA_KIND_COUNT: usize = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NumaTopologyError {
    NoCpus,
    TooManyCpus,
    InvalidNode,
    TooManyNodes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PlacementKind {
    Process = 1,
    Memory = 2,
    Queue = 3,
    StorageWorker = 4,
    NetworkInterrupt = 5,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum PlacementLocality {
    LocalCpu = 1,
    LocalNode = 2,
    RemoteNode = 3,
    UmaFallback = 4,
}

impl PlacementLocality {
    pub const fn is_remote(self) -> bool {
        matches!(self, Self::RemoteNode)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NumaTopology {
    cpu_to_node: [u8; MAX_NUMA_CPUS],
    cpu_count: u16,
    node_count: u8,
    node_mask: u64,
}

impl NumaTopology {
    pub const fn uma() -> Self {
        Self {
            cpu_to_node: [0; MAX_NUMA_CPUS],
            cpu_count: 1,
            node_count: 1,
            node_mask: 1,
        }
    }

    pub fn new(cpu_to_node: &[u8]) -> Result<Self, NumaTopologyError> {
        let mut topology = Self::uma();
        // C borrows the mapping and initializes only this caller-owned topology.
        let code = unsafe {
            ghostos_numa_topology_init(&mut topology, cpu_to_node.as_ptr(), cpu_to_node.len())
        };
        match code {
            0 => Ok(topology),
            1 => Err(NumaTopologyError::NoCpus),
            2 => Err(NumaTopologyError::TooManyCpus),
            3 => Err(NumaTopologyError::InvalidNode),
            _ => Err(NumaTopologyError::TooManyNodes),
        }
    }

    pub const fn cpu_count(self) -> usize {
        self.cpu_count as usize
    }

    pub const fn node_count(self) -> usize {
        self.node_count as usize
    }

    pub const fn is_uma(self) -> bool {
        self.node_count <= 1
    }

    pub const fn node_for_cpu(self, cpu: usize) -> Option<u8> {
        if cpu < self.cpu_count as usize {
            Some(self.cpu_to_node[cpu])
        } else {
            None
        }
    }

    pub const fn has_node(self, node: u8) -> bool {
        node < MAX_NUMA_NODES as u8 && self.node_mask & (1u64 << node) != 0
    }

    pub const fn first_cpu_on_node(self, node: u8) -> Option<u16> {
        let mut cpu = 0;
        while cpu < self.cpu_count as usize {
            if self.cpu_to_node[cpu] == node {
                return Some(cpu as u16)
            }
            cpu += 1;
        }
        None
    }

    pub const fn node_at(self, index: usize) -> Option<u8> {
        if index >= self.node_count as usize {
            return None
        }
        let mut seen = 0;
        let mut node = 0;
        while node < MAX_NUMA_NODES {
            if self.has_node(node as u8) {
                if seen == index {
                    return Some(node as u8)
                }
                seen += 1;
            }
            node += 1;
        }
        None
    }
}

impl Default for NumaTopology {
    fn default() -> Self {
        Self::uma()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NumaDecision {
    pub kind: PlacementKind,
    pub requested_node: u8,
    pub selected_node: u8,
    pub cpu: u16,
    pub locality: PlacementLocality,
    pub sequence: u64,
}

impl NumaDecision {
    pub const fn uma(kind: PlacementKind) -> Self {
        Self {
            kind,
            requested_node: 0,
            selected_node: 0,
            cpu: 0,
            locality: PlacementLocality::UmaFallback,
            sequence: 0,
        }
    }

    pub const fn is_remote(self) -> bool {
        self.locality.is_remote()
    }
}

impl Default for NumaDecision {
    fn default() -> Self {
        Self::uma(PlacementKind::Memory)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
#[repr(C)]
pub struct NumaCounters {
    pub decisions: u64,
    pub local_cpu: u64,
    pub local_node: u64,
    pub remote_node: u64,
    pub uma_fallback: u64,
    pub remote_memory_accesses: u64,
    pub remote_memory_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NumaReport {
    pub topology: NumaTopology,
    pub counters: NumaCounters,
}

/// Bounded placement state shared by process, memory, queue, storage, and
/// interrupt owners. Missing topology is deliberately represented as UMA.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct NumaPlacement {
    topology: NumaTopology,
    cursors: [u16; NUMA_KIND_COUNT],
    counters: NumaCounters,
}

impl NumaPlacement {
    pub const fn new(topology: NumaTopology) -> Self {
        Self {
            topology,
            cursors: [0; NUMA_KIND_COUNT],
            counters: NumaCounters {
                decisions: 0,
                local_cpu: 0,
                local_node: 0,
                remote_node: 0,
                uma_fallback: 0,
                remote_memory_accesses: 0,
                remote_memory_bytes: 0,
            },
        }
    }

    pub const fn uma() -> Self {
        Self::new(NumaTopology::uma())
    }

    pub const fn topology(self) -> NumaTopology {
        self.topology
    }

    pub const fn counters(self) -> NumaCounters {
        self.counters
    }

    pub const fn report(self) -> NumaReport {
        NumaReport {
            topology: self.topology,
            counters: self.counters,
        }
    }

    pub fn set_topology(&mut self, topology: NumaTopology) {
        // C resets cursors while retaining this placement's counters.
        unsafe { ghostos_numa_set_topology(self, &topology) }
    }

    pub fn place(
        &mut self,
        kind: PlacementKind,
        preferred_cpu: Option<u16>,
        preferred_node: Option<u8>,
    ) -> NumaDecision {
        let mut decision = NumaDecision::uma(kind);
        // Both objects have checked C layouts; valid Rust enum values bound kind.
        unsafe {
            ghostos_numa_place(self, kind as u8, preferred_cpu.is_some(),
                preferred_cpu.unwrap_or(0), preferred_node.is_some(),
                preferred_node.unwrap_or(0), &mut decision)
        }
        decision
    }

    pub fn record_memory_access(&mut self, local_node: u8, allocation_node: u8, bytes: u64) {
        // C mutates only caller-owned counters and retains no pointers.
        unsafe { ghostos_numa_record_memory(self, local_node, allocation_node, bytes) }
    }
}

const _: () = {
    assert!(core::mem::size_of::<NumaTopology>() == 144);
    assert!(core::mem::offset_of!(NumaTopology, cpu_count) == 128);
    assert!(core::mem::offset_of!(NumaTopology, node_count) == 130);
    assert!(core::mem::offset_of!(NumaTopology, node_mask) == 136);
    assert!(core::mem::size_of::<NumaDecision>() == 16);
    assert!(core::mem::offset_of!(NumaDecision, cpu) == 4);
    assert!(core::mem::offset_of!(NumaDecision, locality) == 6);
    assert!(core::mem::offset_of!(NumaDecision, sequence) == 8);
    assert!(core::mem::size_of::<NumaCounters>() == 56);
    assert!(core::mem::size_of::<NumaReport>() == 200);
    assert!(core::mem::size_of::<NumaPlacement>() == 216);
    assert!(core::mem::offset_of!(NumaPlacement, cursors) == 144);
    assert!(core::mem::offset_of!(NumaPlacement, counters) == 160);
};

unsafe extern "C" {
    fn ghostos_numa_topology_init(topology: *mut NumaTopology,
        mapping: *const u8, count: usize) -> u32;
    fn ghostos_numa_set_topology(placement: *mut NumaPlacement, topology: *const NumaTopology);
    fn ghostos_numa_place(placement: *mut NumaPlacement, kind: u8,
        has_cpu: bool, preferred_cpu: u16, has_node: bool, preferred_node: u8,
        decision: *mut NumaDecision);
    fn ghostos_numa_record_memory(placement: *mut NumaPlacement,
        local_node: u8, allocation_node: u8, bytes: u64);
}

impl Default for NumaPlacement {
    fn default() -> Self {
        Self::uma()
    }
}

#[cfg(test)]
mod tests {
    use super::{NumaPlacement, NumaTopology, PlacementKind, PlacementLocality};

    #[test]
    fn invalid_target_is_bounded_and_reported_as_remote() {
        let topology = NumaTopology::new(&[0, 0, 1, 1]).unwrap();
        let mut placement = NumaPlacement::new(topology);
        let decision = placement.place(PlacementKind::Process, None, Some(9));
        assert_eq!(decision.selected_node, 0);
        assert_eq!(decision.locality, PlacementLocality::RemoteNode);
    }

    #[test]
    fn uma_hosts_use_safe_fallback_and_never_report_remote_bytes() {
        let topology = NumaTopology::new(&[1, 1]).unwrap();
        let mut placement = NumaPlacement::new(topology);
        let decision = placement.place(PlacementKind::Memory, Some(0), Some(8));
        placement.record_memory_access(1, 8, 4096);
        assert_eq!(decision.locality, PlacementLocality::UmaFallback);
        assert_eq!(placement.counters().remote_memory_bytes, 0);
    }
}
