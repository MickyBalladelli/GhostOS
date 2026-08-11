//! Shared, deterministic test fixtures for SynOS.
//!
//! Production crates must use this crate only from `dev-dependencies` or from
//! integration tests. It intentionally contains no production protocol or
//! device implementation.

#![deny(missing_debug_implementations)]

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;

pub mod property;
pub mod crash;
pub mod fault_matrix;
pub mod differential;

pub const BLOCK_SIZE: usize = 512;
pub const DEFAULT_SEED: u64 = 0x5359_4e4f_535f_5445;

/// Stable FNV-1a hashing. Do not replace this with a process-randomized hash
/// when the result is used in a fixture or evidence file.
pub fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

/// A deterministic source for clocks, IDs, and pseudo-random bytes.
#[derive(Clone, Debug)]
pub struct DeterministicContext {
    seed: u64,
    clock: DeterministicClock,
    entropy: DeterministicEntropy,
}

impl DeterministicContext {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            clock: DeterministicClock::new(1_700_000_000_000_000),
            entropy: DeterministicEntropy::new(seed),
        }
    }

    pub fn seed(&self) -> u64 {
        self.seed
    }

    pub fn clock(&self) -> &DeterministicClock {
        &self.clock
    }

    pub fn clock_mut(&mut self) -> &mut DeterministicClock {
        &mut self.clock
    }

    pub fn entropy(&self) -> &DeterministicEntropy {
        &self.entropy
    }

    pub fn entropy_mut(&mut self) -> &mut DeterministicEntropy {
        &mut self.entropy
    }

    pub fn derived_seed(&self, label: &str) -> u64 {
        let mut input = self.seed.to_le_bytes().to_vec();
        input.extend_from_slice(label.as_bytes());
        stable_hash(&input)
    }

    pub fn locale(&self) -> &'static str {
        "C"
    }

    pub fn cpu_count(&self) -> usize {
        1
    }

    pub fn root_path(&self) -> &'static Path {
        Path::new("/test-root")
    }
}

impl Default for DeterministicContext {
    fn default() -> Self {
        Self::new(DEFAULT_SEED)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeterministicClock {
    now_us: u64,
}

impl DeterministicClock {
    pub const fn new(start_us: u64) -> Self {
        Self { now_us: start_us }
    }

    pub const fn now_us(&self) -> u64 {
        self.now_us
    }

    pub fn set_us(&mut self, now_us: u64) {
        self.now_us = now_us;
    }

    pub fn advance_us(&mut self, delta_us: u64) {
        self.now_us = self.now_us.saturating_add(delta_us);
    }

    pub fn jump(&mut self, delta_us: i64) {
        if delta_us.is_negative() {
            self.now_us = self.now_us.saturating_sub(delta_us.unsigned_abs());
        } else {
            self.now_us = self.now_us.saturating_add(delta_us as u64);
        }
    }
}

pub trait TestClock {
    fn now_us(&self) -> u64;
}

impl TestClock for DeterministicClock {
    fn now_us(&self) -> u64 {
        self.now_us()
    }
}

impl synos_time_sync::MonotonicClock for DeterministicClock {
    fn now_us(&self) -> u64 {
        self.now_us()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeterministicEntropy {
    state: u64,
}

impl DeterministicEntropy {
    pub const fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        self.state = value;
        value.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn fill_bytes(&mut self, destination: &mut [u8]) {
        for chunk in destination.chunks_mut(8) {
            let bytes = self.next_u64().to_le_bytes();
            let length = chunk.len();
            chunk.copy_from_slice(&bytes[..length]);
        }
    }
}

/// All common deterministic fixtures used by host, VM, and integration tests.
#[derive(Clone, Debug)]
pub struct FixtureSet {
    pub context: DeterministicContext,
    pub crash: crash::CrashHarness,
    pub boot: BootInfoFixture,
    pub identity: IdentityFixture,
    pub packet: PacketFixture,
    pub disk: DiskImageFixture,
    pub volume: SynFsVolumeFixture,
    pub manifest: ManifestFixture,
    pub wire_frame: WireFrameFixture,
    pub terminal: TerminalInputFixture,
}

impl FixtureSet {
    pub fn new(seed: u64) -> Self {
        let context = DeterministicContext::new(seed);
        let node_id = NodeId::from_seed(context.derived_seed("node"));
        Self {
            context,
            crash: crash::CrashHarness::without_crash(),
            boot: BootInfoFixture::default(),
            identity: IdentityFixture::new(node_id),
            packet: PacketFixture::default(),
            disk: DiskImageFixture::new(16),
            volume: SynFsVolumeFixture::default(),
            manifest: ManifestFixture::default(),
            wire_frame: WireFrameFixture::default(),
            terminal: TerminalInputFixture::default(),
        }
    }
}

impl Default for FixtureSet {
    fn default() -> Self {
        Self::new(DEFAULT_SEED)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryRegionKind {
    Usable,
    Reserved,
    Acpi,
    Mmio,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryRegion {
    pub base: u64,
    pub length: u64,
    pub kind: MemoryRegionKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootInfoFixture {
    pub magic: [u8; 8],
    pub version: u16,
    pub memory_map: Vec<MemoryRegion>,
    pub framebuffer: Option<(u64, u32, u32, u32)>,
    pub initrd: Option<(u64, u64)>,
}

impl Default for BootInfoFixture {
    fn default() -> Self {
        Self {
            magic: *b"SYNBOOT\0",
            version: 1,
            memory_map: vec![
                MemoryRegion {
                    base: 0,
                    length: 0x9f000,
                    kind: MemoryRegionKind::Usable,
                },
                MemoryRegion {
                    base: 0x100000,
                    length: 0x3f00000,
                    kind: MemoryRegionKind::Usable,
                },
                MemoryRegion {
                    base: 0xfec00000,
                    length: 0x100000,
                    kind: MemoryRegionKind::Mmio,
                },
            ],
            framebuffer: Some((0xe000_0000, 1024, 768, 4096)),
            initrd: Some((0x0200_0000, 0x0010_0000)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NodeId(pub [u8; 16]);

impl NodeId {
    pub fn from_seed(seed: u64) -> Self {
        let mut entropy = DeterministicEntropy::new(seed);
        let mut bytes = [0u8; 16];
        entropy.fill_bytes(&mut bytes);
        Self(bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityFixture {
    pub slot: u16,
    pub generation: u32,
    pub rights: u32,
    pub owner: u64,
}

impl Default for CapabilityFixture {
    fn default() -> Self {
        Self {
            slot: 7,
            generation: 3,
            rights: 0b1111,
            owner: 42,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityFixture {
    pub node_id: NodeId,
    pub subject: u64,
    pub session: u64,
}

impl IdentityFixture {
    pub const fn new(node_id: NodeId) -> Self {
        Self {
            node_id,
            subject: 1001,
            session: 9,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PacketFixture {
    pub source: [u8; 6],
    pub destination: [u8; 6],
    pub ethertype: u16,
    pub payload: Vec<u8>,
}

impl Default for PacketFixture {
    fn default() -> Self {
        Self {
            source: [0x52, 0x54, 0x00, 0x53, 0x59, 0x01],
            destination: [0x52, 0x54, 0x00, 0x53, 0x59, 0x02],
            ethertype: 0x88b5,
            payload: b"synos-test-packet".to_vec(),
        }
    }
}

impl PacketFixture {
    pub fn encode(&self) -> Vec<u8> {
        let mut frame = Vec::with_capacity(14 + self.payload.len());
        frame.extend_from_slice(&self.destination);
        frame.extend_from_slice(&self.source);
        frame.extend_from_slice(&self.ethertype.to_be_bytes());
        frame.extend_from_slice(&self.payload);
        frame
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskImageFixture {
    sectors: Vec<[u8; BLOCK_SIZE]>,
}

impl DiskImageFixture {
    pub fn new(sectors: usize) -> Self {
        Self {
            sectors: vec![[0u8; BLOCK_SIZE]; sectors],
        }
    }

    pub fn sector_count(&self) -> usize {
        self.sectors.len()
    }

    pub fn read_sector(&self, sector: usize, destination: &mut [u8]) -> Result<(), MemoryError> {
        let source = self
            .sectors
            .get(sector)
            .ok_or(MemoryError::OutOfRange)?;
        if destination.len() < BLOCK_SIZE {
            return Err(MemoryError::BufferTooSmall {
                required: BLOCK_SIZE,
            });
        }
        destination[..BLOCK_SIZE].copy_from_slice(source);
        Ok(())
    }

    pub fn write_sector(&mut self, sector: usize, source: &[u8]) -> Result<(), MemoryError> {
        let destination = self
            .sectors
            .get_mut(sector)
            .ok_or(MemoryError::OutOfRange)?;
        if source.len() < BLOCK_SIZE {
            return Err(MemoryError::BufferTooSmall {
                required: BLOCK_SIZE,
            });
        }
        destination.copy_from_slice(&source[..BLOCK_SIZE]);
        Ok(())
    }

    pub fn fill_pattern(&mut self) {
        for (index, sector) in self.sectors.iter_mut().enumerate() {
            sector.fill((index as u8).wrapping_mul(17));
        }
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.sectors.iter().flat_map(|sector| sector.iter()).copied().collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SynFsVolumeFixture {
    pub generation: u64,
    pub files: BTreeMap<String, Vec<u8>>,
}

impl Default for SynFsVolumeFixture {
    fn default() -> Self {
        let mut files = BTreeMap::new();
        files.insert("/README".into(), b"SynOS test volume\n".to_vec());
        Self {
            generation: 1,
            files,
        }
    }
}

impl SynFsVolumeFixture {
    pub fn write(&mut self, path: &str, contents: &[u8]) {
        self.files.insert(path.into(), contents.to_vec());
        self.generation = self.generation.saturating_add(1);
    }

    pub fn read(&self, path: &str) -> Option<&[u8]> {
        self.files.get(path).map(Vec::as_slice)
    }

    pub fn snapshot(&self) -> Self {
        self.clone()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestFixture {
    pub name: String,
    pub version: String,
    pub capabilities: Vec<String>,
}

impl Default for ManifestFixture {
    fn default() -> Self {
        Self {
            name: "fixture-app".into(),
            version: "1.0.0".into(),
            capabilities: vec!["read:/data".into(), "clock".into()],
        }
    }
}

impl ManifestFixture {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut capabilities = self.capabilities.clone();
        capabilities.sort();
        format!(
            "name={}\nversion={}\ncapabilities={}\n",
            self.name,
            self.version,
            capabilities.join(",")
        )
        .into_bytes()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireFrameFixture {
    pub version: u8,
    pub kind: u8,
    pub request_id: u64,
    pub payload: Vec<u8>,
}

impl Default for WireFrameFixture {
    fn default() -> Self {
        Self {
            version: 1,
            kind: 2,
            request_id: 17,
            payload: b"fixture-frame".to_vec(),
        }
    }
}

impl WireFrameFixture {
    pub fn encode(&self) -> Vec<u8> {
        let mut frame = vec![self.version, self.kind];
        frame.extend_from_slice(&self.request_id.to_le_bytes());
        frame.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(&self.payload);
        frame
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalInputFixture {
    pub bytes: Vec<u8>,
}

impl Default for TerminalInputFixture {
    fn default() -> Self {
        Self {
            bytes: b"DIRECTORY /data\nTYPE note\n\x1b[A\x7f\n".to_vec(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FaultPoint {
    BlockRead,
    BlockWrite,
    Flush,
    DeviceRead,
    DeviceWrite,
    DeviceDma,
    DeviceDescriptor,
    DeviceInterrupt,
    DeviceReset,
    DeviceRemoval,
    NetworkSend,
    IpcSend,
    Interrupt,
    CapabilityCheck,
    MetadataRead,
    Allocation,
    ClockRead,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    TornWrite { bytes: usize },
    ShortBuffer { available: usize },
    ShortIo { bytes: usize },
    DmaOverrun { bytes: usize },
    InvalidDescriptor,
    DropPacket,
    DuplicatePacket,
    DelayedInterrupt { ticks: u32 },
    DroppedInterrupt,
    ResetDuringIo,
    DeviceRemoved,
    StaleCapability,
    NodeLoss { node: u16 },
    CorruptMetadata { offset: usize },
    AllocationFailure,
    ClockJump { delta_us: i64 },
}

#[derive(Clone, Debug, Default)]
pub struct FaultPlan {
    actions: VecDeque<(FaultPoint, Fault)>,
}

impl FaultPlan {
    pub fn push(&mut self, point: FaultPoint, fault: Fault) {
        self.actions.push_back((point, fault));
    }

    pub fn once(point: FaultPoint, fault: Fault) -> Self {
        let mut plan = Self::default();
        plan.push(point, fault);
        plan
    }

    pub fn take(&mut self, point: &FaultPoint) -> Option<Fault> {
        let index = self.actions.iter().position(|(candidate, _)| candidate == point)?;
        self.actions.remove(index).map(|(_, fault)| fault)
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    pub fn pending(&self) -> usize {
        self.actions.len()
    }
}

/// Boundary-oriented adapter for consuming a [`FaultPlan`] in a test double
/// or a hand-written fake. It gives the less common faults explicit behavior
/// instead of making every test know how to interpret the enum.
#[derive(Clone, Debug, Default)]
pub struct FailureInjector {
    plan: FaultPlan,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FakeDeviceError {
    ShortIo { bytes: usize },
    DmaOverrun { bytes: usize },
    InvalidDescriptor,
    InterruptDropped,
    ResetDuringIo,
    Removed,
}

impl fmt::Display for FakeDeviceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for FakeDeviceError {}

/// Small register/DMA/interrupt fake for device-boundary tests.
///
/// It is deliberately deterministic and has no host I/O. Faults are consumed
/// once through the same [`FaultPlan`] used by the other test doubles.
#[derive(Clone, Debug, Default)]
pub struct FakeDevice {
    registers: BTreeMap<u64, u64>,
    faults: FaultPlan,
    pending_interrupts: u32,
    io_in_progress: bool,
    removed: bool,
}

impl FakeDevice {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn faults_mut(&mut self) -> &mut FaultPlan {
        &mut self.faults
    }

    pub fn read_register(&mut self, address: u64) -> Result<u64, FakeDeviceError> {
        self.ensure_present()?;
        if let Some(Fault::ShortIo { bytes }) = self.faults.take(&FaultPoint::DeviceRead) {
            return Err(FakeDeviceError::ShortIo { bytes })
        }
        Ok(self.registers.get(&address).copied().unwrap_or(0))
    }

    pub fn write_register(
        &mut self,
        address: u64,
        value: u64,
    ) -> Result<(), FakeDeviceError> {
        self.ensure_present()?;
        if let Some(Fault::ShortIo { bytes }) = self.faults.take(&FaultPoint::DeviceWrite) {
            return Err(FakeDeviceError::ShortIo { bytes })
        }
        self.registers.insert(address, value);
        Ok(())
    }

    pub fn dma(&mut self, descriptor_count: usize) -> Result<usize, FakeDeviceError> {
        self.ensure_present()?;
        if self.faults.take(&FaultPoint::DeviceDescriptor).is_some() {
            return Err(FakeDeviceError::InvalidDescriptor)
        }
        if let Some(Fault::DmaOverrun { bytes }) = self.faults.take(&FaultPoint::DeviceDma) {
            return Err(FakeDeviceError::DmaOverrun { bytes })
        }
        self.io_in_progress = true;
        Ok(descriptor_count)
    }

    pub fn finish_io(&mut self) -> Result<(), FakeDeviceError> {
        self.ensure_present()?;
        if self.faults.take(&FaultPoint::DeviceReset).is_some() {
            self.io_in_progress = false;
            return Err(FakeDeviceError::ResetDuringIo)
        }
        self.io_in_progress = false;
        Ok(())
    }

    pub fn raise_interrupt(&mut self) -> Result<(), FakeDeviceError> {
        self.ensure_present()?;
        if self.faults.take(&FaultPoint::DeviceInterrupt).is_some() {
            return Err(FakeDeviceError::InterruptDropped)
        }
        self.pending_interrupts = self.pending_interrupts.saturating_add(1);
        Ok(())
    }

    pub fn take_interrupt(&mut self) -> bool {
        if self.pending_interrupts == 0 {
            return false
        }
        self.pending_interrupts -= 1;
        true
    }

    pub fn reset(&mut self) {
        self.registers.clear();
        self.pending_interrupts = 0;
        self.io_in_progress = false;
    }

    pub fn remove(&mut self) {
        self.removed = true;
    }

    pub fn is_removed(&self) -> bool {
        self.removed
    }

    pub fn io_in_progress(&self) -> bool {
        self.io_in_progress
    }

    fn ensure_present(&mut self) -> Result<(), FakeDeviceError> {
        if self.removed || self.faults.take(&FaultPoint::DeviceRemoval).is_some() {
            self.removed = true;
            return Err(FakeDeviceError::Removed)
        }
        Ok(())
    }
}

impl FailureInjector {
    pub fn new(plan: FaultPlan) -> Self {
        Self { plan }
    }

    pub fn plan(&self) -> &FaultPlan {
        &self.plan
    }

    pub fn plan_mut(&mut self) -> &mut FaultPlan {
        &mut self.plan
    }

    pub fn check(&mut self, point: FaultPoint) -> Result<(), Fault> {
        self.plan.take(&point).map_or(Ok(()), Err)
    }

    pub fn check_capability(&mut self, generation: u32) -> Result<u32, Fault> {
        match self.plan.take(&FaultPoint::CapabilityCheck) {
            Some(Fault::StaleCapability) => Err(Fault::StaleCapability),
            Some(fault) => Err(fault),
            None => Ok(generation),
        }
    }

    pub fn interrupt_delay(&mut self) -> Result<u32, Fault> {
        match self.plan.take(&FaultPoint::Interrupt) {
            Some(Fault::DelayedInterrupt { ticks }) => Ok(ticks),
            Some(fault) => Err(fault),
            None => Ok(0),
        }
    }

    pub fn apply_clock_jump(&mut self, clock: &mut DeterministicClock) -> Result<(), Fault> {
        match self.plan.take(&FaultPoint::ClockRead) {
            Some(Fault::ClockJump { delta_us }) => {
                clock.jump(delta_us);
                Err(Fault::ClockJump { delta_us })
            }
            Some(fault) => Err(fault),
            None => Ok(()),
        }
    }

    pub fn corrupt_metadata(&mut self, bytes: &mut [u8]) -> Result<(), Fault> {
        match self.plan.take(&FaultPoint::MetadataRead) {
            Some(Fault::CorruptMetadata { offset }) => {
                if let Some(byte) = bytes.get_mut(offset) {
                    *byte ^= 0xff;
                }
                Err(Fault::CorruptMetadata { offset })
            }
            Some(fault) => Err(fault),
            None => Ok(()),
        }
    }

    pub fn allocation(&mut self) -> Result<(), Fault> {
        match self.plan.take(&FaultPoint::Allocation) {
            Some(Fault::AllocationFailure) => Err(Fault::AllocationFailure),
            Some(fault) => Err(fault),
            None => Ok(()),
        }
    }

    pub fn node_loss(&mut self) -> Result<Option<u16>, Fault> {
        match self.plan.take(&FaultPoint::NetworkSend) {
            Some(Fault::NodeLoss { node }) => Ok(Some(node)),
            Some(fault) => Err(fault),
            None => Ok(None),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryError {
    OutOfRange,
    BufferTooSmall { required: usize },
    TornWrite { bytes: usize },
    FlushFailed,
    AllocationFailed,
}

impl fmt::Display for MemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for MemoryError {}

pub trait BlockDevice {
    fn block_count(&self) -> usize;
    fn read_block(&mut self, block: usize, destination: &mut [u8]) -> Result<(), MemoryError>;
    fn write_block(&mut self, block: usize, source: &[u8]) -> Result<(), MemoryError>;
    fn flush(&mut self) -> Result<(), MemoryError>;
    fn discard(&mut self, block: usize) -> Result<(), MemoryError>;
}

#[derive(Clone, Debug)]
pub struct MemoryBlockDevice {
    image: DiskImageFixture,
    faults: FaultPlan,
    pub operations: Vec<String>,
}

impl MemoryBlockDevice {
    pub fn new(blocks: usize) -> Self {
        Self {
            image: DiskImageFixture::new(blocks),
            faults: FaultPlan::default(),
            operations: Vec::new(),
        }
    }

    pub fn with_faults(blocks: usize, faults: FaultPlan) -> Self {
        Self {
            image: DiskImageFixture::new(blocks),
            faults,
            operations: Vec::new(),
        }
    }

    pub fn image(&self) -> &DiskImageFixture {
        &self.image
    }

    pub fn image_mut(&mut self) -> &mut DiskImageFixture {
        &mut self.image
    }

    pub fn faults_mut(&mut self) -> &mut FaultPlan {
        &mut self.faults
    }

    fn check_fault(&mut self, point: FaultPoint) -> Result<Option<Fault>, MemoryError> {
        let Some(fault) = self.faults.take(&point) else {
            return Ok(None);
        };
        match fault {
            Fault::AllocationFailure => Err(MemoryError::AllocationFailed),
            Fault::TornWrite { bytes } if point == FaultPoint::BlockWrite => {
                Ok(Some(Fault::TornWrite { bytes }))
            }
            _ => Ok(Some(fault)),
        }
    }
}

impl BlockDevice for MemoryBlockDevice {
    fn block_count(&self) -> usize {
        self.image.sector_count()
    }

    fn read_block(&mut self, block: usize, destination: &mut [u8]) -> Result<(), MemoryError> {
        if let Some(Fault::ShortBuffer { available }) = self.check_fault(FaultPoint::BlockRead)? {
            if destination.len() < available {
                return Err(MemoryError::BufferTooSmall { required: available });
            }
            self.operations.push(format!("read-short:{block}:{available}"));
        }
        self.operations.push(format!("read:{block}"));
        self.image.read_sector(block, destination)
    }

    fn write_block(&mut self, block: usize, source: &[u8]) -> Result<(), MemoryError> {
        if let Some(Fault::TornWrite { bytes }) = self.check_fault(FaultPoint::BlockWrite)? {
            let mut partial = [0u8; BLOCK_SIZE];
            let count = bytes.min(BLOCK_SIZE).min(source.len());
            partial[..count].copy_from_slice(&source[..count]);
            self.image.write_sector(block, &partial)?;
            self.operations.push(format!("write-torn:{block}:{count}"));
            return Err(MemoryError::TornWrite { bytes: count });
        }
        self.operations.push(format!("write:{block}"));
        self.image.write_sector(block, source)
    }

    fn flush(&mut self) -> Result<(), MemoryError> {
        if matches!(self.check_fault(FaultPoint::Flush)?, Some(Fault::DropPacket)) {
            return Err(MemoryError::FlushFailed);
        }
        self.operations.push("flush".into());
        Ok(())
    }

    fn discard(&mut self, block: usize) -> Result<(), MemoryError> {
        self.image.write_sector(block, &[0u8; BLOCK_SIZE])?;
        self.operations.push(format!("discard:{block}"));
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkDelivery {
    pub source: u16,
    pub destination: u16,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkError {
    NodeOffline(u16),
    Dropped,
    Delayed,
}

impl fmt::Display for NetworkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for NetworkError {}

#[derive(Clone, Debug)]
pub struct MemoryNetwork {
    nodes: BTreeMap<u16, bool>,
    queue: VecDeque<NetworkDelivery>,
    faults: FaultPlan,
}

impl MemoryNetwork {
    pub fn new(node_count: u16) -> Self {
        let nodes = (0..node_count).map(|node| (node, true)).collect();
        Self {
            nodes,
            queue: VecDeque::new(),
            faults: FaultPlan::default(),
        }
    }

    pub fn faults_mut(&mut self) -> &mut FaultPlan {
        &mut self.faults
    }

    pub fn set_node_online(&mut self, node: u16, online: bool) {
        self.nodes.insert(node, online);
    }

    pub fn send(
        &mut self,
        source: u16,
        destination: u16,
        payload: &[u8],
    ) -> Result<(), NetworkError> {
        if !self.nodes.get(&source).copied().unwrap_or(false) {
            return Err(NetworkError::NodeOffline(source));
        }
        if !self.nodes.get(&destination).copied().unwrap_or(false) {
            return Err(NetworkError::NodeOffline(destination));
        }
        if let Some(Fault::NodeLoss { node }) = self.faults.take(&FaultPoint::NetworkSend) {
            self.set_node_online(node, false);
            return Err(NetworkError::NodeOffline(node));
        }
        if matches!(self.faults.take(&FaultPoint::NetworkSend), Some(Fault::DropPacket)) {
            return Err(NetworkError::Dropped);
        }
        let delivery = NetworkDelivery {
            source,
            destination,
            payload: payload.to_vec(),
        };
        let duplicate = matches!(
            self.faults.take(&FaultPoint::NetworkSend),
            Some(Fault::DuplicatePacket)
        );
        self.queue.push_back(delivery.clone());
        if duplicate {
            self.queue.push_back(delivery);
        }
        Ok(())
    }

    pub fn receive(&mut self, destination: u16) -> Option<NetworkDelivery> {
        let index = self
            .queue
            .iter()
            .position(|delivery| delivery.destination == destination)?;
        self.queue.remove(index)
    }

    pub fn queued(&self) -> usize {
        self.queue.len()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IpcError {
    Full,
    Dropped,
    Duplicate,
}

impl fmt::Display for IpcError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for IpcError {}

#[derive(Clone, Debug)]
pub struct MemoryIpcChannel {
    capacity: usize,
    queue: VecDeque<Vec<u8>>,
    faults: FaultPlan,
}

impl MemoryIpcChannel {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            queue: VecDeque::new(),
            faults: FaultPlan::default(),
        }
    }

    pub fn faults_mut(&mut self) -> &mut FaultPlan {
        &mut self.faults
    }

    pub fn send(&mut self, message: &[u8]) -> Result<(), IpcError> {
        if matches!(self.faults.take(&FaultPoint::IpcSend), Some(Fault::DropPacket)) {
            return Err(IpcError::Dropped);
        }
        if self.queue.len() >= self.capacity {
            return Err(IpcError::Full);
        }
        self.queue.push_back(message.to_vec());
        if matches!(self.faults.take(&FaultPoint::IpcSend), Some(Fault::DuplicatePacket))
            && self.queue.len() < self.capacity
        {
            self.queue.push_back(message.to_vec());
        }
        Ok(())
    }

    pub fn receive(&mut self) -> Option<Vec<u8>> {
        self.queue.pop_front()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageError {
    NotFound,
    AllocationFailed,
    CorruptMetadata,
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for StorageError {}

#[derive(Clone, Debug, Default)]
pub struct MemoryStorage {
    entries: BTreeMap<String, Vec<u8>>,
    faults: FaultPlan,
}

impl MemoryStorage {
    pub fn faults_mut(&mut self) -> &mut FaultPlan {
        &mut self.faults
    }

    pub fn write(&mut self, key: &str, value: &[u8]) -> Result<(), StorageError> {
        if matches!(self.faults.take(&FaultPoint::Allocation), Some(Fault::AllocationFailure)) {
            return Err(StorageError::AllocationFailed);
        }
        self.entries.insert(key.into(), value.to_vec());
        Ok(())
    }

    pub fn read(&mut self, key: &str) -> Result<Vec<u8>, StorageError> {
        if matches!(
            self.faults.take(&FaultPoint::MetadataRead),
            Some(Fault::CorruptMetadata { .. })
        ) {
            return Err(StorageError::CorruptMetadata);
        }
        self.entries.get(key).cloned().ok_or(StorageError::NotFound)
    }

    pub fn delete(&mut self, key: &str) -> Result<(), StorageError> {
        self.entries.remove(key).map(|_| ()).ok_or(StorageError::NotFound)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttestationEvidence {
    pub subject: u64,
    pub measurement: [u8; 32],
    pub nonce: [u8; 16],
    pub signature: [u8; 32],
}

#[derive(Clone, Debug)]
pub struct MemoryAttestation {
    key: [u8; 32],
    entropy: DeterministicEntropy,
}

impl MemoryAttestation {
    pub fn new(seed: u64) -> Self {
        let mut entropy = DeterministicEntropy::new(seed);
        let mut key = [0u8; 32];
        entropy.fill_bytes(&mut key);
        Self { key, entropy }
    }

    pub fn quote(&mut self, subject: u64, measurement: &[u8]) -> AttestationEvidence {
        let mut digest_input = self.key.to_vec();
        digest_input.extend_from_slice(&subject.to_le_bytes());
        digest_input.extend_from_slice(measurement);
        let digest = stable_hash(&digest_input);
        let mut measured = [0u8; 32];
        let mut signature = [0u8; 32];
        for (index, bytes) in measured.chunks_mut(8).enumerate() {
            bytes.copy_from_slice(&digest.wrapping_add(index as u64).to_le_bytes());
        }
        self.entropy.fill_bytes(&mut signature);
        AttestationEvidence {
            subject,
            measurement: measured,
            nonce: {
                let mut nonce = [0u8; 16];
                self.entropy.fill_bytes(&mut nonce);
                nonce
            },
            signature,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceleratorJob {
    pub id: u64,
    pub input: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcceleratorError {
    QueueFull,
    AllocationFailed,
}

impl fmt::Display for AcceleratorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for AcceleratorError {}

#[derive(Clone, Debug)]
pub struct MemoryAccelerator {
    capacity: usize,
    next_id: u64,
    jobs: VecDeque<AcceleratorJob>,
    faults: FaultPlan,
}

impl MemoryAccelerator {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            next_id: 1,
            jobs: VecDeque::new(),
            faults: FaultPlan::default(),
        }
    }

    pub fn faults_mut(&mut self) -> &mut FaultPlan {
        &mut self.faults
    }

    pub fn submit(&mut self, input: &[u8]) -> Result<u64, AcceleratorError> {
        if matches!(self.faults.take(&FaultPoint::Allocation), Some(Fault::AllocationFailure)) {
            return Err(AcceleratorError::AllocationFailed);
        }
        if self.jobs.len() >= self.capacity {
            return Err(AcceleratorError::QueueFull);
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.jobs.push_back(AcceleratorJob {
            id,
            input: input.to_vec(),
        });
        Ok(id)
    }

    pub fn complete_next(&mut self) -> Option<(u64, Vec<u8>)> {
        let job = self.jobs.pop_front()?;
        Some((job.id, job.input))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupError {
    pub failures: Vec<String>,
}

impl fmt::Display for CleanupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "cleanup failed: {} action(s)", self.failures.len())
    }
}

impl std::error::Error for CleanupError {}

/// LIFO cleanup actions for files, sockets, processes, terminal modes, and
/// QEMU instances. Drop performs best-effort cleanup after a panic.
#[derive(Default)]
pub struct CleanupGuard {
    actions: Vec<(String, Box<dyn FnMut() -> Result<(), String>>)>,
    cleaned: bool,
}

impl fmt::Debug for CleanupGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CleanupGuard")
            .field("pending_actions", &self.actions.len())
            .field("cleaned", &self.cleaned)
            .finish()
    }
}

impl CleanupGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn defer<F>(&mut self, name: impl Into<String>, action: F)
    where
        F: FnMut() -> Result<(), String> + 'static,
    {
        self.actions.push((name.into(), Box::new(action)));
    }

    pub fn register_file(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        self.defer(format!("file:{}", path.display()), move || {
            if path.is_file() {
                fs::remove_file(&path).map_err(|error| error.to_string())?;
            }
            Ok(())
        });
    }

    pub fn register_directory(&mut self, path: impl Into<PathBuf>) {
        let path = path.into();
        self.defer(format!("directory:{}", path.display()), move || {
            if path.exists() {
                fs::remove_dir_all(&path).map_err(|error| error.to_string())?;
            }
            Ok(())
        });
    }

    pub fn register_process(&mut self, mut child: Child) {
        self.defer("process", move || {
            let _ = child.kill();
            let _ = child.wait();
            Ok(())
        });
    }

    pub fn register_socket(&mut self, path: impl Into<PathBuf>) {
        self.register_file(path);
    }

    pub fn register_qemu(&mut self, child: Child) {
        self.register_process(child);
    }

    pub fn register_terminal_restore<F>(&mut self, restore: F)
    where
        F: FnMut() -> Result<(), String> + 'static,
    {
        self.defer("terminal-restore", restore);
    }

    pub fn cleanup(&mut self) -> Result<(), CleanupError> {
        if self.cleaned {
            return Ok(());
        }
        self.cleaned = true;
        let mut failures = Vec::new();
        while let Some((name, mut action)) = self.actions.pop() {
            if let Err(error) = action() {
                failures.push(format!("{name}: {error}"));
            }
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(CleanupError { failures })
        }
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

/// A deterministic test scope combining fixtures and cleanup.
#[derive(Debug)]
pub struct TestScope {
    pub fixtures: FixtureSet,
    pub cleanup: CleanupGuard,
}

impl TestScope {
    pub fn new(seed: u64) -> Self {
        Self {
            fixtures: FixtureSet::new(seed),
            cleanup: CleanupGuard::new(),
        }
    }
}

impl Default for TestScope {
    fn default() -> Self {
        Self::new(DEFAULT_SEED)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoldenFixture {
    BootImage,
    ProtocolFrame,
    FilesystemBlock,
    Snapshot,
    VmDecodedInstruction,
    VmFirmwareTables,
    VmBootHandoff,
    VmDeviceRegisters,
    VmSnapshot,
    AuditRecord,
    PackageSignature,
    TerminalOutput,
    VmSerialOutput,
}

pub fn golden_text(fixture: GoldenFixture) -> &'static str {
    match fixture {
        GoldenFixture::BootImage => include_str!("../golden/boot-image.hex"),
        GoldenFixture::ProtocolFrame => include_str!("../golden/protocol-frame.hex"),
        GoldenFixture::FilesystemBlock => include_str!("../golden/filesystem-block.hex"),
        GoldenFixture::Snapshot => include_str!("../golden/snapshot.hex"),
        GoldenFixture::VmDecodedInstruction => {
            include_str!("../golden/vm-decoded-instruction.hex")
        }
        GoldenFixture::VmFirmwareTables => include_str!("../golden/vm-firmware-tables.hex"),
        GoldenFixture::VmBootHandoff => include_str!("../golden/vm-boot-handoff.hex"),
        GoldenFixture::VmDeviceRegisters => include_str!("../golden/vm-device-registers.hex"),
        GoldenFixture::VmSnapshot => include_str!("../golden/vm-snapshot.hex"),
        GoldenFixture::AuditRecord => include_str!("../golden/audit-record.json"),
        GoldenFixture::PackageSignature => include_str!("../golden/package-signature.hex"),
        GoldenFixture::TerminalOutput => include_str!("../golden/terminal-output.txt"),
        GoldenFixture::VmSerialOutput => include_str!("../golden/vm-serial-output.txt"),
    }
}

pub fn golden_bytes(fixture: GoldenFixture) -> Result<Vec<u8>, HexError> {
    decode_hex(golden_text(fixture))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HexError {
    OddLength,
    InvalidDigit(u8),
}

impl fmt::Display for HexError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for HexError {}

pub fn decode_hex(input: &str) -> Result<Vec<u8>, HexError> {
    let digits: Vec<u8> = input
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if digits.len() % 2 != 0 {
        return Err(HexError::OddLength);
    }
    digits
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_digit(pair[0])?;
            let low = hex_digit(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn hex_digit(byte: u8) -> Result<u8, HexError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        other => Err(HexError::InvalidDigit(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_produces_same_fixture_bytes() {
        let left = FixtureSet::new(7);
        let right = FixtureSet::new(7);
        assert_eq!(left.identity, right.identity);
        assert_eq!(left.packet.encode(), right.packet.encode());
        assert_eq!(left.manifest.canonical_bytes(), right.manifest.canonical_bytes());
    }

    #[test]
    fn fault_plan_consumes_one_shot_faults() {
        let mut plan = FaultPlan::once(FaultPoint::NetworkSend, Fault::DropPacket);
        assert!(plan.take(&FaultPoint::NetworkSend).is_some());
        assert!(plan.take(&FaultPoint::NetworkSend).is_none());
    }

    #[test]
    fn cleanup_runs_in_reverse_registration_order() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let order = Rc::new(RefCell::new(Vec::new()));
        let mut guard = CleanupGuard::new();
        for label in ["first", "second"] {
            let order = order.clone();
            guard.defer(label, move || {
                order.borrow_mut().push(label);
                Ok(())
            });
        }
        guard.cleanup().expect("cleanup succeeds");
        assert_eq!(&*order.borrow(), &["second", "first"]);
    }

    #[test]
    fn golden_hex_is_decodable() {
        assert!(!golden_bytes(GoldenFixture::BootImage)
            .expect("boot fixture is valid")
            .is_empty());
        assert!(!golden_bytes(GoldenFixture::ProtocolFrame)
            .expect("protocol fixture is valid")
            .is_empty());
    }
}
