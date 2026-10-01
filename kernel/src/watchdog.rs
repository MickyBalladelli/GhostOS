use crate::task::{CpuId, CpuMask, MAX_CPUS};

pub const SERVICE_CAPACITY: usize = 15;
pub const SERVICE_TIMEOUT_US: u64 = 5_000_000;
pub const CPU_TIMEOUT_US: u64 = 100_000;

#[repr(C)]
struct CWatchdogReport {
    stale_services: u32,
    stale_cpus: [u64; 2],
}

unsafe extern "C" {
    fn ghostos_watchdog_init();
    fn ghostos_watchdog_service_ready(role: usize, now_us: u64);
    fn ghostos_watchdog_service_heartbeat(role: usize, sequence: u64, now_us: u64);
    fn ghostos_watchdog_service_activity(role: usize, now_us: u64);
    fn ghostos_watchdog_service_sequence(role: usize) -> u64;
    fn ghostos_watchdog_diagnostics_enabled() -> bool;
    fn ghostos_watchdog_set_diagnostics_enabled(enabled: bool);
    fn ghostos_watchdog_cpu_online(cpu: u8, now_us: u64);
    fn ghostos_watchdog_cpu_tick(cpu: u8, now_us: u64);
    fn ghostos_watchdog_cpu_offline(cpu: u8);
    fn ghostos_watchdog_poll(now_us: u64) -> CWatchdogReport;
}

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

pub(crate) fn init() {
    unsafe { ghostos_watchdog_init() }
}

pub(crate) fn service_ready(role: usize, now_us: u64) {
    unsafe { ghostos_watchdog_service_ready(role, now_us) }
}

pub(crate) fn service_heartbeat(role: usize, sequence: u64, now_us: u64) {
    unsafe { ghostos_watchdog_service_heartbeat(role, sequence, now_us) }
}

pub(crate) fn service_activity(role: usize, now_us: u64) {
    unsafe { ghostos_watchdog_service_activity(role, now_us) }
}

pub(crate) fn service_sequences() -> impl Iterator<Item = u64> {
    (0..SERVICE_CAPACITY).map(|role| unsafe { ghostos_watchdog_service_sequence(role) })
}

pub(crate) fn diagnostics_enabled() -> bool {
    unsafe { ghostos_watchdog_diagnostics_enabled() }
}

pub(crate) fn set_diagnostics_enabled(enabled: bool) {
    unsafe { ghostos_watchdog_set_diagnostics_enabled(enabled) }
}

pub(crate) fn cpu_online(cpu: CpuId, now_us: u64) {
    unsafe { ghostos_watchdog_cpu_online(cpu.raw(), now_us) }
}

pub(crate) fn cpu_tick(cpu: CpuId, now_us: u64) {
    unsafe { ghostos_watchdog_cpu_tick(cpu.raw(), now_us) }
}

pub(crate) fn cpu_offline(cpu: CpuId) {
    unsafe { ghostos_watchdog_cpu_offline(cpu.raw()) }
}

pub(crate) fn poll(now_us: u64) -> WatchdogReport {
    let report = unsafe { ghostos_watchdog_poll(now_us) };
    let mut stale_cpus = CpuMask::EMPTY;
    for raw in 0..MAX_CPUS {
        if report.stale_cpus[raw / 64] & (1u64 << (raw % 64)) != 0 {
            if let Some(cpu) = CpuId::new(raw as u8) {
                stale_cpus = stale_cpus.union(CpuMask::from_cpu(cpu));
            }
        }
    }
    WatchdogReport { stale_services: report.stale_services, stale_cpus }
}
