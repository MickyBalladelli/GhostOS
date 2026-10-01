use crate::persona::ExecutionPersona;

unsafe extern "C" {
    fn ghostos_task_cpu_mask_contains(low: u64, high: u64, cpu: u8) -> bool;
    fn ghostos_task_cpu_mask_union(left: u64, right: u64) -> u64;
    fn ghostos_task_cpu_mask_difference(left: u64, right: u64) -> u64;
    fn ghostos_task_cpu_mask_intersects(
        left_low: u64,
        left_high: u64,
        right_low: u64,
        right_high: u64,
    ) -> bool;
    fn ghostos_task_cpu_mask_from_cpu(cpu: u8, words: *mut u64);
    fn ghostos_task_address_space_id_valid(raw: u32) -> bool;
    fn ghostos_task_thread_id_from_parts(slot: usize, generation: u16) -> u32;
    fn ghostos_task_thread_id_valid(raw: u32) -> bool;
    fn ghostos_task_thread_slot(raw: u32) -> usize;
    fn ghostos_task_thread_generation(raw: u32) -> u16;
}

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

    pub fn contains(self, cpu: CpuId) -> bool {
        unsafe { ghostos_task_cpu_mask_contains(self.0[0], self.0[1], cpu.raw()) }
    }

    pub fn union(self, other: Self) -> Self {
        Self([
            unsafe { ghostos_task_cpu_mask_union(self.0[0], other.0[0]) },
            unsafe { ghostos_task_cpu_mask_union(self.0[1], other.0[1]) },
        ])
    }

    pub fn difference(self, other: Self) -> Self {
        Self([
            unsafe { ghostos_task_cpu_mask_difference(self.0[0], other.0[0]) },
            unsafe { ghostos_task_cpu_mask_difference(self.0[1], other.0[1]) },
        ])
    }

    pub fn intersects(self, other: Self) -> bool {
        unsafe {
            ghostos_task_cpu_mask_intersects(
                self.0[0],
                self.0[1],
                other.0[0],
                other.0[1],
            )
        }
    }

    pub fn from_cpu(cpu: CpuId) -> Self {
        let mut words = [0; 2];
        unsafe { ghostos_task_cpu_mask_from_cpu(cpu.raw(), words.as_mut_ptr()) };
        Self(words)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct AddressSpaceId(u32);

impl AddressSpaceId {
    pub const KERNEL: Self = Self(0);

    pub fn new(raw: u32) -> Option<Self> {
        if unsafe { ghostos_task_address_space_id_valid(raw) } {
            Some(Self(raw))
        } else {
            None
        }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ThreadId(u32);

impl ThreadId {
    pub(crate) fn from_parts(slot: usize, generation: u16) -> Self {
        Self(unsafe { ghostos_task_thread_id_from_parts(slot, generation) })
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub fn new(raw: u32) -> Option<Self> {
        if unsafe { ghostos_task_thread_id_valid(raw) } {
            Some(Self(raw))
        } else {
            None
        }
    }

    pub(crate) fn slot(self) -> usize {
        unsafe { ghostos_task_thread_slot(self.0) }
    }

    pub(crate) fn generation(self) -> u16 {
        unsafe { ghostos_task_thread_generation(self.0) }
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
    pub flags: usize,
    /// General registers in ABI order: rax, rbx, rcx, rdx, rsi, rdi, rbp,
    /// r8-r15, then one reserved slot. Other architectures use the same
    /// stable register slots for their saved machine state.
    pub registers: [usize; 16],
    /// Kept as a compact view for crash reporting and architecture code that
    /// only needs callee-preserved registers.
    pub callee_saved: [usize; 12],
}

impl Context {
    pub const fn new(instruction_pointer: usize, stack_pointer: usize) -> Self {
        Self {
            instruction_pointer,
            stack_pointer,
            flags: 0x202,
            registers: [0; 16],
            callee_saved: [0; 12],
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Thread {
    pub id: ThreadId,
    pub address_space: AddressSpaceId,
    pub address_space_root: Option<crate::PageTableRoot>,
    pub mode: ExecutionMode,
    pub state: ThreadState,
    pub policy: SchedulingPolicy,
    pub persona: ExecutionPersona,
    pub context: Context,
    pub affinity: CpuMask,
    pub numa_node: u8,
    pub wake_at: u64,
    pub switches: u64,
    pub inherited_priority: u8,
    pub inherited_deadline: u64,
}

impl Thread {
    pub(crate) const VACANT: Self = Self {
        id: ThreadId(0),
        address_space: AddressSpaceId::KERNEL,
        address_space_root: None,
        mode: ExecutionMode::Kernel,
        state: ThreadState::Vacant,
        policy: SchedulingPolicy::Cooperative,
        persona: ExecutionPersona::anonymous(),
        context: Context::new(0, 0),
        affinity: CpuMask::EMPTY,
        numa_node: 0,
        wake_at: 0,
        switches: 0,
        inherited_priority: 0,
        inherited_deadline: u64::MAX,
    };
}
