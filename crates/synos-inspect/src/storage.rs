use synos_fabric::NodeId;
use synos_synfs::{
    CapacityForecast, CapacityObservation, CapacityResource, DEFAULT_FORECAST_HORIZON_US,
    BLOCK_SIZE,
    DeviceHealth as SynFsDeviceHealth, PoolLayout, StorageClass, StoragePoolAdmin, SynFs,
};

use crate::{InspectError, Name};

pub const MAX_STORAGE_DEVICES: usize = 32;
pub const MAX_SYNFS_VOLUMES: usize = 16;
pub const MAX_CAPACITY_SAMPLES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageKind {
    Ahci,
    Nvme,
    CxlPersistentMemory,
    NetworkBlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceHealth {
    Online,
    Degraded,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageDeviceSample {
    pub id: u64,
    pub node: NodeId,
    pub kind: StorageKind,
    pub health: DeviceHealth,
    pub capacity_bytes: u64,
    pub allocated_bytes: u64,
    pub media_errors: u64,
    pub temperature_millicelsius: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SynFsVolumeSample {
    pub id: u64,
    pub node: NodeId,
    pub name: Name<32>,
    pub generation: u64,
    pub capacity_bytes: u64,
    pub used_bytes: u64,
    pub cow_overhead_bytes: u64,
    pub retained_versions: u64,
    pub checkpoints: u32,
    pub quota_max_bytes: u64,
    pub quota_used_bytes: u64,
    pub quota_max_files: u64,
    pub quota_used_files: u64,
    pub quota_max_blocks: u64,
    pub quota_used_blocks: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapacitySample {
    pub id: u64,
    pub node: NodeId,
    pub name: Name<32>,
    pub observation: CapacityObservation,
    pub forecast: CapacityForecast,
}

#[derive(Clone, Copy)]
pub struct StorageReport {
    sampled_at_us: u64,
    devices: [Option<StorageDeviceSample>; MAX_STORAGE_DEVICES],
    volumes: [Option<SynFsVolumeSample>; MAX_SYNFS_VOLUMES],
    capacity: [Option<CapacitySample>; MAX_CAPACITY_SAMPLES],
}

impl StorageReport {
    pub const fn new() -> Self {
        Self {
            sampled_at_us: 0,
            devices: [None; MAX_STORAGE_DEVICES],
            volumes: [None; MAX_SYNFS_VOLUMES],
            capacity: [None; MAX_CAPACITY_SAMPLES],
        }
    }

    pub const fn sampled_at_us(&self) -> u64 {
        self.sampled_at_us
    }

    pub fn set_sampled_at_us(&mut self, sampled_at_us: u64) {
        self.sampled_at_us = sampled_at_us
    }

    pub fn devices(&self) -> impl Iterator<Item = StorageDeviceSample> + '_ {
        self.devices.iter().flatten().copied()
    }

    pub fn volumes(&self) -> impl Iterator<Item = SynFsVolumeSample> + '_ {
        self.volumes.iter().flatten().copied()
    }

    pub fn capacity(&self) -> impl Iterator<Item = CapacitySample> + '_ {
        self.capacity.iter().flatten().copied()
    }

    pub fn push_device(&mut self, sample: StorageDeviceSample) -> Result<(), InspectError> {
        if sample.id == 0
            || sample.capacity_bytes == 0
            || sample.allocated_bytes > sample.capacity_bytes
            || self.devices().any(|entry| entry.id == sample.id)
        {
            return Err(InspectError::InvalidSample);
        }
        insert(&mut self.devices, sample)
    }

    pub fn push_volume(&mut self, sample: SynFsVolumeSample) -> Result<(), InspectError> {
        if sample.id == 0
            || sample.capacity_bytes == 0
            || sample.used_bytes > sample.capacity_bytes
            || sample.cow_overhead_bytes > sample.used_bytes
            || exceeds_quota(sample.quota_used_bytes, sample.quota_max_bytes)
            || exceeds_quota(sample.quota_used_files, sample.quota_max_files)
            || exceeds_quota(sample.quota_used_blocks, sample.quota_max_blocks)
            || self.volumes().any(|entry| entry.id == sample.id)
        {
            return Err(InspectError::InvalidSample);
        }
        insert(&mut self.volumes, sample)
    }

    pub fn push_capacity(
        &mut self,
        id: u64,
        node: NodeId,
        name: &str,
        observation: CapacityObservation,
    ) -> Result<(), InspectError> {
        if id == 0
            || observation.capacity_bytes == 0
            || observation.allocated_bytes > observation.capacity_bytes
            || observation.reclaimable_bytes > observation.allocated_bytes
            || observation.fragmented_bytes > observation.allocated_bytes
            || observation.largest_free_extent_bytes > observation.capacity_bytes
            || self.capacity().any(|entry| entry.id == id)
        {
            return Err(InspectError::InvalidSample);
        }
        let forecast = observation.forecast(DEFAULT_FORECAST_HORIZON_US);
        insert(
            &mut self.capacity,
            CapacitySample {
                id,
                node,
                name: Name::new(name)?,
                observation,
                forecast,
            },
        )
    }

    pub fn push_synfs<const BLOCKS: usize>(
        &mut self,
        id: u64,
        node: NodeId,
        name: &str,
        filesystem: &SynFs<BLOCKS>,
    ) -> Result<(), InspectError> {
        let diagnostics = filesystem
            .diagnostics()
            .map_err(|_| InspectError::InvalidSample)?;
        self.push_volume(SynFsVolumeSample {
            id,
            node,
            name: Name::new(name)?,
            generation: diagnostics.generation,
            capacity_bytes: (diagnostics.capacity_blocks as u64).saturating_mul(BLOCK_SIZE as u64),
            used_bytes: (diagnostics.live_blocks as u64).saturating_mul(BLOCK_SIZE as u64),
            cow_overhead_bytes: (diagnostics.cow_snapshot_blocks as u64)
                .saturating_mul(BLOCK_SIZE as u64),
            retained_versions: diagnostics.retained_versions,
            checkpoints: diagnostics.checkpoints as u32,
            quota_max_bytes: diagnostics.max_bytes,
            quota_used_bytes: diagnostics.retained_bytes,
            quota_max_files: diagnostics.max_files,
            quota_used_files: diagnostics.file_count,
            quota_max_blocks: if diagnostics.max_blocks == usize::MAX {
                u64::MAX
            } else {
                diagnostics.max_blocks as u64
            },
            quota_used_blocks: diagnostics.allocated_blocks as u64,
        })?;
        self.push_synfs_capacity(id, node, name, CapacityResource::SynFs, filesystem, 0)
    }

    pub fn push_synfs_capacity<const BLOCKS: usize>(
        &mut self,
        id: u64,
        node: NodeId,
        name: &str,
        resource: CapacityResource,
        filesystem: &SynFs<BLOCKS>,
        growth_bytes_per_hour: u64,
    ) -> Result<(), InspectError> {
        let observation = filesystem
            .capacity_observation(resource, self.sampled_at_us, growth_bytes_per_hour)
            .map_err(|_| InspectError::InvalidSample)?;
        self.push_capacity(id, node, name, observation)
    }

    pub fn append_pool_admin<const DEVICES: usize, const POOLS: usize>(
        &mut self,
        node: NodeId,
        admin: &StoragePoolAdmin<DEVICES, POOLS>,
    ) -> Result<(), InspectError> {
        for device in admin.devices() {
            let allocated_blocks = admin
                .pools()
                .filter(|pool| pool.members().any(|member| member == device.id))
                .fold(0u64, |allocated, pool| {
                    let device_blocks = match pool.layout {
                        PoolLayout::Stripe => {
                            pool.allocated_blocks
                                .saturating_add(pool.member_count() as u64 - 1)
                                / pool.member_count() as u64
                        }
                        PoolLayout::Mirror => pool.allocated_blocks,
                    };
                    allocated.saturating_add(device_blocks)
                });
            self.push_device(StorageDeviceSample {
                id: device.id.raw(),
                node,
                kind: match device.class {
                    StorageClass::Ahci => StorageKind::Ahci,
                    StorageClass::Nvme => StorageKind::Nvme,
                    StorageClass::CxlPersistentMemory => StorageKind::CxlPersistentMemory,
                    StorageClass::NetworkBlock => StorageKind::NetworkBlock,
                },
                health: match device.health {
                    SynFsDeviceHealth::Online => DeviceHealth::Online,
                    SynFsDeviceHealth::Draining => DeviceHealth::Degraded,
                    SynFsDeviceHealth::Failed => DeviceHealth::Failed,
                },
                capacity_bytes: device
                    .capacity_blocks
                    .saturating_mul(device.block_size as u64),
                allocated_bytes: allocated_blocks.saturating_mul(device.block_size as u64),
                media_errors: 0,
                temperature_millicelsius: 0,
            })?
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        *self = Self::new()
    }

    pub(crate) fn retain_node(&mut self, node: NodeId) {
        for entry in &mut self.devices {
            if entry.is_some_and(|sample| sample.node != node) {
                *entry = None
            }
        }
        for entry in &mut self.volumes {
            if entry.is_some_and(|sample| sample.node != node) {
                *entry = None
            }
        }
        for entry in &mut self.capacity {
            if entry.is_some_and(|sample| sample.node != node) {
                *entry = None
            }
        }
    }
}

impl Default for StorageReport {
    fn default() -> Self {
        Self::new()
    }
}

fn insert<T: Copy, const CAPACITY: usize>(
    entries: &mut [Option<T>; CAPACITY],
    sample: T,
) -> Result<(), InspectError> {
    let slot = entries
        .iter_mut()
        .find(|entry| entry.is_none())
        .ok_or(InspectError::Capacity)?;
    *slot = Some(sample);
    Ok(())
}

fn exceeds_quota(used: u64, limit: u64) -> bool {
    limit != u64::MAX && used > limit
}
