#![no_std]
#![deny(unsafe_code)]

//! Hardware reliability, availability, and serviceability primitives.
//!
//! The crate keeps policy in Ring 3-friendly, fixed-capacity structures. A
//! platform adapter supplies the actual EDAC, CXL, PCIe, power, and persistent
//! memory register access.

use ghostos_fabric::{AddressRange, NodeId, PAGE_SIZE, memory::PoolId};
use ghostos_legacy_pc_drivers::{PciAddress, PcieAerStatus};
use ghostos_observability::{EventField, EventKind, Level, TraceEvent, emit, field};
use ghostos_status::{IntoStatus, Status};
use ghostos_ghostfs::{DeviceHealth, StorageDeviceId, StoragePoolAdmin, StoragePoolError};

#[allow(unsafe_code)]
mod native;

pub const DEFAULT_EVENT_CAPACITY: usize = 256;
pub const DEFAULT_POISON_CAPACITY: usize = 128;
pub const DEFAULT_WORKLOAD_CAPACITY: usize = 32;
pub const DEFAULT_DIRTY_PAGE_CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RasError {
    Capacity,
    InvalidArgument,
    AlreadyTracked,
    NotFound,
    PoisonedMemory,
    FlushFailed,
    Controller(Status),
    Storage(StoragePoolError),
}

impl IntoStatus for RasError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::InvalidArgument => Status::INVALID_ARGUMENT,
            Self::AlreadyTracked => Status::ALREADY_EXISTS,
            Self::NotFound => Status::NOT_FOUND,
            Self::PoisonedMemory | Self::FlushFailed => Status::CORRUPT,
            Self::Controller(status) => status,
            Self::Storage(error) => error.status(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FaultSeverity {
    Corrected = 0,
    Warning = 1,
    Error = 2,
    Fatal = 3,
}

impl FaultSeverity {
    const fn level(self) -> Level {
        match self {
            Self::Corrected => Level::Info,
            Self::Warning => Level::Warn,
            Self::Error => Level::Error,
            Self::Fatal => Level::Critical,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FaultSource {
    EccMemory,
    Cxl,
    PcieAer,
    Thermal,
    Power,
    PersistentMemory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HardwareEvent {
    pub sequence: u64,
    pub timestamp_us: u64,
    pub node: NodeId,
    pub source: FaultSource,
    pub severity: FaultSeverity,
    pub device: u64,
    pub address: u64,
    pub detail: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
#[repr(C)]
pub struct ErrorCounters {
    pub corrected_ecc: u64,
    pub uncorrected_ecc: u64,
    pub cxl_poisoned_flits: u64,
    pub aer_correctable: u64,
    pub aer_non_fatal: u64,
    pub aer_fatal: u64,
}

/// Bounded newest-first hardware event history. Older events are overwritten
/// so an interrupt path never waits for a consumer.
pub struct ErrorTelemetry<const CAPACITY: usize = DEFAULT_EVENT_CAPACITY> {
    events: [native::EventSlot; CAPACITY],
    state: native::Telemetry,
}

impl<const CAPACITY: usize> ErrorTelemetry<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY > 0);
        Self { events: [native::EventSlot::EMPTY; CAPACITY], state: native::Telemetry::EMPTY }
    }

    pub fn record(&mut self, mut event: HardwareEvent) -> u64 {
        event.sequence = self.state.record(&mut self.events, event);
        emit(
            TraceEvent::new(event.severity.level(), EventKind::Kernel)
                .at(event.timestamp_us)
                .on_node(event.node.raw())
                .with_field(EventField::unsigned(field::ADDRESS, event.address))
                .with_field(EventField::unsigned(field::STATUS, event.severity as u64))
                .with_field(EventField::unsigned(field::OBJECT, event.device)),
        );
        event.sequence
    }

    pub fn record_ecc(
        &mut self,
        timestamp_us: u64,
        node: NodeId,
        device: u64,
        address: u64,
        corrected: bool,
        syndrome: u32,
    ) -> u64 {
        self.record(HardwareEvent {
            sequence: 0,
            timestamp_us,
            node,
            source: FaultSource::EccMemory,
            severity: if corrected {
                FaultSeverity::Corrected
            } else {
                FaultSeverity::Fatal
            },
            device,
            address,
            detail: syndrome,
        })
    }

    pub fn record_cxl_poison(
        &mut self,
        timestamp_us: u64,
        node: NodeId,
        device: u64,
        address: u64,
        flit_status: u32,
    ) -> u64 {
        self.record(HardwareEvent {
            sequence: 0,
            timestamp_us,
            node,
            source: FaultSource::Cxl,
            severity: FaultSeverity::Fatal,
            device,
            address,
            detail: flit_status,
        })
    }

    pub fn record_aer(
        &mut self,
        timestamp_us: u64,
        node: NodeId,
        address: PciAddress,
        status: PcieAerStatus,
    ) -> u64 {
        let severity = if status.fatal != 0 {
            FaultSeverity::Fatal
        } else if status.non_fatal != 0 {
            FaultSeverity::Error
        } else {
            FaultSeverity::Corrected
        };
        self.record(HardwareEvent {
            sequence: 0,
            timestamp_us,
            node,
            source: FaultSource::PcieAer,
            severity,
            device: ((address.bus as u64) << 16)
                | ((address.device as u64) << 8)
                | address.function as u64,
            address: 0,
            detail: status.correctable ^ status.non_fatal ^ status.fatal,
        })
    }

    pub fn events(&self) -> impl Iterator<Item = HardwareEvent> + '_ {
        (0..self.state.count).filter_map(move |offset| self.state.event(&self.events, offset))
    }

    pub const fn dropped(&self) -> u64 {
        self.state.dropped
    }

    pub const fn counters(&self) -> ErrorCounters {
        self.state.counters
    }
}

impl<const CAPACITY: usize> Default for ErrorTelemetry<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PoisonedRange {
    pub node: NodeId,
    pub device: u64,
    pub range: AddressRange,
    pub sequence: u64,
}

/// Quarantined CXL/ECC ranges. Callers must check admission before copying a
/// page into DSM or handing it to a workload.
pub struct PoisonTracker<const CAPACITY: usize = DEFAULT_POISON_CAPACITY> {
    ranges: [native::Poison; CAPACITY],
}

impl<const CAPACITY: usize> PoisonTracker<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY > 0);
        Self {
            ranges: [native::Poison::EMPTY; CAPACITY],
        }
    }

    pub fn quarantine(&mut self, range: PoisonedRange) -> Result<(), RasError> {
        match native::quarantine(&mut self.ranges, range) {
            0 => Ok(()),
            1 => Err(RasError::InvalidArgument),
            2 => Err(RasError::AlreadyTracked),
            3 => Err(RasError::Capacity),
            -1 => panic!("attempt to add with overflow"),
            _ => unreachable!("native RAS quarantine result"),
        }
    }

    pub fn admit(&self, node: NodeId, range: AddressRange) -> Result<(), RasError> {
        match native::admit(&self.ranges, node, range) {
            0 => Ok(()),
            1 => Err(RasError::PoisonedMemory),
            -1 => panic!("attempt to add with overflow"),
            _ => unreachable!("native RAS admission result"),
        }
    }

    pub fn ranges(&self) -> impl Iterator<Item = PoisonedRange> + '_ {
        self.ranges.iter().filter_map(|range| range.to_public())
    }
}

impl<const CAPACITY: usize> Default for PoisonTracker<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct HardwareDiagnostics<
    const EVENTS: usize = DEFAULT_EVENT_CAPACITY,
    const POISON: usize = DEFAULT_POISON_CAPACITY,
> {
    pub telemetry: ErrorTelemetry<EVENTS>,
    pub poison: PoisonTracker<POISON>,
}

impl<const EVENTS: usize, const POISON: usize> HardwareDiagnostics<EVENTS, POISON> {
    pub const fn new() -> Self {
        Self {
            telemetry: ErrorTelemetry::new(),
            poison: PoisonTracker::new(),
        }
    }

    pub fn record_cxl_poison(
        &mut self,
        timestamp_us: u64,
        node: NodeId,
        device: u64,
        range: AddressRange,
        flit_status: u32,
    ) -> Result<u64, RasError> {
        let sequence =
            self.telemetry
                .record_cxl_poison(timestamp_us, node, device, range.start, flit_status);
        self.poison.quarantine(PoisonedRange {
            node,
            device,
            range,
            sequence,
        })?;
        Ok(sequence)
    }

    pub fn record_ecc(
        &mut self,
        timestamp_us: u64,
        node: NodeId,
        device: u64,
        address: u64,
        corrected: bool,
        syndrome: u32,
    ) -> Result<u64, RasError> {
        let sequence =
            self.telemetry
                .record_ecc(timestamp_us, node, device, address, corrected, syndrome);
        if !corrected {
            let page = address & !(PAGE_SIZE - 1);
            let range =
                AddressRange::new(page, PAGE_SIZE).map_err(|_| RasError::InvalidArgument)?;
            self.poison.quarantine(PoisonedRange {
                node,
                device,
                range,
                sequence,
            })?;
        }
        Ok(sequence)
    }
}

impl<const EVENTS: usize, const POISON: usize> Default for HardwareDiagnostics<EVENTS, POISON> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AerIsolation {
    pub address: PciAddress,
    pub status: PcieAerStatus,
}

pub trait PciSegmentController {
    fn isolate_segment(&mut self, bus: u8) -> Result<(), Status>;
}

/// Converts AER status into a service action and isolates the complete bus
/// segment for uncorrectable errors before more DMA can spread corruption.
pub fn handle_aer<C, const EVENTS: usize, const POISON: usize>(
    diagnostics: &mut HardwareDiagnostics<EVENTS, POISON>,
    controller: &mut C,
    timestamp_us: u64,
    node: NodeId,
    address: PciAddress,
    status: PcieAerStatus,
) -> Result<Option<AerIsolation>, RasError>
where
    C: PciSegmentController,
{
    if !status.has_error() {
        return Ok(None);
    }
    diagnostics
        .telemetry
        .record_aer(timestamp_us, node, address, status);
    if status.requires_isolation() {
        controller
            .isolate_segment(address.bus)
            .map_err(RasError::Controller)?;
        Ok(Some(AerIsolation { address, status }))
    } else {
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BudgetPolicy {
    pub thermal_soft_millicelsius: u32,
    pub thermal_critical_millicelsius: u32,
    pub power_soft_milliwatts: u32,
    pub power_critical_milliwatts: u32,
    pub prediction_horizon_us: u64,
}

impl BudgetPolicy {
    pub const fn validate(self) -> Result<Self, RasError> {
        if self.thermal_soft_millicelsius >= self.thermal_critical_millicelsius
            || self.power_soft_milliwatts >= self.power_critical_milliwatts
            || self.prediction_horizon_us == 0
        {
            Err(RasError::InvalidArgument)
        } else {
            Ok(self)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BudgetReading {
    pub timestamp_us: u64,
    pub thermal_millicelsius: u32,
    pub power_milliwatts: u32,
    pub thermal_rate_millicelsius_per_s: i32,
    pub power_rate_milliwatts_per_s: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u8)]
pub enum WorkloadPriority {
    Background = 0,
    BestEffort = 1,
    LatencySensitive = 2,
    Critical = 3,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkloadId(u64);

impl WorkloadId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

pub trait WorkloadController {
    fn evict(&mut self, workload: WorkloadId) -> Result<(), Status>;
    fn throttle(&mut self, percent: u8) -> Result<(), Status>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetDecision {
    Normal,
    Throttle { percent: u8 },
    Evict { count: usize, throttle_percent: u8 },
    Critical { count: usize, throttle_percent: u8 },
}

pub struct BudgetArbiter<const CAPACITY: usize = DEFAULT_WORKLOAD_CAPACITY> {
    policy: BudgetPolicy,
    workloads: [native::Workload; CAPACITY],
    last_reading: Option<BudgetReading>,
}

impl<const CAPACITY: usize> BudgetArbiter<CAPACITY> {
    pub fn new(policy: BudgetPolicy) -> Result<Self, RasError> {
        Ok(Self {
            policy: policy.validate()?,
            workloads: [native::Workload::EMPTY; CAPACITY],
            last_reading: None,
        })
    }

    pub fn register(&mut self, id: WorkloadId, priority: WorkloadPriority) -> Result<(), RasError> {
        match native::register(&mut self.workloads, id, priority) {
            0 => Ok(()),
            1 => Err(RasError::AlreadyTracked),
            2 => Err(RasError::Capacity),
            _ => unreachable!("native RAS registration result"),
        }
    }

    pub fn unregister(&mut self, id: WorkloadId) -> Result<(), RasError> {
        if native::unregister(&mut self.workloads, id) { Ok(()) } else { Err(RasError::NotFound) }
    }

    pub fn observe<C: WorkloadController>(
        &mut self,
        reading: BudgetReading,
        controller: &mut C,
    ) -> Result<BudgetDecision, RasError> {
        let plan = native::budget_decide(self.policy, reading);
        self.last_reading = Some(reading);
        if plan.mode == 0 {
            controller.throttle(0).map_err(RasError::Controller)?;
            return Ok(BudgetDecision::Normal);
        }
        let throttle_percent = plan.throttle_percent;
        controller
            .throttle(throttle_percent)
            .map_err(RasError::Controller)?;
        let mut evicted = 0;
        if plan.mode >= 2 {
            let mut start = 0;
            while let Some(index) = native::next_workload(&self.workloads, start) {
                controller.evict(self.workloads[index].id()).map_err(RasError::Controller)?;
                native::evicted(&mut self.workloads, index);
                evicted += 1;
                start = index + 1;
            }
        }
        Ok(if plan.mode == 3 {
            BudgetDecision::Critical {
                count: evicted,
                throttle_percent,
            }
        } else if plan.mode == 2 {
            BudgetDecision::Evict {
                count: evicted,
                throttle_percent,
            }
        } else {
            BudgetDecision::Throttle {
                percent: throttle_percent,
            }
        })
    }

    pub const fn last_reading(&self) -> Option<BudgetReading> {
        self.last_reading
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirtyPageState {
    Dirty,
    Flushing,
    Clean,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirtyPage {
    pub pool: PoolId,
    pub page: u64,
    pub generation: u64,
    pub state: DirtyPageState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryMarker<const CAPACITY: usize> {
    pub generation: u64,
    pub pages: [Option<DirtyPage>; CAPACITY],
}

pub trait PersistentMemory {
    fn flush_page(&mut self, pool: PoolId, page: u64) -> Result<(), Status>;
    fn barrier(&mut self) -> Result<(), Status>;
}

pub trait RecoveryJournal<const CAPACITY: usize> {
    fn write_marker(&mut self, marker: &RecoveryMarker<CAPACITY>) -> Result<(), Status>;
    fn load_marker(&mut self) -> Result<Option<RecoveryMarker<CAPACITY>>, Status>;
    fn clear_marker(&mut self) -> Result<(), Status>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FlushProgress {
    pub pool: PoolId,
    pub page: u64,
    pub generation: u64,
    pub state: DirtyPageState,
}

pub struct PersistentPool<const CAPACITY: usize = DEFAULT_DIRTY_PAGE_CAPACITY> {
    pages: [Option<DirtyPage>; CAPACITY],
    generation: u64,
}

impl<const CAPACITY: usize> PersistentPool<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            pages: [None; CAPACITY],
            generation: 1,
        }
    }

    pub fn mark_dirty(&mut self, pool: PoolId, page: u64) -> Result<u64, RasError> {
        if page % PAGE_SIZE != 0 {
            return Err(RasError::InvalidArgument);
        }
        if let Some(entry) = self
            .pages
            .iter_mut()
            .flatten()
            .find(|entry| entry.pool == pool && entry.page == page)
        {
            entry.state = DirtyPageState::Dirty;
            entry.generation = self.generation;
            return Ok(self.generation);
        }
        if self.pages.iter().all(Option::is_some) {
            self.pages
                .iter_mut()
                .filter(|entry| entry.is_some_and(|page| page.state == DirtyPageState::Clean))
                .for_each(|entry| *entry = None);
        }
        let slot = self
            .pages
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(RasError::Capacity)?;
        *slot = Some(DirtyPage {
            pool,
            page,
            generation: self.generation,
            state: DirtyPageState::Dirty,
        });
        Ok(self.generation)
    }

    pub fn flush_next<M: PersistentMemory>(
        &mut self,
        memory: &mut M,
    ) -> Result<Option<FlushProgress>, RasError> {
        let index = self
            .pages
            .iter()
            .position(|entry| entry.is_some_and(|page| page.state == DirtyPageState::Dirty));
        let Some(index) = index else { return Ok(None) };
        let page = self.pages[index].ok_or(RasError::NotFound)?;
        self.pages[index].ok_or(RasError::NotFound)?.state = DirtyPageState::Flushing;
        if memory.flush_page(page.pool, page.page).is_err() {
            self.pages[index].ok_or(RasError::NotFound)?.state = DirtyPageState::Failed;
            return Err(RasError::FlushFailed);
        }
        self.pages[index].ok_or(RasError::NotFound)?.state = DirtyPageState::Clean;
        Ok(Some(FlushProgress {
            pool: page.pool,
            page: page.page,
            generation: page.generation,
            state: DirtyPageState::Clean,
        }))
    }

    pub fn flush_all<M: PersistentMemory>(&mut self, memory: &mut M) -> Result<usize, RasError> {
        let mut flushed = 0;
        while self
            .pages
            .iter()
            .any(|entry| entry.is_some_and(|page| page.state == DirtyPageState::Dirty))
        {
            self.flush_next(memory)?;
            flushed += 1;
        }
        memory.barrier().map_err(RasError::Controller)?;
        self.pages
            .iter_mut()
            .filter(|entry| entry.is_some_and(|page| page.state == DirtyPageState::Clean))
            .for_each(|entry| *entry = None);
        Ok(flushed)
    }

    pub fn prepare_power_cycle<M, J>(
        &mut self,
        memory: &mut M,
        journal: &mut J,
    ) -> Result<usize, RasError>
    where
        M: PersistentMemory,
        J: RecoveryJournal<CAPACITY>,
    {
        let marker = RecoveryMarker {
            generation: self.generation,
            pages: self.pages,
        };
        journal
            .write_marker(&marker)
            .map_err(RasError::Controller)?;
        let flushed = self.flush_all(memory)?;
        journal.clear_marker().map_err(RasError::Controller)?;
        self.generation = self.generation.wrapping_add(1).max(1);
        Ok(flushed)
    }

    pub fn recover<J: RecoveryJournal<CAPACITY>>(
        &mut self,
        journal: &mut J,
    ) -> Result<bool, RasError> {
        let Some(marker) = journal.load_marker().map_err(RasError::Controller)? else {
            return Ok(false);
        };
        self.pages = marker.pages;
        for page in self.pages.iter_mut().flatten() {
            page.state = DirtyPageState::Dirty
        }
        self.generation = marker.generation.wrapping_add(1).max(1);
        Ok(true)
    }

    pub fn pages(&self) -> impl Iterator<Item = DirtyPage> + '_ {
        self.pages.iter().flatten().copied()
    }
}

impl<const CAPACITY: usize> Default for PersistentPool<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

/// Marks a GhostFS device failed after a hardware error, so future pool IO can
/// select mirrors or report degradation instead of retrying corrupt media.
pub fn fail_storage_device<const DEVICES: usize, const POOLS: usize>(
    storage: &mut StoragePoolAdmin<DEVICES, POOLS>,
    device: StorageDeviceId,
) -> Result<(), RasError> {
    storage
        .set_device_health(device, DeviceHealth::Failed)
        .map_err(RasError::Storage)
}
