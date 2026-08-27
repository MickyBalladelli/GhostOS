// Inventory: coverage_59_6.rs (legacy roadmap section 59).
use ghostos_app::{
    AppManifest, ApplicationEvent, ApplicationId, ApplicationRuntime, ApplicationState,
    ApplicationSupervisor, BoundedText, CapabilityKind, CapabilityPolicy, CapabilityRights,
    CapabilityRule, ManifestError,
};
use ghostos_init::{CrashReason, ExitReason, ProcessId};

const MANIFEST: &str = r#"
schema = 1
[application]
name = "demo"
image = "0x1"
kind = "service"
[[capability]]
resource = "disk"
kind = "file"
rights = ["read", "write"]
required = true
[runtime]
placement = "any"
restart = "on-failure"
max_restarts = 1
window_us = 100
initial_backoff_us = 10
max_backoff_us = 20
"#;

#[derive(Default)]
struct Runtime {
    next_process: u64,
    spawned: Vec<ProcessId>,
    fenced: Vec<ProcessId>,
}

impl ApplicationRuntime for Runtime {
    type Error = ();

    fn spawn(&mut self, _request: ghostos_app::AppSpawnRequest<'_>) -> Result<ProcessId, Self::Error> {
        self.next_process += 1;
        let process = ProcessId::new(self.next_process).unwrap();
        self.spawned.push(process);
        Ok(process)
    }

    fn fence_process(&mut self, process: ProcessId) -> Result<(), Self::Error> {
        self.fenced.push(process);
        Ok(())
    }
}

fn policy() -> CapabilityPolicy<2> {
    let mut policy = CapabilityPolicy::new();
    policy
        .allow(CapabilityRule {
            resource: BoundedText::new("disk").unwrap(),
            kind: CapabilityKind::File,
            maximum_rights: CapabilityRights::READ.union(CapabilityRights::WRITE),
        })
        .unwrap();
    policy
}

#[test]
fn application_manifest_and_capability_policy_validate_requests() {
    let manifest = AppManifest::parse(MANIFEST).expect("parse application manifest");
    assert_eq!(manifest.name().as_str(), "demo");
    assert_eq!(manifest.image(), 1);
    assert_eq!(manifest.capabilities().count(), 1);
    assert!(manifest.runtime().restart_mode == ghostos_app::RestartMode::OnFailure);
    assert_eq!(AppManifest::parse("schema = 2"), Err(ManifestError::MissingField));

    let mut restricted = CapabilityPolicy::<2>::new();
    restricted
        .allow(CapabilityRule {
            resource: BoundedText::new("disk").unwrap(),
            kind: CapabilityKind::File,
            maximum_rights: CapabilityRights::READ,
        })
        .unwrap();
    assert!(matches!(restricted.authorize(&manifest), Err(ghostos_app::PolicyError::RightsEscalation)));
}

#[test]
fn supervisor_restarts_after_crash_then_fails_after_restart_budget() {
    let manifest = AppManifest::parse(MANIFEST).unwrap();
    let mut supervisor = ApplicationSupervisor::<2>::new();
    let application = ApplicationId::new(1).unwrap();
    supervisor.register(application, manifest, &policy()).unwrap();
    let mut runtime = Runtime::default();
    let started = supervisor.start(application, &mut runtime).unwrap();
    let process = match started {
        ApplicationEvent::Started { process, generation, .. } => {
            assert_eq!(generation, 1);
            process
        }
        event => panic!("unexpected start event: {event:?}"),
    };
    assert_eq!(supervisor.status(application).unwrap().state, ApplicationState::Running);
    let scheduled = supervisor
        .report_exit(process, ExitReason::Crash(CrashReason::Panic), 0, &mut runtime)
        .unwrap();
    assert!(matches!(scheduled, ApplicationEvent::RestartScheduled { at_us: 10, .. }));
    assert_eq!(runtime.fenced, vec![process]);
    assert!(matches!(supervisor.tick(9, &mut runtime), Ok(None)));
    assert!(matches!(supervisor.tick(10, &mut runtime), Ok(Some(ApplicationEvent::Started { generation: 2, .. }))));

    let second_process = *runtime.spawned.last().unwrap();
    let failed = supervisor
        .report_exit(second_process, ExitReason::Crash(CrashReason::Watchdog), 10, &mut runtime)
        .unwrap();
    assert!(matches!(failed, ApplicationEvent::Failed { reason: Some(CrashReason::Watchdog), .. }));
    assert_eq!(supervisor.status(application).unwrap().state, ApplicationState::Failed);
}
