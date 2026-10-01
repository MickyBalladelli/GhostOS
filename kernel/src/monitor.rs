use crate::scheduler::Scheduler;
use crate::task::{AddressSpaceId, ThreadId, ThreadState, SchedulingPolicy};
use crate::dlm::{KernelDlm, ResourceId, DEFAULT_LOCK_CAPACITY, DEFAULT_NODE_FENCE_CAPACITY};
use core::ffi::{c_char, c_void};
use core::slice;
use core::str;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CMonitorThread {
    thread_id: u32,
    slot: u8,
    state: u8,
    policy: u8,
    switches: u64,
    owner: u64,
    address_space: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CMonitorHistory {
    prev_switches: u64,
    samples: [u8; 10],
    sample_idx: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CMonitorProcess {
    thread_id: u32,
    state: u8,
    policy: u8,
    switches: u64,
    address_space: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CMonitorCpu {
    thread_id: u32,
    switches: u64,
    owner: u64,
    address_space: u32,
    state: u8,
    policy: u8,
    util_percent: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CMonitorLock {
    resource_id: u64,
    owner_node: u32,
    granted: bool,
    occupied: bool,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CMonitorLockContention {
    has_resource: bool,
    resource_id: u64,
    granted: u8,
    queued: u8,
    owner_node: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CMonitorDsmStats {
    remote_faults: u64,
    local_faults: u64,
    latency_us: u16,
}

unsafe extern "C" {
    fn ghostos_monitor_get_processes(threads: *const CMonitorThread, thread_count: usize,
        result: *mut CMonitorProcess) -> usize;
    fn ghostos_monitor_get_top_cpu(threads: *const CMonitorThread, thread_count: usize,
        history: *mut CMonitorHistory, result: *mut CMonitorCpu) -> usize;
    fn ghostos_monitor_get_lock_contentions(locks: *const CMonitorLock, lock_count: usize,
        result: *mut CMonitorLockContention);
    fn ghostos_monitor_get_dsm_stats(result: *mut CMonitorDsmStats);
    fn ghostos_monitor_render_processes(threads: *const CMonitorThread, thread_count: usize,
        write: extern "C" fn(*mut c_void, *const c_char, usize), context: *mut c_void);
    fn ghostos_monitor_render_top_cpu(threads: *const CMonitorThread, thread_count: usize,
        history: *mut CMonitorHistory, write: extern "C" fn(*mut c_void, *const c_char, usize), context: *mut c_void);
    fn ghostos_monitor_render_dsm(locks: *const CMonitorLock, lock_count: usize,
        write: extern "C" fn(*mut c_void, *const c_char, usize), context: *mut c_void);
    fn ghostos_monitor_render_memory(available_bytes: u64, used_bytes: u64,
        write: extern "C" fn(*mut c_void, *const c_char, usize), context: *mut c_void);
}

const _: [(); 32] = [(); core::mem::size_of::<CMonitorThread>()];
const _: [(); 32] = [(); core::mem::size_of::<CMonitorHistory>()];
const _: [(); 24] = [(); core::mem::size_of::<CMonitorProcess>()];
const _: [(); 32] = [(); core::mem::size_of::<CMonitorCpu>()];
const _: [(); 16] = [(); core::mem::size_of::<CMonitorLock>()];
const _: [(); 24] = [(); core::mem::size_of::<CMonitorLockContention>()];
const _: [(); 24] = [(); core::mem::size_of::<CMonitorDsmStats>()];

struct FormatWriter<'a>(&'a mut dyn core::fmt::Write);

extern "C" fn write_callback(context: *mut c_void, bytes: *const c_char, length: usize) {
    if context.is_null() || bytes.is_null() { return; }
    let writer = unsafe { &mut *(context.cast::<FormatWriter<'_>>()) };
    let bytes = unsafe { slice::from_raw_parts(bytes.cast::<u8>(), length) };
    if let Ok(text) = str::from_utf8(bytes) { let _ = writer.0.write_str(text); }
}

pub const MAX_LOCKS: usize = DEFAULT_LOCK_CAPACITY;
pub const MAX_NODES: usize = DEFAULT_NODE_FENCE_CAPACITY;
const MAX_THREADS: usize = 64;

fn scheduler_snapshot(scheduler: &Scheduler) -> [CMonitorThread; MAX_THREADS] {
    let mut threads = [CMonitorThread::default(); MAX_THREADS];
    for (slot, view) in threads.iter_mut().enumerate() {
        view.thread_id = ThreadId::from_parts(slot, 1).raw();
        view.slot = slot as u8;
        if let Ok(thread) = scheduler.thread(ThreadId::from_parts(slot, 1)) {
            view.state = match thread.state {
                ThreadState::Vacant => 0,
                ThreadState::Ready => 1,
                ThreadState::Running => 2,
                ThreadState::Blocked => 3,
                ThreadState::Sleeping => 4,
            };
            view.policy = match thread.policy {
                SchedulingPolicy::Cooperative => 0,
                SchedulingPolicy::Realtime { .. } => 1,
            };
            view.switches = thread.switches;
            view.owner = thread.persona.identity().raw();
            view.address_space = thread.address_space.raw();
        }
    }
    threads
}

fn thread_id(raw: u32) -> ThreadId {
    ThreadId::new(raw).expect("C monitor returned a scheduler thread id")
}

fn state(raw: u8) -> ThreadState {
    match raw {
        0 => ThreadState::Vacant,
        1 => ThreadState::Ready,
        2 => ThreadState::Running,
        3 => ThreadState::Blocked,
        _ => ThreadState::Sleeping,
    }
}

fn policy(raw: u8) -> SchedulingPolicy {
    if raw == 1 { SchedulingPolicy::Realtime { priority: 0, deadline: 0 } }
    else { SchedulingPolicy::Cooperative }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MonitorView {
    Processes,
    TopCpu,
    Dsm,
    Memory,
}

#[derive(Clone, Copy)]
pub struct CpuUsage {
    pub thread_id: ThreadId,
    pub switches: u64,
    pub owner: u64,
    pub address_space: AddressSpaceId,
    pub state: ThreadState,
    pub policy: SchedulingPolicy,
    pub util_percent: u8,
}

#[derive(Clone, Copy)]
pub struct ProcessInfo {
    pub thread_id: ThreadId,
    pub state: ThreadState,
    pub switches: u64,
    pub address_space: AddressSpaceId,
    pub policy: SchedulingPolicy,
}

#[derive(Clone, Copy)]
pub struct LockContention {
    pub resource_id: Option<ResourceId>,
    pub granted: u8,
    pub queued: u8,
    pub owner_node: u32,
}

#[derive(Clone, Copy, Default)]
pub struct DsmPageStats {
    pub remote_faults: u64,
    pub local_faults: u64,
    pub latency_us: u16,
}

#[derive(Clone, Copy)]
pub struct CpuHistoryEntry {
    pub prev_switches: u64,
    pub samples: [u8; 10],
    pub sample_idx: usize,
}

impl CpuHistoryEntry {
    const fn new() -> Self {
        Self {
            prev_switches: 0,
            samples: [0; 10],
            sample_idx: 0,
        }
    }
}

impl Default for CpuHistoryEntry {
    fn default() -> Self {
        Self::new()
    }
}

pub struct MonitorState {
    view: MonitorView,
    pub cpu_history: [CpuHistoryEntry; MAX_THREADS],
    refresh_counter: u64,
}

impl MonitorState {
    pub const fn new() -> Self {
        Self {
            view: MonitorView::Processes,
            cpu_history: [CpuHistoryEntry::new(); MAX_THREADS],
            refresh_counter: 0,
        }
    }

    pub fn switch_view(&mut self) {
        self.view = match self.view {
            MonitorView::Processes => MonitorView::TopCpu,
            MonitorView::TopCpu => MonitorView::Dsm,
            MonitorView::Dsm => MonitorView::Memory,
            MonitorView::Memory => MonitorView::Processes,
        };
    }

    pub fn current_view(&self) -> MonitorView {
        self.view
    }

    pub fn update(&mut self) {
        self.refresh_counter = self.refresh_counter.wrapping_add(1);
    }

    pub fn get_processes(scheduler: &Scheduler) -> [Option<ProcessInfo>; 16] {
        let threads = scheduler_snapshot(scheduler);
        let mut c_result = [CMonitorProcess::default(); 16];
        let count = unsafe { ghostos_monitor_get_processes(threads.as_ptr(), threads.len(), c_result.as_mut_ptr()) };
        let mut result = [None; 16];
        for (index, item) in c_result.iter().take(count).enumerate() {
            result[index] = Some(ProcessInfo {
                thread_id: thread_id(item.thread_id),
                state: state(item.state),
                switches: item.switches,
                address_space: AddressSpaceId::new(item.address_space).unwrap_or(AddressSpaceId::KERNEL),
                policy: policy(item.policy),
            });
        }
        result
    }

    pub fn get_top_cpu(
        scheduler: &Scheduler,
        history: &mut [CpuHistoryEntry; MAX_THREADS],
    ) -> [Option<CpuUsage>; 8] {
        let threads = scheduler_snapshot(scheduler);
        let mut c_history = [CMonitorHistory::default(); MAX_THREADS];
        for (target, source) in c_history.iter_mut().zip(history.iter()) {
            *target = CMonitorHistory { prev_switches: source.prev_switches,
                samples: source.samples, sample_idx: source.sample_idx };
        }
        let mut c_result = [CMonitorCpu::default(); 8];
        let count = unsafe { ghostos_monitor_get_top_cpu(threads.as_ptr(), threads.len(),
            c_history.as_mut_ptr(), c_result.as_mut_ptr()) };
        for (target, source) in history.iter_mut().zip(c_history.iter()) {
            target.prev_switches = source.prev_switches;
            target.samples = source.samples;
            target.sample_idx = source.sample_idx;
        }
        let mut result = [None; 8];
        for (index, item) in c_result.iter().take(count).enumerate() {
            result[index] = Some(CpuUsage { thread_id: thread_id(item.thread_id),
                switches: item.switches, owner: item.owner,
                address_space: AddressSpaceId::new(item.address_space).unwrap_or(AddressSpaceId::KERNEL),
                state: state(item.state), policy: policy(item.policy), util_percent: item.util_percent });
        }
        result
    }

    pub(crate) fn get_lock_contentions(dlm: &KernelDlm) -> [LockContention; 8] {
        let mut c_locks = [CMonitorLock::default(); MAX_LOCKS];
        for i in 0..MAX_LOCKS {
            if let Some(entry) = dlm.lock_summary(i) {
                c_locks[i] = CMonitorLock { resource_id: entry.resource.raw(),
                    owner_node: entry.owner.node.raw(), granted: entry.granted, occupied: true };
            }
        }
        let mut c_result = [CMonitorLockContention::default(); 8];
        unsafe { ghostos_monitor_get_lock_contentions(c_locks.as_ptr(), c_locks.len(), c_result.as_mut_ptr()) };
        let mut result = [LockContention { resource_id: None, granted: 0, queued: 0, owner_node: 0 }; 8];
        for (target, source) in result.iter_mut().zip(c_result.iter()) {
            if source.has_resource {
                target.resource_id = ResourceId::new(source.resource_id);
                target.granted = source.granted;
                target.queued = source.queued;
                target.owner_node = source.owner_node;
            }
        }
        result
    }

    pub fn get_dsm_stats() -> [DsmPageStats; 8] {
        let mut c_result = [CMonitorDsmStats::default(); 8];
        unsafe { ghostos_monitor_get_dsm_stats(c_result.as_mut_ptr()) };
        c_result.map(|stats| DsmPageStats { remote_faults: stats.remote_faults,
            local_faults: stats.local_faults, latency_us: stats.latency_us })
    }

    pub fn render_processes(scheduler: &Scheduler, output: &mut dyn core::fmt::Write) {
        let threads = scheduler_snapshot(scheduler);
        let mut writer = FormatWriter(output);
        unsafe { ghostos_monitor_render_processes(threads.as_ptr(), threads.len(), write_callback,
            (&mut writer as *mut FormatWriter<'_>).cast()) };
    }

    pub fn render_top_cpu(scheduler: &Scheduler, history: &mut [CpuHistoryEntry; MAX_THREADS], output: &mut dyn core::fmt::Write) {
        let threads = scheduler_snapshot(scheduler);
        let mut c_history = [CMonitorHistory::default(); MAX_THREADS];
        for (target, source) in c_history.iter_mut().zip(history.iter()) {
            *target = CMonitorHistory { prev_switches: source.prev_switches,
                samples: source.samples, sample_idx: source.sample_idx };
        }
        let mut writer = FormatWriter(output);
        unsafe { ghostos_monitor_render_top_cpu(threads.as_ptr(), threads.len(), c_history.as_mut_ptr(),
            write_callback, (&mut writer as *mut FormatWriter<'_>).cast()) };
        for (target, source) in history.iter_mut().zip(c_history.iter()) {
            target.prev_switches = source.prev_switches;
            target.samples = source.samples;
            target.sample_idx = source.sample_idx;
        }
    }

    pub(crate) fn render_dsm(dlm: &KernelDlm, output: &mut dyn core::fmt::Write) {
        let mut c_locks = [CMonitorLock::default(); MAX_LOCKS];
        for (index, lock) in c_locks.iter_mut().enumerate() {
            if let Some(entry) = dlm.lock_summary(index) {
                *lock = CMonitorLock { resource_id: entry.resource.raw(),
                    owner_node: entry.owner.node.raw(), granted: entry.granted, occupied: true };
            }
        }
        let mut writer = FormatWriter(output);
        unsafe { ghostos_monitor_render_dsm(c_locks.as_ptr(), c_locks.len(), write_callback,
            (&mut writer as *mut FormatWriter<'_>).cast()) };
    }

    pub fn render_memory(
        available_bytes: u64,
        used_bytes: u64,
        output: &mut dyn core::fmt::Write,
    ) {
        let mut writer = FormatWriter(output);
        unsafe { ghostos_monitor_render_memory(available_bytes, used_bytes, write_callback,
            (&mut writer as *mut FormatWriter<'_>).cast()) };
    }
}
