use core::cell::UnsafeCell;

use crate::contention::{LockShardReport, LOCK_DURATION_BUCKETS};

const QUOTA_SHARDS: usize = 3;
const DEFAULT_POLICY: QuotaPolicy = QuotaPolicy {
    ipc: BucketConfig { capacity: 1_024, refill_per_second: 4_096 },
    page_fault: BucketConfig { capacity: 128, refill_per_second: 128 },
    memory: BucketConfig { capacity: 256 * 1024 * 1024, refill_per_second: 512 * 1024 * 1024 },
    max_memory_bytes: 1024 * 1024 * 1024,
};

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
            Some(Self { capacity, refill_per_second })
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
            Some(Self { ipc, page_fault, memory, max_memory_bytes })
        }
    }

    pub fn is_valid(self) -> bool {
        // SAFETY: C validates a small by-value policy record.
        unsafe { ghostos_quota_policy_valid(c_policy(self)) }
    }
}

impl Default for QuotaPolicy {
    fn default() -> Self {
        DEFAULT_POLICY
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaResource {
    IpcMessages,
    PageFaults,
    MemoryBytes,
}

impl QuotaResource {
    pub const fn index(self) -> usize {
        match self {
            Self::IpcMessages => 0,
            Self::PageFaults => 1,
            Self::MemoryBytes => 2,
        }
    }
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuotaContentionReport {
    pub resources: [LockShardReport; QUOTA_SHARDS],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CBucketConfig {
    capacity: u64,
    refill_per_second: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CQuotaPolicy {
    resources: [CBucketConfig; QUOTA_SHARDS],
    max_memory_bytes: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CBucketState {
    tokens: u64,
    last_refill_us: u64,
    remainder: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CLockState {
    next_ticket: u64,
    serving_ticket: u64,
    owner: u64,
    acquisitions: u64,
    contended_acquisitions: u64,
    spins: u64,
    releases: u64,
    duration_histogram: [u64; LOCK_DURATION_BUCKETS],
    max_duration: u64,
}

impl CLockState {
    const EMPTY: Self = Self {
        next_ticket: 0,
        serving_ticket: 0,
        owner: 0,
        acquisitions: 0,
        contended_acquisitions: 0,
        spins: 0,
        releases: 0,
        duration_histogram: [0; LOCK_DURATION_BUCKETS],
        max_duration: 0,
    };
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CQuotaState {
    policy: CQuotaPolicy,
    locks: [CLockState; QUOTA_SHARDS],
    clock: u64,
    buckets: [CBucketState; QUOTA_SHARDS],
    memory_in_use: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CQuotaLockReport {
    active_owner: u64,
    acquisitions: u64,
    contended_acquisitions: u64,
    spins: u64,
    releases: u64,
    duration_histogram: [u64; LOCK_DURATION_BUCKETS],
    max_duration: u64,
}

impl CQuotaLockReport {
    const EMPTY: Self = Self {
        active_owner: 0,
        acquisitions: 0,
        contended_acquisitions: 0,
        spins: 0,
        releases: 0,
        duration_histogram: [0; LOCK_DURATION_BUCKETS],
        max_duration: 0,
    };
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CQuotaResult {
    retry_after_us: u64,
    decision: u32,
}

unsafe extern "C" {
    fn ghostos_quota_policy_valid(policy: CQuotaPolicy) -> bool;
    fn ghostos_quota_configure(quota: *mut CQuotaState, policy: CQuotaPolicy);
    fn ghostos_quota_get_policy(quota: *const CQuotaState) -> CQuotaPolicy;
    fn ghostos_quota_consume(quota: *mut CQuotaState, resource: u32, now_us: u64, amount: u64) -> CQuotaResult;
    fn ghostos_quota_refund(quota: *mut CQuotaState, resource: u32, amount: u64);
    fn ghostos_quota_release_memory(quota: *mut CQuotaState, amount: u64);
    fn ghostos_quota_memory_in_use(quota: *const CQuotaState) -> u64;
    fn ghostos_quota_lock_reports(quota: *const CQuotaState, reports: *mut CQuotaLockReport);
}

fn c_policy(policy: QuotaPolicy) -> CQuotaPolicy {
    CQuotaPolicy {
        resources: [policy.ipc, policy.page_fault, policy.memory].map(|bucket| CBucketConfig {
            capacity: bucket.capacity,
            refill_per_second: bucket.refill_per_second,
        }),
        max_memory_bytes: policy.max_memory_bytes,
    }
}

fn rust_policy(policy: CQuotaPolicy) -> QuotaPolicy {
    let bucket = |bucket: CBucketConfig| BucketConfig {
        capacity: bucket.capacity,
        refill_per_second: bucket.refill_per_second,
    };
    QuotaPolicy {
        ipc: bucket(policy.resources[0]),
        page_fault: bucket(policy.resources[1]),
        memory: bucket(policy.resources[2]),
        max_memory_bytes: policy.max_memory_bytes,
    }
}

/// Fixed-size quota state with independent, fair C ticket locks per resource.
pub struct CapabilityQuota {
    raw: UnsafeCell<CQuotaState>,
}

// SAFETY: C serializes mutable bucket and lock state by resource ticket lock;
// policy changes require an exclusive Rust borrow.
unsafe impl Sync for CapabilityQuota {}

impl CapabilityQuota {
    pub const fn new() -> Self {
        let policy = DEFAULT_POLICY;
        Self {
            raw: UnsafeCell::new(CQuotaState {
                policy: CQuotaPolicy {
                    resources: [
                        CBucketConfig { capacity: policy.ipc.capacity, refill_per_second: policy.ipc.refill_per_second },
                        CBucketConfig { capacity: policy.page_fault.capacity, refill_per_second: policy.page_fault.refill_per_second },
                        CBucketConfig { capacity: policy.memory.capacity, refill_per_second: policy.memory.refill_per_second },
                    ],
                    max_memory_bytes: policy.max_memory_bytes,
                },
                locks: [CLockState::EMPTY; QUOTA_SHARDS],
                clock: 0,
                buckets: [
                    CBucketState { tokens: policy.ipc.capacity, last_refill_us: 0, remainder: 0 },
                    CBucketState { tokens: policy.page_fault.capacity, last_refill_us: 0, remainder: 0 },
                    CBucketState { tokens: policy.memory.capacity, last_refill_us: 0, remainder: 0 },
                ],
                memory_in_use: 0,
            }),
        }
    }

    pub fn configure(&mut self, policy: QuotaPolicy) {
        // SAFETY: caller has exclusive access to this quota while C resets it.
        unsafe { ghostos_quota_configure(self.raw.get(), c_policy(policy)) }
    }

    pub fn policy(&self) -> QuotaPolicy {
        // SAFETY: C copies the stable policy from this quota.
        rust_policy(unsafe { ghostos_quota_get_policy(self.raw.get()) })
    }

    pub fn consume(&self, resource: QuotaResource, now_us: u64, amount: u64) -> QuotaDecision {
        // SAFETY: C synchronizes access to quota state with the resource ticket lock.
        let result = unsafe { ghostos_quota_consume(self.raw.get(), resource.index() as u32, now_us, amount) };
        match result.decision {
            0 => QuotaDecision::Allowed,
            1 => QuotaDecision::Throttled { retry_after_us: result.retry_after_us },
            _ => QuotaDecision::Rejected,
        }
    }

    pub fn refund(&self, resource: QuotaResource, amount: u64) {
        // SAFETY: C synchronizes the refund with the resource ticket lock.
        unsafe { ghostos_quota_refund(self.raw.get(), resource.index() as u32, amount) }
    }

    pub fn release_memory(&self, amount: u64) {
        // SAFETY: C synchronizes the memory release with the memory ticket lock.
        unsafe { ghostos_quota_release_memory(self.raw.get(), amount) }
    }

    pub fn usage(&self) -> QuotaUsage {
        let policy = self.policy();
        // SAFETY: C atomically reads the memory-use counter.
        let memory_in_use = unsafe { ghostos_quota_memory_in_use(self.raw.get()) };
        QuotaUsage { memory_in_use, max_memory_bytes: policy.max_memory_bytes }
    }

    pub fn contention_report(&self) -> QuotaContentionReport {
        let mut reports = [CQuotaLockReport::EMPTY; QUOTA_SHARDS];
        // SAFETY: C fills the three-element output array from atomic lock metrics.
        unsafe { ghostos_quota_lock_reports(self.raw.get(), reports.as_mut_ptr()) };
        QuotaContentionReport {
            resources: reports.map(|report| LockShardReport {
                active_owner: report.active_owner,
                acquisitions: report.acquisitions,
                contended_acquisitions: report.contended_acquisitions,
                spins: report.spins,
                releases: report.releases,
                duration_histogram: report.duration_histogram,
                max_duration: report.max_duration,
            }),
        }
    }
}

impl Default for CapabilityQuota {
    fn default() -> Self {
        Self::new()
    }
}
