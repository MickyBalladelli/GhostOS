use ghostos_fabric::NodeId;

pub const DEFAULT_NODE_CAPACITY: usize = 16;
pub const DEFAULT_CAPABILITY_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeHealth {
    Healthy,
    Degraded,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeSample {
    pub node: NodeId,
    pub health: NodeHealth,
    pub ram_used_bytes: u64,
    pub ram_total_bytes: u64,
    pub vram_used_bytes: u64,
    pub vram_total_bytes: u64,
    pub remote_faults: u64,
    pub dsm_latency_p50_ns: u32,
    pub dsm_latency_p99_ns: u32,
}

impl NodeSample {
    pub fn validate(self) -> Result<Self, SnapshotError> {
        if self.ram_used_bytes > self.ram_total_bytes
            || self.vram_used_bytes > self.vram_total_bytes
            || self.dsm_latency_p50_ns > self.dsm_latency_p99_ns
        {
            return Err(SnapshotError::InvalidSample)
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityKind {
    Memory,
    AddressSpace,
    Ipc,
    Lock,
    Namespace,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilitySample {
    pub handle: u64,
    pub parent: Option<u64>,
    pub owner: u32,
    pub rights: u16,
    pub kind: CapabilityKind,
}

impl CapabilitySample {
    pub fn validate(self) -> Result<Self, SnapshotError> {
        if self.handle == 0 || self.parent == Some(self.handle) {
            return Err(SnapshotError::InvalidSample)
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    Capacity,
    Cycle,
    Duplicate,
    InvalidSample,
    MissingParent,
}

/// One immutable generation of topology telemetry.
///
/// Writers build a complete generation off-screen and publish it with
/// [`TopologySnapshot::finish`]. Renderers never observe a partially updated
/// frame.
#[derive(Clone, Copy)]
pub struct TopologySnapshot<
    const NODES: usize = DEFAULT_NODE_CAPACITY,
    const CAPABILITIES: usize = DEFAULT_CAPABILITY_CAPACITY,
> {
    generation: u64,
    sampled_at_us: u64,
    nodes: [Option<NodeSample>; NODES],
    capabilities: [Option<CapabilitySample>; CAPABILITIES],
}

impl<const NODES: usize, const CAPABILITIES: usize>
    TopologySnapshot<NODES, CAPABILITIES>
{
    pub const fn new() -> Self {
        Self {
            generation: 0,
            sampled_at_us: 0,
            nodes: [None; NODES],
            capabilities: [None; CAPABILITIES],
        }
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub const fn sampled_at_us(&self) -> u64 {
        self.sampled_at_us
    }

    pub fn nodes(&self) -> impl Iterator<Item = &NodeSample> {
        self.nodes.iter().flatten()
    }

    pub fn capabilities(&self) -> impl Iterator<Item = &CapabilitySample> {
        self.capabilities.iter().flatten()
    }

    pub fn begin(&mut self) {
        self.nodes.fill(None);
        self.capabilities.fill(None)
    }

    pub fn push_node(&mut self, sample: NodeSample) -> Result<(), SnapshotError> {
        let sample = sample.validate()?;
        if self.nodes().any(|existing| existing.node == sample.node) {
            return Err(SnapshotError::Duplicate)
        }
        let slot = self
            .nodes
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(SnapshotError::Capacity)?;
        *slot = Some(sample);
        Ok(())
    }

    pub fn push_capability(
        &mut self,
        sample: CapabilitySample,
    ) -> Result<(), SnapshotError> {
        let sample = sample.validate()?;
        if self
            .capabilities()
            .any(|existing| existing.handle == sample.handle)
        {
            return Err(SnapshotError::Duplicate)
        }
        let slot = self
            .capabilities
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(SnapshotError::Capacity)?;
        *slot = Some(sample);
        Ok(())
    }

    pub fn finish(&mut self, sampled_at_us: u64) -> Result<u64, SnapshotError> {
        for capability in self.capabilities() {
            if let Some(parent) = capability.parent
                && !self
                    .capabilities()
                    .any(|candidate| candidate.handle == parent)
            {
                return Err(SnapshotError::MissingParent)
            }
            let mut parent = capability.parent;
            let mut depth = 0;
            while let Some(handle) = parent {
                depth += 1;
                if depth >= CAPABILITIES {
                    return Err(SnapshotError::Cycle)
                }
                parent = self
                    .capabilities()
                    .find(|candidate| candidate.handle == handle)
                    .and_then(|candidate| candidate.parent)
            }
        }
        self.sampled_at_us = sampled_at_us;
        self.generation = self.generation.wrapping_add(1).max(1);
        Ok(self.generation)
    }

    pub fn clear(&mut self) {
        self.begin();
        self.generation = 0;
        self.sampled_at_us = 0
    }
}

impl<const NODES: usize, const CAPABILITIES: usize> Default
    for TopologySnapshot<NODES, CAPABILITIES>
{
    fn default() -> Self {
        Self::new()
    }
}
