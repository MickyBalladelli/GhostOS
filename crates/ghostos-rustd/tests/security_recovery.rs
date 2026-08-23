use ghostos_pkg::{PackageDaemon, SigningKey, bundle_size, encode_bundle};
use ghostos_rustd::{
    ArtifactError, ArtifactSandbox, BuildRequest, DynamicArtifact, DynamicArtifactKind,
    JobId, NetworkPolicy, Profile, Target, Text, ToolKind, ToolchainComponent, ToolchainError,
    ToolchainManifest, ToolchainPlan, ToolchainRequest, ToolchainStage, ToolchainStageError,
    ToolchainStageResult,
};
use ghostos_ghostfs::SynFs;
use ghostos_system_model::ContentId;

fn install<const PACKAGES: usize, const KEYS: usize, const BLOCKS: usize>(
    daemon: &mut PackageDaemon<PACKAGES, KEYS>,
    filesystem: &mut SynFs<BLOCKS>,
    key: SigningKey,
    payload: &[u8],
) -> ContentId {
    let mut encoded = vec![0; bundle_size(payload.len(), 0).unwrap()];
    let info = encode_bundle(payload, 0, &[], key, &mut encoded).unwrap();
    daemon
        .install_bundle(filesystem, &encoded, &mut [0; 128])
        .unwrap();
    info.package
}

fn request(target: Target) -> ToolchainRequest {
    ToolchainRequest {
        build: BuildRequest {
            source_root: Text::new("/system/sources/demo").unwrap(),
            manifest: Text::new("Cargo.toml").unwrap(),
            binary: Text::new("demo").unwrap(),
            target,
            profile: Profile::Release,
            locked: true,
            network: NetworkPolicy::Denied,
            limits: ghostos_rustd::ResourceLimits::DEFAULT,
            features: [None; ghostos_rustd::MAX_FEATURES],
        },
        run_build_scripts: false,
        run_proc_macros: false,
        capabilities: ghostos_rustd::CompilerCapabilities::MINIMUM,
    }
}

fn component(kind: ToolKind, package: ContentId, target: Target) -> ToolchainComponent {
    ToolchainComponent::new(kind, package, "/system/tool", target).unwrap()
}

#[test]
fn compiler_toolchain_rotation_revocation_replay_rollback_and_downgrade_are_fenced() {
    let old_key = SigningKey::new([101; 32]);
    let recovered_key = SigningKey::new([102; 32]);
    let mut filesystem = SynFs::<64>::new();
    filesystem.create_directory("system/store", true).unwrap();
    filesystem.create_directory("system/manifests", true).unwrap();
    let mut daemon = PackageDaemon::<8, 2>::new();
    daemon.trust_key(old_key).unwrap();
    let old_package = install(&mut daemon, &mut filesystem, old_key, b"stage-1");

    let mut old_manifest = ToolchainManifest::new();
    old_manifest
        .install_authorized(&daemon, component(ToolKind::Cargo, old_package, Target::X86_64))
        .unwrap();
    old_manifest
        .install_authorized(&daemon, component(ToolKind::Rustc, old_package, Target::X86_64))
        .unwrap();
    old_manifest
        .install_authorized(&daemon, component(ToolKind::Linker, old_package, Target::X86_64))
        .unwrap();
    let plan = ToolchainPlan::build(
        &old_manifest,
        request(Target::X86_64),
        ghostos_rustd::ToolchainPolicy::RESTRICTED,
    )
    .unwrap();
    assert_eq!(plan.len(), 3);

    let stage1 = ToolchainStageResult::new(
        ToolchainStage::Stage1,
        Target::X86_64,
        old_package,
        ContentId::hash(b"stage-1-output"),
        Some(ContentId::hash(b"stage-0")),
    )
    .unwrap();
    let bad_stage2 = ToolchainStageResult::new(
        ToolchainStage::Stage2,
        Target::Aarch64,
        ContentId::hash(b"stage-2"),
        ContentId::hash(b"stage-2-output"),
        Some(old_package),
    )
    .unwrap();
    assert_eq!(
        stage1.compare_rebuild(bad_stage2),
        Err(ToolchainStageError::TargetMismatch)
    );
    let replayed_stage2 = ToolchainStageResult::new(
        ToolchainStage::Stage2,
        Target::X86_64,
        ContentId::hash(b"stage-2"),
        ContentId::hash(b"stage-2-output"),
        Some(ContentId::hash(b"other-stage-1")),
    )
    .unwrap();
    assert_eq!(
        stage1.compare_rebuild(replayed_stage2),
        Err(ToolchainStageError::InvalidTransition)
    );

    let mut sandbox = ArtifactSandbox::new();
    let job = JobId::from_raw(7).unwrap();
    let artifact = DynamicArtifact {
        kind: DynamicArtifactKind::ProcMacro,
        package: old_package,
        payload: ContentId::hash(b"old-proc-macro"),
        executable: Text::new("/system/tool/proc-macro").unwrap(),
        target: Target::X86_64,
    };
    sandbox.stage_for_job(job, &daemon, artifact).unwrap();
    sandbox.release_job(job);
    assert_eq!(sandbox.resolve(artifact.payload), Err(ArtifactError::NotFound));
    sandbox.stage_for_job(job, &daemon, artifact).unwrap();
    daemon.revoke_key(old_key.id()).unwrap();
    assert_eq!(
        sandbox.stage(&daemon, artifact),
        Err(ArtifactError::PackageNotAuthorized)
    );

    daemon.trust_key(recovered_key).unwrap();
    let recovered_package = install(
        &mut daemon,
        &mut filesystem,
        recovered_key,
        b"stage-2-recovered",
    );
    let recovered = component(ToolKind::Rustc, recovered_package, Target::X86_64);
    let mut recovered_manifest = ToolchainManifest::new();
    recovered_manifest
        .install_authorized(&daemon, recovered)
        .unwrap();
    assert_eq!(recovered_manifest.component(ToolKind::Rustc), Some(recovered));
    assert!(matches!(
        ToolchainPlan::build(
            &recovered_manifest,
            request(Target::X86_64),
            ghostos_rustd::ToolchainPolicy::RESTRICTED,
        ),
        Err(ToolchainError::MissingComponent(ToolKind::Cargo))
    ));
}
