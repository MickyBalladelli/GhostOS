use core::ptr::NonNull;

use crate::{
    AddressRange, Error, NodeId,
    memory::{GlobalAddressSpace, MemoryKind, MemoryPool, PoolId, Transport},
};

pub const CXL_VENDOR_ID: u16 = 0x1e98;
pub const HDM_DECODER_GRANULARITY: u64 = 256 * 1024 * 1024;
pub const MAX_CXL_DEVICES: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CxlVersion {
    V3_0,
    V3_1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceType {
    Type1,
    Type2,
    Type3,
}

/// Information obtained from PCIe CXL DVSECs and the Register Locator DVSEC.
///
/// The PCI service performs config-space enumeration. This structure is the
/// capability-safe handoff to the fabric service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Endpoint {
    pub node: NodeId,
    pub serial: u64,
    pub version: CxlVersion,
    pub device_type: DeviceType,
    pub component_register_base: u64,
    pub component_register_bytes: u32,
    pub hdm_decoder_offset: u32,
    pub volatile_capacity: u64,
    pub persistent_capacity: u64,
}

impl Endpoint {
    pub fn validate(self) -> Result<Self, Error> {
        if self.component_register_base == 0
            || self.component_register_bytes < 0x100
            || self.hdm_decoder_offset as u64 + 0x30 > self.component_register_bytes as u64
            || self
                .volatile_capacity
                .checked_add(self.persistent_capacity)
                .is_none()
        {
            return Err(Error::InvalidDevice)
        }
        Ok(self)
    }

    pub const fn capacity(self) -> u64 {
        self.volatile_capacity + self.persistent_capacity
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiscoveredDevice {
    pub endpoint: Endpoint,
    pub decoder_count: u8,
}

pub struct Registry<const CAPACITY: usize = MAX_CXL_DEVICES> {
    devices: [Option<DiscoveredDevice>; CAPACITY],
}

impl<const CAPACITY: usize> Registry<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            devices: [None; CAPACITY],
        }
    }

    /// Records a validated Type-3 endpoint discovered during boot.
    pub fn discover(
        &mut self,
        endpoint: Endpoint,
        decoder_count: u8,
    ) -> Result<usize, Error> {
        let endpoint = endpoint.validate()?;
        if endpoint.device_type != DeviceType::Type3 || endpoint.capacity() == 0 {
            return Err(Error::Unsupported)
        }
        if decoder_count == 0 || decoder_count > 32 {
            return Err(Error::InvalidDevice)
        }
        if let Some(index) = self.devices.iter().position(|entry| {
            entry.is_some_and(|device| device.endpoint.serial == endpoint.serial)
        }) {
            self.devices[index] = Some(DiscoveredDevice {
                endpoint,
                decoder_count,
            });
            return Ok(index)
        }
        let index = self
            .devices
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        self.devices[index] = Some(DiscoveredDevice {
            endpoint,
            decoder_count,
        });
        Ok(index)
    }

    pub fn devices(&self) -> impl Iterator<Item = &DiscoveredDevice> {
        self.devices.iter().flatten()
    }

    pub fn find_serial(&self, serial: u64) -> Option<&DiscoveredDevice> {
        self.devices()
            .find(|device| device.endpoint.serial == serial)
    }
}

impl<const CAPACITY: usize> Default for Registry<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecoderConfig {
    pub host_physical: AddressRange,
    pub device_physical_offset: u64,
    pub interleave_granularity: u8,
    pub interleave_ways: u8,
    pub host_only_coherent: bool,
}

impl DecoderConfig {
    fn validate(self) -> Result<Self, Error> {
        if self.host_physical.start % HDM_DECODER_GRANULARITY != 0
            || self.host_physical.length % HDM_DECODER_GRANULARITY != 0
            || self.device_physical_offset % HDM_DECODER_GRANULARITY != 0
            || self.interleave_granularity > 6
            || self.interleave_ways > 6
        {
            return Err(Error::Alignment)
        }
        Ok(self)
    }
}

/// Volatile access to a mapped CXL Component Register block.
///
/// Offset discovery happens from the PCI Register Locator and component
/// capability array; no physical register address is guessed here.
pub struct ComponentRegisters {
    base: NonNull<u8>,
    bytes: usize,
    hdm_offset: usize,
}

impl ComponentRegisters {
    const CAPABILITY: usize = 0x00;
    const DECODER_STRIDE: usize = 0x20;
    const DECODER_BASE: usize = 0x10;
    const BASE_LOW: usize = 0x00;
    const BASE_HIGH: usize = 0x04;
    const SIZE_LOW: usize = 0x08;
    const SIZE_HIGH: usize = 0x0c;
    const CONTROL: usize = 0x10;
    const DPA_SKIP_LOW: usize = 0x14;
    const DPA_SKIP_HIGH: usize = 0x18;
    const COMMIT: u32 = 1 << 9;
    const COMMITTED: u32 = 1 << 10;
    const COMMIT_ERROR: u32 = 1 << 11;
    const HOST_ONLY: u32 = 1 << 12;

    /// # Safety
    /// `base..base + bytes` must be an exclusive, uncached mapping of the
    /// endpoint's CXL Component Register block. `hdm_offset` must come from
    /// that block's component capability array.
    pub unsafe fn new(base: usize, bytes: usize, hdm_offset: usize) -> Result<Self, Error> {
        if base == 0
            || hdm_offset.checked_add(0x30).is_none()
            || hdm_offset + 0x30 > bytes
        {
            return Err(Error::InvalidDevice)
        }
        Ok(Self {
            base: NonNull::new(base as *mut u8).ok_or(Error::InvalidDevice)?,
            bytes,
            hdm_offset,
        })
    }

    pub fn decoder_count(&self) -> Result<u8, Error> {
        let encoded = (self.read(self.hdm_offset + Self::CAPABILITY) & 0x0f) as u8;
        if encoded > 5 {
            return Err(Error::Unsupported)
        }
        Ok(1 << encoded)
    }

    pub fn program_decoder(
        &mut self,
        decoder: u8,
        config: DecoderConfig,
        mut spin_limit: usize,
    ) -> Result<(), Error> {
        let config = config.validate()?;
        if decoder >= self.decoder_count()? {
            return Err(Error::InvalidDevice)
        }
        let offset = self
            .hdm_offset
            .checked_add(Self::DECODER_BASE + decoder as usize * Self::DECODER_STRIDE)
            .ok_or(Error::InvalidDevice)?;
        if offset + Self::DPA_SKIP_HIGH + size_of::<u32>() > self.bytes {
            return Err(Error::InvalidDevice)
        }
        let previous = self.read(offset + Self::CONTROL);
        if previous & Self::COMMITTED != 0 {
            return Err(Error::Busy)
        }

        self.write(offset + Self::BASE_LOW, config.host_physical.start as u32);
        self.write(
            offset + Self::BASE_HIGH,
            (config.host_physical.start >> 32) as u32,
        );
        self.write(offset + Self::SIZE_LOW, config.host_physical.length as u32);
        self.write(
            offset + Self::SIZE_HIGH,
            (config.host_physical.length >> 32) as u32,
        );
        self.write(
            offset + Self::DPA_SKIP_LOW,
            config.device_physical_offset as u32,
        );
        self.write(
            offset + Self::DPA_SKIP_HIGH,
            (config.device_physical_offset >> 32) as u32,
        );

        let mut control = config.interleave_granularity as u32
            | (config.interleave_ways as u32) << 4
            | Self::COMMIT;
        if config.host_only_coherent {
            control |= Self::HOST_ONLY
        }
        self.write(offset + Self::CONTROL, control);

        loop {
            let status = self.read(offset + Self::CONTROL);
            if status & Self::COMMIT_ERROR != 0 {
                return Err(Error::DecoderCommitFailed)
            }
            if status & Self::COMMITTED != 0 {
                return Ok(())
            }
            if spin_limit == 0 {
                return Err(Error::DecoderCommitFailed)
            }
            spin_limit -= 1;
            core::hint::spin_loop()
        }
    }

    fn read(&self, offset: usize) -> u32 {
        // Safety: construction proves the register block mapping and callers
        // only reach offsets checked against its length.
        unsafe { core::ptr::read_volatile(self.base.as_ptr().add(offset).cast()) }
    }

    fn write(&mut self, offset: usize, value: u32) {
        // Safety: same mapping invariant as read; ownership is exclusive.
        unsafe {
            core::ptr::write_volatile(self.base.as_ptr().add(offset).cast(), value)
        }
    }
}

/// Commits one endpoint HDM decoder and publishes the resulting memory window
/// in the cluster-global address space.
pub fn map_type3_device<const POOLS: usize, const OVERRIDES: usize>(
    device: &DiscoveredDevice,
    registers: &mut ComponentRegisters,
    decoder: u8,
    pool_id: PoolId,
    global: AddressRange,
    device_physical_offset: u64,
    mirror: Option<NodeId>,
    latency_ns: u32,
    space: &mut GlobalAddressSpace<POOLS, OVERRIDES>,
    spin_limit: usize,
) -> Result<MemoryPool, Error> {
    if decoder >= device.decoder_count
        || device_physical_offset
            .checked_add(global.length)
            .is_none_or(|end| end > device.endpoint.capacity())
    {
        return Err(Error::InvalidRange)
    }
    registers.program_decoder(
        decoder,
        DecoderConfig {
            host_physical: global,
            device_physical_offset,
            interleave_granularity: 0,
            interleave_ways: 0,
            host_only_coherent: true,
        },
        spin_limit,
    )?;
    let pool = MemoryPool {
        id: pool_id,
        node: device.endpoint.node,
        mirror,
        kind: MemoryKind::Ram,
        transport: Transport::Cxl,
        global,
        backing_start: device_physical_offset,
        latency_ns,
    };
    space.add_pool(pool)?;
    Ok(pool)
}
