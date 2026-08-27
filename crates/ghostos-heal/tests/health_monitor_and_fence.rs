// Inventory: coverage_59_9.rs (legacy roadmap section 59).
use ghostos_heal::{
    HealthConfig, HealthFault, HealthMonitor, HealthState, HotSwapCoordinator, HotSwapError,
    HotSwapRequest, HotSwapRuntime, KernelPatchCoordinator, KernelPatchError, KernelPatchRequest,
    KernelPatchRuntime, RecoveryCoordinator, RecoveryError, RecoveryRuntime,
};
use ghostos_init::ProcessId;
use ghostos_ipc::{DescriptorInheritance, InheritableDescriptor};
use ghostos_ghostfs::{ReadOnlySnapshot, RmsMapHandle, SynFs};

#[test]
fn health_monitor_distinguishes_unknown_healthy_and_stalled() {
    let monitor = HealthMonitor::<2>::new();
    let config = HealthConfig::new(10, 20, 5).unwrap().with_memory_checksum(55);
    let token = monitor.register(7, config).unwrap();
    assert_eq!(monitor.check(token, 0).unwrap().state, HealthState::Unknown);
    monitor.heartbeat(token, 10, 1).unwrap();
    monitor.driver_progress(token, 10, 1).unwrap();
    monitor.memory_checksum(token, 55).unwrap();
    assert_eq!(monitor.check(token, 12).unwrap().state, HealthState::Healthy);
    assert_eq!(monitor.check(token, 20).unwrap().state, HealthState::Stalled(HealthFault::DriverStall));
    monitor.memory_checksum(token, 1).unwrap();
    assert_eq!(monitor.check(token, 20).unwrap().state, HealthState::Stalled(HealthFault::MemoryCorruption));
    monitor.unregister(token).unwrap();
    assert_eq!(monitor.check(token, 20), Err(ghostos_heal::HealthError::StaleToken));
}

struct Runtime {
    fenced: Vec<ProcessId>,
    restored: Vec<u64>,
    spawned: ProcessId,
}

impl RecoveryRuntime for Runtime {
    type Error = ();

    fn fence_process(&mut self, process: ProcessId) -> Result<(), Self::Error> {
        self.fenced.push(process);
        Ok(())
    }

    fn restore_state<const MAX_BLOCKS: usize>(
        &mut self,
        service: u64,
        _snapshot: &ReadOnlySnapshot<'_, MAX_BLOCKS>,
    ) -> Result<(), Self::Error> {
        self.restored.push(service);
        Ok(())
    }

    fn spawn_recovered(&mut self, _request: ghostos_heal::RecoverySpawnRequest) -> Result<ProcessId, Self::Error> {
        Ok(self.spawned)
    }
}

#[test]
fn recovery_fences_restores_spawns_and_rejects_stale_processes() {
    let process = ProcessId::new(10).unwrap();
    let replacement = ProcessId::new(11).unwrap();
    let mut filesystem = SynFs::<64>::new();
    filesystem.write("/state", b"clean").unwrap();
    let mut coordinator = RecoveryCoordinator::<2>::new();
    coordinator.register(7, 0x55, process).unwrap();
    let checkpoint = coordinator.capture_clean_snapshot(&mut filesystem, 7).unwrap();
    let mut runtime = Runtime { fenced: Vec::new(), restored: Vec::new(), spawned: replacement };
    let receipt = coordinator
        .recover(7, process, &filesystem, RmsMapHandle::from_capability(1 << 32).unwrap(), &mut runtime)
        .unwrap();
    assert_eq!(receipt.process, replacement);
    assert_eq!(receipt.generation, 2);
    assert_eq!(receipt.snapshot, checkpoint);
    assert_eq!(runtime.fenced, vec![process]);
    assert_eq!(runtime.restored, vec![7]);
    assert_eq!(coordinator.status(7).unwrap().process, Some(replacement));
    assert_eq!(coordinator.recover(7, process, &filesystem, RmsMapHandle::from_capability(1 << 32).unwrap(), &mut runtime), Err(RecoveryError::StaleProcess));
}

struct PatchRuntime {
    redirects: Vec<usize>,
    restores: Vec<usize>,
    quiesced: usize,
    resumed: usize,
}

impl KernelPatchRuntime for PatchRuntime {
    type Error = ();

    fn validate(&mut self, _request: KernelPatchRequest) -> Result<(), Self::Error> { Ok(()) }
    fn quiesce(&mut self) -> Result<(), Self::Error> { self.quiesced += 1; Ok(()) }
    fn redirect(&mut self, request: KernelPatchRequest) -> Result<(), Self::Error> { self.redirects.push(request.target); Ok(()) }
    fn restore(&mut self, request: KernelPatchRequest) -> Result<(), Self::Error> { self.restores.push(request.target); Ok(()) }
    fn resume(&mut self) -> Result<(), Self::Error> { self.resumed += 1; Ok(()) }
}

#[test]
fn kernel_patch_transaction_validates_generation_and_rolls_back() {
    let mut coordinator = KernelPatchCoordinator::<2>::new();
    let mut runtime = PatchRuntime { redirects: Vec::new(), restores: Vec::new(), quiesced: 0, resumed: 0 };
    let request = KernelPatchRequest::new(1, 0x1000, 0x2000, 1);
    let receipt = coordinator.apply(&mut runtime, &[request]).unwrap();
    assert_eq!(receipt.generation, 2);
    assert_eq!(runtime.redirects, vec![0x1000]);
    assert_eq!(coordinator.apply(&mut runtime, &[request]), Err(KernelPatchError::StaleGeneration));
    let rollback = coordinator.rollback_last(&mut runtime).unwrap();
    assert_eq!(rollback.generation, 3);
    assert_eq!(runtime.restores, vec![0x1000]);
    assert!(coordinator.records().next().is_none());
}

struct SwapRuntime {
    events: Vec<&'static str>,
}

impl DescriptorInheritance for SwapRuntime {
    type Error = ();

    fn inherit_descriptors(&mut self, _source_process: u64, _target_process: u64, _descriptors: &[InheritableDescriptor]) -> Result<(), Self::Error> {
        self.events.push("inherit");
        Ok(())
    }
}

impl HotSwapRuntime for SwapRuntime {
    fn spawn_replacement(&mut self, _request: ghostos_heal::ReplacementSpawnRequest<'_>) -> Result<ProcessId, Self::Error> {
        self.events.push("spawn");
        Ok(ProcessId::new(11).unwrap())
    }
    fn replacement_ready(&mut self, _process: ProcessId) -> Result<(), Self::Error> { self.events.push("ready"); Ok(()) }
    fn switch_service(&mut self, _service: u64, _expected_old: ProcessId, _replacement: ProcessId) -> Result<(), Self::Error> { self.events.push("switch"); Ok(()) }
    fn drain_process(&mut self, _process: ProcessId) -> Result<(), Self::Error> { self.events.push("drain"); Ok(()) }
    fn fence_process(&mut self, _process: ProcessId) -> Result<(), Self::Error> { self.events.push("fence"); Ok(()) }
}

#[test]
fn hot_swap_preserves_connection_order_and_rejects_invalid_requests() {
    let old = ProcessId::new(10).unwrap();
    let descriptors = [InheritableDescriptor::channel(1).unwrap()];
    let request = HotSwapRequest::new(7, old, 3, 0x55, &descriptors);
    let mut runtime = SwapRuntime { events: Vec::new() };
    let receipt = HotSwapCoordinator::new().replace(&mut runtime, request).unwrap();
    assert_eq!(receipt.replacement_process.raw(), 11);
    assert_eq!(runtime.events, vec!["spawn", "inherit", "ready", "switch", "drain", "fence"]);
    assert_eq!(HotSwapCoordinator::new().replace(&mut runtime, HotSwapRequest::new(0, old, 3, 0x55, &descriptors)), Err(HotSwapError::InvalidRequest));
}
