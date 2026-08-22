//! Deterministic cluster fixtures for VM and GhostOS integration tests.
//!
//! The harness drives VM instances in bounded steps, models an Ethernet-like
//! transport with reproducible faults, and exposes CXL address-space actions
//! without requiring cluster hardware.

use crate::{
    DhcpServerConfig, DeterministicVmNetwork, NetworkBackendConfig, Vm, VmConfig, VmError,
};
use core::mem::size_of;
use std::collections::VecDeque;
use std::cell::RefCell;
use std::rc::Rc;
use ghostos_fabric::memory::{GlobalAddressSpace, MemoryKind, MemoryPool, PoolId, Transport};
use ghostos_fabric::{AddressRange, Error as FabricError, NodeId as FabricNodeId, PAGE_SIZE};

const MAX_CLUSTER_NODES: usize = 1_000;

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

    pub fn pending_packets(&self) -> usize {
        self.in_flight.len() + self.delivered.len()
    }

    pub fn clear_trace(&mut self) {
        self.trace.clear();
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
    ) -> Result<ghostos_fabric::memory::ResolvedAddress, FabricError> {
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

    pub fn discover(&self) -> Vec<MemoryPool> {
        self.space.pools().copied().collect()
    }

    pub fn access(
        &self,
        requester: ClusterNodeId,
        address: u64,
        write: bool,
    ) -> Result<ghostos_fabric::memory::ResolvedAddress, FabricError> {
        let requester = fabric_node(requester)?;
        let resolved = self.resolve(address)?;
        if resolved.node != requester && write {
            return Err(FabricError::NotOwner)
        }
        Ok(resolved)
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
            .begin_migration(ghostos_fabric::memory::Migration {
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
pub struct SharedMemoryMapping {
    pub node: ClusterNodeId,
    pub offset: usize,
    pub length: usize,
    pub epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SharedMemoryDevice {
    pub size: usize,
    pub epoch: u64,
    pub present: bool,
}

/// Deterministic shared-memory fixture for ivshmem-style cluster tests.
pub struct SharedMemoryFixture {
    bytes: Vec<u8>,
    epoch: u64,
    present: bool,
    corrupted: bool,
    nodes: Vec<ClusterNodeId>,
    failed_nodes: Vec<ClusterNodeId>,
}

impl SharedMemoryFixture {
    const MAX_SIZE: usize = 64 * 1024 * 1024;

    pub fn new(size: usize) -> Result<Self, ClusterError> {
        if size == 0 || size % PAGE_SIZE as usize != 0 || size > Self::MAX_SIZE {
            return Err(ClusterError::InvalidSharedMemorySize)
        }
        Ok(Self {
            bytes: vec![0; size],
            epoch: 1,
            present: true,
            corrupted: false,
            nodes: Vec::new(),
            failed_nodes: Vec::new(),
        })
    }

    pub fn discover(&self) -> SharedMemoryDevice {
        SharedMemoryDevice {
            size: self.bytes.len(),
            epoch: self.epoch,
            present: self.present,
        }
    }

    pub fn register_node(&mut self, node: ClusterNodeId) {
        if !self.nodes.contains(&node) {
            self.nodes.push(node);
        }
        self.failed_nodes.retain(|failed| *failed != node);
    }

    pub fn fail_node(&mut self, node: ClusterNodeId) {
        if !self.failed_nodes.contains(&node) {
            self.failed_nodes.push(node);
        }
    }

    pub fn restore_node(&mut self, node: ClusterNodeId) {
        self.failed_nodes.retain(|failed| *failed != node);
    }

    pub fn map(
        &self,
        node: ClusterNodeId,
        offset: usize,
        length: usize,
    ) -> Result<SharedMemoryMapping, ClusterError> {
        self.check_access(node, offset, length)?;
        Ok(SharedMemoryMapping {
            node,
            offset,
            length,
            epoch: self.epoch,
        })
    }

    pub fn read(
        &self,
        node: ClusterNodeId,
        offset: usize,
        length: usize,
    ) -> Result<Vec<u8>, ClusterError> {
        self.check_access(node, offset, length)?;
        if self.corrupted {
            return Err(ClusterError::CorruptSharedMemory)
        }
        Ok(self.bytes[offset..offset + length].to_vec())
    }

    pub fn write(
        &mut self,
        node: ClusterNodeId,
        offset: usize,
        data: &[u8],
    ) -> Result<(), ClusterError> {
        self.check_access(node, offset, data.len())?;
        if self.corrupted {
            return Err(ClusterError::CorruptSharedMemory)
        }
        self.bytes[offset..offset + data.len()].copy_from_slice(data);
        Ok(())
    }

    pub fn hot_remove(&mut self) {
        self.present = false;
        self.epoch = self.epoch.saturating_add(1);
    }

    pub fn restore(&mut self) {
        self.present = true;
        self.corrupted = false;
        self.epoch = self.epoch.saturating_add(1);
    }

    pub fn corrupt(&mut self) {
        self.corrupted = true;
    }

    pub fn repair(&mut self) {
        self.corrupted = false;
        self.epoch = self.epoch.saturating_add(1);
    }

    fn check_access(
        &self,
        node: ClusterNodeId,
        offset: usize,
        length: usize,
    ) -> Result<(), ClusterError> {
        if !self.present {
            return Err(ClusterError::SharedMemoryUnavailable)
        }
        if !self.nodes.contains(&node) {
            return Err(ClusterError::UnknownNode(node))
        }
        if self.failed_nodes.contains(&node) {
            return Err(ClusterError::NodeNotRunning(node))
        }
        let end = offset
            .checked_add(length)
            .ok_or(ClusterError::InvalidSharedMemoryRange)?;
        if end > self.bytes.len() {
            return Err(ClusterError::InvalidSharedMemoryRange)
        }
        Ok(())
    }
}

impl Default for SharedMemoryFixture {
    fn default() -> Self {
        Self::new(PAGE_SIZE as usize).expect("default shared memory size is valid")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterHeartbeat {
    pub source: ClusterNodeId,
    pub sequence: u64,
    pub epoch: u64,
    pub tick: u64,
}

impl ClusterHeartbeat {
    pub const WIRE_BYTES: usize = 28;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterStatus {
    pub epoch: u64,
    pub members: usize,
    pub running: usize,
    pub quorum: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterScaleEvidence {
    pub nodes: usize,
    pub discovered_nodes: usize,
    pub heartbeat_messages: usize,
    pub control_plane_traffic_bytes: usize,
    pub control_plane_memory_bytes: usize,
    pub convergence_ticks: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterWorkload {
    Ipc,
    FilesystemCommit,
    MemoryFetch,
    Inference,
    MembershipChange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterFaultRecord {
    pub node: ClusterNodeId,
    pub workload: ClusterWorkload,
    pub recovered: bool,
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
    KillNodeDuring {
        node: ClusterNodeId,
        workload: ClusterWorkload,
    },
}

#[derive(Debug)]
pub enum ClusterError {
    DuplicateNode(ClusterNodeId),
    UnknownNode(ClusterNodeId),
    InvalidNetworkConfig,
    NodeNotRunning(ClusterNodeId),
    InvalidSharedMemorySize,
    InvalidSharedMemoryRange,
    SharedMemoryUnavailable,
    CorruptSharedMemory,
    StaleEpoch,
    NoFaultToRecover,
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
    vm_network: Rc<RefCell<DeterministicVmNetwork>>,
    cxl: CxlFabricFixture,
    shared_memory: SharedMemoryFixture,
    epoch: u64,
    heartbeat_sequences: Vec<(ClusterNodeId, u64)>,
    observed_heartbeats: Vec<(ClusterNodeId, ClusterNodeId, u64)>,
    fault_records: Vec<ClusterFaultRecord>,
}

impl VmCluster {
    pub fn new(network: ClusterNetworkConfig) -> Result<Self, ClusterError> {
        let vm_network = DeterministicVmNetwork::new(Some(DhcpServerConfig::default()))
            .map_err(|error| ClusterError::Vm(VmError::Network(error.to_string())))?;
        Ok(Self {
            nodes: Vec::new(),
            network: ClusterNetwork::new(network)?,
            vm_network,
            cxl: CxlFabricFixture::new(),
            shared_memory: SharedMemoryFixture::default(),
            epoch: 1,
            heartbeat_sequences: Vec::new(),
            observed_heartbeats: Vec::new(),
            fault_records: Vec::new(),
        })
    }

    pub fn add_node(&mut self, id: ClusterNodeId, config: VmConfig) -> Result<(), ClusterError> {
        if self.nodes.iter().any(|node| node.id == id) {
            return Err(ClusterError::DuplicateNode(id));
        }
        let mut config = config;
        if matches!(config.network, NetworkBackendConfig::Deterministic) {
            config.network = NetworkBackendConfig::DeterministicShared {
                network: self.vm_network.clone(),
            };
            config.dhcp_server = None;
        }
        self.nodes.push(ClusterNode {
            id,
            vm: Vm::try_with_config(config)?,
            state: ClusterNodeState::Running,
            executed_steps: 0,
        });
        self.shared_memory.register_node(id);
        self.heartbeat_sequences.push((id, 0));
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

    pub fn vm_network(&self) -> Rc<RefCell<DeterministicVmNetwork>> {
        self.vm_network.clone()
    }

    pub fn cxl(&self) -> &CxlFabricFixture {
        &self.cxl
    }

    pub fn cxl_mut(&mut self) -> &mut CxlFabricFixture {
        &mut self.cxl
    }

    pub fn shared_memory(&self) -> &SharedMemoryFixture {
        &self.shared_memory
    }

    pub fn shared_memory_mut(&mut self) -> &mut SharedMemoryFixture {
        &mut self.shared_memory
    }

    pub fn status(&self) -> ClusterStatus {
        let running = self
            .nodes
            .iter()
            .filter(|node| node.state == ClusterNodeState::Running)
            .count();
        ClusterStatus {
            epoch: self.epoch,
            members: self.nodes.len(),
            running,
            quorum: running >= self.nodes.len() / 2 + 1,
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn discover_nodes(&self) -> Vec<ClusterNodeId> {
        self.nodes.iter().map(ClusterNode::id).collect()
    }

    pub fn heartbeat(&mut self, source: ClusterNodeId) -> Result<ClusterHeartbeat, ClusterError> {
        let node = self.node(source).ok_or(ClusterError::UnknownNode(source))?;
        if node.state != ClusterNodeState::Running {
            return Err(ClusterError::NodeNotRunning(source));
        }
        let sequence = self
            .heartbeat_sequences
            .iter_mut()
            .find(|entry| entry.0 == source)
            .ok_or(ClusterError::UnknownNode(source))?;
        sequence.1 = sequence.1.saturating_add(1);
        Ok(ClusterHeartbeat {
            source,
            sequence: sequence.1,
            epoch: self.epoch,
            tick: self.network.tick(),
        })
    }

    pub fn observe_heartbeat(
        &mut self,
        target: ClusterNodeId,
        heartbeat: ClusterHeartbeat,
    ) -> Result<(), ClusterError> {
        let node = self.node(target).ok_or(ClusterError::UnknownNode(target))?;
        if node.state != ClusterNodeState::Running {
            return Err(ClusterError::NodeNotRunning(target));
        }
        if heartbeat.epoch != self.epoch {
            return Err(ClusterError::StaleEpoch);
        }
        if !self
            .observed_heartbeats
            .iter()
            .any(|entry| entry.0 == target && entry.1 == heartbeat.source)
        {
            self.observed_heartbeats
                .push((target, heartbeat.source, heartbeat.sequence));
        } else if let Some(entry) = self
            .observed_heartbeats
            .iter_mut()
            .find(|entry| entry.0 == target && entry.1 == heartbeat.source)
        {
            if heartbeat.sequence <= entry.2 {
                return Ok(())
            }
            entry.2 = heartbeat.sequence;
        }
        Ok(())
    }

    pub fn fault_records(&self) -> &[ClusterFaultRecord] {
        &self.fault_records
    }

    pub fn evidence(&self) -> ClusterEvidence {
        ClusterEvidence {
            tick: self.network.tick(),
            epoch: self.epoch,
            node_states: self.nodes.iter().map(|node| (node.id, node.state)).collect(),
            serial_output: self
                .nodes
                .iter()
                .map(|node| (node.id, node.serial_output()))
                .collect(),
            network_trace: self.network.trace().to_vec(),
            fault_records: self.fault_records.clone(),
            network_events: self.network.trace().len(),
            pending_packets: self.network.pending_packets(),
            cxl_devices: self.cxl.pools().count(),
            shared_memory: self.shared_memory.discover(),
        }
    }

    pub fn scale_evidence(&self) -> ClusterScaleEvidence {
        let nodes = self.nodes.len();
        let discovered_nodes = self.discover_nodes().len();
        let heartbeat_messages = self.network.trace().len();
        let control_plane_traffic_bytes = discovered_nodes
            .saturating_mul(size_of::<ClusterNodeId>())
            .saturating_add(heartbeat_messages.saturating_mul(ClusterHeartbeat::WIRE_BYTES));
        let control_plane_memory_bytes = nodes
            .saturating_mul(size_of::<ClusterNodeId>())
            .saturating_add(
                self.heartbeat_sequences
                    .len()
                    .saturating_mul(size_of::<(ClusterNodeId, u64)>()),
            )
            .saturating_add(
                self.observed_heartbeats
                    .len()
                    .saturating_mul(size_of::<(ClusterNodeId, ClusterNodeId, u64)>()),
            )
            .saturating_add(self.fault_records.len().saturating_mul(size_of::<ClusterFaultRecord>()));
        ClusterScaleEvidence {
            nodes,
            discovered_nodes,
            heartbeat_messages,
            control_plane_traffic_bytes,
            control_plane_memory_bytes,
            convergence_ticks: self.network.tick(),
        }
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
        if target.state != ClusterNodeState::Running {
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
                self.require_node(left)?;
                self.require_node(right)?;
                self.network.reconnect(left, right);
            }
            ClusterFault::IsolateNode(id) => {
                self.set_node_state(id, ClusterNodeState::Isolated)?;
            }
            ClusterFault::FailNode(id) => {
                self.set_node_state(id, ClusterNodeState::Failed)?;
                self.cxl.fail_node(id)?;
                self.shared_memory.fail_node(id);
                self.epoch = self.epoch.saturating_add(1);
            }
            ClusterFault::RecoverNode(id) => {
                self.set_node_state(id, ClusterNodeState::Running)?;
                self.cxl.restore_node(id)?;
                self.shared_memory.restore_node(id);
                self.shared_memory.register_node(id);
                self.epoch = self.epoch.saturating_add(1);
            }
            ClusterFault::KillNodeDuring { node, workload } => {
                self.set_node_state(node, ClusterNodeState::Failed)?;
                self.cxl.fail_node(node)?;
                self.shared_memory.fail_node(node);
                self.epoch = self.epoch.saturating_add(1);
                self.fault_records.push(ClusterFaultRecord {
                    node,
                    workload,
                    recovered: false,
                });
            }
        }
        Ok(())
    }

    pub fn recover_last_fault(&mut self) -> Result<(), ClusterError> {
        let index = self
            .fault_records
            .len()
            .checked_sub(1)
            .ok_or(ClusterError::NoFaultToRecover)?;
        if self.fault_records[index].recovered {
            return Ok(())
        }
        let node = self.fault_records[index].node;
        self.set_node_state(node, ClusterNodeState::Running)?;
        self.cxl.restore_node(node)?;
        self.shared_memory.restore_node(node);
        self.shared_memory.register_node(node);
        self.epoch = self.epoch.saturating_add(1);
        self.fault_records[index].recovered = true;
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClusterEvidence {
    pub tick: u64,
    pub epoch: u64,
    pub node_states: Vec<(ClusterNodeId, ClusterNodeState)>,
    pub serial_output: Vec<(ClusterNodeId, Vec<u8>)>,
    pub network_trace: Vec<ClusterNetworkTrace>,
    pub fault_records: Vec<ClusterFaultRecord>,
    pub network_events: usize,
    pub pending_packets: usize,
    pub cxl_devices: usize,
    pub shared_memory: SharedMemoryDevice,
}

impl ClusterEvidence {
    pub fn to_text(&self) -> String {
        let states = self
            .node_states
            .iter()
            .map(|(node, state)| format!("{}:{state:?}", node.raw()))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "tick={} epoch={} nodes={} network_events={} pending_packets={} cxl_devices={} shared_memory={{size={},epoch={},present={}}}\n",
            self.tick,
            self.epoch,
            states,
            self.network_trace.len(),
            self.pending_packets,
            self.cxl_devices,
            self.shared_memory.size,
            self.shared_memory.epoch,
            self.shared_memory.present,
        )
    }
}
