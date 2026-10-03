use crate::{ErrorCounters, FaultSeverity, FaultSource, HardwareEvent, PoisonedRange};
use ghostos_fabric::{AddressRange, NodeId};

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Event {
    sequence: u64,
    timestamp_us: u64,
    node: u32,
    source: u32,
    severity: u32,
    device: u64,
    address: u64,
    detail: u32,
}

impl Event {
    const EMPTY: Self = Self { sequence: 0, timestamp_us: 0, node: 0, source: 0,
        severity: 0, device: 0, address: 0, detail: 0 };
    fn from_public(event: HardwareEvent) -> Self {
        Self { sequence: event.sequence, timestamp_us: event.timestamp_us, node: event.node.raw(),
            source: match event.source {
                FaultSource::EccMemory => 0, FaultSource::Cxl => 1, FaultSource::PcieAer => 2,
                FaultSource::Thermal => 3, FaultSource::Power => 4, FaultSource::PersistentMemory => 5,
            }, severity: event.severity as u32, device: event.device, address: event.address,
            detail: event.detail }
    }
    fn to_public(self) -> HardwareEvent {
        HardwareEvent { sequence: self.sequence, timestamp_us: self.timestamp_us,
            node: NodeId::from_valid_raw(self.node), source: match self.source {
                0 => FaultSource::EccMemory, 1 => FaultSource::Cxl, 2 => FaultSource::PcieAer,
                3 => FaultSource::Thermal, 4 => FaultSource::Power, 5 => FaultSource::PersistentMemory,
                _ => unreachable!("native RAS source"),
            }, severity: match self.severity {
                0 => FaultSeverity::Corrected, 1 => FaultSeverity::Warning,
                2 => FaultSeverity::Error, 3 => FaultSeverity::Fatal,
                _ => unreachable!("native RAS severity"),
            }, device: self.device, address: self.address, detail: self.detail }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct EventSlot { event: Event, occupied: bool }
impl EventSlot {
    pub(crate) const EMPTY: Self = Self { event: Event::EMPTY, occupied: false };
}

#[repr(C)]
pub(crate) struct Telemetry {
    cursor: usize,
    pub(crate) count: usize,
    pub(crate) dropped: u64,
    next_sequence: u64,
    pub(crate) counters: ErrorCounters,
}
impl Telemetry {
    pub(crate) const EMPTY: Self = Self { cursor: 0, count: 0, dropped: 0, next_sequence: 1,
        counters: ErrorCounters { corrected_ecc: 0, uncorrected_ecc: 0, cxl_poisoned_flits: 0,
            aer_correctable: 0, aer_non_fatal: 0, aer_fatal: 0 } };
    pub(crate) fn record(&mut self, slots: &mut [EventSlot], event: HardwareEvent) -> u64 {
        assert!(!slots.is_empty() && self.cursor < slots.len() && self.count <= slots.len());
        let mut native = Event::from_public(event);
        unsafe { ghostos_ras_record(self, slots.as_mut_ptr(), slots.len(), &mut native) }
    }
    pub(crate) fn event(&self, slots: &[EventSlot], offset: usize) -> Option<HardwareEvent> {
        if slots.is_empty() { return None }
        let mut event = Event::EMPTY;
        unsafe { ghostos_ras_event_get(self, slots.as_ptr(), slots.len(), offset, &mut event) }
            .then(|| event.to_public())
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Poison {
    node: u32,
    device: u64,
    start: u64,
    length: u64,
    sequence: u64,
    occupied: bool,
}
impl Poison {
    pub(crate) const EMPTY: Self = Self { node: 0, device: 0, start: 0, length: 0,
        sequence: 0, occupied: false };
    pub(crate) fn to_public(self) -> Option<PoisonedRange> {
        self.occupied.then_some(PoisonedRange { node: NodeId::from_valid_raw(self.node),
            device: self.device, range: AddressRange { start: self.start, length: self.length },
            sequence: self.sequence })
    }
}

pub(crate) fn quarantine(slots: &mut [Poison], range: PoisonedRange) -> i32 {
    let poison = Poison { node: range.node.raw(), device: range.device, start: range.range.start,
        length: range.range.length, sequence: range.sequence, occupied: true };
    unsafe { ghostos_ras_quarantine(slots.as_mut_ptr(), slots.len(), poison, cfg!(debug_assertions)) }
}

pub(crate) fn admit(slots: &[Poison], node: NodeId, range: AddressRange) -> i32 {
    unsafe { ghostos_ras_admit(slots.as_ptr(), slots.len(), node.raw(), range.start,
        range.length, cfg!(debug_assertions)) }
}

const _: () = {
    assert!(ghostos_fabric::PAGE_SIZE == 4096);
    assert!(core::mem::size_of::<Event>() == 56);
    assert!(core::mem::size_of::<EventSlot>() == 64);
    assert!(core::mem::size_of::<Telemetry>() == 80);
    assert!(core::mem::offset_of!(Telemetry, counters) == 32);
    assert!(core::mem::size_of::<Poison>() == 48);
    assert!(core::mem::offset_of!(Poison, occupied) == 40);
};

unsafe extern "C" {
    fn ghostos_ras_record(state: *mut Telemetry, slots: *mut EventSlot,
        capacity: usize, event: *mut Event) -> u64;
    fn ghostos_ras_event_get(state: *const Telemetry, slots: *const EventSlot,
        capacity: usize, offset: usize, event: *mut Event) -> bool;
    fn ghostos_ras_quarantine(slots: *mut Poison, capacity: usize, poison: Poison, checked: bool) -> i32;
    fn ghostos_ras_admit(slots: *const Poison, capacity: usize,
        node: u32, start: u64, length: u64, checked: bool) -> i32;
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Workload { id: u64, priority: u8, active: bool, occupied: bool }
impl Workload {
    pub(crate) const EMPTY: Self = Self { id: 0, priority: 0, active: false, occupied: false };
    pub(crate) fn id(self) -> crate::WorkloadId {
        crate::WorkloadId::new(self.id).expect("native RAS workload")
    }
}
#[repr(C)]
struct BudgetPolicy { thermal_soft: u32, thermal_critical: u32, power_soft: u32, power_critical: u32, horizon_us: u64 }
#[repr(C)]
struct BudgetReading { timestamp_us: u64, thermal: u32, power: u32, thermal_rate: i32, power_rate: i32 }
#[repr(C)]
pub(crate) struct BudgetPlan { pub(crate) mode: u32, pub(crate) throttle_percent: u8 }

pub(crate) fn budget_decide(policy: crate::BudgetPolicy, reading: crate::BudgetReading) -> BudgetPlan {
    let p = BudgetPolicy { thermal_soft: policy.thermal_soft_millicelsius,
        thermal_critical: policy.thermal_critical_millicelsius, power_soft: policy.power_soft_milliwatts,
        power_critical: policy.power_critical_milliwatts, horizon_us: policy.prediction_horizon_us };
    let r = BudgetReading { timestamp_us: reading.timestamp_us, thermal: reading.thermal_millicelsius,
        power: reading.power_milliwatts, thermal_rate: reading.thermal_rate_millicelsius_per_s,
        power_rate: reading.power_rate_milliwatts_per_s };
    unsafe { ghostos_ras_budget_decide(p, r) }
}
pub(crate) fn register(slots: &mut [Workload], id: crate::WorkloadId, priority: crate::WorkloadPriority) -> i32 {
    unsafe { ghostos_ras_workload_register(slots.as_mut_ptr(), slots.len(), id.raw(), priority as u8) }
}
pub(crate) fn unregister(slots: &mut [Workload], id: crate::WorkloadId) -> bool {
    unsafe { ghostos_ras_workload_unregister(slots.as_mut_ptr(), slots.len(), id.raw()) }
}
pub(crate) fn next_workload(slots: &[Workload], start: usize) -> Option<usize> {
    let index = unsafe { ghostos_ras_workload_next(slots.as_ptr(), slots.len(), start) };
    (index < slots.len()).then_some(index)
}
pub(crate) fn evicted(slots: &mut [Workload], index: usize) {
    assert!(index < slots.len());
    unsafe { ghostos_ras_workload_evicted(slots.as_mut_ptr(), index) }
}
const _: () = {
    assert!(core::mem::size_of::<Workload>() == 16);
    assert!(core::mem::size_of::<BudgetPolicy>() == 24);
    assert!(core::mem::size_of::<BudgetReading>() == 24);
    assert!(core::mem::size_of::<BudgetPlan>() == 8);
};
unsafe extern "C" {
    fn ghostos_ras_workload_register(slots: *mut Workload, capacity: usize, id: u64, priority: u8) -> i32;
    fn ghostos_ras_workload_unregister(slots: *mut Workload, capacity: usize, id: u64) -> bool;
    fn ghostos_ras_budget_decide(policy: BudgetPolicy, reading: BudgetReading) -> BudgetPlan;
    fn ghostos_ras_workload_next(slots: *const Workload, capacity: usize, start: usize) -> usize;
    fn ghostos_ras_workload_evicted(slots: *mut Workload, index: usize);
}
