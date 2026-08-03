use synos_pkg::{PackageBundle, PackageDaemon, PackageError, SigningKey, bundle_size, encode_bundle};
use synos_synfs::SynFs;
use synos_system_model::ContentId;

#[test]
fn supply_chain_checks_signature_hash_and_dependency_bounds() {
    let key = SigningKey::new([1; 32]);
    let dependency = ContentId::hash(b"base");
    let payload = b"signed executable";
    let required = bundle_size(payload.len(), 1).unwrap();
    let mut bundle = vec![0; required];
    let info = encode_bundle(payload, 0, &[dependency], key, &mut bundle).unwrap();
    PackageBundle::decode(&bundle).unwrap().verify(key).unwrap();

    let mut tampered_signature = bundle.clone();
    tampered_signature[112] ^= 1;
    assert_eq!(PackageBundle::decode(&tampered_signature).unwrap().verify(key), Err(PackageError::InvalidSignature));
    let mut tampered_hash = bundle.clone();
    tampered_hash[64] ^= 1;
    assert!(matches!(PackageBundle::decode(&tampered_hash), Err(PackageError::CorruptBundle)));
    assert_eq!(bundle_size(payload.len(), synos_system_model::MAX_DEPENDENCIES + 1), Err(PackageError::TooManyDependencies));

    let mut filesystem = SynFs::<128>::new();
    filesystem.create_directory("system/store", true).unwrap();
    filesystem.create_directory("system/manifests", true).unwrap();
    let mut daemon = PackageDaemon::<2, 1>::new();
    daemon.trust_key(key).unwrap();
    let mut verification = [0; 64];
    assert_eq!(daemon.install_bundle(&mut filesystem, &bundle, &mut verification).unwrap(), info.package);
    let receipt = daemon.authorize_instantiation(info.package).unwrap();
    daemon.validate_instantiation(receipt).unwrap();
}

#[test]
fn package_hash_mismatch_cannot_become_an_instantiation_receipt() {
    let key = SigningKey::new([2; 32]);
    let payload = b"immutable";
    let required = bundle_size(payload.len(), 0).unwrap();
    let mut bundle = vec![0; required];
    let info = encode_bundle(payload, 0, &[], key, &mut bundle).unwrap();
    let mut filesystem = SynFs::<128>::new();
    filesystem.create_directory("system/store", true).unwrap();
    filesystem.create_directory("system/manifests", true).unwrap();
    let mut daemon = PackageDaemon::<2, 1>::new();
    daemon.trust_key(key).unwrap();
    let mut altered = bundle.clone();
    let last = altered.len() - 1;
    altered[last] ^= 1;
    assert_eq!(daemon.install_bundle(&mut filesystem, &altered, &mut [0; 64]), Err(PackageError::CorruptBundle));
    assert_eq!(daemon.authorize_instantiation(info.package), Err(PackageError::InstantiationDenied));
}
