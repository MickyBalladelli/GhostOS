use synos_status::Status;
use synos_system_model::command::{
    ArgumentKind, ArgumentSpec, CommandSpec, OutputText, OutputValue,
    StructuredOutput,
};

use crate::{
    Error, Text,
    interpreter::{CommandExecutor, ExecutionToken},
    parser::{CommandCall, CommandRegistry, RouteId, Value},
};

pub const SHOW_MEMORY_ROUTE: u16 = 1;
pub const SHOW_PROCESS_ROUTE: u16 = 2;
pub const MONITOR_ROUTE: u16 = 3;
pub const DEFAULT_DIAGNOSTIC_QUEUE: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemorySnapshot {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub local_bytes: u64,
    pub cxl_bytes: u64,
    pub layer2_bytes: u64,
    pub vram_bytes: u64,
    pub active_leases: u64,
    pub failed_nodes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterSnapshot {
    pub nodes: u64,
    pub healthy_nodes: u64,
    pub degraded_nodes: u64,
    pub heartbeat_period_us: u64,
    pub remote_pages: u64,
    pub migrations: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessState {
    Ready,
    Running,
    Blocked,
    Stopped,
}

impl ProcessState {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "READY",
            Self::Running => "RUNNING",
            Self::Blocked => "BLOCKED",
            Self::Stopped => "STOPPED",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProcessSnapshot {
    pub pid: u64,
    pub name: Text<64>,
    pub state: ProcessState,
    pub threads: u64,
    pub resident_bytes: u64,
    pub cpu_time_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MonitorSnapshot {
    pub sample: u64,
    pub interval_us: u64,
    pub runnable_threads: u64,
    pub cpu_busy_percent: u64,
    pub memory_used_bytes: u64,
    pub remote_faults: u64,
    pub network_bytes: u64,
}

pub trait DiagnosticSource {
    fn memory(&mut self, cluster: bool) -> Result<MemorySnapshot, Status>;
    fn cluster(&mut self) -> Result<ClusterSnapshot, Status>;
    fn process(&mut self, pid: Option<u64>) -> Result<ProcessSnapshot, Status>;
    fn monitor(
        &mut self,
        interval_us: u64,
        samples: u64,
    ) -> Result<MonitorSnapshot, Status>;
}

pub fn register_builtin_commands<const CAPACITY: usize>(
    registry: &mut CommandRegistry<CAPACITY>,
) -> Result<(), Error> {
    let cluster = ArgumentSpec::new("CLUSTER", ArgumentKind::Boolean, false, false)
        .map_err(|_| Error::InvalidValue)?;
    registry.register(
        CommandSpec::new("SHOW-MEMORY", &[cluster])
            .map_err(|_| Error::InvalidValue)?,
        RouteId::new(SHOW_MEMORY_ROUTE).expect("nonzero route"),
    )?;

    let pid = ArgumentSpec::new("PID", ArgumentKind::Integer, false, true)
        .map_err(|_| Error::InvalidValue)?;
    registry.register(
        CommandSpec::new("SHOW-PROCESS", &[pid])
            .map_err(|_| Error::InvalidValue)?,
        RouteId::new(SHOW_PROCESS_ROUTE).expect("nonzero route"),
    )?;

    let interval =
        ArgumentSpec::new("INTERVAL", ArgumentKind::Integer, false, false)
            .map_err(|_| Error::InvalidValue)?;
    let samples = ArgumentSpec::new("SAMPLES", ArgumentKind::Integer, false, false)
        .map_err(|_| Error::InvalidValue)?;
    registry.register(
        CommandSpec::new("MONITOR", &[interval, samples])
            .map_err(|_| Error::InvalidValue)?,
        RouteId::new(MONITOR_ROUTE).expect("nonzero route"),
    )
}

struct CompletionSlot {
    generation: u32,
    completion: Option<Result<StructuredOutput, Status>>,
}

impl CompletionSlot {
    const EMPTY: Self = Self {
        generation: 0,
        completion: None,
    };
}

/// Adapter that exposes diagnostic snapshots through the async shell contract.
pub struct DiagnosticExecutor<
    Source,
    const CAPACITY: usize = DEFAULT_DIAGNOSTIC_QUEUE,
> {
    source: Source,
    slots: [CompletionSlot; CAPACITY],
}

impl<Source, const CAPACITY: usize> DiagnosticExecutor<Source, CAPACITY> {
    pub fn new(source: Source) -> Self {
        Self {
            source,
            slots: [const { CompletionSlot::EMPTY }; CAPACITY],
        }
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    pub fn source_mut(&mut self) -> &mut Source {
        &mut self.source
    }
}

impl<Source: DiagnosticSource, const CAPACITY: usize> CommandExecutor
    for DiagnosticExecutor<Source, CAPACITY>
{
    fn submit(
        &mut self,
        command: CommandCall,
        _pipeline_input: Option<&StructuredOutput>,
    ) -> Result<ExecutionToken, Error> {
        let slot = self
            .slots
            .iter()
            .position(|slot| slot.completion.is_none())
            .ok_or(Error::Capacity)?;
        self.slots[slot].generation = self.slots[slot].generation.wrapping_add(1).max(1);
        self.slots[slot].completion = Some(self.execute(command));
        ExecutionToken::new(
            (self.slots[slot].generation as u64) << 32 | slot as u64,
        )
        .ok_or(Error::InvalidHandle)
    }

    fn poll(
        &mut self,
        token: ExecutionToken,
    ) -> Option<Result<StructuredOutput, Status>> {
        let slot = token.raw() as u32 as usize;
        let generation = (token.raw() >> 32) as u32;
        let entry = self.slots.get_mut(slot)?;
        if entry.generation != generation {
            return Some(Err(Status::NOT_FOUND))
        }
        entry.completion.take()
    }

    fn cancel(&mut self, token: ExecutionToken) -> Result<(), Error> {
        let slot = token.raw() as u32 as usize;
        let generation = (token.raw() >> 32) as u32;
        let entry = self.slots.get_mut(slot).ok_or(Error::InvalidHandle)?;
        if entry.generation != generation || entry.completion.is_none() {
            return Err(Error::InvalidHandle)
        }
        entry.completion = None;
        Ok(())
    }
}

impl<Source: DiagnosticSource, const CAPACITY: usize>
    DiagnosticExecutor<Source, CAPACITY>
{
    fn execute(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        match command.route.raw() {
            SHOW_MEMORY_ROUTE => {
                let cluster = boolean(command.get("CLUSTER"))?;
                if cluster {
                    cluster_output(
                        self.source.memory(true)?,
                        self.source.cluster()?,
                    )
                } else {
                    memory_output(self.source.memory(false)?)
                }
            }
            SHOW_PROCESS_ROUTE => {
                let pid = unsigned(command.get("PID"))?;
                process_output(self.source.process(pid)?)
            }
            MONITOR_ROUTE => {
                let interval = unsigned(command.get("INTERVAL"))?.unwrap_or(1_000_000);
                let samples = unsigned(command.get("SAMPLES"))?.unwrap_or(1);
                if interval == 0 || samples == 0 {
                    return Err(Status::INVALID_ARGUMENT)
                }
                monitor_output(self.source.monitor(interval, samples)?)
            }
            _ => Err(Status::NOT_FOUND),
        }
    }
}

fn boolean(value: Option<Value>) -> Result<bool, Status> {
    match value {
        Some(Value::Boolean(value)) => Ok(value),
        None => Ok(false),
        _ => Err(Status::INVALID_ARGUMENT),
    }
}

fn unsigned(value: Option<Value>) -> Result<Option<u64>, Status> {
    match value {
        Some(Value::Integer(value)) if value >= 0 => Ok(Some(value as u64)),
        None => Ok(None),
        _ => Err(Status::INVALID_ARGUMENT),
    }
}

fn memory_output(snapshot: MemorySnapshot) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert(&mut output, "total-bytes", OutputValue::Unsigned(snapshot.total_bytes))?;
    insert(&mut output, "free-bytes", OutputValue::Unsigned(snapshot.free_bytes))?;
    insert(&mut output, "local-bytes", OutputValue::Unsigned(snapshot.local_bytes))?;
    insert(&mut output, "cxl-bytes", OutputValue::Unsigned(snapshot.cxl_bytes))?;
    insert(&mut output, "layer2-bytes", OutputValue::Unsigned(snapshot.layer2_bytes))?;
    insert(&mut output, "vram-bytes", OutputValue::Unsigned(snapshot.vram_bytes))?;
    insert(
        &mut output,
        "active-leases",
        OutputValue::Unsigned(snapshot.active_leases),
    )?;
    insert(
        &mut output,
        "failed-nodes",
        OutputValue::Unsigned(snapshot.failed_nodes),
    )?;
    Ok(output)
}

fn cluster_output(
    memory: MemorySnapshot,
    snapshot: ClusterSnapshot,
) -> Result<StructuredOutput, Status> {
    let mut output = memory_output(memory)?;
    insert(&mut output, "nodes", OutputValue::Unsigned(snapshot.nodes))?;
    insert(
        &mut output,
        "healthy-nodes",
        OutputValue::Unsigned(snapshot.healthy_nodes),
    )?;
    insert(
        &mut output,
        "degraded-nodes",
        OutputValue::Unsigned(snapshot.degraded_nodes),
    )?;
    insert(
        &mut output,
        "heartbeat-us",
        OutputValue::Unsigned(snapshot.heartbeat_period_us),
    )?;
    insert(
        &mut output,
        "remote-pages",
        OutputValue::Unsigned(snapshot.remote_pages),
    )?;
    insert(
        &mut output,
        "migrations",
        OutputValue::Unsigned(snapshot.migrations),
    )?;
    Ok(output)
}

fn process_output(snapshot: ProcessSnapshot) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert(&mut output, "pid", OutputValue::Unsigned(snapshot.pid))?;
    insert(
        &mut output,
        "name",
        OutputValue::Text(
            OutputText::new(snapshot.name.as_str())
                .map_err(|_| Status::INVALID_ARGUMENT)?,
        ),
    )?;
    insert(
        &mut output,
        "state",
        OutputValue::Text(
            OutputText::new(snapshot.state.as_str())
                .map_err(|_| Status::INVALID_ARGUMENT)?,
        ),
    )?;
    insert(&mut output, "threads", OutputValue::Unsigned(snapshot.threads))?;
    insert(
        &mut output,
        "resident-bytes",
        OutputValue::Unsigned(snapshot.resident_bytes),
    )?;
    insert(
        &mut output,
        "cpu-time-us",
        OutputValue::Unsigned(snapshot.cpu_time_us),
    )?;
    Ok(output)
}

fn monitor_output(snapshot: MonitorSnapshot) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert(&mut output, "sample", OutputValue::Unsigned(snapshot.sample))?;
    insert(
        &mut output,
        "interval-us",
        OutputValue::Unsigned(snapshot.interval_us),
    )?;
    insert(
        &mut output,
        "runnable",
        OutputValue::Unsigned(snapshot.runnable_threads),
    )?;
    insert(
        &mut output,
        "cpu-busy-percent",
        OutputValue::Unsigned(snapshot.cpu_busy_percent),
    )?;
    insert(
        &mut output,
        "memory-used-bytes",
        OutputValue::Unsigned(snapshot.memory_used_bytes),
    )?;
    insert(
        &mut output,
        "remote-faults",
        OutputValue::Unsigned(snapshot.remote_faults),
    )?;
    insert(
        &mut output,
        "network-bytes",
        OutputValue::Unsigned(snapshot.network_bytes),
    )?;
    Ok(output)
}

fn insert(
    output: &mut StructuredOutput,
    name: &str,
    value: OutputValue,
) -> Result<(), Status> {
    output
        .insert(name, value)
        .map_err(|_| Status::INVALID_ARGUMENT)
}
