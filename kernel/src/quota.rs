use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const MICROS_PER_SECOND: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BucketConfig {
    pub capacity: u64,
    pub refill_per_second: u64,
}

impl BucketConfig {
    pub const fn new(capacity: u64, refill_per_second: u64) -> Option<Self> {
        if capacity == 0 || refill_per_second == 0 {
            None
        } else {
            Some(Self {
                capacity,
                refill_per_second,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuotaPolicy {
    pub ipc: BucketConfig,
    pub page_fault: BucketConfig,
    pub memory: BucketConfig,
    pub max_memory_bytes: u64,
}

impl QuotaPolicy {
    pub const fn new(
        ipc: BucketConfig,
        page_fault: BucketConfig,
        memory: BucketConfig,
        max_memory_bytes: u64,
    ) -> Option<Self> {
        if max_memory_bytes == 0 {
            None
        } else {
            Some(Self {
                ipc,
                page_fault,
                memory,
                max_memory_bytes,
            })
        }
    }

    pub const fn is_valid(self) -> bool {
        self.ipc.capacity > 0
            && self.ipc.refill_per_second > 0
            && self.page_fault.capacity > 0
            && self.page_fault.refill_per_second > 0
            && self.memory.capacity > 0
            && self.memory.refill_per_second > 0
            && self.max_memory_bytes > 0
    }
}

impl Default for QuotaPolicy {
    fn default() -> Self {
        Self {
            ipc: BucketConfig {
                capacity: 1_024,
                refill_per_second: 4_096,
            },
            page_fault: BucketConfig {
                capacity: 128,
                refill_per_second: 128,
            },
            memory: BucketConfig {
                capacity: 256 * 1024 * 1024,
                refill_per_second: 512 * 1024 * 1024,
            },
            max_memory_bytes: 1024 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaResource {
    IpcMessages,
    PageFaults,
    MemoryBytes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaDecision {
    Allowed,
    Throttled { retry_after_us: u64 },
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuotaUsage {
    pub memory_in_use: u64,
    pub max_memory_bytes: u64,
}

struct BucketState {
    tokens: AtomicU64,
    last_refill_us: AtomicU64,
    remainder: AtomicU64,
}

impl BucketState {
    const fn new(tokens: u64) -> Self {
        Self {
            tokens: AtomicU64::new(tokens),
            last_refill_us: AtomicU64::new(0),
            remainder: AtomicU64::new(0),
        }
    }

    fn load(&self) -> BucketValues {
        BucketValues {
            tokens: self.tokens.load(Ordering::Relaxed),
            last_refill_us: self.last_refill_us.load(Ordering::Relaxed),
            remainder: self.remainder.load(Ordering::Relaxed),
        }
    }

    fn store(&self, values: BucketValues) {
        self.tokens.store(values.tokens, Ordering::Relaxed);
        self.last_refill_us
            .store(values.last_refill_us, Ordering::Relaxed);
        self.remainder.store(values.remainder, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy)]
struct BucketValues {
    tokens: u64,
    last_refill_us: u64,
    remainder: u64,
}

/// A fixed-size, lock-protected quota state suitable for kernel capability
/// descriptors. The lock only protects a few integer operations and never
/// allocates or sleeps.
pub struct CapabilityQuota {
    policy: QuotaPolicy,
    locked: AtomicBool,
    ipc: BucketState,
    page_fault: BucketState,
    memory: BucketState,
    memory_in_use: AtomicU64,
}

impl CapabilityQuota {
    pub const fn new() -> Self {
        let policy = QuotaPolicy {
            ipc: BucketConfig {
                capacity: 1_024,
                refill_per_second: 4_096,
            },
            page_fault: BucketConfig {
                capacity: 128,
                refill_per_second: 128,
            },
            memory: BucketConfig {
                capacity: 256 * 1024 * 1024,
                refill_per_second: 512 * 1024 * 1024,
            },
            max_memory_bytes: 1024 * 1024 * 1024,
        };
        Self {
            policy,
            locked: AtomicBool::new(false),
            ipc: BucketState::new(policy.ipc.capacity),
            page_fault: BucketState::new(policy.page_fault.capacity),
            memory: BucketState::new(policy.memory.capacity),
            memory_in_use: AtomicU64::new(0),
        }
    }

    pub fn configure(&mut self, policy: QuotaPolicy) {
        self.policy = policy;
        self.ipc.store(BucketValues {
            tokens: policy.ipc.capacity,
            last_refill_us: 0,
            remainder: 0,
        });
        self.page_fault.store(BucketValues {
            tokens: policy.page_fault.capacity,
            last_refill_us: 0,
            remainder: 0,
        });
        self.memory.store(BucketValues {
            tokens: policy.memory.capacity,
            last_refill_us: 0,
            remainder: 0,
        });
        self.memory_in_use.store(0, Ordering::Relaxed);
    }

    pub fn policy(&self) -> QuotaPolicy {
        self.policy
    }

    pub fn consume(
        &self,
        resource: QuotaResource,
        now_us: u64,
        amount: u64,
    ) -> QuotaDecision {
        if amount == 0 {
            return QuotaDecision::Allowed
        }
        self.lock();
        let decision = match resource {
            QuotaResource::IpcMessages => {
                acquire(self.policy.ipc, &self.ipc, now_us, amount)
            }
            QuotaResource::PageFaults => {
                acquire(self.policy.page_fault, &self.page_fault, now_us, amount)
            }
            QuotaResource::MemoryBytes => {
                let memory_in_use = self.memory_in_use.load(Ordering::Relaxed);
                if amount > self.policy.max_memory_bytes
                    || memory_in_use > self.policy.max_memory_bytes - amount
                {
                    QuotaDecision::Rejected
                } else {
                    let decision = acquire(self.policy.memory, &self.memory, now_us, amount);
                    if decision == QuotaDecision::Allowed {
                        self.memory_in_use
                            .store(memory_in_use.saturating_add(amount), Ordering::Relaxed);
                    }
                    decision
                }
            }
        };
        self.unlock();
        decision
    }

    pub fn refund(&self, resource: QuotaResource, amount: u64) {
        if amount == 0 {
            return
        }
        self.lock();
        match resource {
            QuotaResource::IpcMessages => {
                let mut values = self.ipc.load();
                values.tokens = values
                    .tokens
                    .saturating_add(amount)
                    .min(self.policy.ipc.capacity);
                self.ipc.store(values);
            }
            QuotaResource::PageFaults => {
                let mut values = self.page_fault.load();
                values.tokens = values
                    .tokens
                    .saturating_add(amount)
                    .min(self.policy.page_fault.capacity);
                self.page_fault.store(values)
            }
            QuotaResource::MemoryBytes => {
                let mut values = self.memory.load();
                values.tokens = values
                    .tokens
                    .saturating_add(amount)
                    .min(self.policy.memory.capacity);
                self.memory.store(values);
                let in_use = self.memory_in_use.load(Ordering::Relaxed);
                self.memory_in_use
                    .store(in_use.saturating_sub(amount), Ordering::Relaxed);
            }
        }
        self.unlock()
    }

    pub fn release_memory(&self, amount: u64) {
        if amount == 0 {
            return
        }
        self.lock();
        let in_use = self.memory_in_use.load(Ordering::Relaxed);
        self.memory_in_use
            .store(in_use.saturating_sub(amount), Ordering::Relaxed);
        self.unlock()
    }

    pub fn usage(&self) -> QuotaUsage {
        self.lock();
        let usage = QuotaUsage {
            memory_in_use: self.memory_in_use.load(Ordering::Relaxed),
            max_memory_bytes: self.policy.max_memory_bytes,
        };
        self.unlock();
        usage
    }

    fn lock(&self) {
        while self.locked.swap(true, Ordering::Acquire) {
            core::hint::spin_loop()
        }
    }

    fn unlock(&self) {
        self.locked.store(false, Ordering::Release)
    }
}

impl Default for CapabilityQuota {
    fn default() -> Self {
        Self::new()
    }
}

fn acquire(
    config: BucketConfig,
    state: &BucketState,
    now_us: u64,
    amount: u64,
) -> QuotaDecision {
    if amount > config.capacity {
        return QuotaDecision::Rejected
    }
    let mut values = state.load();
    refill(config, &mut values, now_us);
    let decision = if values.tokens >= amount {
        values.tokens -= amount;
        QuotaDecision::Allowed
    } else {
        let missing = amount - values.tokens;
        let numerator = missing.saturating_mul(MICROS_PER_SECOND);
        let retry_after_us = numerator
            .saturating_add(config.refill_per_second - 1)
            .checked_div(config.refill_per_second)
            .unwrap_or(u64::MAX)
            .max(1);
        QuotaDecision::Throttled { retry_after_us }
    };
    state.store(values);
    decision
}

fn refill(config: BucketConfig, state: &mut BucketValues, now_us: u64) {
    if now_us < state.last_refill_us {
        state.last_refill_us = now_us;
        state.remainder = 0;
        return
    }
    let elapsed = now_us - state.last_refill_us;
    let produced = elapsed
        .saturating_mul(config.refill_per_second)
        .saturating_add(state.remainder);
    let added = produced / MICROS_PER_SECOND;
    state.remainder = produced % MICROS_PER_SECOND;
    state.tokens = state.tokens.saturating_add(added).min(config.capacity);
    state.last_refill_us = now_us;
}
