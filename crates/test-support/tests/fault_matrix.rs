use synos_test_support::fault_matrix::{
    matrix, Fault, FaultInjectionController, FaultMatrix, FaultTarget, MatrixError,
    RecoveryEvidence, Workflow, MATRIX_SIZE,
};

#[test]
fn matrix_has_every_fault_and_workflow_cell() {
    let cells: Vec<_> = matrix().collect();

    assert_eq!(cells.len(), MATRIX_SIZE);
    assert_eq!(cells.first().map(|cell| cell.fault), Some(Fault::PowerLoss));
    assert_eq!(cells.first().map(|cell| cell.workflow), Some(Workflow::Boot));
    assert_eq!(cells.last().map(|cell| cell.fault), Some(Fault::DependencyOutage));
    assert_eq!(cells.last().map(|cell| cell.workflow), Some(Workflow::Device));
    assert!(cells.iter().all(|cell| cell.rto_budget_ms > 0));
    assert!(cells.iter().all(|cell| !cell.behavior.detail.is_empty()));
}

#[test]
fn controller_injects_only_its_selected_cell_once() {
    let target = FaultTarget::new(Fault::PacketLoss, Workflow::Rpc);
    let mut controller = FaultInjectionController::new(target);

    assert!(controller
        .checkpoint(Fault::PacketLoss, Workflow::Store, 100)
        .is_ok());
    let injected = controller
        .checkpoint(Fault::PacketLoss, Workflow::Rpc, 120)
        .expect_err("selected fault must inject");
    assert_eq!(injected.target, target);
    assert_eq!(injected.detected_at_ms, 120);
    assert!(controller
        .checkpoint(Fault::PacketLoss, Workflow::Rpc, 130)
        .is_ok());
    assert!(controller.fired());
}

#[test]
fn complete_matrix_accepts_bounded_recovery_evidence() {
    let mut results = FaultMatrix::<MATRIX_SIZE>::new();

    for cell in matrix() {
        let target = FaultTarget::new(cell.fault, cell.workflow);
        let evidence = RecoveryEvidence::observed(
            target,
            1_000,
            1_000 + cell.rto_budget_ms,
            cell.behavior,
            true,
            true,
            true,
            true,
            true,
            false,
        );
        results.record(evidence).expect("cell evidence should pass");
    }

    assert_eq!(results.recorded(), MATRIX_SIZE);
    assert!(results.complete().is_ok());
}

#[test]
fn evidence_rejects_rto_and_continuity_failures() {
    let target = FaultTarget::new(Fault::PowerLoss, Workflow::Boot);
    let cell = target.cell();

    let slow = RecoveryEvidence::observed(
        target,
        0,
        cell.rto_budget_ms + 1,
        cell.behavior,
        true,
        true,
        true,
        true,
        true,
        false,
    );
    assert!(matches!(slow.validate(), Err(MatrixError::RtoExceeded { .. })));

    let unsafe_recovery = RecoveryEvidence::observed(
        target,
        0,
        1,
        cell.behavior,
        true,
        true,
        false,
        true,
        true,
        false,
    );
    assert_eq!(unsafe_recovery.validate(), Err(MatrixError::CapabilityContinuityLost));
}
