//! Adaptive, bounded cache policy state shared by storage, package, network,
//! cluster, compiler, and VM cache producers.

pub const MAX_CACHE_POLICIES: usize = 128;
pub const CACHE_RATE_SCALE: u64 = 1_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CacheKind {
    SynFsMetadata = 1,
    PackageArtifacts = 2,
    CompilerOutputs = 3,
    Dns = 4,
    ClusterMembership = 5,
    VmTranslationBlocks = 6,
}

impl CacheKind {
    pub const ALL: [Self; 6] = [
        Self::SynFsMetadata,
        Self::PackageArtifacts,
        Self::CompilerOutputs,
        Self::Dns,
        Self::ClusterMembership,
        Self::VmTranslationBlocks,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::SynFsMetadata => "ghostfs-metadata",
            Self::PackageArtifacts => "package-artifacts",
            Self::CompilerOutputs => "compiler-outputs",
            Self::Dns => "dns",
            Self::ClusterMembership => "cluster-membership",
            Self::VmTranslationBlocks => "vm-translation-blocks",
        }
    }

    pub const fn default_policy(self) -> CachePolicy {
        let (memory_ceiling_bytes, ttl_us, max_ttl_us, stale_risk) = match self {
            Self::SynFsMetadata => (16 * 1024 * 1024, 1_000_000, 60_000_000, 20),
            Self::PackageArtifacts => (256 * 1024 * 1024, 300_000_000, 86_400_000_000, 5),
            Self::CompilerOutputs => (512 * 1024 * 1024, 600_000_000, 86_400_000_000, 5),
            Self::Dns => (4 * 1024 * 1024, 30_000_000, 300_000_000, 10),
            Self::ClusterMembership => (1 * 1024 * 1024, 5_000_000, 60_000_000, 5),
            Self::VmTranslationBlocks => (64 * 1024 * 1024, 1_000_000, 10_000_000, 5),
        };
        CachePolicy::new(
            memory_ceiling_bytes,
            900,
            stale_risk,
            10_000,
            ttl_us / 4,
            max_ttl_us,
            ttl_us,
        )
        .expect("built-in cache policy is valid")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CachePolicy {
    pub memory_ceiling_bytes: u64,
    pub target_hit_rate_per_mille: u16,
    pub max_stale_risk_per_mille: u16,
    pub eviction_cost_budget_us: u64,
    pub min_ttl_us: u64,
    pub max_ttl_us: u64,
    pub ttl_us: u64,
}

impl CachePolicy {
    pub const fn new(
        memory_ceiling_bytes: u64,
        target_hit_rate_per_mille: u16,
        max_stale_risk_per_mille: u16,
        eviction_cost_budget_us: u64,
        min_ttl_us: u64,
        max_ttl_us: u64,
        ttl_us: u64,
    ) -> Option<Self> {
        if memory_ceiling_bytes == 0
            || target_hit_rate_per_mille > CACHE_RATE_SCALE as u16
            || max_stale_risk_per_mille > CACHE_RATE_SCALE as u16
            || eviction_cost_budget_us == 0
            || min_ttl_us == 0
            || min_ttl_us > max_ttl_us
            || ttl_us < min_ttl_us
            || ttl_us > max_ttl_us
        {
            None
        } else {
            Some(Self {
                memory_ceiling_bytes,
                target_hit_rate_per_mille,
                max_stale_risk_per_mille,
                eviction_cost_budget_us,
                min_ttl_us,
                max_ttl_us,
                ttl_us,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheEvent {
    Hit,
    Miss,
    Eviction { bytes: u64, cost_us: u64 },
    StaleRejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CachePolicyError {
    Capacity,
    InvalidWorkload,
    InvalidPolicy,
    UnknownPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CachePolicyReport {
    pub kind: CacheKind,
    pub workload: u64,
    pub policy: CachePolicy,
    pub current_bytes: u64,
    pub accesses: u64,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub stale_rejections: u64,
    pub eviction_cost_us: u64,
    pub hit_rate_per_mille: u16,
    pub stale_risk_per_mille: u16,
    pub average_eviction_cost_us: u64,
}

#[derive(Clone, Copy)]
struct CachePolicyState {
    kind: CacheKind,
    workload: u64,
    policy: CachePolicy,
    current_bytes: u64,
    accesses: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
    stale_rejections: u64,
    eviction_cost_us: u64,
    sample_accesses: u64,
    sample_hits: u64,
    sample_evictions: u64,
    sample_stale_rejections: u64,
    sample_eviction_cost_us: u64,
}

impl CachePolicyState {
    const fn new(kind: CacheKind, workload: u64, policy: CachePolicy) -> Self {
        Self {
            kind,
            workload,
            policy,
            current_bytes: 0,
            accesses: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            stale_rejections: 0,
            eviction_cost_us: 0,
            sample_accesses: 0,
            sample_hits: 0,
            sample_evictions: 0,
            sample_stale_rejections: 0,
            sample_eviction_cost_us: 0,
        }
    }

    fn report(self) -> CachePolicyReport {
        CachePolicyReport {
            kind: self.kind,
            workload: self.workload,
            policy: self.policy,
            current_bytes: self.current_bytes,
            accesses: self.accesses,
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            stale_rejections: self.stale_rejections,
            eviction_cost_us: self.eviction_cost_us,
            hit_rate_per_mille: rate(self.hits, self.accesses),
            stale_risk_per_mille: rate(self.stale_rejections, self.accesses),
            average_eviction_cost_us: average(self.eviction_cost_us, self.evictions),
        }
    }
}

/// Fixed-capacity policy registry. Each `(kind, workload)` pair has an
/// independent ceiling, TTL, counters, and adaptive tuning state.
#[derive(Clone, Copy)]
pub struct CachePolicyRegistry<const CAPACITY: usize = MAX_CACHE_POLICIES> {
    states: [Option<CachePolicyState>; CAPACITY],
}

impl<const CAPACITY: usize> CachePolicyRegistry<CAPACITY> {
    pub const fn new() -> Self {
        assert!(CAPACITY > 0);
        Self { states: [None; CAPACITY] }
    }

    pub fn configure(
        &mut self,
        kind: CacheKind,
        workload: u64,
        policy: CachePolicy,
    ) -> Result<(), CachePolicyError> {
        validate(workload, policy)?;
        if let Some(index) = self.index(kind, workload) {
            let state = self.states[index].as_mut().expect("cache index is occupied");
            state.policy = policy;
            state.current_bytes = state.current_bytes.min(policy.memory_ceiling_bytes);
            return Ok(())
        }
        let slot = self
            .states
            .iter_mut()
            .find(|state| state.is_none())
            .ok_or(CachePolicyError::Capacity)?;
        *slot = Some(CachePolicyState::new(kind, workload, policy));
        Ok(())
    }

    pub fn policy(&self, kind: CacheKind, workload: u64) -> Option<CachePolicy> {
        self.index(kind, workload)
            .and_then(|index| self.states[index].map(|state| state.policy))
    }

    pub fn configure_workload(&mut self, workload: u64) -> Result<(), CachePolicyError> {
        for kind in CacheKind::ALL {
            self.configure(kind, workload, kind.default_policy())?;
        }
        Ok(())
    }

    pub fn admit(
        &mut self,
        kind: CacheKind,
        workload: u64,
        bytes: u64,
    ) -> Result<bool, CachePolicyError> {
        let index = self.ensure(kind, workload)?;
        let state = self.states[index].as_mut().expect("cache index is occupied");
        if bytes > state.policy.memory_ceiling_bytes.saturating_sub(state.current_bytes) {
            return Ok(false)
        }
        state.current_bytes = state.current_bytes.saturating_add(bytes);
        Ok(true)
    }

    pub fn release(
        &mut self,
        kind: CacheKind,
        workload: u64,
        bytes: u64,
    ) -> Result<(), CachePolicyError> {
        let index = self.ensure(kind, workload)?;
        let state = self.states[index].as_mut().expect("cache index is occupied");
        state.current_bytes = state.current_bytes.saturating_sub(bytes);
        Ok(())
    }

    pub fn observe(
        &mut self,
        kind: CacheKind,
        workload: u64,
        event: CacheEvent,
    ) -> Result<(), CachePolicyError> {
        let index = self.ensure(kind, workload)?;
        let state = self.states[index].as_mut().expect("cache index is occupied");
        match event {
            CacheEvent::Hit => {
                state.accesses = state.accesses.saturating_add(1);
                state.hits = state.hits.saturating_add(1);
                state.sample_accesses = state.sample_accesses.saturating_add(1);
                state.sample_hits = state.sample_hits.saturating_add(1);
            }
            CacheEvent::Miss => {
                state.accesses = state.accesses.saturating_add(1);
                state.misses = state.misses.saturating_add(1);
                state.sample_accesses = state.sample_accesses.saturating_add(1);
            }
            CacheEvent::Eviction { bytes, cost_us } => {
                state.current_bytes = state.current_bytes.saturating_sub(bytes);
                state.evictions = state.evictions.saturating_add(1);
                state.eviction_cost_us = state.eviction_cost_us.saturating_add(cost_us);
                state.sample_evictions = state.sample_evictions.saturating_add(1);
                state.sample_eviction_cost_us =
                    state.sample_eviction_cost_us.saturating_add(cost_us);
            }
            CacheEvent::StaleRejected => {
                state.stale_rejections = state.stale_rejections.saturating_add(1);
                state.sample_stale_rejections = state.sample_stale_rejections.saturating_add(1);
            }
        }
        Ok(())
    }

    /// Tune the effective TTL using the observed workload. Stale risk and
    /// eviction pressure win over hit-rate improvement.
    pub fn retune(
        &mut self,
        kind: CacheKind,
        workload: u64,
    ) -> Result<CachePolicy, CachePolicyError> {
        let index = self.ensure(kind, workload)?;
        let state = self.states[index].as_mut().expect("cache index is occupied");
        let pressure = state.current_bytes >= state.policy.memory_ceiling_bytes
            || average(state.sample_eviction_cost_us, state.sample_evictions)
                > state.policy.eviction_cost_budget_us;
        if rate(state.sample_stale_rejections, state.sample_accesses)
            > state.policy.max_stale_risk_per_mille
            || pressure
        {
            state.policy.ttl_us = (state.policy.ttl_us.saturating_mul(3) / 4)
                .max(state.policy.min_ttl_us);
        } else if rate(state.sample_hits, state.sample_accesses)
            < state.policy.target_hit_rate_per_mille
        {
            state.policy.ttl_us = (state.policy.ttl_us.saturating_mul(9) / 8)
                .min(state.policy.max_ttl_us);
        }
        state.sample_accesses = 0;
        state.sample_hits = 0;
        state.sample_evictions = 0;
        state.sample_stale_rejections = 0;
        state.sample_eviction_cost_us = 0;
        Ok(state.policy)
    }

    pub fn clear_bytes(
        &mut self,
        kind: CacheKind,
        workload: u64,
    ) -> Result<(), CachePolicyError> {
        let index = self.ensure(kind, workload)?;
        self.states[index]
            .as_mut()
            .expect("cache index is occupied")
            .current_bytes = 0;
        Ok(())
    }

    pub fn reports(&self) -> impl Iterator<Item = CachePolicyReport> + '_ {
        self.states.iter().flatten().copied().map(CachePolicyState::report)
    }

    fn ensure(&mut self, kind: CacheKind, workload: u64) -> Result<usize, CachePolicyError> {
        if workload == 0 {
            return Err(CachePolicyError::InvalidWorkload)
        }
        if let Some(index) = self.index(kind, workload) {
            return Ok(index)
        }
        let policy = kind.default_policy();
        let slot = self
            .states
            .iter_mut()
            .find(|state| state.is_none())
            .ok_or(CachePolicyError::Capacity)?;
        *slot = Some(CachePolicyState::new(kind, workload, policy));
        Ok(self
            .states
            .iter()
            .position(|state| state.is_some_and(|state| state.kind == kind && state.workload == workload))
            .expect("new cache policy is present"))
    }

    fn index(&self, kind: CacheKind, workload: u64) -> Option<usize> {
        self.states.iter().position(|state| {
            state.is_some_and(|state| state.kind == kind && state.workload == workload)
        })
    }
}

impl<const CAPACITY: usize> Default for CachePolicyRegistry<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn validate(workload: u64, policy: CachePolicy) -> Result<(), CachePolicyError> {
    if workload == 0 {
        return Err(CachePolicyError::InvalidWorkload)
    }
    if CachePolicy::new(
        policy.memory_ceiling_bytes,
        policy.target_hit_rate_per_mille,
        policy.max_stale_risk_per_mille,
        policy.eviction_cost_budget_us,
        policy.min_ttl_us,
        policy.max_ttl_us,
        policy.ttl_us,
    )
    .is_none()
    {
        Err(CachePolicyError::InvalidPolicy)
    } else {
        Ok(())
    }
}

fn rate(numerator: u64, denominator: u64) -> u16 {
    if denominator == 0 {
        0
    } else {
        numerator
            .saturating_mul(CACHE_RATE_SCALE)
            .checked_div(denominator)
            .unwrap_or(CACHE_RATE_SCALE)
            .min(CACHE_RATE_SCALE) as u16
    }
}

fn average(total: u64, count: u64) -> u64 {
    if count == 0 { 0 } else { total / count }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policies_are_separate_per_workload_and_report_metrics() {
        let mut registry = CachePolicyRegistry::<8>::new();
        registry.observe(CacheKind::Dns, 1, CacheEvent::Hit).unwrap();
        registry.observe(CacheKind::Dns, 1, CacheEvent::Miss).unwrap();
        registry.observe(CacheKind::Dns, 2, CacheEvent::Miss).unwrap();
        registry.admit(CacheKind::Dns, 1, 512).unwrap();
        registry.observe(
            CacheKind::Dns,
            1,
            CacheEvent::Eviction { bytes: 128, cost_us: 40 },
        )
        .unwrap();

        let mut reports = registry.reports();
        let first = reports.next().expect("first workload report");
        let second = reports.next().expect("second workload report");
        assert_eq!(first.workload, 1);
        assert_eq!(first.hit_rate_per_mille, 500);
        assert_eq!(first.current_bytes, 384);
        assert_eq!(second.workload, 2);
    }

    #[test]
    fn stale_risk_shortens_ttl_and_hit_rate_grows_it() {
        let mut registry = CachePolicyRegistry::<2>::new();
        let initial = registry.policy(CacheKind::Dns, 1).unwrap_or_else(|| {
            registry.observe(CacheKind::Dns, 1, CacheEvent::Miss).unwrap();
            registry.policy(CacheKind::Dns, 1).expect("policy created")
        });
        registry.observe(CacheKind::Dns, 1, CacheEvent::StaleRejected).unwrap();
        let shortened = registry.retune(CacheKind::Dns, 1).unwrap();
        assert!(shortened.ttl_us < initial.ttl_us);

        for _ in 0..20 {
            registry.observe(CacheKind::Dns, 1, CacheEvent::Hit).unwrap();
        }
        let lengthened = registry.retune(CacheKind::Dns, 1).unwrap();
        assert!(lengthened.ttl_us >= shortened.ttl_us);
    }
}
