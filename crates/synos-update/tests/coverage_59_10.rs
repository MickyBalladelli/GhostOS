use synos_pkg::{PackageDaemon, SigningKey, SystemConfiguration, bundle_size, encode_bundle};
use synos_status::Status;
use synos_synfs::SynFs;
use synos_system_model::{ContentId, RootManifest};
use synos_update::{HealthCheck, UpdateError, UpdateManager, UpdatePlan};

struct HealthCheckStub {
    fail: bool,
}

impl HealthCheck for HealthCheckStub {
    fn check<const BLOCKS: usize>(
        &mut self,
        _filesystem: &SynFs<BLOCKS>,
        _configuration: &RootManifest,
    ) -> Result<(), Status> {
        if self.fail { Err(Status::BUSY) } else { Ok(()) }
    }
}

fn install<const PACKAGES: usize, const KEYS: usize>(
    daemon: &mut PackageDaemon<PACKAGES, KEYS>,
    filesystem: &mut SynFs<256>,
    key: SigningKey,
    payload: &[u8],
) -> ContentId {
    let required = bundle_size(payload.len(), 0).unwrap();
    let mut bundle = vec![0; required];
    let info = encode_bundle(payload, 0, &[], key, &mut bundle).unwrap();
    let mut verification = [0; 128];
    assert_eq!(daemon.install_bundle(filesystem, &bundle, &mut verification).unwrap(), info.package);
    info.package
}

#[test]
fn atomic_updates_health_failures_rollback_and_revision_guards() {
    let key = SigningKey::new([4; 32]);
    let mut filesystem = SynFs::<256>::new();
    filesystem.create_directory("system/store", true).unwrap();
    filesystem.create_directory("system/manifests", true).unwrap();
    let mut daemon = PackageDaemon::<4, 2>::new();
    daemon.trust_key(key).unwrap();
    let first = install(&mut daemon, &mut filesystem, key, b"first");
    let second = install(&mut daemon, &mut filesystem, key, b"second");

    let mut initial = SystemConfiguration::new(1);
    initial.bind("init", first).unwrap();
    let mut manager = UpdateManager::<2>::new();
    manager.apply(&mut filesystem, &mut daemon, UpdatePlan::new(initial), &mut HealthCheckStub { fail: false }).unwrap();
    assert_eq!(manager.history().count(), 1);

    let mut next = SystemConfiguration::new(2);
    next.bind("init", second).unwrap();
    assert_eq!(manager.apply(&mut filesystem, &mut daemon, UpdatePlan::new(next), &mut HealthCheckStub { fail: true }), Err(UpdateError::HealthCheck(Status::BUSY)));
    assert_eq!(daemon.active_configuration().unwrap().revision(), 1);

    let receipt = manager.apply(&mut filesystem, &mut daemon, UpdatePlan::new(next).expect_previous_revision(1), &mut HealthCheckStub { fail: false }).unwrap();
    assert_eq!(receipt.record.revision, 2);
    assert_eq!(manager.apply(&mut filesystem, &mut daemon, UpdatePlan::new(next), &mut HealthCheckStub { fail: false }), Err(UpdateError::AlreadyActive));
    let rollback = manager.rollback_last(&mut filesystem, &mut daemon).unwrap();
    assert!(rollback.rolled_back);
    assert_eq!(daemon.active_configuration().unwrap().revision(), 1);
}

#[test]
fn unsigned_or_untrusted_packages_cannot_cross_instantiation_gate() {
    let key = SigningKey::new([5; 32]);
    let other = SigningKey::new([6; 32]);
    let payload = b"package";
    let required = bundle_size(payload.len(), 0).unwrap();
    let mut bundle = vec![0; required];
    let info = encode_bundle(payload, 0, &[], key, &mut bundle).unwrap();
    let daemon = PackageDaemon::<2, 1>::new();
    assert_eq!(daemon.verify_bundle(&bundle), Err(synos_pkg::PackageError::UnknownSigningKey));
    assert_eq!(daemon.authorize_instantiation(info.package), Err(synos_pkg::PackageError::InstantiationDenied));
    assert_eq!(synos_pkg::PackageBundle::decode(&bundle).unwrap().verify(other), Err(synos_pkg::PackageError::UnknownSigningKey));
}
