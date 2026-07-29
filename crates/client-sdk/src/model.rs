use synos_auth::TransportRights;
use synos_fabric::NodeId;
use synos_kernel::Rights;

use crate::ProtocolError;

pub const MAX_CLUSTER_NODES: usize = 64;
pub const MAX_JOB_COMMAND_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NodeHealth {
    Healthy = 1,
    Degraded = 2,
    Failed = 3,
}

impl NodeHealth {
    pub(crate) const fn from_wire(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Healthy),
            2 => Ok(Self::Degraded),
            3 => Ok(Self::Failed),
            _ => Err(ProtocolError::InvalidValue),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterNode {
    pub node: NodeId,
    pub health: NodeHealth,
    pub cpu_load_permille: u16,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub running_jobs: u32,
    pub queued_jobs: u32,
}

impl ClusterNode {
    pub const fn validate(self) -> Result<Self, ProtocolError> {
        if self.cpu_load_permille > 1_000 || self.memory_used_bytes > self.memory_total_bytes {
            Err(ProtocolError::InvalidValue)
        } else {
            Ok(self)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterState {
    pub generation: u64,
    pub sampled_at_us: u64,
    nodes: [Option<ClusterNode>; MAX_CLUSTER_NODES],
}

impl ClusterState {
    pub const fn new(generation: u64, sampled_at_us: u64) -> Self {
        Self {
            generation,
            sampled_at_us,
            nodes: [None; MAX_CLUSTER_NODES],
        }
    }

    pub fn push(&mut self, node: ClusterNode) -> Result<(), ProtocolError> {
        let node = node.validate()?;
        if self.nodes().any(|candidate| candidate.node == node.node) {
            return Err(ProtocolError::DuplicateNode);
        }
        let slot = self
            .nodes
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(ProtocolError::Capacity)?;
        *slot = Some(node);
        Ok(())
    }

    pub fn nodes(&self) -> impl Iterator<Item = ClusterNode> + '_ {
        self.nodes.iter().flatten().copied()
    }

    pub fn node_count(&self) -> usize {
        self.nodes().count()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobSpec<'a> {
    pub command: &'a str,
    pub priority: u8,
    pub timeout_us: u64,
}

impl<'a> JobSpec<'a> {
    pub const fn new(
        command: &'a str,
        priority: u8,
        timeout_us: u64,
    ) -> Result<Self, ProtocolError> {
        if command.is_empty()
            || command.len() > MAX_JOB_COMMAND_BYTES
            || priority == 0
            || timeout_us == 0
        {
            Err(ProtocolError::InvalidValue)
        } else {
            Ok(Self {
                command,
                priority,
                timeout_us,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobReceipt {
    pub job_id: u64,
    pub accepted_at_us: u64,
}

impl JobReceipt {
    pub const fn validate(self) -> Result<Self, ProtocolError> {
        if self.job_id == 0 {
            Err(ProtocolError::InvalidValue)
        } else {
            Ok(self)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityDelegation {
    pub resource: u64,
    pub subject: NodeId,
    pub rights: Rights,
    pub transports: TransportRights,
    pub valid_for_us: u64,
}

impl CapabilityDelegation {
    pub const fn new(
        resource: u64,
        subject: NodeId,
        rights: Rights,
        transports: TransportRights,
        valid_for_us: u64,
    ) -> Result<Self, ProtocolError> {
        if resource == 0 || rights.is_empty() || valid_for_us == 0 {
            Err(ProtocolError::InvalidValue)
        } else {
            Ok(Self {
                resource,
                subject,
                rights,
                transports,
                valid_for_us,
            })
        }
    }
}
