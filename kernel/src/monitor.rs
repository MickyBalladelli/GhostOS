use crate::scheduler::Scheduler;
use crate::task::{AddressSpaceId, ThreadId, ThreadState, SchedulingPolicy};
use crate::dlm::{KernelDlm, ResourceId, DEFAULT_LOCK_CAPACITY, DEFAULT_NODE_FENCE_CAPACITY};

pub const MAX_LOCKS: usize = DEFAULT_LOCK_CAPACITY;
pub const MAX_NODES: usize = DEFAULT_NODE_FENCE_CAPACITY;
const MAX_THREADS: usize = 64;

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
        let mut result: [Option<ProcessInfo>; 16] = [None; 16];
        let mut count = 0;
        
        for i in 0..MAX_THREADS {
            if let Ok(thread) = scheduler.thread(ThreadId::from_parts(i, 1)) {
                if thread.state != ThreadState::Vacant && count < 16 {
                    result[count] = Some(ProcessInfo {
                        thread_id: thread.id,
                        state: thread.state,
                        switches: thread.switches,
                        address_space: thread.address_space,
                        policy: thread.policy,
                    });
                    count += 1;
                }
            }
        }
        result
    }

    fn sort_by_key(
        arr: &mut [(ThreadId, u64, u64)],
        key_fn: impl Fn(&(ThreadId, u64, u64)) -> u64,
    ) {
        for i in 0..arr.len() {
            for j in (i + 1)..arr.len() {
                if key_fn(&arr[j]) > key_fn(&arr[i]) {
                    arr.swap(i, j);
                }
            }
        }
    }

    pub fn get_top_cpu(
        scheduler: &Scheduler,
        history: &mut [CpuHistoryEntry; MAX_THREADS],
    ) -> [Option<CpuUsage>; 8] {
        let mut cpu_threads: [(ThreadId, u64, u64); MAX_THREADS] =
            [(ThreadId::from_parts(0, 0), 0, 0); MAX_THREADS];
        let mut active_count = 0;
        
        for i in 0..MAX_THREADS {
            if let Ok(thread) = scheduler.thread(ThreadId::from_parts(i, 1)) {
                if thread.state != ThreadState::Vacant {
                    let history_entry = &mut history[thread.id.slot()];
                    let delta = thread.switches.saturating_sub(history_entry.prev_switches);
                    history_entry.prev_switches = thread.switches;
                    history_entry.samples[history_entry.sample_idx] = (delta.min(100)) as u8;
                    history_entry.sample_idx = (history_entry.sample_idx + 1) % 10;
                    let activity = history_entry
                        .samples
                        .iter()
                        .map(|sample| *sample as u64)
                        .sum::<u64>()
                        / 10;
                    cpu_threads[active_count] = (thread.id, thread.switches, activity);
                    active_count += 1;
                }
            }
        }
        
        Self::sort_by_key(&mut cpu_threads[..active_count], |thread| thread.2);
        let total_activity = cpu_threads[..active_count]
            .iter()
            .map(|thread| thread.2)
            .sum::<u64>();
        
        let mut result: [Option<CpuUsage>; 8] = [None; 8];
        
        for (index, (id, switches, activity)) in cpu_threads[..active_count]
            .iter()
            .take(8)
            .enumerate()
        {
            let Ok(thread) = scheduler.thread(*id) else { continue };
            let util = if total_activity == 0 {
                0
            } else {
                (activity.saturating_mul(100) / total_activity).min(100) as u8
            };
            result[index] = Some(CpuUsage {
                thread_id: *id,
                switches: *switches,
                owner: thread.persona.identity().raw(),
                address_space: thread.address_space,
                state: thread.state,
                policy: thread.policy,
                util_percent: util,
            });
        }
        
        result
    }

    pub(crate) fn get_lock_contentions(dlm: &KernelDlm) -> [LockContention; 8] {
        let mut result: [LockContention; 8] = [LockContention {
            resource_id: None,
            granted: 0,
            queued: 0,
            owner_node: 0,
        }; 8];
        
        for i in 0..MAX_LOCKS {
            if let Some(entry) = dlm.lock_summary(i) {
                result[i % 8] = LockContention {
                    resource_id: Some(entry.resource),
                    granted: u8::from(entry.granted),
                    queued: u8::from(!entry.granted),
                    owner_node: entry.owner.node.raw(),
                };
            }
        }
        
        result
    }

    pub fn get_dsm_stats() -> [DsmPageStats; 8] {
        [DsmPageStats::default(); 8]
    }

    pub fn render_processes(scheduler: &Scheduler, output: &mut dyn core::fmt::Write) {
        let _ = writeln!(output, "\x1b[1;36m=== PROCESSES ===\x1b[0m");
        let _ = writeln!(output, "{:<12} {:<10} {:<8} {:<8} {:<10}", 
            "THREAD", "STATE", "SWITCHES", "SPACE", "POLICY");
        
        let processes = Self::get_processes(scheduler);
        for proc in processes.iter() {
            if let Some(p) = proc {
                let state_str = match p.state {
                    ThreadState::Vacant => "VACANT",
                    ThreadState::Ready => "READY",
                    ThreadState::Running => "RUNNING",
                    ThreadState::Blocked => "BLOCKED",
                    ThreadState::Sleeping => "SLEEPING",
                };
                let policy_str = match p.policy {
                    SchedulingPolicy::Cooperative => "COOP",
                    SchedulingPolicy::Realtime { priority: _, .. } => "RT",
                };
                let _ = writeln!(output, "{:<12} {:<10} {:<8} {:<8} {:<10}",
                    write_thread_id(p.thread_id),
                    state_str,
                    p.switches,
                    write_addr_space(p.address_space),
                    policy_str
                );
            }
        }
    }

    pub fn render_top_cpu(scheduler: &Scheduler, history: &mut [CpuHistoryEntry; MAX_THREADS], output: &mut dyn core::fmt::Write) {
        let _ = writeln!(output, "\x1b[1;32m=== TOP CPU ===\x1b[0m");
        let _ = writeln!(output, "{:<12} {:<10} {:<10}", 
            "THREAD", "SWITCHES", "UTIL%");
        
        let top = Self::get_top_cpu(scheduler, history);
        for cpu in top.iter().flatten() {
            let _ = writeln!(output, "{:<12} {:<10} {:<3}%",
                write_thread_id(cpu.thread_id),
                cpu.switches,
                cpu.util_percent
            );
        }
    }

    pub(crate) fn render_dsm(dlm: &KernelDlm, output: &mut dyn core::fmt::Write) {
        let _ = writeln!(output, "\x1b[1;33m=== DISTRIBUTED SHARED MEMORY (DSM) LOCKS ===\x1b[0m");
        let _ = writeln!(output, "{:<12} {:<8} {:<8} {:<10}", 
            "RESOURCE", "GRANTED", "QUEUED", "OWNER");
        
        let locks = Self::get_lock_contentions(dlm);
        for lock in locks.iter() {
            let _ = writeln!(output, "{:<12} {:<8} {:<8} {:<10}",
                lock.resource_id.map(write_resource_id).unwrap_or("----"),
                lock.granted,
                lock.queued,
                lock.owner_node
            );
        }
    }

    pub fn render_memory(
        available_bytes: u64,
        used_bytes: u64,
        output: &mut dyn core::fmt::Write,
    ) {
        let _ = writeln!(output, "\x1b[1;34m=== MEMORY ===\x1b[0m");
        let _ = writeln!(output, "Available: {} bytes", available_bytes);
        let _ = writeln!(output, "Used:      {} bytes", used_bytes);
    }
}

#[inline]
fn write_thread_id(id: ThreadId) -> &'static str {
    match id.raw() {
        0 => "0000",
        1 => "0001",
        2 => "0002",
        3 => "0003",
        4 => "0004",
        5 => "0005",
        6 => "0006",
        7 => "0007",
        8 => "0008",
        9 => "0009",
        10 => "000a",
        11 => "000b",
        12 => "000c",
        13 => "000d",
        14 => "000e",
        15 => "000f",
        16 => "0010",
        17 => "0011",
        18 => "0012",
        19 => "0013",
        20 => "0014",
        21 => "0015",
        22 => "0016",
        23 => "0017",
        24 => "0018",
        25 => "0019",
        26 => "001a",
        27 => "001b",
        28 => "001c",
        29 => "001d",
        30 => "001e",
        31 => "001f",
        32 => "0020",
        33 => "0021",
        34 => "0022",
        35 => "0023",
        36 => "0024",
        37 => "0025",
        38 => "0026",
        39 => "0027",
        40 => "0028",
        41 => "0029",
        42 => "002a",
        43 => "002b",
        44 => "002c",
        45 => "002d",
        46 => "002e",
        47 => "002f",
        48 => "0030",
        49 => "0031",
        50 => "0032",
        51 => "0033",
        52 => "0034",
        53 => "0035",
        54 => "0036",
        55 => "0037",
        56 => "0038",
        57 => "0039",
        58 => "003a",
        59 => "003b",
        60 => "003c",
        61 => "003d",
        62 => "003e",
        63 => "003f",
        _ => "????",
    }
}

#[inline]
fn write_addr_space(as_id: AddressSpaceId) -> &'static str {
    match as_id.raw() {
        0 => "0x00",
        1 => "0x01",
        2 => "0x02",
        3 => "0x03",
        4 => "0x04",
        5 => "0x05",
        6 => "0x06",
        7 => "0x07",
        8 => "0x08",
        9 => "0x09",
        10 => "0x0a",
        11 => "0x0b",
        12 => "0x0c",
        13 => "0x0d",
        14 => "0x0e",
        15 => "0x0f",
        _ => "0x??",
    }
}

#[inline]
fn write_resource_id(id: ResourceId) -> &'static str {
    match id.raw() {
        0 => "0000",
        1 => "0001",
        2 => "0002",
        3 => "0003",
        4 => "0004",
        5 => "0005",
        6 => "0006",
        7 => "0007",
        8 => "0008",
        9 => "0009",
        10 => "000a",
        11 => "000b",
        12 => "000c",
        13 => "000d",
        14 => "000e",
        15 => "000f",
        16 => "0010",
        17 => "0011",
        18 => "0012",
        19 => "0013",
        20 => "0014",
        21 => "0015",
        22 => "0016",
        23 => "0017",
        24 => "0018",
        25 => "0019",
        26 => "001a",
        27 => "001b",
        28 => "001c",
        29 => "001d",
        30 => "001e",
        31 => "001f",
        32 => "0020",
        33 => "0021",
        34 => "0022",
        35 => "0023",
        36 => "0024",
        37 => "0025",
        38 => "0026",
        39 => "0027",
        40 => "0028",
        41 => "0029",
        42 => "002a",
        43 => "002b",
        44 => "002c",
        45 => "002d",
        46 => "002e",
        47 => "002f",
        48 => "0030",
        49 => "0031",
        50 => "0032",
        51 => "0033",
        52 => "0034",
        53 => "0035",
        54 => "0036",
        55 => "0037",
        56 => "0038",
        57 => "0039",
        58 => "003a",
        59 => "003b",
        60 => "003c",
        61 => "003d",
        62 => "003e",
        63 => "003f",
        64 => "0040",
        65 => "0041",
        66 => "0042",
        67 => "0043",
        68 => "0044",
        69 => "0045",
        70 => "0046",
        71 => "0047",
        72 => "0048",
        73 => "0049",
        74 => "004a",
        75 => "004b",
        76 => "004c",
        77 => "004d",
        78 => "004e",
        79 => "004f",
        80 => "0050",
        81 => "0051",
        82 => "0052",
        83 => "0053",
        84 => "0054",
        85 => "0055",
        86 => "0056",
        87 => "0057",
        88 => "0058",
        89 => "0059",
        90 => "005a",
        91 => "005b",
        92 => "005c",
        93 => "005d",
        94 => "005e",
        95 => "005f",
        96 => "0060",
        97 => "0061",
        98 => "0062",
        99 => "0063",
        100 => "0064",
        101 => "0065",
        102 => "0066",
        103 => "0067",
        104 => "0068",
        105 => "0069",
        106 => "006a",
        107 => "006b",
        108 => "006c",
        109 => "006d",
        110 => "006e",
        111 => "006f",
        112 => "0070",
        113 => "0071",
        114 => "0072",
        115 => "0073",
        116 => "0074",
        117 => "0075",
        118 => "0076",
        119 => "0077",
        120 => "0078",
        121 => "0079",
        122 => "007a",
        123 => "007b",
        124 => "007c",
        125 => "007d",
        126 => "007e",
        127 => "007f",
        128 => "0080",
        129 => "0081",
        130 => "0082",
        131 => "0083",
        132 => "0084",
        133 => "0085",
        134 => "0086",
        135 => "0087",
        136 => "0088",
        137 => "0089",
        138 => "008a",
        139 => "008b",
        140 => "008c",
        141 => "008d",
        142 => "008e",
        143 => "008f",
        144 => "0090",
        145 => "0091",
        146 => "0092",
        147 => "0093",
        148 => "0094",
        149 => "0095",
        150 => "0096",
        151 => "0097",
        152 => "0098",
        153 => "0099",
        154 => "009a",
        155 => "009b",
        156 => "009c",
        157 => "009d",
        158 => "009e",
        159 => "009f",
        160 => "00a0",
        161 => "00a1",
        162 => "00a2",
        163 => "00a3",
        164 => "00a4",
        165 => "00a5",
        166 => "00a6",
        167 => "00a7",
        168 => "00a8",
        169 => "00a9",
        170 => "00aa",
        171 => "00ab",
        172 => "00ac",
        173 => "00ad",
        174 => "00ae",
        175 => "00af",
        176 => "00b0",
        177 => "00b1",
        178 => "00b2",
        179 => "00b3",
        180 => "00b4",
        181 => "00b5",
        182 => "00b6",
        183 => "00b7",
        184 => "00b8",
        185 => "00b9",
        186 => "00ba",
        187 => "00bb",
        188 => "00bc",
        189 => "00bd",
        190 => "00be",
        191 => "00bf",
        192 => "00c0",
        193 => "00c1",
        194 => "00c2",
        195 => "00c3",
        196 => "00c4",
        197 => "00c5",
        198 => "00c6",
        199 => "00c7",
        200 => "00c8",
        201 => "00c9",
        202 => "00ca",
        203 => "00cb",
        204 => "00cc",
        205 => "00cd",
        206 => "00ce",
        207 => "00cf",
        208 => "00d0",
        209 => "00d1",
        210 => "00d2",
        211 => "00d3",
        212 => "00d4",
        213 => "00d5",
        214 => "00d6",
        215 => "00d7",
        216 => "00d8",
        217 => "00d9",
        218 => "00da",
        219 => "00db",
        220 => "00dc",
        221 => "00dd",
        222 => "00de",
        223 => "00df",
        _ => "????",
    }
}
