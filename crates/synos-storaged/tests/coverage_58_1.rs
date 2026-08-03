use synos_storaged::{
    AdmissionPolicy, BootstrapChecks, ClockHealth, ClusterBootstrapState, ClusterCreateRequest,
    MetadataText, SeedEntropy,
};

fn request() -> ClusterCreateRequest {
    ClusterCreateRequest::new(
        MetadataText::new("local").unwrap(),
        [7; synos_storaged::NODE_ID_BYTES],
        10,
    )
}

#[test]
fn local_bootstrap_gates_activation_and_signs_advertisements() {
    let mut entropy = SeedEntropy::new(0x5a5a);
    let request = request();
    let mut state = ClusterBootstrapState::create(request, &mut entropy).unwrap();

    let mut unhealthy = BootstrapChecks::from_request(&request);
    unhealthy.clock = ClockHealth {
        synchronized: false,
        offset_us: 0,
        uncertainty_us: 0,
    };
    assert!(state.activate(unhealthy).is_err());

    state
        .activate(BootstrapChecks::from_request(&request))
        .unwrap();
    let advertisement = state.publish_advertisement(20, 100).unwrap();
    advertisement.verify(state.root, 21).unwrap();
}

#[test]
fn bootstrap_resume_round_trip_and_credentials_are_recoverable() {
    let mut entropy = SeedEntropy::new(0x1234);
    let request = request();
    let state = ClusterBootstrapState::create(request, &mut entropy).unwrap();
    let mut resumed =
        ClusterBootstrapState::create_or_resume(Some(&state), request, &mut entropy).unwrap();
    assert_eq!(resumed, state);

    let mut encoded = vec![0; ClusterBootstrapState::encoded_len()];
    resumed.encode(&mut encoded).unwrap();
    assert_eq!(ClusterBootstrapState::decode(&encoded).unwrap(), resumed);

    let token = resumed.bootstrap_token.token;
    resumed.consume_bootstrap_token(token, 20, 1).unwrap();
    assert!(resumed.consume_bootstrap_token(token, 21, 1).is_err());
    resumed
        .rotate_bootstrap_credentials(&mut entropy, 1_000)
        .unwrap();
    resumed.revoke_bootstrap_credentials().unwrap();
    assert!(
        resumed
            .consume_bootstrap_token(resumed.bootstrap_token.token, 21, 1)
            .is_err()
    );
}

#[test]
fn attested_creation_requires_verified_measurement() {
    let mut entropy = SeedEntropy::new(0x9abc);
    let mut request = request();
    request.admission_policy = AdmissionPolicy::Attested;
    let mut state = ClusterBootstrapState::create(request, &mut entropy).unwrap();
    assert!(
        state
            .activate(BootstrapChecks::from_request(&request))
            .is_err()
    );
}
