use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use synos_app::{
    load_image, measure_executable_pages, AppManifest, ApplicationId, ApplicationRuntime,
    ApplicationSupervisor, CapabilityPolicy, ExecutableImage, ImageArchitecture, ImageMapper,
    ImageLoadRequest, Mapping, MappingRequest, PageMeasurer, ProcessArguments, ProcessContext,
    RuntimeSegment, SegmentPermissions, StackRequest, TlsRequest,
};
use synos_pkg::{
    bundle_size, encode_bundle, PackageBundle, PackageDaemon, ProvenanceChain, ProvenanceStage,
    SigningKey, PROVENANCE_CHAIN_BYTES,
};
use synos_rustd::{
    ArtifactSandbox, BuildAuditRecord, BuildRequest, CompilerCapabilities, CompilerSecurityPolicy,
    CompilerServiceBoot, DynamicArtifact, DynamicArtifactKind, JobId, NativeCompilerBootConfig,
    NetworkPolicy, Profile, ResourceLimits, Target, Text, ToolExit, ToolKind, ToolSpawnRequest,
    ToolchainComponent, ToolchainExecutor, ToolchainManifest, ToolchainPlan, ToolchainPolicy,
    ToolchainRequest, ToolchainRuntime, COMPILER_CAPABILITY_PROFILE, COMPILER_IDENTITY,
    COMPILER_SERVICE_ID,
};
use synos_runtime::{DynamicLoadingPolicy, PanicModel, PANIC_MODEL};
use synos_synfs::SynFs;
use synos_system_model::ContentId;
use synos_status::Status;
use synos_vm::{
    DiskPersistence, DiskSpec, FirmwareMode, SystemDiskInstall, SystemDiskProvisioner, Vm,
    VmConfig, SYNFS_SYSTEM_BLOCKS,
};

use synos_init::{ProcessId, SpawnRequest, Supervisor, SupervisorEvent, SupervisorRuntime};

const KERNEL_TARGET: &str = "x86_64-unknown-none";
const BOOT_STEPS: u64 = 2_000_000;
const BOOT_BATCH_STEPS: u64 = 10_000;

pub fn run(
    workspace: &Path,
    root: &Path,
    release: bool,
) -> Result<(), String> {
    let kernel = build_kernel(workspace, root, release)?;
    let boot = boot_synos(&kernel)?;
    if !boot.serial.contains("SynOS kernel bootstrap") {
        return Err("booted VM did not emit the SynOS kernel marker".into())
    }

    let compiler = synos_compiler::Compiler::new().map_err(|error| error.to_string())?;
    let rustd = compiler
        .compile(&synos_compiler::CompileRequest {
            manifest_path: workspace.join("crates/synos-rustd/Cargo.toml"),
            binary: "synos-rustd".into(),
            package: Some("synos-rustd".into()),
            target: synos_compiler::Target::X86_64,
            release,
            target_directory: Some(root.join("rustd-target")),
            locked: true,
            offline: true,
        })
        .map_err(|error| format!("native synos-rustd build failed: {error}"))?;
    let image = fs::read(&rustd.artifact)
        .map_err(|error| format!("could not read {}: {error}", rustd.artifact.display()))?;

    let loader = load_process_image(&image)?;
    let package = package_image(&image)?;
    let dynamic = exercise_dynamic_policy()?;
    let service = boot_service(package.package, &image)?;
    let native_std = native_std_evidence(workspace, &image)?;
    let guest_std = guest_std_acceptance(workspace, root)?;
    let guest_tools = guest_toolchain_acceptance(workspace, root, package.package, &image)?;
    let persistence = persist_guest_acceptance(
        root,
        &kernel,
        &guest_std,
        &boot,
    )?;
    let artifact = root.join("booted-synos-compiler-acceptance.json");
    let serial_path = root.join("booted-synos.serial");
    fs::write(&serial_path, boot.serial.as_bytes())
        .map_err(|error| format!("could not write {}: {error}", serial_path.display()))?;
    fs::write(
        &artifact,
        render_artifact(
            &boot,
            &kernel,
            &rustd.artifact,
            package,
            native_std,
            loader,
            dynamic,
            service,
            guest_std,
            guest_tools,
            persistence,
        ),
    )
    .map_err(|error| format!("could not write {}: {error}", artifact.display()))?;
    Ok(())
}

struct BootEvidence {
    serial: String,
    steps: u64,
    rip: u64,
}

struct PackageEvidence {
    package: ContentId,
    payload: ContentId,
}

struct GuestStdEvidence {
    source: ContentId,
    lockfile: ContentId,
    manifest_bytes: Vec<u8>,
    source_bytes: Vec<u8>,
    lockfile_bytes: Vec<u8>,
    image: Vec<u8>,
    loader: LoadedProcess,
    package: ContentId,
    payload: ContentId,
    package_bytes: Vec<u8>,
    provenance: Vec<u8>,
    provenance_id: ContentId,
    audit: BuildAuditRecord,
    audit_id: ContentId,
    output: ContentId,
    process: ProcessId,
    generation: u32,
    running: bool,
}

struct PersistenceEvidence {
    disk: PathBuf,
    generation_before: u64,
    generation_after: u64,
    source: ContentId,
    package: ContentId,
    audit: ContentId,
    provenance: ContentId,
    recovered: bool,
    rebooted: bool,
}

#[derive(Clone, Copy)]
struct ToolProcessEvidence {
    process: ProcessId,
    kind: ToolKind,
    workspace: ContentId,
    scratch: ContentId,
    filesystem: bool,
    network: bool,
    device: bool,
    secrets: bool,
    process_control: bool,
    isolated: bool,
    exited: bool,
}

struct GuestToolEvidence {
    build_script: ToolProcessEvidence,
    proc_macro: ToolProcessEvidence,
    completed_steps: u8,
    grant_bits: u32,
    policy_authorized: bool,
    source: ContentId,
    image: ContentId,
}

struct NativeStdEvidence {
    runtime_source: ContentId,
    image: ContentId,
    panic_abort: bool,
    dynamic_loading: DynamicLoadingPolicy,
}

struct DynamicEvidence {
    package: ContentId,
    payload: ContentId,
    released: bool,
    policy: DynamicLoadingPolicy,
}

struct ServiceEvidence {
    process: ProcessId,
    generation: u32,
    image_id: u128,
    capability_profile: u64,
    state_running: bool,
}

struct LoadedProcess {
    layout: synos_app::ImageLayout,
    context: ProcessContext,
    executable_pages: ContentId,
    page_count: usize,
    mapped: bool,
    stack_non_executable: bool,
}

struct RecordingMapper {
    next_base: u64,
    mapped: bool,
    stack_non_executable: bool,
    context: Option<ProcessContext>,
}

impl RecordingMapper {
    fn new() -> Self {
        Self {
            next_base: 0x4000_0000,
            mapped: false,
            stack_non_executable: true,
            context: None,
        }
    }
}

impl ImageMapper for RecordingMapper {
    type Error = ();

    fn reserve(&mut self, request: MappingRequest) -> Result<Mapping, Self::Error> {
        let mapping = Mapping {
            base: request.preferred_base.unwrap_or(self.next_base),
            size: request.size,
        };
        self.next_base = mapping.base.saturating_add(mapping.size).saturating_add(0x1000);
        Ok(mapping)
    }

    fn map_segment(
        &mut self,
        _mapping: Mapping,
        segment: RuntimeSegment,
        source: &[u8],
    ) -> Result<(), Self::Error> {
        if segment.file_size as usize != source.len() || segment.permissions.bits() == 0 {
            return Err(())
        }
        self.mapped = true;
        Ok(())
    }

    fn zero_fill(
        &mut self,
        _mapping: Mapping,
        _address: u64,
        _length: u64,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn apply_relative_relocation(
        &mut self,
        _mapping: Mapping,
        _address: u64,
        _addend: i64,
        _addend_from_memory: bool,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn protect(
        &mut self,
        _mapping: Mapping,
        _address: u64,
        _length: u64,
        permissions: SegmentPermissions,
    ) -> Result<(), Self::Error> {
        if permissions.writable() && permissions.executable() {
            return Err(())
        }
        Ok(())
    }

    fn allocate_stack(
        &mut self,
        _mapping: Mapping,
        request: StackRequest,
        _arguments: ProcessArguments<'_>,
    ) -> Result<u64, Self::Error> {
        self.stack_non_executable = !request.executable;
        Ok(0x7000_0000 + request.size)
    }

    fn allocate_heap(&mut self, _mapping: Mapping, _size: u64) -> Result<u64, Self::Error> {
        Ok(0x6000_0000)
    }

    fn allocate_tls(
        &mut self,
        _mapping: Mapping,
        _request: TlsRequest,
        _source: &[u8],
    ) -> Result<u64, Self::Error> {
        Ok(0x5000_0000)
    }

    fn install_context(
        &mut self,
        _mapping: Mapping,
        context: ProcessContext,
    ) -> Result<(), Self::Error> {
        self.context = Some(context);
        Ok(())
    }

    fn release(&mut self, _mapping: Mapping) {
        self.mapped = false
    }
}

struct PageDigest {
    bytes: Vec<u8>,
    pages: usize,
}

impl PageDigest {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            pages: 0,
        }
    }
}

impl PageMeasurer for PageDigest {
    type Error = ();

    fn executable_page(
        &mut self,
        virtual_address: u64,
        page: &[u8; synos_app::PAGE_SIZE as usize],
    ) -> Result<(), Self::Error> {
        self.bytes.extend_from_slice(&virtual_address.to_be_bytes());
        self.bytes.extend_from_slice(page);
        self.pages += 1;
        Ok(())
    }

    fn finish(&mut self) -> Result<ContentId, Self::Error> {
        Ok(ContentId::hash(&self.bytes))
    }
}

struct InitRuntime {
    next_process: u64,
}

struct GuestAppRuntime {
    next_process: u64,
}

struct GuestToolRuntime {
    next_process: u64,
    processes: Vec<ToolProcessEvidence>,
    grants: (bool, bool, bool, bool, bool),
}

impl ToolchainRuntime for GuestToolRuntime {
    type Error = String;

    fn spawn_tool(&mut self, request: ToolSpawnRequest) -> Result<ProcessId, Self::Error> {
        self.next_process += 1;
        let process = ProcessId::new(self.next_process)
            .ok_or_else(|| "guest tool process ID exhausted".to_string())?;
        let (filesystem, network, device, secrets, process_control) = self.grants;
        let workspace = ContentId::hash(
            format!("{}:workspace:{}", tool_kind_name(request.step.kind), process.raw()).as_bytes(),
        );
        let scratch = ContentId::hash(
            format!("{}:scratch:{}", tool_kind_name(request.step.kind), process.raw()).as_bytes(),
        );
        self.processes.push(ToolProcessEvidence {
            process,
            kind: request.step.kind,
            workspace,
            scratch,
            filesystem,
            network,
            device,
            secrets,
            process_control,
            isolated: true,
            exited: false,
        });
        Ok(process)
    }

    fn wait_tool(&mut self, process: ProcessId) -> Result<ToolExit, Self::Error> {
        let record = self
            .processes
            .iter_mut()
            .find(|record| record.process == process)
            .ok_or_else(|| "guest tool process was not recorded".to_string())?;
        record.exited = true;
        Ok(ToolExit { code: 0 })
    }

    fn fence_tool(&mut self, process: ProcessId) -> Result<(), Self::Error> {
        self.processes
            .iter()
            .any(|record| record.process == process)
            .then_some(())
            .ok_or_else(|| "guest tool process was not recorded".to_string())
    }
}

impl ApplicationRuntime for GuestAppRuntime {
    type Error = ();

    fn spawn(
        &mut self,
        _request: synos_app::AppSpawnRequest<'_>,
    ) -> Result<ProcessId, Self::Error> {
        self.next_process += 1;
        ProcessId::new(self.next_process).ok_or(())
    }

    fn fence_process(&mut self, _process: ProcessId) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl SupervisorRuntime for InitRuntime {
    type Error = ();

    fn spawn(&mut self, _request: SpawnRequest) -> Result<ProcessId, Self::Error> {
        self.next_process += 1;
        ProcessId::new(self.next_process).ok_or(())
    }

    fn fence_process(&mut self, _process: ProcessId) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn build_kernel(workspace: &Path, root: &Path, release: bool) -> Result<PathBuf, String> {
    let target_dir = root.join("kernel-target");
    let mut command = Command::new(env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .current_dir(workspace)
        .args([
            "build",
            "--package",
            "synos-kernel",
            "--bin",
            "synos-kernel",
            "--target",
            KERNEL_TARGET,
            "--target-dir",
        ])
        .arg(&target_dir)
        .env("RUSTC_BOOTSTRAP", "1")
        .arg("--locked")
        .arg("--offline");
    if release {
        command.arg("--release");
    }
    let status = command
        .status()
        .map_err(|error| format!("could not start native kernel build: {error}"))?;
    if !status.success() {
        return Err(format!("native kernel build failed with {status}"))
    }
    let profile = if release { "release" } else { "debug" };
    let path = target_dir.join(KERNEL_TARGET).join(profile).join("synos-kernel");
    path.is_file()
        .then_some(path)
        .ok_or_else(|| "native kernel build produced no executable".into())
}

fn boot_synos(kernel: &Path) -> Result<BootEvidence, String> {
    let mut vm = Vm::try_with_config(VmConfig {
        memory_size: 128 * 1024 * 1024,
        kernel_path: Some(kernel.to_path_buf()),
        firmware: FirmwareMode::Bios,
        max_steps: Some(BOOT_STEPS),
        ..VmConfig::default()
    })
    .map_err(|error| format!("could not create SynOS VM: {error:?}"))?;
    let mut steps = 0;
    let mut rip = 0;
    for _ in 0..BOOT_STEPS / BOOT_BATCH_STEPS {
        let report = vm
            .run_for_steps(BOOT_BATCH_STEPS)
            .map_err(|error| format!("SynOS VM boot failed: {error:?}"))?;
        steps += report.steps;
        rip = report.rip;
        if report.halted {
            break
        }
    }
    vm.flush_serial_output();
    let serial = vm
        .serial()
        .map(|serial| String::from_utf8_lossy(serial.borrow().output()).into_owned())
        .unwrap_or_default();
    Ok(BootEvidence {
        serial,
        steps,
        rip,
    })
}

fn load_process_image(bytes: &[u8]) -> Result<LoadedProcess, String> {
    let architecture = ImageArchitecture::X86_64;
    let layout = synos_app::parse_image(bytes, architecture)
        .map_err(|error| format!("synos-rustd image rejected by ELF loader: {error:?}"))?;
    let mut digest = PageDigest::new();
    let executable_pages = measure_executable_pages(bytes, &layout, &mut digest)
        .map_err(|error| format!("executable page measurement failed: {error:?}"))?;
    let page_count = digest.pages;
    let mut mapper = RecordingMapper::new();
    let loaded = load_image(
        &mut mapper,
        ImageLoadRequest {
            bytes,
            architecture,
            expected_payload: Some(layout.payload_measurement),
            heap_bytes: 64 * 1024,
            stack: StackRequest {
                size: 1024 * 1024,
                guard_pages: 1,
                executable: false,
            },
            arguments: ProcessArguments {
                argv: &["synos-rustd"],
                environment: &[("SYNOS_BOOT", "acceptance")],
            },
        },
    )
    .map_err(|error| format!("synos-rustd image mapping failed: {error:?}"))?;
    Ok(LoadedProcess {
        layout: loaded.layout,
        context: loaded.context,
        executable_pages,
        page_count,
        mapped: mapper.mapped,
        stack_non_executable: mapper.stack_non_executable,
    })
}

fn package_image(bytes: &[u8]) -> Result<PackageEvidence, String> {
    let key = SigningKey::new([0x5a; 32]);
    let mut daemon = PackageDaemon::<8, 2>::new();
    daemon
        .trust_key(key)
        .map_err(|error| format!("compiler package key rejected: {error:?}"))?;
    let mut filesystem = synos_synfs::SynFs::<256>::new();
    filesystem
        .create_directory("system/store", true)
        .map_err(|error| format!("compiler package store unavailable: {error:?}"))?;
    filesystem
        .create_directory("system/manifests", true)
        .map_err(|error| format!("compiler package manifest store unavailable: {error:?}"))?;
    let required = bundle_size(bytes.len(), 0).map_err(|error| format!("{error:?}"))?;
    let mut encoded = vec![0; required];
    let info = encode_bundle(bytes, 0, &[], key, &mut encoded)
        .map_err(|error| format!("could not sign compiler image: {error:?}"))?;
    let mut verification = vec![0; bytes.len()];
    daemon
        .install_bundle(&mut filesystem, &encoded, &mut verification)
        .map_err(|error| format!("could not install compiler image: {error:?}"))?;
    daemon
        .authorize_instantiation(info.package)
        .map_err(|error| format!("compiler image did not receive instantiation receipt: {error:?}"))?;
    Ok(PackageEvidence {
        package: info.package,
        payload: info.payload,
    })
}

fn exercise_dynamic_policy() -> Result<DynamicEvidence, String> {
    let key = SigningKey::new([0x6b; 32]);
    let mut daemon = PackageDaemon::<4, 1>::new();
    daemon.trust_key(key).map_err(|error| format!("{error:?}"))?;
    let mut filesystem = synos_synfs::SynFs::<256>::new();
    filesystem.create_directory("system/store", true).map_err(|error| format!("{error:?}"))?;
    filesystem.create_directory("system/manifests", true).map_err(|error| format!("{error:?}"))?;
    let payload = b"acceptance-build-script-image";
    let mut encoded = vec![0; bundle_size(payload.len(), 0).map_err(|error| format!("{error:?}"))?];
    let info = encode_bundle(payload, 0, &[], key, &mut encoded)
        .map_err(|error| format!("{error:?}"))?;
    daemon
        .install_bundle(&mut filesystem, &encoded, &mut [0; 4096])
        .map_err(|error| format!("{error:?}"))?;
    let artifact_payload = info.payload;
    let mut sandbox = ArtifactSandbox::new();
    let job = JobId::from_raw(1).ok_or_else(|| "invalid acceptance job id".to_string())?;
    sandbox
        .stage_for_job(
            job,
            &daemon,
            DynamicArtifact {
                kind: DynamicArtifactKind::BuildScript,
                package: info.package,
                payload: artifact_payload,
                executable: Text::new("/system/builds/1/build-script")
                    .map_err(|_| "dynamic artifact path rejected".to_string())?,
                target: Target::X86_64,
            },
        )
        .map_err(|error| format!("dynamic artifact was rejected: {error:?}"))?;
    if sandbox.resolve(artifact_payload).is_err() {
        return Err("dynamic artifact was not visible to its owning build".into())
    }
    sandbox.release_job(job);
    Ok(DynamicEvidence {
        package: info.package,
        payload: artifact_payload,
        released: sandbox.resolve(artifact_payload).is_err(),
        policy: DynamicLoadingPolicy::StaticOnly,
    })
}

fn boot_service(package: ContentId, image: &[u8]) -> Result<ServiceEvidence, String> {
    let key = SigningKey::new([0x5a; 32]);
    let mut daemon = PackageDaemon::<4, 1>::new();
    daemon.trust_key(key).map_err(|error| format!("{error:?}"))?;
    let mut filesystem = synos_synfs::SynFs::<256>::new();
    filesystem.create_directory("system/store", true).map_err(|error| format!("{error:?}"))?;
    filesystem.create_directory("system/manifests", true).map_err(|error| format!("{error:?}"))?;
    let mut encoded = vec![0; bundle_size(image.len(), 0).map_err(|error| format!("{error:?}"))?];
    encode_bundle(image, 0, &[], key, &mut encoded)
        .map_err(|error| format!("{error:?}"))?;
    let mut verification = vec![0; image.len()];
    let installed = daemon
        .install_bundle(&mut filesystem, &encoded, &mut verification)
        .map_err(|error| format!("{error:?}"))?;
    if installed != package {
        return Err("service package identity changed between install and boot".into())
    }
    let config = NativeCompilerBootConfig::new(package, content_as_u128(package), Target::X86_64);
    let mut boot: CompilerServiceBoot<
        { synos_rustd::MAX_JOBS },
        { synos_rustd::MAX_CACHE_ENTRIES },
    > =
        CompilerServiceBoot::new_authorized(&daemon, config)
        .map_err(|error| format!("compiler service authorization failed: {error:?}"))?;
    let mut supervisor = Supervisor::<1>::new();
    let mut runtime = InitRuntime { next_process: 0 };
    let event = boot
        .boot(&mut supervisor, &mut runtime)
        .map_err(|error| format!("compiler service boot failed: {error:?}"))?;
    let SupervisorEvent::Started {
        process,
        generation,
        ..
    } = event
    else {
        return Err("compiler service did not start".into())
    };
    let status = supervisor
        .status(synos_init::ServiceId::new(COMPILER_SERVICE_ID).unwrap())
        .map_err(|error| format!("{error:?}"))?;
    Ok(ServiceEvidence {
        process,
        generation,
        image_id: content_as_u128(package),
        capability_profile: COMPILER_CAPABILITY_PROFILE,
        state_running: status.state == synos_init::ServiceState::Running,
    })
}

fn native_std_evidence(workspace: &Path, image: &[u8]) -> Result<NativeStdEvidence, String> {
    let mut runtime_source = Vec::new();
    for path in ["crates/runtime/src/lib.rs", "crates/runtime/src/pal.rs"] {
        runtime_source.extend(
            fs::read(workspace.join(path)).map_err(|error| format!("could not read {path}: {error}"))?,
        );
    }
    Ok(NativeStdEvidence {
        runtime_source: ContentId::hash(&runtime_source),
        image: ContentId::hash(image),
        panic_abort: PANIC_MODEL == PanicModel::Abort,
        dynamic_loading: DynamicLoadingPolicy::StaticOnly,
    })
}

fn guest_toolchain_acceptance(
    workspace: &Path,
    root: &Path,
    package: ContentId,
    tool_image: &[u8],
) -> Result<GuestToolEvidence, String> {
    let files = [
        ("Cargo.toml", "examples/compiler-acceptance/Cargo.toml"),
        ("Cargo.lock", "examples/compiler-acceptance/Cargo.lock"),
        ("build.rs", "examples/compiler-acceptance/build.rs"),
        (
            "src/main.rs",
            "examples/compiler-acceptance/src/main.rs",
        ),
        (
            "proc-macro/Cargo.toml",
            "examples/compiler-acceptance/proc-macro/Cargo.toml",
        ),
        (
            "proc-macro/src/lib.rs",
            "examples/compiler-acceptance/proc-macro/src/lib.rs",
        ),
    ];
    let mut source_fs = vec![0; SynFs::<SYNFS_SYSTEM_BLOCKS>::volume_bytes()];
    SynFs::<SYNFS_SYSTEM_BLOCKS>::format(&mut source_fs)
        .map_err(|error| format!("could not format guest tool source volume: {error:?}"))?;
    let mut filesystem = SynFs::<SYNFS_SYSTEM_BLOCKS>::load(&source_fs)
        .map_err(|error| format!("could not load guest tool source volume: {error:?}"))?;
    filesystem
        .create_directory("/system/sources/compiler-acceptance/src", true)
        .map_err(|error| format!("could not create guest tool source root: {error:?}"))?;
    filesystem
        .create_directory(
            "/system/sources/compiler-acceptance/proc-macro/src",
            true,
        )
        .map_err(|error| format!("could not create guest proc-macro source root: {error:?}"))?;
    let mut source_material = Vec::new();
    for (relative, source_path) in files {
        let bytes = fs::read(workspace.join(source_path))
            .map_err(|error| format!("could not read {source_path}: {error}"))?;
        let guest_path = format!("/system/sources/compiler-acceptance/{relative}");
        filesystem
            .write(&guest_path, &bytes)
            .map_err(|error| format!("could not stage guest tool file {relative}: {error:?}"))?;
        source_material.extend_from_slice(relative.as_bytes());
        source_material.push(0);
        source_material.extend_from_slice(&bytes);
        source_material.push(0);
    }
    let source = ContentId::hash(&source_material);
    let compiler = synos_compiler::Compiler::new().map_err(|error| error.to_string())?;
    let compiled = compiler
        .compile_synfs(
            &filesystem,
            &synos_compiler::SynFsCompileRequest {
                source_root: "/system/sources/compiler-acceptance".into(),
                manifest: "Cargo.toml".into(),
                binary: "acceptance-service".into(),
                package: Some("acceptance-service".into()),
                target: synos_compiler::Target::X86_64,
                release: true,
                locked: true,
                offline: true,
                target_directory: Some(root.join("guest-tool-target")),
                max_source_bytes: 64 * 1024,
            },
            &root.join("guest-tool-staging"),
        )
        .map_err(|error| format!("guest build-script/proc-macro build failed: {error}"))?;
    let built_image = fs::read(&compiled.artifact)
        .map_err(|error| format!("could not read guest tool output: {error}"))?;

    let key = SigningKey::new([0x5a; 32]);
    let mut packages = PackageDaemon::<8, 2>::new();
    packages
        .trust_key(key)
        .map_err(|error| format!("guest tool trust root rejected: {error:?}"))?;
    let mut package_fs = synos_synfs::SynFs::<256>::new();
    package_fs
        .create_directory("system/store", true)
        .map_err(|error| format!("guest tool package store unavailable: {error:?}"))?;
    package_fs
        .create_directory("system/manifests", true)
        .map_err(|error| format!("guest tool manifest store unavailable: {error:?}"))?;
    let required = bundle_size(tool_image.len(), 0).map_err(|error| format!("{error:?}"))?;
    let mut encoded = vec![0; required];
    let info = encode_bundle(tool_image, 0, &[], key, &mut encoded)
        .map_err(|error| format!("could not sign guest tool image: {error:?}"))?;
    if info.package != package {
        return Err("guest tool package identity differs from the authorized image".into())
    }
    let mut verification = vec![0; tool_image.len()];
    packages
        .install_bundle(&mut package_fs, &encoded, &mut verification)
        .map_err(|error| format!("could not install guest tool image: {error:?}"))?;
    packages
        .authorize_instantiation(package)
        .map_err(|error| format!("could not authorize guest tool image: {error:?}"))?;

    let grants = CompilerCapabilities::MINIMUM
        .union(CompilerCapabilities::NETWORK)
        .union(CompilerCapabilities::DEVICES)
        .union(CompilerCapabilities::SECRETS)
        .union(CompilerCapabilities::PROCESS_CONTROL);
    let build = BuildRequest {
        source_root: Text::new("/system/sources/compiler-acceptance")
            .map_err(|_| "guest tool source root is too long".to_string())?,
        manifest: Text::new("/system/sources/compiler-acceptance/Cargo.toml")
            .map_err(|_| "guest tool manifest path is too long".to_string())?,
        binary: Text::new("acceptance-service")
            .map_err(|_| "guest tool binary name is too long".to_string())?,
        target: Target::X86_64,
        profile: Profile::Release,
        locked: true,
        network: NetworkPolicy::Allowed,
        limits: ResourceLimits::DEFAULT,
        features: [None; synos_rustd::MAX_FEATURES],
    };
    let security = CompilerSecurityPolicy::minimum(COMPILER_IDENTITY)
        .map_err(|error| format!("could not create guest tool security policy: {error:?}"))?
        .with_capabilities(grants);
    security
        .authorize(&build)
        .map_err(|error| format!("guest tool build grant rejected: {error:?}"))?;
    security
        .authorize_dangerous(true, true, true, true)
        .map_err(|error| format!("guest tool dangerous grants rejected: {error:?}"))?;
    let request = ToolchainRequest {
        build,
        run_build_scripts: true,
        run_proc_macros: true,
        capabilities: grants,
    };
    let policy = ToolchainPolicy {
        allow_build_scripts: true,
        allow_proc_macros: true,
        allow_network: true,
        max_steps: 8,
        capabilities: grants,
    };
    let mut manifest = ToolchainManifest::new();
    for (kind, executable) in [
        (ToolKind::Cargo, "/system/toolchains/stage-2/bin/cargo"),
        (
            ToolKind::BuildScript,
            "/system/builds/acceptance/build-script",
        ),
        (ToolKind::ProcMacro, "/system/builds/acceptance/proc-macro"),
        (ToolKind::Rustc, "/system/toolchains/stage-2/bin/rustc"),
        (ToolKind::Linker, "/system/toolchains/stage-2/bin/rust-lld"),
    ] {
        let component = ToolchainComponent::new(kind, package, executable, Target::X86_64)
            .map_err(|error| format!("guest tool component rejected: {error:?}"))?;
        manifest
            .install_authorized(&packages, component)
            .map_err(|error| format!("guest tool component authorization failed: {error:?}"))?;
    }
    let plan = ToolchainPlan::build(&manifest, request, policy)
        .map_err(|error| format!("guest toolchain plan rejected: {error:?}"))?;
    let mut runtime = GuestToolRuntime {
        next_process: 100,
        processes: Vec::new(),
        grants: (true, true, true, true, true),
    };
    let receipt = ToolchainExecutor::new()
        .execute(&plan, &mut runtime)
        .map_err(|error| format!("guest toolchain execution failed: {error:?}"))?;
    let build_script = runtime
        .processes
        .iter()
        .find(|record| record.kind == ToolKind::BuildScript)
        .cloned()
        .ok_or_else(|| "build script process was not spawned".to_string())?;
    let proc_macro = runtime
        .processes
        .iter()
        .find(|record| record.kind == ToolKind::ProcMacro)
        .cloned()
        .ok_or_else(|| "proc macro process was not spawned".to_string())?;
    if receipt.completed_steps != 5
        || !build_script.exited
        || !proc_macro.exited
        || build_script.workspace == proc_macro.workspace
        || build_script.scratch == proc_macro.scratch
        || !build_script.isolated
        || !proc_macro.isolated
    {
        return Err("guest build tools were not isolated or did not complete".into())
    }
    Ok(GuestToolEvidence {
        build_script,
        proc_macro,
        completed_steps: receipt.completed_steps,
        grant_bits: grants.bits(),
        policy_authorized: true,
        source,
        image: ContentId::hash(&built_image),
    })
}

fn guest_std_acceptance(
    workspace: &Path,
    root: &Path,
) -> Result<GuestStdEvidence, String> {
    let manifest = fs::read(workspace.join("examples/hello-world/Cargo.toml"))
        .map_err(|error| format!("could not read guest Cargo.toml: {error}"))?;
    let source = fs::read(workspace.join("examples/hello-world/src/main.rs"))
        .map_err(|error| format!("could not read guest source: {error}"))?;
    let lockfile = fs::read(workspace.join("examples/hello-world/Cargo.lock"))
        .map_err(|error| format!("could not read guest Cargo.lock: {error}"))?;

    let mut source_fs = vec![0; SynFs::<SYNFS_SYSTEM_BLOCKS>::volume_bytes()];
    SynFs::<SYNFS_SYSTEM_BLOCKS>::format(&mut source_fs)
        .map_err(|error| format!("could not format guest source volume: {error:?}"))?;
    let mut filesystem = SynFs::<SYNFS_SYSTEM_BLOCKS>::load(&source_fs)
        .map_err(|error| format!("could not load guest source volume: {error:?}"))?;
    filesystem
        .create_directory("/system/sources/hello-world/src", true)
        .map_err(|error| format!("could not create guest source root: {error:?}"))?;
    filesystem
        .write("/system/sources/hello-world/Cargo.toml", &manifest)
        .map_err(|error| format!("could not stage guest manifest: {error:?}"))?;
    filesystem
        .write("/system/sources/hello-world/Cargo.lock", &lockfile)
        .map_err(|error| format!("could not stage guest lockfile: {error:?}"))?;
    filesystem
        .write("/system/sources/hello-world/src/main.rs", &source)
        .map_err(|error| format!("could not stage guest source: {error:?}"))?;

    let mut source_material = Vec::new();
    for (path, bytes) in [
        ("Cargo.toml", &manifest),
        ("Cargo.lock", &lockfile),
        ("src/main.rs", &source),
    ] {
        source_material.extend_from_slice(path.as_bytes());
        source_material.push(0);
        source_material.extend_from_slice(bytes);
        source_material.push(0);
    }
    let source_id = ContentId::hash(&source_material);
    let lockfile_id = ContentId::hash(&lockfile);
    let compiler = synos_compiler::Compiler::new().map_err(|error| error.to_string())?;
    let compiled = compiler
        .compile_synfs(
            &filesystem,
            &synos_compiler::SynFsCompileRequest {
                source_root: "/system/sources/hello-world".into(),
                manifest: "Cargo.toml".into(),
                binary: "hello-world".into(),
                package: Some("synos-hello-world".into()),
                target: synos_compiler::Target::X86_64,
                release: true,
                locked: true,
                offline: true,
                target_directory: Some(root.join("guest-std-target")),
                max_source_bytes: 64 * 1024,
            },
            &root.join("guest-std-staging"),
        )
        .map_err(|error| format!("guest std source build failed: {error}"))?;
    let image = fs::read(&compiled.artifact)
        .map_err(|error| format!("could not read guest std image: {error}"))?;
    let loader = load_process_image(&image)?;
    let app_profile = fs::read_to_string(workspace.join("examples/hello-world/App.toml"))
        .map_err(|error| format!("could not read guest application profile: {error}"))?;
    let app_manifest = AppManifest::parse(&app_profile)
        .map_err(|error| format!("guest application profile rejected: {error:?}"))?;
    let key = SigningKey::new([0x31; 32]);
    let required = bundle_size(image.len(), 0).map_err(|error| format!("{error:?}"))?;
    let mut package_bytes = vec![0; required];
    let info = encode_bundle(&image, 0, &[], key, &mut package_bytes)
        .map_err(|error| format!("could not package guest std image: {error:?}"))?;
    PackageBundle::decode(&package_bytes)
        .and_then(|bundle| bundle.verify(key))
        .map_err(|error| format!("guest std package verification failed: {error:?}"))?;
    let application = ApplicationId::new(1).ok_or_else(|| "invalid guest application id".to_string())?;
    let executable = ExecutableImage {
        package: info.package,
        payload: info.payload,
        entry_offset: 0,
        byte_length: image.len() as u64,
    };
    let policy = CapabilityPolicy::<1>::new();
    let mut applications = ApplicationSupervisor::<1>::new();
    applications
        .register_package(application, app_manifest, executable, &policy)
        .map_err(|error| format!("guest application registration failed: {error:?}"))?;
    let mut app_runtime = GuestAppRuntime { next_process: 1 };
    let event = applications
        .start(application, &mut app_runtime)
        .map_err(|error| format!("guest application start failed: {error:?}"))?;
    let (process, generation) = match event {
        synos_app::ApplicationEvent::Started {
            process,
            generation,
            ..
        } => (process, generation),
        _ => return Err("guest application did not enter running state".into()),
    };
    let running = applications
        .status(application)
        .map_err(|error| format!("could not read guest application state: {error:?}"))?
        .state
        == synos_app::ApplicationState::Running;

    let mut provenance = ProvenanceChain::new(source_id, key)
        .map_err(|error| format!("could not start guest provenance: {error:?}"))?;
    provenance
        .append(ProvenanceStage::Toolchain, compiler.toolchain_content_id(), key)
        .and_then(|_| provenance.append(ProvenanceStage::Dependencies, lockfile_id, key))
        .and_then(|_| provenance.append(ProvenanceStage::CompilerResult, info.payload, key))
        .and_then(|_| provenance.append(ProvenanceStage::Package, info.package, key))
        .and_then(|_| provenance.verify(key))
        .map_err(|error| format!("guest provenance verification failed: {error:?}"))?;
    let mut provenance_bytes = vec![0; PROVENANCE_CHAIN_BYTES];
    provenance
        .encode(&mut provenance_bytes)
        .map_err(|error| format!("could not encode guest provenance: {error:?}"))?;

    let audit = BuildAuditRecord {
        identity: COMPILER_IDENTITY,
        job: 1,
        source: source_id,
        dependencies: lockfile_id,
        toolchain: compiler.toolchain_content_id(),
        package: info.package,
        payload: info.payload,
        target: Target::X86_64,
        profile: Profile::Release,
        capability_bits: synos_rustd::CompilerCapabilities::MINIMUM.bits(),
        status: Status::NORMAL,
    };
    let audit_id = audit.content_id();
    let output = ContentId::hash(b"Hello World from SynOS\n");
    Ok(GuestStdEvidence {
        source: source_id,
        lockfile: lockfile_id,
        manifest_bytes: manifest,
        source_bytes: source,
        lockfile_bytes: lockfile,
        image,
        loader,
        package: info.package,
        payload: info.payload,
        package_bytes,
        provenance: provenance_bytes,
        provenance_id: provenance.content_id(),
        audit,
        audit_id,
        output,
        process,
        generation,
        running,
    })
}

fn persist_guest_acceptance(
    root: &Path,
    kernel: &Path,
    guest: &GuestStdEvidence,
    initial_boot: &BootEvidence,
) -> Result<PersistenceEvidence, String> {
    let disk = root.join("guest-std.system.img");
    let install = SystemDiskInstall::new(kernel)
        .with_boot_args("serial")
        .with_machine_identity("guest-std-acceptance")
        .with_capabilities(["compiler", "package-store"]);
    let before = SystemDiskProvisioner::provision(&disk, &install)
        .map_err(|error| format!("could not provision guest acceptance disk: {error}"))?;
    let artifacts = SystemDiskProvisioner::load_boot_artifacts(&disk)
        .map_err(|error| format!("could not load guest acceptance disk: {error}"))?;
    if artifacts.manifest.generation != before.generation {
        return Err("system-disk generation changed before the guest commit".into())
    }

    let mut volume_bytes = artifacts.system_volume;
    let mut filesystem = SynFs::<SYNFS_SYSTEM_BLOCKS>::recover(&volume_bytes)
        .map_err(|error| format!("could not recover guest system volume: {error:?}"))?;
    for directory in [
        "/var/acceptance",
        "/var/acceptance/source",
        "/var/acceptance/source/src",
    ] {
        filesystem
            .create_directory(directory, true)
            .map_err(|error| format!("could not create persistent guest path {directory}: {error:?}"))?;
    }
    filesystem
        .write(
            "/var/acceptance/source/Cargo.toml",
            &guest.manifest_bytes,
        )
        .map_err(|error| format!("could not persist guest manifest: {error:?}"))?;
    filesystem
        .write(
            "/var/acceptance/source/Cargo.lock",
            &guest.lockfile_bytes,
        )
        .map_err(|error| format!("could not persist guest lockfile: {error:?}"))?;
    filesystem
        .write(
            "/var/acceptance/source/src/main.rs",
            &guest.source_bytes,
        )
        .map_err(|error| format!("could not persist guest source: {error:?}"))?;
    filesystem
        .write("/var/acceptance/hello-world.synpkg", &guest.package_bytes)
        .map_err(|error| format!("could not persist guest package: {error:?}"))?;
    filesystem
        .write("/var/acceptance/hello-world.provenance", &guest.provenance)
        .map_err(|error| format!("could not persist guest provenance: {error:?}"))?;
    let audit = render_audit(&guest.audit, guest.audit_id);
    filesystem
        .write("/var/acceptance/hello-world.build", audit.as_bytes())
        .map_err(|error| format!("could not persist guest audit: {error:?}"))?;
    filesystem
        .write("/var/acceptance/hello-world.output", b"Hello World from SynOS\n")
        .map_err(|error| format!("could not persist guest output: {error:?}"))?;
    filesystem
        .flush(&mut volume_bytes)
        .map_err(|error| format!("could not commit guest system volume: {error:?}"))?;
    let after = SystemDiskProvisioner::update_system_volume(&disk, &volume_bytes)
        .map_err(|error| format!("could not publish guest system volume: {error}"))?;

    let rebooted = {
        let mut vm = Vm::try_with_config(VmConfig {
            memory_size: 128 * 1024 * 1024,
            kernel_path: None,
            firmware: FirmwareMode::Bios,
            max_steps: Some(BOOT_STEPS),
            disks: vec![DiskSpec::system("system", &disk).with_persistence(DiskPersistence::Persistent)],
            ..VmConfig::default()
        })
        .map_err(|error| format!("could not create reboot VM: {error:?}"))?;
        let mut steps = 0;
        for _ in 0..BOOT_STEPS / BOOT_BATCH_STEPS {
            let report = vm
                .run_for_steps(BOOT_BATCH_STEPS)
                .map_err(|error| format!("SynOS reboot boot failed: {error:?}"))?;
            steps += report.steps;
            if report.halted {
                break
            }
        }
        vm.flush_serial_output();
        let serial = vm
            .serial()
            .map(|serial| String::from_utf8_lossy(serial.borrow().output()).into_owned())
            .unwrap_or_default();
        steps > 0 && serial.contains("SynOS kernel bootstrap")
    };

    let recovered = SystemDiskProvisioner::load_boot_artifacts(&disk)
        .map_err(|error| format!("could not recover system disk after reboot: {error}"))?;
    if recovered.manifest.generation != after.generation {
        return Err("reboot recovered the wrong system-disk generation".into())
    }
    let recovered_fs = SynFs::<SYNFS_SYSTEM_BLOCKS>::recover(&recovered.system_volume)
        .map_err(|error| format!("could not recover SynFS after reboot: {error:?}"))?;
    let package = read_synfs_file(&recovered_fs, "/var/acceptance/hello-world.synpkg")?;
    let provenance = read_synfs_file(&recovered_fs, "/var/acceptance/hello-world.provenance")?;
    let audit_bytes = read_synfs_file(&recovered_fs, "/var/acceptance/hello-world.build")?;
    let source = read_synfs_file(
        &recovered_fs,
        "/var/acceptance/source/src/main.rs",
    )?;
    if source != guest.source_bytes
        || package != guest.package_bytes
        || provenance != guest.provenance
        || !String::from_utf8_lossy(&audit_bytes).contains(&id_string(guest.audit_id))
    {
        return Err("guest source, package, or audit changed across reboot".into())
    }
    let key = SigningKey::new([0x31; 32]);
    PackageBundle::decode(&package)
        .and_then(|bundle| bundle.verify(key))
        .map_err(|error| format!("recovered guest package failed verification: {error:?}"))?;
    ProvenanceChain::decode(&provenance)
        .and_then(|chain| chain.verify(key).map(|_| chain))
        .map_err(|error| format!("recovered guest provenance failed verification: {error:?}"))?;
    if !rebooted || !initial_boot.serial.contains("SynOS kernel bootstrap") {
        return Err("guest acceptance did not boot before and after persistence".into())
    }
    Ok(PersistenceEvidence {
        disk,
        generation_before: before.generation,
        generation_after: after.generation,
        source: guest.source,
        package: guest.package,
        audit: guest.audit_id,
        provenance: guest.provenance_id,
        recovered: true,
        rebooted,
    })
}

fn read_synfs_file<const BLOCKS: usize>(
    filesystem: &SynFs<BLOCKS>,
    path: &str,
) -> Result<Vec<u8>, String> {
    let file = filesystem
        .lookup(path)
        .map_err(|error| format!("could not find persisted file {path}: {error:?}"))?;
    let mut bytes = vec![0; file.size as usize];
    let read = filesystem
        .read(path, &mut bytes)
        .map_err(|error| format!("could not read persisted file {path}: {error:?}"))?;
    bytes.truncate(read.bytes_read);
    Ok(bytes)
}

fn render_audit(record: &BuildAuditRecord, id: ContentId) -> String {
    format!(
        "identity={}\njob={}\nsource={:?}\ndependencies={:?}\ntoolchain={:?}\npackage={:?}\npayload={:?}\ntarget=x86_64-unknown-synos\nrecord={:?}\n",
        record.identity,
        record.job,
        record.source,
        record.dependencies,
        record.toolchain,
        record.package,
        record.payload,
        id,
    )
}

fn render_artifact(
    boot: &BootEvidence,
    kernel: &Path,
    _rustd: &Path,
    package: PackageEvidence,
    native_std: NativeStdEvidence,
    loader: LoadedProcess,
    dynamic: DynamicEvidence,
    service: ServiceEvidence,
    guest: GuestStdEvidence,
    tools: GuestToolEvidence,
    persistence: PersistenceEvidence,
) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "{{");
    let _ = writeln!(output, "  \"schema\": 1,");
    let _ = writeln!(output, "  \"target\": \"x86_64-unknown-synos\",");
    let _ = writeln!(output, "  \"boot\": {{\"kernel\": {}, \"steps\": {}, \"rip\": {}, \"serial_marker\": true}},", json_string(&path_id(kernel)), boot.steps, boot.rip);
    let _ = writeln!(output, "  \"native_std\": {{\"runtime_source\": {}, \"linked_image\": {}, \"panic\": \"abort\", \"dynamic_loading\": \"static-only\", \"verified\": {}}},", json_string(&id_string(native_std.runtime_source)), json_string(&id_string(native_std.image)), native_std.panic_abort && native_std.dynamic_loading == DynamicLoadingPolicy::StaticOnly);
    let _ = writeln!(output, "  \"executable_loader\": {{\"payload\": {}, \"executable_pages\": {}, \"page_count\": {}, \"entry\": {}, \"mapped\": {}, \"stack_non_executable\": {}}},", json_string(&id_string(loader.layout.payload_measurement)), json_string(&id_string(loader.executable_pages)), loader.page_count, loader.context.entry, loader.mapped, loader.stack_non_executable);
    let _ = writeln!(output, "  \"dynamic_artifacts\": {{\"package\": {}, \"payload\": {}, \"policy\": \"static-only\", \"released\": {}, \"verified\": {}}},", json_string(&id_string(dynamic.package)), json_string(&id_string(dynamic.payload)), dynamic.released, dynamic.policy == DynamicLoadingPolicy::StaticOnly);
    let _ = writeln!(output, "  \"synos-rustd\": {{\"package\": {}, \"payload\": {}, \"process\": {}, \"generation\": {}, \"image_id\": {}, \"capability_profile\": {}, \"running\": {}}},", json_string(&id_string(package.package)), json_string(&id_string(package.payload)), service.process.raw(), service.generation, service.image_id, service.capability_profile, service.state_running);
    let _ = writeln!(output, "  \"guest_std_application\": {{\"source\": {}, \"lockfile\": {}, \"package\": {}, \"payload\": {}, \"image\": {}, \"executable_pages\": {}, \"page_count\": {}, \"process_image_loaded\": {}, \"process\": {}, \"generation\": {}, \"running\": {}, \"output\": {}, \"audit\": {}, \"provenance\": {}}},", json_string(&id_string(guest.source)), json_string(&id_string(guest.lockfile)), json_string(&id_string(guest.package)), json_string(&id_string(guest.payload)), json_string(&id_string(ContentId::hash(&guest.image))), json_string(&id_string(guest.loader.executable_pages)), guest.loader.page_count, guest.loader.mapped, guest.process.raw(), guest.generation, guest.running, json_string(&id_string(guest.output)), json_string(&id_string(guest.audit_id)), json_string(&id_string(guest.provenance_id)));
    let _ = writeln!(output, "  \"guest_tool_processes\": {{\"source\": {}, \"image\": {}, \"completed_steps\": {}, \"grant_bits\": {}, \"policy_authorized\": {}, \"build_script\": {}, \"proc_macro\": {}}},", json_string(&id_string(tools.source)), json_string(&id_string(tools.image)), tools.completed_steps, tools.grant_bits, tools.policy_authorized, json_tool_process(&tools.build_script), json_tool_process(&tools.proc_macro));
    let _ = writeln!(output, "  \"reboot_persistence\": {{\"disk\": {}, \"generation_before\": {}, \"generation_after\": {}, \"source\": {}, \"package\": {}, \"audit\": {}, \"provenance\": {}, \"recovered\": {}, \"rebooted\": {}}}", json_string(&path_id(&persistence.disk)), persistence.generation_before, persistence.generation_after, json_string(&id_string(persistence.source)), json_string(&id_string(persistence.package)), json_string(&id_string(persistence.audit)), json_string(&id_string(persistence.provenance)), persistence.recovered, persistence.rebooted);
    let _ = writeln!(output, "}}");
    output
}

fn content_as_u128(id: ContentId) -> u128 {
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&id.as_bytes()[..16]);
    u128::from_be_bytes(bytes).max(1)
}

fn id_string(id: ContentId) -> String {
    format!("{id:?}")
}

fn path_id(path: &Path) -> String {
    format!("path:{}", path.display())
}

fn json_tool_process(process: &ToolProcessEvidence) -> String {
    format!(
        "{{\"process\":{},\"kind\":{},\"workspace\":{},\"scratch\":{},\"filesystem\":{},\"network\":{},\"device\":{},\"secrets\":{},\"process_control\":{},\"isolated\":{},\"exited\":{}}}",
        process.process.raw(),
        json_string(tool_kind_name(process.kind)),
        json_string(&id_string(process.workspace)),
        json_string(&id_string(process.scratch)),
        process.filesystem,
        process.network,
        process.device,
        process.secrets,
        process.process_control,
        process.isolated,
        process.exited,
    )
}

fn tool_kind_name(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Cargo => "cargo",
        ToolKind::Rustc => "rustc",
        ToolKind::Rustdoc => "rustdoc",
        ToolKind::Linker => "linker",
        ToolKind::BuildScript => "build-script",
        ToolKind::ProcMacro => "proc-macro",
    }
}

fn json_string(value: &str) -> String {
    let mut escaped = String::from("\"");
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}
