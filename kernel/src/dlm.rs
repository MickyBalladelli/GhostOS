use crate::capability::{
    CapabilityError, CapabilityHandle, CapabilityObject, CapabilitySpace, Rights,
};
use crate::task::AddressSpaceId;
use synos_status::{IntoStatus, Severity, Status, facility};

pub const MAX_RESOURCE_NAME_BYTES: usize = 64;
pub const DEFAULT_LOCK_CAPACITY: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct NodeId(u32);

impl NodeId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ResourceId(u64);

impl ResourceId {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceKind {
    SharedMemory,
    File,
    Named,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockRange {
    WholeObject,
    Bytes { start: u64, length: u64 },
}

impl LockRange {
    pub const fn bytes(start: u64, length: u64) -> Result<Self, LockError> {
        if length == 0 || start.checked_add(length).is_none() {
            Err(LockError::InvalidRange)
        } else {
            Ok(Self::Bytes { start, length })
        }
    }

    pub const fn overlaps(self, other: Self) -> bool {
        match (self, other) {
            (Self::WholeObject, _) | (_, Self::WholeObject) => true,
            (
                Self::Bytes {
                    start: left,
                    length: left_length,
                },
                Self::Bytes {
                    start: right,
                    length: right_length,
                },
            ) => left < right + right_length && right < left + left_length,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceName {
    bytes: [u8; MAX_RESOURCE_NAME_BYTES],
    len: u8,
}

impl ResourceName {
    pub fn new(name: &str) -> Result<Self, LockError> {
        let bytes = name.as_bytes();
        if bytes.is_empty() || bytes.len() > MAX_RESOURCE_NAME_BYTES || bytes.contains(&0) {
            return Err(LockError::InvalidResource);
        }
        let mut stored = [0; MAX_RESOURCE_NAME_BYTES];
        stored[..bytes.len()].copy_from_slice(bytes);
        Ok(Self {
            bytes: stored,
            len: bytes.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).expect("ResourceName invariant")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LockOwner {
    pub node: NodeId,
    pub address_space: AddressSpaceId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum LockMode {
    Null = 0,
    ConcurrentRead = 1,
    ConcurrentWrite = 2,
    ProtectedRead = 3,
    ProtectedWrite = 4,
    Exclusive = 5,
}

impl LockMode {
    const fn required_rights(self) -> Rights {
        match self {
            Self::Null | Self::ConcurrentRead | Self::ProtectedRead => Rights::READ,
            Self::ConcurrentWrite | Self::ProtectedWrite | Self::Exclusive => Rights::WRITE,
        }
    }

    /// OpenVMS DLM compatibility for NL, CR, CW, PR, PW, and EX modes.
    pub const fn compatible(self, granted: Self) -> bool {
        const MATRIX: [[bool; 6]; 6] = [
            [true, true, true, true, true, true],
            [true, true, true, true, true, false],
            [true, true, true, false, false, false],
            [true, true, false, true, false, false],
            [true, true, false, false, false, false],
            [true, false, false, false, false, false],
        ];
        MATRIX[self as usize][granted as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct LockHandle(u64);

impl LockHandle {
    const fn from_parts(slot: usize, generation: u32) -> Self {
        Self(((generation as u64) << 32) | slot as u64)
    }

    const fn slot(self) -> usize {
        self.0 as u32 as usize
    }

    const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockGrant {
    Granted(LockHandle),
    Queued(LockHandle),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockError {
    AccessDenied,
    Capacity,
    InvalidHandle,
    InvalidResource,
    InvalidRange,
    NotOwner,
    Expired,
    WouldBlock,
}

impl From<CapabilityError> for LockError {
    fn from(_: CapabilityError) -> Self {
        Self::AccessDenied
    }
}

impl IntoStatus for LockError {
    fn status(self) -> Status {
        let (severity, code) = match self {
            Self::WouldBlock => (Severity::Warning, 1),
            Self::Capacity => (Severity::Error, 2),
            Self::InvalidHandle => (Severity::Error, 3),
            Self::InvalidResource => (Severity::Error, 4),
            Self::InvalidRange => (Severity::Error, 5),
            Self::Expired => (Severity::Warning, 6),
            Self::AccessDenied | Self::NotOwner => return Status::ACCESS_DENIED,
        };
        Status::new(severity, facility::DLM, code, 0).expect("valid DLM status")
    }
}

#[derive(Clone, Copy)]
struct LockEntry {
    occupied: bool,
    generation: u32,
    resource: ResourceId,
    kind: ResourceKind,
    name: ResourceName,
    owner: LockOwner,
    mode: LockMode,
    granted: bool,
    sequence: u64,
    range: LockRange,
    lease_epoch: u64,
    expires_at_us: u64,
}

impl LockEntry {
    const VACANT: Self = Self {
        occupied: false,
        generation: 0,
        resource: ResourceId(0),
        kind: ResourceKind::Named,
        name: ResourceName {
            bytes: [0; MAX_RESOURCE_NAME_BYTES],
            len: 0,
        },
        owner: LockOwner {
            node: NodeId(0),
            address_space: AddressSpaceId::KERNEL,
        },
        mode: LockMode::Null,
        granted: false,
        sequence: 0,
        range: LockRange::WholeObject,
        lease_epoch: 0,
        expires_at_us: 0,
    };
}

/// Kernel-resident, fixed-capacity cluster lock table.
///
/// Requests are capability checked. Conflicting requests may wait in FIFO
/// order; releasing a lock promotes every compatible waiter.
pub struct DistributedLockManager<const CAPACITY: usize = DEFAULT_LOCK_CAPACITY> {
    locks: [LockEntry; CAPACITY],
    sequence: u64,
}

impl<const CAPACITY: usize> DistributedLockManager<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            locks: [LockEntry::VACANT; CAPACITY],
            sequence: 0,
        }
    }

    pub fn acquire<const CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<CAPABILITIES>,
        authority: CapabilityHandle,
        owner: LockOwner,
        resource: ResourceId,
        kind: ResourceKind,
        name: ResourceName,
        mode: LockMode,
        wait: bool,
    ) -> Result<LockGrant, LockError> {
        self.acquire_inner(
            capabilities,
            authority,
            owner,
            resource,
            kind,
            name,
            LockRange::WholeObject,
            mode,
            wait,
            u64::MAX,
        )
    }

    /// Acquire a renewable object or byte-range lease.
    ///
    /// Disjoint byte ranges never conflict, avoiding 4 KiB false sharing.
    pub fn acquire_range<const CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<CAPABILITIES>,
        authority: CapabilityHandle,
        owner: LockOwner,
        resource: ResourceId,
        kind: ResourceKind,
        name: ResourceName,
        range: LockRange,
        mode: LockMode,
        wait: bool,
        now_us: u64,
        lease_duration_us: u64,
    ) -> Result<LockGrant, LockError> {
        if lease_duration_us == 0
            || matches!(
                range,
                LockRange::Bytes { start, length }
                    if length == 0 || start.checked_add(length).is_none()
            )
        {
            return Err(LockError::InvalidRange);
        }
        let expires_at_us = now_us
            .checked_add(lease_duration_us)
            .ok_or(LockError::InvalidRange)?;
        self.expire(now_us);
        self.acquire_inner(
            capabilities,
            authority,
            owner,
            resource,
            kind,
            name,
            range,
            mode,
            wait,
            expires_at_us,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn acquire_inner<const CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<CAPABILITIES>,
        authority: CapabilityHandle,
        owner: LockOwner,
        resource: ResourceId,
        kind: ResourceKind,
        name: ResourceName,
        range: LockRange,
        mode: LockMode,
        wait: bool,
        expires_at_us: u64,
    ) -> Result<LockGrant, LockError> {
        capabilities.authorize(
            owner.address_space,
            authority,
            CapabilityObject::DistributedResource(resource),
            mode.required_rights(),
        )?;
        if self.locks.iter().any(|entry| {
            entry.occupied
                && entry.resource == resource
                && (entry.kind != kind || entry.name != name)
        }) {
            return Err(LockError::InvalidResource);
        }

        let compatible =
            !self.has_waiter(resource, range) && self.can_grant(resource, range, mode, None);
        if !compatible && !wait {
            return Err(LockError::WouldBlock);
        }
        let slot = self
            .locks
            .iter()
            .position(|entry| !entry.occupied)
            .ok_or(LockError::Capacity)?;
        let generation = self.locks[slot].generation.wrapping_add(1).max(1);
        self.sequence = self.sequence.wrapping_add(1);
        let handle = LockHandle::from_parts(slot, generation);
        self.locks[slot] = LockEntry {
            occupied: true,
            generation,
            resource,
            kind,
            name,
            owner,
            mode,
            granted: compatible,
            sequence: self.sequence,
            range,
            lease_epoch: 1,
            expires_at_us,
        };
        Ok(if compatible {
            LockGrant::Granted(handle)
        } else {
            LockGrant::Queued(handle)
        })
    }

    pub fn convert<const CAPABILITIES: usize>(
        &mut self,
        capabilities: &CapabilitySpace<CAPABILITIES>,
        authority: CapabilityHandle,
        owner: LockOwner,
        handle: LockHandle,
        mode: LockMode,
    ) -> Result<(), LockError> {
        let slot = self.owned_slot(handle, owner)?;
        let resource = self.locks[slot].resource;
        capabilities.authorize(
            owner.address_space,
            authority,
            CapabilityObject::DistributedResource(resource),
            mode.required_rights(),
        )?;
        let range = self.locks[slot].range;
        if !self.locks[slot].granted || !self.can_grant(resource, range, mode, Some(slot)) {
            return Err(LockError::WouldBlock);
        }
        self.locks[slot].mode = mode;
        self.promote(resource);
        Ok(())
    }

    pub fn release(&mut self, owner: LockOwner, handle: LockHandle) -> Result<usize, LockError> {
        let slot = self.owned_slot(handle, owner)?;
        let resource = self.locks[slot].resource;
        self.locks[slot].occupied = false;
        Ok(self.promote(resource))
    }

    pub fn renew(
        &mut self,
        owner: LockOwner,
        handle: LockHandle,
        expected_epoch: u64,
        now_us: u64,
        duration_us: u64,
    ) -> Result<u64, LockError> {
        if duration_us == 0 {
            return Err(LockError::InvalidRange);
        }
        let slot = self.owned_slot(handle, owner)?;
        let entry = &mut self.locks[slot];
        if entry.expires_at_us <= now_us || entry.lease_epoch != expected_epoch {
            return Err(LockError::Expired);
        }
        entry.expires_at_us = now_us
            .checked_add(duration_us)
            .ok_or(LockError::InvalidRange)?;
        entry.lease_epoch = entry.lease_epoch.wrapping_add(1).max(1);
        Ok(entry.lease_epoch)
    }

    pub fn expire(&mut self, now_us: u64) -> usize {
        let mut expired = 0;
        for entry in &mut self.locks {
            if entry.occupied && entry.expires_at_us <= now_us {
                entry.occupied = false;
                expired += 1
            }
        }
        if expired != 0 {
            self.promote_all()
        }
        expired
    }

    /// Remove locks held or queued by a failed cluster node.
    pub fn release_node(&mut self, node: NodeId) -> usize {
        let mut removed = 0;
        for entry in &mut self.locks {
            if entry.occupied && entry.owner.node == node {
                entry.occupied = false;
                removed += 1
            }
        }
        self.promote_all();
        removed
    }

    pub fn is_granted(&self, handle: LockHandle) -> Result<bool, LockError> {
        Ok(self.locks[self.valid_slot(handle)?].granted)
    }

    pub fn resource(
        &self,
        handle: LockHandle,
    ) -> Result<(ResourceId, ResourceKind, ResourceName), LockError> {
        let entry = self.locks[self.valid_slot(handle)?];
        Ok((entry.resource, entry.kind, entry.name))
    }

    pub fn lease(&self, handle: LockHandle) -> Result<(LockRange, u64, u64), LockError> {
        let entry = self.locks[self.valid_slot(handle)?];
        Ok((entry.range, entry.lease_epoch, entry.expires_at_us))
    }

    pub fn used(&self) -> usize {
        self.locks.iter().filter(|entry| entry.occupied).count()
    }

    fn can_grant(
        &self,
        resource: ResourceId,
        range: LockRange,
        mode: LockMode,
        except: Option<usize>,
    ) -> bool {
        self.locks.iter().enumerate().all(|(slot, entry)| {
            Some(slot) == except
                || !entry.occupied
                || !entry.granted
                || entry.resource != resource
                || !range.overlaps(entry.range)
                || mode.compatible(entry.mode)
        })
    }

    fn has_waiter(&self, resource: ResourceId, range: LockRange) -> bool {
        self.locks.iter().any(|entry| {
            entry.occupied
                && !entry.granted
                && entry.resource == resource
                && range.overlaps(entry.range)
        })
    }

    fn promote(&mut self, resource: ResourceId) -> usize {
        let mut promoted = 0;
        loop {
            let next = self
                .locks
                .iter()
                .enumerate()
                .filter(|(_, entry)| entry.occupied && !entry.granted && entry.resource == resource)
                .filter(|(slot, entry)| {
                    self.can_grant(resource, entry.range, entry.mode, Some(*slot))
                        && !self.locks.iter().any(|older| {
                            older.occupied
                                && !older.granted
                                && older.resource == resource
                                && older.sequence < entry.sequence
                                && older.range.overlaps(entry.range)
                        })
                })
                .min_by_key(|(_, entry)| entry.sequence)
                .map(|(slot, _)| slot);
            let Some(slot) = next else { break };
            self.locks[slot].granted = true;
            promoted += 1
        }
        promoted
    }

    fn promote_all(&mut self) {
        for slot in 0..CAPACITY {
            if self.locks[slot].occupied && !self.locks[slot].granted {
                let resource = self.locks[slot].resource;
                self.promote(resource);
            }
        }
    }

    fn owned_slot(&self, handle: LockHandle, owner: LockOwner) -> Result<usize, LockError> {
        let slot = self.valid_slot(handle)?;
        if self.locks[slot].owner != owner {
            return Err(LockError::NotOwner);
        }
        Ok(slot)
    }

    fn valid_slot(&self, handle: LockHandle) -> Result<usize, LockError> {
        let slot = handle.slot();
        let entry = self.locks.get(slot).ok_or(LockError::InvalidHandle)?;
        if !entry.occupied || entry.generation != handle.generation() {
            return Err(LockError::InvalidHandle);
        }
        Ok(slot)
    }
}

impl<const CAPACITY: usize> Default for DistributedLockManager<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
