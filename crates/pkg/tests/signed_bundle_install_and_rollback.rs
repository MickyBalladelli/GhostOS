use ghostos_pkg::{
    bundle_size, encode_bundle, PackageBundle, PackageDaemon, PackageError, SigningKey,
    SystemConfiguration,
};
use ghostos_ghostfs::SynFs;
use ghostos_system_model::ContentId;
use ghostos_test_support::crash::{CrashBoundary, CrashDomain, CrashHarness, CrashPoint};

#[test]
fn signed_bundle_round_trip_detects_tampering_and_trust_failures() {
    let key = SigningKey::new([7; 32]);
    let dependency = ContentId::hash(b"dependency");
    let payload = b"package payload";
    let required = bundle_size(payload.len(), 1).unwrap();
    let mut encoded = vec![0; required];
    let info = encode_bundle(payload, 0, &[dependency], key, &mut encoded).unwrap();
    let bundle = PackageBundle::decode(&encoded).unwrap();
    assert_eq!(bundle.info(), info);
    bundle.verify(key).expect("verify signed bundle");

    encoded[required - 1] ^= 1;
    assert!(matches!(PackageBundle::decode(&encoded), Err(PackageError::CorruptBundle)));

    let mut clean = vec![0; required];
    encode_bundle(payload, 0, &[dependency], key, &mut clean).unwrap();
    let other_key = SigningKey::new([8; 32]);
    assert_eq!(PackageBundle::decode(&clean).unwrap().verify(other_key), Err(PackageError::UnknownSigningKey));
}

#[test]
fn package_install_and_activation_survive_duplicate_and_rollback_attempts() {
    let key = SigningKey::new([9; 32]);
    let payload = b"immutable package";
    let required = bundle_size(payload.len(), 0).unwrap();
    let mut bundle = vec![0; required];
    let info = encode_bundle(payload, 0, &[], key, &mut bundle).unwrap();
    let mut filesystem = SynFs::<64>::new();
    filesystem
        .create_directory("system/store", true)
        .expect("create package store");
    filesystem
        .create_directory("system/manifests", true)
        .expect("create manifest store");
    let mut daemon = PackageDaemon::<4, 2>::new();
    daemon.trust_key(key).expect("trust package key");
    let mut verification = [0; 64];
    let installed = daemon
        .install_bundle(&mut filesystem, &bundle, &mut verification)
        .expect("install signed package");
    assert_eq!(installed, info.package);
    assert!(daemon.contains(installed));
    assert!(daemon.is_instantiation_authorized(installed));
    let receipt = daemon.authorize_instantiation(installed).unwrap();
    daemon.validate_instantiation(receipt).expect("validate receipt");

    let mut configuration = SystemConfiguration::new(1);
    configuration.bind("init", installed).expect("bind package");
    assert_eq!(configuration.bind("init", installed), Err(PackageError::DuplicateBinding));
    let point = CrashPoint::new(CrashDomain::PackageActivation, CrashBoundary::ManifestSlot, 1);
    let mut harness = CrashHarness::new(Some(point));
    assert_eq!(
        daemon.activate_with_interruption(&mut filesystem, &configuration, &mut harness),
        Err(PackageError::Interrupted)
    );
    daemon.activate(&mut filesystem, &configuration).expect("activate package root");
    assert!(daemon.active_configuration().is_some());
    assert!(matches!(
        daemon.activate(&mut filesystem, &configuration),
        Err(PackageError::Repository(ghostos_system_model::RepositoryError::Model(
            ghostos_system_model::Error::StaleRevision
        )))
    ));
}
