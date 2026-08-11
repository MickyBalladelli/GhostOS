//! Bounded service-level objectives and error-budget accounting.

pub const SLO_COUNT: usize = 9;
pub const BUDGET_SCALE: u64 = 1_000_000;
pub const DEFAULT_OBJECTIVE_PER_MILLION: u64 = 999_000;
pub const DEFAULT_WINDOW_US: u64 = 30 * 24 * 60 * 60 * 1_000_000;
pub const DEFAULT_MAX_AGE_US: u64 = 24 * 60 * 60 * 1_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SloKind {
    Boot = 0,
    InteractiveShell = 1,
    Ipc = 2,
    StorageCommit = 3,
    Dhcp = 4,
    Rpc = 5,
    PackageActivation = 6,
    SnapshotRestore = 7,
    ClusterConvergence = 8,
}

impl SloKind {
    pub const ALL: [Self; SLO_COUNT] = [
        Self::Boot,
        Self::InteractiveShell,
        Self::Ipc,
        Self::StorageCommit,
        Self::Dhcp,
        Self::Rpc,
        Self::PackageActivation,
        Self::SnapshotRestore,
        Self::ClusterConvergence,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Boot => "boot",
            Self::InteractiveShell => "interactive-shell",
            Self::Ipc => "ipc",
            Self::StorageCommit => "storage-commit",
            Self::Dhcp => "dhcp",
            Self::Rpc => "rpc",
            Self::PackageActivation => "package-activation",
            Self::SnapshotRestore => "snapshot-restore",
            Self::ClusterConvergence => "cluster-convergence",
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn definition(self) -> SloDefinition {
        SloDefinition {
            kind: self,
            objective_per_million: DEFAULT_OBJECTIVE_PER_MILLION,
            window_us: DEFAULT_WINDOW_US,
            max_age_us: DEFAULT_MAX_AGE_US,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SloDefinition {
    pub kind: SloKind,
    pub objective_per_million: u64,
    pub window_us: u64,
    pub max_age_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SloObservation {
    pub kind: SloKind,
    pub window_started_at_us: u64,
    pub observed_at_us: u64,
    pub total_events: u64,
    pub bad_events: u64,
}

impl SloObservation {
    pub const fn new(
        kind: SloKind,
        window_started_at_us: u64,
        observed_at_us: u64,
        total_events: u64,
        bad_events: u64,
    ) -> Option<Self> {
        if window_started_at_us == 0
            || observed_at_us < window_started_at_us
            || total_events == 0
            || bad_events > total_events
        {
            None
        } else {
            Some(Self {
                kind,
                window_started_at_us,
                observed_at_us,
                total_events,
                bad_events,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SloMeasurement {
    pub definition: SloDefinition,
    pub window_started_at_us: u64,
    pub observed_at_us: u64,
    pub total_events: u64,
    pub bad_events: u64,
    pub allowed_bad_events: u64,
    pub remaining_bad_events: u64,
    pub consumed_per_million: u64,
}

impl SloMeasurement {
    pub const fn within_budget(self) -> bool {
        self.bad_events <= self.allowed_bad_events
    }

    pub const fn budget_exhausted(self) -> bool {
        !self.within_budget()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SloError {
    InvalidReportTimestamp,
    InvalidDefinition,
    InvalidObservation,
    Duplicate,
    Capacity,
    Missing,
    Stale,
    ClockSkew,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SloReportStatus {
    pub total: usize,
    pub fresh: usize,
    pub missing: usize,
    pub stale: usize,
    pub exhausted: usize,
}

impl SloReportStatus {
    pub const fn release_ready(self) -> bool {
        self.total == SLO_COUNT
            && self.fresh == SLO_COUNT
            && self.missing == 0
            && self.stale == 0
            && self.exhausted == 0
    }
}

#[derive(Clone, Copy)]
pub struct SloReport {
    generated_at_us: u64,
    observations: [Option<SloObservation>; SLO_COUNT],
}

impl SloReport {
    pub const fn new(generated_at_us: u64) -> Self {
        Self {
            generated_at_us,
            observations: [None; SLO_COUNT],
        }
    }

    pub const fn generated_at_us(&self) -> u64 {
        self.generated_at_us
    }

    pub const fn set_generated_at_us(&mut self, generated_at_us: u64) -> Result<(), SloError> {
        if generated_at_us == 0 {
            Err(SloError::InvalidReportTimestamp)
        } else {
            self.generated_at_us = generated_at_us;
            Ok(())
        }
    }

    pub fn observations(&self) -> impl Iterator<Item = SloObservation> + '_ {
        self.observations.iter().flatten().copied()
    }

    pub fn record(&mut self, observation: SloObservation) -> Result<(), SloError> {
        if SloObservation::new(
            observation.kind,
            observation.window_started_at_us,
            observation.observed_at_us,
            observation.total_events,
            observation.bad_events,
        )
        .is_none()
        {
            return Err(SloError::InvalidObservation)
        }
        if self.generated_at_us != 0 && observation.observed_at_us > self.generated_at_us {
            return Err(SloError::ClockSkew)
        }
        if self
            .observations()
            .any(|entry| entry.kind == observation.kind)
        {
            return Err(SloError::Duplicate)
        }
        let slot = self
            .observations
            .get_mut(observation.kind.index())
            .ok_or(SloError::Capacity)?;
        *slot = Some(observation);
        Ok(())
    }

    pub fn measurement_at(
        &self,
        kind: SloKind,
        now_us: u64,
    ) -> Result<SloMeasurement, SloError> {
        if now_us == 0 || self.generated_at_us == 0 {
            return Err(SloError::InvalidReportTimestamp)
        }
        if now_us < self.generated_at_us {
            return Err(SloError::ClockSkew)
        }
        let observation = self.observations[kind.index()].ok_or(SloError::Missing)?;
        if now_us < observation.observed_at_us {
            return Err(SloError::ClockSkew)
        }
        let definition = kind.definition();
        if definition.objective_per_million == 0
            || definition.objective_per_million > BUDGET_SCALE
            || definition.window_us == 0
            || definition.max_age_us == 0
        {
            return Err(SloError::InvalidDefinition)
        }
        if now_us - observation.observed_at_us > definition.max_age_us
            || observation.observed_at_us - observation.window_started_at_us > definition.window_us
        {
            return Err(SloError::Stale)
        }
        let allowed_bad_events = allowed_bad_events(
            observation.total_events,
            definition.objective_per_million,
        );
        let remaining_bad_events = allowed_bad_events.saturating_sub(observation.bad_events);
        let consumed_per_million = if allowed_bad_events == 0 {
            if observation.bad_events == 0 { 0 } else { BUDGET_SCALE }
        } else {
            ((observation.bad_events as u128 * BUDGET_SCALE as u128)
                / allowed_bad_events as u128)
                .min(BUDGET_SCALE as u128) as u64
        };
        Ok(SloMeasurement {
            definition,
            window_started_at_us: observation.window_started_at_us,
            observed_at_us: observation.observed_at_us,
            total_events: observation.total_events,
            bad_events: observation.bad_events,
            allowed_bad_events,
            remaining_bad_events,
            consumed_per_million,
        })
    }

    pub fn status(&self, now_us: u64) -> SloReportStatus {
        let mut status = SloReportStatus {
            total: SLO_COUNT,
            fresh: 0,
            missing: 0,
            stale: 0,
            exhausted: 0,
        };
        for kind in SloKind::ALL {
            match self.measurement_at(kind, now_us) {
                Ok(measurement) => {
                    status.fresh += 1;
                    if measurement.budget_exhausted() {
                        status.exhausted += 1
                    }
                }
                Err(SloError::Missing) => status.missing += 1,
                Err(SloError::Stale) => status.stale += 1,
                Err(_) => status.stale += 1,
            }
        }
        status
    }
}

impl Default for SloReport {
    fn default() -> Self {
        Self::new(0)
    }
}

const fn allowed_bad_events(total_events: u64, objective_per_million: u64) -> u64 {
    let allowed = total_events as u128 * (BUDGET_SCALE - objective_per_million) as u128;
    ((allowed + BUDGET_SCALE as u128 - 1) / BUDGET_SCALE as u128) as u64
}
