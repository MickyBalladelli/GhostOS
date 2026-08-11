use synos_init::{
    CapabilityFence, CrashReason, ExitReason, FaultCause, FaultDomain, FaultState, ProcessId,
    RestartPolicy, ServiceId, ServiceKind, ServiceName, ServiceSpec, Supervisor, SupervisorError,
    SupervisorEvent, SupervisorRuntime,
};

struct Runtime {
    next_process: u64,
    spawned_fences: Vec<Option<CapabilityFence>>,
}

impl SupervisorRuntime for Runtime {
    type Error = ();

    fn spawn(&mut self, request: synos_init::SpawnRequest) -> Result<ProcessId, Self::Error> {
        self.spawned_fences.push(request.fault_fence);
        let process = ProcessId::new(self.next_process).unwrap();
        self.next_process += 1;
        Ok(process)
    }

    fn fence_process(&mut self, _process: ProcessId) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn spec(id: u32, name: &'static str) -> ServiceSpec {
    ServiceSpec {
        id: ServiceId::new(id).unwrap(),
        name: ServiceName::new(name).unwrap(),
        kind: ServiceKind::System,
        image_id: u128::from(id),
        capability_profile: u64::from(id),
        restart: RestartPolicy::on_failure(1, 100, 1, 1).unwrap(),
    }
}

#[test]
fn one_fault_invalidates_only_its_domain_epoch() {
    let mut domains = synos_init::FaultDomainRegistry::<4>::new();
    domains.attach(FaultDomain::StorageDaemon, 7).unwrap();
    domains.attach(FaultDomain::NetworkDaemon, 8).unwrap();
    let storage = domains.issue_capability(FaultDomain::StorageDaemon).unwrap();
    let network = domains.issue_capability(FaultDomain::NetworkDaemon).unwrap();

    let report = domains
        .report_fault(FaultDomain::StorageDaemon, FaultCause::Corruption)
        .unwrap();
    assert_eq!(report.attached_services, 1);
    assert_eq!(domains.status(FaultDomain::StorageDaemon).state, FaultState::Faulted);
    assert_eq!(domains.status(FaultDomain::NetworkDaemon).state, FaultState::Healthy);
    assert_eq!(domains.validate(network), Ok(()));
    assert_eq!(
        domains.validate(storage),
        Err(synos_init::FaultDomainError::StaleCapability)
    );

    let lease = domains.begin_recovery(FaultDomain::StorageDaemon).unwrap();
    let fresh = domains.complete_recovery(lease).unwrap();
    assert_ne!(fresh.epoch(), storage.epoch());
    assert_ne!(fresh.generation(), storage.generation());
    assert_eq!(domains.validate(fresh), Ok(()));
}

#[test]
fn supervisor_contains_crash_and_keeps_other_service_inspectable() {
    let mut supervisor = Supervisor::<2>::new();
    let storage = ServiceId::new(1).unwrap();
    let network = ServiceId::new(2).unwrap();
    supervisor.register(spec(1, "storage")).unwrap();
    supervisor.register(spec(2, "network")).unwrap();
    supervisor
        .bind_fault_domain(storage, FaultDomain::StorageDaemon)
        .unwrap();
    supervisor
        .bind_fault_domain(network, FaultDomain::NetworkDaemon)
        .unwrap();

    let mut runtime = Runtime { next_process: 10, spawned_fences: Vec::new() };
    let storage_process = match supervisor.start(storage, &mut runtime).unwrap() {
        SupervisorEvent::Started { process, .. } => process,
        _ => panic!("storage did not start"),
    };
    supervisor.start(network, &mut runtime).unwrap();
    let old_fence = supervisor
        .fault_domains()
        .issue_capability(FaultDomain::StorageDaemon)
        .unwrap();

    supervisor
        .report_exit(
            storage_process,
            ExitReason::Crash(CrashReason::ProtectionFault),
            0,
            &mut runtime,
        )
        .unwrap();

    assert_eq!(supervisor.accepts_work(network), Ok(()));
    assert_eq!(supervisor.accepts_work(storage), Err(SupervisorError::NotReady));
    assert_eq!(supervisor.status(network).unwrap().process, Some(ProcessId::new(11).unwrap()));
    assert_eq!(
        supervisor.fault_domains().validate(old_fence),
        Err(synos_init::FaultDomainError::StaleCapability)
    );

    let fresh_fence: CapabilityFence = supervisor
        .recover_domain(FaultDomain::StorageDaemon)
        .unwrap();
    assert_eq!(supervisor.fault_domains().validate(fresh_fence), Ok(()));
    let restarted = supervisor.tick(1, &mut runtime).unwrap().unwrap();
    assert!(matches!(restarted, SupervisorEvent::Started { generation: 2, .. }));
    assert_eq!(runtime.spawned_fences[0], Some(old_fence));
    assert_eq!(runtime.spawned_fences[2], Some(fresh_fence));
    assert_eq!(supervisor.accepts_work(network), Ok(()));
}
