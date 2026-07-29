use core::fmt;
use synos_status::{IntoStatus, Status};

pub const MAX_POOL_NAME_BYTES: usize = 32;
pub const MAX_POOL_MEMBERS: usize = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct StorageDeviceId(u64);

impl StorageDeviceId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct StoragePoolId(u32);

impl StoragePoolId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct PoolName {
    bytes: [u8; MAX_POOL_NAME_BYTES],
    length: u8,
}

impl PoolName {
    pub fn new(name: &str) -> Result<Self, StoragePoolError> {
        if name.is_empty()
            || name.len() > MAX_POOL_NAME_BYTES
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(StoragePoolError::InvalidName)
        }
        let mut bytes = [0; MAX_POOL_NAME_BYTES];
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        Ok(Self {
            bytes,
            length: name.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.length as usize])
            .expect("PoolName invariant")
    }
}

impl fmt::Debug for PoolName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("PoolName").field(&self.as_str()).finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageClass {
    Nvme,
    CxlPersistentMemory,
    NetworkBlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceHealth {
    Online,
    Draining,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageDevice {
    pub id: StorageDeviceId,
    pub class: StorageClass,
    pub capacity_blocks: u64,
    pub block_size: u32,
    /// Devices sharing a fault domain may fail together.
    pub fault_domain: u32,
    pub health: DeviceHealth,
}

impl StorageDevice {
    fn validate(self) -> Result<Self, StoragePoolError> {
        if self.capacity_blocks == 0
            || self.block_size < 512
            || !self.block_size.is_power_of_two()
        {
            return Err(StoragePoolError::InvalidDevice)
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PoolLayout {
    Stripe,
    Mirror,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PoolHealth {
    Online,
    Degraded,
    Offline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoragePool {
    pub id: StoragePoolId,
    pub name: PoolName,
    pub layout: PoolLayout,
    pub block_size: u32,
    pub capacity_blocks: u64,
    pub allocated_blocks: u64,
    members: [Option<StorageDeviceId>; MAX_POOL_MEMBERS],
    member_count: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockPlacement {
    pub device: StorageDeviceId,
    pub device_block: u64,
    /// Zero for stripes; mirror copy number for mirrored pools.
    pub replica: u8,
}

impl StoragePool {
    pub fn members(&self) -> impl Iterator<Item = StorageDeviceId> + '_ {
        self.members.iter().flatten().copied()
    }

    pub const fn member_count(&self) -> u8 {
        self.member_count
    }

    pub const fn free_blocks(&self) -> u64 {
        self.capacity_blocks - self.allocated_blocks
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoragePoolError {
    AlreadyExists,
    Busy,
    CapacityOverflow,
    DeviceAlreadyClaimed,
    DeviceFailed,
    DeviceNotFound,
    DuplicateMember,
    FaultDomainConflict,
    IncompatibleBlockSize,
    InvalidDevice,
    InvalidLayout,
    InvalidName,
    NoSpace,
    PoolNotFound,
    RegistryFull,
}

impl IntoStatus for StoragePoolError {
    fn status(self) -> Status {
        match self {
            Self::DeviceNotFound | Self::PoolNotFound => Status::NOT_FOUND,
            Self::Busy | Self::DeviceAlreadyClaimed => Status::BUSY,
            Self::NoSpace | Self::RegistryFull => Status::NO_SPACE,
            Self::DeviceFailed => Status::CORRUPT,
            Self::AlreadyExists
            | Self::CapacityOverflow
            | Self::DuplicateMember
            | Self::FaultDomainConflict
            | Self::IncompatibleBlockSize
            | Self::InvalidDevice
            | Self::InvalidLayout
            | Self::InvalidName => Status::INVALID_ARGUMENT,
        }
    }
}

/// Heap-free SynFS pool control plane for local and remote block providers.
pub struct StoragePoolAdmin<const DEVICES: usize = 32, const POOLS: usize = 8> {
    devices: [Option<StorageDevice>; DEVICES],
    pools: [Option<StoragePool>; POOLS],
}

impl<const DEVICES: usize, const POOLS: usize> StoragePoolAdmin<DEVICES, POOLS> {
    pub const fn new() -> Self {
        Self {
            devices: [None; DEVICES],
            pools: [None; POOLS],
        }
    }

    pub fn register_device(
        &mut self,
        device: StorageDevice,
    ) -> Result<(), StoragePoolError> {
        let device = device.validate()?;
        if self
            .devices
            .iter()
            .flatten()
            .any(|existing| existing.id == device.id)
        {
            return Err(StoragePoolError::AlreadyExists)
        }
        let slot = self
            .devices
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(StoragePoolError::RegistryFull)?;
        *slot = Some(device);
        Ok(())
    }

    pub fn unregister_device(
        &mut self,
        id: StorageDeviceId,
    ) -> Result<StorageDevice, StoragePoolError> {
        if self
            .pools
            .iter()
            .flatten()
            .any(|pool| pool.members().any(|member| member == id))
        {
            return Err(StoragePoolError::DeviceAlreadyClaimed)
        }
        let slot = self
            .devices
            .iter_mut()
            .find(|entry| entry.is_some_and(|device| device.id == id))
            .ok_or(StoragePoolError::DeviceNotFound)?;
        slot.take().ok_or(StoragePoolError::DeviceNotFound)
    }

    pub fn set_device_health(
        &mut self,
        id: StorageDeviceId,
        health: DeviceHealth,
    ) -> Result<(), StoragePoolError> {
        self.device_mut(id)?.health = health;
        Ok(())
    }

    pub fn create_pool(
        &mut self,
        id: StoragePoolId,
        name: &str,
        layout: PoolLayout,
        members: &[StorageDeviceId],
    ) -> Result<StoragePool, StoragePoolError> {
        if members.is_empty()
            || members.len() > MAX_POOL_MEMBERS
            || (layout == PoolLayout::Mirror && members.len() < 2)
        {
            return Err(StoragePoolError::InvalidLayout)
        }
        if self
            .pools
            .iter()
            .flatten()
            .any(|pool| pool.id == id || pool.name.as_str() == name)
        {
            return Err(StoragePoolError::AlreadyExists)
        }

        let mut selected = [None; MAX_POOL_MEMBERS];
        let mut block_size = None;
        let mut member_capacity = u64::MAX;
        for (index, member) in members.iter().copied().enumerate() {
            if members[..index].contains(&member) {
                return Err(StoragePoolError::DuplicateMember)
            }
            if self
                .pools
                .iter()
                .flatten()
                .any(|pool| pool.members().any(|claimed| claimed == member))
            {
                return Err(StoragePoolError::DeviceAlreadyClaimed)
            }
            let device = self.device(member)?;
            if device.health != DeviceHealth::Online {
                return Err(StoragePoolError::DeviceFailed)
            }
            if block_size.is_some_and(|size| size != device.block_size) {
                return Err(StoragePoolError::IncompatibleBlockSize)
            }
            if layout == PoolLayout::Mirror
                && members[..index].iter().copied().any(|other| {
                    self.device(other)
                        .is_ok_and(|existing| existing.fault_domain == device.fault_domain)
                })
            {
                return Err(StoragePoolError::FaultDomainConflict)
            }
            block_size = Some(device.block_size);
            member_capacity = member_capacity.min(device.capacity_blocks);
            selected[index] = Some(member)
        }

        let capacity_blocks = match layout {
            PoolLayout::Stripe => member_capacity
                .checked_mul(members.len() as u64)
                .ok_or(StoragePoolError::CapacityOverflow)?,
            PoolLayout::Mirror => member_capacity,
        };
        let pool = StoragePool {
            id,
            name: PoolName::new(name)?,
            layout,
            block_size: block_size.ok_or(StoragePoolError::InvalidLayout)?,
            capacity_blocks,
            allocated_blocks: 0,
            members: selected,
            member_count: members.len() as u8,
        };
        let slot = self
            .pools
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(StoragePoolError::RegistryFull)?;
        *slot = Some(pool);
        Ok(pool)
    }

    pub fn destroy_pool(
        &mut self,
        id: StoragePoolId,
    ) -> Result<StoragePool, StoragePoolError> {
        let slot = self
            .pools
            .iter_mut()
            .find(|entry| entry.is_some_and(|pool| pool.id == id))
            .ok_or(StoragePoolError::PoolNotFound)?;
        if slot.is_some_and(|pool| pool.allocated_blocks != 0) {
            return Err(StoragePoolError::Busy)
        }
        slot.take().ok_or(StoragePoolError::PoolNotFound)
    }

    pub fn attach_device(
        &mut self,
        id: StoragePoolId,
        device_id: StorageDeviceId,
    ) -> Result<StoragePool, StoragePoolError> {
        let current = self.pool(id)?;
        if current.member_count as usize == MAX_POOL_MEMBERS {
            return Err(StoragePoolError::RegistryFull)
        }
        if current.allocated_blocks != 0 {
            return Err(StoragePoolError::Busy)
        }
        if self
            .pools
            .iter()
            .flatten()
            .any(|pool| pool.members().any(|member| member == device_id))
        {
            return Err(StoragePoolError::DeviceAlreadyClaimed)
        }
        let device = self.device(device_id)?;
        if device.health != DeviceHealth::Online {
            return Err(StoragePoolError::DeviceFailed)
        }
        if device.block_size != current.block_size {
            return Err(StoragePoolError::IncompatibleBlockSize)
        }
        if current.layout == PoolLayout::Mirror {
            if device.capacity_blocks < current.capacity_blocks {
                return Err(StoragePoolError::NoSpace)
            }
            if current.members().any(|member| {
                self.device(member)
                    .is_ok_and(|existing| existing.fault_domain == device.fault_domain)
            }) {
                return Err(StoragePoolError::FaultDomainConflict)
            }
        } else {
            let member_capacity = current.capacity_blocks / current.member_count as u64;
            if device.capacity_blocks < member_capacity {
                return Err(StoragePoolError::NoSpace)
            }
        }

        let pool = self.pool_mut(id)?;
        pool.members[pool.member_count as usize] = Some(device_id);
        pool.member_count += 1;
        if pool.layout == PoolLayout::Stripe {
            let member_capacity =
                pool.capacity_blocks / (pool.member_count as u64 - 1);
            pool.capacity_blocks = member_capacity
                .checked_mul(pool.member_count as u64)
                .ok_or(StoragePoolError::CapacityOverflow)?
        }
        Ok(*pool)
    }

    pub fn detach_device(
        &mut self,
        id: StoragePoolId,
        device_id: StorageDeviceId,
    ) -> Result<StoragePool, StoragePoolError> {
        let current = self.pool(id)?;
        let minimum_members = match current.layout {
            PoolLayout::Stripe => 1,
            PoolLayout::Mirror => 2,
        };
        if current.member_count as usize <= minimum_members {
            return Err(StoragePoolError::InvalidLayout)
        }
        if current.layout == PoolLayout::Stripe && current.allocated_blocks != 0 {
            return Err(StoragePoolError::Busy)
        }
        let Some(index) = current.members().position(|member| member == device_id) else {
            return Err(StoragePoolError::DeviceNotFound)
        };
        let new_capacity = match current.layout {
            PoolLayout::Stripe => {
                let member_capacity =
                    current.capacity_blocks / current.member_count as u64;
                member_capacity
                    .checked_mul(current.member_count as u64 - 1)
                    .ok_or(StoragePoolError::CapacityOverflow)?
            }
            PoolLayout::Mirror => current
                .members()
                .filter(|member| *member != device_id)
                .map(|member| self.device(member).map(|device| device.capacity_blocks))
                .try_fold(u64::MAX, |capacity, member| {
                    member.map(|device_capacity| capacity.min(device_capacity))
                })?,
        };
        if current.allocated_blocks > new_capacity {
            return Err(StoragePoolError::Busy)
        }

        let pool = self.pool_mut(id)?;
        let last = pool.member_count as usize - 1;
        pool.members[index] = pool.members[last];
        pool.members[last] = None;
        pool.member_count -= 1;
        pool.capacity_blocks = new_capacity;
        Ok(*pool)
    }

    /// Resolve a logical pool block to one physical provider block.
    pub fn resolve_block(
        &self,
        id: StoragePoolId,
        logical_block: u64,
        replica: u8,
    ) -> Result<BlockPlacement, StoragePoolError> {
        let pool = self.pool(id)?;
        if logical_block >= pool.capacity_blocks {
            return Err(StoragePoolError::NoSpace)
        }
        let (member_index, device_block) = match pool.layout {
            PoolLayout::Stripe => {
                if replica != 0 {
                    return Err(StoragePoolError::InvalidLayout)
                }
                (
                    (logical_block % pool.member_count as u64) as usize,
                    logical_block / pool.member_count as u64,
                )
            }
            PoolLayout::Mirror => {
                if replica >= pool.member_count {
                    return Err(StoragePoolError::InvalidLayout)
                }
                (replica as usize, logical_block)
            }
        };
        let device = pool.members[member_index].ok_or(StoragePoolError::InvalidLayout)?;
        if self.device(device)?.health == DeviceHealth::Failed {
            return Err(StoragePoolError::DeviceFailed)
        }
        Ok(BlockPlacement {
            device,
            device_block,
            replica,
        })
    }

    /// Reserve a contiguous logical tail range and return its first block.
    pub fn allocate(
        &mut self,
        id: StoragePoolId,
        blocks: u64,
    ) -> Result<u64, StoragePoolError> {
        if blocks == 0 {
            return Err(StoragePoolError::NoSpace)
        }
        if self.pool_health(id)? == PoolHealth::Offline {
            return Err(StoragePoolError::DeviceFailed)
        }
        let current = self.pool(id)?;
        if current.members().any(|member| {
            self.device(member)
                .is_ok_and(|device| device.health == DeviceHealth::Draining)
        }) {
            return Err(StoragePoolError::Busy)
        }
        let pool = self.pool_mut(id)?;
        let start = pool.allocated_blocks;
        pool.allocated_blocks = start
            .checked_add(blocks)
            .filter(|end| *end <= pool.capacity_blocks)
            .ok_or(StoragePoolError::NoSpace)?;
        Ok(start)
    }

    /// Release the most recently reserved tail range.
    pub fn release(
        &mut self,
        id: StoragePoolId,
        start_block: u64,
        blocks: u64,
    ) -> Result<(), StoragePoolError> {
        let pool = self.pool_mut(id)?;
        let end = start_block
            .checked_add(blocks)
            .ok_or(StoragePoolError::InvalidDevice)?;
        if end != pool.allocated_blocks {
            return Err(StoragePoolError::Busy)
        }
        pool.allocated_blocks = pool
            .allocated_blocks
            .checked_sub(blocks)
            .ok_or(StoragePoolError::InvalidDevice)?;
        Ok(())
    }

    pub fn device(&self, id: StorageDeviceId) -> Result<StorageDevice, StoragePoolError> {
        self.devices
            .iter()
            .flatten()
            .find(|device| device.id == id)
            .copied()
            .ok_or(StoragePoolError::DeviceNotFound)
    }

    pub fn pool(&self, id: StoragePoolId) -> Result<StoragePool, StoragePoolError> {
        self.pools
            .iter()
            .flatten()
            .find(|pool| pool.id == id)
            .copied()
            .ok_or(StoragePoolError::PoolNotFound)
    }

    pub fn devices(&self) -> impl Iterator<Item = StorageDevice> + '_ {
        self.devices.iter().flatten().copied()
    }

    pub fn pools(&self) -> impl Iterator<Item = StoragePool> + '_ {
        self.pools.iter().flatten().copied()
    }

    pub fn pool_health(
        &self,
        id: StoragePoolId,
    ) -> Result<PoolHealth, StoragePoolError> {
        let pool = self.pool(id)?;
        let online = pool
            .members()
            .filter(|member| {
                self.device(*member)
                    .is_ok_and(|device| device.health == DeviceHealth::Online)
            })
            .count();
        let available = pool
            .members()
            .filter(|member| {
                self.device(*member)
                    .is_ok_and(|device| device.health != DeviceHealth::Failed)
            })
            .count();
        Ok(match pool.layout {
            PoolLayout::Stripe if online == pool.member_count as usize => PoolHealth::Online,
            PoolLayout::Stripe if available == pool.member_count as usize => PoolHealth::Degraded,
            PoolLayout::Stripe => PoolHealth::Offline,
            PoolLayout::Mirror if online == pool.member_count as usize => PoolHealth::Online,
            PoolLayout::Mirror if available != 0 => PoolHealth::Degraded,
            PoolLayout::Mirror => PoolHealth::Offline,
        })
    }

    fn device_mut(
        &mut self,
        id: StorageDeviceId,
    ) -> Result<&mut StorageDevice, StoragePoolError> {
        self.devices
            .iter_mut()
            .flatten()
            .find(|device| device.id == id)
            .ok_or(StoragePoolError::DeviceNotFound)
    }

    fn pool_mut(
        &mut self,
        id: StoragePoolId,
    ) -> Result<&mut StoragePool, StoragePoolError> {
        self.pools
            .iter_mut()
            .flatten()
            .find(|pool| pool.id == id)
            .ok_or(StoragePoolError::PoolNotFound)
    }
}

impl<const DEVICES: usize, const POOLS: usize> Default for StoragePoolAdmin<DEVICES, POOLS> {
    fn default() -> Self {
        Self::new()
    }
}
