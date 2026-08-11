use super::{SharedBuffer, SharedRegionId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum BufferOwner {
    IpcProducer = 1,
    IpcConsumer = 2,
    Network = 3,
    Storage = 4,
    WebGpu = 5,
    RpcClient = 6,
    RpcTransport = 7,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct BufferRights(u8);

impl BufferRights {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const TRANSFER: Self = Self(1 << 2);
    pub const ALL: Self = Self(Self::READ.0 | Self::WRITE.0 | Self::TRANSFER.0);

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferCapability {
    region: SharedRegionId,
    generation: u32,
    rights: BufferRights,
    token: u64,
}

impl BufferCapability {
    pub const fn new(
        region: SharedRegionId,
        generation: u32,
        rights: BufferRights,
        token: u64,
    ) -> Option<Self> {
        if generation == 0 || token == 0 || rights.0 == 0 {
            return None;
        }
        Some(Self {
            region,
            generation,
            rights,
            token,
        })
    }

    pub const fn region(self) -> SharedRegionId {
        self.region
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }

    pub const fn rights(self) -> BufferRights {
        self.rights
    }

    pub const fn token(self) -> u64 {
        self.token
    }

    pub const fn wire_word(self) -> u64 {
        self.generation as u64 | ((self.rights.bits() as u64) << 32)
    }

    pub fn authorize(
        self,
        descriptor: SharedBuffer,
        required: BufferRights,
    ) -> Result<(), BufferError> {
        if descriptor.region != self.region || !self.rights.contains(required) {
            return Err(BufferError::CapabilityDenied);
        }
        if descriptor.writable && !self.rights.contains(BufferRights::WRITE) {
            return Err(BufferError::CapabilityDenied);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BufferError {
    CapabilityDenied,
    InvalidDescriptor,
    OwnerMismatch,
}

/// A bounded borrow with an explicit capability and owner.
///
/// The lease is the only API used by guarded paths to access payload bytes.
/// Transferring it consumes the old owner and records the new owner before the
/// next subsystem can borrow the bytes.
pub struct BufferLease<'a> {
    descriptor: SharedBuffer,
    capability: BufferCapability,
    owner: BufferOwner,
    bytes: &'a mut [u8],
}

impl<'a> BufferLease<'a> {
    pub fn new(
        descriptor: SharedBuffer,
        capability: BufferCapability,
        owner: BufferOwner,
        bytes: &'a mut [u8],
    ) -> Result<Self, BufferError> {
        if descriptor.length as usize != bytes.len() {
            return Err(BufferError::InvalidDescriptor);
        }
        let required = if descriptor.writable {
            BufferRights::WRITE
        } else {
            BufferRights::READ
        };
        capability.authorize(descriptor, required)?;
        Ok(Self {
            descriptor,
            capability,
            owner,
            bytes,
        })
    }

    pub const fn descriptor(&self) -> SharedBuffer {
        self.descriptor
    }

    pub const fn capability(&self) -> BufferCapability {
        self.capability
    }

    pub const fn owner(&self) -> BufferOwner {
        self.owner
    }

    pub fn as_slice(&self) -> Result<&[u8], BufferError> {
        self.capability
            .authorize(self.descriptor, BufferRights::READ)?;
        Ok(self.bytes)
    }

    pub fn as_mut_slice(&mut self) -> Result<&mut [u8], BufferError> {
        self.capability
            .authorize(self.descriptor, BufferRights::WRITE)?;
        Ok(self.bytes)
    }

    pub fn transfer(self, next: BufferOwner) -> Result<Self, BufferError> {
        if self.owner == next {
            return Err(BufferError::OwnerMismatch);
        }
        if !self.capability.rights.contains(BufferRights::TRANSFER) {
            return Err(BufferError::CapabilityDenied);
        }
        Ok(Self { owner: next, ..self })
    }

    pub fn transfer_to(&mut self, next: BufferOwner) -> Result<(), BufferError> {
        if self.owner == next {
            return Err(BufferError::OwnerMismatch);
        }
        if !self.capability.rights.contains(BufferRights::TRANSFER) {
            return Err(BufferError::CapabilityDenied);
        }
        self.owner = next;
        Ok(())
    }
}
