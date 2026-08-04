use synos_auth::TransportRights;
use synos_fabric::NodeId;
use synos_kernel::Rights;

use crate::ProtocolError;

pub const MAX_CLUSTER_NODES: usize = 64;
pub const MAX_TOPOLOGY_LINKS: usize = 64;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TopologyTransport {
    Cxl = 1,
    Ethernet = 2,
    Wireless = 3,
    Cellular5g = 4,
    Loopback = 5,
    Tunnel = 6,
}

impl TopologyTransport {
    pub(crate) const fn from_wire(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Cxl),
            2 => Ok(Self::Ethernet),
            3 => Ok(Self::Wireless),
            4 => Ok(Self::Cellular5g),
            5 => Ok(Self::Loopback),
            6 => Ok(Self::Tunnel),
            _ => Err(ProtocolError::InvalidValue),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TopologyRoute {
    Direct = 1,
    Nat = 2,
    Relay = 3,
    Offline = 4,
}

impl TopologyRoute {
    pub(crate) const fn from_wire(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Direct),
            2 => Ok(Self::Nat),
            3 => Ok(Self::Relay),
            4 => Ok(Self::Offline),
            _ => Err(ProtocolError::InvalidValue),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum TopologyReachability {
    Unknown = 1,
    Reachable = 2,
    Unreachable = 3,
    Offline = 4,
}

impl TopologyReachability {
    pub(crate) const fn from_wire(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Unknown),
            2 => Ok(Self::Reachable),
            3 => Ok(Self::Unreachable),
            4 => Ok(Self::Offline),
            _ => Err(ProtocolError::InvalidValue),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TopologyLink {
    pub from: NodeId,
    pub to: NodeId,
    pub transport: TopologyTransport,
    pub route: TopologyRoute,
    pub reachability: TopologyReachability,
    pub latency_us: u64,
    pub bandwidth_mbps: u64,
    pub mtu: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TopologyState {
    pub generation: u64,
    pub sampled_at_us: u64,
    links: [Option<TopologyLink>; MAX_TOPOLOGY_LINKS],
}

impl TopologyState {
    pub const fn new(generation: u64, sampled_at_us: u64) -> Self {
        Self { generation, sampled_at_us, links: [None; MAX_TOPOLOGY_LINKS] }
    }

    pub fn push(&mut self, link: TopologyLink) -> Result<(), ProtocolError> {
        if link.from == link.to || link.latency_us == 0 || link.bandwidth_mbps == 0 || link.mtu < 576 {
            return Err(ProtocolError::InvalidValue)
        }
        if self.links().any(|existing| existing.from == link.from && existing.to == link.to && existing.transport == link.transport) {
            return Err(ProtocolError::DuplicateNode)
        }
        let slot = self.links.iter_mut().find(|entry| entry.is_none()).ok_or(ProtocolError::Capacity)?;
        *slot = Some(link);
        Ok(())
    }

    pub fn links(&self) -> impl Iterator<Item = TopologyLink> + '_ {
        self.links.iter().flatten().copied()
    }

    pub fn link_count(&self) -> usize {
        self.links().count()
    }
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
