use ghostos_observability::{
    ContentId, ProfileAggregator, ProfileDomain, ProfileError, ProfileMetadata, ProfileRetention,
    ProfileRing, ProfileSample, RetainedProfile, redacted_host_id, validate_profile_export,
};

#[test]
fn samples_are_symbolized_bounded_and_folded() {
    let ring = ProfileRing::<2>::new();
    let sample = ProfileSample::single(ProfileDomain::Ipc, 1, 0, 0x2001)
        .with_frame(0x2002, 4);
    ring.push(sample);
    ring.push(sample);
    ring.push(ProfileSample::single(ProfileDomain::Scheduler, 3, 1, 0x3001));
    assert_eq!(ring.dropped(), 1);

    let metadata = ProfileMetadata::new(ContentId::hash(b"revision"), b"host-name-secret", 1000, 1);
    let mut archive = ProfileAggregator::<4>::new(metadata);
    assert_eq!(archive.ingest(sample), Ok(()));
    assert_eq!(archive.ingest(sample), Ok(()));
    assert_eq!(archive.sample_count(), 2);
    assert_eq!(archive.stacks().next().unwrap().samples, 2);
    assert_eq!(archive.stacks().next().unwrap().depth(), 2);
}

#[test]
fn profile_export_is_revision_tagged_redacted_and_checked() {
    let host = b"host.example.invalid/10.0.0.4";
    let metadata = ProfileMetadata::new(ContentId::hash(b"revision"), host, 1000, 1);
    let mut archive = ProfileAggregator::<2>::new(metadata);
    archive
        .ingest(ProfileSample::single(ProfileDomain::Boot, 2, 0, 0x1001))
        .unwrap();
    let mut encoded = vec![0; archive.encoded_len()];
    let length = archive.encode(&mut encoded).unwrap();
    assert_eq!(validate_profile_export(&encoded), Ok(length));
    assert!(!encoded.windows(host.len()).any(|window| window == host));
    encoded[20] ^= 1;
    assert_eq!(validate_profile_export(&encoded), Err(ProfileError::InvalidExport));
}

#[test]
fn host_identity_is_redacted_and_retention_is_revision_keyed() {
    let first = redacted_host_id(b"host-a");
    let second = redacted_host_id(b"host-b");
    assert_ne!(first, second);
    assert_eq!(first, redacted_host_id(b"host-a"));

    let revision = ContentId::hash(b"revision");
    let mut retention = ProfileRetention::<1>::new();
    retention
        .retain(RetainedProfile { revision, host_id: first, sample_count: 1 })
        .unwrap();
    retention
        .retain(RetainedProfile { revision, host_id: first, sample_count: 2 })
        .unwrap();
    assert_eq!(retention.entries().next().unwrap().sample_count, 2);
    assert_eq!(
        retention.retain(RetainedProfile { revision, host_id: second, sample_count: 1 }),
        Err(ProfileError::Capacity)
    );
}
