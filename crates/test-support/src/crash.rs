use std::fmt;

pub use ghostos_durability::{CrashBoundary, CrashDomain, InterruptionInjector};

const DOMAIN_COUNT: usize = 6;
const BOUNDARY_COUNT: usize = 6;

/// One deterministic crash target. Occurrences start at one for each
/// domain/boundary pair.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct CrashPoint {
    pub domain: CrashDomain,
    pub boundary: CrashBoundary,
    pub occurrence: u32,
}

impl CrashPoint {
    pub const fn new(domain: CrashDomain, boundary: CrashBoundary, occurrence: u32) -> Self {
        Self {
            domain,
            boundary,
            occurrence: if occurrence == 0 { 1 } else { occurrence },
        }
    }

    pub fn matrix() -> impl Iterator<Item = Self> {
        CrashDomain::ALL.into_iter().flat_map(|domain| {
            CrashBoundary::ALL
                .into_iter()
                .map(move |boundary| Self::new(domain, boundary, 1))
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CrashEvent {
    pub point: CrashPoint,
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CrashInjected {
    pub point: CrashPoint,
    pub sequence: u64,
}

impl fmt::Display for CrashInjected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "crash injected at {:?}/{:?} occurrence {} (sequence {})",
            self.point.domain,
            self.point.boundary,
            self.point.occurrence,
            self.sequence
        )
    }
}

impl std::error::Error for CrashInjected {}

/// Deterministic, one-shot crash injector shared by persistence tests.
///
/// The harness does not panic or terminate the process. A matching checkpoint
/// returns [`CrashInjected`], allowing the caller to drop the process model,
/// reopen durable state, and verify recovery. The occurrence counters and
/// event log make the same scenario replayable from a seed and target point.
#[derive(Clone, Debug)]
pub struct CrashHarness {
    target: Option<CrashPoint>,
    occurrences: [u32; DOMAIN_COUNT * BOUNDARY_COUNT],
    events: Vec<CrashEvent>,
    next_sequence: u64,
    injected: bool,
}

impl CrashHarness {
    pub fn new(target: Option<CrashPoint>) -> Self {
        Self {
            target,
            occurrences: [0; DOMAIN_COUNT * BOUNDARY_COUNT],
            events: Vec::new(),
            next_sequence: 1,
            injected: false,
        }
    }

    pub fn without_crash() -> Self {
        Self::new(None)
    }

    /// Selects one of the shared first-occurrence matrix cases without using
    /// process randomness.
    pub fn from_seed(seed: u64) -> Self {
        let index = (seed as usize) % (DOMAIN_COUNT * BOUNDARY_COUNT);
        let domain = CrashDomain::ALL[index / BOUNDARY_COUNT];
        let boundary = CrashBoundary::ALL[index % BOUNDARY_COUNT];
        Self::new(Some(CrashPoint::new(domain, boundary, 1)))
    }

    pub fn target(&self) -> Option<CrashPoint> {
        self.target
    }

    pub fn set_target(&mut self, target: Option<CrashPoint>) {
        self.target = target;
        self.reset_observation();
    }

    pub fn reset_observation(&mut self) {
        self.occurrences.fill(0);
        self.events.clear();
        self.next_sequence = 1;
        self.injected = false;
    }

    pub fn events(&self) -> &[CrashEvent] {
        &self.events
    }

    pub fn injected(&self) -> bool {
        self.injected
    }

    pub fn checkpoint(
        &mut self,
        domain: CrashDomain,
        boundary: CrashBoundary,
    ) -> Result<(), CrashInjected> {
        let index = domain.index() * BOUNDARY_COUNT + boundary.index();
        self.occurrences[index] = self.occurrences[index].saturating_add(1);
        let point = CrashPoint::new(domain, boundary, self.occurrences[index]);
        let event = CrashEvent {
            point,
            sequence: self.next_sequence,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.events.push(event);
        if !self.injected && self.target == Some(point) {
            self.injected = true;
            return Err(CrashInjected {
                point,
                sequence: event.sequence,
            });
        }
        Ok(())
    }
}

impl Default for CrashHarness {
    fn default() -> Self {
        Self::without_crash()
    }
}

impl InterruptionInjector for CrashHarness {
    fn checkpoint(&mut self, domain: CrashDomain, boundary: CrashBoundary) -> bool {
        Self::checkpoint(self, domain, boundary).is_err()
    }
}
