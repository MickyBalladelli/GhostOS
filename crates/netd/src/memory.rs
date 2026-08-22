use ghostos_ipc::{
    BufferCapability, BufferError, BufferLease, BufferOwner, SharedBuffer, SharedRegionId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryError {
    InvalidRegion,
    InvalidRange,
    ReadOnly,
    Capability(BufferError),
}

/// Resolves IPC descriptors only for the duration of a callback.
///
/// This prevents a socket operation from retaining a pointer after the shared
/// region capability has been revoked or remapped.
pub trait SharedMemory {
    fn read<R>(
        &self,
        descriptor: SharedBuffer,
        read: impl FnOnce(&[u8]) -> R,
    ) -> Result<R, MemoryError>;

    fn write<R>(
        &mut self,
        descriptor: SharedBuffer,
        write: impl FnOnce(&mut [u8]) -> R,
    ) -> Result<R, MemoryError>;
}

pub struct MappedRegion<'a> {
    id: SharedRegionId,
    bytes: &'a mut [u8],
}

impl<'a> MappedRegion<'a> {
    pub const fn new(id: SharedRegionId, bytes: &'a mut [u8]) -> Self {
        Self { id, bytes }
    }

    fn range(&self, descriptor: SharedBuffer) -> Result<core::ops::Range<usize>, MemoryError> {
        if descriptor.region != self.id {
            return Err(MemoryError::InvalidRegion)
        }
        let start = descriptor.offset as usize;
        let end = start
            .checked_add(descriptor.length as usize)
            .ok_or(MemoryError::InvalidRange)?;
        if end > self.bytes.len() {
            return Err(MemoryError::InvalidRange)
        }
        Ok(start..end)
    }

    pub fn lease(
        &mut self,
        descriptor: SharedBuffer,
        capability: BufferCapability,
        owner: BufferOwner,
    ) -> Result<BufferLease<'_>, MemoryError> {
        let range = self.range(descriptor)?;
        BufferLease::new(descriptor, capability, owner, &mut self.bytes[range])
            .map_err(MemoryError::Capability)
    }
}

impl SharedMemory for MappedRegion<'_> {
    fn read<R>(
        &self,
        descriptor: SharedBuffer,
        read: impl FnOnce(&[u8]) -> R,
    ) -> Result<R, MemoryError> {
        let range = self.range(descriptor)?;
        Ok(read(&self.bytes[range]))
    }

    fn write<R>(
        &mut self,
        descriptor: SharedBuffer,
        write: impl FnOnce(&mut [u8]) -> R,
    ) -> Result<R, MemoryError> {
        if !descriptor.writable {
            return Err(MemoryError::ReadOnly)
        }
        let range = self.range(descriptor)?;
        Ok(write(&mut self.bytes[range]))
    }
}
