use crate::capability::{CapabilityHandle, CapabilityObject, CapabilitySpace, Rights};
use crate::task::{
    AddressSpaceId, Context, ExecutionMode, MAX_THREADS, SchedulingPolicy, Thread, ThreadId,
    ThreadState,
};
use synos_status::{IntoStatus, Severity, Status, facility};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    Full,
    InvalidThread,
    InvalidContext,
    InvalidExecutionMode,
    AccessDenied,
}

impl IntoStatus for SchedulerError {
    fn status(self) -> Status {
        match self {
            Self::Full => Status::NO_SPACE,
            Self::AccessDenied => Status::ACCESS_DENIED,
            Self::InvalidThread | Self::InvalidContext | Self::InvalidExecutionMode => {
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
}

impl Scheduler {
    pub const fn new() -> Self {
        Self {
            threads: [Thread::VACANT; MAX_THREADS],
            generations: [0; MAX_THREADS],
            current: None,
            cooperative_cursor: 0,
            clock: 0,
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
            context: Context::new(entry, stack_top),
            wake_at: 0,
            switches: 0,
        };
        Ok(id)
    }

    pub fn remove(&mut self, id: ThreadId) -> Result<(), SchedulerError> {
        let slot = self.slot(id)?;
        if self.current == Some(id) {
            self.current = None
        }
        self.threads[slot] = Thread::VACANT;
        Ok(())
    }

    pub fn current(&self) -> Option<ThreadId> {
        self.current
    }

    pub fn thread(&self, id: ThreadId) -> Result<&Thread, SchedulerError> {
        let slot = self.slot(id)?;
        Ok(&self.threads[slot])
    }

    pub fn context_mut(&mut self, id: ThreadId) -> Result<&mut Context, SchedulerError> {
        let slot = self.slot(id)?;
        Ok(&mut self.threads[slot].context)
    }

    pub fn dispatch(&mut self) -> Option<ContextSwitch> {
        if let Some(current) = self.current {
            if self.thread(current).ok()?.state == ThreadState::Running {
                return None;
            }
        }

        let previous = self.current.take();
        let Some(next) = self.pick_next() else {
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
        self.clock = self.clock.saturating_add(elapsed);
        for thread in &mut self.threads {
            if thread.state == ThreadState::Sleeping && thread.wake_at <= self.clock {
                thread.state = ThreadState::Ready
            }
        }

        let Some(current) = self.current else {
            return self.dispatch();
        };
        let candidate = self.pick_realtime();
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

    fn pick_next(&mut self) -> Option<ThreadId> {
        if let Some(realtime) = self.pick_realtime() {
            return Some(realtime);
        }

        for offset in 1..=MAX_THREADS {
            let slot = (self.cooperative_cursor + offset) % MAX_THREADS;
            let thread = self.threads[slot];
            if thread.state == ThreadState::Ready && thread.policy == SchedulingPolicy::Cooperative
            {
                self.cooperative_cursor = slot;
                return Some(thread.id);
            }
        }
        None
    }

    fn pick_realtime(&self) -> Option<ThreadId> {
        self.threads
            .iter()
            .filter(|thread| {
                thread.state == ThreadState::Ready
                    && matches!(thread.policy, SchedulingPolicy::Realtime { .. })
            })
            .min_by_key(|thread| match thread.policy {
                SchedulingPolicy::Realtime { priority, deadline } => {
                    (u8::MAX - priority, deadline, thread.id.raw())
                }
                SchedulingPolicy::Cooperative => (u8::MAX, u64::MAX, u32::MAX),
            })
            .map(|thread| thread.id)
    }

    fn outranks(&self, candidate: ThreadId, current: ThreadId) -> bool {
        let candidate = self.threads[candidate.slot()].policy;
        let current = self.threads[current.slot()].policy;
        match (candidate, current) {
            (
                SchedulingPolicy::Realtime {
                    priority: candidate_priority,
                    deadline: candidate_deadline,
                },
                SchedulingPolicy::Realtime {
                    priority: current_priority,
                    deadline: current_deadline,
                },
            ) => {
                candidate_priority > current_priority
                    || (candidate_priority == current_priority
                        && candidate_deadline < current_deadline)
            }
            (SchedulingPolicy::Realtime { .. }, SchedulingPolicy::Cooperative) => true,
            _ => false,
        }
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Self::new()
    }
}
