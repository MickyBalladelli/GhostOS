//! Kernel-owned liveness tracking for services and CPUs.
//!
//! Heartbeats are written with atomics so a stalled service or CPU cannot hold
//! a lock needed by the watchdog. Recovery is latched once per fault and is
//! cleared only after the owner reports progress again.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::task::{CpuId, CpuMask, MAX_CPUS};

pub const SERVICE_CAPACITY: usize = 14;
pub const SERVICE_TIMEOUT_US: u64 = 5_000_000;
pub const CPU_TIMEOUT_US: u64 = 100_000;

static SERVICE_READY: AtomicU32 = AtomicU32::new(0);
static SERVICE_HEARTBEATS: [AtomicU64; SERVICE_CAPACITY] =
    [const { AtomicU64::new(0) }; SERVICE_CAPACITY];
static SERVICE_SEQUENCES: [AtomicU64; SERVICE_CAPACITY] =
    [const { AtomicU64::new(0) }; SERVICE_CAPACITY];
static SERVICE_LATCHED: AtomicU32 = AtomicU32::new(0);

static CPU_ONLINE: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static CPU_HEARTBEATS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static CPU_LATCHED: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WatchdogReport {
    pub stale_services: u32,
    pub stale_cpus: CpuMask,
}

impl WatchdogReport {
    pub const NONE: Self = Self {
        stale_services: 0,
        stale_cpus: CpuMask::EMPTY,
    };

    pub const fn has_fault(self) -> bool {
        self.stale_services != 0 || !self.stale_cpus.is_empty()
    }
}

pub(crate) fn service_ready(role: usize, now_us: u64) {
    let Some(slot) = SERVICE_HEARTBEATS.get(role) else { return };
    SERVICE_READY.fetch_or(1u32 << role, Ordering::Release);
    if slot.load(Ordering::Acquire) == 0 {
        slot.store(now_us, Ordering::Release);
    }
}

pub(crate) fn service_heartbeat(role: usize, sequence: u64, now_us: u64) {
    let Some(heartbeat) = SERVICE_HEARTBEATS.get(role) else { return };
    let Some(sequence_slot) = SERVICE_SEQUENCES.get(role) else { return };
    if sequence == 0 || SERVICE_READY.load(Ordering::Acquire) & (1u32 << role) == 0 {
        return
    }
    sequence_slot.store(sequence, Ordering::Relaxed);
    heartbeat.store(now_us, Ordering::Release);
    SERVICE_LATCHED.fetch_and(!(1u32 << role), Ordering::Release);
}

pub(crate) fn service_sequences() -> impl Iterator<Item = u64> {
    SERVICE_SEQUENCES
        .iter()
        .map(|sequence| sequence.load(Ordering::Acquire))
}

pub(crate) fn cpu_online(cpu: CpuId, now_us: u64) {
    let raw = cpu.raw() as usize;
    let Some(heartbeat) = CPU_HEARTBEATS.get(raw) else { return };
    CPU_ONLINE[raw / 64].fetch_or(1u64 << (raw % 64), Ordering::Release);
    heartbeat.store(now_us, Ordering::Release);
}

pub(crate) fn cpu_tick(cpu: CpuId, now_us: u64) {
    let raw = cpu.raw() as usize;
    let Some(heartbeat) = CPU_HEARTBEATS.get(raw) else { return };
    heartbeat.store(now_us, Ordering::Release);
    CPU_LATCHED[raw / 64].fetch_and(!(1u64 << (raw % 64)), Ordering::Release);
}

pub(crate) fn cpu_offline(cpu: CpuId) {
    let raw = cpu.raw() as usize;
    if raw >= MAX_CPUS {
        return
    }
    CPU_ONLINE[raw / 64].fetch_and(!(1u64 << (raw % 64)), Ordering::Release);
}

pub(crate) fn poll(now_us: u64) -> WatchdogReport {
    let ready = SERVICE_READY.load(Ordering::Acquire);
    let mut stale_services = 0;
    for role in 1..SERVICE_CAPACITY {
        let bit = 1u32 << role;
        let Some(heartbeat) = SERVICE_HEARTBEATS.get(role) else { continue };
        let last = heartbeat.load(Ordering::Acquire);
        if ready & bit != 0
            && last != 0
            && now_us.saturating_sub(last) > SERVICE_TIMEOUT_US
            && SERVICE_LATCHED.fetch_or(bit, Ordering::AcqRel) & bit == 0
        {
            stale_services |= bit;
        }
    }

    let mut stale_cpus = CpuMask::EMPTY;
    for raw in 0..MAX_CPUS {
        let word = raw / 64;
        let bit = 1u64 << (raw % 64);
        if CPU_ONLINE[word].load(Ordering::Acquire) & bit == 0 {
            continue
        }
        let Some(heartbeat) = CPU_HEARTBEATS.get(raw) else { continue };
        let last = heartbeat.load(Ordering::Acquire);
        if last != 0
            && now_us.saturating_sub(last) > CPU_TIMEOUT_US
            && CPU_LATCHED[word].fetch_or(bit, Ordering::AcqRel) & bit == 0
        {
            if let Some(cpu) = CpuId::new(raw as u8) {
                stale_cpus = stale_cpus.union(CpuMask::from_cpu(cpu));
            }
        }
    }

    WatchdogReport {
        stale_services,
        stale_cpus,
    }
}
