use synos_init::{
    CrashReason, ExitReason, ProcessId, RestartPolicy, ServiceId, ServiceKind, ServiceName,
    ServiceReadiness, ServiceSpec, ServiceState, SpawnRequest, Supervisor, SupervisorError,
    SupervisorEvent, SupervisorRuntime,
};

struct Runtime {
    next_process: u64,
}

impl Runtime {
    const fn new() -> Self {
        Self { next_process: 1 }
    }
}

impl SupervisorRuntime for Runtime {
    type Error = ();

    fn spawn(&mut self, _request: SpawnRequest) -> Result<ProcessId, Self::Error> {
        let process = ProcessId::new(self.next_process).unwrap();
        self.next_process += 1;
        Ok(process)
    }

    fn fence_process(&mut self, _process: ProcessId) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn supervisor() -> Supervisor<1> {
    let mut supervisor = Supervisor::new();
    supervisor
        .register(ServiceSpec {
            id: ServiceId::new(1).unwrap(),
            name: ServiceName::new("worker").unwrap(),
            kind: ServiceKind::System,
            image_id: 1,
            capability_profile: 1,
            restart: RestartPolicy::on_failure(3, 100, 10, 40).unwrap(),
        })
        .unwrap();
    supervisor
}

#[test]
fn restart_deadline_is_exclusive_before_and_inclusive_at_boundary() {
    let service = ServiceId::new(1).unwrap();
    let mut supervisor = supervisor();
    let mut runtime = Runtime::new();
    let started = supervisor.start(service, &mut runtime).unwrap();
    let process = match started {
        SupervisorEvent::Started { process, generation, .. } => {
            assert_eq!(generation, 1);
            process
        }
        _ => panic!("service did not start"),
    };

    assert!(matches!(
        supervisor.report_exit(
            process,
            ExitReason::Crash(CrashReason::Watchdog),
            0,
            &mut runtime,
        ),
        Ok(SupervisorEvent::RestartScheduled { at_us: 10, .. })
    ));
    assert_eq!(supervisor.tick(9, &mut runtime), Ok(None));
    assert!(matches!(
        supervisor.tick(10, &mut runtime),
        Ok(Some(SupervisorEvent::Started { generation: 2, .. }))
    ));
}

#[test]
fn stale_exit_cannot_affect_the_restarted_generation() {
    let service = ServiceId::new(1).unwrap();
    let mut supervisor = supervisor();
    let mut runtime = Runtime::new();
    let first = match supervisor.start(service, &mut runtime).unwrap() {
        SupervisorEvent::Started { process, .. } => process,
        _ => panic!("service did not start"),
    };
    supervisor
        .report_exit(
            first,
            ExitReason::Crash(CrashReason::Panic),
            0,
            &mut runtime,
        )
        .unwrap();
    let second = match supervisor.tick(10, &mut runtime).unwrap().unwrap() {
        SupervisorEvent::Started { process, .. } => process,
        _ => panic!("service did not restart"),
    };

    assert_ne!(first, second);
    assert_eq!(
        supervisor.report_exit(
            first,
            ExitReason::Crash(CrashReason::UnexpectedExit),
            11,
            &mut runtime,
        ),
        Err(SupervisorError::StaleExit)
    );
    assert_eq!(supervisor.status(service).unwrap().state, ServiceState::Running);
}

#[test]
fn restart_storm_hits_the_policy_limit_and_fails_cleanly() {
    let service = ServiceId::new(1).unwrap();
    let mut supervisor = supervisor();
    let mut runtime = Runtime::new();
    let mut process = match supervisor.start(service, &mut runtime).unwrap() {
        SupervisorEvent::Started { process, .. } => process,
        _ => panic!("service did not start"),
    };

    for (now_us, restart_at_us) in [(0, 10), (10, 30), (30, 70)] {
        assert!(matches!(
            supervisor.report_exit(
                process,
                ExitReason::Crash(CrashReason::ProtectionFault),
                now_us,
                &mut runtime,
            ),
            Ok(SupervisorEvent::RestartScheduled { at_us, .. }) if at_us == restart_at_us
        ));
        process = match supervisor.tick(restart_at_us, &mut runtime).unwrap().unwrap() {
            SupervisorEvent::Started { process, .. } => process,
            _ => panic!("service did not restart"),
        };
    }

    assert!(matches!(
        supervisor.report_exit(
            process,
            ExitReason::Crash(CrashReason::ProtectionFault),
            70,
            &mut runtime,
        ),
        Ok(SupervisorEvent::Failed { .. })
    ));
    assert_eq!(supervisor.status(service).unwrap().state, ServiceState::Failed);
}

#[test]
fn dependency_cycle_is_rejected_without_mutating_the_graph() {
    let mut supervisor = Supervisor::<2>::new();
    for (id, name) in [(1, "one"), (2, "two")] {
        supervisor
            .register(ServiceSpec {
                id: ServiceId::new(id).unwrap(),
                name: ServiceName::new(name).unwrap(),
                kind: ServiceKind::System,
                image_id: id as u128,
                capability_profile: id as u64,
                restart: RestartPolicy::NEVER,
            })
            .unwrap();
    }

    supervisor
        .add_dependency(ServiceId::new(1).unwrap(), ServiceId::new(2).unwrap())
        .unwrap();
    assert_eq!(
        supervisor.add_dependency(ServiceId::new(2).unwrap(), ServiceId::new(1).unwrap()),
        Err(SupervisorError::DependencyCycle)
    );
    assert_eq!(supervisor.readiness(ServiceId::new(2).unwrap()).unwrap(), ServiceReadiness::Waiting);
}

#[test]
fn readiness_blocks_dependents_during_dependency_restart() {
    let mut supervisor = Supervisor::<2>::new();
    let dependency = ServiceId::new(1).unwrap();
    let worker = ServiceId::new(2).unwrap();
    supervisor
        .register(ServiceSpec {
            id: dependency,
            name: ServiceName::new("dependency").unwrap(),
            kind: ServiceKind::System,
            image_id: 1,
            capability_profile: 1,
            restart: RestartPolicy::on_failure(1, 100, 10, 10).unwrap(),
        })
        .unwrap();
    supervisor
        .register_with_dependencies(
            ServiceSpec {
                id: worker,
                name: ServiceName::new("worker").unwrap(),
                kind: ServiceKind::System,
                image_id: 2,
                capability_profile: 2,
                restart: RestartPolicy::NEVER,
            },
            &[dependency],
        )
        .unwrap();

    let mut runtime = Runtime::new();
    assert_eq!(supervisor.start(worker, &mut runtime), Err(SupervisorError::DependencyNotReady));
    let dependency_process = match supervisor.start(dependency, &mut runtime).unwrap() {
        SupervisorEvent::Started { process, .. } => process,
        _ => panic!("dependency did not start"),
    };
    supervisor.start(worker, &mut runtime).unwrap();
    assert_eq!(supervisor.accepts_work(worker), Ok(()));

    supervisor
        .report_exit(
            dependency_process,
            ExitReason::Crash(CrashReason::Watchdog),
            0,
            &mut runtime,
        )
        .unwrap();
    assert_eq!(supervisor.accepts_work(worker), Err(SupervisorError::NotReady));
    supervisor.tick(10, &mut runtime).unwrap();
    assert_eq!(supervisor.readiness(worker).unwrap(), ServiceReadiness::Ready);
}

#[test]
fn boot_and_reboot_trace_dependencies_and_reverse_shutdown() {
    let mut supervisor = Supervisor::<3>::new();
    for (id, name) in [(1, "base"), (2, "middle"), (3, "top")] {
        supervisor
            .register(ServiceSpec {
                id: ServiceId::new(id).unwrap(),
                name: ServiceName::new(name).unwrap(),
                kind: ServiceKind::System,
                image_id: id as u128,
                capability_profile: id as u64,
                restart: RestartPolicy::NEVER,
            })
            .unwrap();
    }
    supervisor
        .add_dependency(ServiceId::new(2).unwrap(), ServiceId::new(1).unwrap())
        .unwrap();
    supervisor
        .add_dependency(ServiceId::new(3).unwrap(), ServiceId::new(2).unwrap())
        .unwrap();

    let mut runtime = Runtime::new();
    let boot = supervisor.boot(&mut runtime).unwrap();
    let boot_events: Vec<_> = boot.events().collect();
    assert_eq!(boot_events[0], synos_init::LifecycleEvent::BootStarted);
    assert_eq!(
        boot_events[1..7],
        [
            synos_init::LifecycleEvent::ServiceStarted { service: ServiceId::new(1).unwrap(), generation: 1 },
            synos_init::LifecycleEvent::ServiceReady { service: ServiceId::new(1).unwrap() },
            synos_init::LifecycleEvent::ServiceStarted { service: ServiceId::new(2).unwrap(), generation: 1 },
            synos_init::LifecycleEvent::ServiceReady { service: ServiceId::new(2).unwrap() },
            synos_init::LifecycleEvent::ServiceStarted { service: ServiceId::new(3).unwrap(), generation: 1 },
            synos_init::LifecycleEvent::ServiceReady { service: ServiceId::new(3).unwrap() },
        ]
    );

    let reboot = supervisor.reboot(&mut runtime).unwrap();
    let reboot_events: Vec<_> = reboot.events().collect();
    let stopped: Vec<_> = reboot_events
        .iter()
        .filter_map(|event| match event {
            synos_init::LifecycleEvent::ServiceStopped { service } => Some(*service),
            _ => None,
        })
        .collect();
    assert_eq!(stopped, vec![ServiceId::new(3).unwrap(), ServiceId::new(2).unwrap(), ServiceId::new(1).unwrap()]);
    assert_eq!(supervisor.readiness(ServiceId::new(3).unwrap()).unwrap(), ServiceReadiness::Ready);
}
