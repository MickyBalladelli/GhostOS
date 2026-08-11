use synos_update::{
    DrainAuditEvent, DrainCoordinator, DrainObservation, DrainPhase, DrainPlan, DrainProgress,
    DrainResource, DrainRuntime, DrainState, DrainTarget, ForcedCleanup,
};

struct Runtime {
    observation: DrainObservation,
    cleanup: ForcedCleanup,
    quiesced: usize,
    handoffs: usize,
    force_calls: usize,
    audits: Vec<DrainAuditEvent>,
}

impl Runtime {
    fn clean() -> Self {
        Self {
            observation: DrainObservation::default(),
            cleanup: ForcedCleanup::CLEAN,
            quiesced: 0,
            handoffs: 0,
            force_calls: 0,
            audits: Vec::new(),
        }
    }
}

impl DrainRuntime for Runtime {
    type Error = ();

    fn quiesce(
        &mut self,
        _target: DrainTarget,
        _resource: DrainResource,
    ) -> Result<(), Self::Error> {
        self.quiesced += 1;
        Ok(())
    }

    fn request_handoff(
        &mut self,
        _target: DrainTarget,
        _resource: DrainResource,
    ) -> Result<(), Self::Error> {
        self.handoffs += 1;
        Ok(())
    }

    fn observe(&mut self, _target: DrainTarget) -> Result<DrainObservation, Self::Error> {
        Ok(self.observation)
    }

    fn force_cleanup(
        &mut self,
        _target: DrainTarget,
        _resource: DrainResource,
    ) -> Result<ForcedCleanup, Self::Error> {
        self.force_calls += 1;
        self.observation = DrainObservation::default();
        Ok(self.cleanup)
    }

    fn record_audit(&mut self, event: DrainAuditEvent) -> Result<(), Self::Error> {
        self.audits.push(event);
        Ok(())
    }
}

fn target() -> DrainTarget {
    DrainTarget::new(7, 3).expect("valid drain target")
}

#[test]
fn cooperative_drain_covers_every_resource_and_completes_cleanly() {
    let mut runtime = Runtime::clean();
    let mut coordinator = DrainCoordinator::new();
    coordinator
        .begin(&mut runtime, target(), DrainPlan::defaults(), 100)
        .expect("begin drain");

    let progress = coordinator.step(&mut runtime, 101).expect("step drain");
    let DrainProgress::Complete(receipt) = progress else {
        panic!("clean drain must complete")
    };
    assert!(!receipt.forced);
    assert_eq!(receipt.resources, 6);
    assert_eq!(runtime.quiesced, 6);
    assert_eq!(runtime.handoffs, 6);
    assert_eq!(runtime.force_calls, 0);
    assert_eq!(coordinator.state(), DrainState::Complete);
    assert!(runtime.audits.iter().any(|event| event.phase == DrainPhase::Completed));
}

#[test]
fn expired_grace_period_forces_cleanup_then_completes() {
    let mut runtime = Runtime::clean();
    runtime.observation.processes = 1;
    let mut coordinator = DrainCoordinator::new();
    let plan = DrainPlan::new(10, 10).expect("valid drain plan");
    coordinator.begin(&mut runtime, target(), plan, 100).unwrap();

    assert!(matches!(
        coordinator.step(&mut runtime, 105).unwrap(),
        DrainProgress::Waiting { .. }
    ));
    assert!(matches!(
        coordinator.step(&mut runtime, 110).unwrap(),
        DrainProgress::Forced { .. }
    ));
    let progress = coordinator.step(&mut runtime, 111).unwrap();
    let DrainProgress::Complete(receipt) = progress else {
        panic!("forced cleanup must complete")
    };
    assert!(receipt.forced);
    assert_eq!(runtime.force_calls, 6);
    assert!(coordinator.forced_cleanup().safe());
}

#[test]
fn forced_termination_rejects_live_locks_or_partial_publication() {
    let mut runtime = Runtime::clean();
    runtime.observation.queues = 1;
    runtime.cleanup = ForcedCleanup {
        stale_capabilities_fenced: true,
        ownership_released: true,
        live_locks: true,
        partial_publications: false,
    };
    let mut coordinator = DrainCoordinator::new();
    coordinator
        .begin(
            &mut runtime,
            target(),
            DrainPlan::new(1, 1).unwrap(),
            0,
        )
        .unwrap();

    let error = coordinator.step(&mut runtime, 1).unwrap_err();
    assert!(matches!(
        error,
        synos_update::DrainError::ForceCleanupIncomplete(_)
    ));
    assert_eq!(coordinator.state(), DrainState::Failed);
}

#[test]
fn backwards_clock_fails_closed() {
    let mut runtime = Runtime::clean();
    let mut coordinator = DrainCoordinator::new();
    coordinator.begin(&mut runtime, target(), DrainPlan::defaults(), 10).unwrap();
    assert!(matches!(
        coordinator.step(&mut runtime, 9),
        Err(synos_update::DrainError::ClockReversed)
    ));
    assert_eq!(coordinator.state(), DrainState::Failed);
}
