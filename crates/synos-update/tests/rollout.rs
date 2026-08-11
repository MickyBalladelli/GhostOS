use synos_update::{
    ArtifactKind, ArtifactSpec, CompatibilityContract, CompatibilityMode, HealthReport,
    ReleaseBundle, ReleaseVerifier, RollbackReport, RolloutAuditEvent, RolloutCoordinator,
    RolloutError, RolloutPhase, RolloutPlan, RolloutRuntime, RolloutStrategy, RolloutTarget,
    ARTIFACT_COUNT,
};

fn bundle(id: u8, version: u64, minimum_peer_version: u64) -> ReleaseBundle {
    let artifacts = ArtifactKind::ALL.map(|kind| ArtifactSpec {
        kind,
        version,
        digest: [id.wrapping_add(kind as u8); 32],
        compatibility: CompatibilityContract {
            minimum_peer_version,
            maximum_peer_version: version,
            mode: CompatibilityMode::ReadWrite,
        },
    });
    ReleaseBundle {
        release_id: [id; 32],
        artifacts,
        migration_id: u64::from(id),
        rollback_target: [id.wrapping_add(1); 32],
        capability_epoch: u64::from(id),
        audit_sequence: u64::from(id),
        signature: [id; 64],
    }
}

struct Verifier;

impl ReleaseVerifier for Verifier {
    type Error = ();

    fn verify(&mut self, bundle: &ReleaseBundle) -> Result<(), Self::Error> {
        assert_ne!(bundle.signature, [0; 64]);
        Ok(())
    }
}

struct Runtime {
    fail_health: bool,
    stages: usize,
    activations: usize,
    switches: usize,
    drains: usize,
    rollbacks: usize,
    audits: Vec<RolloutAuditEvent>,
}

impl Runtime {
    fn healthy() -> HealthReport {
        HealthReport {
            observed_ms: 1,
            ready: true,
            mixed_version_compatible: true,
            data_continuity: true,
            capability_continuity: true,
            stale_capabilities_fenced: true,
            audit_continuity: true,
            duplicate_side_effects: false,
        }
    }
}

impl RolloutRuntime for Runtime {
    type Error = ();

    fn stage(
        &mut self,
        _bundle: &ReleaseBundle,
        _target: RolloutTarget,
    ) -> Result<(), Self::Error> {
        self.stages += 1;
        Ok(())
    }

    fn activate(
        &mut self,
        _bundle: &ReleaseBundle,
        _target: RolloutTarget,
    ) -> Result<(), Self::Error> {
        self.activations += 1;
        Ok(())
    }

    fn switch_traffic(&mut self, _target: RolloutTarget) -> Result<(), Self::Error> {
        self.switches += 1;
        Ok(())
    }

    fn drain(&mut self, _target: RolloutTarget) -> Result<(), Self::Error> {
        self.drains += 1;
        Ok(())
    }

    fn health(&mut self, _target: RolloutTarget) -> Result<HealthReport, Self::Error> {
        if self.fail_health {
            return Ok(HealthReport {
                ready: false,
                ..Self::healthy()
            })
        }
        Ok(Self::healthy())
    }

    fn rollback(
        &mut self,
        _running: &ReleaseBundle,
        _rollback_target: [u8; 32],
        _target: RolloutTarget,
        _capability_epoch: u64,
    ) -> Result<RollbackReport, Self::Error> {
        self.rollbacks += 1;
        Ok(RollbackReport {
            data_continuity: true,
            capability_continuity: true,
            stale_capabilities_fenced: true,
            audit_continuity: true,
            duplicate_side_effects: false,
        })
    }

    fn record_audit(&mut self, event: RolloutAuditEvent) -> Result<(), Self::Error> {
        self.audits.push(event);
        Ok(())
    }
}

#[test]
fn all_rollout_strategies_stage_activate_and_preserve_receipts() {
    let running = bundle(1, 1, 1);
    let target = bundle(2, 2, 1);
    let plans = [
        RolloutPlan::rolling(3),
        RolloutPlan::canary(3, 1),
        RolloutPlan::blue_green(),
        RolloutPlan::emergency(),
    ];

    for plan in plans {
        let mut runtime = Runtime {
            fail_health: false,
            stages: 0,
            activations: 0,
            switches: 0,
            drains: 0,
            rollbacks: 0,
            audits: Vec::new(),
        };
        let mut verifier = Verifier;
        let mut coordinator = RolloutCoordinator::<8>::new();
        let receipt = coordinator
            .execute(&mut runtime, &mut verifier, running, target, plan)
            .expect("rollout should complete");

        assert_eq!(receipt.strategy, plan.strategy);
        assert_eq!(receipt.release_id, target.release_id);
        assert_eq!(receipt.previous_release_id, running.release_id);
        assert!(!receipt.rolled_back);
        assert_eq!(coordinator.state(), synos_update::RolloutState::Completed);
        assert!(runtime.stages > 0);
        assert_eq!(runtime.stages, runtime.activations);
        assert!(runtime.audits.iter().any(|event| event.phase == RolloutPhase::Completed));
        if plan.strategy == RolloutStrategy::BlueGreen {
            assert_eq!(runtime.switches, 1);
        }
    }
}

#[test]
fn incompatible_mixed_version_is_rejected_before_stage() {
    let running = bundle(1, 1, 1);
    let target = bundle(2, 2, 2);
    let mut runtime = Runtime {
        fail_health: false,
        stages: 0,
        activations: 0,
        switches: 0,
        drains: 0,
        rollbacks: 0,
        audits: Vec::new(),
    };
    let mut verifier = Verifier;
    let mut coordinator = RolloutCoordinator::<8>::new();

    assert!(matches!(
        coordinator.execute(
            &mut runtime,
            &mut verifier,
            running,
            target,
            RolloutPlan::rolling(1),
        ),
        Err(RolloutError::Compatibility(_))
    ));
    assert_eq!(runtime.stages, 0);
}

#[test]
fn failed_health_check_rolls_back_and_checks_integrity() {
    let running = bundle(1, 1, 1);
    let target = bundle(2, 2, 1);
    let mut runtime = Runtime {
        fail_health: true,
        stages: 0,
        activations: 0,
        switches: 0,
        drains: 0,
        rollbacks: 0,
        audits: Vec::new(),
    };
    let mut verifier = Verifier;
    let mut coordinator = RolloutCoordinator::<8>::new();

    assert!(matches!(
        coordinator.execute(
            &mut runtime,
            &mut verifier,
            running,
            target,
            RolloutPlan::canary(2, 1),
        ),
        Err(RolloutError::HealthFailed(_))
    ));
    assert_eq!(runtime.rollbacks, 1);
    assert_eq!(coordinator.state(), synos_update::RolloutState::RolledBack);
}

#[test]
fn release_bundle_has_all_artifacts() {
    let release = bundle(7, 3, 1);
    assert_eq!(release.artifacts.len(), ARTIFACT_COUNT);
    assert!(release.validate().is_ok());
}
