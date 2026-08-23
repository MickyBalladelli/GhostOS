use ghostos_storaged::{
    AdmissionInvitation, AdmissionError, AdmissionPolicy, AdmissionWorkflow, Certificate, ClusterId, ClusterMetadata,
    ClusterMetadataCatalog, ClusterMetadataError, NodeAttestation, NodeCapabilities,
    ClusterOperation, ClusterRole, ClusterSecurityAuthority, ClusterSecurityPolicy,
    InvitationDecision, IoOperation, MemberHealth, MemberRole, MemberSpec,
    MembershipError, MembershipRegistry, MetadataText, MountOptions, MountStateError, Protocol,
    ProtocolCompatibility, QuorumPolicy, SeedEntropy, StorageDaemon, StorageError, StoragePath,
    StorageRights, AdmissionEndpoint, ClusterBootstrapError, ClusterBootstrapState, Endpoint,
    ClusterCreateRequest,
    MembershipChangeKind, MembershipReason, MembershipState, NvmeCommand, NvmeQueue,
    SecurityPolicy, ActionRequest, ClusterHealth, ClusterSample, FailureController, FailureKind,
    NodeHealthSample, OperatorAction, RecoveryHooks, RecoveryState, MAX_PENDING_IO,
};
use ghostos_auth::CapabilityKey;
use ghostos_fabric::NodeId;
use ghostos_ghostfs::SynFs;
use ghostos_test_support::crash::{CrashBoundary, CrashDomain, CrashHarness, CrashPoint};
use ghostos_status::Status;
use ghostos_time_sync::ManualClock;

struct RecoveryProbe {
    events: Vec<&'static str>,
}

impl RecoveryProbe {
    fn new() -> Self {
        Self { events: Vec::new() }
    }

    fn record(&mut self, event: &'static str) -> Result<(), Status> {
        self.events.push(event);
        Ok(())
    }
}

impl RecoveryHooks for RecoveryProbe {
    fn retry(&mut self, _node: NodeId) -> Result<(), Status> { self.record("retry") }
    fn drain(&mut self, _node: NodeId) -> Result<(), Status> { self.record("drain") }
    fn fence(&mut self, _node: NodeId) -> Result<(), Status> { self.record("fence") }
    fn unfence(&mut self, _node: NodeId) -> Result<(), Status> { self.record("unfence") }
    fn release_shared_memory(&mut self, _node: NodeId) -> Result<(), Status> { self.record("memory") }
    fn release_storage(&mut self, _node: NodeId) -> Result<(), Status> { self.record("storage") }
    fn release_jobs(&mut self, _node: NodeId) -> Result<(), Status> { self.record("jobs") }
    fn revoke_capabilities(&mut self, _node: NodeId) -> Result<(), Status> { self.record("capabilities") }
    fn release_dlm_leases(&mut self, _node: NodeId) -> Result<(), Status> { self.record("leases") }
    fn reconcile_membership(&mut self, _node: NodeId) -> Result<(), Status> { self.record("membership") }
    fn reconcile_ghostfs_cow(&mut self, _node: NodeId) -> Result<(), Status> { self.record("ghostfs") }
    fn reconcile_logs(&mut self, _node: NodeId) -> Result<(), Status> { self.record("logs") }
    fn reconcile_reservations(&mut self, _node: NodeId) -> Result<(), Status> { self.record("reservations") }
    fn reconcile_workloads(&mut self, _node: NodeId) -> Result<(), Status> { self.record("workloads") }
    fn rollback(&mut self) -> Result<(), Status> { self.record("rollback") }
    fn abandon(&mut self, _node: NodeId) -> Result<(), Status> { self.record("abandon") }
}

fn failure_sample(node: NodeId, reachable: bool, heartbeat_age_us: u64, clock_offset_us: i64) -> NodeHealthSample {
    NodeHealthSample {
        node,
        reachable,
        heartbeat_age_us,
        heartbeat_timeout_us: 100,
        clock_offset_us,
        maximum_clock_skew_us: 50,
        protocol_version: 1,
        minimum_protocol_version: 1,
        maximum_protocol_version: 1,
        generation: 7,
        membership_epoch: 3,
    }
}

fn partition_sample() -> ClusterSample {
    ClusterSample {
        generation: 7,
        membership_epoch: 3,
        voting_members: 3,
        available_votes: 1,
        required_votes: 2,
        partitioned: true,
        sampled_at_us: 0,
    }
}

#[test]
fn partition_fences_before_storage_and_memory_release() {
    let node = NodeId::new(2).unwrap();
    let mut controller = FailureController::<2, 16>::new();
    let report = controller.observe(partition_sample(), &[failure_sample(node, false, 100, 0)]);
    assert_eq!(report.health, ClusterHealth::Partitioned);
    assert_eq!(controller.state(node), Some(RecoveryState::Failed));
    assert!(controller.events().any(|event| event.kind == FailureKind::Partition));
    assert!(controller.events().any(|event| event.kind == FailureKind::NodeFailure));

    let mut probe = RecoveryProbe::new();
    assert_eq!(
        controller.apply(
            ActionRequest {
                action: OperatorAction::Recover,
                node: Some(node),
                confirm: false,
                force: false,
            },
            &mut probe,
        ),
        Err(ghostos_storaged::FailureError::ReconciliationRequired)
    );
    assert!(probe.events.is_empty());
    controller
        .apply(
            ActionRequest {
                action: OperatorAction::Fence,
                node: Some(node),
                confirm: true,
                force: true,
            },
            &mut probe,
        )
        .unwrap();
    assert_eq!(probe.events, ["fence"]);

    let receipt = controller
        .apply(
            ActionRequest {
                action: OperatorAction::Recover,
                node: Some(node),
                confirm: false,
                force: false,
            },
            &mut probe,
        )
        .unwrap();
    assert_eq!(receipt.state, RecoveryState::Reconciled);
    assert!(receipt.fenced_before_release);
    assert_eq!(probe.events[0], "fence");
    assert_eq!(&probe.events[1..3], ["memory", "storage"]);
}

#[test]
fn clock_skew_is_detected_at_local_sample_time_before_recovery() {
    let node = NodeId::new(2).unwrap();
    let clock = ManualClock::new(5_000);
    let mut controller = FailureController::<2, 16>::new();
    let mut sample = failure_sample(node, true, 1, 51);
    sample.maximum_clock_skew_us = 50;
    let report = controller.observe_with_clock(
        ClusterSample {
            partitioned: false,
            available_votes: 2,
            required_votes: 2,
            sampled_at_us: 1,
            ..partition_sample()
        },
        &[sample],
        &clock,
    );
    assert_eq!(report.sampled_at_us, 5_000);
    assert_eq!(controller.state(node), Some(RecoveryState::Failed));
    assert!(controller.events().any(|event| event.kind == FailureKind::ClockSkew));

    let mut probe = RecoveryProbe::new();
    controller
        .apply(
            ActionRequest {
                action: OperatorAction::Fence,
                node: Some(node),
                confirm: true,
                force: true,
            },
            &mut probe,
        )
        .unwrap();
    controller
        .apply(
            ActionRequest {
                action: OperatorAction::Recover,
                node: Some(node),
                confirm: false,
                force: false,
            },
            &mut probe,
        )
        .unwrap();
    assert_eq!(probe.events.first(), Some(&"fence"));
    assert!(probe.events.iter().position(|event| *event == "storage").unwrap() > 0);
}

#[test]
fn storage_paths_capabilities_and_io_boundaries_are_enforced() {
    assert_eq!(StoragePath::new("SYS$STORAGE:").unwrap(), StoragePath::ROOT);
    assert!(StoragePath::ROOT.contains(StoragePath::mount_root("DATA").unwrap()));
    assert!(StoragePath::new("SYS$STORAGE:DATA/../escape").is_err());
    assert!(StoragePath::new("SYS$STORAGE:DATA//file").is_err());

    let mut daemon = StorageDaemon::new(0x5a5a);
    let admin = daemon.bootstrap_capability(100).expect("bootstrap admin capability");
    let mount = daemon
        .mount(
            admin,
            Endpoint::new("storage.example", "/volume", 2049).unwrap(),
            Protocol::Pnfs(ghostos_storaged::NfsMinorVersion::V42),
            StoragePath::mount_root("DATA").unwrap(),
            MountOptions::DEFAULT,
            10,
        )
        .expect("mount storage endpoint");
    let scoped = daemon
        .capability(
            admin,
            mount.id,
            StorageRights::READ,
            StoragePath::new("SYS$STORAGE:DATA/files").unwrap(),
            90,
            10,
        )
        .expect("attenuate storage capability");
    assert_eq!(
        daemon.submit_io(
            scoped,
            IoOperation::Read,
            StoragePath::new("SYS$STORAGE:DATA/files/a").unwrap(),
            4,
            10,
        )
        .unwrap()
        .length,
        4
    );
    assert!(matches!(
        daemon.submit_io(
            scoped,
            IoOperation::Write,
            StoragePath::new("SYS$STORAGE:DATA/files/a").unwrap(),
            4,
            10,
        ),
        Err(StorageError::Capability(_))
    ));
    let completion = daemon.complete_next(Ok(()), 99).expect("complete pending IO");
    assert_eq!(completion.bytes, 4);
    assert_eq!(daemon.poll_completion(), Some(completion));
    for _ in 0..MAX_PENDING_IO {
        daemon
            .submit_io(
                scoped,
                IoOperation::Read,
                StoragePath::new("SYS$STORAGE:DATA/files/a").unwrap(),
                4,
                10,
            )
            .unwrap();
    }
    assert!(matches!(
        daemon.submit_io(
            scoped,
            IoOperation::Read,
            StoragePath::new("SYS$STORAGE:DATA/files/a").unwrap(),
            4,
            10,
        ),
        Err(StorageError::QueueFull)
    ));
    assert!(matches!(daemon.bootstrap_capability(0), Err(_)));
}

#[test]
fn remote_nvme_queue_fails_fast_and_reuses_completed_slots() {
    let mut queue = NvmeQueue::<2>::new();
    let command = |command_id| NvmeCommand {
        command_id,
        offset: 0,
        length: 4096,
        write: false,
    };

    queue.submit(command(1)).unwrap();
    queue.submit(command(2)).unwrap();
    assert!(queue.submit(command(3)).is_err());
    assert_eq!(queue.complete(1), Some(command(1)));
    queue.submit(command(3)).unwrap();
    assert_eq!(queue.complete(99), None);
}

#[test]
fn mount_catalog_round_trips_through_ghostfs_and_rejects_corruption() {
    let mut daemon = StorageDaemon::new(7);
    let admin = daemon.bootstrap_capability(100).unwrap();
    let mounted = daemon
        .mount(
            admin,
            Endpoint::new("host", "/export", 445).unwrap(),
            Protocol::Smb {
                dialect: ghostos_storaged::SmbDialect::Smb311,
                multichannel: true,
                direct: true,
            },
            StoragePath::mount_root("REMOTE").unwrap(),
            MountOptions::DEFAULT,
            1,
        )
        .unwrap();

    let catalog = daemon.mount_catalog();
    let mut encoded = vec![0; catalog.encoded_len()];
    let length = catalog.encode(&mut encoded).unwrap();
    assert_eq!(ghostos_storaged::MountCatalog::decode(&encoded).unwrap().version(), catalog.version());
    assert!(matches!(
        ghostos_storaged::MountCatalog::decode(&encoded[..length - 1]),
        Err(MountStateError::Corrupt)
    ));
    encoded[0] ^= 1;
    assert!(matches!(
        ghostos_storaged::MountCatalog::decode(&encoded),
        Err(MountStateError::Corrupt)
    ));

        let mut filesystem = SynFs::<64>::new();
    let mut staging = vec![0; catalog.encoded_len()];
    catalog.save_to_ghostfs(&mut filesystem, &mut staging).expect("persist mount catalog");
    let mut restored_bytes = vec![0; catalog.encoded_len()];
    filesystem.read("/system/mounts.dat", &mut restored_bytes).expect("read mount catalog");
    let restored = ghostos_storaged::MountCatalog::decode(&restored_bytes).unwrap();
    let mut restored_daemon = StorageDaemon::new(7);
    restored_daemon.restore_catalog(restored).expect("restore mount catalog");
    let mut mount_infos = [mounted; 1];
    assert_eq!(restored_daemon.list_mounts(&mut mount_infos), 1);
}

fn cluster_metadata(id: ClusterId, name: &str) -> ClusterMetadata {
    ClusterMetadata::new(
        id,
        MetadataText::new(name).unwrap(),
        MetadataText::new("test cluster").unwrap(),
        [1; 16],
        10,
    )
}

#[test]
fn cluster_metadata_is_generation_safe_and_persistent() {
    let id = ClusterId::new([1; 16]).unwrap();
    let mut catalog = ClusterMetadataCatalog::new();
    let metadata = cluster_metadata(id, "compute");
    catalog.create_cluster(metadata, 0).unwrap();
    assert_eq!(catalog.create_cluster(metadata, 1), Err(ClusterMetadataError::AlreadyExists));

    let generation = catalog.cluster(id).unwrap().generation;
    catalog
        .add_alias(id, MetadataText::new("primary").unwrap(), generation)
        .unwrap();
    assert_eq!(catalog.cluster_by_name("PRIMARY").unwrap().id, id);
    assert_eq!(
        catalog.rename_cluster(id, MetadataText::new("renamed").unwrap(), generation),
        Err(ClusterMetadataError::StaleGeneration)
    );

    let generation = catalog.cluster(id).unwrap().generation;
    catalog
        .add_invitation(
            id,
            ghostos_storaged::Invitation {
                cluster_id: id,
                token: [7; 32],
                expires_at: 100,
                scope: 1,
                used: false,
                revoked: false,
            },
            generation,
        )
        .unwrap();
    let generation = catalog.cluster(id).unwrap().generation;
    catalog
        .add_certificate(
            id,
            Certificate {
                fingerprint: [8; 32],
                issued_at: 10,
                expires_at: 100,
                revoked: false,
            },
            generation,
        )
        .unwrap();
    catalog.set_active_cluster(id, catalog.catalog_generation()).unwrap();

        let mut filesystem = SynFs::<64>::new();
    let mut staging = vec![0; ClusterMetadataCatalog::encoded_len()];
    catalog.save_to_ghostfs(&mut filesystem, &mut staging).unwrap();
    let restored = ClusterMetadataCatalog::load_from_ghostfs(&filesystem, &mut staging).unwrap();
    assert_eq!(restored.active_cluster(), Some(id));
    assert_eq!(restored.cluster_by_name("renamed"), None);

    let mut encoded = vec![0; ClusterMetadataCatalog::encoded_len()];
    catalog.encode(&mut encoded).unwrap();
    encoded[0] ^= 1;
    assert!(matches!(
        ClusterMetadataCatalog::decode(&encoded),
        Err(ClusterMetadataError::Corrupt)
    ));

    catalog.set_writer(1, [9; 16]).unwrap();
    assert_eq!(catalog.set_writer(1, [10; 16]), Err(ClusterMetadataError::SplitBrain));
}

#[test]
fn admission_states_and_security_authorization_reject_stale_or_unsafe_actions() {
    let cluster = ClusterId::new([2; 16]).unwrap();
    let mut workflow = AdmissionWorkflow::<4, 4, 4>::new(
        cluster,
        AdmissionPolicy::Open,
        1,
        SecurityPolicy::DEFAULT,
    )
    .unwrap();
    let point = CrashPoint::new(CrashDomain::Storage, CrashBoundary::JournalRecord, 1);
    let mut harness = CrashHarness::new(Some(point));
    assert_eq!(
        workflow.commit_change_with_interruption(
            MembershipChangeKind::Join,
            NodeId::new(4).unwrap(),
            MembershipState::Joined,
            MembershipReason::Approved,
            1,
            &mut harness,
        ),
        Err(AdmissionError::Interrupted)
    );
    let invitation = AdmissionInvitation {
        cluster,
        token: [3; 32],
        expires_at_us: 10,
        scope: ghostos_storaged::INVITATION_SCOPE_JOIN,
        target: None,
        fingerprint: None,
        one_time: true,
        used: false,
        revoked: false,
        decision: InvitationDecision::Pending,
    };
    assert_eq!(
        invitation.usable(cluster, NodeId::new(4).unwrap(), [0; 32], 1, 10),
        Err(AdmissionError::InvitationExpired)
    );
    let mut revoked = invitation;
    revoked.revoked = true;
    assert_eq!(
        revoked.usable(cluster, NodeId::new(4).unwrap(), [0; 32], 1, 1),
        Err(AdmissionError::InvitationRevoked)
    );

    let mut authority = ClusterSecurityAuthority::<2, 2, 8, 8>::new(
        cluster,
        CapabilityKey::new([4; 32]),
        [5; 16],
        ClusterSecurityPolicy::STRICT,
        1,
    )
    .unwrap();
    let node = NodeId::new(4).unwrap();
    let admin = authority
        .bootstrap_capability(node, ClusterRole::Administrator, 100, 1)
        .unwrap();
    let point = CrashPoint::new(CrashDomain::Storage, CrashBoundary::CapabilityChange, 1);
    let mut harness = CrashHarness::new(Some(point));
    assert_eq!(
        authority.revoke_capability_with_interruption(&admin, node, 77, 2, &mut harness),
        Err(ghostos_storaged::SecurityError::Interrupted)
    );
    authority
        .authorize(&admin, node, ClusterOperation::Fence, Some(node), 2)
        .unwrap();

    let read_only = authority
        .bootstrap_capability(NodeId::new(6).unwrap(), ClusterRole::ReadOnly, 100, 2)
        .unwrap();
    assert_eq!(
        authority.authorize(&read_only, NodeId::new(6).unwrap(), ClusterOperation::Fence, None, 2),
        Err(ghostos_storaged::SecurityError::AccessDenied)
    );
    assert_eq!(authority.audit.records().count(), 3);

    authority
        .rotate_key(&admin, node, CapabilityKey::new([6; 32]), [6; 16], 3)
        .unwrap();
    authority.revoke_key(&admin, node, 1, 4).unwrap();
    assert_eq!(
        authority.authorize(&admin, node, ClusterOperation::Fence, Some(node), 4),
        Err(ghostos_storaged::SecurityError::KeyNotFound)
    );

    let relaxed_identity_policy = ghostos_storaged::ClusterSecurityPolicy {
        required_capabilities: NodeCapabilities::empty(),
        require_certificate: false,
        require_attestation: true,
        require_encryption: false,
        allowed_attestation_roots: u8::MAX,
    };
    let mut attestation_authority = ClusterSecurityAuthority::<2, 2, 8, 8>::new(
        cluster,
        CapabilityKey::new([8; 32]),
        [8; 16],
        relaxed_identity_policy,
        1,
    )
    .unwrap();
    let node_owner = attestation_authority
        .bootstrap_capability(node, ClusterRole::NodeOwner, 100, 3)
        .unwrap();
    assert_eq!(
        attestation_authority.admit_node(
            &node_owner,
            node,
            None,
            NodeAttestation::NONE,
            NodeCapabilities::empty(),
            2,
        ),
        Err(ghostos_storaged::SecurityError::AttestationRejected)
    );
}

#[test]
fn quorum_partition_and_membership_state_round_trip_are_bounded() {
    let cluster = ClusterId::new([6; 16]).unwrap();
    let node = NodeId::new(7).unwrap();
    let member = MemberSpec {
        node,
        role: MemberRole::Voter,
        fingerprint: [7; 32],
        endpoint: AdmissionEndpoint::new("node-7").unwrap(),
        capacity: 100,
        zone: ghostos_storaged::MembershipLabel::new("zone-a").unwrap(),
        rack: ghostos_storaged::MembershipLabel::new("rack-a").unwrap(),
    };
    let mut registry = MembershipRegistry::<2>::new(cluster, QuorumPolicy::SINGLE_NODE).unwrap();
    registry.bootstrap_member(member, 1).unwrap();
    assert!(registry.quorum().has_quorum);
    registry.elect_leader().unwrap();
    let duplicate = MemberSpec {
        node: NodeId::new(8).unwrap(),
        role: MemberRole::Observer,
        fingerprint: [7; 32],
        endpoint: AdmissionEndpoint::new("node-8").unwrap(),
        capacity: 100,
        zone: ghostos_storaged::MembershipLabel::new("zone-a").unwrap(),
        rack: ghostos_storaged::MembershipLabel::new("rack-a").unwrap(),
    };
    assert_eq!(
        registry.register_member(duplicate, 2),
        Err(MembershipError::DuplicateIdentity)
    );
    registry.set_reachability(node, false, 2).unwrap();
    registry.set_health(node, MemberHealth::Failed).unwrap();
    assert!(!registry.quorum().has_quorum);
    assert_eq!(registry.elect_leader(), Err(MembershipError::QuorumUnavailable));

    let mut encoded = vec![0; MembershipRegistry::<2>::encoded_len()];
    registry.encode(&mut encoded).unwrap();
    let restored = MembershipRegistry::<2>::decode(&encoded).unwrap();
    assert_eq!(restored.cluster, cluster);
    assert!(!restored.quorum().has_quorum);
    encoded[0] ^= 1;
    assert!(matches!(
        MembershipRegistry::<2>::decode(&encoded),
        Err(MembershipError::Corrupt)
    ));
}

#[test]
fn cluster_catalog_rejects_full_capacity_without_overwriting_state() {
    let mut catalog = ClusterMetadataCatalog::new();
    for index in 0..ghostos_storaged::MAX_CLUSTERS {
        let mut raw = [0; 16];
        raw[0] = index as u8 + 1;
        let id = ClusterId::new(raw).unwrap();
        let name = format!("cluster-{index}");
        catalog
            .create_cluster(cluster_metadata(id, &name), index as u64)
            .unwrap();
    }

    let mut raw = [0; 16];
    raw[0] = 99;
    let result = catalog.create_cluster(
        cluster_metadata(ClusterId::new(raw).unwrap(), "overflow"),
        ghostos_storaged::MAX_CLUSTERS as u64,
    );
    assert_eq!(result, Err(ClusterMetadataError::Capacity));
    assert!(catalog.cluster_by_name("cluster-0").is_some());
}

#[test]
fn bootstrap_rejects_incompatible_protocol_versions() {
    let mut request = ClusterCreateRequest::new(MetadataText::new("protocol").unwrap(), [9; 16], 1);
    request.protocols = ProtocolCompatibility {
        control_version: 1,
        data_version: 1,
        minimum_version: 2,
    };
    assert_eq!(
        ClusterBootstrapState::create(request, &mut SeedEntropy::new(1)),
        Err(ClusterBootstrapError::ProtocolMismatch)
    );
}
