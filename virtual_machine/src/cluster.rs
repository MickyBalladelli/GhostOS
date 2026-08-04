//! Deterministic cluster fixtures for VM and SynOS integration tests.
//!
//! The harness drives VM instances in bounded steps, models an Ethernet-like
//! transport with reproducible faults, and exposes CXL address-space actions
//! without requiring cluster hardware.

use crate::{Vm, VmConfig, VmError};
use std::collections::VecDeque;
use synos_fabric::memory::{GlobalAddressSpace, MemoryKind, MemoryPool, PoolId, Transport};
use synos_fabric::{AddressRange, Error as FabricError, NodeId as FabricNodeId, PAGE_SIZE};

const MAX_CLUSTER_NODES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ClusterNodeId(u32);

impl ClusterNodeId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 || raw as usize > MAX_CLUSTER_NODES {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterNodeState {
    Running,
    Isolated,
    Failed,
    Stopped,
}

pub struct ClusterNode {
    id: ClusterNodeId,
    vm: Vm,
    state: ClusterNodeState,
    executed_steps: u64,
}

impl ClusterNode {
    pub fn id(&self) -> ClusterNodeId {
        self.id
    }

    pub fn state(&self) -> ClusterNodeState {
        self.state
    }

    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    pub fn vm_mut(&mut self) -> &mut Vm {
        &mut self.vm
    }

    pub fn executed_steps(&self) -> u64 {
        self.executed_steps
    }

    pub fn serial_output(&self) -> Vec<u8> {
        self.vm
            .serial()
            .map(|serial| serial.borrow().output().to_vec())
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterNetworkConfig {
    pub latency_ticks: u64,
    pub loss_percent: u8,
    pub duplicate_percent: u8,
    pub reorder: bool,
    pub seed: u64,
}

impl Default for ClusterNetworkConfig {
    fn default() -> Self {
        Self {
            latency_ticks: 1,
            loss_percent: 0,
            duplicate_percent: 0,
            reorder: false,
            seed: 0x5359_4e4f_5343_4c55,
        }
    }
}

impl ClusterNetworkConfig {
    fn validate(self) -> Result<Self, ClusterError> {
        if self.loss_percent > 100 || self.duplicate_percent > 100 {
            return Err(ClusterError::InvalidNetworkConfig);
        }
        Ok(self)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClusterPacket {
    pub source: ClusterNodeId,
    pub target: ClusterNodeId,
    pub payload: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterNetworkOutcome {
    Queued { copies: u8, deliver_at: u64 },
    Dropped,
    Partitioned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterNetworkTrace {
    pub tick: u64,
    pub source: ClusterNodeId,
    pub target: ClusterNodeId,
    pub outcome: ClusterNetworkOutcome,
}

struct InFlightPacket {
    packet: ClusterPacket,
    deliver_at: u64,
    sequence: u64,
}

#[derive(Clone, Copy)]
struct PageRedirect {
    global_page: u64,
    target: PoolId,
    target_backing_page: u64,
}

/// A deterministic, bounded network for cluster tests.
pub struct ClusterNetwork {
    config: ClusterNetworkConfig,
    tick: u64,
    random_state: u64,
    next_sequence: u64,
    in_flight: Vec<InFlightPacket>,
    delivered: VecDeque<ClusterPacket>,
    partitions: Vec<(ClusterNodeId, ClusterNodeId)>,
    trace: Vec<ClusterNetworkTrace>,
}

impl ClusterNetwork {
    pub fn new(config: ClusterNetworkConfig) -> Result<Self, ClusterError> {
        let config = config.validate()?;
        Ok(Self {
            random_state: config.seed.max(1),
            config,
            tick: 0,
            next_sequence: 0,
            in_flight: Vec::new(),
            delivered: VecDeque::new(),
            partitions: Vec::new(),
            trace: Vec::new(),
        })
    }

    pub fn config(&self) -> ClusterNetworkConfig {
        self.config
    }

    pub fn tick(&self) -> u64 {
        self.tick
    }

    pub fn trace(&self) -> &[ClusterNetworkTrace] {
        &self.trace
    }

    pub fn partition(&mut self, left: ClusterNodeId, right: ClusterNodeId) {
        if !self.is_partitioned(left, right) {
            self.partitions.push((left, right));
        }
    }

    pub fn reconnect(&mut self, left: ClusterNodeId, right: ClusterNodeId) {
        self.partitions
            .retain(|(a, b)| !((*a == left && *b == right) || (*a == right && *b == left)));
    }

    pub fn is_partitioned(&self, left: ClusterNodeId, right: ClusterNodeId) -> bool {
        self.partitions
            .iter()
            .any(|(a, b)| (*a == left && *b == right) || (*a == right && *b == left))
    }

    pub fn send(&mut self, packet: ClusterPacket) -> ClusterNetworkOutcome {
        if self.is_partitioned(packet.source, packet.target) {
            let outcome = ClusterNetworkOutcome::Partitioned;
            self.record(&packet, outcome);
            return outcome;
        }
        if self.sample_percent() < self.config.loss_percent {
            let outcome = ClusterNetworkOutcome::Dropped;
            self.record(&packet, outcome);
            return outcome;
        }

        self.next_sequence = self.next_sequence.wrapping_add(1);
        let deliver_at = self.tick.saturating_add(self.config.latency_ticks);
        let copies = if self.sample_percent() < self.config.duplicate_percent {
            2
        } else {
            1
        };
        for _ in 0..copies {
            self.in_flight.push(InFlightPacket {
                packet: packet.clone(),
                deliver_at,
                sequence: self.next_sequence,
            });
        }
        let outcome = ClusterNetworkOutcome::Queued { copies, deliver_at };
        self.record(&packet, outcome);
        outcome
    }

    pub fn advance(&mut self, ticks: u64) {
        for _ in 0..ticks {
            self.tick = self.tick.saturating_add(1);
            let mut ready = Vec::new();
            let mut pending = Vec::with_capacity(self.in_flight.len());
            for packet in self.in_flight.drain(..) {
                if packet.deliver_at <= self.tick {
                    ready.push(packet);
                } else {
                    pending.push(packet);
                }
            }
            self.in_flight = pending;
            ready.sort_by_key(|packet| packet.sequence);
            if self.config.reorder && self.tick % 2 == 0 {
                ready.reverse();
            }
            self.delivered
                .extend(ready.into_iter().map(|packet| packet.packet));
        }
    }

    pub fn receive(&mut self, target: ClusterNodeId) -> Vec<ClusterPacket> {
        let mut packets = Vec::new();
        let mut remaining = VecDeque::new();
        while let Some(packet) = self.delivered.pop_front() {
            if packet.target == target {
                packets.push(packet);
            } else {
                remaining.push_back(packet);
            }
        }
        self.delivered = remaining;
        packets
    }

    fn record(&mut self, packet: &ClusterPacket, outcome: ClusterNetworkOutcome) {
        self.trace.push(ClusterNetworkTrace {
            tick: self.tick,
            source: packet.source,
            target: packet.target,
            outcome,
        });
    }

    fn sample_percent(&mut self) -> u8 {
        self.random_state ^= self.random_state << 7;
        self.random_state ^= self.random_state >> 9;
        self.random_state ^= self.random_state << 8;
        (self.random_state % 100) as u8
    }
}

impl Default for ClusterNetwork {
    fn default() -> Self {
        Self::new(ClusterNetworkConfig::default()).expect("default network config is valid")
    }
}

/// A small CXL memory model backed by the fabric address-space implementation.
/// It models discovery, global mapping, page migration, and hot removal.
pub struct CxlFabricFixture {
    space: GlobalAddressSpace<16, 64>,
    next_pool_id: u32,
    bytes: Vec<(u64, u8)>,
    redirects: Vec<PageRedirect>,
}

impl CxlFabricFixture {
    pub fn new() -> Self {
        Self {
            space: GlobalAddressSpace::new(),
            next_pool_id: 1,
            bytes: Vec::new(),
            redirects: Vec::new(),
        }
    }

    pub fn add_device(
        &mut self,
        node: ClusterNodeId,
        global: AddressRange,
        backing_start: u64,
        mirror: Option<ClusterNodeId>,
        latency_ns: u32,
    ) -> Result<PoolId, FabricError> {
        let node = fabric_node(node)?;
        let mirror = mirror.map(fabric_node).transpose()?;
        let pool_id = PoolId::new(self.next_pool_id).ok_or(FabricError::Capacity)?;
        self.next_pool_id = self.next_pool_id.saturating_add(1);
        self.space.add_pool(MemoryPool {
            id: pool_id,
            node,
            mirror,
            kind: MemoryKind::Ram,
            transport: Transport::Cxl,
            global,
            backing_start,
            latency_ns,
        })?;
        Ok(pool_id)
    }

    pub fn resolve(
        &self,
        address: u64,
    ) -> Result<synos_fabric::memory::ResolvedAddress, FabricError> {
        let global_page = address & !(PAGE_SIZE - 1);
        let page_offset = address - global_page;
        if let Some(redirect) = self
            .redirects
            .iter()
            .find(|redirect| redirect.global_page == global_page)
        {
            let target = self
                .space
                .pool(redirect.target)
                .ok_or(FabricError::InvalidAddress)?;
            let target_address = target
                .global
                .start
                .checked_add(
                    redirect
                        .target_backing_page
                        .saturating_sub(target.backing_start),
                )
                .and_then(|address| address.checked_add(page_offset))
                .ok_or(FabricError::InvalidRange)?;
            return self.space.resolve(target_address);
        }
        self.space.resolve(address)
    }

    pub fn write(&mut self, address: u64, data: &[u8]) -> Result<(), FabricError> {
        for (offset, byte) in data.iter().copied().enumerate() {
            let address = address
                .checked_add(offset as u64)
                .ok_or(FabricError::InvalidRange)?;
            self.resolve(address)?;
            if let Some(entry) = self.bytes.iter_mut().find(|entry| entry.0 == address) {
                entry.1 = byte;
            } else {
                self.bytes.push((address, byte));
            }
        }
        Ok(())
    }

    pub fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, FabricError> {
        let mut data = Vec::with_capacity(length);
        for offset in 0..length {
            let address = address
                .checked_add(offset as u64)
                .ok_or(FabricError::InvalidRange)?;
            self.resolve(address)?;
            data.push(
                self.bytes
                    .iter()
                    .find(|entry| entry.0 == address)
                    .map(|entry| entry.1)
                    .unwrap_or(0),
            );
        }
        Ok(data)
    }

    pub fn migrate_page(
        &mut self,
        global_page: u64,
        target: PoolId,
        target_backing_page: u64,
    ) -> Result<(), FabricError> {
        if global_page % PAGE_SIZE != 0 {
            return Err(FabricError::Alignment);
        }
        let source = self
            .space
            .page_pool(global_page)
            .ok_or(FabricError::InvalidAddress)?
            .id;
        self.space
            .begin_migration(synos_fabric::memory::Migration {
                global_page,
                source,
                target,
                target_backing_page,
            })?;
        self.space
            .redirect_page(global_page, target, target_backing_page)?;
        self.redirects
            .retain(|redirect| redirect.global_page != global_page);
        self.redirects.push(PageRedirect {
            global_page,
            target,
            target_backing_page,
        });
        Ok(())
    }

    pub fn hot_remove(&mut self, pool: PoolId) -> Result<(), FabricError> {
        self.space.begin_pool_drain(pool)?;
        self.space.remove_pool(pool).map(|_| ())
    }

    pub fn fail_node(&mut self, node: ClusterNodeId) -> Result<(), FabricError> {
        self.space.mark_node_failed(fabric_node(node)?)
    }

    pub fn restore_node(&mut self, node: ClusterNodeId) -> Result<(), FabricError> {
        self.space.mark_node_alive(fabric_node(node)?)
    }

    pub fn pools(&self) -> impl Iterator<Item = &MemoryPool> {
        self.space.pools()
    }
}

impl Default for CxlFabricFixture {
    fn default() -> Self {
        Self::new()
    }
}

fn fabric_node(node: ClusterNodeId) -> Result<FabricNodeId, FabricError> {
    FabricNodeId::new(node.raw()).ok_or(FabricError::InvalidDevice)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterFault {
    Partition {
        left: ClusterNodeId,
        right: ClusterNodeId,
    },
    Reconnect {
        left: ClusterNodeId,
        right: ClusterNodeId,
    },
    IsolateNode(ClusterNodeId),
    FailNode(ClusterNodeId),
    RecoverNode(ClusterNodeId),
}

#[derive(Debug)]
pub enum ClusterError {
    DuplicateNode(ClusterNodeId),
    UnknownNode(ClusterNodeId),
    InvalidNetworkConfig,
    NodeNotRunning(ClusterNodeId),
    Vm(VmError),
    Fabric(FabricError),
}

impl From<VmError> for ClusterError {
    fn from(error: VmError) -> Self {
        Self::Vm(error)
    }
}

impl From<FabricError> for ClusterError {
    fn from(error: FabricError) -> Self {
        Self::Fabric(error)
    }
}

/// Multi-node VM coordinator used by deterministic and opt-in cluster tests.
pub struct VmCluster {
    nodes: Vec<ClusterNode>,
    network: ClusterNetwork,
    cxl: CxlFabricFixture,
}

impl VmCluster {
    pub fn new(network: ClusterNetworkConfig) -> Result<Self, ClusterError> {
        Ok(Self {
            nodes: Vec::new(),
            network: ClusterNetwork::new(network)?,
            cxl: CxlFabricFixture::new(),
        })
    }

    pub fn add_node(&mut self, id: ClusterNodeId, config: VmConfig) -> Result<(), ClusterError> {
        if self.nodes.iter().any(|node| node.id == id) {
            return Err(ClusterError::DuplicateNode(id));
        }
        self.nodes.push(ClusterNode {
            id,
            vm: Vm::try_with_config(config)?,
            state: ClusterNodeState::Running,
            executed_steps: 0,
        });
        Ok(())
    }

    pub fn node(&self, id: ClusterNodeId) -> Option<&ClusterNode> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn node_mut(&mut self, id: ClusterNodeId) -> Option<&mut ClusterNode> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &ClusterNode> {
        self.nodes.iter()
    }

    pub fn network(&self) -> &ClusterNetwork {
        &self.network
    }

    pub fn network_mut(&mut self) -> &mut ClusterNetwork {
        &mut self.network
    }

    pub fn cxl(&self) -> &CxlFabricFixture {
        &self.cxl
    }

    pub fn cxl_mut(&mut self) -> &mut CxlFabricFixture {
        &mut self.cxl
    }

    pub fn run_node(&mut self, id: ClusterNodeId, steps: u64) -> Result<u64, ClusterError> {
        let node = self.node_mut(id).ok_or(ClusterError::UnknownNode(id))?;
        if node.state != ClusterNodeState::Running {
            return Err(ClusterError::NodeNotRunning(id));
        }
        let report = node.vm.run_for_steps(steps)?;
        node.executed_steps = node.executed_steps.saturating_add(report.steps);
        Ok(report.steps)
    }

    pub fn send(&mut self, packet: ClusterPacket) -> Result<ClusterNetworkOutcome, ClusterError> {
        let source = self
            .node(packet.source)
            .ok_or(ClusterError::UnknownNode(packet.source))?;
        if source.state != ClusterNodeState::Running {
            return Err(ClusterError::NodeNotRunning(packet.source));
        }
        let target = self
            .node(packet.target)
            .ok_or(ClusterError::UnknownNode(packet.target))?;
        if target.state == ClusterNodeState::Failed || target.state == ClusterNodeState::Stopped {
            return Err(ClusterError::NodeNotRunning(packet.target));
        }
        Ok(self.network.send(packet))
    }

    pub fn advance(&mut self, ticks: u64) {
        self.network.advance(ticks)
    }

    pub fn receive(&mut self, target: ClusterNodeId) -> Result<Vec<ClusterPacket>, ClusterError> {
        self.node(target).ok_or(ClusterError::UnknownNode(target))?;
        Ok(self.network.receive(target))
    }

    pub fn inject_fault(&mut self, fault: ClusterFault) -> Result<(), ClusterError> {
        match fault {
            ClusterFault::Partition { left, right } => {
                self.require_node(left)?;
                self.require_node(right)?;
                self.network.partition(left, right);
            }
            ClusterFault::Reconnect { left, right } => {
                self.network.reconnect(left, right);
            }
            ClusterFault::IsolateNode(id) => {
                self.set_node_state(id, ClusterNodeState::Isolated)?;
            }
            ClusterFault::FailNode(id) => {
                self.set_node_state(id, ClusterNodeState::Failed)?;
                self.cxl.fail_node(id)?;
            }
            ClusterFault::RecoverNode(id) => {
                self.set_node_state(id, ClusterNodeState::Running)?;
                self.cxl.restore_node(id)?;
            }
        }
        Ok(())
    }

    fn require_node(&self, id: ClusterNodeId) -> Result<(), ClusterError> {
        self.node(id)
            .map(|_| ())
            .ok_or(ClusterError::UnknownNode(id))
    }

    fn set_node_state(
        &mut self,
        id: ClusterNodeId,
        state: ClusterNodeState,
    ) -> Result<(), ClusterError> {
        let node = self.node_mut(id).ok_or(ClusterError::UnknownNode(id))?;
        node.state = state;
        Ok(())
    }
}
