use synos_status::{IntoStatus, Status};
use synos_system_model::quota::{
    QuotaCharge, QuotaLedger, QuotaPolicy, QuotaRejection, QuotaResource,
    QUOTA_RESOURCE_COUNT,
};

#[test]
fn one_policy_names_and_limits_all_resource_dimensions() {
    let resources = [
        QuotaResource::Memory,
        QuotaResource::Cpu,
        QuotaResource::Ipc,
        QuotaResource::Storage,
        QuotaResource::Network,
        QuotaResource::Log,
        QuotaResource::Audit,
        QuotaResource::ControlPlane,
    ];
    let limits = [10, 20, 30, 40, 50, 60, 70, 80];
    let policy = QuotaPolicy::new(limits);

    assert_eq!(resources.len(), QUOTA_RESOURCE_COUNT);
    assert_eq!(policy.limits(), limits);
    assert_eq!(QuotaResource::Memory.name(), "memory");
    assert_eq!(QuotaResource::Audit.name(), "audit");
    assert_eq!(QuotaResource::ControlPlane.name(), "control-plane");
    for (resource, limit) in resources.into_iter().zip(limits) {
        assert_eq!(policy.limit(resource), limit);
    }
}

#[test]
fn ledger_exposes_consumption_and_structured_rejection_reason() {
    let policy = QuotaPolicy::new([
        10,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
        u64::MAX,
    ]);
    let mut ledger = QuotaLedger::new(policy);

    ledger.reserve(QuotaResource::Memory, 7).unwrap();
    assert_eq!(ledger.usage().consumed(QuotaResource::Memory), 7);
    assert_eq!(ledger.remaining(QuotaResource::Memory), 3);
    assert_eq!(ledger.usage().consumption()[QuotaResource::Memory.index()], 7);

    let rejection = ledger.reserve(QuotaResource::Memory, 4).unwrap_err();
    assert_eq!(
        rejection,
        QuotaRejection::LimitExceeded {
            resource: QuotaResource::Memory,
            requested: 4,
            consumed: 7,
            limit: 10,
        }
    );
    assert_eq!(rejection.resource(), QuotaResource::Memory);
    assert_eq!(rejection.status(), Status::NO_SPACE);
    assert_eq!(ledger.usage().consumed(QuotaResource::Memory), 7);
}

#[test]
fn batch_reservation_and_release_are_atomic() {
    let policy = QuotaPolicy::new([10, 20, 30, 40, 50, 60, 70, 80]);
    let mut ledger = QuotaLedger::new(policy);
    let charges = [
        QuotaCharge::new(QuotaResource::Memory, 4),
        QuotaCharge::new(QuotaResource::Ipc, 8),
    ];
    ledger.reserve_all(&charges).unwrap();
    assert_eq!(ledger.usage().consumption(), [4, 0, 8, 0, 0, 0, 0]);

    let rejected = [
        QuotaCharge::new(QuotaResource::Storage, 20),
        QuotaCharge::new(QuotaResource::Network, 51),
    ];
    assert!(matches!(
        ledger.reserve_all(&rejected),
        Err(QuotaRejection::LimitExceeded {
            resource: QuotaResource::Network,
            requested: 51,
            consumed: 0,
            limit: 50,
        })
    ));
    assert_eq!(ledger.usage().consumption(), [4, 0, 8, 0, 0, 0, 0]);

    ledger.release_all(&charges).unwrap();
    assert_eq!(ledger.usage().consumption(), [0; QUOTA_RESOURCE_COUNT]);
    assert!(matches!(
        ledger.release(QuotaResource::Memory, 1),
        Err(QuotaRejection::ReleaseExceedsConsumption {
            resource: QuotaResource::Memory,
            released: 1,
            consumed: 0,
        })
    ));
}
