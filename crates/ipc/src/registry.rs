use super::{BufferCapability, BufferRights, SharedBuffer, SharedRegionId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct BufferPrincipal(u64);

impl BufferPrincipal {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RegistryState {
    Vacant,
    Live,
    Revoking,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegistryEntry {
    region: u32,
    generation: u32,
    rights: BufferRights,
    token: u64,
    owner: u64,
    lease_bits: u64,
    state: RegistryState,
}

impl RegistryEntry {
    const VACANT: Self = Self {
        region: 0,
        generation: 0,
        rights: BufferRights::READ,
        token: 0,
        owner: 0,
        lease_bits: 0,
        state: RegistryState::Vacant,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BufferLeasePermit {
    slot: u32,
    generation: u32,
    token: u64,
    lease_bit: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BufferRevocation {
    Complete,
    Pending { active_leases: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BufferRegistryError {
    Capacity,
    AlreadyRegistered,
    NotFound,
    OwnerMismatch,
    CapabilityDenied,
    Revoked,
    LeaseOverflow,
    InvalidDescriptor,
}

/// Kernel-owned lifetime table for shared IPC regions.
///
/// A revocation rejects new access immediately. Existing users must present
/// their permit when releasing the region; the slot is reusable only after
/// the last such permit is returned. This keeps generation reuse separate
/// from in-flight requests.
pub struct BufferRegistry<const CAPACITY: usize> {
    entries: [RegistryEntry; CAPACITY],
}

impl<const CAPACITY: usize> BufferRegistry<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            entries: [RegistryEntry::VACANT; CAPACITY],
        }
    }

    pub fn register(
        &mut self,
        owner: BufferPrincipal,
        region: SharedRegionId,
        rights: BufferRights,
        token: u64,
    ) -> Result<BufferCapability, BufferRegistryError> {
        if token == 0 || rights.bits() == 0 {
            return Err(BufferRegistryError::CapabilityDenied)
        }
        if self.entries.iter().any(|entry| {
            entry.state != RegistryState::Vacant && entry.region == region.raw()
        }) {
            return Err(BufferRegistryError::AlreadyRegistered)
        }
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.state == RegistryState::Vacant)
            .ok_or(BufferRegistryError::Capacity)?;
        let generation = entry.generation.wrapping_add(1).max(1);
        *entry = RegistryEntry {
            region: region.raw(),
            generation,
            rights,
            token,
            owner: owner.raw(),
            lease_bits: 0,
            state: RegistryState::Live,
        };
        BufferCapability::new(region, generation, rights, token)
            .ok_or(BufferRegistryError::CapabilityDenied)
    }

    pub fn authorize(
        &self,
        owner: BufferPrincipal,
        capability: BufferCapability,
        descriptor: SharedBuffer,
        required: BufferRights,
    ) -> Result<(), BufferRegistryError> {
        let entry = self.entry(capability)?;
        if entry.state != RegistryState::Live {
            return Err(BufferRegistryError::Revoked)
        }
        if entry.owner != owner.raw() {
            return Err(BufferRegistryError::OwnerMismatch)
        }
        capability
            .authorize(descriptor, required)
            .map_err(|_| BufferRegistryError::CapabilityDenied)
    }

    pub fn acquire(
        &mut self,
        owner: BufferPrincipal,
        capability: BufferCapability,
        descriptor: SharedBuffer,
        required: BufferRights,
    ) -> Result<BufferLeasePermit, BufferRegistryError> {
        self.authorize(owner, capability, descriptor, required)?;
        let slot = self.slot(capability)?;
        let entry = &mut self.entries[slot];
        let lease_bit = (!entry.lease_bits).trailing_zeros();
        if lease_bit == u64::BITS {
            return Err(BufferRegistryError::LeaseOverflow)
        }
        entry.lease_bits |= 1u64 << lease_bit;
        Ok(BufferLeasePermit {
            slot: slot as u32,
            generation: entry.generation,
            token: entry.token,
            lease_bit: lease_bit as u8,
        })
    }

    pub fn release(&mut self, permit: BufferLeasePermit) -> Result<(), BufferRegistryError> {
        let entry = self
            .entries
            .get_mut(permit.slot as usize)
            .ok_or(BufferRegistryError::NotFound)?;
        if entry.state == RegistryState::Vacant
            || entry.generation != permit.generation
            || entry.token != permit.token
            || entry.lease_bits & (1u64 << permit.lease_bit) == 0
        {
            return Err(BufferRegistryError::NotFound)
        }
        entry.lease_bits &= !(1u64 << permit.lease_bit);
        if entry.state == RegistryState::Revoking && entry.lease_bits == 0 {
            Self::vacate(entry)
        }
        Ok(())
    }

    pub fn transfer(
        &mut self,
        current: BufferPrincipal,
        next: BufferPrincipal,
        capability: BufferCapability,
    ) -> Result<(), BufferRegistryError> {
        let entry = self.entry_mut(capability)?;
        if entry.state != RegistryState::Live {
            return Err(BufferRegistryError::Revoked)
        }
        if entry.owner != current.raw() {
            return Err(BufferRegistryError::OwnerMismatch)
        }
        if entry.lease_bits != 0 {
            return Err(BufferRegistryError::LeaseOverflow)
        }
        if !entry.rights.contains(BufferRights::TRANSFER) {
            return Err(BufferRegistryError::CapabilityDenied)
        }
        entry.owner = next.raw();
        Ok(())
    }

    pub fn revoke(
        &mut self,
        owner: BufferPrincipal,
        region: SharedRegionId,
    ) -> Result<BufferRevocation, BufferRegistryError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.state != RegistryState::Vacant && entry.region == region.raw())
            .ok_or(BufferRegistryError::NotFound)?;
        if entry.owner != owner.raw() {
            return Err(BufferRegistryError::OwnerMismatch)
        }
        entry.state = RegistryState::Revoking;
        if entry.lease_bits == 0 {
            Self::vacate(entry);
            Ok(BufferRevocation::Complete)
        } else {
            Ok(BufferRevocation::Pending {
                active_leases: entry.lease_bits.count_ones(),
            })
        }
    }

    pub fn cleanup_owner(&mut self, owner: BufferPrincipal) -> usize {
        let mut cleaned = 0;
        for entry in &mut self.entries {
            if entry.state == RegistryState::Live && entry.owner == owner.raw() {
                entry.state = RegistryState::Revoking;
                if entry.lease_bits == 0 {
                    Self::vacate(entry)
                }
                cleaned += 1
            }
        }
        cleaned
    }

    pub fn active_leases(
        &self,
        capability: BufferCapability,
    ) -> Result<u32, BufferRegistryError> {
        Ok(self.entry(capability)?.lease_bits.count_ones())
    }

    fn slot(&self, capability: BufferCapability) -> Result<usize, BufferRegistryError> {
        self.entries
            .iter()
            .position(|entry| {
                entry.state != RegistryState::Vacant
                    && entry.region == capability.region().raw()
                    && entry.generation == capability.generation()
                    && entry.rights.bits() == capability.rights().bits()
                    && entry.token == capability.token()
            })
            .ok_or(BufferRegistryError::NotFound)
    }

    fn entry(
        &self,
        capability: BufferCapability,
    ) -> Result<&RegistryEntry, BufferRegistryError> {
        self.slot(capability).map(|slot| &self.entries[slot])
    }

    fn entry_mut(
        &mut self,
        capability: BufferCapability,
    ) -> Result<&mut RegistryEntry, BufferRegistryError> {
        let slot = self.slot(capability)?;
        Ok(&mut self.entries[slot])
    }

    fn vacate(entry: &mut RegistryEntry) {
        entry.region = 0;
        entry.token = 0;
        entry.owner = 0;
        entry.lease_bits = 0;
        entry.state = RegistryState::Vacant
    }
}

impl<const CAPACITY: usize> Default for BufferRegistry<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
