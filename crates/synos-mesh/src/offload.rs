use synos_fabric::NodeId;
use synos_status::{IntoStatus, Status};

use crate::discovery::{NodeAdvertisement, NodeRole};

pub const DEFAULT_OFFLOAD_CAPACITY: usize = 32;
pub const DEFAULT_WORKLOAD_CHUNK_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkloadClass {
    Interactive,
    Heavy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkloadSpec {
    pub id: u64,
    pub class: WorkloadClass,
    pub cpu_millis: u32,
    pub memory_bytes: u64,
    pub input_bytes: u64,
    pub deadline_us: u64,
}

impl WorkloadSpec {
    pub const fn valid(self) -> bool {
        self.id != 0 && self.cpu_millis != 0 && self.memory_bytes != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComputeTarget {
    pub node: NodeId,
    pub role: NodeRole,
    pub cpu_capacity_millis: u32,
    pub free_cpu_millis: u32,
    pub memory_bytes: u64,
    pub free_memory_bytes: u64,
    pub bandwidth_mbps: u32,
    pub latency_us: u32,
    pub failed: bool,
}

impl ComputeTarget {
    pub const fn from_advertisement(
        advertisement: NodeAdvertisement,
        free_cpu_millis: u32,
        free_memory_bytes: u64,
    ) -> Self {
        Self {
            node: advertisement.node,
            role: advertisement.role,
            cpu_capacity_millis: advertisement.cpu_capacity_millis,
            free_cpu_millis,
            memory_bytes: advertisement.memory_bytes,
            free_memory_bytes,
            bandwidth_mbps: advertisement.bandwidth_mbps,
            latency_us: advertisement.latency_us,
            failed: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OffloadPlan {
    pub workload_id: u64,
    pub target: NodeId,
    pub remote: bool,
    pub estimated_latency_us: u64,
    pub score: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OffloadError {
    Capacity,
    InvalidRequest,
    NoEligibleTarget,
    SessionClosed,
    SequenceOverflow,
    ChunkTooLarge { required: usize },
    TransportRejected,
}

impl IntoStatus for OffloadError {
    fn status(self) -> Status {
        match self {
            Self::Capacity | Self::ChunkTooLarge { .. } => Status::NO_SPACE,
            Self::NoEligibleTarget => Status::NOT_FOUND,
            Self::SessionClosed | Self::SequenceOverflow | Self::TransportRejected => Status::BUSY,
            Self::InvalidRequest => Status::INVALID_ARGUMENT,
        }
    }
}

pub struct OffloadPlanner<const CAPACITY: usize = DEFAULT_OFFLOAD_CAPACITY> {
    local: NodeId,
    targets: [Option<ComputeTarget>; CAPACITY],
}

impl<const CAPACITY: usize> OffloadPlanner<CAPACITY> {
    pub const fn new(local: NodeId) -> Self {
        Self {
            local,
            targets: [None; CAPACITY],
        }
    }

    pub fn update(&mut self, target: ComputeTarget) -> Result<(), OffloadError> {
        if target.node == self.local || target.cpu_capacity_millis == 0 {
            return Err(OffloadError::InvalidRequest);
        }
        if let Some(existing) = self.targets.iter_mut().flatten().find(|entry| entry.node == target.node) {
            *existing = target;
            return Ok(())
        }
        let slot = self
            .targets
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(OffloadError::Capacity)?;
        *slot = Some(target);
        Ok(())
    }

    pub fn remove(&mut self, node: NodeId) -> Result<(), OffloadError> {
        let target = self
            .targets
            .iter_mut()
            .find(|entry| entry.is_some_and(|entry| entry.node == node))
            .ok_or(OffloadError::NoEligibleTarget)?;
        *target = None;
        Ok(())
    }

    pub fn mark_failed(&mut self, node: NodeId, failed: bool) -> Result<(), OffloadError> {
        let target = self
            .targets
            .iter_mut()
            .flatten()
            .find(|entry| entry.node == node)
            .ok_or(OffloadError::NoEligibleTarget)?;
        target.failed = failed;
        Ok(())
    }

    pub fn plan(&self, workload: WorkloadSpec) -> Result<OffloadPlan, OffloadError> {
        if !workload.valid() {
            return Err(OffloadError::InvalidRequest);
        }
        let target = self
            .targets
            .iter()
            .flatten()
            .filter(|target| {
                !target.failed
                    && target.free_cpu_millis >= workload.cpu_millis
                    && target.free_memory_bytes >= workload.memory_bytes
                    && (workload.class != WorkloadClass::Heavy || target.role == NodeRole::Cluster)
            })
            .min_by_key(|target| {
                let cpu_load = (workload.cpu_millis as u64)
                    .saturating_mul(1_000_000)
                    .checked_div(target.free_cpu_millis as u64)
                    .unwrap_or(u64::MAX);
                let bandwidth_penalty = if target.bandwidth_mbps == 0 {
                    u64::MAX / 2
                } else {
                    workload.input_bytes
                        .saturating_mul(8_000)
                        .checked_div(target.bandwidth_mbps as u64)
                        .unwrap_or(u64::MAX)
                };
                (cpu_load.saturating_add(bandwidth_penalty), target.latency_us, target.node.raw())
            })
            .ok_or(OffloadError::NoEligibleTarget)?;
        let transfer_us = if target.bandwidth_mbps == 0 {
            u64::MAX / 2
        } else {
            workload
                .input_bytes
                .saturating_mul(8_000)
                .checked_div(target.bandwidth_mbps as u64)
                .unwrap_or(u64::MAX)
        };
        let estimated_latency_us = transfer_us.saturating_add(target.latency_us as u64);
        if workload.deadline_us != 0 && estimated_latency_us > workload.deadline_us {
            return Err(OffloadError::NoEligibleTarget);
        }
        Ok(OffloadPlan {
            workload_id: workload.id,
            target: target.node,
            remote: true,
            estimated_latency_us,
            score: estimated_latency_us,
        })
    }

    pub fn targets(&self) -> impl Iterator<Item = ComputeTarget> + '_ {
        self.targets.iter().flatten().copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkloadChunk<const MAX_BYTES: usize = DEFAULT_WORKLOAD_CHUNK_BYTES> {
    pub workload_id: u64,
    pub sequence: u32,
    pub final_chunk: bool,
    pub len: u32,
    pub bytes: [u8; MAX_BYTES],
}

pub trait WorkloadTransport<const MAX_BYTES: usize> {
    fn send_chunk(
        &mut self,
        target: NodeId,
        chunk: WorkloadChunk<MAX_BYTES>,
    ) -> Result<(), OffloadError>;
}

pub struct OffloadSession<const MAX_BYTES: usize = DEFAULT_WORKLOAD_CHUNK_BYTES> {
    plan: OffloadPlan,
    next_sequence: u32,
    closed: bool,
}

impl<const MAX_BYTES: usize> OffloadSession<MAX_BYTES> {
    pub const fn open(plan: OffloadPlan) -> Self {
        Self {
            plan,
            next_sequence: 0,
            closed: false,
        }
    }

    pub const fn plan(&self) -> OffloadPlan {
        self.plan
    }

    pub const fn is_closed(&self) -> bool {
        self.closed
    }

    pub fn send<T: WorkloadTransport<MAX_BYTES>>(
        &mut self,
        transport: &mut T,
        bytes: &[u8],
        final_chunk: bool,
    ) -> Result<(), OffloadError> {
        if self.closed {
            return Err(OffloadError::SessionClosed);
        }
        let chunk = self.make_chunk(bytes, final_chunk)?;
        transport.send_chunk(self.plan.target, chunk)?;
        if final_chunk {
            self.closed = true;
        }
        Ok(())
    }

    pub fn chunk(
        &mut self,
        bytes: &[u8],
        final_chunk: bool,
    ) -> Result<WorkloadChunk<MAX_BYTES>, OffloadError> {
        if self.closed {
            return Err(OffloadError::SessionClosed);
        }
        let chunk = self.make_chunk(bytes, final_chunk)?;
        if final_chunk {
            self.closed = true;
        }
        Ok(chunk)
    }

    fn make_chunk(
        &mut self,
        bytes: &[u8],
        final_chunk: bool,
    ) -> Result<WorkloadChunk<MAX_BYTES>, OffloadError> {
        if bytes.len() > MAX_BYTES {
            return Err(OffloadError::ChunkTooLarge { required: bytes.len() });
        }
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(OffloadError::SequenceOverflow)?;
        let mut chunk = WorkloadChunk {
            workload_id: self.plan.workload_id,
            sequence,
            final_chunk,
            len: bytes.len() as u32,
            bytes: [0; MAX_BYTES],
        };
        chunk.bytes[..bytes.len()].copy_from_slice(bytes);
        Ok(chunk)
    }
}
