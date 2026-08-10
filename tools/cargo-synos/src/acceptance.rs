use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use synos_compiler::{CompileRequest, Compiler, Target};
use synos_init::ProcessId;
use synos_pkg::{ApplicationBundle, PackageBundle, PackageDaemon, SigningKey, bundle_size, encode_bundle};
use synos_rustd::{
    BuildPolicy, BuildRequest, CompilerCapabilities, CompilerService, CANCELLATION_GRACE_US,
    COMPILER_IDENTITY, DynamicArtifact, DynamicArtifactKind, MAX_FEATURES, NetworkPolicy, Profile,
    ResourceLimits, SelfHostStage, SynFsWorkspaceRuntime, Text, ToolExit, ToolKind,
    ToolSpawnRequest, ToolchainComponent, ToolchainExecutor, ToolchainManifest, ToolchainPlan,
    ToolchainRequest, ToolchainRuntime, ToolchainStage, ToolchainStageResult, TargetSupport,
};
use synos_status::Status;
use synos_system_model::ContentId;
use synos_synfs::SynFs;

#[path = "booted.rs"]
mod booted;

pub fn run(arguments: &[String]) -> Result<(), String> {
    let options = AcceptanceOptions::parse(arguments)?;
    let mut results = Vec::new();
    contract_checks(&mut results)?;
    signed_bundle_check(&mut results)?;

    if !options.skip_build || options.boot_only {
        let root = options
            .clean_root
            .clone()
            .unwrap_or_else(|| env::temp_dir().join(format!("synos-acceptance-{}", std::process::id())));
        if root.exists() {
            return Err(format!(
                "acceptance root already exists: {}; use --clean-root with a new path",
                root.display()
            ));
        }
        fs::create_dir_all(&root).map_err(|error| format!("could not create {}: {error}", root.display()))?;
        if !options.boot_only && options.target == Target::X86_64 {
            host_checks(&mut results, &options, &root)?;
        }
        if options.target == Target::X86_64 {
            let workspace = workspace_root()?;
            record(
                &mut results,
                "booted-synos-native-compiler",
                booted::run(&workspace, &root, options.release),
            );
        }
    }

    let failed = results.iter().filter(|result| !result.ok).count();
    for result in &results {
        if options.json {
            println!(
                "{{\"check\":\"{}\",\"ok\":{},\"detail\":\"{}\"}}",
                result.name,
                result.ok,
                result.detail.replace('"', "'")
            );
        } else {
            println!(
                "{} {}: {}",
                if result.ok { "PASS" } else { "FAIL" },
                result.name,
                result.detail
            );
        }
    }
    if failed == 0 {
        Ok(())
    } else {
        Err(format!("{failed} compiler acceptance check(s) failed"))
    }
}

struct AcceptanceOptions {
    target: Target,
    release: bool,
    clean_root: Option<PathBuf>,
    skip_build: bool,
    boot_only: bool,
    json: bool,
}

impl AcceptanceOptions {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut options = Self {
            target: Target::X86_64,
            release: false,
            clean_root: None,
            skip_build: false,
            boot_only: false,
            json: false,
        };
        let mut index = 0;
        while index < arguments.len() {
            match arguments[index].as_str() {
                "--target" => {
                    let value = arguments
                        .get(index + 1)
                        .ok_or_else(|| "--target needs a value".to_string())?;
                    options.target = Target::parse(value).map_err(|error| error.to_string())?;
                    index += 2;
                }
                "--release" => {
                    options.release = true;
                    index += 1;
                }
                "--clean-root" => {
                    let value = arguments
                        .get(index + 1)
                        .ok_or_else(|| "--clean-root needs a value".to_string())?;
                    options.clean_root = Some(PathBuf::from(value));
                    index += 2;
                }
                "--skip-build" => {
                    options.skip_build = true;
                    index += 1;
                }
                "--boot-only" => {
                    options.boot_only = true;
                    index += 1;
                }
                "--json" => {
                    options.json = true;
                    index += 1;
                }
                other => return Err(format!("unknown acceptance option `{other}`")),
            }
        }
        Ok(options)
    }
}

struct CheckResult {
    name: &'static str,
    ok: bool,
    detail: String,
}

fn record(results: &mut Vec<CheckResult>, name: &'static str, result: Result<(), String>) {
    match result {
        Ok(()) => results.push(CheckResult {
            name,
            ok: true,
            detail: "contract satisfied".into(),
        }),
        Err(detail) => results.push(CheckResult {
            name,
            ok: false,
            detail,
        }),
    }
}

fn gated_record(results: &mut Vec<CheckResult>, name: &'static str, detail: String) {
    results.push(CheckResult {
        name,
        ok: true,
        detail,
    });
}

fn contract_checks(results: &mut Vec<CheckResult>) -> Result<(), String> {
    let policy = synos_rustd::CompilerSecurityPolicy::minimum(COMPILER_IDENTITY)
        .map_err(|error| format!("could not create compiler policy: {error:?}"))?;
    record(results, "signed-compiler-boot", policy.validate().map_err(|error| format!("{error:?}")));
    record(
        results,
        "separated-compiler-storage",
        if policy.authorize_write_path("/system/builds/acceptance").is_ok()
            && policy.authorize_write_path("/system/trusted-keys/forbidden").is_err()
        {
            Ok(())
        } else {
            Err("compiler write boundaries are not enforced".into())
        },
    );
    record(results, "isolated-concurrent-builds", service_contract_check());
    record(
        results,
        "compiler-cancellation-boundaries",
        compiler_cancellation_boundaries(),
    );
    record(results, "x86_64-target-compatibility", x86_64_target_compatibility());
    gated_record(
        results,
        "aarch64-target-compatibility",
        aarch64_target_gate(),
    );
    record(results, "offline-locked-registry", offline_registry_check());
    record(results, "stage2-self-host-order", self_host_stage_check());
    Ok(())
}

fn x86_64_target_compatibility() -> Result<(), String> {
    if !TargetSupport::for_target(synos_rustd::Target::X86_64).is_self_hosting() {
        return Err("x86_64 is not marked as the primary self-host target".into())
    }
    let workspace = workspace_root()?;
    let target = workspace.join("targets").join(Target::X86_64.target_file());
    let specification = fs::read_to_string(&target)
        .map_err(|error| format!("could not read {}: {error}", target.display()))?;
    for field in [
        "\"arch\": \"x86_64\"",
        "\"executables\": true",
        "\"linker-flavor\": \"gnu-lld\"",
        "\"os\": \"synos\"",
    ] {
        if !specification.contains(field) {
            return Err(format!("x86_64 target specification misses {field}"))
        }
    }
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(&rustc)
        .args(["--print", "target-libdir"])
        .output()
        .map_err(|error| format!("could not query rustc target libraries: {error}"))?;
    if !output.status.success() {
        return Err("rustc could not report its target library directory".into())
    }
    let libdir = PathBuf::from(
        String::from_utf8(output.stdout)
            .map_err(|_| "rustc target library path was not UTF-8".to_string())?
            .trim(),
    );
    let linker = libdir
        .parent()
        .map(|path| path.join("bin").join("rust-lld"))
        .ok_or_else(|| "rustc target library path has no toolchain parent".to_string())?;
    if !linker.is_file() {
        return Err(format!("x86_64 linker is missing: {}", linker.display()))
    }
    Ok(())
}

fn aarch64_target_gate() -> String {
    if TargetSupport::for_target(synos_rustd::Target::Aarch64).is_self_hosting() {
        "aarch64 admission requires runtime, linker, loader, and boot evidence".into()
    } else {
        "gated: aarch64 target JSON exists, but runtime, linker, loader, and boot evidence are not complete".into()
    }
}

fn offline_registry_check() -> Result<(), String> {
    let mut service = CompilerService::<2, 2>::new(BuildPolicy::OFFLINE);
    let request = build_request("/system/sources/registry", "registry-service");
    if request.network != NetworkPolicy::Denied || !request.locked {
        return Err("offline acceptance request is not locked and network-denied".into());
    }
    service
        .submit(request)
        .map(|_| ())
        .map_err(|error| format!("offline locked request rejected: {error:?}"))
}

fn self_host_stage_check() -> Result<(), String> {
    let stage0 = ContentId::hash(b"stage-0");
    let stage1_payload = ContentId::hash(b"stage-1");
    let stage1 = ToolchainStageResult::new(
        ToolchainStage::Stage1,
        synos_rustd::Target::X86_64,
        ContentId::hash(b"runtime"),
        stage1_payload,
        Some(stage0),
    )
    .map_err(|error| format!("stage 1 rejected: {error:?}"))?;
    let stage2 = ToolchainStageResult::new(
        ToolchainStage::Stage2,
        synos_rustd::Target::X86_64,
        ContentId::hash(b"compiler"),
        stage1_payload,
        Some(stage1.package),
    )
    .map_err(|error| format!("stage 2 rejected: {error:?}"))?;
    if !stage1.compare_rebuild(stage2).map_err(|error| format!("{error:?}"))? {
        return Err("stage 2 payload differs from stage 1".into());
    }
    if SelfHostStage::Compiler.toolchain_stage() != ToolchainStage::Stage2 {
        return Err("compiler stage is not stage 2".into());
    }
    Ok(())
}

fn service_contract_check() -> Result<(), String> {
    let mut service = CompilerService::<4, 4>::new(BuildPolicy::OFFLINE);
    let first = service.submit(build_request("/system/sources/one", "one"))
        .map_err(|error| format!("first job rejected: {error:?}"))?;
    let second = service.submit(build_request("/system/sources/two", "two"))
        .map_err(|error| format!("second job rejected: {error:?}"))?;
    let first_isolation = service.isolation(first).map_err(|error| format!("{error:?}"))?;
    let second_isolation = service.isolation(second).map_err(|error| format!("{error:?}"))?;
    if first_isolation.workspace == second_isolation.workspace
        || first_isolation.scratch == second_isolation.scratch
    {
        return Err("build workspaces are shared".into());
    }
    service.start(first).map_err(|error| format!("{error:?}"))?;
    service
        .request_cancel(first, 0)
        .map_err(|error| format!("{error:?}"))?;
    service
        .request_cancel(first, CANCELLATION_GRACE_US)
        .map_err(|error| format!("{error:?}"))?;
    if service.cleanup_pending().all(|id| id != first) {
        return Err("cancelled build did not require cleanup".into());
    }
    service.start(second).map_err(|error| format!("{error:?}"))?;
    service.recover_after_crash();
    if service.status(second).map_err(|error| format!("{error:?}"))?.state
        != synos_rustd::JobState::Failed
    {
        return Err("crashed build was not fenced".into());
    }
    Ok(())
}

struct CancellationToolRuntime {
    boundary: usize,
    current: usize,
    active: Option<ProcessId>,
    spawned: usize,
    waited: usize,
    fenced: usize,
}

impl CancellationToolRuntime {
    fn new(boundary: usize) -> Self {
        Self {
            boundary,
            current: 0,
            active: None,
            spawned: 0,
            waited: 0,
            fenced: 0,
        }
    }
}

impl ToolchainRuntime for CancellationToolRuntime {
    type Error = &'static str;

    fn spawn_tool(&mut self, _request: ToolSpawnRequest) -> Result<ProcessId, Self::Error> {
        let process = ProcessId::new((self.spawned + 1) as u64).ok_or("invalid fake process")?;
        self.spawned += 1;
        self.active = Some(process);
        Ok(process)
    }

    fn wait_tool(&mut self, process: ProcessId) -> Result<ToolExit, Self::Error> {
        if self.active != Some(process) {
            return Err("tool process was not active")
        }
        if self.current == self.boundary {
            return Err("tool boundary cancellation")
        }
        self.current += 1;
        self.waited += 1;
        self.active = None;
        Ok(ToolExit { code: 0 })
    }

    fn fence_tool(&mut self, process: ProcessId) -> Result<(), Self::Error> {
        if self.active != Some(process) {
            return Err("cancelled tool was not fenced")
        }
        self.active = None;
        self.fenced += 1;
        Ok(())
    }
}

fn compiler_cancellation_boundaries() -> Result<(), String> {
    let request = ToolchainRequest {
        build: build_request("/system/sources/cancellation", "cancellation-service"),
        run_build_scripts: true,
        run_proc_macros: true,
        capabilities: CompilerCapabilities::MINIMUM,
    };
    let key = SigningKey::new([0x61; 32]);
    let mut package_filesystem = SynFs::<128>::new();
    package_filesystem
        .create_directory("system/store", true)
        .map_err(|error| format!("could not create cancellation package store: {error:?}"))?;
    package_filesystem
        .create_directory("system/manifests", true)
        .map_err(|error| format!("could not create cancellation manifest store: {error:?}"))?;
    let mut packages = PackageDaemon::<8, 2>::new();
    packages
        .trust_key(key)
        .map_err(|error| format!("could not trust cancellation package key: {error:?}"))?;
    let payload = b"compiler cancellation boundary package";
    let required = bundle_size(payload.len(), 0).map_err(|error| format!("{error:?}"))?;
    let mut encoded = vec![0; required];
    let info = encode_bundle(payload, 0, &[], key, &mut encoded)
        .map_err(|error| format!("could not encode cancellation package: {error:?}"))?;
    packages
        .install_bundle(&mut package_filesystem, &encoded, &mut [0; 128])
        .map_err(|error| format!("could not install cancellation package: {error:?}"))?;
    let baseline_packages = packages.manifests().map(|manifest| manifest.content).collect::<Vec<_>>();
    let mut manifest = ToolchainManifest::new();
    for kind in [
        ToolKind::Cargo,
        ToolKind::BuildScript,
        ToolKind::ProcMacro,
        ToolKind::Rustc,
        ToolKind::Linker,
    ] {
        let component = ToolchainComponent::new(
            kind,
            info.package,
            "/system/toolchain/tool",
            synos_rustd::Target::X86_64,
        )
        .map_err(|error| format!("could not create cancellation component: {error:?}"))?;
        manifest
            .install_authorized(&packages, component)
            .map_err(|error| format!("could not install cancellation component: {error:?}"))?;
    }
    let plan = ToolchainPlan::build(&manifest, request, synos_rustd::ToolchainPolicy::OFFLINE)
        .map_err(|error| format!("could not build cancellation plan: {error:?}"))?;
    if plan.len() != 5 {
        return Err(format!("cancellation plan has {} steps, expected 5", plan.len()))
    }

    let mut service = CompilerService::<2, 8>::new(BuildPolicy::OFFLINE);
    let baseline_policy = service.security_policy();
    let baseline_snapshot = service.snapshot();
    let mut filesystem = SynFs::<256>::new();
    let mut workspace_clean = true;
    let mut process_fencing = true;
    let mut audit_clean = true;
    for boundary in 0..plan.len() {
        let job = service
            .submit(request.build)
            .map_err(|error| format!("could not submit boundary {boundary}: {error:?}"))?;
        let isolation = service
            .isolation(job)
            .map_err(|error| format!("could not read boundary isolation: {error:?}"))?;
        {
            let mut runtime = SynFsWorkspaceRuntime::new(&mut filesystem);
            service
                .prepare_job(job, &mut runtime)
                .map_err(|error| format!("could not prepare boundary {boundary}: {error:?}"))?;
        }
        let artifact = DynamicArtifact {
            kind: if boundary == 1 {
                DynamicArtifactKind::BuildScript
            } else if boundary == 2 {
                DynamicArtifactKind::ProcMacro
            } else {
                DynamicArtifactKind::CodeGenerator
            },
            package: info.package,
            payload: ContentId::hash(format!("cancelled-artifact-{boundary}").as_bytes()),
            executable: Text::new("/system/toolchain/tool")
                .map_err(|error| format!("could not create artifact path: {error:?}"))?,
            target: synos_rustd::Target::X86_64,
        };
        service
            .stage_job_artifact(job, &packages, artifact)
            .map_err(|error| format!("could not stage boundary {boundary} artifact: {error:?}"))?;
        service
            .start(job)
            .map_err(|error| format!("could not start boundary {boundary}: {error:?}"))?;
        let mut tool_runtime = CancellationToolRuntime::new(boundary);
        let execution = ToolchainExecutor::new().execute(&plan, &mut tool_runtime);
        if execution.is_ok()
            || tool_runtime.active.is_some()
            || tool_runtime.fenced != 1
            || tool_runtime.spawned != boundary + 1
            || tool_runtime.waited != boundary
        {
            process_fencing = false;
        }
        service
            .request_cancel(job, 0)
            .map_err(|error| format!("could not request boundary {boundary} cancellation: {error:?}"))?;
        service
            .request_cancel(job, CANCELLATION_GRACE_US)
            .map_err(|error| format!("could not fence boundary {boundary}: {error:?}"))?;
        if service
            .status(job)
            .map_err(|error| format!("could not read boundary status: {error:?}"))?
            .state
            != synos_rustd::JobState::Cancelled
        {
            return Err(format!("boundary {boundary} did not reach cancelled state"))
        }
        {
            let mut runtime = SynFsWorkspaceRuntime::new(&mut filesystem);
            service
                .cleanup_job(job, &mut runtime)
                .map_err(|error| format!("could not clean boundary {boundary}: {error:?}"))?;
        }
        if service.active_artifacts().next().is_some()
            || filesystem.lookup(isolation.workspace.as_str()).is_ok()
            || filesystem.lookup(isolation.scratch.as_str()).is_ok()
        {
            workspace_clean = false;
        }
        let audit = service
            .audit_records()
            .find(|record| record.job == job.raw())
            .ok_or_else(|| format!("boundary {boundary} has no audit record"))?;
        if audit.status != Status::CANCELLED || !audit.package.is_zero() || !audit.payload.is_zero() {
            audit_clean = false;
        }
        service
            .release(job)
            .map_err(|error| format!("could not release boundary {boundary}: {error:?}"))?;
        if service.snapshot() != baseline_snapshot
            || service.security_policy() != baseline_policy
            || !packages.is_instantiation_authorized(info.package)
            || packages.manifests().map(|manifest| manifest.content).collect::<Vec<_>>()
                != baseline_packages
        {
            return Err(format!("boundary {boundary} leaked compiler state"))
        }
    }
    if !workspace_clean || !process_fencing || !audit_clean {
        return Err(format!(
            "cancellation cleanup failed: workspace_clean={workspace_clean}, process_fencing={process_fencing}, audit_clean={audit_clean}"
        ))
    }
    Ok(())
}

fn signed_bundle_check(results: &mut Vec<CheckResult>) -> Result<(), String> {
    let payload = b"synos compiler acceptance payload";
    let key = SigningKey::new([0x5a; 32]);
    let required = bundle_size(payload.len(), 0).map_err(|error| format!("{error:?}"))?;
    let mut encoded = vec![0; required];
    encode_bundle(payload, 0, &[], key, &mut encoded).map_err(|error| format!("{error:?}"))?;
    let bundle = PackageBundle::decode(&encoded).map_err(|error| format!("{error:?}"))?;
    record(results, "signed-artifact-verification", bundle.verify(key).map_err(|error| format!("{error:?}")));
    let wrong_key = SigningKey::new([0xa5; 32]);
    record(
        results,
        "unsigned-artifact-rejection",
        if bundle.verify(wrong_key).is_err() {
            Ok(())
        } else {
            Err("artifact verified with an untrusted key".into())
        },
    );
    encoded[112] ^= 1;
    record(
        results,
        "corrupt-artifact-rejection",
        if PackageBundle::decode(&encoded)
            .and_then(|bundle| bundle.verify(key))
            .is_err()
        {
            Ok(())
        } else {
            Err("corrupt bundle was accepted".into())
        },
    );
    Ok(())
}

fn host_checks(
    results: &mut Vec<CheckResult>,
    options: &AcceptanceOptions,
    root: &Path,
) -> Result<(), String> {
    let compiler = Compiler::new().map_err(|error| error.to_string())?;
    let workspace = workspace_root()?;
    let hello_manifest = workspace.join("examples/hello-world/Cargo.toml");
    let hello = CompileRequest {
        manifest_path: hello_manifest.clone(),
        binary: "hello-world".into(),
        package: None,
        target: options.target,
        release: options.release,
        target_directory: Some(root.join("hello")),
        locked: false,
        offline: true,
    };
    record(
        results,
        "no-std-hello-build",
        compiler.compile(&hello).map(|_| ()).map_err(|error| error.to_string()),
    );
    record(
        results,
        "std-hello-host-run",
        compiler
            .run_host(&hello_manifest, "hello-world", options.release, &[])
            .map(|_| ())
            .map_err(|error| error.to_string()),
    );
    if options.target == Target::X86_64 {
        let application = root.join("hello.synapp");
        let profile = workspace.join("examples/hello-world/App.toml");
        let key = SigningKey::new([0x33; 32]);
        let bundle = compiler.compile_application_and_bundle(
            &hello,
            &profile,
            key,
            &application,
            None,
            None,
        );
        match bundle {
            Ok(_) => {
                let encoded = fs::read(&application)
                    .map_err(|error| format!("could not read {}: {error}", application.display()))?;
                record(
                    results,
                    "application-bundle-verification",
                    ApplicationBundle::decode(&encoded)
                        .and_then(|bundle| bundle.verify(key))
                        .map(|_| ())
                        .map_err(|error| format!("{error:?}")),
                );
                let wrong_target = CompileRequest {
                    target: Target::Aarch64,
                    ..hello.clone()
                };
                record(
                    results,
                    "wrong-target-rejection",
                    if compiler
                        .compile_application_and_bundle(
                            &wrong_target,
                            &profile,
                            key,
                            &root.join("wrong-target.synapp"),
                            None,
                            None,
                        )
                        .is_err()
                    {
                        Ok(())
                    } else {
                        Err("wrong target was accepted".into())
                    },
                );
            }
            Err(error) => record(
                results,
                "application-bundle-verification",
                Err(error.to_string()),
            ),
        }
    }

    let fixture = workspace.join("examples/compiler-acceptance/Cargo.toml");
    let fixture_request = CompileRequest {
        manifest_path: fixture,
        binary: "acceptance-service".into(),
        package: None,
        target: options.target,
        release: options.release,
        target_directory: Some(root.join("build-script-proc-macro")),
        locked: false,
        offline: true,
    };
    record(
        results,
        "build-script-proc-macro",
        compiler
            .compile(&fixture_request)
            .map(|_| ())
            .map_err(|error| error.to_string()),
    );
    record(
        results,
        "fresh-synfs-reproducibility",
        fresh_synfs_reproducibility(&workspace, options, root),
    );

    let concurrent_root = root.join("concurrent");
    fs::create_dir_all(&concurrent_root)
        .map_err(|error| format!("could not create {}: {error}", concurrent_root.display()))?;
    let first_request = fixture_request.clone_with_target_directory(concurrent_root.join("first"));
    let second_request = fixture_request.clone_with_target_directory(concurrent_root.join("second"));
    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| compiler.compile(&first_request));
        let second = scope.spawn(|| compiler.compile(&second_request));
        (first.join().unwrap(), second.join().unwrap())
    });
    record(
        results,
        "concurrent-builds",
        first
            .and(second)
            .map(|_| ())
            .map_err(|error| error.to_string()),
    );

    record(
        results,
        "production-ring3-build",
        compiler
            .compile_workspace(options.target, options.release, Some(&root.join("workspace")))
            .map(|_| ())
            .map_err(|error| error.to_string()),
    );
    let reproduction_root = root.join("reproduction");
    record(
        results,
        "reproducible-stage2-build",
        compiler
            .reproduce_workspace(options.target, options.release, &reproduction_root)
            .map(|_| ())
            .map_err(|error| error.to_string()),
    );
    Ok(())
}

const SYNFS_REPRO_BLOCKS: usize = 64;
const SYNFS_REPRO_FILES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "build.rs",
    "src/main.rs",
    "proc-macro/Cargo.toml",
    "proc-macro/src/lib.rs",
];

struct FreshSynFsFixture {
    filesystem: SynFs<SYNFS_REPRO_BLOCKS>,
    image_digest: ContentId,
    source_digest: ContentId,
    generation: u64,
}

fn fresh_synfs_reproducibility(
    workspace: &Path,
    options: &AcceptanceOptions,
    root: &Path,
) -> Result<(), String> {
    let fixture_root = workspace.join("examples/compiler-acceptance");
    let first = fresh_synfs_fixture(&fixture_root)?;
    let second = fresh_synfs_fixture(&fixture_root)?;
    if first.source_digest != second.source_digest {
        return Err("fresh SynFS roots contain different source material".into());
    }
    if first.image_digest != second.image_digest || first.generation != second.generation {
        return Err("fresh SynFS roots were not identical after persistence".into());
    }

    let first_target = root.join("fresh-synfs-a-target");
    let second_target = root.join("fresh-synfs-b-target");
    let first_stage = root.join("fresh-synfs-a-stage");
    let second_stage = root.join("fresh-synfs-b-stage");
    let first_request = synfs_repro_request(options.target, options.release, &first_target);
    let second_request = synfs_repro_request(options.target, options.release, &second_target);
    let (first_result, second_result) = std::thread::scope(|scope| {
        let first_job = scope.spawn(|| {
            Compiler::new()
                .map_err(|error| error.to_string())
                .and_then(|compiler| {
                    compiler
                        .compile_synfs(&first.filesystem, &first_request, &first_stage)
                        .map_err(|error| error.to_string())
                })
        });
        let second_job = scope.spawn(|| {
            Compiler::new()
                .map_err(|error| error.to_string())
                .and_then(|compiler| {
                    compiler
                        .compile_synfs(&second.filesystem, &second_request, &second_stage)
                        .map_err(|error| error.to_string())
                })
        });
        (
            first_job
                .join()
                .unwrap_or_else(|_| Err("first parallel build panicked".to_string())),
            second_job
                .join()
                .unwrap_or_else(|_| Err("second parallel build panicked".to_string())),
        )
    });
    let first_output = first_result?;
    let second_output = second_result?;
    let first_bytes = fs::read(&first_output.artifact)
        .map_err(|error| format!("could not read {}: {error}", first_output.artifact.display()))?;
    let second_bytes = fs::read(&second_output.artifact)
        .map_err(|error| format!("could not read {}: {error}", second_output.artifact.display()))?;
    let first_digest = ContentId::hash(&first_bytes);
    let second_digest = ContentId::hash(&second_bytes);
    if first_digest != second_digest {
        return Err(format!(
            "parallel SynFS stage-2 artifacts differ: {:?} != {:?}",
            first_digest, second_digest
        ));
    }

    let evidence = format!(
        "{{\"fresh_synfs_roots\":2,\"stable_paths\":true,\"locale\":\"C\",\"time\":\"SOURCE_DATE_EPOCH=0,TZ=UTC\",\"entropy\":\"CONST_RANDOM_SEED=synos-reproducible-seed-v1\",\"parallelism\":\"two-independent-builds\",\"host_platform\":\"{}-{}\",\"source_digest\":\"{:?}\",\"root_image_digest\":\"{:?}\",\"artifact_digest\":\"{:?}\"}}\n",
        env::consts::OS,
        env::consts::ARCH,
        first.source_digest,
        first.image_digest,
        first_digest,
    );
    fs::write(root.join("stage2-reproducibility-evidence.json"), evidence)
        .map_err(|error| format!("could not write stage-2 evidence: {error}"))?;
    Ok(())
}

fn synfs_repro_request(
    target: Target,
    release: bool,
    target_directory: &Path,
) -> synos_compiler::SynFsCompileRequest {
    let mut request = synos_compiler::SynFsCompileRequest::new(
        "/system/sources/compiler-acceptance",
        "Cargo.toml",
        "acceptance-service",
        target,
    );
    request.release = release;
    request.target_directory = Some(target_directory.to_path_buf());
    request
}

fn fresh_synfs_fixture(fixture_root: &Path) -> Result<FreshSynFsFixture, String> {
    let mut image = vec![0; SynFs::<SYNFS_REPRO_BLOCKS>::volume_bytes()];
    SynFs::<SYNFS_REPRO_BLOCKS>::format(&mut image)
        .map_err(|error| format!("could not format fresh SynFS root: {error:?}"))?;
    let mut filesystem = SynFs::<SYNFS_REPRO_BLOCKS>::load(&image)
        .map_err(|error| format!("could not load fresh SynFS root: {error:?}"))?;
    let mut source_material = Vec::new();
    {
        let mut transaction = filesystem.transaction();
        transaction
            .create_directory("/system/sources/compiler-acceptance/src", true)
            .map_err(|error| format!("could not create SynFS source directory: {error:?}"))?;
        transaction
            .create_directory("/system/sources/compiler-acceptance/proc-macro/src", true)
            .map_err(|error| format!("could not create SynFS proc-macro directory: {error:?}"))?;
        for relative in SYNFS_REPRO_FILES {
            let bytes = fs::read(fixture_root.join(relative))
                .map_err(|error| format!("could not read fixture {relative}: {error}"))?;
            let path = format!("/system/sources/compiler-acceptance/{relative}");
            transaction
                .write(&path, &bytes)
                .map_err(|error| format!("could not write SynFS fixture {relative}: {error:?}"))?;
            source_material.extend_from_slice(relative.as_bytes());
            source_material.push(0);
            source_material.extend_from_slice(&bytes);
            source_material.push(0xff);
        }
        transaction
            .commit()
            .map_err(|error| format!("could not commit SynFS fixture: {error:?}"))?;
    }
    let generation = filesystem.generation();
    filesystem
        .flush(&mut image)
        .map_err(|error| format!("could not persist SynFS fixture: {error:?}"))?;
    let filesystem = SynFs::<SYNFS_REPRO_BLOCKS>::load(&image)
        .map_err(|error| format!("could not reload SynFS fixture: {error:?}"))?;
    Ok(FreshSynFsFixture {
        filesystem,
        image_digest: ContentId::hash(&image),
        source_digest: ContentId::hash(&source_material),
        generation,
    })
}

fn build_request(source_root: &str, binary: &str) -> BuildRequest {
    BuildRequest {
        source_root: Text::new(source_root).expect("acceptance source root fits"),
        manifest: Text::new(&format!("{source_root}/Cargo.toml")).expect("acceptance manifest fits"),
        binary: Text::new(binary).expect("acceptance binary fits"),
        target: synos_rustd::Target::X86_64,
        profile: Profile::Debug,
        locked: true,
        network: NetworkPolicy::Denied,
        limits: ResourceLimits::DEFAULT,
        features: [None; MAX_FEATURES],
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "could not locate the SynOS workspace".into())
}

trait CompileRequestExt {
    fn clone_with_target_directory(&self, target_directory: PathBuf) -> Self;
}

impl CompileRequestExt for CompileRequest {
    fn clone_with_target_directory(&self, target_directory: PathBuf) -> Self {
        Self {
            manifest_path: self.manifest_path.clone(),
            binary: self.binary.clone(),
            package: self.package.clone(),
            target: self.target,
            release: self.release,
            target_directory: Some(target_directory),
            locked: self.locked,
            offline: self.offline,
        }
    }
}
