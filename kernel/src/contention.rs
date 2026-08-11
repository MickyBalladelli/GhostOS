use core::sync::atomic::{AtomicU64, Ordering};

pub const LOCK_DURATION_BUCKETS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LockShardReport {
    pub active_owner: u64,
    pub acquisitions: u64,
    pub contended_acquisitions: u64,
    pub spins: u64,
    pub releases: u64,
    pub duration_histogram: [u64; LOCK_DURATION_BUCKETS],
    pub max_duration: u64,
}

struct TicketLock {
    next_ticket: AtomicU64,
    serving_ticket: AtomicU64,
    owner: AtomicU64,
    acquisitions: AtomicU64,
    contended_acquisitions: AtomicU64,
    spins: AtomicU64,
    releases: AtomicU64,
    duration_histogram: [AtomicU64; LOCK_DURATION_BUCKETS],
    max_duration: AtomicU64,
}

impl TicketLock {
    const fn new() -> Self {
        Self {
            next_ticket: AtomicU64::new(0),
            serving_ticket: AtomicU64::new(0),
            owner: AtomicU64::new(0),
            acquisitions: AtomicU64::new(0),
            contended_acquisitions: AtomicU64::new(0),
            spins: AtomicU64::new(0),
            releases: AtomicU64::new(0),
            duration_histogram: [const { AtomicU64::new(0) }; LOCK_DURATION_BUCKETS],
            max_duration: AtomicU64::new(0),
        }
    }

    fn lock(&self, now: u64) -> LockGuard<'_> {
        let ticket = self.next_ticket.fetch_add(1, Ordering::Relaxed);
        let mut spins = 0u64;
        while self.serving_ticket.load(Ordering::Acquire) != ticket {
            spins = spins.saturating_add(1);
            core::hint::spin_loop()
        }

        self.acquisitions.fetch_add(1, Ordering::Relaxed);
        if spins != 0 {
            self.contended_acquisitions.fetch_add(1, Ordering::Relaxed);
            self.spins.fetch_add(spins, Ordering::Relaxed);
        }
        let owner = ticket.wrapping_add(1);
        self.owner.store(owner, Ordering::Release);
        LockGuard {
            lock: self,
            ticket,
            started_at: now,
            spins,
            released: false,
        }
    }

    fn unlock(&self, guard: &mut LockGuard<'_>, ended_at: u64) {
        let mut duration = ended_at.saturating_sub(guard.started_at);
        if duration == 0 {
            duration = guard.spins;
        }
        self.duration_histogram[duration_bucket(duration)].fetch_add(1, Ordering::Relaxed);
        self.max_duration.fetch_max(duration, Ordering::Relaxed);
        self.releases.fetch_add(1, Ordering::Relaxed);
        self.owner.store(0, Ordering::Release);
        self.serving_ticket
            .store(guard.ticket.wrapping_add(1), Ordering::Release);
        guard.released = true;
    }

    fn report(&self) -> LockShardReport {
        LockShardReport {
            active_owner: self.owner.load(Ordering::Acquire),
            acquisitions: self.acquisitions.load(Ordering::Relaxed),
            contended_acquisitions: self.contended_acquisitions.load(Ordering::Relaxed),
            spins: self.spins.load(Ordering::Relaxed),
            releases: self.releases.load(Ordering::Relaxed),
            duration_histogram: core::array::from_fn(|index| {
                self.duration_histogram[index].load(Ordering::Relaxed)
            }),
            max_duration: self.max_duration.load(Ordering::Relaxed),
        }
    }
}

pub struct LockGuard<'a> {
    lock: &'a TicketLock,
    ticket: u64,
    started_at: u64,
    spins: u64,
    released: bool,
}

impl LockGuard<'_> {
    pub fn unlock(mut self, ended_at: u64) {
        let lock = self.lock;
        lock.unlock(&mut self, ended_at)
    }
}

impl Drop for LockGuard<'_> {
    fn drop(&mut self) {
        if !self.released {
            let lock = self.lock;
            lock.unlock(self, self.started_at)
        }
    }
}

/// Fixed-size fair locks split by independent resource class.
///
/// A stalled waiter on one shard cannot block unrelated resource classes.
/// Ticket ordering preserves FIFO service within each shard.
pub struct ShardedTicketLock<const SHARDS: usize> {
    shards: [TicketLock; SHARDS],
}

impl<const SHARDS: usize> ShardedTicketLock<SHARDS> {
    pub const fn new() -> Self {
        Self {
            shards: [const { TicketLock::new() }; SHARDS],
        }
    }

    pub fn lock(&self, shard: usize, now: u64) -> LockGuard<'_> {
        self.shards[shard % SHARDS].lock(now)
    }

    pub fn report(&self, shard: usize) -> LockShardReport {
        self.shards[shard % SHARDS].report()
    }

    pub fn reports(&self) -> [LockShardReport; SHARDS] {
        core::array::from_fn(|index| self.shards[index].report())
    }
}

impl<const SHARDS: usize> Default for ShardedTicketLock<SHARDS> {
    fn default() -> Self {
        Self::new()
    }
}

pub const fn duration_bucket(duration: u64) -> usize {
    match duration {
        0 => 0,
        1 => 1,
        2..=3 => 2,
        4..=7 => 3,
        8..=15 => 4,
        16..=31 => 5,
        32..=63 => 6,
        _ => 7,
    }
}
