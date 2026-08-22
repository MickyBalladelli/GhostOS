use ghostos_inspect::{
    CompatibilityMetadata, DependencyMetadata, HealthState, HealthTransport,
    OperationalHealth, QuotaMetadata, RecoveryMetadata, RunbookError, RunbookGenerator,
    RunbookLink, RunbookMetadata, RunbookSource, ServiceHealthMetadata,
};
use ghostos_observability::{Alert, AlertLevel, AlertRegistry, CorrelationId, TelemetryDimensions};
use ghostos_system_model::quota::QuotaResource;
use ghostos_update::{ArtifactKind, CompatibilityContract, CompatibilityMode, ReleaseBundle};

fn bundle() -> ReleaseBundle {
    let artifact = |kind| ghostos_update::ArtifactSpec {
        kind,
        version: 2,
        digest: [kind as u8 + 1; 32],
        compatibility: CompatibilityContract {
            minimum_peer_version: 1,
            maximum_peer_version: 2,
            mode: CompatibilityMode::ReadWrite,
        },
    };
    ReleaseBundle {
        release_id: [1; 32],
        artifacts: [
            artifact(ArtifactKind::Kernel),
            artifact(ArtifactKind::Service),
            artifact(ArtifactKind::Package),
            artifact(ArtifactKind::Client),
            artifact(ArtifactKind::Schema),
            artifact(ArtifactKind::ClusterProtocol),
        ],
        migration_id: 7,
        rollback_target: [2; 32],
        capability_epoch: 3,
        audit_sequence: 4,
        signature: [5; 64],
    }
}

fn metadata() -> RunbookMetadata {
    RunbookMetadata {
        alert_code: 9,
        severity: AlertLevel::Critical,
        source: RunbookSource::ServiceHealth,
        health: ServiceHealthMetadata::from_sample(
            OperationalHealth::new(
                100,
                1,
                HealthTransport::Http,
                HealthState::Degraded,
                4,
                8,
                0,
                1,
                true,
            )
            .unwrap(),
        ),
        dependency: DependencyMetadata {
            dependency: 2,
            required: true,
            available: false,
            timeout_us: 50,
            degraded_behavior: "serve the last healthy snapshot",
        },
        recovery: RecoveryMetadata {
            service: 1,
            generation: 4,
            checkpoint_available: true,
            recovery_deadline_us: 1_000,
            rollback_generation: 3,
        },
        quota: QuotaMetadata {
            resource: QuotaResource::ControlPlane,
            consumed: 2,
            limit: 8,
            recovery_reserved: 1,
        },
        compatibility: CompatibilityMetadata::from_bundle(bundle(), ArtifactKind::Service)
            .unwrap(),
        diagnosis: RunbookLink::new(1, "inspect health", "SHOW-HEALTH --service 1"),
        safe_action: RunbookLink::new(2, "enter degraded mode", "service pause --safe"),
        rollback: RunbookLink::new(3, "restore prior generation", "service rollback --last-good"),
        recovery_proof: RunbookLink::new(4, "verify recovery", "SHOW-AUDIT --correlation alert"),
    }
}

fn alert() -> Alert {
    Alert {
        sequence: 0,
        timestamp: 100,
        level: AlertLevel::Critical,
        code: 9,
        dimensions: TelemetryDimensions {
            node: 1,
            ..TelemetryDimensions::default()
        },
        correlation: CorrelationId::from_raw(1),
    }
}

#[test]
fn every_generated_alert_has_four_operator_links() {
    let mut generator = RunbookGenerator::<1>::new();
    generator.register(metadata()).unwrap();
    let mut alerts = AlertRegistry::<1>::new();
    alerts.push(alert()).unwrap();
    let mut output = [None];

    assert_eq!(generator.generate(&alerts, &mut output).unwrap(), 1);
    let runbook = output[0].unwrap();
    assert_eq!(runbook.diagnosis().id, 1);
    assert_eq!(runbook.safe_action().id, 2);
    assert_eq!(runbook.rollback().id, 3);
    assert_eq!(runbook.recovery_proof().id, 4);
}

#[test]
fn incomplete_runbook_is_rejected_before_registration() {
    let mut generator = RunbookGenerator::<1>::new();
    let mut invalid = metadata();
    invalid.rollback = RunbookLink::new(0, "", "");

    assert_eq!(generator.register(invalid), Err(RunbookError::MissingRollback));
    assert_eq!(generator.register(metadata()), Ok(()));
}
