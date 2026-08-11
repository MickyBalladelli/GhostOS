use crate::{BLOCK_SIZE, Error, FileName, SynFs};

pub const DEFAULT_FORECAST_HORIZON_US: u64 = 24 * 60 * 60 * 1_000_000;
pub const DEFAULT_ALLOCATION_RESERVE_BYTES: u64 = 8 * BLOCK_SIZE as u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CapacityResource {
    SynFs = 1,
    PackageCache = 2,
    Journal = 3,
    Snapshot = 4,
    Log = 5,
    ClusterMetadata = 6,
}

impl CapacityResource {
    pub const fn name(self) -> &'static str {
        match self {
            Self::SynFs => "synfs",
            Self::PackageCache => "package-cache",
            Self::Journal => "journal",
            Self::Snapshot => "snapshot",
            Self::Log => "log",
            Self::ClusterMetadata => "cluster-metadata",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapacityObservation {
    pub resource: CapacityResource,
    pub sampled_at_us: u64,
    pub capacity_bytes: u64,
    pub allocated_bytes: u64,
    pub reclaimable_bytes: u64,
    pub fragmented_bytes: u64,
    pub largest_free_extent_bytes: u64,
    pub allocation_unit_bytes: u64,
    pub gc_pending_bytes: u64,
    pub gc_work_limit_bytes: u64,
    pub growth_bytes_per_hour: u64,
}

impl CapacityObservation {
    pub const fn empty(resource: CapacityResource, sampled_at_us: u64) -> Self {
        Self {
            resource,
            sampled_at_us,
            capacity_bytes: 0,
            allocated_bytes: 0,
            reclaimable_bytes: 0,
            fragmented_bytes: 0,
            largest_free_extent_bytes: 0,
            allocation_unit_bytes: 1,
            gc_pending_bytes: 0,
            gc_work_limit_bytes: 0,
            growth_bytes_per_hour: 0,
        }
    }

    pub fn forecast(self, horizon_us: u64) -> CapacityForecast {
        let horizon_growth = self
            .growth_bytes_per_hour
            .saturating_mul(horizon_us)
            / 3_600_000_000;
        let forecast_allocated_bytes = self.allocated_bytes.saturating_add(horizon_growth);
        let free_bytes = self.capacity_bytes.saturating_sub(self.allocated_bytes);
        let forecast_free_bytes = self
            .capacity_bytes
            .saturating_sub(forecast_allocated_bytes);
        let reserve_bytes = self
            .allocation_unit_bytes
            .max(DEFAULT_ALLOCATION_RESERVE_BYTES)
            .saturating_mul(2);
        let warning_free_bytes = reserve_bytes.max(self.capacity_bytes / 10);
        let critical_free_bytes = reserve_bytes;
        let hours_to_warning = hours_until(self.growth_bytes_per_hour, free_bytes, warning_free_bytes);
        let hours_to_failure = hours_until(self.growth_bytes_per_hour, free_bytes, 0);
        CapacityForecast {
            resource: self.resource,
            sampled_at_us: self.sampled_at_us,
            capacity_bytes: self.capacity_bytes,
            allocated_bytes: self.allocated_bytes,
            free_bytes,
            reclaimable_bytes: self.reclaimable_bytes,
            fragmented_bytes: self.fragmented_bytes,
            largest_free_extent_bytes: self.largest_free_extent_bytes,
            growth_bytes_per_hour: self.growth_bytes_per_hour,
            forecast_allocated_bytes,
            forecast_free_bytes,
            warning_free_bytes,
            critical_free_bytes,
            hours_to_warning,
            hours_to_failure,
            gc_pending_bytes: self.gc_pending_bytes,
            gc_work_limit_bytes: self.gc_work_limit_bytes,
            gc_bounded: self.gc_work_limit_bytes != 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapacityForecast {
    pub resource: CapacityResource,
    pub sampled_at_us: u64,
    pub capacity_bytes: u64,
    pub allocated_bytes: u64,
    pub free_bytes: u64,
    pub reclaimable_bytes: u64,
    pub fragmented_bytes: u64,
    pub largest_free_extent_bytes: u64,
    pub growth_bytes_per_hour: u64,
    pub forecast_allocated_bytes: u64,
    pub forecast_free_bytes: u64,
    pub warning_free_bytes: u64,
    pub critical_free_bytes: u64,
    pub hours_to_warning: u64,
    pub hours_to_failure: u64,
    pub gc_pending_bytes: u64,
    pub gc_work_limit_bytes: u64,
    pub gc_bounded: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FragmentationReport {
    pub capacity_blocks: usize,
    pub allocated_blocks: usize,
    pub live_blocks: usize,
    pub reclaimable_blocks: usize,
    pub free_blocks: usize,
    pub free_runs: usize,
    pub largest_free_run: usize,
    pub fragmented_blocks: usize,
    pub fragmentation_percent: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SynFsPathDiagnostics {
    pub retained_versions: u64,
    pub retained_bytes: u64,
    pub oldest_version: Option<u32>,
    pub newest_version: Option<u32>,
}

fn hours_until(growth_bytes_per_hour: u64, free_bytes: u64, threshold_bytes: u64) -> u64 {
    if free_bytes <= threshold_bytes {
        return 0
    }
    if growth_bytes_per_hour == 0 {
        return u64::MAX
    }
    (free_bytes - threshold_bytes)
        .saturating_add(growth_bytes_per_hour - 1)
        / growth_bytes_per_hour
}

impl<const MAX_BLOCKS: usize> SynFs<MAX_BLOCKS> {
    pub fn fragmentation_report(&self) -> Result<FragmentationReport, Error> {
        let diagnostics = self.diagnostics()?;
        let mut free_runs = 0;
        let mut largest_free_run = 0;
        let mut current_free_run = 0;
        for slot in &self.arena.slots {
            if slot.block.is_none() {
                current_free_run += 1;
                continue
            }
            if current_free_run != 0 {
                free_runs += 1;
                largest_free_run = largest_free_run.max(current_free_run);
                current_free_run = 0
            }
        }
        if current_free_run != 0 {
            free_runs += 1;
            largest_free_run = largest_free_run.max(current_free_run)
        }
        let fragmented_blocks = diagnostics
            .allocated_blocks
            .saturating_sub(diagnostics.live_blocks);
        let fragmentation_percent = if diagnostics.capacity_blocks == 0 {
            0
        } else {
            (fragmented_blocks as u64).saturating_mul(100)
                / diagnostics.capacity_blocks as u64
        };
        Ok(FragmentationReport {
            capacity_blocks: diagnostics.capacity_blocks,
            allocated_blocks: diagnostics.allocated_blocks,
            live_blocks: diagnostics.live_blocks,
            reclaimable_blocks: fragmented_blocks,
            free_blocks: diagnostics.free_blocks,
            free_runs,
            largest_free_run,
            fragmented_blocks,
            fragmentation_percent,
        })
    }

    pub fn capacity_observation(
        &self,
        resource: CapacityResource,
        sampled_at_us: u64,
        growth_bytes_per_hour: u64,
    ) -> Result<CapacityObservation, Error> {
        let diagnostics = self.diagnostics()?;
        let fragmentation = self.fragmentation_report()?;
        let block_bytes = BLOCK_SIZE as u64;
        Ok(CapacityObservation {
            resource,
            sampled_at_us,
            capacity_bytes: (diagnostics.capacity_blocks as u64).saturating_mul(block_bytes),
            allocated_bytes: (diagnostics.allocated_blocks as u64).saturating_mul(block_bytes),
            reclaimable_bytes: (fragmentation.reclaimable_blocks as u64).saturating_mul(block_bytes),
            fragmented_bytes: (fragmentation.fragmented_blocks as u64).saturating_mul(block_bytes),
            largest_free_extent_bytes: (fragmentation.largest_free_run as u64).saturating_mul(block_bytes),
            allocation_unit_bytes: block_bytes,
            gc_pending_bytes: (fragmentation.reclaimable_blocks as u64).saturating_mul(block_bytes),
            gc_work_limit_bytes: (MAX_BLOCKS as u64).saturating_mul(block_bytes),
            growth_bytes_per_hour,
        })
    }

    pub fn path_diagnostics(&self, path: &str) -> Result<SynFsPathDiagnostics, Error> {
        let (retained_versions, oldest_version) = self.retained_version_span(path)?;
        let file = FileName::new(path)?;
        let mut retained_bytes = 0u64;
        let mut ordinal = 0;
        while let Some(record) = self.record_at(self.root, ordinal)? {
            if record.key.file == file {
                retained_bytes = retained_bytes.saturating_add(record.size);
            }
            ordinal = ordinal.saturating_add(1)
        }
        Ok(SynFsPathDiagnostics {
            retained_versions: retained_versions as u64,
            retained_bytes,
            oldest_version,
            newest_version: self.lookup(path).ok().map(|version| version.version),
        })
    }
}
