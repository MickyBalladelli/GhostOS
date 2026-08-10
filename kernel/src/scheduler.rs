use crate::capability::{CapabilityHandle, CapabilityObject, CapabilitySpace, Rights};
use crate::persona::{ExecutionPersona, PersonaError, RightIdentifier};
use crate::task::{
    AddressSpaceId, Context, CpuId, CpuMask, ExecutionMode, MAX_THREADS, SchedulingPolicy, Thread,
    ThreadId, ThreadState,
};
use crate::partition::{CorePartition, CorePartitionError};
use synos_observability::{
    CapabilityDomain, CapabilityTraceStage, Level, emit_capability_trace,
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
}

pub struct Scheduler {
    threads: [Thread; MAX_THREADS],
    generations: [u16; MAX_THREADS],
    current: Option<ThreadId>,
    cooperative_cursor: usize,
    clock: u64,
    partition: CorePartition,
    current_cpu: CpuId,
    ipc_waiters: [Option<IpcWait>; MAX_THREADS],
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
            cooperative_cursor: 0,
            clock: 0,
            partition: CorePartition::new(),
            current_cpu: CpuId::new(0).expect("CPU 0 is valid"),
            ipc_waiters: [None; MAX_THREADS],
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
        self.threads[slot] = Thread {
            id,
            address_space,
            mode,
            state: ThreadState::Ready,
            policy,
            persona: ExecutionPersona::anonymous(),
            context: Context::new(entry, stack_top),
            affinity: CpuMask::all(),
            wake_at: 0,
            switches: 0,
            inherited_priority: 0,
            inherited_deadline: u64::MAX,
        };
        Ok(id)
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
        Ok(())
    }

    pub fn current(&self) -> Option<ThreadId> {
        self.current
    }

    pub const fn partition(&self) -> CorePartition {
        self.partition
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
            .map_err(map_partition_error)
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
        Ok(())
    }

    pub fn ipc_complete(&mut self, endpoint: u32, waiter: ThreadId) {
        for entry in &mut self.ipc_waiters {
            if entry.is_some_and(|entry| entry.endpoint == endpoint && entry.waiter == waiter) {
                *entry = None
            }
        }
        self.recompute_inheritance()
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
            .map_err(map_persona_error)
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
            .map_err(map_persona_error)
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
            .map_err(map_persona_error)
    }

    pub fn dispatch(&mut self) -> Option<ContextSwitch> {
        self.dispatch_on(self.current_cpu)
    }

    pub fn dispatch_on(&mut self, cpu: CpuId) -> Option<ContextSwitch> {
        if !self.partition.accepts_kernel_work(cpu) {
            return None
        }
        self.current_cpu = cpu;
        if let Some(current) = self.current {
            if self.thread(current).ok()?.state == ThreadState::Running {
                return None;
            }
        }

        let previous = self.current.take();
        let Some(next) = self.pick_next(self.partition.housekeeping()) else {
            return None;
        };
        self.current = Some(next);
        let thread = &mut self.threads[next.slot()];
        thread.state = ThreadState::Running;
        thread.switches = thread.switches.saturating_add(1);
        Some(ContextSwitch { previous, next })
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
        None
    }

    pub const fn clock(&self) -> u64 {
        self.clock
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
