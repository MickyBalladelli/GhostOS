use synos_init::{ServiceId, ServiceKind, ServiceName, ServiceSpec, Supervisor, SupervisorError, SupervisorRuntime, RestartPolicy, SpawnRequest, ProcessId};
use synos_test_support::crash::{CrashBoundary, CrashDomain, CrashHarness, CrashPoint};

struct Runtime;

impl SupervisorRuntime for Runtime {
    type Error = ();

    fn spawn(&mut self, _request: SpawnRequest) -> Result<ProcessId, Self::Error> {
        Ok(ProcessId::new(9).unwrap())
    }

    fn fence_process(&mut self, _process: ProcessId) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[test]
fn interruption_hook_runs_after_service_restart_spawn() {
    let mut supervisor = Supervisor::<1>::new();
    supervisor
        .register(ServiceSpec {
            id: ServiceId::new(1).unwrap(),
            name: ServiceName::new("compiler").unwrap(),
            kind: ServiceKind::Compiler,
            image_id: 1,
            capability_profile: 0,
            restart: RestartPolicy::NEVER,
        })
        .unwrap();
    let point = CrashPoint::new(
        CrashDomain::CompilerJob,
        CrashBoundary::ServiceRestart,
        1,
    );
    let mut harness = CrashHarness::new(Some(point));
    assert_eq!(
        supervisor.start_with_interruption(
            ServiceId::new(1).unwrap(),
            &mut Runtime,
            &mut harness,
        ),
        Err(SupervisorError::Interrupted)
    );
}
