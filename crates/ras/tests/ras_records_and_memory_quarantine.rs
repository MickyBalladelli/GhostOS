// Inventory: coverage_59_9.rs (legacy roadmap section 59).
use ghostos_fabric::{AddressRange, NodeId, PAGE_SIZE};
use ghostos_legacy_pc_drivers::{PciAddress, PcieAerStatus};
use ghostos_ras::{
    BudgetArbiter, BudgetDecision, BudgetPolicy, BudgetReading, ErrorTelemetry, FaultSeverity,
    HardwareDiagnostics, PoisonTracker, PoisonedRange, WorkloadController, WorkloadId,
};
use ghostos_status::Status;

#[test]
fn ras_records_bounded_hardware_history_and_quarantines_memory() {
    let mut telemetry = ErrorTelemetry::<2>::new();
    assert_eq!(telemetry.record_ecc(1, NodeId::LOCAL, 3, PAGE_SIZE, true, 7), 1);
    assert_eq!(telemetry.record_ecc(2, NodeId::LOCAL, 3, PAGE_SIZE * 2, false, 8), 2);
    assert_eq!(telemetry.record_cxl_poison(3, NodeId::LOCAL, 4, PAGE_SIZE * 3, 9), 3);
    assert_eq!(telemetry.dropped(), 1);
    assert_eq!(telemetry.counters().corrected_ecc, 1);
    assert_eq!(telemetry.counters().uncorrected_ecc, 1);
    assert_eq!(telemetry.counters().cxl_poisoned_flits, 1);
    assert_eq!(telemetry.events().next().unwrap().severity, FaultSeverity::Fatal);

    let range = AddressRange::new(PAGE_SIZE * 4, PAGE_SIZE).unwrap();
    let mut poison = PoisonTracker::<1>::new();
    poison.quarantine(PoisonedRange { node: NodeId::LOCAL, device: 1, range, sequence: 1 }).unwrap();
    assert!(poison.admit(NodeId::LOCAL, range).is_err());
    assert!(poison.quarantine(PoisonedRange { node: NodeId::LOCAL, device: 1, range, sequence: 2 }).is_err());

    let mut diagnostics = HardwareDiagnostics::<4, 4>::new();
    diagnostics.record_ecc(4, NodeId::LOCAL, 5, PAGE_SIZE * 5, false, 10).unwrap();
    assert!(diagnostics.poison.admit(NodeId::LOCAL, AddressRange::new(PAGE_SIZE * 5, PAGE_SIZE).unwrap()).is_err());
}

#[derive(Default)]
struct Controller {
    evicted: Vec<u64>,
    throttle: Vec<u8>,
}

impl WorkloadController for Controller {
    fn evict(&mut self, workload: WorkloadId) -> Result<(), Status> {
        self.evicted.push(workload.raw());
        Ok(())
    }

    fn throttle(&mut self, percent: u8) -> Result<(), Status> {
        self.throttle.push(percent);
        Ok(())
    }
}

#[test]
fn ras_budget_prediction_throttles_and_evicts_before_critical() {
    let policy = BudgetPolicy {
        thermal_soft_millicelsius: 70000,
        thermal_critical_millicelsius: 90000,
        power_soft_milliwatts: 100,
        power_critical_milliwatts: 150,
        prediction_horizon_us: 1_000_000,
    };
    let mut arbiter = BudgetArbiter::<2>::new(policy).unwrap();
    arbiter.register(WorkloadId::new(1).unwrap(), ghostos_ras::WorkloadPriority::BestEffort).unwrap();
    arbiter.register(WorkloadId::new(2).unwrap(), ghostos_ras::WorkloadPriority::Critical).unwrap();
    let mut controller = Controller::default();
    let decision = arbiter.observe(BudgetReading {
        timestamp_us: 1,
        thermal_millicelsius: 60000,
        power_milliwatts: 80,
        thermal_rate_millicelsius_per_s: 40000,
        power_rate_milliwatts_per_s: 80,
    }, &mut controller).unwrap();
    assert_eq!(decision, BudgetDecision::Evict { count: 1, throttle_percent: 75 });
    assert_eq!(controller.evicted, vec![1]);
    assert_eq!(controller.throttle, vec![75]);
}

#[test]
fn aer_status_requires_isolation_only_for_uncorrectable_errors() {
    let mut diagnostics = HardwareDiagnostics::<4, 4>::new();
    let mut isolated = Vec::new();
    struct Segment<'a>(&'a mut Vec<u8>);
    impl ghostos_ras::PciSegmentController for Segment<'_> {
        fn isolate_segment(&mut self, bus: u8) -> Result<(), Status> { self.0.push(bus); Ok(()) }
    }
    let address = PciAddress::new(4, 2, 0).unwrap();
    let mut controller = Segment(&mut isolated);
    assert!(ghostos_ras::handle_aer(&mut diagnostics, &mut controller, 1, NodeId::LOCAL, address, PcieAerStatus { correctable: 1, non_fatal: 0, fatal: 0 }).unwrap().is_none());
    assert!(ghostos_ras::handle_aer(&mut diagnostics, &mut controller, 2, NodeId::LOCAL, address, PcieAerStatus { correctable: 0, non_fatal: 1, fatal: 0 }).unwrap().is_some());
    assert_eq!(isolated, vec![4]);
}
