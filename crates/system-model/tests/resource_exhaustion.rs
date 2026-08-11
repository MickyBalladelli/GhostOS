use synos_system_model::quota::{
    QuotaCharge, QuotaLedger, QuotaPolicy, QuotaRejection, QuotaResource,
};

const RESOURCES: [QuotaResource; 8] = [
    QuotaResource::Memory,
    QuotaResource::Cpu,
    QuotaResource::Ipc,
    QuotaResource::Storage,
    QuotaResource::Network,
    QuotaResource::Log,
    QuotaResource::Audit,
    QuotaResource::ControlPlane,
];

fn bounded_policy() -> QuotaPolicy {
    QuotaPolicy::new([1; 8])
}

#[test]
fn hostile_tenant_exhaustion_isolated_from_neighbors_and_recovery() {
    let mut hostile = QuotaLedger::new(bounded_policy());
    let mut unrelated = QuotaLedger::new(bounded_policy());
    let mut recovery = QuotaLedger::new(bounded_policy());

    for resource in RESOURCES {
        hostile
            .reserve(resource, 1)
            .expect("hostile tenant gets its allowance");
        assert_eq!(
            hostile.reserve(resource, 1),
            Err(QuotaRejection::LimitExceeded {
                resource,
                requested: 1,
                consumed: 1,
                limit: 1,
            })
        );
        assert_eq!(hostile.usage().consumed(resource), 1);

        unrelated
            .reserve(resource, 1)
            .expect("unrelated tenant keeps its allowance");
        recovery
            .reserve(resource, 1)
            .expect("recovery traffic keeps its allowance");
    }
}

#[test]
fn hostile_multi_resource_reservation_fails_atomically() {
    let mut hostile = QuotaLedger::new(bounded_policy());
    let mut charges = [QuotaCharge::new(QuotaResource::Memory, 1); RESOURCES.len() + 1];
    for (charge, resource) in charges[..RESOURCES.len()].iter_mut().zip(RESOURCES) {
        *charge = QuotaCharge::new(resource, 1);
    }
    charges[RESOURCES.len()] = QuotaCharge::new(QuotaResource::ControlPlane, 2);

    assert!(matches!(
        hostile.reserve_all(&charges),
        Err(QuotaRejection::LimitExceeded {
            resource: QuotaResource::ControlPlane,
            requested: 2,
            consumed: 1,
            limit: 1,
        })
    ));
    assert_eq!(hostile.usage().consumption(), [0; 8]);
}
