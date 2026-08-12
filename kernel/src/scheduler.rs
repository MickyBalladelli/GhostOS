use crate::capability::{CapabilityHandle, CapabilityObject, CapabilitySpace, Rights};
use crate::persona::{ExecutionPersona, PersonaError, RightIdentifier};
use crate::task::{
    AddressSpaceId, Context, CpuId, CpuMask, ExecutionMode, MAX_THREADS, SchedulingPolicy, Thread,
    ThreadId, ThreadState, MAX_CPUS,
};
use crate::partition::{CorePartition, CorePartitionError};
use synos_observability::{
    field, CapabilityDomain, CapabilityTraceStage, EventField, EventKind, Level, ProfileDomain,
    ProfileSample, ScalePolicy, emit_capability_trace, info, record_profile_sample,
};
use synos_numa::{NumaPlacement, NumaReport, NumaTopology, PlacementKind};
use synos_power::{
    CpuIdleState, IdleRequest, PowerClusterConfig, PowerMetrics, PowerPolicy, ProcessorSet,
    ThermalReading, WorkloadClass, WorkloadRequest,
};
use synos_status::{IntoStatus, Severity, Status, facility};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    Full,
    InvalidThread,
    InvalidContext,
    InvalidExecutionMode,
    InvalidPriority,
    InvalidCpuMask,
    InvalidPowerPolicy,
    NoHousekeepingCore,
    IpcWaitTableFull,
    AccessDenied,
}

impl IntoStatus for SchedulerError {
    fn status(self) -> Status {
        match self {
            Self::Full => Status::NO_SPACE,
            Self::AccessDenied => Status::ACCESS_DENIED,
            Self::InvalidThread
            | Self::InvalidContext
            | Self::InvalidExecutionMode
            | Self::InvalidPriority
            | Self::InvalidCpuMask
            | Self::InvalidPowerPolicy
            | Self::NoHousekeepingCore
            | Self::IpcWaitTableFull => {
                Status::new(Severity::Error, facility::KERNEL, 3, 0)
                    .expect("valid scheduler status")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextSwitch {
    pub previous: Option<ThreadId>,
    pub next: ThreadId,
    pub previous_address_space_root: Option<crate::PageTableRoot>,
    pub next_address_space_root: Option<crate::PageTableRoot>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PerCpuSchedulerState {
    pub cpu: CpuId,
    pub online: bool,
    pub current: Option<ThreadId>,
    pub interrupt_depth: u16,
    pub reschedule_pending: bool,
    pub clock: u64,
}

impl PerCpuSchedulerState {
    const fn new(cpu: CpuId) -> Self {
        Self {
            cpu,
            online: cpu.raw() == 0,
            current: None,
            interrupt_depth: 0,
            reschedule_pending: false,
            clock: 0,
        }
    }
}

const fn new_per_cpu_state() -> [PerCpuSchedulerState; MAX_CPUS] {
    let mut states = [PerCpuSchedulerState::new(CpuId::new(0).expect("CPU 0 is valid")); MAX_CPUS];
    let mut index = 0;
    while index < MAX_CPUS {
        states[index] = PerCpuSchedulerState::new(CpuId::new(index as u8).expect("CPU is valid"));
        index += 1;
    }
    states
}

pub struct Scheduler {
    threads: [Thread; MAX_THREADS],
    generations: [u16; MAX_THREADS],
    current: Option<ThreadId>,
    cooperative_cursor: usize,
    clock: u64,
    partition: CorePartition,
    current_cpu: CpuId,
    per_cpu: [PerCpuSchedulerState; MAX_CPUS],
    ipc_waiters: [Option<IpcWait>; MAX_THREADS],
    numa: NumaPlacement,
    power: PowerPolicy,
}

#[derive(Clone, Copy)]
struct IpcWait {
    endpoint: u32,
    owner: ThreadId,
    waiter: ThreadId,
}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            threads: [Thread::VACANT; MAX_THREADS],
            generations: [0; MAX_THREADS],
            current: None,
            cooperative_cursor: MAX_THREADS - 1,
            clock: 0,
            partition: CorePartition::new(),
            current_cpu: CpuId::new(0).expect("CPU 0 is valid"),
            per_cpu: new_per_cpu_state(),
            ipc_waiters: [None; MAX_THREADS],
            numa: NumaPlacement::uma(),
            power: PowerPolicy::new(),
        }
    }

    pub fn create<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        address_space: AddressSpaceId,
        mode: ExecutionMode,
        policy: SchedulingPolicy,
        entry: usize,
        stack_top: usize,
    ) -> Result<ThreadId, SchedulerError> {
        capabilities
            .authorize(
                caller,
                authority,
                CapabilityObject::AddressSpace(address_space),
                Rights::CREATE,
            )
            .map_err(|_| SchedulerError::AccessDenied)?;

        if entry == 0 || stack_top == 0 {
            return Err(SchedulerError::InvalidContext);
        }
        if (mode == ExecutionMode::Kernel) != (address_space == AddressSpaceId::KERNEL) {
            return Err(SchedulerError::InvalidExecutionMode);
        }

        let slot = self
            .threads
            .iter()
            .position(|thread| thread.state == ThreadState::Vacant)
            .ok_or(SchedulerError::Full)?;
        let generation = self.generations[slot].wrapping_add(1).max(1);
        self.generations[slot] = generation;
        let id = ThreadId::from_parts(slot, generation);
        let placement = self.numa.place(
            PlacementKind::Process,
            Some(self.current_cpu.raw() as u16),
            None,
        );
        let power_placement = self.power.place(WorkloadRequest {
            affinity: ProcessorSet::all(),
            class: match policy {
                SchedulingPolicy::Realtime { .. } => WorkloadClass::LatencyCritical,
                SchedulingPolicy::Cooperative => WorkloadClass::Background,
            },
            runnable: 1,
            latency_budget_us: match policy {
                SchedulingPolicy::Realtime { deadline, .. } => deadline.saturating_sub(self.clock),
                SchedulingPolicy::Cooperative => 0,
            },
            preferred_cluster: None,
        }).ok();
        self.threads[slot] = Thread {
            id,
            address_space,
            address_space_root: None,
            mode,
            state: ThreadState::Ready,
            policy,
            persona: ExecutionPersona::anonymous(),
            context: Context::new(entry, stack_top),
            affinity: power_placement
                .map(|placement| CpuMask::from_words(
                    placement.cpus.raw_words()[0],
                    placement.cpus.raw_words()[1],
                ))
                .unwrap_or_else(CpuMask::all),
            numa_node: placement.selected_node,
            wake_at: 0,
            switches: 0,
            inherited_priority: 0,
            inherited_deadline: u64::MAX,
        };
        self.debug_check();
        info!(
            EventKind::Kernel,
            EventField::unsigned(field::NUMA_KIND, PlacementKind::Process as u64),
            EventField::unsigned(field::NUMA_REQUESTED_NODE, placement.requested_node as u64),
            EventField::unsigned(field::NUMA_SELECTED_NODE, placement.selected_node as u64),
            EventField::unsigned(field::NUMA_LOCALITY, placement.locality as u64),
        );
        Ok(id)
    }

    /// Create a user thread bound to its own page-table root.
    pub fn create_user<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        address_space: AddressSpaceId,
        root: crate::PageTableRoot,
        policy: SchedulingPolicy,
        entry: usize,
        stack_top: usize,
    ) -> Result<ThreadId, SchedulerError> {
        let id = self.create(
            capabilities,
            caller,
            authority,
            address_space,
            ExecutionMode::User,
            policy,
            entry,
            stack_top,
        )?;
        self.threads[id.slot()].address_space_root = Some(root);
        self.debug_check();
        Ok(id)
    }

    /// Configure the hardware topology. UMA is the default and remains a
    /// valid fallback when firmware does not describe NUMA nodes.
    pub fn configure_numa<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        topology: NumaTopology,
    ) -> Result<(), SchedulerError> {
        self.authorize_system_control(capabilities, caller, authority)?;
        self.numa.set_topology(topology);
        self.debug_check();
        Ok(())
    }

    pub const fn numa_report(&self) -> NumaReport {
        self.numa.report()
    }

    pub const fn power_metrics(&self) -> PowerMetrics {
        self.power.metrics()
    }

    pub fn configure_power_clusters<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        clusters: &[PowerClusterConfig],
    ) -> Result<(), SchedulerError> {
        self.authorize_system_control(capabilities, caller, authority)?;
        self.power
            .replace_clusters(clusters)
            .map_err(|_| SchedulerError::InvalidPowerPolicy)?;
        self.debug_check();
        Ok(())
    }

    pub fn configure_thermal_policy<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        trips: synos_power::ThermalTripPoints,
        hysteresis_deci_kelvin: u32,
    ) -> Result<(), SchedulerError> {
        self.authorize_system_control(capabilities, caller, authority)?;
        self.power
            .configure_thermal(trips, hysteresis_deci_kelvin)
            .map_err(|_| SchedulerError::InvalidPowerPolicy)?;
        self.debug_check();
        Ok(())
    }

    pub fn update_thermal_policy(&mut self, reading: ThermalReading) -> synos_power::ThermalAction {
        self.power.update_thermal(reading)
    }

    pub fn idle_state_on(
        &mut self,
        cpu: CpuId,
        next_wake_us: u64,
        latency_budget_us: u64,
    ) -> CpuIdleState {
        self.power.request_idle(
            cpu.raw() as u16,
            IdleRequest {
                now_us: self.clock,
                next_wake_us,
                latency_budget_us,
            },
        )
    }

    /// Stop a task after proving control over the exact task or its address
    /// space. The capability is checked before the task is removed.
    pub fn stop<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        id: ThreadId,
    ) -> Result<(), SchedulerError> {
        let slot = self.authorized_control(capabilities, caller, authority, id)?;
        if self.current == Some(id) {
            self.current = None
        }
        self.clear_ipc_waits(id);
        self.threads[slot] = Thread::VACANT;
        emit_capability_trace(
            Level::Info,
            CapabilityDomain::Process,
            CapabilityTraceStage::Revoked,
            authority.raw(),
            id.raw() as u16,
        );
        self.debug_check();
        Ok(())
    }

    /// Terminate is the explicit process-control spelling of [`Self::stop`].
    pub fn terminate<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        id: ThreadId,
    ) -> Result<(), SchedulerError> {
        self.stop(capabilities, caller, authority, id)
    }

    /// Change a task's dynamic realtime priority under the same control
    /// capability used by STOP. Priorities are one (lowest) through 255.
    pub fn set_priority<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        id: ThreadId,
        priority: u8,
    ) -> Result<(), SchedulerError> {
        if priority == 0 {
            return Err(SchedulerError::InvalidPriority)
        }
        let slot = self.authorized_control(capabilities, caller, authority, id)?;
        let deadline = match self.threads[slot].policy {
            SchedulingPolicy::Realtime { deadline, .. } => deadline,
            SchedulingPolicy::Cooperative => 0,
        };
        self.threads[slot].policy = SchedulingPolicy::Realtime {
            priority,
            deadline,
        };
        self.debug_check();
        Ok(())
    }

    pub fn current(&self) -> Option<ThreadId> {
        self.current
    }

    /// Remove the faulting current thread without requiring a user-visible
    /// control capability. The page-fault path uses this for fatal user
    /// memory violations, then dispatches another runnable thread.
    pub fn terminate_current_fault(&mut self) -> Option<ContextSwitch> {
        let current = self.current.take()?;
        let slot = current.slot();
        self.clear_ipc_waits(current);
        self.threads[slot] = Thread::VACANT;
        self.debug_check();
        self.dispatch()
    }

    pub const fn partition(&self) -> CorePartition {
        self.partition
    }

    pub fn per_cpu_state(&self, cpu: CpuId) -> Option<PerCpuSchedulerState> {
        self.per_cpu.get(cpu.raw() as usize).copied()
    }

    pub fn enter_interrupt(&mut self, cpu: CpuId) -> Result<(), SchedulerError> {
        let state = self
            .per_cpu
            .get_mut(cpu.raw() as usize)
            .ok_or(SchedulerError::InvalidCpuMask)?;
        state.interrupt_depth = state.interrupt_depth.saturating_add(1);
        Ok(())
    }

    pub fn exit_interrupt(&mut self, cpu: CpuId) -> Result<(), SchedulerError> {
        let state = self
            .per_cpu
            .get_mut(cpu.raw() as usize)
            .ok_or(SchedulerError::InvalidCpuMask)?;
        state.interrupt_depth = state.interrupt_depth.saturating_sub(1);
        Ok(())
    }

    pub fn request_reschedule(&mut self, cpu: CpuId) -> Result<(), SchedulerError> {
        let state = self
            .per_cpu
            .get_mut(cpu.raw() as usize)
            .ok_or(SchedulerError::InvalidCpuMask)?;
        state.reschedule_pending = true;
        Ok(())
    }

    pub fn take_reschedule(&mut self, cpu: CpuId) -> Result<bool, SchedulerError> {
        let state = self
            .per_cpu
            .get_mut(cpu.raw() as usize)
            .ok_or(SchedulerError::InvalidCpuMask)?;
        let pending = state.reschedule_pending;
        state.reschedule_pending = false;
        Ok(pending)
    }

    pub const fn scale_policy(cpu_count: usize) -> Option<ScalePolicy> {
        ScalePolicy::for_cpu_count(cpu_count)
    }

    /// Apply one of the qualified CPU layouts to scheduler-owned paths.
    /// Housekeeping CPUs run scheduler, IPC, and timer work; isolated CPUs
    /// remain available only to explicitly pinned workloads.
    pub fn configure_scale_policy<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        policy: ScalePolicy,
    ) -> Result<(), SchedulerError> {
        self.authorize_system_control(capabilities, caller, authority)?;
        let online = policy.online().words();
        self.partition
            .set_online(CpuMask::from_words(online[0], online[1]))
            .map_err(map_partition_error)?;
        let isolated = policy.isolated().words();
        let isolated = CpuMask::from_words(isolated[0], isolated[1]);
        if !isolated.is_empty() {
            self.partition.isolate(isolated).map_err(map_partition_error)?
        }
        self.sync_per_cpu_online();
        for raw in 0..crate::task::MAX_CPUS as u8 {
            if let Some(cpu) = CpuId::new(raw) {
                crate::arch::interrupts::set_core_isolated(
                    cpu.raw(),
                    self.partition.is_isolated(cpu),
                )
            }
        }
        self.debug_check();
        Ok(())
    }

    pub fn set_online_cores<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        online: CpuMask,
    ) -> Result<(), SchedulerError> {
        self.authorize_system_control(capabilities, caller, authority)?;
        self.partition
            .set_online(online)
            .map_err(map_partition_error)?;
        self.sync_per_cpu_online();
        self.debug_check();
        Ok(())
    }

    pub fn isolate_cores<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        cpus: CpuMask,
    ) -> Result<(), SchedulerError> {
        self.authorize_system_control(capabilities, caller, authority)?;
        self.partition.isolate(cpus).map_err(map_partition_error)?;
        for raw in 0..crate::task::MAX_CPUS as u8 {
            if let Some(cpu) = CpuId::new(raw) {
                crate::arch::interrupts::set_core_isolated(cpu.raw(), self.partition.is_isolated(cpu));
            }
        }
        self.debug_check();
        Ok(())
    }

    pub fn release_cores<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        cpus: CpuMask,
    ) -> Result<(), SchedulerError> {
        self.authorize_system_control(capabilities, caller, authority)?;
        self.partition.release(cpus);
        for raw in 0..crate::task::MAX_CPUS as u8 {
            if let Some(cpu) = CpuId::new(raw) {
                crate::arch::interrupts::set_core_isolated(
                    cpu.raw(),
                    self.partition.is_isolated(cpu),
                );
            }
        }
        self.debug_check();
        Ok(())
    }

    pub fn set_affinity<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        id: ThreadId,
        affinity: CpuMask,
    ) -> Result<(), SchedulerError> {
        self.authorized_control(capabilities, caller, authority, id)?;
        if affinity.is_empty() || !affinity.intersects(self.partition.online()) {
            return Err(SchedulerError::InvalidCpuMask)
        }
        let slot = self.slot(id)?;
        self.threads[slot].affinity = affinity;
        self.debug_check();
        Ok(())
    }

    pub fn effective_priority(&self, id: ThreadId) -> Result<u8, SchedulerError> {
        let slot = self.slot(id)?;
        Ok(self.effective_key(&self.threads[slot]).0)
    }

    /// Record a bounded IPC wait and propagate the waiter's effective priority
    /// through the owner chain. This is deterministic and allocation-free.
    pub fn ipc_wait(
        &mut self,
        endpoint: u32,
        owner: ThreadId,
        waiter: ThreadId,
    ) -> Result<(), SchedulerError> {
        if owner == waiter {
            return Err(SchedulerError::InvalidThread)
        }
        self.slot(owner)?;
        self.slot(waiter)?;
        let slot = self
            .ipc_waiters
            .iter_mut()
            .find(|entry| match entry {
                None => true,
                Some(entry) => entry.endpoint == endpoint && entry.waiter == waiter,
            })
            .ok_or(SchedulerError::IpcWaitTableFull)?;
        *slot = Some(IpcWait {
            endpoint,
            owner,
            waiter,
        });
        self.recompute_inheritance();
        self.debug_check();
        Ok(())
    }

    pub fn ipc_complete(&mut self, endpoint: u32, waiter: ThreadId) {
        for entry in &mut self.ipc_waiters {
            if entry.is_some_and(|entry| entry.endpoint == endpoint && entry.waiter == waiter) {
                *entry = None
            }
        }
        self.recompute_inheritance();
        self.debug_check()
    }

    fn clear_ipc_waits(&mut self, thread: ThreadId) {
        for entry in &mut self.ipc_waiters {
            if entry.is_some_and(|entry| entry.owner == thread || entry.waiter == thread) {
                *entry = None
            }
        }
        self.recompute_inheritance()
    }

    pub fn thread(&self, id: ThreadId) -> Result<&Thread, SchedulerError> {
        let slot = self.slot(id)?;
        Ok(&self.threads[slot])
    }

    pub fn context_mut(&mut self, id: ThreadId) -> Result<&mut Context, SchedulerError> {
        let slot = self.slot(id)?;
        Ok(&mut self.threads[slot].context)
    }

    pub fn install_persona<const MAX_CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        id: ThreadId,
        persona: ExecutionPersona,
    ) -> Result<(), SchedulerError> {
        let slot = self.slot(id)?;
        let address_space = self.threads[slot].address_space;
        capabilities
            .authorize(
                caller,
                authority,
                CapabilityObject::AddressSpace(address_space),
                Rights::CREATE,
            )
            .map_err(|_| SchedulerError::AccessDenied)?;
        self.threads[slot].persona = persona;
        self.debug_check();
        Ok(())
    }

    pub fn disable_right(
        &mut self,
        caller: AddressSpaceId,
        id: ThreadId,
        right: RightIdentifier,
    ) -> Result<(), SchedulerError> {
        let slot = self.owned_thread(caller, id)?;
        self.threads[slot]
            .persona
            .disable(right)
            .map_err(map_persona_error)?;
        self.debug_check();
        Ok(())
    }

    pub fn enable_right(
        &mut self,
        caller: AddressSpaceId,
        id: ThreadId,
        right: RightIdentifier,
    ) -> Result<(), SchedulerError> {
        let slot = self.owned_thread(caller, id)?;
        self.threads[slot]
            .persona
            .enable(right)
            .map_err(map_persona_error)?;
        self.debug_check();
        Ok(())
    }

    pub fn drop_right(
        &mut self,
        caller: AddressSpaceId,
        id: ThreadId,
        right: RightIdentifier,
    ) -> Result<(), SchedulerError> {
        let slot = self.owned_thread(caller, id)?;
        self.threads[slot]
            .persona
            .drop_right(right)
            .map_err(map_persona_error)?;
        self.debug_check();
        Ok(())
    }

    pub fn dispatch(&mut self) -> Option<ContextSwitch> {
        self.dispatch_on(self.current_cpu)
    }

    pub fn dispatch_on(&mut self, cpu: CpuId) -> Option<ContextSwitch> {
        if !self.partition.accepts_kernel_work(cpu) {
            self.debug_check();
            return None
        }
        self.current_cpu = cpu;
        if let Some(state) = self.per_cpu.get_mut(cpu.raw() as usize) {
            state.online = true;
            state.reschedule_pending = false;
        }
        if let Some(current) = self.current {
            if self.thread(current).ok()?.state == ThreadState::Running {
                self.debug_check();
                return None;
            }
        }

        let previous = self.current.take();
        if let Some(state) = self.per_cpu.get_mut(cpu.raw() as usize) {
            state.current = None;
        }
        let Some(next) = self.pick_next(self.partition.housekeeping()) else {
            self.idle_state_on(
                cpu,
                self.clock.saturating_add(1_000),
                1_000,
            );
            self.debug_check();
            return None;
        };
        self.current = Some(next);
        if let Some(state) = self.per_cpu.get_mut(cpu.raw() as usize) {
            state.current = Some(next);
        }
        let thread = &mut self.threads[next.slot()];
        thread.state = ThreadState::Running;
        thread.switches = thread.switches.saturating_add(1);
        record_profile_sample(ProfileSample::single(
            ProfileDomain::Scheduler,
            self.clock,
            cpu.raw() as u32,
            0x3001,
        ));
        self.debug_check();
        Some(ContextSwitch {
            previous,
            next,
            previous_address_space_root: previous
                .and_then(|id| self.thread(id).ok()?.address_space_root),
            next_address_space_root: self.thread(next).ok()?.address_space_root,
        })
    }

    pub fn yield_current(&mut self) -> Result<Option<ContextSwitch>, SchedulerError> {
        let current = self.current.ok_or(SchedulerError::InvalidThread)?;
        let slot = self.slot(current)?;
        self.threads[slot].state = ThreadState::Ready;
        Ok(self.dispatch())
    }

    pub fn block_current(&mut self) -> Result<Option<ContextSwitch>, SchedulerError> {
        let current = self.current.ok_or(SchedulerError::InvalidThread)?;
        let slot = self.slot(current)?;
        self.threads[slot].state = ThreadState::Blocked;
        Ok(self.dispatch())
    }

    pub fn sleep_current(
        &mut self,
        duration: u64,
    ) -> Result<Option<ContextSwitch>, SchedulerError> {
        let current = self.current.ok_or(SchedulerError::InvalidThread)?;
        let slot = self.slot(current)?;
        self.threads[slot].state = ThreadState::Sleeping;
        self.threads[slot].wake_at = self.clock.saturating_add(duration);
        Ok(self.dispatch())
    }

    pub fn wake(&mut self, id: ThreadId) -> Result<(), SchedulerError> {
        let slot = self.slot(id)?;
        match self.threads[slot].state {
            ThreadState::Blocked | ThreadState::Sleeping => {
                self.threads[slot].state = ThreadState::Ready
            }
            _ => {}
        }
        self.debug_check();
        Ok(())
    }

    pub fn tick(&mut self, elapsed: u64) -> Option<ContextSwitch> {
        self.tick_on(self.current_cpu, elapsed)
    }

    pub fn tick_on(&mut self, cpu: CpuId, elapsed: u64) -> Option<ContextSwitch> {
        if !self.partition.accepts_timer(cpu) {
            return None
        }
        self.clock = self.clock.saturating_add(elapsed);
        if let Some(state) = self.per_cpu.get_mut(cpu.raw() as usize) {
            state.clock = state.clock.saturating_add(elapsed);
        }
        for thread in &mut self.threads {
            if thread.state == ThreadState::Sleeping && thread.wake_at <= self.clock {
                thread.state = ThreadState::Ready
            }
        }

        let Some(current) = self.current else {
            return self.dispatch_on(cpu);
        };
        let candidate = self.pick_realtime(self.partition.housekeeping());
        if let Some(next) = candidate {
            if next != current && self.outranks(next, current) {
                self.threads[current.slot()].state = ThreadState::Ready;
                return self.dispatch();
            }
        }
        self.debug_check();
        None
    }

    pub const fn clock(&self) -> u64 {
        self.clock
    }

    pub(crate) fn crash_snapshot(&self) -> crate::crash::SchedulerState {
        let mut state_counts = [0u16; 5];
        for thread in self.threads {
            let index = match thread.state {
                ThreadState::Vacant => 0,
                ThreadState::Ready => 1,
                ThreadState::Running => 2,
                ThreadState::Blocked => 3,
                ThreadState::Sleeping => 4,
            };
            state_counts[index] = state_counts[index].saturating_add(1)
        }
        let (current_thread, current_instruction_pointer, current_stack_pointer) = self
            .current
            .and_then(|id| self.threads.get(id.slot()).map(|thread| (id, thread)))
            .map_or((0, 0, 0), |(id, thread)| {
                (
                    id.raw(),
                    thread.context.instruction_pointer as u64,
                    thread.context.stack_pointer as u64,
                )
            });
        crate::crash::SchedulerState {
            clock: self.clock,
            current_thread,
            current_cpu: self.current_cpu.raw(),
            state_counts,
            current_instruction_pointer,
            current_stack_pointer,
        }
    }

    /// Validate scheduler ownership, generations, runnable states, and waiters.
    pub fn check_invariants(&self) -> Result<(), crate::invariants::InvariantFailure> {
        let mut running = None;
        for (slot, thread) in self.threads.iter().enumerate() {
            if thread.state == ThreadState::Vacant {
                if thread.id.raw() != 0 || !thread.affinity.is_empty() {
                    return Err(crate::invariants::InvariantFailure::new(
                        crate::invariants::InvariantId::SchedulerState,
                    ))
                }
                continue
            }
            if thread.id.slot() != slot
                || thread.id.generation() == 0
                || self.generations[slot] != thread.id.generation()
                || thread.context.instruction_pointer == 0
                || thread.context.stack_pointer == 0
                || thread.affinity.is_empty()
                || (thread.mode == ExecutionMode::Kernel)
                    != (thread.address_space == AddressSpaceId::KERNEL)
            {
                return Err(crate::invariants::InvariantFailure::new(
                    crate::invariants::InvariantId::SchedulerState,
                ))
            }
            crate::invariants::check_address_space(thread.address_space)?;
            if thread.state == ThreadState::Running {
                if running.replace(thread.id).is_some() {
                    return Err(crate::invariants::InvariantFailure::new(
                        crate::invariants::InvariantId::SchedulerState,
                    ))
                }
            }
        }
        if self.current != running {
            return Err(crate::invariants::InvariantFailure::new(
                crate::invariants::InvariantId::SchedulerState,
            ))
        }
        if self.partition.online().is_empty()
            || !self.partition.isolated().difference(self.partition.online()).is_empty()
            || self.partition.housekeeping().is_empty()
        {
            return Err(crate::invariants::InvariantFailure::new(
                crate::invariants::InvariantId::SchedulerState,
            ))
        }
        for waiter in self.ipc_waiters.iter().flatten() {
            if waiter.endpoint == 0
                || waiter.owner == waiter.waiter
                || self.slot(waiter.owner).is_err()
                || self.slot(waiter.waiter).is_err()
            {
                return Err(crate::invariants::InvariantFailure::new(
                    crate::invariants::InvariantId::SchedulerState,
                ))
            }
        }
        Ok(())
    }

    fn debug_check(&self) {
        crate::invariants::debug_assert_valid(self.check_invariants())
    }

    fn sync_per_cpu_online(&mut self) {
        let online = self.partition.online();
        for state in &mut self.per_cpu {
            state.online = online.contains(state.cpu);
            if !state.online {
                state.current = None;
                state.reschedule_pending = false;
            }
        }
    }

    fn slot(&self, id: ThreadId) -> Result<usize, SchedulerError> {
        let slot = id.slot();
        let thread = self
            .threads
            .get(slot)
            .ok_or(SchedulerError::InvalidThread)?;
        if thread.state == ThreadState::Vacant || id.generation() != self.generations[slot] {
            return Err(SchedulerError::InvalidThread);
        }
        Ok(slot)
    }

    fn owned_thread(
        &self,
        caller: AddressSpaceId,
        id: ThreadId,
    ) -> Result<usize, SchedulerError> {
        let slot = self.slot(id)?;
        if self.threads[slot].address_space != caller {
            return Err(SchedulerError::AccessDenied)
        }
        Ok(slot)
    }

    fn authorized_control<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        id: ThreadId,
    ) -> Result<usize, SchedulerError> {
        let slot = self.slot(id)?;
        let address_space = self.threads[slot].address_space;
        let exact = capabilities.authorize(
            caller,
            authority,
            CapabilityObject::Thread(id),
            Rights::CONTROL,
        );
        if exact.is_ok()
            || capabilities
                .authorize(
                    caller,
                    authority,
                    CapabilityObject::AddressSpace(address_space),
                    Rights::CONTROL,
                )
                .is_ok()
            || capabilities
                .authorize(caller, authority, CapabilityObject::SystemControl, Rights::CONTROL)
                .is_ok()
        {
            emit_capability_trace(
                Level::Trace,
                CapabilityDomain::Process,
                CapabilityTraceStage::DaemonAuthorized,
                authority.raw(),
                id.raw() as u16,
            );
            Ok(slot)
        } else {
            Err(SchedulerError::AccessDenied)
        }
    }

    fn pick_next(&mut self, cpus: CpuMask) -> Option<ThreadId> {
        if let Some(realtime) = self.pick_realtime(cpus) {
            return Some(realtime);
        }

        for offset in 1..=MAX_THREADS {
            let slot = (self.cooperative_cursor + offset) % MAX_THREADS;
            let thread = self.threads[slot];
            if thread.state == ThreadState::Ready
                && thread.policy == SchedulingPolicy::Cooperative
                && thread.affinity.intersects(cpus)
            {
                self.cooperative_cursor = slot;
                return Some(thread.id);
            }
        }
        None
    }

    fn pick_realtime(&self, cpus: CpuMask) -> Option<ThreadId> {
        self.threads
            .iter()
            .filter(|thread| {
                thread.state == ThreadState::Ready
                    && thread.affinity.intersects(cpus)
                    && self.effective_key(thread).0 != 0
            })
            .min_by_key(|thread| match thread.policy {
                SchedulingPolicy::Realtime { .. } | SchedulingPolicy::Cooperative => {
                    let (priority, deadline) = self.effective_key(thread);
                    (u8::MAX - priority, deadline, thread.id.raw())
                }
            })
            .map(|thread| thread.id)
    }

    fn outranks(&self, candidate: ThreadId, current: ThreadId) -> bool {
        let candidate = self.effective_key(&self.threads[candidate.slot()]);
        let current = self.effective_key(&self.threads[current.slot()]);
        candidate.0 > current.0 || (candidate.0 == current.0 && candidate.1 < current.1)
    }

    fn effective_key(&self, thread: &Thread) -> (u8, u64) {
        let (base_priority, base_deadline) = match thread.policy {
            SchedulingPolicy::Realtime { priority, deadline } => (priority, deadline),
            SchedulingPolicy::Cooperative => (0, u64::MAX),
        };
        if thread.inherited_priority > base_priority {
            (thread.inherited_priority, thread.inherited_deadline)
        } else {
            (base_priority, base_deadline)
        }
    }

    fn recompute_inheritance(&mut self) {
        for thread in &mut self.threads {
            thread.inherited_priority = 0;
            thread.inherited_deadline = u64::MAX;
        }
        for _ in 0..MAX_THREADS {
            let mut changed = false;
            for entry in self.ipc_waiters.iter().flatten().copied() {
                let waiter = self.threads[entry.waiter.slot()];
                let (priority, deadline) = self.effective_key(&waiter);
                let owner = &mut self.threads[entry.owner.slot()];
                if priority > owner.inherited_priority
                    || (priority == owner.inherited_priority
                        && deadline < owner.inherited_deadline)
                {
                    owner.inherited_priority = priority;
                    owner.inherited_deadline = deadline;
                    changed = true
                }
            }
            if !changed {
                break
            }
        }
    }

    fn authorize_system_control<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
    ) -> Result<(), SchedulerError> {
        capabilities
            .authorize(caller, authority, CapabilityObject::SystemControl, Rights::CONTROL)
            .map_err(|_| SchedulerError::AccessDenied)
    }
}

fn map_persona_error(_: PersonaError) -> SchedulerError {
    SchedulerError::AccessDenied
}

fn map_partition_error(error: CorePartitionError) -> SchedulerError {
    match error {
        CorePartitionError::EmptyMask | CorePartitionError::OfflineCore => {
            SchedulerError::InvalidCpuMask
        }
        CorePartitionError::NoHousekeepingCore => SchedulerError::NoHousekeepingCore,
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}
