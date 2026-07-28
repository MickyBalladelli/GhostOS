use crate::ipc::{ChannelId, SharedRegionId};
use crate::task::AddressSpaceId;

pub const MAX_CAPABILITIES: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct CapabilityHandle(u64);

impl CapabilityHandle {
    const fn from_parts(slot: usize, generation: u32) -> Self {
        Self(((generation as u64) << 32) | slot as u64)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    pub const fn from_raw(raw: u64) -> Option<Self> {
        let generation = (raw >> 32) as u32;
        if generation == 0 {
            None
        } else {
            Some(Self(raw))
        }
    }

    const fn slot(self) -> usize {
        self.0 as u32 as usize
    }

    const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct Rights(u16);

impl Rights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXECUTE: Self = Self(1 << 2);
    pub const MAP: Self = Self(1 << 3);
    pub const CREATE: Self = Self(1 << 4);
    pub const SEND: Self = Self(1 << 5);
    pub const RECEIVE: Self = Self(1 << 6);
    pub const DELEGATE: Self = Self(1 << 7);
    pub const REVOKE: Self = Self(1 << 8);
    pub const ALL: Self = Self((1 << 9) - 1);

    pub const fn from_bits(bits: u16) -> Option<Self> {
        if bits & !Self::ALL.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    pub const fn bits(self) -> u16 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityObject {
    MemoryRegion(SharedRegionId),
    AddressSpace(AddressSpaceId),
    IpcChannel(ChannelId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityInfo {
    pub owner: AddressSpaceId,
    pub object: CapabilityObject,
    pub rights: Rights,
    pub parent: Option<CapabilityHandle>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityError {
    Full,
    InvalidHandle,
    AccessDenied,
    RightsEscalation,
    EmptyRights,
}

#[derive(Clone, Copy)]
struct CapabilityEntry {
    generation: u32,
    occupied: bool,
    info: CapabilityInfo,
}

impl CapabilityEntry {
    const VACANT: Self = Self {
        generation: 0,
        occupied: false,
        info: CapabilityInfo {
            owner: AddressSpaceId::KERNEL,
            object: CapabilityObject::AddressSpace(AddressSpaceId::KERNEL),
            rights: Rights::NONE,
            parent: None,
        },
    };
}

/// Fixed-size kernel capability space with a capability derivation tree.
///
/// User code only receives generation-checked handles. Ownership, object
/// identity, rights, and derivation links remain in protected kernel memory.
pub struct CapabilitySpace<const CAPACITY: usize = MAX_CAPABILITIES> {
    entries: [CapabilityEntry; CAPACITY],
}

impl<const CAPACITY: usize> CapabilitySpace<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            entries: [CapabilityEntry::VACANT; CAPACITY],
        }
    }

    /// Mint an initial capability for a trusted kernel or user-space manager.
    pub fn mint_root(
        &mut self,
        owner: AddressSpaceId,
        object: CapabilityObject,
        rights: Rights,
    ) -> Result<CapabilityHandle, CapabilityError> {
        self.insert(owner, object, rights, None)
    }

    /// Give another address space a child capability with equal or fewer rights.
    pub fn delegate(
        &mut self,
        caller: AddressSpaceId,
        source: CapabilityHandle,
        new_owner: AddressSpaceId,
        rights: Rights,
    ) -> Result<CapabilityHandle, CapabilityError> {
        let source_info = self.authorize_handle(caller, source, Rights::DELEGATE)?;
        if rights.is_empty() {
            return Err(CapabilityError::EmptyRights);
        }
        if !source_info.rights.contains(rights) {
            return Err(CapabilityError::RightsEscalation);
        }

        self.insert(new_owner, source_info.object, rights, Some(source))
    }

    pub fn authorize(
        &self,
        caller: AddressSpaceId,
        handle: CapabilityHandle,
        object: CapabilityObject,
        required: Rights,
    ) -> Result<(), CapabilityError> {
        let info = self.authorize_handle(caller, handle, required)?;
        if info.object != object {
            return Err(CapabilityError::AccessDenied);
        }
        Ok(())
    }

    pub fn authorize_mapping(
        &self,
        caller: AddressSpaceId,
        handle: CapabilityHandle,
        region: SharedRegionId,
        writable: bool,
        executable: bool,
    ) -> Result<(), CapabilityError> {
        let mut required = Rights::MAP.union(Rights::READ);
        if writable {
            required = required.union(Rights::WRITE)
        }
        if executable {
            required = required.union(Rights::EXECUTE)
        }
        self.authorize(
            caller,
            handle,
            CapabilityObject::MemoryRegion(region),
            required,
        )
    }

    pub fn inspect(
        &self,
        caller: AddressSpaceId,
        handle: CapabilityHandle,
    ) -> Result<CapabilityInfo, CapabilityError> {
        self.authorize_handle(caller, handle, Rights::NONE)
    }

    /// Revoke every capability derived from `authority`, preserving authority.
    pub fn revoke(
        &mut self,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
    ) -> Result<usize, CapabilityError> {
        self.authorize_handle(caller, authority, Rights::REVOKE)?;

        let mut descendants = [false; CAPACITY];
        for (slot, descendant) in descendants.iter_mut().enumerate() {
            *descendant = self.entries[slot].occupied && self.is_descendant(slot, authority)
        }

        let mut revoked = 0;
        for (slot, descendant) in descendants.iter().enumerate() {
            if *descendant {
                self.entries[slot].occupied = false;
                revoked += 1
            }
        }
        Ok(revoked)
    }

    /// Drop an owned handle and all authority derived from it.
    pub fn delete(
        &mut self,
        caller: AddressSpaceId,
        handle: CapabilityHandle,
    ) -> Result<usize, CapabilityError> {
        self.authorize_handle(caller, handle, Rights::NONE)?;
        let revoked = self.revoke_descendants_unchecked(handle);
        let slot = self.valid_slot(handle)?;
        self.entries[slot].occupied = false;
        Ok(revoked + 1)
    }

    pub fn used(&self) -> usize {
        self.entries.iter().filter(|entry| entry.occupied).count()
    }

    pub const fn capacity(&self) -> usize {
        CAPACITY
    }

    fn insert(
        &mut self,
        owner: AddressSpaceId,
        object: CapabilityObject,
        rights: Rights,
        parent: Option<CapabilityHandle>,
    ) -> Result<CapabilityHandle, CapabilityError> {
        if rights.is_empty() {
            return Err(CapabilityError::EmptyRights);
        }
        let slot = self
            .entries
            .iter()
            .position(|entry| !entry.occupied)
            .ok_or(CapabilityError::Full)?;
        let generation = self.entries[slot].generation.wrapping_add(1).max(1);
        let handle = CapabilityHandle::from_parts(slot, generation);
        self.entries[slot] = CapabilityEntry {
            generation,
            occupied: true,
            info: CapabilityInfo {
                owner,
                object,
                rights,
                parent,
            },
        };
        Ok(handle)
    }

    fn authorize_handle(
        &self,
        caller: AddressSpaceId,
        handle: CapabilityHandle,
        required: Rights,
    ) -> Result<CapabilityInfo, CapabilityError> {
        let slot = self.valid_slot(handle)?;
        let info = self.entries[slot].info;
        if info.owner != caller || !info.rights.contains(required) {
            return Err(CapabilityError::AccessDenied);
        }
        Ok(info)
    }

    fn valid_slot(&self, handle: CapabilityHandle) -> Result<usize, CapabilityError> {
        let slot = handle.slot();
        let entry = self
            .entries
            .get(slot)
            .ok_or(CapabilityError::InvalidHandle)?;
        if !entry.occupied || entry.generation != handle.generation() {
            return Err(CapabilityError::InvalidHandle);
        }
        Ok(slot)
    }

    fn is_descendant(&self, slot: usize, ancestor: CapabilityHandle) -> bool {
        let mut parent = self.entries[slot].info.parent;
        for _ in 0..CAPACITY {
            let Some(handle) = parent else {
                return false;
            };
            if handle == ancestor {
                return true;
            }
            let Ok(parent_slot) = self.valid_slot(handle) else {
                return false;
            };
            parent = self.entries[parent_slot].info.parent
        }
        false
    }

    fn revoke_descendants_unchecked(&mut self, authority: CapabilityHandle) -> usize {
        let mut descendants = [false; CAPACITY];
        for (slot, descendant) in descendants.iter_mut().enumerate() {
            *descendant = self.entries[slot].occupied && self.is_descendant(slot, authority)
        }

        let mut revoked = 0;
        for (slot, descendant) in descendants.iter().enumerate() {
            if *descendant {
                self.entries[slot].occupied = false;
                revoked += 1
            }
        }
        revoked
    }
}

impl<const CAPACITY: usize> Default for CapabilitySpace<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
