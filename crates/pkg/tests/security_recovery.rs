use synos_pkg::{
    ApplicationPackageManifest, PackageDaemon, PackageError, SigningKey,
    SystemConfiguration, application_bundle_size, bundle_size, encode_application_bundle,
    encode_bundle,
};
use synos_synfs::SynFs;
use synos_system_model::{ContentId, Error as ModelError, RepositoryError};

fn filesystem() -> SynFs<64> {
    let mut filesystem = SynFs::new();
    filesystem.create_directory("system/store", true).unwrap();
    filesystem.create_directory("system/manifests", true).unwrap();
    filesystem
}

fn bundle(payload: &[u8], key: SigningKey) -> (Vec<u8>, ContentId) {
    let mut encoded = vec![0; bundle_size(payload.len(), 0).unwrap()];
    let info = encode_bundle(payload, 0, &[], key, &mut encoded).unwrap();
    (encoded, info.package)
}

#[test]
fn package_key_rotation_revocation_and_generation_downgrade_are_fenced() {
    let old_key = SigningKey::new([41; 32]);
    let new_key = SigningKey::new([42; 32]);
    let (old_bundle, old_package) = bundle(b"old package", old_key);
    let (new_bundle, new_package) = bundle(b"new package", new_key);
    let mut filesystem = filesystem();
    let mut daemon = PackageDaemon::<4, 2>::new();
    daemon.trust_key(old_key).unwrap();
    daemon.trust_key(new_key).unwrap();
    let mut verification = [0; 128];
    daemon
        .install_bundle(&mut filesystem, &old_bundle, &mut verification)
        .unwrap();
    daemon
        .install_bundle(&mut filesystem, &new_bundle, &mut verification)
        .unwrap();

    let old_receipt = daemon.authorize_instantiation(old_package).unwrap();
    daemon.revoke_key(old_key.id()).unwrap();
    assert_eq!(
        daemon.validate_instantiation(old_receipt),
        Err(PackageError::InstantiationDenied)
    );
    assert_eq!(
        daemon.verify_bundle(&old_bundle),
        Err(PackageError::UnknownSigningKey)
    );
    assert!(daemon.is_instantiation_authorized(new_package));

    let mut first = SystemConfiguration::new(1);
    first.bind("init", new_package).unwrap();
    daemon.activate(&mut filesystem, &first).unwrap();
    let mut second = SystemConfiguration::new(2);
    second.bind("init", new_package).unwrap();
    daemon.activate(&mut filesystem, &second).unwrap();
    assert!(matches!(
        daemon.activate(&mut filesystem, &first),
        Err(PackageError::Repository(RepositoryError::Model(
            ModelError::StaleRevision
        )))
    ));
}

#[test]
fn application_signature_rotation_replay_and_rollback_are_fenced() {
    let old_key = SigningKey::new([51; 32]);
    let new_key = SigningKey::new([52; 32]);
    let (old_inner, old_package) = bundle(b"old application", old_key);
    let (new_inner, new_package) = bundle(b"new application", new_key);
    let old_metadata = ApplicationPackageManifest::new(
        1,
        1,
        1,
        "demo",
        0,
        4096,
        100,
        4096,
        ContentId::hash(b"old symbols"),
        ContentId::hash(b"old build"),
    )
    .unwrap();
    let new_metadata = ApplicationPackageManifest::new(
        1,
        1,
        1,
        "demo",
        0,
        4096,
        100,
        4096,
        ContentId::hash(b"new symbols"),
        ContentId::hash(b"new build"),
    )
    .unwrap();
    let mut old_app = vec![0; application_bundle_size(old_inner.len()).unwrap()];
    encode_application_bundle(&old_inner, old_metadata, old_key, &mut old_app).unwrap();
    let mut new_app = vec![0; application_bundle_size(new_inner.len()).unwrap()];
    encode_application_bundle(&new_inner, new_metadata, new_key, &mut new_app).unwrap();

    let mut filesystem = filesystem();
    let mut daemon = PackageDaemon::<4, 2>::new();
    daemon.trust_key(old_key).unwrap();
    daemon.trust_key(new_key).unwrap();
    let mut verification = [0; 128];
    daemon
        .install_application_bundle(&mut filesystem, &old_app, &mut verification)
        .unwrap();
    let mut first = SystemConfiguration::new(1);
    first.bind("demo", old_package).unwrap();
    daemon.activate(&mut filesystem, &first).unwrap();
    let replay = daemon
        .install_application_bundle(&mut filesystem, &old_app, &mut verification)
        .unwrap();
    assert_eq!(replay.package, old_package);
    assert_eq!(daemon.application_manifest(old_package), Some(old_metadata));
    let old_receipt = daemon.authorize_instantiation(old_package).unwrap();
    daemon.revoke_key(old_key.id()).unwrap();
    assert_eq!(
        daemon.validate_instantiation(old_receipt),
        Err(PackageError::InstantiationDenied)
    );

    daemon
        .install_application_bundle(&mut filesystem, &new_app, &mut verification)
        .unwrap();
    assert_eq!(daemon.application_manifest(new_package).unwrap(), new_metadata);
    assert!(daemon.is_instantiation_authorized(new_package));
    let mut second = SystemConfiguration::new(2);
    second.bind("demo", new_package).unwrap();
    daemon.activate(&mut filesystem, &second).unwrap();
    assert!(matches!(
        daemon.activate(&mut filesystem, &first),
        Err(PackageError::Repository(RepositoryError::Model(
            ModelError::StaleRevision
        )))
    ));
}

#[test]
fn package_trust_root_recovery_keeps_old_bytes_but_requires_new_key() {
    let old_key = SigningKey::new([61; 32]);
    let recovered_key = SigningKey::new([62; 32]);
    let (old_bundle, old_package) = bundle(b"recovery package", old_key);
    let mut filesystem = filesystem();
    let mut daemon = PackageDaemon::<4, 2>::new();
    daemon.trust_key(old_key).unwrap();
    daemon
        .install_bundle(&mut filesystem, &old_bundle, &mut [0; 128])
        .unwrap();
    daemon.revoke_key(old_key.id()).unwrap();
    assert!(daemon.contains(old_package));
    assert_eq!(
        daemon.authorize_instantiation(old_package),
        Err(PackageError::InstantiationDenied)
    );
    daemon.trust_key(recovered_key).unwrap();
    let (recovered_bundle, recovered_package) = bundle(b"recovered package", recovered_key);
    daemon
        .install_bundle(&mut filesystem, &recovered_bundle, &mut [0; 128])
        .unwrap();
    assert!(daemon.is_instantiation_authorized(recovered_package));
}
