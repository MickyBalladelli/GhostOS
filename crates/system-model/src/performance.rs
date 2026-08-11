//! Bounded performance budgets for public control-plane work.
//!
//! The model contains measurements only. It deliberately has no tenant,
//! principal, resource, command argument, or payload fields.

pub const PERFORMANCE_DIAGNOSTICS_VERSION: u16 = 1;
pub const DEFAULT_CONTROL_PLANE_BUDGET_US: u64 = 100_000;
pub const DEFAULT_CONTROL_PLANE_TAIL_US: u64 = 250_000;
pub const DEFAULT_CONTROL_PLANE_RETRIES: u32 = 3;
pub const DEFAULT_CONTROL_PLANE_BYTES: u32 = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PerformanceBudget {
    pub queue_wait_us: u64,
    pub service_time_us: u64,
    pub tail_latency_us: u64,
    pub max_retries: u32,
    pub max_request_bytes: u32,
    pub max_response_bytes: u32,
}

impl PerformanceBudget {
    pub const fn control_plane() -> Self {
        Self {
            queue_wait_us: DEFAULT_CONTROL_PLANE_BUDGET_US,
            service_time_us: DEFAULT_CONTROL_PLANE_BUDGET_US,
            tail_latency_us: DEFAULT_CONTROL_PLANE_TAIL_US,
            max_retries: DEFAULT_CONTROL_PLANE_RETRIES,
            max_request_bytes: DEFAULT_CONTROL_PLANE_BYTES,
            max_response_bytes: DEFAULT_CONTROL_PLANE_BYTES,
        }
    }

    pub const fn for_route(route: u16) -> Self {
        let mut budget = Self::control_plane();
        // Mutations get a little more service time, while reads retain the
        // interactive default. The route is an opaque public route number.
        if route >= 13 {
            budget.service_time_us = 250_000;
            budget.tail_latency_us = 500_000;
        }
        budget
    }

    pub const fn within(self, diagnostics: PerformanceDiagnostics) -> bool {
        diagnostics.queue_wait_us <= self.queue_wait_us
            && diagnostics.service_time_us <= self.service_time_us
            && diagnostics.tail_latency_us <= self.tail_latency_us
            && diagnostics.retries <= self.max_retries
            && diagnostics.request_bytes <= self.max_request_bytes
            && diagnostics.response_bytes <= self.max_response_bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PerformanceDiagnostics {
    pub version: u16,
    pub queue_wait_us: u64,
    pub service_time_us: u64,
    pub retries: u32,
    pub request_bytes: u32,
    pub response_bytes: u32,
    pub tail_latency_us: u64,
    pub budget: PerformanceBudget,
    pub budget_exceeded: bool,
}

impl PerformanceDiagnostics {
    pub const fn empty() -> Self {
        Self {
            version: PERFORMANCE_DIAGNOSTICS_VERSION,
            queue_wait_us: 0,
            service_time_us: 0,
            retries: 0,
            request_bytes: 0,
            response_bytes: 0,
            tail_latency_us: 0,
            budget: PerformanceBudget::control_plane(),
            budget_exceeded: false,
        }
    }

    pub const fn new(
        queue_wait_us: u64,
        service_time_us: u64,
        retries: u32,
        request_bytes: u32,
        response_bytes: u32,
        tail_latency_us: u64,
        budget: PerformanceBudget,
    ) -> Self {
        let candidate = Self {
            version: PERFORMANCE_DIAGNOSTICS_VERSION,
            queue_wait_us,
            service_time_us,
            retries,
            request_bytes,
            response_bytes,
            tail_latency_us,
            budget,
            budget_exceeded: false,
        };
        Self {
            budget_exceeded: !budget.within(candidate),
            ..candidate
        }
    }

    pub const fn is_redacted(self) -> bool {
        true
    }
}

/// Fixed-size rolling latency window. It retains measurements only, so its
/// percentile cannot reveal tenant data.
#[derive(Clone, Copy)]
pub struct TailLatencyWindow<const CAPACITY: usize> {
    samples: [u64; CAPACITY],
    count: usize,
    cursor: usize,
}

impl<const CAPACITY: usize> TailLatencyWindow<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY > 0);
        Self {
            samples: [0; CAPACITY],
            count: 0,
            cursor: 0,
        }
    }

    pub fn record(&mut self, value_us: u64) {
        self.samples[self.cursor] = value_us;
        self.cursor = (self.cursor + 1) % CAPACITY;
        self.count = self.count.saturating_add(1).min(CAPACITY);
    }

    pub fn p99(&self) -> u64 {
        if self.count == 0 {
            return 0
        }
        let mut ordered = [0; CAPACITY];
        ordered[..self.count].copy_from_slice(&self.samples[..self.count]);
        let mut index = 1;
        while index < self.count {
            let value = ordered[index];
            let mut position = index;
            while position > 0 && ordered[position - 1] > value {
                ordered[position] = ordered[position - 1];
                position -= 1;
            }
            ordered[position] = value;
            index += 1;
        }
        let rank = self.count.saturating_mul(99).saturating_add(99) / 100;
        ordered[rank.saturating_sub(1).min(self.count - 1)]
    }
}

impl<const CAPACITY: usize> Default for TailLatencyWindow<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
