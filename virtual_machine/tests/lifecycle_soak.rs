//! Long-running lifecycle campaign with explicit ownership accounting.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use ghostos_init::{
    CrashReason, ExitReason, ProcessId, RestartPolicy, ServiceId, ServiceKind,
    ServiceName, ServiceSpec, SpawnRequest, Supervisor, SupervisorEvent,
    SupervisorRuntime,
};
use ghostos_vm::{FirmwareMode, PowerNotification, Vm, VmConfig, PAGE_SIZE};

const RESOURCE_NAMES: [&str; 6] = [
    "pages",
    "handles",
    "irq_routes",
    "timers",
    "capabilities",
    "worker_tasks",
];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum ResourceKind {
    Pages,
    Handles,
    IrqRoutes,
    Timers,
    Capabilities,
    WorkerTasks,
}

impl ResourceKind {
    const fn index(self) -> usize {
        match self {
            Self::Pages => 0,
            Self::Handles => 1,
            Self::IrqRoutes => 2,
            Self::Timers => 3,
            Self::Capabilities => 4,
            Self::WorkerTasks => 5,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Counts {
    values: [u64; 6],
}

impl Counts {
    fn json(self) -> String {
        let mut output = String::from("{");
        for (index, name) in RESOURCE_NAMES.iter().enumerate() {
            if index != 0 {
                output.push(',');
            }
            output.push('"');
            output.push_str(name);
            output.push_str("\":");
            output.push_str(&self.values[index].to_string());
        }
        output.push('}');
        output
    }
}

#[derive(Debug)]
struct OwnershipLedger {
    next_owner: u64,
    active: BTreeMap<(ResourceKind, u64), ()>,
    outstanding: Counts,
    peak: Counts,
}

impl OwnershipLedger {
    fn new() -> Self {
        Self {
            next_owner: 1,
            active: BTreeMap::new(),
            outstanding: Counts::default(),
            peak: Counts::default(),
        }
    }

    fn claim(&mut self, kind: ResourceKind) -> u64 {
        let owner = self.next_owner;
        self.next_owner += 1;
        assert!(self.active.insert((kind, owner), ()).is_none());
        let index = kind.index();
        self.outstanding.values[index] += 1;
        self.peak.values[index] = self.peak.values[index].max(self.outstanding.values[index]);
        owner
    }

    fn release(&mut self, kind: ResourceKind, owner: u64) -> Result<(), String> {
        if self.active.remove(&(kind, owner)).is_none() {
            return Err(format!("released unknown {:?} owner {}", kind, owner));
        }
        self.outstanding.values[kind.index()] -= 1;
        Ok(())
    }

    fn is_clean(&self) -> bool {
        self.active.is_empty() && self.outstanding.values.iter().all(|value| *value == 0)
    }
}

struct Runtime<'a> {
    next_process: u64,
    ledger: &'a mut OwnershipLedger,
    capabilities: BTreeMap<u64, u64>,
    workers: BTreeMap<u64, u64>,
}

impl<'a> Runtime<'a> {
    fn new(ledger: &'a mut OwnershipLedger) -> Self {
        Self {
            next_process: 1,
            ledger,
            capabilities: BTreeMap::new(),
            workers: BTreeMap::new(),
        }
    }
}

impl SupervisorRuntime for Runtime<'_> {
    type Error = String;

    fn spawn(&mut self, _request: SpawnRequest) -> Result<ProcessId, Self::Error> {
        let process = ProcessId::new(self.next_process).ok_or("invalid process ID")?;
        self.next_process += 1;
        let capability = self.ledger.claim(ResourceKind::Capabilities);
        let worker = self.ledger.claim(ResourceKind::WorkerTasks);
        self.capabilities.insert(process.raw(), capability);
        self.workers.insert(process.raw(), worker);
        Ok(process)
    }

    fn fence_process(&mut self, process: ProcessId) -> Result<(), Self::Error> {
        let capability = self
            .capabilities
            .remove(&process.raw())
            .ok_or_else(|| format!("missing capability for process {}", process.raw()))?;
        let worker = self
            .workers
            .remove(&process.raw())
            .ok_or_else(|| format!("missing worker for process {}", process.raw()))?;
        self.ledger
            .release(ResourceKind::Capabilities, capability)?;
        self.ledger.release(ResourceKind::WorkerTasks, worker)
    }
}

fn started_process(event: SupervisorEvent) -> Result<ProcessId, String> {
    match event {
        SupervisorEvent::Started { process, .. } => Ok(process),
        other => Err(format!("expected started event, got {:?}", other)),
    }
}

fn service_restart_campaign(ledger: &mut OwnershipLedger) -> Result<(), String> {
    let service = ServiceId::new(1).ok_or("invalid service ID")?;
    let mut supervisor = Supervisor::<1>::new();
    supervisor
        .register(ServiceSpec {
            id: service,
            name: ServiceName::new("lifecycle-worker").map_err(|error| format!("{error:?}"))?,
            kind: ServiceKind::System,
            image_id: 1,
            capability_profile: 1,
            restart: RestartPolicy::on_failure(2, 100, 10, 20)
                .map_err(|error| format!("{error:?}"))?,
        })
        .map_err(|error| format!("{error:?}"))?;

    let mut runtime = Runtime::new(ledger);
    let first = started_process(
        supervisor
            .start(service, &mut runtime)
            .map_err(|error| format!("{error:?}"))?,
    )?;
    supervisor
        .report_exit(
            first,
            ExitReason::Crash(CrashReason::Watchdog),
            0,
            &mut runtime,
        )
        .map_err(|error| format!("{error:?}"))?;
    let restarted = started_process(
        supervisor
            .tick(10, &mut runtime)
            .map_err(|error| format!("{error:?}"))?
            .ok_or("service restart did not become due")?,
    )?;
    if restarted == first {
        return Err("service reused a fenced process ID".into());
    }
    supervisor
        .shutdown(&mut runtime)
        .map_err(|error| format!("{error:?}"))?;
    Ok(())
}

fn vm_lifecycle_campaign(cycle: usize, ledger: &mut OwnershipLedger) -> Result<(), String> {
    let mut vm = Vm::with_config(VmConfig {
        memory_size: 4 * 1024 * 1024,
        max_memory_size: 8 * 1024 * 1024,
        firmware: FirmwareMode::Bios,
        ..VmConfig::default()
    });
    let marker = (cycle as u8).wrapping_add(1);
    vm.mmu_mut()
        .write_byte(0x2000, marker)
        .map_err(|error| format!("write cycle marker: {error:?}"))?;

    let snapshot_pages = ledger.claim(ResourceKind::Pages);
    let snapshot_handle = ledger.claim(ResourceKind::Handles);
    let snapshot = vm.snapshot();
    vm.mmu_mut()
        .write_byte(0x2000, marker.wrapping_add(1))
        .map_err(|error| format!("mutate suspended VM: {error:?}"))?;
    vm.restore_snapshot(&snapshot)
        .map_err(|error| format!("resume VM: {error:?}"))?;
    if vm.mmu().read_byte(0x2000).map_err(|error| format!("read resumed VM: {error:?}"))? != marker {
        return Err("suspend/resume restored the wrong page contents".into());
    }
    ledger.release(ResourceKind::Handles, snapshot_handle)?;
    ledger.release(ResourceKind::Pages, snapshot_pages)?;

    let timer = ledger.claim(ResourceKind::Timers);
    let reboot_route = ledger.claim(ResourceKind::IrqRoutes);
    vm.request_reboot();
    if !vm
        .take_power_notifications()
        .iter()
        .any(|notification| matches!(notification, PowerNotification::Reboot))
    {
        return Err("reboot notification was not routed".into());
    }
    vm.reset();
    ledger.release(ResourceKind::IrqRoutes, reboot_route)?;
    ledger.release(ResourceKind::Timers, timer)?;

    let hotplug_page = ledger.claim(ResourceKind::Pages);
    let before = vm.mmu().ram_size();
    vm.hotplug_memory(PAGE_SIZE)
        .map_err(|error| format!("hotplug memory: {error:?}"))?;
    if vm.mmu().ram_size() != before + PAGE_SIZE {
        return Err("hotplug did not add exactly one page".into());
    }
    let hotplug_route = ledger.claim(ResourceKind::IrqRoutes);
    let hotplug = vm.memory_hotplug();
    if hotplug.borrow().current_memory() != vm.mmu().ram_size() as u64 {
        return Err("hotplug device and MMU disagree on memory ownership".into());
    }
    hotplug.borrow().clear_pending();
    ledger.release(ResourceKind::IrqRoutes, hotplug_route)?;
    drop(hotplug);
    drop(vm);
    ledger.release(ResourceKind::Pages, hotplug_page)
}

fn run_cycle(cycle: usize, ledger: &mut OwnershipLedger) -> Result<(), String> {
    service_restart_campaign(ledger)?;
    vm_lifecycle_campaign(cycle, ledger)
}

fn report_path() -> PathBuf {
    std::env::var_os("GHOSTOS_LIFECYCLE_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("build/soak/lifecycle/report.json"))
}

fn write_report(
    path: &PathBuf,
    requested: usize,
    completed: usize,
    failures: &[String],
    cycles: &[(usize, Counts, Counts)],
) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create lifecycle report directory");
    }
    let state = if failures.is_empty() && completed == requested {
        "passed"
    } else {
        "failed"
    };
    let mut output = String::new();
    output.push_str("{\n  \"schema\": 1,\n  \"campaign\": \"lifecycle\",\n");
    output.push_str(&format!(
        "  \"cycles_requested\": {},\n  \"cycles_completed\": {},\n  \"result_state\": \"{}\",\n  \"zero_unreclaimed_ownership\": {},\n",
        requested,
        completed,
        state,
        if state == "passed" { "true" } else { "false" }
    ));
    output.push_str("  \"resource_classes\": [\"pages\", \"handles\", \"irq_routes\", \"timers\", \"capabilities\", \"worker_tasks\"],\n  \"cycles\": [");
    for (index, (cycle, outstanding, peak)) in cycles.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&format!(
            "\n    {{\"cycle\": {}, \"outstanding\": {}, \"peak\": {}}}",
            cycle,
            outstanding.json(),
            peak.json()
        ));
    }
    output.push_str("\n  ],\n  \"failures\": [");
    for (index, failure) in failures.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&format!("\"{}\"", failure.replace('"', "\\\"")));
    }
    output.push_str("],\n  \"reason\": \"");
    output.push_str(if state == "passed" {
        "all lifecycle cycles reclaimed every tracked ownership class"
    } else {
        "one or more lifecycle cycles left ownership or failed"
    });
    output.push_str("\"\n}\n");
    fs::write(path, output).expect("write lifecycle report");
}

#[test]
fn lifecycle_campaign_reclaims_all_ownership() {
    let cycles = std::env::var("GHOSTOS_LIFECYCLE_CYCLES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(64);
    let path = report_path();
    let mut completed = 0;
    let mut failures = Vec::new();
    let mut evidence = Vec::new();

    for cycle in 1..=cycles {
        let mut ledger = OwnershipLedger::new();
        let result = run_cycle(cycle, &mut ledger);
        if result.is_ok() && !ledger.is_clean() {
            failures.push(format!("cycle {} left ownership: {:?}", cycle, ledger.outstanding));
        }
        if let Err(error) = result {
            failures.push(format!("cycle {} failed: {}", cycle, error));
        }
        evidence.push((cycle, ledger.outstanding, ledger.peak));
        if failures.is_empty() {
            completed += 1;
        } else {
            break;
        }
    }

    write_report(&path, cycles, completed, &failures, &evidence);
    assert!(
        failures.is_empty(),
        "lifecycle campaign failed; retained report: {}",
        path.display()
    );
}
