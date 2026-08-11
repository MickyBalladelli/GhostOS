#![no_std]
#![forbid(unsafe_code)]

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

impl PlacementKind {
    const fn index(self) -> usize {
        self as usize - 1
    }
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
        if cpu_to_node.is_empty() {
            return Err(NumaTopologyError::NoCpus)
        }
        if cpu_to_node.len() > MAX_NUMA_CPUS {
            return Err(NumaTopologyError::TooManyCpus)
        }
        let mut mapping = [0; MAX_NUMA_CPUS];
        let mut node_mask = 0u64;
        for (index, node) in cpu_to_node.iter().copied().enumerate() {
            if node as usize >= MAX_NUMA_NODES {
                return Err(NumaTopologyError::InvalidNode)
            }
            mapping[index] = node;
            node_mask |= 1u64 << node;
        }
        let node_count = node_mask.count_ones() as usize;
        if node_count > MAX_NUMA_NODES {
            return Err(NumaTopologyError::TooManyNodes)
        }
        Ok(Self {
            cpu_to_node: mapping,
            cpu_count: cpu_to_node.len() as u16,
            node_count: node_count as u8,
            node_mask,
        })
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
pub struct NumaReport {
    pub topology: NumaTopology,
    pub counters: NumaCounters,
}

/// Bounded placement state shared by process, memory, queue, storage, and
/// interrupt owners. Missing topology is deliberately represented as UMA.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
        self.topology = topology;
        self.cursors = [0; NUMA_KIND_COUNT];
    }

    pub fn place(
        &mut self,
        kind: PlacementKind,
        preferred_cpu: Option<u16>,
        preferred_node: Option<u8>,
    ) -> NumaDecision {
        let requested_node = preferred_node
            .or_else(|| preferred_cpu.and_then(|cpu| self.topology.node_for_cpu(cpu as usize)))
            .unwrap_or(0);
        let selected_node = if self.topology.has_node(requested_node) {
            requested_node
        } else {
            self.next_node(kind)
        };
        let cpu = preferred_cpu
            .filter(|cpu| self.topology.node_for_cpu(*cpu as usize) == Some(selected_node))
            .or_else(|| self.topology.first_cpu_on_node(selected_node))
            .unwrap_or(0);
        let locality = if self.topology.is_uma() {
            PlacementLocality::UmaFallback
        } else if requested_node != selected_node {
            PlacementLocality::RemoteNode
        } else if preferred_cpu == Some(cpu) {
            PlacementLocality::LocalCpu
        } else {
            PlacementLocality::LocalNode
        };
        self.counters.decisions = self.counters.decisions.saturating_add(1);
        match locality {
            PlacementLocality::LocalCpu => self.counters.local_cpu = self.counters.local_cpu.saturating_add(1),
            PlacementLocality::LocalNode => self.counters.local_node = self.counters.local_node.saturating_add(1),
            PlacementLocality::RemoteNode => self.counters.remote_node = self.counters.remote_node.saturating_add(1),
            PlacementLocality::UmaFallback => self.counters.uma_fallback = self.counters.uma_fallback.saturating_add(1),
        }
        NumaDecision {
            kind,
            requested_node,
            selected_node,
            cpu,
            locality,
            sequence: self.counters.decisions,
        }
    }

    pub fn record_memory_access(&mut self, local_node: u8, allocation_node: u8, bytes: u64) {
        if !self.topology.is_uma() && local_node != allocation_node {
            self.counters.remote_memory_accesses =
                self.counters.remote_memory_accesses.saturating_add(1);
            self.counters.remote_memory_bytes =
                self.counters.remote_memory_bytes.saturating_add(bytes);
        }
    }

    fn next_node(&mut self, kind: PlacementKind) -> u8 {
        let count = self.topology.node_count().max(1);
        let cursor = &mut self.cursors[kind.index()];
        let node = self
            .topology
            .node_at(*cursor as usize % count)
            .unwrap_or(0);
        *cursor = cursor.wrapping_add(1);
        node
    }
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
