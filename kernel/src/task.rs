use crate::persona::ExecutionPersona;

pub const MAX_THREADS: usize = 64;
pub const MAX_CPUS: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct CpuId(u8);

impl CpuId {
    pub const fn new(raw: u8) -> Option<Self> {
        if raw < MAX_CPUS as u8 {
            Some(Self(raw))
        } else {
            None
        }
    }

    pub const fn raw(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct CpuMask([u64; 2]);

impl CpuMask {
    pub const EMPTY: Self = Self([0; 2]);
    pub const CPU0: Self = Self([1, 0]);

    pub const fn from_raw(raw: u64) -> Self {
        Self([raw, 0])
    }

    pub const fn from_words(low: u64, high: u64) -> Self {
        Self([low, high])
    }

    pub const fn all() -> Self {
        Self([u64::MAX; 2])
    }

    pub const fn raw(self) -> u64 {
        self.0[0]
    }

    pub const fn raw_words(self) -> [u64; 2] {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0[0] == 0 && self.0[1] == 0
    }

    pub const fn contains(self, cpu: CpuId) -> bool {
        let raw = cpu.raw() as usize;
        self.0[raw / 64] & (1u64 << (raw % 64)) != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self([self.0[0] | other.0[0], self.0[1] | other.0[1]])
    }

    pub const fn difference(self, other: Self) -> Self {
        Self([self.0[0] & !other.0[0], self.0[1] & !other.0[1]])
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0[0] & other.0[0] != 0 || self.0[1] & other.0[1] != 0
    }

    pub const fn from_cpu(cpu: CpuId) -> Self {
        let raw = cpu.raw() as usize;
        if raw < 64 {
            Self([1u64 << raw, 0])
        } else {
            Self([0, 1u64 << (raw - 64)])
        }
    }
}

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
    pub affinity: CpuMask,
    pub wake_at: u64,
    pub switches: u64,
    pub inherited_priority: u8,
    pub inherited_deadline: u64,
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
        affinity: CpuMask::EMPTY,
        wake_at: 0,
        switches: 0,
        inherited_priority: 0,
        inherited_deadline: u64::MAX,
    };
}
