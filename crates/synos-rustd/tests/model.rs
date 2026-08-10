use synos_rustd::{
    BuildPolicy, BuildRequest, CancellationDisposition, CompilerService, Error, JobState,
    NetworkPolicy, Profile, ResourceLimits, Target, Text, MAX_FEATURES,
};

fn request(deadline_us: u64) -> BuildRequest {
    BuildRequest {
        source_root: Text::new("/system/sources/demo").unwrap(),
        manifest: Text::new("Cargo.toml").unwrap(),
        binary: Text::new("demo").unwrap(),
        target: Target::X86_64,
        profile: Profile::Release,
        locked: true,
        network: NetworkPolicy::Denied,
        limits: ResourceLimits {
            deadline_us,
            ..ResourceLimits::DEFAULT
        },
        features: [None; MAX_FEATURES],
    }
}

#[test]
fn compiler_deadline_expires_only_at_the_boundary() {
    let mut service = CompilerService::<1, 1>::new(BuildPolicy::OFFLINE);
    let id = service.submit(request(10)).unwrap();
    service.start_at(id, 1).unwrap();

    assert_eq!(service.expire(10), 0);
    assert_eq!(service.status(id).unwrap().state, JobState::Running);
    assert_eq!(service.expire(11), 1);
    assert_eq!(service.status(id).unwrap().state, JobState::Failed);
    assert_eq!(service.expire(12), 0);
}

#[test]
fn compiler_cancellation_grace_handles_repeated_requests_once() {
    let mut service = CompilerService::<1, 1>::new(BuildPolicy::OFFLINE);
    let id = service.submit(request(100)).unwrap();
    service.start_at(id, 10).unwrap();

    assert_eq!(
        service.request_cancel(id, 20),
        Ok(CancellationDisposition::RequestCooperativeStop)
    );
    assert_eq!(
        service.request_cancel(id, 20),
        Ok(CancellationDisposition::RequestCooperativeStop)
    );
    assert_eq!(service.status(id).unwrap().state, JobState::Running);

    assert_eq!(
        service.request_cancel(id, 20 + synos_rustd::CANCELLATION_GRACE_US - 1),
        Ok(CancellationDisposition::RequestCooperativeStop)
    );
    assert_eq!(
        service.request_cancel(id, 20 + synos_rustd::CANCELLATION_GRACE_US),
        Ok(CancellationDisposition::RequestCooperativeStop)
    );
    assert_eq!(service.status(id).unwrap().state, JobState::Cancelled);
    assert_eq!(service.request_cancel(id, 20 + synos_rustd::CANCELLATION_GRACE_US), Ok(CancellationDisposition::AlreadyTerminal));
    assert_eq!(service.complete(id, panic_result()), Err(Error::InvalidTransition));
}

fn panic_result() -> synos_rustd::BuildResult {
    synos_rustd::BuildResult {
        package: synos_system_model::ContentId::hash(b"package"),
        payload: synos_system_model::ContentId::hash(b"payload"),
        target: Target::X86_64,
    }
}
