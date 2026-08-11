//! Shared CPU scale and isolation policy for hot kernel and daemon paths.
//!
//! The policy is fixed-capacity so callers can publish it in diagnostics
//! without allocating or sharing a mutable global scheduler lock.

pub const SCALE_CPU_TIERS: [usize; 5] = [1, 2, 8, 32, 128];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ScalePath {
    Scheduler = 1,
    Ipc = 2,
    Timers = 3,
    Logging = 4,
    Audit = 5,
}

impl ScalePath {
    pub const ALL: [Self; 5] = [
        Self::Scheduler,
        Self::Ipc,
        Self::Timers,
        Self::Logging,
        Self::Audit,
    ];

    pub const fn p99_budget_us(self) -> u32 {
        match self {
            Self::Scheduler => 100,
            Self::Ipc => 250,
            Self::Timers => 100,
            Self::Logging => 5_000,
            Self::Audit => 10_000,
        }
    }

    pub const fn queue_capacity(self) -> u32 {
        match self {
            Self::Scheduler => 256,
            Self::Ipc => 1_024,
            Self::Timers => 256,
            Self::Logging => 256,
            Self::Audit => 256,
        }
    }

    pub const fn lock_saturation_per_cpu(self) -> u32 {
        match self {
            Self::Scheduler => 4_096,
            Self::Ipc => 8_192,
            Self::Timers => 4_096,
            Self::Logging => 2_048,
            Self::Audit => 1_024,
        }
    }
}

/// Two-word CPU set. The high word is required for the 128-CPU qualification
/// tier; keeping the representation here avoids platform-sized bitsets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AffinitySet {
    words: [u64; 2],
}

impl AffinitySet {
    pub const EMPTY: Self = Self { words: [0; 2] };

    pub const fn from_words(low: u64, high: u64) -> Self {
        Self { words: [low, high] }
    }

    pub const fn contains(self, cpu: usize) -> bool {
        if cpu >= 128 {
            return false
        }
        let word = cpu / 64;
        let bit = cpu % 64;
        self.words[word] & (1u64 << bit) != 0
    }

    pub const fn with_cpu(self, cpu: usize) -> Self {
        if cpu >= 128 {
            return self
        }
        let mut words = self.words;
        words[cpu / 64] |= 1u64 << (cpu % 64);
        Self { words }
    }

    pub const fn difference(self, other: Self) -> Self {
        Self {
            words: [self.words[0] & !other.words[0], self.words[1] & !other.words[1]],
        }
    }

    pub const fn count(self) -> usize {
        self.words[0].count_ones() as usize + self.words[1].count_ones() as usize
    }

    pub const fn words(self) -> [u64; 2] {
        self.words
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScalePolicy {
    cpu_count: usize,
    online: AffinitySet,
    housekeeping: AffinitySet,
    isolated: AffinitySet,
}

impl ScalePolicy {
    pub const fn for_cpu_count(cpu_count: usize) -> Option<Self> {
        if !is_scale_tier(cpu_count) {
            return None
        }

        let mut online = AffinitySet::EMPTY;
        let mut cpu = 0;
        while cpu < cpu_count {
            online = online.with_cpu(cpu);
            cpu += 1;
        }

        let isolated_count = if cpu_count < 8 {
            0
        } else {
            cpu_count / 8
        };
        let mut isolated = AffinitySet::EMPTY;
        let first_isolated = cpu_count - isolated_count;
        cpu = first_isolated;
        while cpu < cpu_count {
            isolated = isolated.with_cpu(cpu);
            cpu += 1;
        }

        Some(Self {
            cpu_count,
            online,
            housekeeping: online.difference(isolated),
            isolated,
        })
    }

    pub const fn cpu_count(self) -> usize {
        self.cpu_count
    }

    pub const fn online(self) -> AffinitySet {
        self.online
    }

    pub const fn housekeeping(self) -> AffinitySet {
        self.housekeeping
    }

    pub const fn isolated(self) -> AffinitySet {
        self.isolated
    }

    pub const fn affinity(self, path: ScalePath) -> AffinitySet {
        match path {
            ScalePath::Scheduler
            | ScalePath::Ipc
            | ScalePath::Timers
            | ScalePath::Logging
            | ScalePath::Audit => self.housekeeping,
        }
    }

    pub const fn accepts(self, path: ScalePath, cpu: usize) -> bool {
        self.affinity(path).contains(cpu)
    }

    pub const fn isolation_is_explicit(self) -> bool {
        self.online.difference(self.housekeeping).count() == self.isolated.count()
    }
}

const fn is_scale_tier(cpu_count: usize) -> bool {
    matches!(cpu_count, 1 | 2 | 8 | 32 | 128)
}

#[cfg(test)]
mod tests {
    use super::{ScalePath, ScalePolicy};

    #[test]
    fn policy_covers_requested_tiers_and_keeps_housekeeping() {
        for cpu_count in [1, 2, 8, 32, 128] {
            let policy = ScalePolicy::for_cpu_count(cpu_count).expect("scale tier");
            assert_eq!(policy.online().count(), cpu_count);
            assert!(policy.housekeeping().count() != 0);
            assert!(policy.isolation_is_explicit());
            for path in ScalePath::ALL {
                assert_eq!(policy.affinity(path), policy.housekeeping());
            }
        }
    }

    #[test]
    fn unsupported_tiers_are_rejected() {
        assert!(ScalePolicy::for_cpu_count(4).is_none());
        assert!(ScalePolicy::for_cpu_count(129).is_none());
    }
}
