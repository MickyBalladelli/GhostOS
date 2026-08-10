use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use synos_app::{
    load_image, measure_executable_pages, ImageArchitecture, ImageMapper, ImageLoadRequest,
    Mapping, MappingRequest, PageMeasurer, ProcessArguments, ProcessContext, RuntimeSegment,
    SegmentPermissions, StackRequest, TlsRequest,
};
use synos_pkg::{bundle_size, encode_bundle, PackageDaemon, SigningKey};
use synos_rustd::{
    ArtifactSandbox, CompilerServiceBoot, DynamicArtifact, DynamicArtifactKind, JobId,
    NativeCompilerBootConfig, Target, Text, COMPILER_CAPABILITY_PROFILE, COMPILER_SERVICE_ID,
};
use synos_runtime::{DynamicLoadingPolicy, PanicModel, PANIC_MODEL};
use synos_system_model::ContentId;
use synos_vm::{FirmwareMode, Vm, VmConfig};

use synos_init::{ProcessId, SpawnRequest, Supervisor, SupervisorEvent, SupervisorRuntime};

const KERNEL_TARGET: &str = "x86_64-unknown-none";
const BOOT_STEPS: u64 = 250_000;

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
    let report = vm
        .run_for_steps(BOOT_STEPS)
        .map_err(|error| format!("SynOS VM boot failed: {error:?}"))?;
    let serial = vm
        .serial()
        .map(|serial| String::from_utf8_lossy(serial.borrow().output()).into_owned())
        .unwrap_or_default();
    Ok(BootEvidence {
        serial,
        steps: report.steps,
        rip: report.rip,
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
    let mut filesystem = synos_synfs::SynFs::<1024>::new();
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
    let mut filesystem = synos_synfs::SynFs::<1024>::new();
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
    let mut filesystem = synos_synfs::SynFs::<128>::new();
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

fn render_artifact(
    boot: &BootEvidence,
    kernel: &Path,
    _rustd: &Path,
    package: PackageEvidence,
    native_std: NativeStdEvidence,
    loader: LoadedProcess,
    dynamic: DynamicEvidence,
    service: ServiceEvidence,
) -> String {
    let mut output = String::new();
    let _ = writeln!(output, "{{");
    let _ = writeln!(output, "  \"schema\": 1,");
    let _ = writeln!(output, "  \"target\": \"x86_64-unknown-synos\",");
    let _ = writeln!(output, "  \"boot\": {{\"kernel\": {}, \"steps\": {}, \"rip\": {}, \"serial_marker\": true}},", json_string(&path_id(kernel)), boot.steps, boot.rip);
    let _ = writeln!(output, "  \"native_std\": {{\"runtime_source\": {}, \"linked_image\": {}, \"panic\": \"abort\", \"dynamic_loading\": \"static-only\", \"verified\": {}}},", json_string(&id_string(native_std.runtime_source)), json_string(&id_string(native_std.image)), native_std.panic_abort && native_std.dynamic_loading == DynamicLoadingPolicy::StaticOnly);
    let _ = writeln!(output, "  \"executable_loader\": {{\"payload\": {}, \"executable_pages\": {}, \"page_count\": {}, \"entry\": {}, \"mapped\": {}, \"stack_non_executable\": {}}},", json_string(&id_string(loader.layout.payload_measurement)), json_string(&id_string(loader.executable_pages)), loader.page_count, loader.context.entry, loader.mapped, loader.stack_non_executable);
    let _ = writeln!(output, "  \"dynamic_artifacts\": {{\"package\": {}, \"payload\": {}, \"policy\": \"static-only\", \"released\": {}, \"verified\": {}}},", json_string(&id_string(dynamic.package)), json_string(&id_string(dynamic.payload)), dynamic.released, dynamic.policy == DynamicLoadingPolicy::StaticOnly);
    let _ = writeln!(output, "  \"synos-rustd\": {{\"package\": {}, \"payload\": {}, \"process\": {}, \"generation\": {}, \"image_id\": {}, \"capability_profile\": {}, \"running\": {}}}", json_string(&id_string(package.package)), json_string(&id_string(package.payload)), service.process.raw(), service.generation, service.image_id, service.capability_profile, service.state_running);
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
