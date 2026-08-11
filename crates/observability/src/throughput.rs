//! Bounded producer batching and interrupt moderation.
//!
//! The policy is deliberately small and copy-free. Callers provide the number
//! of queued items and the size of one item; the controller returns a bounded
//! work grant and whether the producer should signal a consumer.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProducerPolicy {
    pub max_batch: usize,
    pub max_bytes: usize,
    pub max_delay_us: u64,
    pub interrupt_every: usize,
    pub fairness_quota: usize,
}

impl ProducerPolicy {
    pub const STORAGE: Self = Self {
        max_batch: 16,
        max_bytes: 4 * 1024 * 1024,
        max_delay_us: 2_000,
        interrupt_every: 8,
        fairness_quota: 4,
    };

    pub const NETWORK: Self = Self {
        max_batch: 32,
        max_bytes: 64 * 1024,
        max_delay_us: 1_000,
        interrupt_every: 16,
        fairness_quota: 8,
    };

    pub const LOGGING: Self = Self {
        max_batch: 32,
        max_bytes: 4 * 1024,
        max_delay_us: 5_000,
        interrupt_every: 16,
        fairness_quota: 8,
    };

    pub const AUDIT: Self = Self {
        max_batch: 16,
        max_bytes: 4 * 1024,
        max_delay_us: 10_000,
        interrupt_every: 8,
        fairness_quota: 4,
    };

    pub const fn valid(self) -> bool {
        self.max_batch != 0
            && self.max_bytes != 0
            && self.max_delay_us != 0
            && self.interrupt_every != 0
            && self.fairness_quota != 0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BatchDecision {
    pub count: usize,
    pub coalesced: bool,
    pub interrupt: bool,
    pub fairness_yield: bool,
}

/// Stateful gate shared by producer loops. `since_interrupt` bounds the time
/// between consumer wakeups even when a queue never reaches its batch size.
pub struct BatchController {
    policy: ProducerPolicy,
    since_interrupt: usize,
}

impl BatchController {
    pub const fn new(policy: ProducerPolicy) -> Self {
        assert!(policy.valid());
        Self {
            policy,
            since_interrupt: 0,
        }
    }

    pub const fn policy(&self) -> ProducerPolicy {
        self.policy
    }

    pub fn plan(
        &mut self,
        pending: usize,
        item_bytes: usize,
        oldest_age_us: u64,
        interactive_pending: bool,
    ) -> BatchDecision {
        if pending == 0 {
            return BatchDecision::default()
        }

        let item_bytes = item_bytes.max(1);
        let memory_limit = (self.policy.max_bytes / item_bytes).max(1);
        let unconstrained = pending
            .min(self.policy.max_batch)
            .min(memory_limit);
        let count = if interactive_pending {
            unconstrained.min(self.policy.fairness_quota)
        } else {
            unconstrained
        };
        let fairness_yield = interactive_pending && count < unconstrained;
        let age_due = oldest_age_us >= self.policy.max_delay_us;
        let interrupt = age_due
            || self.since_interrupt.saturating_add(count) >= self.policy.interrupt_every
            || fairness_yield;
        self.since_interrupt = if interrupt {
            0
        } else {
            self.since_interrupt.saturating_add(count)
        };
        BatchDecision {
            count,
            coalesced: count > 1,
            interrupt,
            fairness_yield,
        }
    }
}

impl Default for BatchController {
    fn default() -> Self {
        Self::new(ProducerPolicy::LOGGING)
    }
}

#[cfg(test)]
mod tests {
    use super::{BatchController, ProducerPolicy};

    #[test]
    fn policy_bounds_work_memory_and_interrupts() {
        let mut controller = BatchController::new(ProducerPolicy {
            max_batch: 4,
            max_bytes: 8,
            max_delay_us: 10,
            interrupt_every: 4,
            fairness_quota: 2,
        });
        let first = controller.plan(8, 2, 0, false);
        assert_eq!(first.count, 4);
        assert!(first.coalesced);
        assert!(first.interrupt);
        let delayed = controller.plan(1, 2, 10, false);
        assert!(delayed.interrupt);
    }

    #[test]
    fn interactive_work_gets_a_fair_slice() {
        let mut controller = BatchController::new(ProducerPolicy::STORAGE);
        let decision = controller.plan(16, 4096, 0, true);
        assert_eq!(decision.count, ProducerPolicy::STORAGE.fairness_quota);
        assert!(decision.fairness_yield);
        assert!(decision.interrupt);
    }
}
