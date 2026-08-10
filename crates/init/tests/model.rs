use synos_init::{
    CrashReason, ExitReason, ProcessId, RestartPolicy, ServiceId, ServiceKind, ServiceName,
    ServiceSpec, ServiceState, SpawnRequest, Supervisor, SupervisorError, SupervisorEvent,
    SupervisorRuntime,
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
