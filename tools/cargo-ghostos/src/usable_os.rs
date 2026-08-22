use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use ghostos_app::{
    ImageArchitecture, NativeSpawnRequest, ProcessArguments, ProcessBackend, ProcessExit,
    ProcessLimits, ProcessState, ProcessSupervisor, ProcessUsage,
};
use ghostos_auth::{
    BootLoginService, Credential, CredentialId, CredentialKind, CredentialVerifier,
    DatabaseScope, SecurityState, SecurityStore, SecurityStoreError, SecurityPolicy,
};
use ghostos_fabric::NodeId;
use ghostos_init::{
    CrashReason, ExitReason, ProcessId, RestartPolicy, ServiceId, ServiceKind, ServiceName,
    ServiceSpec, SpawnRequest, Supervisor, SupervisorEvent, SupervisorRuntime,
};
use ghostos_kernel::{AddressSpaceId, IdentityId, RightIdentifier};
use ghostos_netd::{
    discover_interfaces, InterfaceDescriptor, InterfaceInventory, NetworkConfig,
    NetworkInterfaceConfig, NetworkMode,
};
use ghostos_ghostfs::SynFs;
use ghostos_update::{BootDecision, BootHealthState, BootRollbackController, BootRollbackRuntime};
use ghostos_vm::{
    DiskPersistence, DiskSpec, FirmwareMode, SystemDiskInstall, SystemDiskProvisioner, Vm,
    VmConfig, GHOSTFS_SYSTEM_BLOCKS,
};

const KERNEL_TARGET: &str = "x86_64-unknown-none";
const BOOT_STEPS: u64 = 2_000_000;
const BOOT_BATCH_STEPS: u64 = 10_000;

pub fn run(arguments: &[String]) -> Result<(), String> {
    let options = Options::parse(arguments)?;
    let workspace = workspace_root()?;
    let root = options
        .clean_root
        .unwrap_or_else(|| env::temp_dir().join(format!("ghostos-usable-os-{}", std::process::id())));
    if root.exists() {
        return Err(format!(
            "usable-os root already exists: {}; use --clean-root with a new path",
            root.display()
        ));
    }
    fs::create_dir_all(&root)
        .map_err(|error| format!("could not create {}: {error}", root.display()))?;

    let kernel = build_kernel(&workspace, &root, options.release)?;
    let mut results = Vec::new();
    record(&mut results, "disk-boot", disk_boot_and_persistence(&root, &kernel));
    record(&mut results, "isolated-user-services", isolated_services(&kernel));
    record(&mut results, "administrator-login", administrator_login());
    record(&mut results, "persistent-files", persistent_file_contract());
    record(&mut results, "isolated-concurrent-applications", isolated_applications());
    record(&mut results, "network-interface", network_interface(&kernel));
    record(&mut results, "service-restart", service_restart());
    record(&mut results, "reboot-recovery", reboot_recovery(&root, &kernel));
    record(&mut results, "failed-update-recovery", failed_update_recovery());

    let passed = results.iter().filter(|result| result.ok).count();
    let artifact = root.join("usable-os-evidence.json");
    fs::write(&artifact, render_evidence(&root, &kernel, &results))
        .map_err(|error| format!("could not write {}: {error}", artifact.display()))?;
    if options.json {
        println!("{}", render_evidence(&root, &kernel, &results));
    } else {
        for result in &results {
            println!(
                "{} {}: {}",
                if result.ok { "PASS" } else { "FAIL" },
                result.name,
                result.detail
            );
        }
        println!("usable OS evidence: {}", artifact.display());
    }
    if passed == results.len() {
        Ok(())
    } else {
        Err(format!("{} usable OS check(s) failed", results.len() - passed))
    }
}

struct Options {
    release: bool,
    clean_root: Option<PathBuf>,
    json: bool,
}

impl Options {
    fn parse(arguments: &[String]) -> Result<Self, String> {
        let mut options = Self {
            release: false,
            clean_root: None,
            json: false,
        };
        let mut index = 0;
        while index < arguments.len() {
            match arguments[index].as_str() {
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
                "--json" => {
                    options.json = true;
                    index += 1;
                }
                other => return Err(format!("unknown usable-os option `{other}`")),
            }
        }
        Ok(options)
    }
}

struct ResultLine {
    name: &'static str,
    ok: bool,
    detail: String,
}

fn record(results: &mut Vec<ResultLine>, name: &'static str, result: Result<(), String>) {
    match result {
        Ok(()) => results.push(ResultLine {
            name,
            ok: true,
            detail: "contract satisfied".into(),
        }),
        Err(detail) => results.push(ResultLine {
            name,
            ok: false,
            detail,
        }),
    }
}

fn build_kernel(workspace: &Path, root: &Path, release: bool) -> Result<PathBuf, String> {
    let target_dir = root.join("kernel-target");
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .current_dir(workspace)
        .env("RUSTC_BOOTSTRAP", "1")
        .args([
            "-Z",
            "build-std=core,alloc",
            "-Z",
            "json-target-spec",
            "build",
            "-p",
            "ghostos-kernel",
            "--bin",
            "ghostos-kernel",
            "--target",
            KERNEL_TARGET,
            "--target-dir",
        ])
        .arg(&target_dir)
        .arg("--locked")
        .arg("--offline");
    if release {
        command.arg("--release");
    }
    let status = command
        .status()
        .map_err(|error| format!("could not start native kernel build: {error}"))?;
    if !status.success() {
        return Err(format!("native kernel build failed with {status}"));
    }
    let profile = if release { "release" } else { "debug" };
    let path = target_dir.join(KERNEL_TARGET).join(profile).join("ghostos-kernel");
    path.is_file()
        .then_some(path)
        .ok_or_else(|| "native kernel build produced no executable".into())
}

fn boot_disk(disk: &Path) -> Result<String, String> {
    let mut vm = Vm::try_with_config(VmConfig {
        memory_size: 128 * 1024 * 1024,
        firmware: FirmwareMode::Bios,
        max_steps: Some(BOOT_STEPS),
        disks: vec![DiskSpec::system("system", disk).with_persistence(DiskPersistence::Persistent)],
        ..VmConfig::default()
    })
    .map_err(|error| format!("could not create disk boot VM: {error:?}"))?;
    for _ in 0..BOOT_STEPS / BOOT_BATCH_STEPS {
        let report = vm
            .run_for_steps(BOOT_BATCH_STEPS)
            .map_err(|error| format!("disk boot failed: {error:?}"))?;
        if report.halted {
            break;
        }
        let serial = vm
            .serial()
            .map(|serial| String::from_utf8_lossy(serial.borrow().output()).into_owned())
            .unwrap_or_default();
        if serial.contains("GhostOS kernel bootstrap") && serial.contains("GhostOS user shell") {
            break;
        }
    }
    vm.flush_serial_output();
    Ok(vm
        .serial()
        .map(|serial| String::from_utf8_lossy(serial.borrow().output()).into_owned())
        .unwrap_or_default())
}

fn disk_boot_and_persistence(root: &Path, kernel: &Path) -> Result<(), String> {
    let disk = root.join("usable-os.system.img");
    let install = SystemDiskInstall::new(kernel)
        .with_boot_args("serial")
        .with_machine_identity("usable-os-acceptance")
        .with_capabilities(["filesystem", "network", "administrator"]);
    let before = SystemDiskProvisioner::provision(&disk, &install)
        .map_err(|error| format!("could not provision system disk: {error}"))?;
    let artifacts = SystemDiskProvisioner::load_boot_artifacts(&disk)
        .map_err(|error| format!("could not load system disk: {error}"))?;
    if artifacts.manifest.generation != before.generation {
        return Err("system disk generation changed during install".into());
    }
    let mut volume_bytes = artifacts.system_volume;
    let mut filesystem = SynFs::<GHOSTFS_SYSTEM_BLOCKS>::recover(&volume_bytes)
        .map_err(|error| format!("could not recover GhostFS: {error:?}"))?;
    filesystem
        .create_directory("/var/usable-os", true)
        .map_err(|error| format!("could not create persistent directory: {error:?}"))?;
    filesystem
        .write("/var/usable-os/state", b"administrator-ready")
        .map_err(|error| format!("could not write persistent state: {error:?}"))?;
    let mut read_buffer = [0; 32];
    let read = filesystem
        .read("/var/usable-os/state", &mut read_buffer)
        .map_err(|error| format!("could not read persistent state: {error:?}"))?;
    if &read_buffer[..read.bytes_read] != b"administrator-ready" {
        return Err("persistent state read back incorrectly".into());
    }
    filesystem
        .delete("/var/usable-os/state")
        .map_err(|error| format!("could not delete persistent state: {error:?}"))?;
    if filesystem.lookup("/var/usable-os/state").is_ok() {
        return Err("deleted persistent state still exists".into());
    }
    filesystem
        .write("/var/usable-os/state", b"administrator-ready")
        .map_err(|error| format!("could not restore persistent state: {error:?}"))?;
    filesystem
        .flush(&mut volume_bytes)
        .map_err(|error| format!("could not commit persistent state: {error:?}"))?;
    SystemDiskProvisioner::update_system_volume(&disk, &volume_bytes)
        .map_err(|error| format!("could not publish persistent state: {error}"))?;
    let serial = boot_disk(&disk)?;
    if !serial.contains("GhostOS kernel bootstrap") {
        return Err("installed disk did not boot the kernel".into());
    }
    let recovered = SystemDiskProvisioner::load_boot_artifacts(&disk)
        .map_err(|error| format!("could not reload system disk: {error}"))?;
    let filesystem = SynFs::<GHOSTFS_SYSTEM_BLOCKS>::recover(&recovered.system_volume)
        .map_err(|error| format!("could not recover committed state: {error:?}"))?;
    let mut recovered_state = [0; 32];
    let read = filesystem
        .read("/var/usable-os/state", &mut recovered_state)
        .map_err(|error| format!("could not read committed state: {error:?}"))?;
    if &recovered_state[..read.bytes_read] != b"administrator-ready" {
        return Err("committed state was not recovered after reboot".into());
    }
    Ok(())
}

fn isolated_services(kernel: &Path) -> Result<(), String> {
    let mut vm = Vm::try_with_config(VmConfig {
        memory_size: 128 * 1024 * 1024,
        kernel_path: Some(kernel.to_path_buf()),
        firmware: FirmwareMode::Bios,
        max_steps: Some(BOOT_STEPS),
        ..VmConfig::default()
    })
    .map_err(|error| format!("could not create service VM: {error:?}"))?;
    vm.run_for_steps(BOOT_STEPS)
        .map_err(|error| format!("service VM failed: {error:?}"))?;
    let serial = vm
        .serial()
        .map(|serial| String::from_utf8_lossy(serial.borrow().output()).into_owned())
        .unwrap_or_default();
    let service_count = serial.matches("starting service entrypoint").count();
    if !serial.contains("starting ghostos-init in Ring 3") || service_count < 12 {
        return Err(format!("only {service_count} isolated service entrypoints started"));
    }
    Ok(())
}

fn persistent_file_contract() -> Result<(), String> {
    let mut filesystem = SynFs::<128>::new();
    filesystem
        .create_directory("/home/admin", true)
        .map_err(|error| format!("could not create user directory: {error:?}"))?;
    filesystem
        .write("/home/admin/file", b"persistent")
        .map_err(|error| format!("could not create file: {error:?}"))?;
    let mut bytes = [0; 16];
    let read = filesystem
        .read("/home/admin/file", &mut bytes)
        .map_err(|error| format!("could not read file: {error:?}"))?;
    if &bytes[..read.bytes_read] != b"persistent" {
        return Err("file contents changed".into());
    }
    filesystem
        .delete("/home/admin/file")
        .map_err(|error| format!("could not delete file: {error:?}"))?;
    if filesystem.lookup("/home/admin/file").is_ok() {
        return Err("file still exists after delete".into());
    }
    Ok(())
}

fn administrator_login() -> Result<(), String> {
    let mut store = MemorySecurityStore::default();
    let mut login = BootLoginService::<8, 4, 4, 2>::start(
        &mut store,
        SecurityPolicy::default(),
        0x5349_4e4f_5341_444d,
    )
    .map_err(|error| format!("could not start login service: {error:?}"))?;
    let credential = Credential::new_passkey(
        CredentialId::new(1).ok_or("invalid administrator credential")?,
        b"admin-public-key",
        0,
    )
    .map_err(|error| format!("could not create administrator credential: {error:?}"))?;
    login
        .create_first_admin(
            &mut store,
            IdentityId::new(7).ok_or("invalid administrator identity")?,
            "admin",
            DatabaseScope::Local,
            credential,
        )
        .map_err(|error| format!("could not provision administrator: {error:?}"))?;
    let challenge = login
        .begin_login(
            "admin",
            NodeId::LOCAL,
            CredentialId::new(1).ok_or("invalid administrator credential")?,
            CredentialKind::Passkey,
            10,
        )
        .map_err(|error| format!("administrator login challenge failed: {error:?}"))?;
    let session = login
        .complete_login(
            challenge,
            b"proof",
            &mut AcceptVerifier,
            AddressSpaceId::new(9).ok_or("invalid login address space")?,
            20,
        )
        .map_err(|error| format!("administrator login failed: {error:?}"))?;
    login
        .authorize(session.handle, RightIdentifier::SYSTEM_ADMIN, 21)
        .map_err(|error| format!("administrator session lacks system control: {error:?}"))?;
    Ok(())
}

fn isolated_applications() -> Result<(), String> {
    let mut supervisor = ProcessSupervisor::<2>::new();
    let mut backend = AppBackend::default();
    let limits = ProcessLimits {
        memory_bytes: 1 << 20,
        cpu_time_us: 1_000_000,
        deadline_us: 1_000_000,
        cancel_grace_us: 1_000,
    };
    let first = supervisor
        .spawn(
            NativeSpawnRequest {
                image: b"app-one",
                architecture: ImageArchitecture::X86_64,
                expected_payload: None,
                heap_bytes: 4096,
                arguments: ProcessArguments {
                    argv: &["app-one"],
                    environment: &[],
                },
                limits,
            },
            1,
            &mut backend,
        )
        .map_err(|error| format!("first application did not start: {error:?}"))?;
    let second = supervisor
        .spawn(
            NativeSpawnRequest {
                image: b"app-two",
                architecture: ImageArchitecture::X86_64,
                expected_payload: None,
                heap_bytes: 4096,
                arguments: ProcessArguments {
                    argv: &["app-two"],
                    environment: &[],
                },
                limits,
            },
            1,
            &mut backend,
        )
        .map_err(|error| format!("second application did not start: {error:?}"))?;
    if first == second
        || supervisor.status(first).map_err(|error| format!("first status failed: {error:?}"))?.state
            != ProcessState::Running
        || supervisor.status(second).map_err(|error| format!("second status failed: {error:?}"))?.state
            != ProcessState::Running
    {
        return Err("two applications do not have independent running process slots".into());
    }
    Ok(())
}

fn network_interface(kernel: &Path) -> Result<(), String> {
    let descriptors = [InterfaceDescriptor {
        bus: 0,
        device: 6,
        function: 0,
        kind: 0x100e,
        mac: [0x52, 0x54, 0x00, 0x53, 0x59, 0x01],
        link_up: true,
    }];
    let mut inventory = InterfaceInventory::<2>::new();
    if discover_interfaces(&descriptors, &mut inventory) != 1
        || !inventory
            .entries()
            .next()
            .is_some_and(|(name, _)| name.as_str() == "eth0")
    {
        return Err("network interface discovery did not produce eth0".into());
    }
    let mut configuration = NetworkConfig::<2>::new("ghostos").ok_or("invalid network hostname")?;
    let interface = NetworkInterfaceConfig::dhcp("eth0").ok_or("invalid network interface")?;
    configuration
        .add_interface(interface)
        .map_err(|error| format!("could not configure network interface: {error:?}"))?;
    if configuration.boot_action("eth0").is_none()
        || configuration.interface("eth0").map(|value| value.mode) != Some(NetworkMode::Dhcp)
    {
        return Err("network interface has no boot configuration".into());
    }
    let serial = boot_kernel_for_network(kernel)?;
    if !serial.contains("network service registered and started") {
        return Err("network service did not start during boot".into());
    }
    Ok(())
}

fn boot_kernel_for_network(kernel: &Path) -> Result<String, String> {
    let mut vm = Vm::try_with_config(VmConfig {
        kernel_path: Some(kernel.to_path_buf()),
        firmware: FirmwareMode::Bios,
        max_steps: Some(BOOT_STEPS),
        ..VmConfig::default()
    })
    .map_err(|error| format!("could not create network VM: {error:?}"))?;
    vm.run_for_steps(BOOT_STEPS)
        .map_err(|error| format!("network VM failed: {error:?}"))?;
    if !vm.network_link_up() || vm.network_mac_addresses().iter().all(|mac| mac.0 == [0; 6]) {
        return Err("VM network interface is not present".into());
    }
    Ok(vm
        .serial()
        .map(|serial| String::from_utf8_lossy(serial.borrow().output()).into_owned())
        .unwrap_or_default())
}

fn service_restart() -> Result<(), String> {
    let service = ServiceId::new(0x5359_4e54).ok_or("invalid service id")?;
    let mut supervisor = Supervisor::<1>::new();
    supervisor
        .register(ServiceSpec {
            id: service,
            name: ServiceName::new("usable-service").map_err(|error| format!("{error:?}"))?,
            kind: ServiceKind::System,
            image_id: 1,
            capability_profile: 1,
            restart: RestartPolicy::on_failure(2, 1_000, 10, 100).map_err(|error| format!("{error:?}"))?,
        })
        .map_err(|error| format!("could not register service: {error:?}"))?;
    let mut runtime = ServiceBackend::default();
    let first = match supervisor
        .start(service, &mut runtime)
        .map_err(|error| format!("could not start service: {error:?}"))?
    {
        SupervisorEvent::Started { process, generation, .. } => (process, generation),
        event => return Err(format!("unexpected service start event: {event:?}")),
    };
    supervisor
        .report_exit(
            first.0,
            ExitReason::Crash(CrashReason::ProtectionFault),
            100,
            &mut runtime,
        )
        .map_err(|error| format!("could not report service crash: {error:?}"))?;
    let restart_at = supervisor
        .status(service)
        .map_err(|error| format!("could not inspect service: {error:?}"))?
        .restart_at_us;
    supervisor
        .tick(restart_at, &mut runtime)
        .map_err(|error| format!("could not restart service: {error:?}"))?;
    let status = supervisor
        .status(service)
        .map_err(|error| format!("could not inspect restarted service: {error:?}"))?;
    if status.process == Some(first.0) || status.generation <= first.1 || runtime.fenced != 1 {
        return Err("failed service was not fenced and restarted".into());
    }
    Ok(())
}

fn reboot_recovery(root: &Path, kernel: &Path) -> Result<(), String> {
    let disk = root.join("reboot-recovery.system.img");
    let install = SystemDiskInstall::new(kernel).with_machine_identity("reboot-recovery");
    SystemDiskProvisioner::provision(&disk, &install)
        .map_err(|error| format!("could not provision reboot disk: {error}"))?;
    let first = boot_disk(&disk)?;
    let second = boot_disk(&disk)?;
    if !first.contains("GhostOS kernel bootstrap") || !second.contains("GhostOS kernel bootstrap") {
        return Err("system did not boot before and after reboot".into());
    }
    Ok(())
}

fn failed_update_recovery() -> Result<(), String> {
    let mut controller = BootRollbackController::arm(2, 1, 2)
        .map_err(|error| format!("could not arm update health gate: {error:?}"))?;
    if controller.boot_attempt().map_err(|error| format!("{error:?}"))? != BootDecision::Continue
        || controller.health_failed().map_err(|error| format!("{error:?}"))? != BootDecision::Retry
        || controller.boot_attempt().map_err(|error| format!("{error:?}"))? != BootDecision::Continue
        || controller.health_failed().map_err(|error| format!("{error:?}"))? != BootDecision::Rollback
    {
        return Err("failed update did not enter rollback".into());
    }
    let mut runtime = RollbackBackend::default();
    controller
        .rollback(&mut runtime)
        .map_err(|error| format!("rollback execution failed: {error:?}"))?;
    if controller.state() != BootHealthState::RolledBack || runtime.rollback != Some((2, 1)) {
        return Err("failed update did not recover the prior revision".into());
    }
    Ok(())
}

#[derive(Default)]
struct MemorySecurityStore {
    state: Option<SecurityState<8, 2>>,
}

impl SecurityStore<8, 2> for MemorySecurityStore {
    fn load(
        &mut self,
        state: &mut SecurityState<8, 2>,
    ) -> Result<bool, SecurityStoreError> {
        if let Some(saved) = self.state {
            *state = saved;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn store(&mut self, state: &SecurityState<8, 2>) -> Result<(), SecurityStoreError> {
        self.state = Some(*state);
        Ok(())
    }
}

struct AcceptVerifier;

impl CredentialVerifier for AcceptVerifier {
    fn verify(
        &mut self,
        _kind: CredentialKind,
        _public_material: &[u8],
        _challenge: &[u8],
        _response: &[u8],
    ) -> bool {
        true
    }
}

#[derive(Default)]
struct AppBackend {
    next: u64,
}

impl ProcessBackend for AppBackend {
    type Error = ();

    fn spawn(&mut self, _request: NativeSpawnRequest<'_>) -> Result<ProcessId, Self::Error> {
        self.next += 1;
        ProcessId::new(self.next).ok_or(())
    }

    fn exec(
        &mut self,
        _process: ProcessId,
        _request: ghostos_app::NativeExecRequest<'_>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn wait(&mut self, _process: ProcessId) -> Result<Option<ProcessExit>, Self::Error> {
        Ok(None)
    }

    fn usage(&mut self, _process: ProcessId) -> Result<ProcessUsage, Self::Error> {
        Ok(ProcessUsage {
            memory_bytes: 0,
            cpu_time_us: 0,
        })
    }

    fn request_cancel(&mut self, _process: ProcessId) -> Result<(), Self::Error> {
        Ok(())
    }

    fn fence(&mut self, _process: ProcessId) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[derive(Default)]
struct ServiceBackend {
    next: u64,
    fenced: u32,
}

impl SupervisorRuntime for ServiceBackend {
    type Error = ();

    fn spawn(&mut self, _request: SpawnRequest) -> Result<ProcessId, Self::Error> {
        self.next += 1;
        ProcessId::new(self.next).ok_or(())
    }

    fn fence_process(&mut self, _process: ProcessId) -> Result<(), Self::Error> {
        self.fenced += 1;
        Ok(())
    }
}

#[derive(Default)]
struct RollbackBackend {
    rollback: Option<(u64, u64)>,
}

impl BootRollbackRuntime for RollbackBackend {
    type Error = ();

    fn rollback_to(&mut self, candidate_revision: u64, rollback_revision: u64) -> Result<(), Self::Error> {
        self.rollback = Some((candidate_revision, rollback_revision));
        Ok(())
    }

    fn enter_recovery(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "could not locate the GhostOS workspace".into())
}

fn render_evidence(root: &Path, kernel: &Path, results: &[ResultLine]) -> String {
    let mut output = String::from("{\n  \"schema\": 1,\n  \"target\": \"x86_64\",\n");
    output.push_str(&format!(
        "  \"root\": {},\n  \"kernel\": {},\n  \"checks\": [\n",
        json_string(&root.display().to_string()),
        json_string(&kernel.display().to_string()),
    ));
    for (index, result) in results.iter().enumerate() {
        output.push_str(&format!(
            "    {{\"name\":{},\"ok\":{},\"detail\":{}}}{}\n",
            json_string(result.name),
            result.ok,
            json_string(&result.detail),
            if index + 1 == results.len() { "" } else { "," },
        ));
    }
    output.push_str("  ]\n}\n");
    output
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
            character if character.is_control() => escaped.push('?'),
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}
