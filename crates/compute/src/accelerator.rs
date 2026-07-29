use synos_ipc::SharedBuffer;
use synos_legacy_pc_drivers::{Bar, PciAddress, PciDevice};
use synos_platform_io::{AsyncQueue, Completion, DEFAULT_QUEUE_CAPACITY, RequestToken, Submission};
use synos_status::{IntoStatus, Status};

use crate::{Error, framework::BindingAccess, tensor::SharedTensor};

pub const MAX_ACCELERATOR_BARS: usize = 6;
pub const MAX_DISPATCH_BINDINGS: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct AcceleratorId(u32);

impl AcceleratorId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct AcceleratorCapability(u64);

impl AcceleratorCapability {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcceleratorKind {
    Gpu,
    Npu,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComputeApi {
    Vulkan { major: u8, minor: u8 },
    Native,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryBar {
    pub index: u8,
    pub address: u64,
    pub prefetchable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceLimits {
    pub max_workgroups: [u32; 3],
    pub max_bindings: u8,
    pub shared_memory_bytes: u32,
    pub coherent_host_memory: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcceleratorDevice {
    pub id: AcceleratorId,
    pub kind: AcceleratorKind,
    pub pci_address: PciAddress,
    pub vendor_id: u16,
    pub device_id: u16,
    pub api: ComputeApi,
    pub bars: [Option<MemoryBar>; MAX_ACCELERATOR_BARS],
    pub limits: DeviceLimits,
}

impl AcceleratorDevice {
    pub fn from_pci(
        id: AcceleratorId,
        device: PciDevice,
        api: ComputeApi,
        limits: DeviceLimits,
    ) -> Result<Self, Error> {
        let kind = match device.class {
            0x03 => AcceleratorKind::Gpu,
            0x12 => AcceleratorKind::Npu,
            _ => return Err(Error::UnsupportedDevice),
        };
        if limits.max_bindings == 0
            || limits.max_bindings as usize > MAX_DISPATCH_BINDINGS
            || limits.max_workgroups.contains(&0)
        {
            return Err(Error::InvalidDevice);
        }
        let mut bars = [None; MAX_ACCELERATOR_BARS];
        for (index, bar) in device.bars.iter().copied().enumerate() {
            bars[index] = match bar {
                Bar::Memory32 {
                    address,
                    prefetchable,
                } => Some(MemoryBar {
                    index: index as u8,
                    address: address as u64,
                    prefetchable,
                }),
                Bar::Memory64 {
                    address,
                    prefetchable,
                } => Some(MemoryBar {
                    index: index as u8,
                    address,
                    prefetchable,
                }),
                Bar::Unused | Bar::Io { .. } => None,
            }
        }
        if bars.iter().all(Option::is_none) {
            return Err(Error::InvalidDevice);
        }
        Ok(Self {
            id,
            kind,
            pci_address: device.address,
            vendor_id: device.vendor_id,
            device_id: device.device_id,
            api,
            bars,
            limits,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KernelFormat {
    SpirV,
    Native,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct KernelHandle(u32);

impl KernelHandle {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelDescriptor {
    pub handle: KernelHandle,
    pub format: KernelFormat,
    pub code: SharedBuffer,
}

impl KernelDescriptor {
    pub fn validate(self) -> Result<Self, Error> {
        if self.code.length == 0
            || self.code.writable
            || matches!(self.format, KernelFormat::SpirV) && self.code.length % 4 != 0
        {
            return Err(Error::InvalidKernel);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchBinding {
    pub slot: u16,
    pub tensor: SharedTensor,
    pub access: BindingAccess,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ComputeDispatch {
    pub kernel: KernelHandle,
    pub workgroups: [u32; 3],
    bindings: [Option<DispatchBinding>; MAX_DISPATCH_BINDINGS],
    binding_count: u8,
}

impl ComputeDispatch {
    pub fn new(
        kernel: KernelHandle,
        workgroups: [u32; 3],
        bindings: &[DispatchBinding],
    ) -> Result<Self, Error> {
        if workgroups.contains(&0) || bindings.len() > MAX_DISPATCH_BINDINGS {
            return Err(Error::InvalidDispatch);
        }
        let mut stored = [None; MAX_DISPATCH_BINDINGS];
        for (index, binding) in bindings.iter().copied().enumerate() {
            binding.tensor.validate()?;
            if binding.access.writable() && !binding.tensor.buffer.writable {
                return Err(Error::ReadOnly);
            }
            if bindings[..index]
                .iter()
                .any(|existing| existing.slot == binding.slot)
            {
                return Err(Error::DuplicateBinding);
            }
            stored[index] = Some(binding)
        }
        Ok(Self {
            kernel,
            workgroups,
            bindings: stored,
            binding_count: bindings.len() as u8,
        })
    }

    pub fn bindings(&self) -> impl Iterator<Item = DispatchBinding> + '_ {
        self.bindings[..self.binding_count as usize]
            .iter()
            .flatten()
            .copied()
    }

    pub fn validate_for(self, device: AcceleratorDevice) -> Result<Self, Error> {
        if self.binding_count > device.limits.max_bindings
            || self
                .workgroups
                .iter()
                .zip(device.limits.max_workgroups)
                .any(|(requested, maximum)| *requested > maximum)
        {
            return Err(Error::InvalidDispatch);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AcceleratorOperation {
    LoadKernel(KernelDescriptor),
    Dispatch(ComputeDispatch),
    UnloadKernel(KernelHandle),
    Reset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcceleratorRequest {
    pub device: AcceleratorId,
    pub capability: AcceleratorCapability,
    pub operation: AcceleratorOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcceleratorResult {
    pub status: u32,
    pub device_timestamp_ns: u64,
}

/// Device-specific Vulkan or native NPU implementation living in Ring 3.
pub trait AcceleratorDriver {
    fn device(&self) -> AcceleratorDevice;

    fn load_kernel(&mut self, kernel: KernelDescriptor) -> Result<u64, Error>;

    fn dispatch_compute(&mut self, dispatch: ComputeDispatch) -> Result<u64, Error>;

    fn unload_kernel(&mut self, kernel: KernelHandle) -> Result<u64, Error>;

    fn reset(&mut self) -> Result<u64, Error>;
}

/// Bounded request queue shared by framework clients and one Ring 3 driver.
pub struct AcceleratorQueue<const CAPACITY: usize = DEFAULT_QUEUE_CAPACITY> {
    device: AcceleratorDevice,
    capability: AcceleratorCapability,
    queue: AsyncQueue<AcceleratorRequest, AcceleratorResult, CAPACITY>,
}

impl<const CAPACITY: usize> AcceleratorQueue<CAPACITY> {
    pub const fn new(device: AcceleratorDevice, capability: AcceleratorCapability) -> Self {
        Self {
            device,
            capability,
            queue: AsyncQueue::new(),
        }
    }

    pub fn submit(&mut self, request: AcceleratorRequest) -> Result<RequestToken, Error> {
        if request.device != self.device.id || request.capability != self.capability {
            return Err(Error::AccessDenied);
        }
        match request.operation {
            AcceleratorOperation::LoadKernel(kernel) => {
                kernel.validate()?;
            }
            AcceleratorOperation::Dispatch(dispatch) => {
                dispatch.validate_for(self.device)?;
            }
            AcceleratorOperation::UnloadKernel(_) | AcceleratorOperation::Reset => {}
        }
        self.queue.submit(request).map_err(Into::into)
    }

    pub fn dispatch(&mut self) -> Option<Submission<AcceleratorRequest>> {
        self.queue.dispatch()
    }

    pub fn complete(
        &mut self,
        token: RequestToken,
        result: AcceleratorResult,
    ) -> Result<(), Error> {
        self.queue.complete(token, result).map_err(Into::into)
    }

    pub fn poll(&mut self) -> Option<Completion<AcceleratorResult>> {
        self.queue.poll()
    }

    pub fn cancel(&mut self, token: RequestToken) -> Result<(), Error> {
        self.queue.cancel(token).map_err(Into::into)
    }

    pub fn pending(&self) -> usize {
        self.queue.pending()
    }

    pub fn service_one<D: AcceleratorDriver>(&mut self, driver: &mut D) -> Result<bool, Error> {
        if driver.device().id != self.device.id {
            return Err(Error::InvalidDevice);
        }
        let Some(submission) = self.dispatch() else {
            return Ok(false);
        };
        let executed = match submission.request.operation {
            AcceleratorOperation::LoadKernel(kernel) => driver.load_kernel(kernel),
            AcceleratorOperation::Dispatch(dispatch) => driver.dispatch_compute(dispatch),
            AcceleratorOperation::UnloadKernel(kernel) => driver.unload_kernel(kernel),
            AcceleratorOperation::Reset => driver.reset(),
        };
        let result = match executed {
            Ok(timestamp) => AcceleratorResult {
                status: Status::NORMAL.raw(),
                device_timestamp_ns: timestamp,
            },
            Err(error) => AcceleratorResult {
                status: error.status().raw(),
                device_timestamp_ns: 0,
            },
        };
        self.complete(submission.token, result)?;
        Ok(true)
    }

    pub const fn device(&self) -> AcceleratorDevice {
        self.device
    }
}

/// Safe system-call boundary used by a Ring 3 PCIe accelerator driver.
///
/// The platform service checks the capability before touching configuration
/// space, BAR mappings, interrupts, or bus-mastering state.
pub trait PciePlatform {
    fn enable_device(
        &mut self,
        capability: AcceleratorCapability,
        address: PciAddress,
    ) -> Result<(), Error>;

    fn map_bar(
        &mut self,
        capability: AcceleratorCapability,
        address: PciAddress,
        bar: MemoryBar,
    ) -> Result<u64, Error>;

    fn bind_interrupt(
        &mut self,
        capability: AcceleratorCapability,
        address: PciAddress,
        vector: u16,
    ) -> Result<(), Error>;

    fn ring_doorbell(
        &mut self,
        capability: AcceleratorCapability,
        mapped_bar: u64,
        offset: u32,
        value: u32,
    ) -> Result<(), Error>;
}
