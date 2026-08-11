use synos_rustd::{
    BuildPolicy, BuildResult, CacheKey, CompilerService, NetworkPolicy, Profile, RemoteCache,
    RemoteCacheEntry, SharedImmutableArtifacts, Target,
};
use synos_system_model::ContentId;

fn result() -> BuildResult {
    BuildResult {
        package: ContentId::hash(b"package"),
        payload: ContentId::hash(b"payload"),
        target: Target::X86_64,
    }
}

fn key() -> CacheKey {
    CacheKey {
        source: ContentId::hash(b"source"),
        lockfile: ContentId::hash(b"lockfile"),
        toolchain: ContentId::hash(b"toolchain"),
        target: Target::X86_64,
        profile: Profile::Release,
        features: ContentId::hash(b"features"),
    }
}

struct TestRemoteCache {
    entry: Option<RemoteCacheEntry>,
    calls: usize,
}

impl RemoteCache for TestRemoteCache {
    type Error = ();

    fn lookup(&mut self, _key: CacheKey) -> Result<Option<RemoteCacheEntry>, Self::Error> {
        self.calls += 1;
        Ok(self.entry)
    }
}

#[test]
fn shared_artifacts_are_immutable_and_leased() {
    let mut store = SharedImmutableArtifacts::<1>::new();
    store.publish(result()).unwrap();
    assert!(store.contains(result()));
    store.acquire(result()).unwrap();
    assert_eq!(store.entries().next().unwrap().leases, 1);
    store.release(result()).unwrap();
    assert_eq!(store.entries().next().unwrap().leases, 0);
    assert!(store
        .can_publish(BuildResult {
            package: result().package,
            payload: ContentId::hash(b"different"),
            target: Target::X86_64,
        })
        .is_err());
}

#[test]
fn remote_cache_requires_network_and_valid_proof() {
    let valid = RemoteCacheEntry::new(key(), result()).unwrap();
    let mut remote = TestRemoteCache {
        entry: Some(valid),
        calls: 0,
    };
    let mut service = CompilerService::<1, 2>::new(BuildPolicy::OFFLINE);
    assert_eq!(
        service
            .lookup_remote_cache(key(), NetworkPolicy::Denied, &mut remote)
            .unwrap(),
        None
    );
    assert_eq!(remote.calls, 0);
    assert_eq!(
        service
            .lookup_remote_cache(key(), NetworkPolicy::Allowed, &mut remote)
            .unwrap(),
        Some(result())
    );
    assert_eq!(service.snapshot().shared_artifact_entries, 1);
    assert_eq!(service.snapshot().content_cache_entries, 1);

    remote.entry = Some(RemoteCacheEntry {
        proof: ContentId::hash(b"tampered"),
        ..valid
    });
    assert!(service
        .lookup_remote_cache(
            CacheKey {
                source: ContentId::hash(b"other-source"),
                ..key()
            },
            NetworkPolicy::Allowed,
            &mut remote,
        )
        .is_err());
    assert_eq!(service.snapshot().shared_artifact_entries, 1);
}

#[test]
fn cancelled_job_publishes_no_artifact() {
    let request = synos_rustd::BuildRequest {
        source_root: synos_rustd::Text::new("/system/sources/demo").unwrap(),
        manifest: synos_rustd::Text::new("Cargo.toml").unwrap(),
        binary: synos_rustd::Text::new("demo").unwrap(),
        target: Target::X86_64,
        profile: Profile::Release,
        locked: true,
        network: NetworkPolicy::Denied,
        limits: synos_rustd::ResourceLimits::DEFAULT,
        features: [None; synos_rustd::MAX_FEATURES],
    };
    let mut service = CompilerService::<1, 1>::new(BuildPolicy::OFFLINE);
    let id = service.submit(request).unwrap();
    service.cancel(id).unwrap();
    assert_eq!(service.snapshot().shared_artifact_entries, 0);
    assert_eq!(service.snapshot().content_cache_entries, 0);
}
