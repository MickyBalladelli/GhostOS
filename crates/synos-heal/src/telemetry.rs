use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use synos_observability::{EventField, EventKind, Level, TraceEvent, emit, field};
use synos_status::{IntoStatus, Status};

pub const DEFAULT_HEALTH_CAPACITY: usize = 64;
const RESERVED_SERVICE: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HealthConfig {
    pub heartbeat_timeout_us: u64,
    pub progress_timeout_us: u64,
    pub driver_timeout_us: u64,
    pub expected_memory_checksum: u64,
}

impl HealthConfig {
    pub const fn new(
        heartbeat_timeout_us: u64,
        progress_timeout_us: u64,
        driver_timeout_us: u64,
    ) -> Result<Self, HealthError> {
        if heartbeat_timeout_us == 0 || progress_timeout_us == 0 || driver_timeout_us == 0 {
            return Err(HealthError::InvalidConfig);
        }
        Ok(Self {
            heartbeat_timeout_us,
            progress_timeout_us,
            driver_timeout_us,
            expected_memory_checksum: 0,
        })
    }

    pub const fn with_memory_checksum(mut self, checksum: u64) -> Self {
        self.expected_memory_checksum = checksum;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HealthToken {
    slot: usize,
    service: u64,
    registration: u64,
}

impl HealthToken {
    pub const fn service(self) -> u64 {
        self.service
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthFault {
    Deadlock,
    DriverStall,
    MemoryCorruption,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthState {
    Unknown,
    Healthy,
    Stalled(HealthFault),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HealthReport {
    pub service: u64,
    pub state: HealthState,
    pub heartbeat_at_us: u64,
    pub progress: u64,
    pub driver_progress: u64,
    pub memory_checksum: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthError {
    Capacity,
    InvalidService,
    InvalidConfig,
    AlreadyRegistered,
    StaleToken,
}

impl IntoStatus for HealthError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::InvalidService | Self::InvalidConfig | Self::AlreadyRegistered => {
                Status::INVALID_ARGUMENT
            }
            Self::StaleToken => Status::NOT_FOUND,
        }
    }
}

struct HealthSlot {
    service: AtomicU64,
    registration: AtomicU64,
    heartbeat_seen: AtomicBool,
    heartbeat_at_us: AtomicU64,
    progress: AtomicU64,
    progress_seen: AtomicBool,
    progress_at_us: AtomicU64,
    driver_progress: AtomicU64,
    driver_seen: AtomicBool,
    driver_at_us: AtomicU64,
    expected_memory_checksum: AtomicU64,
    memory_checksum: AtomicU64,
    memory_checked: AtomicBool,
    memory_corrupt: AtomicBool,
    heartbeat_timeout_us: AtomicU64,
    progress_timeout_us: AtomicU64,
    driver_timeout_us: AtomicU64,
}

impl HealthSlot {
    const fn new() -> Self {
        Self {
            service: AtomicU64::new(0),
            registration: AtomicU64::new(0),
            heartbeat_seen: AtomicBool::new(false),
            heartbeat_at_us: AtomicU64::new(0),
            progress: AtomicU64::new(0),
            progress_seen: AtomicBool::new(false),
            progress_at_us: AtomicU64::new(0),
            driver_progress: AtomicU64::new(0),
            driver_seen: AtomicBool::new(false),
            driver_at_us: AtomicU64::new(0),
            expected_memory_checksum: AtomicU64::new(0),
            memory_checksum: AtomicU64::new(0),
            memory_checked: AtomicBool::new(false),
            memory_corrupt: AtomicBool::new(false),
            heartbeat_timeout_us: AtomicU64::new(0),
            progress_timeout_us: AtomicU64::new(0),
            driver_timeout_us: AtomicU64::new(0),
        }
    }
}

/// Lock-free, fixed-capacity health registry.
///
/// Daemons only write atomics when they report progress. The supervisor reads
/// the same atomics while checking every service, so a stalled daemon cannot
/// hold a lock needed by its own watchdog.
pub struct HealthMonitor<const CAPACITY: usize = DEFAULT_HEALTH_CAPACITY> {
    slots: [HealthSlot; CAPACITY],
}

impl<const CAPACITY: usize> HealthMonitor<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY > 0);
        Self {
            slots: [const { HealthSlot::new() }; CAPACITY],
        }
    }

    pub fn register(&self, service: u64, config: HealthConfig) -> Result<HealthToken, HealthError> {
        if service == 0 || service == RESERVED_SERVICE {
            return Err(HealthError::InvalidService);
        }
        if self.slots.iter().any(|slot| {
            let registered = slot.service.load(Ordering::Acquire);
            registered == service
        }) {
            return Err(HealthError::AlreadyRegistered);
        }

        for (slot_index, slot) in self.slots.iter().enumerate() {
            if slot
                .service
                .compare_exchange(0, RESERVED_SERVICE, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                continue;
            }
            slot.heartbeat_timeout_us
                .store(config.heartbeat_timeout_us, Ordering::Relaxed);
            slot.progress_timeout_us
                .store(config.progress_timeout_us, Ordering::Relaxed);
            slot.driver_timeout_us
                .store(config.driver_timeout_us, Ordering::Relaxed);
            let registration = slot
                .registration
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_add(1)
                .max(1);
            slot.expected_memory_checksum
                .store(config.expected_memory_checksum, Ordering::Relaxed);
            slot.heartbeat_seen.store(false, Ordering::Relaxed);
            slot.heartbeat_at_us.store(0, Ordering::Relaxed);
            slot.progress.store(0, Ordering::Relaxed);
            slot.progress_seen.store(false, Ordering::Relaxed);
            slot.progress_at_us.store(0, Ordering::Relaxed);
            slot.driver_progress.store(0, Ordering::Relaxed);
            slot.driver_seen.store(false, Ordering::Relaxed);
            slot.driver_at_us.store(0, Ordering::Relaxed);
            slot.memory_checksum.store(0, Ordering::Relaxed);
            slot.memory_checked.store(false, Ordering::Relaxed);
            slot.memory_corrupt.store(false, Ordering::Relaxed);
            slot.service.store(service, Ordering::Release);
            return Ok(HealthToken {
                slot: slot_index,
                service,
                registration,
            });
        }
        Err(HealthError::Capacity)
    }

    pub fn unregister(&self, token: HealthToken) -> Result<(), HealthError> {
        let slot = self.slot(token)?;
        slot.service.store(0, Ordering::Release);
        Ok(())
    }

    pub fn heartbeat(
        &self,
        token: HealthToken,
        now_us: u64,
        progress: u64,
    ) -> Result<(), HealthError> {
        let slot = self.slot(token)?;
        let previous = slot.progress.swap(progress, Ordering::Relaxed);
        let was_seen = slot.progress_seen.swap(true, Ordering::Relaxed);
        if previous != progress || !was_seen {
            slot.progress_at_us.store(now_us, Ordering::Relaxed);
        }
        slot.heartbeat_seen.store(true, Ordering::Relaxed);
        slot.heartbeat_at_us.store(now_us, Ordering::Release);
        Ok(())
    }

    pub fn driver_progress(
        &self,
        token: HealthToken,
        now_us: u64,
        progress: u64,
    ) -> Result<(), HealthError> {
        let slot = self.slot(token)?;
        let previous = slot.driver_progress.swap(progress, Ordering::Relaxed);
        let was_seen = slot.driver_seen.swap(true, Ordering::Relaxed);
        if previous != progress || !was_seen {
            slot.driver_at_us.store(now_us, Ordering::Relaxed);
        }
        Ok(())
    }

    pub fn memory_checksum(&self, token: HealthToken, checksum: u64) -> Result<(), HealthError> {
        let slot = self.slot(token)?;
        slot.memory_checksum.store(checksum, Ordering::Release);
        slot.memory_checked.store(true, Ordering::Release);
        Ok(())
    }

    pub fn report_memory_corruption(&self, token: HealthToken) -> Result<(), HealthError> {
        let slot = self.slot(token)?;
        slot.memory_corrupt.store(true, Ordering::Release);
        Ok(())
    }

    pub fn check(&self, token: HealthToken, now_us: u64) -> Result<HealthReport, HealthError> {
        let slot = self.slot(token)?;
        Ok(self.read_report(token.service, slot, now_us))
    }

    pub fn check_all(&self, now_us: u64, reports: &mut [Option<HealthReport>]) -> usize {
        reports.fill(None);
        let mut written = 0;
        for slot in &self.slots {
            let service = slot.service.load(Ordering::Acquire);
            if service == 0 || service == RESERVED_SERVICE || written == reports.len() {
                continue;
            }
            reports[written] = Some(self.read_report(service, slot, now_us));
            written += 1;
        }
        written
    }

    pub fn active(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| {
                let service = slot.service.load(Ordering::Acquire);
                service != 0 && service != RESERVED_SERVICE
            })
            .count()
    }

    fn slot(&self, token: HealthToken) -> Result<&HealthSlot, HealthError> {
        let slot = self.slots.get(token.slot).ok_or(HealthError::StaleToken)?;
        if slot.service.load(Ordering::Acquire) != token.service
            || slot.registration.load(Ordering::Acquire) != token.registration
        {
            return Err(HealthError::StaleToken);
        }
        Ok(slot)
    }

    fn read_report(&self, service: u64, slot: &HealthSlot, now_us: u64) -> HealthReport {
        let heartbeat_seen = slot.heartbeat_seen.load(Ordering::Acquire);
        let heartbeat_at_us = slot.heartbeat_at_us.load(Ordering::Acquire);
        let progress = slot.progress.load(Ordering::Relaxed);
        let driver_progress = slot.driver_progress.load(Ordering::Relaxed);
        let memory_checksum = slot.memory_checksum.load(Ordering::Acquire);
        let fault = if slot.memory_corrupt.load(Ordering::Acquire)
            || (slot.memory_checked.load(Ordering::Acquire)
                && slot.expected_memory_checksum.load(Ordering::Relaxed) != 0
                && memory_checksum != slot.expected_memory_checksum.load(Ordering::Relaxed))
        {
            Some(HealthFault::MemoryCorruption)
        } else if !heartbeat_seen {
            None
        } else if now_us.saturating_sub(heartbeat_at_us)
            > slot.heartbeat_timeout_us.load(Ordering::Relaxed)
        {
            Some(HealthFault::Deadlock)
        } else if slot.driver_seen.load(Ordering::Relaxed)
            && now_us.saturating_sub(slot.driver_at_us.load(Ordering::Relaxed))
                > slot.driver_timeout_us.load(Ordering::Relaxed)
        {
            Some(HealthFault::DriverStall)
        } else if slot.progress_seen.load(Ordering::Relaxed)
            && now_us.saturating_sub(slot.progress_at_us.load(Ordering::Relaxed))
                > slot.progress_timeout_us.load(Ordering::Relaxed)
        {
            Some(HealthFault::Deadlock)
        } else {
            None
        };

        let state = match (heartbeat_seen, fault) {
            (_, Some(fault)) => HealthState::Stalled(fault),
            (false, None) => HealthState::Unknown,
            (_, None) => HealthState::Healthy,
        };
        if let HealthState::Stalled(fault) = state {
            emit(
                TraceEvent::new(Level::Error, EventKind::Operator)
                    .at(now_us)
                    .with_field(EventField::unsigned(field::OBJECT, service))
                    .with_field(EventField::unsigned(field::STATUS, fault as u64))
                    .with_field(EventField::unsigned(field::OPERATION, progress)),
            );
        }
        HealthReport {
            service,
            state,
            heartbeat_at_us,
            progress,
            driver_progress,
            memory_checksum,
        }
    }
}

impl<const CAPACITY: usize> Default for HealthMonitor<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
