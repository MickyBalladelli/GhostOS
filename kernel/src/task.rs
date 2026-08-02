use crate::persona::ExecutionPersona;

pub const MAX_THREADS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct AddressSpaceId(u32);

impl AddressSpaceId {
    pub const KERNEL: Self = Self(0);

    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ThreadId(u32);

impl ThreadId {
    pub(crate) const fn from_parts(slot: usize, generation: u16) -> Self {
        Self(((generation as u32) << 16) | slot as u32)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 || raw >> 16 == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    pub(crate) const fn slot(self) -> usize {
        (self.0 & 0xffff) as usize
    }

    pub(crate) const fn generation(self) -> u16 {
        (self.0 >> 16) as u16
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionMode {
    Kernel,
    User,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadState {
    Vacant,
    Ready,
    Running,
    Blocked,
    Sleeping,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulingPolicy {
    Cooperative,
    Realtime { priority: u8, deadline: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Context {
    pub instruction_pointer: usize,
    pub stack_pointer: usize,
    pub callee_saved: [usize; 12],
}

impl Context {
    pub const fn new(instruction_pointer: usize, stack_pointer: usize) -> Self {
        Self {
            instruction_pointer,
            stack_pointer,
            callee_saved: [0; 12],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Thread {
    pub id: ThreadId,
    pub address_space: AddressSpaceId,
    pub mode: ExecutionMode,
    pub state: ThreadState,
    pub policy: SchedulingPolicy,
    pub persona: ExecutionPersona,
    pub context: Context,
    pub wake_at: u64,
    pub switches: u64,
}

impl Thread {
    pub(crate) const VACANT: Self = Self {
        id: ThreadId(0),
        address_space: AddressSpaceId::KERNEL,
        mode: ExecutionMode::Kernel,
        state: ThreadState::Vacant,
        policy: SchedulingPolicy::Cooperative,
        persona: ExecutionPersona::anonymous(),
        context: Context::new(0, 0),
        wake_at: 0,
        switches: 0,
    };
}
