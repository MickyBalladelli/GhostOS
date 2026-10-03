//! Deterministic cluster fixtures for VM and GhostOS integration tests.
//!
//! The harness drives VM instances in bounded steps, models an Ethernet-like
//! transport with reproducible faults, and exposes CXL address-space actions
//! without requiring cluster hardware.

use crate::{
    DhcpServerConfig, DeterministicVmNetwork, NetworkBackendConfig, Vm, VmConfig, VmError,
};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::rc::Rc;
use ghostos_fabric::memory::{GlobalAddressSpace, MemoryKind, MemoryPool, PoolId, Transport};
use ghostos_fabric::{AddressRange, Error as FabricError, NodeId as FabricNodeId, PAGE_SIZE};

#[repr(C)]
struct CClusterNode {
    used: bool,
    id: CClusterNodeId,
    state: CClusterNodeState,
    executed_steps: u64,
    heartbeat_sequence: u64,
    vm_context: *mut c_void,
    run_vm: Option<unsafe extern "C" fn(*mut c_void, u64, *mut u64) -> CClusterError>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CClusterNodeId {
    raw: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CClusterNetworkConfig {
    latency_ticks: u64,
    loss_percent: u8,
    duplicate_percent: u8,
    reorder: bool,
    seed: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CClusterNetworkOutcome {
    result: CClusterNetworkResult,
    copies: u8,
    deliver_at: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CClusterNetworkTrace {
    tick: u64,
    source: CClusterNodeId,
    target: CClusterNodeId,
    outcome: CClusterNetworkOutcome,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CClusterPacket {
    used: bool,
    source: CClusterNodeId,
    target: CClusterNodeId,
    deliver_at: u64,
    sequence: u64,
    length: usize,
    payload: [u8; 1500],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CClusterPartition {
    left: CClusterNodeId,
    right: CClusterNodeId,
}

#[repr(C)]
struct CClusterNetwork {
    config: CClusterNetworkConfig,
    tick: u64,
    random_state: u64,
    next_sequence: u64,
    in_flight: [CClusterPacket; 4096],
    delivered: [CClusterPacket; 4096],
    delivered_count: usize,
    partitions: [CClusterPartition; 4096],
    partition_count: usize,
    trace: [CClusterNetworkTrace; 8192],
    trace_count: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CClusterSharedDevice {
    size: usize,
    epoch: u64,
    present: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CClusterSharedMapping {
    node: CClusterNodeId,
    offset: usize,
    length: usize,
    epoch: u64,
}

#[repr(C)]
struct CClusterSharedMemory {
    bytes: *mut u8,
    size: usize,
    epoch: u64,
    present: bool,
    corrupted: bool,
    nodes: [CClusterNodeId; 1000],
    node_count: usize,
    failed: [CClusterNodeId; 1000],
    failed_count: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CClusterHeartbeat {
    source: CClusterNodeId,
    sequence: u64,
    epoch: u64,
    tick: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CClusterStatus {
    epoch: u64,
    members: usize,
    running: usize,
    quorum: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CClusterScaleEvidence {
    nodes: usize,
    discovered_nodes: usize,
    heartbeat_messages: usize,
    control_plane_traffic_bytes: usize,
    control_plane_memory_bytes: usize,
    convergence_ticks: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CClusterFaultRecord {
    node: CClusterNodeId,
    workload: CClusterWorkload,
    recovered: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CClusterObservedHeartbeat {
    target: CClusterNodeId,
    source: CClusterNodeId,
    sequence: u64,
}

#[repr(C)]
struct CCluster {
    nodes: [CClusterNode; 1000],
    node_count: usize,
    network: CClusterNetwork,
    shared_memory: CClusterSharedMemory,
    epoch: u64,
    observed: [CClusterObservedHeartbeat; 4000],
    observed_count: usize,
    faults: [CClusterFaultRecord; 1024],
    fault_count: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq)]
enum CClusterError {
    Ok = 0,
    InvalidId,
    InvalidConfig,
    Capacity,
    DuplicateNode,
    UnknownNode,
    NodeNotRunning,
    InvalidSharedSize,
    InvalidSharedRange,
    SharedUnavailable,
    SharedCorrupt,
    StaleEpoch,
    NoFault,
    BufferTooSmall,
    VmError,
}

#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq)]
enum CClusterNodeState {
    Running,
    Isolated,
    Failed,
    Stopped,
}

#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq)]
enum CClusterNetworkResult {
    Queued,
    Dropped,
    Partitioned,
}

#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq)]
enum CClusterWorkload {
    Ipc,
    FilesystemCommit,
    MemoryFetch,
    Inference,
    MembershipChange,
}

unsafe extern "C" {
    fn ghostos_cluster_node_id_from_raw(raw: u32, out: *mut CClusterNodeId) -> bool;
    fn ghostos_vm_cluster_init(
        cluster: *mut CCluster,
        config: CClusterNetworkConfig,
        shared_storage: *mut u8,
        shared_size: usize,
    ) -> CClusterError;
    fn ghostos_vm_cluster_add_node(
        cluster: *mut CCluster,
        id: CClusterNodeId,
        vm_context: *mut c_void,
        run_vm: Option<unsafe extern "C" fn(*mut c_void, u64, *mut u64) -> CClusterError>,
    ) -> CClusterError;
    fn ghostos_vm_cluster_status(cluster: *const CCluster) -> CClusterStatus;
    fn ghostos_vm_cluster_heartbeat(
        cluster: *mut CCluster,
        source: CClusterNodeId,
        out: *mut CClusterHeartbeat,
    ) -> CClusterError;
    fn ghostos_vm_cluster_observe_heartbeat(
        cluster: *mut CCluster,
        target: CClusterNodeId,
        heartbeat: CClusterHeartbeat,
    ) -> CClusterError;
    fn ghostos_vm_cluster_run_node(
        cluster: *mut CCluster,
        id: CClusterNodeId,
        steps: u64,
        executed: *mut u64,
    ) -> CClusterError;
    fn ghostos_vm_cluster_send(
        cluster: *mut CCluster,
        source: CClusterNodeId,
        target: CClusterNodeId,
        payload: *const u8,
        length: usize,
        out: *mut CClusterNetworkOutcome,
    ) -> CClusterError;
    fn ghostos_vm_cluster_fault(
        cluster: *mut CCluster,
        kind: u32,
        left: CClusterNodeId,
        right: CClusterNodeId,
        workload: CClusterWorkload,
    ) -> CClusterError;
    fn ghostos_vm_cluster_recover_last_fault(cluster: *mut CCluster) -> CClusterError;
    fn ghostos_vm_cluster_scale(cluster: *const CCluster) -> CClusterScaleEvidence;
    fn ghostos_cluster_network_receive(
        network: *mut CClusterNetwork,
        target: CClusterNodeId,
        out: *mut CClusterPacket,
        capacity: usize,
    ) -> usize;
    fn ghostos_cluster_network_advance(network: *mut CClusterNetwork, ticks: u64);
    fn ghostos_cluster_shared_discover(memory: *const CClusterSharedMemory) -> CClusterSharedDevice;
    fn ghostos_cluster_shared_map(
        memory: *const CClusterSharedMemory,
        node: CClusterNodeId,
        offset: usize,
        length: usize,
        out: *mut CClusterSharedMapping,
    ) -> CClusterError;
    fn ghostos_cluster_shared_read(
        memory: *const CClusterSharedMemory,
        node: CClusterNodeId,
        offset: usize,
        out: *mut u8,
        length: usize,
    ) -> CClusterError;
    fn ghostos_cluster_shared_write(
        memory: *mut CClusterSharedMemory,
        node: CClusterNodeId,
        offset: usize,
        data: *const u8,
        length: usize,
    ) -> CClusterError;
    fn ghostos_cluster_shared_hot_remove(memory: *mut CClusterSharedMemory);
    fn ghostos_cluster_shared_restore(memory: *mut CClusterSharedMemory);
    fn ghostos_cluster_shared_corrupt(memory: *mut CClusterSharedMemory);
    fn ghostos_cluster_shared_repair(memory: *mut CClusterSharedMemory);
}

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
    nodes: Vec<Box<ClusterNode>>,
    network: ClusterNetwork,
    vm_network: Rc<RefCell<DeterministicVmNetwork>>,
    cxl: CxlFabricFixture,
    _shared_memory: SharedMemoryFixture,
    epoch: u64,
    heartbeat_sequences: Vec<(ClusterNodeId, u64)>,
    observed_heartbeats: Vec<(ClusterNodeId, ClusterNodeId, u64)>,
    fault_records: Vec<ClusterFaultRecord>,
    c_cluster: Box<CCluster>,
    _shared_memory_storage: Vec<u8>,
}

impl VmCluster {
    pub fn new(network: ClusterNetworkConfig) -> Result<Self, ClusterError> {
        let vm_network = DeterministicVmNetwork::new(Some(DhcpServerConfig::default()))
            .map_err(|error| ClusterError::Vm(VmError::Network(error.to_string())))?;
        let mut c_cluster = Box::<std::mem::MaybeUninit<CCluster>>::new_zeroed();
        let mut shared_memory_storage = vec![0; PAGE_SIZE as usize];
        map_cluster_error(
            unsafe {
                ghostos_vm_cluster_init(
                    c_cluster.as_mut_ptr().cast::<CCluster>(),
                    to_c_network_config(network),
                    shared_memory_storage.as_mut_ptr(),
                    shared_memory_storage.len(),
                )
            },
            None,
        )?;
        let c_cluster = unsafe { Box::from_raw(Box::into_raw(c_cluster).cast::<CCluster>()) };
        Ok(Self {
            nodes: Vec::new(),
            network: ClusterNetwork::new(network)?,
            vm_network,
            cxl: CxlFabricFixture::new(),
            _shared_memory: SharedMemoryFixture::default(),
            epoch: 1,
            heartbeat_sequences: Vec::new(),
            observed_heartbeats: Vec::new(),
            fault_records: Vec::new(),
            c_cluster,
            _shared_memory_storage: shared_memory_storage,
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
        self.nodes.push(Box::new(ClusterNode {
            id,
            vm: Vm::try_with_config(config)?,
            state: ClusterNodeState::Running,
            executed_steps: 0,
        }));
        let node_ref = self.nodes.last_mut().expect("cluster node inserted").as_mut();
        map_cluster_error(
            unsafe {
                ghostos_vm_cluster_add_node(
                    &mut *self.c_cluster,
                    to_c_node_id(id),
                    (&mut node_ref.vm as *mut Vm).cast(),
                    Some(run_vm_from_c),
                )
            },
            Some(id),
        )?;
        self.sync_from_c();
        Ok(())
    }

    pub fn node(&self, id: ClusterNodeId) -> Option<&ClusterNode> {
        self.nodes.iter().find(|node| node.id == id).map(Box::as_ref)
    }

    pub fn node_mut(&mut self, id: ClusterNodeId) -> Option<&mut ClusterNode> {
        self.nodes.iter_mut().find(|node| node.id == id).map(Box::as_mut)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &ClusterNode> {
        self.nodes.iter().map(Box::as_ref)
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

    pub fn shared_memory(&self) -> SharedMemoryFixtureView<'_> {
        SharedMemoryFixtureView {
            memory: &self.c_cluster.shared_memory,
        }
    }

    pub fn shared_memory_mut(&mut self) -> SharedMemoryFixtureMut<'_> {
        SharedMemoryFixtureMut {
            memory: &mut self.c_cluster.shared_memory,
        }
    }

    pub fn status(&self) -> ClusterStatus {
        from_c_status(unsafe { ghostos_vm_cluster_status(&*self.c_cluster) })
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn discover_nodes(&self) -> Vec<ClusterNodeId> {
        self.nodes.iter().map(|node| node.id()).collect()
    }

    pub fn heartbeat(&mut self, source: ClusterNodeId) -> Result<ClusterHeartbeat, ClusterError> {
        let mut heartbeat = CClusterHeartbeat {
            source: to_c_node_id(source),
            sequence: 0,
            epoch: 0,
            tick: 0,
        };
        map_cluster_error(
            unsafe { ghostos_vm_cluster_heartbeat(&mut *self.c_cluster, to_c_node_id(source), &mut heartbeat) },
            Some(source),
        )?;
        self.sync_from_c();
        Ok(from_c_heartbeat(heartbeat))
    }

    pub fn observe_heartbeat(
        &mut self,
        target: ClusterNodeId,
        heartbeat: ClusterHeartbeat,
    ) -> Result<(), ClusterError> {
        map_cluster_error(
            unsafe {
                ghostos_vm_cluster_observe_heartbeat(
                    &mut *self.c_cluster,
                    to_c_node_id(target),
                    to_c_heartbeat(heartbeat),
                )
            },
            Some(target),
        )?;
        self.sync_from_c();
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
            shared_memory: self.shared_memory().discover(),
        }
    }

    pub fn scale_evidence(&self) -> ClusterScaleEvidence {
        from_c_scale_evidence(unsafe { ghostos_vm_cluster_scale(&*self.c_cluster) })
    }

    pub fn run_node(&mut self, id: ClusterNodeId, steps: u64) -> Result<u64, ClusterError> {
        let mut executed = 0;
        map_cluster_error(
            unsafe { ghostos_vm_cluster_run_node(&mut *self.c_cluster, to_c_node_id(id), steps, &mut executed) },
            Some(id),
        )?;
        self.sync_from_c();
        Ok(executed)
    }

    pub fn send(&mut self, packet: ClusterPacket) -> Result<ClusterNetworkOutcome, ClusterError> {
        let mut outcome = CClusterNetworkOutcome {
            result: CClusterNetworkResult::Dropped,
            copies: 0,
            deliver_at: 0,
        };
        map_cluster_error(
            unsafe {
                ghostos_vm_cluster_send(
                    &mut *self.c_cluster,
                    to_c_node_id(packet.source),
                    to_c_node_id(packet.target),
                    packet.payload.as_ptr(),
                    packet.payload.len(),
                    &mut outcome,
                )
            },
            Some(packet.source),
        )?;
        self.sync_from_c();
        Ok(from_c_network_outcome(outcome))
    }

    pub fn advance(&mut self, ticks: u64) {
        unsafe { ghostos_cluster_network_advance(&mut self.c_cluster.network, ticks) };
        self.sync_from_c();
    }

    pub fn receive(&mut self, target: ClusterNodeId) -> Result<Vec<ClusterPacket>, ClusterError> {
        self.node(target).ok_or(ClusterError::UnknownNode(target))?;
        let mut packets = vec![unsafe { std::mem::zeroed::<CClusterPacket>() }; 4096];
        let count = unsafe {
            ghostos_cluster_network_receive(
                &mut self.c_cluster.network,
                to_c_node_id(target),
                packets.as_mut_ptr(),
                packets.len(),
            )
        };
        packets.truncate(count);
        self.sync_from_c();
        Ok(packets.into_iter().map(from_c_packet).collect())
    }

    pub fn inject_fault(&mut self, fault: ClusterFault) -> Result<(), ClusterError> {
        let (kind, left, right, workload) = match fault {
            ClusterFault::Partition { left, right } => (0, left, right, ClusterWorkload::Ipc),
            ClusterFault::Reconnect { left, right } => (1, left, right, ClusterWorkload::Ipc),
            ClusterFault::IsolateNode(id) => (2, id, id, ClusterWorkload::Ipc),
            ClusterFault::FailNode(id) => (3, id, id, ClusterWorkload::Ipc),
            ClusterFault::RecoverNode(id) => (4, id, id, ClusterWorkload::Ipc),
            ClusterFault::KillNodeDuring { node, workload } => (5, node, node, workload),
        };
        map_cluster_error(
            unsafe {
                ghostos_vm_cluster_fault(
                    &mut *self.c_cluster,
                    kind,
                    to_c_node_id(left),
                    to_c_node_id(right),
                    to_c_workload(workload),
                )
            },
            Some(left),
        )?;
        match fault {
            ClusterFault::FailNode(id) | ClusterFault::KillNodeDuring { node: id, .. } => {
                self.cxl.fail_node(id)?;
            }
            ClusterFault::RecoverNode(id) => {
                self.cxl.restore_node(id)?;
            }
            _ => {}
        }
        self.sync_from_c();
        Ok(())
    }

    pub fn recover_last_fault(&mut self) -> Result<(), ClusterError> {
        let node = self
            .fault_records
            .last()
            .copied()
            .ok_or(ClusterError::NoFaultToRecover)?
            .node;
        map_cluster_error(unsafe { ghostos_vm_cluster_recover_last_fault(&mut *self.c_cluster) }, Some(node))?;
        self.cxl.restore_node(node)?;
        self.sync_from_c();
        Ok(())
    }

    fn sync_from_c(&mut self) {
        self.epoch = self.c_cluster.epoch;
        self.heartbeat_sequences.clear();
        for i in 0..self.c_cluster.node_count {
            let c_node = &self.c_cluster.nodes[i];
            let id = from_c_node_id(c_node.id);
            self.heartbeat_sequences.push((id, c_node.heartbeat_sequence));
        }
        self.observed_heartbeats.clear();
        for i in 0..self.c_cluster.observed_count {
            let observed = &self.c_cluster.observed[i];
            self.observed_heartbeats.push((
                from_c_node_id(observed.target),
                from_c_node_id(observed.source),
                observed.sequence,
            ));
        }
        self.fault_records.clear();
        for i in 0..self.c_cluster.fault_count {
            self.fault_records.push(from_c_fault_record(self.c_cluster.faults[i]));
        }
        let c_nodes = &self.c_cluster.nodes[..self.c_cluster.node_count];
        for node in &mut self.nodes {
            if let Some(c_node) = c_nodes.iter().find(|c_node| c_node.id.raw == node.id.raw()) {
                node.state = from_c_node_state(c_node.state);
                node.executed_steps = c_node.executed_steps;
            }
        }
        self.sync_network_from_c();
    }

    fn sync_network_from_c(&mut self) {
        let c_network = &self.c_cluster.network;
        self.network = ClusterNetwork::new(from_c_network_config(c_network.config))
            .expect("C cluster network config is valid");
        self.network.tick = c_network.tick;
        self.network.random_state = c_network.random_state;
        self.network.next_sequence = c_network.next_sequence;
        self.network.partitions = c_network.partitions[..c_network.partition_count]
            .iter()
            .map(|entry| (from_c_node_id(entry.left), from_c_node_id(entry.right)))
            .collect();
        self.network.trace = c_network.trace[..c_network.trace_count]
            .iter()
            .copied()
            .map(from_c_network_trace)
            .collect();
        self.network.delivered = c_network.delivered[..c_network.delivered_count]
            .iter()
            .copied()
            .map(from_c_packet)
            .collect();
        self.network.in_flight = c_network.in_flight
            .iter()
            .copied()
            .filter(|packet| packet.used)
            .map(from_c_in_flight_packet)
            .collect();
    }

    fn c_node(&self, id: ClusterNodeId) -> Option<&CClusterNode> {
        let node = unsafe { ghostos_vm_cluster_node((&*self.c_cluster as *const CCluster).cast_mut(), to_c_node_id(id)) };
        if node.is_null() {
            None
        } else {
            Some(unsafe { &*node })
        }
    }
}

pub struct SharedMemoryFixtureView<'a> {
    memory: &'a CClusterSharedMemory,
}

impl SharedMemoryFixtureView<'_> {
    pub fn discover(&self) -> SharedMemoryDevice {
        from_c_shared_device(unsafe { ghostos_cluster_shared_discover(self.memory) })
    }

    pub fn map(&self, node: ClusterNodeId, offset: usize, length: usize) -> Result<SharedMemoryMapping, ClusterError> {
        let mut mapping = CClusterSharedMapping {
            node: to_c_node_id(node),
            offset,
            length,
            epoch: 0,
        };
        map_cluster_error(
            unsafe { ghostos_cluster_shared_map(self.memory, to_c_node_id(node), offset, length, &mut mapping) },
            Some(node),
        )?;
        Ok(from_c_shared_mapping(mapping))
    }

    pub fn read(&self, node: ClusterNodeId, offset: usize, length: usize) -> Result<Vec<u8>, ClusterError> {
        let mut bytes = vec![0; length];
        map_cluster_error(
            unsafe { ghostos_cluster_shared_read(self.memory, to_c_node_id(node), offset, bytes.as_mut_ptr(), length) },
            Some(node),
        )?;
        Ok(bytes)
    }
}

pub struct SharedMemoryFixtureMut<'a> {
    memory: &'a mut CClusterSharedMemory,
}

impl SharedMemoryFixtureMut<'_> {
    pub fn map(&mut self, node: ClusterNodeId, offset: usize, length: usize) -> Result<SharedMemoryMapping, ClusterError> {
        SharedMemoryFixtureView { memory: self.memory }.map(node, offset, length)
    }

    pub fn write(&mut self, node: ClusterNodeId, offset: usize, data: &[u8]) -> Result<(), ClusterError> {
        map_cluster_error(
            unsafe { ghostos_cluster_shared_write(self.memory, to_c_node_id(node), offset, data.as_ptr(), data.len()) },
            Some(node),
        )
    }

    pub fn corrupt(&mut self) {
        unsafe { ghostos_cluster_shared_corrupt(self.memory) }
    }

    pub fn repair(&mut self) {
        unsafe { ghostos_cluster_shared_repair(self.memory) }
    }

    pub fn hot_remove(&mut self) {
        unsafe { ghostos_cluster_shared_hot_remove(self.memory) }
    }

    pub fn restore(&mut self) {
        unsafe { ghostos_cluster_shared_restore(self.memory) }
    }
}

unsafe extern "C" fn run_vm_from_c(context: *mut c_void, steps: u64, executed: *mut u64) -> CClusterError {
    if context.is_null() {
        return CClusterError::VmError;
    }
    let vm = unsafe { &mut *context.cast::<Vm>() };
    match vm.run_for_steps(steps) {
        Ok(report) => {
            if !executed.is_null() {
                unsafe { *executed = report.steps };
            }
            CClusterError::Ok
        }
        Err(_) => CClusterError::VmError,
    }
}

fn to_c_node_id(id: ClusterNodeId) -> CClusterNodeId {
    let mut out = CClusterNodeId { raw: 0 };
    assert!(unsafe { ghostos_cluster_node_id_from_raw(id.raw(), &mut out) });
    out
}

fn from_c_node_id(id: CClusterNodeId) -> ClusterNodeId {
    ClusterNodeId::new(id.raw).expect("valid C node id")
}

fn to_c_network_config(config: ClusterNetworkConfig) -> CClusterNetworkConfig {
    CClusterNetworkConfig {
        latency_ticks: config.latency_ticks,
        loss_percent: config.loss_percent,
        duplicate_percent: config.duplicate_percent,
        reorder: config.reorder,
        seed: config.seed,
    }
}

fn from_c_network_config(config: CClusterNetworkConfig) -> ClusterNetworkConfig {
    ClusterNetworkConfig {
        latency_ticks: config.latency_ticks,
        loss_percent: config.loss_percent,
        duplicate_percent: config.duplicate_percent,
        reorder: config.reorder,
        seed: config.seed,
    }
}

fn to_c_heartbeat(heartbeat: ClusterHeartbeat) -> CClusterHeartbeat {
    CClusterHeartbeat {
        source: to_c_node_id(heartbeat.source),
        sequence: heartbeat.sequence,
        epoch: heartbeat.epoch,
        tick: heartbeat.tick,
    }
}

fn from_c_heartbeat(heartbeat: CClusterHeartbeat) -> ClusterHeartbeat {
    ClusterHeartbeat {
        source: from_c_node_id(heartbeat.source),
        sequence: heartbeat.sequence,
        epoch: heartbeat.epoch,
        tick: heartbeat.tick,
    }
}

fn from_c_status(status: CClusterStatus) -> ClusterStatus {
    ClusterStatus {
        epoch: status.epoch,
        members: status.members,
        running: status.running,
        quorum: status.quorum,
    }
}

fn to_c_workload(workload: ClusterWorkload) -> CClusterWorkload {
    match workload {
        ClusterWorkload::Ipc => CClusterWorkload::Ipc,
        ClusterWorkload::FilesystemCommit => CClusterWorkload::FilesystemCommit,
        ClusterWorkload::MemoryFetch => CClusterWorkload::MemoryFetch,
        ClusterWorkload::Inference => CClusterWorkload::Inference,
        ClusterWorkload::MembershipChange => CClusterWorkload::MembershipChange,
    }
}

fn from_c_workload(workload: CClusterWorkload) -> ClusterWorkload {
    match workload {
        CClusterWorkload::Ipc => ClusterWorkload::Ipc,
        CClusterWorkload::FilesystemCommit => ClusterWorkload::FilesystemCommit,
        CClusterWorkload::MemoryFetch => ClusterWorkload::MemoryFetch,
        CClusterWorkload::Inference => ClusterWorkload::Inference,
        CClusterWorkload::MembershipChange => ClusterWorkload::MembershipChange,
    }
}

fn from_c_node_state(state: CClusterNodeState) -> ClusterNodeState {
    match state {
        CClusterNodeState::Running => ClusterNodeState::Running,
        CClusterNodeState::Isolated => ClusterNodeState::Isolated,
        CClusterNodeState::Failed => ClusterNodeState::Failed,
        CClusterNodeState::Stopped => ClusterNodeState::Stopped,
    }
}

fn from_c_network_outcome(outcome: CClusterNetworkOutcome) -> ClusterNetworkOutcome {
    match outcome.result {
        CClusterNetworkResult::Queued => ClusterNetworkOutcome::Queued {
            copies: outcome.copies,
            deliver_at: outcome.deliver_at,
        },
        CClusterNetworkResult::Dropped => ClusterNetworkOutcome::Dropped,
        CClusterNetworkResult::Partitioned => ClusterNetworkOutcome::Partitioned,
    }
}

fn from_c_network_trace(trace: CClusterNetworkTrace) -> ClusterNetworkTrace {
    ClusterNetworkTrace {
        tick: trace.tick,
        source: from_c_node_id(trace.source),
        target: from_c_node_id(trace.target),
        outcome: from_c_network_outcome(trace.outcome),
    }
}

fn from_c_packet(packet: CClusterPacket) -> ClusterPacket {
    ClusterPacket {
        source: from_c_node_id(packet.source),
        target: from_c_node_id(packet.target),
        payload: packet.payload[..packet.length].to_vec(),
    }
}

fn from_c_in_flight_packet(packet: CClusterPacket) -> InFlightPacket {
    InFlightPacket {
        packet: from_c_packet(packet),
        deliver_at: packet.deliver_at,
        sequence: packet.sequence,
    }
}

fn from_c_fault_record(record: CClusterFaultRecord) -> ClusterFaultRecord {
    ClusterFaultRecord {
        node: from_c_node_id(record.node),
        workload: from_c_workload(record.workload),
        recovered: record.recovered,
    }
}

fn from_c_scale_evidence(evidence: CClusterScaleEvidence) -> ClusterScaleEvidence {
    ClusterScaleEvidence {
        nodes: evidence.nodes,
        discovered_nodes: evidence.discovered_nodes,
        heartbeat_messages: evidence.heartbeat_messages,
        control_plane_traffic_bytes: evidence.control_plane_traffic_bytes,
        control_plane_memory_bytes: evidence.control_plane_memory_bytes,
        convergence_ticks: evidence.convergence_ticks,
    }
}

fn from_c_shared_device(device: CClusterSharedDevice) -> SharedMemoryDevice {
    SharedMemoryDevice {
        size: device.size,
        epoch: device.epoch,
        present: device.present,
    }
}

fn from_c_shared_mapping(mapping: CClusterSharedMapping) -> SharedMemoryMapping {
    SharedMemoryMapping {
        node: from_c_node_id(mapping.node),
        offset: mapping.offset,
        length: mapping.length,
        epoch: mapping.epoch,
    }
}

fn map_cluster_error(error: CClusterError, node: Option<ClusterNodeId>) -> Result<(), ClusterError> {
    match error {
        CClusterError::Ok => Ok(()),
        CClusterError::InvalidId | CClusterError::UnknownNode => {
            Err(ClusterError::UnknownNode(node.expect("node id required")))
        }
        CClusterError::InvalidConfig | CClusterError::Capacity => Err(ClusterError::InvalidNetworkConfig),
        CClusterError::DuplicateNode => Err(ClusterError::DuplicateNode(node.expect("node id required"))),
        CClusterError::NodeNotRunning => {
            Err(ClusterError::NodeNotRunning(node.expect("node id required")))
        }
        CClusterError::InvalidSharedSize => Err(ClusterError::InvalidSharedMemorySize),
        CClusterError::InvalidSharedRange | CClusterError::BufferTooSmall => {
            Err(ClusterError::InvalidSharedMemoryRange)
        }
        CClusterError::SharedUnavailable => Err(ClusterError::SharedMemoryUnavailable),
        CClusterError::SharedCorrupt => Err(ClusterError::CorruptSharedMemory),
        CClusterError::StaleEpoch => Err(ClusterError::StaleEpoch),
        CClusterError::NoFault => Err(ClusterError::NoFaultToRecover),
        CClusterError::VmError => Err(ClusterError::Vm(VmError::InvalidConfiguration)),
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
