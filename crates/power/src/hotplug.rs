use synos_fabric::{
    Error as FabricError,
    cxl::{
        DeviceState as CxlDeviceState, Endpoint as CxlEndpoint,
        Registry as CxlRegistry,
    },
    memory::{GlobalAddressSpace, LeaseTable, PoolId},
};
use synos_legacy_pc_drivers::{
    NvmeNamespaceState, NvmeRegistry, PciAddress,
    storage::DriverError,
};
use synos_status::{IntoStatus, Status};
use synos_synfs::{
    DeviceHealth, StorageDeviceId, StoragePoolAdmin, StoragePoolError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotPlugDevice {
    Cxl { serial: u64 },
    Nvme {
        controller: PciAddress,
        namespace_id: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotPlugState {
    Online,
    Draining,
    Removed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotPlugTransition {
    pub device: HotPlugDevice,
    pub state: HotPlugState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotPlugError {
    ActiveUsers { count: usize },
    Driver(DriverError),
    Fabric(FabricError),
    Storage(StoragePoolError),
}

impl From<DriverError> for HotPlugError {
    fn from(error: DriverError) -> Self {
        Self::Driver(error)
    }
}

impl From<FabricError> for HotPlugError {
    fn from(error: FabricError) -> Self {
        Self::Fabric(error)
    }
}

impl From<StoragePoolError> for HotPlugError {
    fn from(error: StoragePoolError) -> Self {
        Self::Storage(error)
    }
}

impl IntoStatus for HotPlugError {
    fn status(self) -> Status {
        match self {
            Self::ActiveUsers { .. } => Status::BUSY,
            Self::Driver(error) => error.status(),
            Self::Fabric(error) => error.status(),
            Self::Storage(error) => error.status(),
        }
    }
}

pub fn handle_cxl_insertion<const CXL: usize>(
    registry: &mut CxlRegistry<CXL>,
    endpoint: CxlEndpoint,
    decoder_count: u8,
) -> Result<HotPlugTransition, HotPlugError> {
    let serial = endpoint.serial;
    registry.discover(endpoint, decoder_count)?;
    Ok(HotPlugTransition {
        device: HotPlugDevice::Cxl { serial },
        state: HotPlugState::Online,
    })
}

pub fn handle_nvme_insertion<const NAMESPACES: usize>(
    registry: &mut NvmeRegistry<NAMESPACES>,
    controller: PciAddress,
    namespace_id: u32,
    capacity_blocks: u64,
    block_size: u32,
) -> Result<HotPlugTransition, HotPlugError> {
    registry.discover(
        controller,
        namespace_id,
        capacity_blocks,
        block_size,
    )?;
    Ok(HotPlugTransition {
        device: HotPlugDevice::Nvme {
            controller,
            namespace_id,
        },
        state: HotPlugState::Online,
    })
}

/// Stop new CXL leases while existing users migrate or release their ranges.
pub fn begin_cxl_memory_removal<
    const CXL: usize,
    const POOLS: usize,
    const OVERRIDES: usize,
>(
    registry: &mut CxlRegistry<CXL>,
    serial: u64,
    address_space: &mut GlobalAddressSpace<POOLS, OVERRIDES>,
    pool: PoolId,
) -> Result<HotPlugTransition, HotPlugError> {
    registry.begin_remove(serial)?;
    if let Err(error) = address_space.begin_pool_drain(pool) {
        registry.cancel_remove(serial)?;
        return Err(error.into())
    }
    Ok(HotPlugTransition {
        device: HotPlugDevice::Cxl { serial },
        state: HotPlugState::Draining,
    })
}

/// Remove a drained CXL window only after every generation-checked lease ends.
pub fn complete_cxl_memory_removal<
    const CXL: usize,
    const POOLS: usize,
    const OVERRIDES: usize,
    const LEASES: usize,
>(
    registry: &mut CxlRegistry<CXL>,
    serial: u64,
    address_space: &mut GlobalAddressSpace<POOLS, OVERRIDES>,
    leases: &LeaseTable<LEASES>,
    pool: PoolId,
) -> Result<HotPlugTransition, HotPlugError> {
    if registry
        .find_serial(serial)
        .is_none_or(|device| device.state != CxlDeviceState::Draining)
        || !address_space.is_pool_draining(pool)
    {
        return Err(FabricError::Busy.into())
    }
    let active = leases.active_for_pool(pool);
    if active != 0 {
        return Err(HotPlugError::ActiveUsers { count: active })
    }
    registry.complete_remove(serial)?;
    address_space.remove_pool(pool)?;
    Ok(HotPlugTransition {
        device: HotPlugDevice::Cxl { serial },
        state: HotPlugState::Removed,
    })
}

/// Reject new SynFS allocations while an NVMe namespace drains.
pub fn begin_nvme_storage_removal<
    const NAMESPACES: usize,
    const DEVICES: usize,
    const POOLS: usize,
>(
    registry: &mut NvmeRegistry<NAMESPACES>,
    controller: PciAddress,
    namespace_id: u32,
    storage: &mut StoragePoolAdmin<DEVICES, POOLS>,
    storage_device: StorageDeviceId,
) -> Result<HotPlugTransition, HotPlugError> {
    registry.begin_remove(controller, namespace_id)?;
    if let Err(error) = storage.set_device_health(storage_device, DeviceHealth::Draining) {
        registry.cancel_remove(controller, namespace_id)?;
        return Err(error.into())
    }
    Ok(HotPlugTransition {
        device: HotPlugDevice::Nvme {
            controller,
            namespace_id,
        },
        state: HotPlugState::Draining,
    })
}

/// Complete NVMe removal only when the namespace is no longer in a SynFS pool.
pub fn complete_nvme_storage_removal<
    const NAMESPACES: usize,
    const DEVICES: usize,
    const POOLS: usize,
>(
    registry: &mut NvmeRegistry<NAMESPACES>,
    controller: PciAddress,
    namespace_id: u32,
    storage: &mut StoragePoolAdmin<DEVICES, POOLS>,
    storage_device: StorageDeviceId,
) -> Result<HotPlugTransition, HotPlugError> {
    if registry
        .find(controller, namespace_id)
        .is_none_or(|namespace| namespace.state != NvmeNamespaceState::Draining)
    {
        return Err(DriverError::Busy.into())
    }
    storage.unregister_device(storage_device)?;
    registry.complete_remove(controller, namespace_id)?;
    Ok(HotPlugTransition {
        device: HotPlugDevice::Nvme {
            controller,
            namespace_id,
        },
        state: HotPlugState::Removed,
    })
}
