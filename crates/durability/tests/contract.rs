use ghostos_durability::{
    ContractError, DurabilityEvent, DurabilityTrace,
};

fn durable_transaction<const CAPACITY: usize>() -> DurabilityTrace<CAPACITY> {
    let mut trace = DurabilityTrace::new();
    trace
        .record(DurabilityEvent::ApplicationWrite { transaction: 1 })
        .unwrap();
    trace
        .record(DurabilityEvent::SynFsWrite { transaction: 1 })
        .unwrap();
    trace
        .record(DurabilityEvent::Rename { transaction: 1 })
        .unwrap();
    trace
        .record(DurabilityEvent::Commit { transaction: 1 })
        .unwrap();
    trace
        .record(DurabilityEvent::StorageDaemonWrite { transaction: 1 })
        .unwrap();
    trace
        .record(DurabilityEvent::CacheFlush { transaction: 1 })
        .unwrap();
    trace
        .record(DurabilityEvent::BlockDataWrite { transaction: 1 })
        .unwrap();
    trace
        .record(DurabilityEvent::BlockCommitRecord { transaction: 1 })
        .unwrap();
    trace
        .record(DurabilityEvent::BlockFlush { transaction: 1 })
        .unwrap();
    trace
        .record(DurabilityEvent::SyncAcknowledged { transaction: 1 })
        .unwrap();
    trace.record(DurabilityEvent::PowerLoss).unwrap();
    trace
        .record(DurabilityEvent::Recovered { transaction: 1 })
        .unwrap();
    trace
}

#[test]
fn contract_accepts_only_fenced_commit_as_recovered() {
    assert_eq!(durable_transaction::<16>().verify(), Ok(()));
}

#[test]
fn contract_rejects_commit_record_before_data() {
    let mut trace = DurabilityTrace::<16>::new();
    for event in [
        DurabilityEvent::ApplicationWrite { transaction: 1 },
        DurabilityEvent::SynFsWrite { transaction: 1 },
        DurabilityEvent::Commit { transaction: 1 },
        DurabilityEvent::StorageDaemonWrite { transaction: 1 },
        DurabilityEvent::CacheFlush { transaction: 1 },
        DurabilityEvent::BlockCommitRecord { transaction: 1 },
        DurabilityEvent::BlockDataWrite { transaction: 1 },
        DurabilityEvent::BlockFlush { transaction: 1 },
        DurabilityEvent::SyncAcknowledged { transaction: 1 },
    ] {
        trace.record(event).unwrap();
    }
    assert_eq!(
        trace.verify(),
        Err(ContractError::InvalidOrder { transaction: 1 })
    );
}

#[test]
fn contract_rejects_power_loss_recovery_without_sync_ack() {
    let mut trace = DurabilityTrace::<8>::new();
    trace
        .record(DurabilityEvent::PowerLoss)
        .expect("power loss fits");
    trace
        .record(DurabilityEvent::Recovered { transaction: 7 })
        .expect("recovery fits");
    assert_eq!(
        trace.verify(),
        Err(ContractError::RecoveredVolatile { transaction: 7 })
    );
}
