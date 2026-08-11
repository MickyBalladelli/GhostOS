//! Block-device integration for SynFS.
//!
//! The device trait is deliberately small. AHCI, NVMe, CXL persistent memory,
//! and network-block drivers can each implement it without making SynFS know
//! about controller registers or transport details. The queue above that
//! trait is fixed-capacity and completion based, so a filesystem daemon never
//! needs to wait inside the storage layer.

use crate::{
    BLOCK_SIZE, DeviceHealth, PoolLayout, StorageClass, StorageDevice, StorageDeviceId,
    StoragePoolAdmin, StoragePoolError, StoragePoolId,
};
use synos_status::{IntoStatus, Severity, Status, facility};

pub const MAX_BLOCK_IO_BYTES: usize = BLOCK_SIZE;
pub const DEFAULT_BLOCK_IO_QUEUE: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockOperation {
    Read,
    Write,
    Flush,
    Discard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct BlockRequestToken(u64);

impl BlockRequestToken {
    fn new(slot: usize, generation: u32) -> Self {
        Self((u64::from(generation) << 32) | slot as u64)
    }

    fn slot(self) -> usize {
        self.0 as u32 as usize
    }

    fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockRequest {
    pub operation: BlockOperation,
    pub pool: StoragePoolId,
    pub block: u64,
    bytes: u16,
    pub data: [u8; MAX_BLOCK_IO_BYTES],
}

impl BlockRequest {
    pub fn read(pool: StoragePoolId, block: u64) -> Self {
        Self {
            operation: BlockOperation::Read,
            pool,
            block,
            bytes: 0,
            data: [0; MAX_BLOCK_IO_BYTES],
        }
    }

    pub fn write(pool: StoragePoolId, block: u64, data: &[u8]) -> Result<Self, BlockIoError> {
        if data.is_empty() || data.len() > MAX_BLOCK_IO_BYTES {
            return Err(BlockIoError::InvalidRequest);
        }
        let mut request = Self {
            operation: BlockOperation::Write,
            pool,
            block,
            bytes: data.len() as u16,
            data: [0; MAX_BLOCK_IO_BYTES],
        };
        request.data[..data.len()].copy_from_slice(data);
        Ok(request)
    }

    pub fn flush(pool: StoragePoolId) -> Self {
        Self {
            operation: BlockOperation::Flush,
            pool,
            block: 0,
            bytes: 0,
            data: [0; MAX_BLOCK_IO_BYTES],
        }
    }

    pub fn discard(pool: StoragePoolId, block: u64) -> Self {
        Self {
            operation: BlockOperation::Discard,
            pool,
            block,
            bytes: 0,
            data: [0; MAX_BLOCK_IO_BYTES],
        }
    }

    pub const fn byte_len(&self) -> usize {
        self.bytes as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockIoError {
    QueueFull,
    InvalidToken,
    InvalidRequest,
    InvalidBlockSize,
    DeviceUnavailable,
    DeviceIo { device: StorageDeviceId },
    Pool(StoragePoolError),
}

impl IntoStatus for BlockIoError {
    fn status(self) -> Status {
        match self {
            Self::QueueFull => Status::BUSY,
            Self::DeviceUnavailable | Self::Pool(StoragePoolError::DeviceNotFound) => {
                Status::NOT_FOUND
            }
            Self::DeviceIo { .. } | Self::Pool(StoragePoolError::DeviceFailed) => {
                Status::new(Severity::Error, facility::DRIVER, 20, 0)
                    .unwrap_or(Status::INVALID_ARGUMENT)
            }
            Self::InvalidToken | Self::InvalidRequest | Self::InvalidBlockSize | Self::Pool(_) => {
                Status::INVALID_ARGUMENT
            }
        }
    }
}

/// A controller-specific block backend.
///
/// Implement this for an AHCI port, NVMe namespace, CXL persistent-memory
/// range, or network-block session. Calls may only enqueue hardware work;
/// [`BlockIoQueue`] delivers the completion later.
pub trait BlockDevice {
    fn read_block(&mut self, block: u64, output: &mut [u8]) -> Result<(), ()>;
    fn write_block(&mut self, block: u64, input: &[u8]) -> Result<(), ()>;
    /// Complete only after all earlier writes survive a power loss.
    fn flush(&mut self) -> Result<(), ()>;
    fn discard_block(&mut self, block: u64) -> Result<(), ()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BlockIoResult {
    Complete { bytes: u16 },
    Degraded { bytes: u16, failed_devices: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockCompletion {
    pub token: BlockRequestToken,
    pub result: Result<BlockIoResult, BlockIoError>,
    pub data: [u8; MAX_BLOCK_IO_BYTES],
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SlotState {
    Vacant,
    Queued,
    Completed,
}

#[derive(Clone, Copy)]
struct Slot {
    generation: u32,
    state: SlotState,
    request: Option<BlockRequest>,
    completion: Option<BlockCompletion>,
}

impl Slot {
    const EMPTY: Self = Self {
        generation: 0,
        state: SlotState::Vacant,
        request: None,
        completion: None,
    };
}

/// Fixed-capacity asynchronous request queue.
pub struct BlockIoQueue<const CAPACITY: usize = DEFAULT_BLOCK_IO_QUEUE> {
    slots: [Slot; CAPACITY],
    submit_cursor: usize,
    completion_cursor: usize,
}

impl<const CAPACITY: usize> BlockIoQueue<CAPACITY> {
    pub fn new() -> Self {
        Self {
            slots: [Slot::EMPTY; CAPACITY],
            submit_cursor: 0,
            completion_cursor: 0,
        }
    }

    pub fn submit(&mut self, request: BlockRequest) -> Result<BlockRequestToken, BlockIoError> {
        let slot_index = self
            .find_from(self.submit_cursor, SlotState::Vacant)
            .ok_or(BlockIoError::QueueFull)?;
        let slot = &mut self.slots[slot_index];
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.state = SlotState::Queued;
        slot.request = Some(request);
        slot.completion = None;
        self.submit_cursor = Self::next(slot_index);
        Ok(BlockRequestToken::new(slot_index, slot.generation))
    }

    /// Dispatch every queued request. The backend owns the device-specific
    /// asynchronous work; this method only records its eventual result.
    pub fn dispatch<D, const DEVICES: usize, const POOLS: usize>(
        &mut self,
        storage: &mut StoragePoolIo<D, DEVICES, POOLS>,
    ) -> usize
    where
        D: BlockDevice,
    {
        let mut dispatched = 0;
        while let Some(index) = self.find_from(self.submit_cursor, SlotState::Queued) {
            let Some(request) = self.slots[index].request else {
                self.slots[index].state = SlotState::Vacant;
                continue
            };
            let token = BlockRequestToken::new(index, self.slots[index].generation);
            let completion = storage.execute(token, request);
            self.slots[index].completion = Some(completion);
            self.slots[index].state = SlotState::Completed;
            self.submit_cursor = Self::next(index);
            dispatched += 1;
        }
        dispatched
    }

    pub fn poll(&mut self) -> Option<BlockCompletion> {
        let index = self.find_from(self.completion_cursor, SlotState::Completed)?;
        let slot = &mut self.slots[index];
        let Some(completion) = slot.completion.take() else {
            slot.state = SlotState::Vacant;
            slot.request = None;
            self.completion_cursor = Self::next(index);
            return None
        };
        slot.state = SlotState::Vacant;
        slot.request = None;
        self.completion_cursor = Self::next(index);
        Some(completion)
    }

    pub fn cancel(&mut self, token: BlockRequestToken) -> Result<(), BlockIoError> {
        let slot = self.slot_mut(token)?;
        if slot.state != SlotState::Queued {
            return Err(BlockIoError::InvalidToken);
        }
        slot.state = SlotState::Vacant;
        slot.request = None;
        Ok(())
    }

    pub fn pending(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.state == SlotState::Queued)
            .count()
    }

    fn slot_mut(&mut self, token: BlockRequestToken) -> Result<&mut Slot, BlockIoError> {
        let slot = self
            .slots
            .get_mut(token.slot())
            .filter(|slot| slot.generation == token.generation())
            .ok_or(BlockIoError::InvalidToken)?;
        Ok(slot)
    }

    fn find_from(&self, start: usize, state: SlotState) -> Option<usize> {
        (0..CAPACITY)
            .map(|offset| (start + offset) % CAPACITY)
            .find(|index| self.slots[*index].state == state)
    }

    fn next(index: usize) -> usize {
        (index + 1) % CAPACITY
    }
}

impl<const CAPACITY: usize> Default for BlockIoQueue<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

struct RegisteredDevice<D> {
    id: StorageDeviceId,
    backend: D,
}

/// Storage-pool control and I/O plane.
pub struct StoragePoolIo<D, const DEVICES: usize = 32, const POOLS: usize = 8> {
    admin: StoragePoolAdmin<DEVICES, POOLS>,
    devices: [Option<RegisteredDevice<D>>; DEVICES],
    default_pool: Option<StoragePoolId>,
    volume_start: Option<u64>,
}

impl<D, const DEVICES: usize, const POOLS: usize> StoragePoolIo<D, DEVICES, POOLS>
where
    D: BlockDevice,
{
    pub fn new() -> Self {
        Self {
            admin: StoragePoolAdmin::new(),
            devices: core::array::from_fn(|_| None),
            default_pool: None,
            volume_start: None,
        }
    }

    pub fn admin(&self) -> &StoragePoolAdmin<DEVICES, POOLS> {
        &self.admin
    }

    pub fn admin_mut(&mut self) -> &mut StoragePoolAdmin<DEVICES, POOLS> {
        &mut self.admin
    }

    /// Select the pool used by the SynFS [`BlockStore`] view.
    pub fn select_pool(&mut self, id: StoragePoolId) -> Result<(), BlockIoError> {
        if self.volume_start.is_some() {
            return Err(BlockIoError::Pool(StoragePoolError::Busy));
        }
        self.admin.pool(id).map_err(BlockIoError::Pool)?;
        self.default_pool = Some(id);
        Ok(())
    }

    pub const fn selected_pool(&self) -> Option<StoragePoolId> {
        self.default_pool
    }

    pub fn register_device(
        &mut self,
        id: StorageDeviceId,
        class: StorageClass,
        capacity_blocks: u64,
        block_size: u32,
        fault_domain: u32,
        backend: D,
    ) -> Result<(), BlockIoError> {
        if self.devices.iter().all(Option::is_some) {
            return Err(BlockIoError::Pool(StoragePoolError::RegistryFull));
        }
        self.admin
            .register_device(StorageDevice {
                id,
                class,
                capacity_blocks,
                block_size,
                fault_domain,
                health: DeviceHealth::Online,
            })
            .map_err(BlockIoError::Pool)?;
        let slot = self
            .devices
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(BlockIoError::Pool(StoragePoolError::RegistryFull))?;
        *slot = Some(RegisteredDevice { id, backend });
        Ok(())
    }

    pub fn unregister_device(&mut self, id: StorageDeviceId) -> Result<D, BlockIoError> {
        let device = self
            .admin
            .unregister_device(id)
            .map_err(BlockIoError::Pool)?;
        let _ = device;
        let slot = self
            .devices
            .iter_mut()
            .find(|entry| entry.as_ref().is_some_and(|entry| entry.id == id))
            .ok_or(BlockIoError::DeviceUnavailable)?;
        let registered = slot.take().ok_or(BlockIoError::DeviceUnavailable)?;
        Ok(registered.backend)
    }

    pub fn complete_remove(&mut self, id: StorageDeviceId) -> Result<D, BlockIoError> {
        if self.admin.device(id).map_err(BlockIoError::Pool)?.health != DeviceHealth::Draining {
            return Err(BlockIoError::Pool(StoragePoolError::Busy));
        }
        self.unregister_device(id)
    }

    pub fn begin_remove(&mut self, id: StorageDeviceId) -> Result<(), BlockIoError> {
        self.admin
            .set_device_health(id, DeviceHealth::Draining)
            .map_err(BlockIoError::Pool)
    }

    pub fn create_pool(
        &mut self,
        id: StoragePoolId,
        name: &str,
        layout: PoolLayout,
        members: &[StorageDeviceId],
    ) -> Result<(), BlockIoError> {
        self.admin
            .create_pool(id, name, layout, members)
            .map(|_| ())
            .map_err(BlockIoError::Pool)?;
        if self.default_pool.is_none() {
            self.default_pool = Some(id)
        }
        Ok(())
    }

    /// Register a replacement member. Rebuild it with [`Self::rebuild_mirror`]
    /// before considering the mirror healthy again.
    pub fn replace_device(
        &mut self,
        pool_id: StoragePoolId,
        failed_id: StorageDeviceId,
        replacement_id: StorageDeviceId,
        class: StorageClass,
        capacity_blocks: u64,
        block_size: u32,
        fault_domain: u32,
        backend: D,
    ) -> Result<(), BlockIoError> {
        self.register_device(
            replacement_id,
            class,
            capacity_blocks,
            block_size,
            fault_domain,
            backend,
        )?;
        if let Err(error) = self
            .admin
            .replace_device(pool_id, failed_id, replacement_id)
            .map(|_| ())
            .map_err(BlockIoError::Pool)
        {
            let _ = self.unregister_device(replacement_id);
            return Err(error);
        }
        Ok(())
    }

    /// Copy allocated mirror blocks onto a replacement member in bounded
    /// chunks. Call again with the returned start block until it reaches the
    /// pool's allocated block count.
    pub fn rebuild_mirror(
        &mut self,
        pool_id: StoragePoolId,
        target: StorageDeviceId,
        start_block: u64,
        max_blocks: u64,
        scratch: &mut [u8],
    ) -> Result<u64, BlockIoError> {
        let pool = self.admin.pool(pool_id).map_err(BlockIoError::Pool)?;
        if pool.layout != PoolLayout::Mirror || !pool.members().any(|id| id == target) {
            return Err(BlockIoError::InvalidRequest);
        }
        let device_size = pool.block_size as usize;
        if scratch.len() < device_size || start_block > pool.allocated_blocks {
            return Err(BlockIoError::InvalidRequest);
        }
        self.admin
            .set_device_health(target, DeviceHealth::Online)
            .map_err(BlockIoError::Pool)?;

        let target_replica = pool
            .members()
            .position(|id| id == target)
            .ok_or(BlockIoError::DeviceUnavailable)? as u8;
        let end = start_block
            .saturating_add(max_blocks)
            .min(pool.allocated_blocks);
        let mut block = start_block;
        while block < end {
            let mut source = None;
            for replica in 0..pool.member_count() {
                if replica == target_replica {
                    continue;
                }
                let placement = self
                    .admin
                    .resolve_block(pool_id, block, replica)
                    .map_err(BlockIoError::Pool)?;
                if self
                    .admin
                    .device(placement.device)
                    .is_ok_and(|device| device.health == DeviceHealth::Online)
                {
                    source = Some(placement.device);
                    break;
                }
            }
            let source = source.ok_or(BlockIoError::DeviceUnavailable)?;
            let target_placement = self
                .admin
                .resolve_block(pool_id, block, target_replica)
                .map_err(BlockIoError::Pool)?;
            self.with_device(source, |backend| {
                backend.read_block(block, &mut scratch[..device_size])
            })
            .map_err(|_| BlockIoError::DeviceIo { device: source })?;
            self.with_device(target_placement.device, |backend| {
                backend.write_block(block, &scratch[..device_size])
            })
            .map_err(|_| BlockIoError::DeviceIo { device: target })?;
            block += 1;
        }
        Ok(end - start_block)
    }

    fn execute(&mut self, token: BlockRequestToken, request: BlockRequest) -> BlockCompletion {
        let mut completion = BlockCompletion {
            token,
            result: Err(BlockIoError::InvalidRequest),
            data: [0; MAX_BLOCK_IO_BYTES],
        };
        let result = self.execute_request(&request, &mut completion.data);
        completion.result = result;
        completion
    }

    fn execute_request(
        &mut self,
        request: &BlockRequest,
        output: &mut [u8; MAX_BLOCK_IO_BYTES],
    ) -> Result<BlockIoResult, BlockIoError> {
        let pool = self.admin.pool(request.pool).map_err(BlockIoError::Pool)?;
        let device_size = pool.block_size as usize;
        if device_size == 0 || device_size > MAX_BLOCK_IO_BYTES {
            return Err(BlockIoError::InvalidBlockSize);
        }
        if request.operation != BlockOperation::Flush && request.block >= pool.allocated_blocks {
            return Err(BlockIoError::Pool(StoragePoolError::NoSpace));
        }
        if matches!(request.operation, BlockOperation::Read) && request.byte_len() != 0
            || matches!(request.operation, BlockOperation::Write)
                && request.byte_len() != device_size
        {
            return Err(BlockIoError::InvalidRequest);
        }
        if request.operation == BlockOperation::Flush {
            return self.flush_pool(request.pool);
        }

        let replicas = match pool.layout {
            PoolLayout::Stripe => 1,
            PoolLayout::Mirror => pool.member_count(),
        };
        let mut successful = 0u8;
        let mut failed = 0u8;
        let mut first_error = None;
        let mut pool_error = None;
        for replica in 0..replicas {
            let placement = match self
                .admin
                .resolve_block(request.pool, request.block, replica)
            {
                Ok(placement) => placement,
                Err(StoragePoolError::DeviceFailed) => {
                    failed += 1;
                    pool_error = Some(StoragePoolError::DeviceFailed);
                    continue;
                }
                Err(error) => return Err(BlockIoError::Pool(error)),
            };
            if request.operation != BlockOperation::Read
                && self
                    .admin
                    .device(placement.device)
                    .is_ok_and(|device| device.health == DeviceHealth::Draining)
            {
                failed += 1;
                continue;
            }
            let result = match request.operation {
                BlockOperation::Read => self.with_device(placement.device, |backend| {
                    backend.read_block(placement.device_block, &mut output[..device_size])
                }),
                BlockOperation::Write => self.with_device(placement.device, |backend| {
                    backend.write_block(placement.device_block, &request.data[..device_size])
                }),
                BlockOperation::Discard => self.with_device(placement.device, |backend| {
                    backend.discard_block(placement.device_block)
                }),
                BlockOperation::Flush => unreachable!(),
            };
            match result {
                Ok(()) => successful += 1,
                Err(()) => {
                    failed += 1;
                    first_error.get_or_insert(placement.device);
                    let _ = self
                        .admin
                        .set_device_health(placement.device, DeviceHealth::Failed);
                }
            }
            if request.operation == BlockOperation::Read && successful != 0 {
                break;
            }
        }
        if successful == 0 {
            return Err(pool_error.map_or_else(
                || {
                    first_error.map_or(BlockIoError::DeviceUnavailable, |device| {
                        BlockIoError::DeviceIo { device }
                    })
                },
                BlockIoError::Pool,
            ));
        }
        let bytes = if request.operation == BlockOperation::Discard {
            0
        } else {
            device_size as u16
        };
        Ok(if failed == 0 {
            BlockIoResult::Complete { bytes }
        } else {
            BlockIoResult::Degraded {
                bytes,
                failed_devices: failed,
            }
        })
    }

    fn flush_pool(&mut self, pool_id: StoragePoolId) -> Result<BlockIoResult, BlockIoError> {
        let pool = self.admin.pool(pool_id).map_err(BlockIoError::Pool)?;
        let mut failed = 0;
        for device in pool.members() {
            if self.admin.device(device).is_ok_and(|device| {
                matches!(device.health, DeviceHealth::Failed | DeviceHealth::Draining)
            }) {
                failed += 1;
                continue;
            }
            if self.with_device(device, BlockDevice::flush).is_err() {
                failed += 1;
                let _ = self.admin.set_device_health(device, DeviceHealth::Failed);
            }
        }
        if failed == pool.member_count() {
            return Err(BlockIoError::DeviceUnavailable);
        }
        Ok(if failed == 0 {
            BlockIoResult::Complete { bytes: 0 }
        } else {
            BlockIoResult::Degraded {
                bytes: 0,
                failed_devices: failed,
            }
        })
    }

    fn with_device(
        &mut self,
        id: StorageDeviceId,
        operation: impl FnOnce(&mut D) -> Result<(), ()>,
    ) -> Result<(), ()> {
        let device = self
            .devices
            .iter_mut()
            .flatten()
            .find(|device| device.id == id)
            .ok_or(())?;
        operation(&mut device.backend)
    }

    fn factor(&self, pool: StoragePoolId) -> Result<usize, BlockIoError> {
        let size = self
            .admin
            .pool(pool)
            .map_err(BlockIoError::Pool)?
            .block_size as usize;
        if size == 0 || BLOCK_SIZE % size != 0 {
            return Err(BlockIoError::InvalidBlockSize);
        }
        Ok(BLOCK_SIZE / size)
    }

    fn volume_physical_block(
        &self,
        pool: StoragePoolId,
        block: u64,
        index: usize,
    ) -> Result<u64, BlockIoError> {
        let factor = self.factor(pool)?;
        let start = self.volume_start.ok_or(BlockIoError::DeviceUnavailable)?;
        start
            .checked_add(
                block
                    .checked_mul(factor as u64)
                    .and_then(|block| block.checked_add(index as u64))
                    .ok_or(BlockIoError::InvalidRequest)?,
            )
            .ok_or(BlockIoError::InvalidRequest)
    }

    fn selected_pool_id(&self) -> Result<StoragePoolId, BlockIoError> {
        self.default_pool.ok_or(BlockIoError::DeviceUnavailable)
    }
}

impl<D, const DEVICES: usize, const POOLS: usize> Default for StoragePoolIo<D, DEVICES, POOLS>
where
    D: BlockDevice,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Block-sized interface used by the persistent SynFS volume format.
pub trait BlockStore {
    fn read_block(&mut self, block: u64, output: &mut [u8]) -> Result<(), BlockIoError>;
    fn write_block(&mut self, block: u64, input: &[u8]) -> Result<(), BlockIoError>;
    /// Complete only after earlier writes are durable across power loss.
    fn flush(&mut self) -> Result<(), BlockIoError>;
    fn discard_block(&mut self, block: u64) -> Result<(), BlockIoError>;

    fn reserve(&mut self, _blocks: u64) -> Result<(), BlockIoError> {
        Ok(())
    }

    fn release(&mut self, _blocks: u64) -> Result<(), BlockIoError> {
        Ok(())
    }
}

impl<D, const DEVICES: usize, const POOLS: usize> StoragePoolIo<D, DEVICES, POOLS>
where
    D: BlockDevice,
{
    fn execute_store_request(
        &mut self,
        request: BlockRequest,
    ) -> Result<BlockIoResult, BlockIoError> {
        let token = BlockRequestToken::new(0, 1);
        let mut completion = self.execute(token, request);
        if completion.result.is_ok() {
            completion.data.fill(0);
        }
        completion.result
    }
}

impl<D, const DEVICES: usize, const POOLS: usize> BlockStore for StoragePoolIo<D, DEVICES, POOLS>
where
    D: BlockDevice,
{
    fn read_block(&mut self, block: u64, output: &mut [u8]) -> Result<(), BlockIoError> {
        if output.len() != BLOCK_SIZE {
            return Err(BlockIoError::InvalidBlockSize);
        }
        let pool_id = self.selected_pool_id()?;
        let pool = self.admin.pool(pool_id).map_err(BlockIoError::Pool)?;
        let factor = self.factor(pool.id)?;
        for index in 0..factor {
            let physical_block = self.volume_physical_block(pool.id, block, index)?;
            let request = BlockRequest::read(pool.id, physical_block);
            let token = BlockRequestToken::new(0, 1);
            let completion = self.execute(token, request);
            completion.result?;
            output[index * pool.block_size as usize..(index + 1) * pool.block_size as usize]
                .copy_from_slice(&completion.data[..pool.block_size as usize]);
        }
        Ok(())
    }

    fn write_block(&mut self, block: u64, input: &[u8]) -> Result<(), BlockIoError> {
        if input.len() != BLOCK_SIZE {
            return Err(BlockIoError::InvalidBlockSize);
        }
        let pool_id = self.selected_pool_id()?;
        let pool = self.admin.pool(pool_id).map_err(BlockIoError::Pool)?;
        let factor = self.factor(pool.id)?;
        for index in 0..factor {
            let start = index * pool.block_size as usize;
            let request = BlockRequest::write(
                pool.id,
                self.volume_physical_block(pool.id, block, index)?,
                &input[start..start + pool.block_size as usize],
            )?;
            self.execute_store_request(request)?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockIoError> {
        let pool_id = self.selected_pool_id()?;
        self.execute_store_request(BlockRequest::flush(pool_id))
            .map(|_| ())
    }

    fn discard_block(&mut self, block: u64) -> Result<(), BlockIoError> {
        let pool_id = self.selected_pool_id()?;
        let pool = self.admin.pool(pool_id).map_err(BlockIoError::Pool)?;
        let factor = self.factor(pool.id)?;
        for index in 0..factor {
            self.execute_store_request(BlockRequest::discard(
                pool.id,
                self.volume_physical_block(pool.id, block, index)?,
            ))?;
        }
        Ok(())
    }

    fn reserve(&mut self, blocks: u64) -> Result<(), BlockIoError> {
        if self.volume_start.is_some() || blocks == 0 {
            return Err(BlockIoError::InvalidRequest);
        }
        let pool = self.selected_pool_id()?;
        let factor = self.factor(pool)? as u64;
        let physical_blocks = blocks
            .checked_mul(factor)
            .ok_or(BlockIoError::InvalidRequest)?;
        self.volume_start = Some(
            self.admin
                .allocate(pool, physical_blocks)
                .map_err(BlockIoError::Pool)?,
        );
        Ok(())
    }

    fn release(&mut self, blocks: u64) -> Result<(), BlockIoError> {
        let pool = self.selected_pool_id()?;
        let factor = self.factor(pool)? as u64;
        let start = self
            .volume_start
            .take()
            .ok_or(BlockIoError::InvalidRequest)?;
        self.admin
            .release(
                pool,
                start,
                blocks
                    .checked_mul(factor)
                    .ok_or(BlockIoError::InvalidRequest)?,
            )
            .map_err(BlockIoError::Pool)
    }
}
