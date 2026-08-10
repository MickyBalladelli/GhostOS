use crate::dlm::ResourceId;
use crate::ipc::{ChannelId, SharedRegionId};
use crate::quota::{CapabilityQuota, QuotaDecision, QuotaPolicy, QuotaResource, QuotaUsage};
use crate::task::AddressSpaceId;
use synos_observability::{
    CapabilityDomain, CapabilityTraceStage, EventField, Level, audit_event,
    emit_capability_trace, field,
};
use synos_status::{IntoStatus, Severity, Status, facility};

pub const MAX_CAPABILITIES: usize = 256;
const NO_DESCRIPTOR: usize = usize::MAX;

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
    /// Permit administrative control of a process or task.
    pub const CONTROL: Self = Self(1 << 9);
    pub const ALL: Self = Self((1 << 10) - 1);

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
pub struct PhysicalRange {
    pub start: u64,
    pub length: u64,
}

impl PhysicalRange {
    pub const fn new(start: u64, length: u64) -> Option<Self> {
        if length == 0 || start.checked_add(length).is_none() {
            None
        } else {
            Some(Self { start, length })
        }
    }

    pub const fn contains(self, other: Self) -> bool {
        other.start >= self.start
            && other.start.saturating_add(other.length) <= self.start.saturating_add(self.length)
    }

    pub const fn overlaps(self, other: Self) -> bool {
        self.start < other.start.saturating_add(other.length)
            && other.start < self.start.saturating_add(self.length)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityObject {
    UntypedMemory(PhysicalRange),
    MemoryRegion(SharedRegionId),
    AddressSpace(AddressSpaceId),
    Thread(crate::task::ThreadId),
    SystemControl,
    IpcChannel(ChannelId),
    DistributedResource(ResourceId),
    LogicalNamespace { scope: u8, id: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityInfo {
    pub owner: AddressSpaceId,
    pub object: CapabilityObject,
    pub rights: Rights,
    pub parent: Option<CapabilityHandle>,
    pub backing: Option<PhysicalRange>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityLinks {
    pub parent: Option<CapabilityHandle>,
    pub first_child: Option<CapabilityHandle>,
    pub next_sibling: Option<CapabilityHandle>,
}

pub trait CapabilityRevocationHook {
    fn revoke(&mut self, capability: CapabilityInfo);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityError {
    Full,
    InvalidHandle,
    AccessDenied,
    RightsEscalation,
    EmptyRights,
    InvalidQuota,
}

impl IntoStatus for CapabilityError {
    fn status(self) -> Status {
        match self {
            Self::AccessDenied | Self::RightsEscalation => Status::ACCESS_DENIED,
            Self::Full => Status::NO_SPACE,
            Self::InvalidHandle | Self::EmptyRights | Self::InvalidQuota => {
                Status::new(Severity::Error, facility::KERNEL, 1, 0)
                    .expect("valid capability status")
            }
        }
    }
}

#[derive(Clone, Copy)]
#[repr(C, align(64))]
struct CapabilityDescriptorPage {
    generation: u32,
    occupied: bool,
    info: CapabilityInfo,
    parent_slot: usize,
    first_child: usize,
    next_sibling: usize,
}

impl CapabilityDescriptorPage {
    const VACANT: Self = Self {
        generation: 0,
        occupied: false,
        info: CapabilityInfo {
            owner: AddressSpaceId::KERNEL,
            object: CapabilityObject::AddressSpace(AddressSpaceId::KERNEL),
            rights: Rights::NONE,
            parent: None,
            backing: None,
        },
        parent_slot: NO_DESCRIPTOR,
        first_child: NO_DESCRIPTOR,
        next_sibling: NO_DESCRIPTOR,
    };
}

/// Fixed-size kernel capability space with a capability derivation tree.
///
/// User code only receives generation-checked handles. Ownership, object
/// identity, rights, and derivation links remain in protected kernel memory.
pub struct CapabilitySpace<const CAPACITY: usize = MAX_CAPABILITIES> {
    entries: [CapabilityDescriptorPage; CAPACITY],
    quotas: [CapabilityQuota; CAPACITY],
}

impl<const CAPACITY: usize> CapabilitySpace<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            entries: [CapabilityDescriptorPage::VACANT; CAPACITY],
            quotas: [const { CapabilityQuota::new() }; CAPACITY],
        }
    }

    /// Mint an initial capability for a trusted kernel or user-space manager.
    pub fn mint_root(
        &mut self,
        owner: AddressSpaceId,
        object: CapabilityObject,
        rights: Rights,
    ) -> Result<CapabilityHandle, CapabilityError> {
        self.insert(owner, object, rights, None, None)
    }

    /// Seed physical memory as an untyped token for a user-space manager.
    pub fn mint_untyped(
        &mut self,
        owner: AddressSpaceId,
        memory: PhysicalRange,
        rights: Rights,
    ) -> Result<CapabilityHandle, CapabilityError> {
        self.insert(
            owner,
            CapabilityObject::UntypedMemory(memory),
            rights,
            None,
            Some(memory),
        )
    }

    /// Retype part of an untyped token into a shared-memory resource.
    ///
    /// No kernel heap is used. The CDT node and physical backing metadata live
    /// in the fixed resource descriptor slot itself.
    pub fn retype_memory(
        &mut self,
        caller: AddressSpaceId,
        source: CapabilityHandle,
        new_owner: AddressSpaceId,
        region: SharedRegionId,
        memory: PhysicalRange,
        rights: Rights,
    ) -> Result<CapabilityHandle, CapabilityError> {
        let source_info = self.authorize_handle(caller, source, Rights::CREATE)?;
        let CapabilityObject::UntypedMemory(untyped) = source_info.object else {
            return Err(CapabilityError::AccessDenied);
        };
        if rights.is_empty() || !source_info.rights.contains(rights) {
            return Err(if rights.is_empty() {
                CapabilityError::EmptyRights
            } else {
                CapabilityError::RightsEscalation
            });
        }
        if !untyped.contains(memory) {
            return Err(CapabilityError::AccessDenied);
        }
        let source_slot = self.valid_slot(source)?;
        if self.entries.iter().enumerate().any(|(slot, entry)| {
            slot != source_slot
                && entry.occupied
                && self.is_descendant(slot, source)
                && entry
                    .info
                    .backing
                    .is_some_and(|backing| backing.overlaps(memory))
        }) {
            return Err(CapabilityError::AccessDenied);
        }
        self.insert(
            new_owner,
            CapabilityObject::MemoryRegion(region),
            rights,
            Some(source),
            Some(memory),
        )
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

        self.insert(
            new_owner,
            source_info.object,
            rights,
            Some(source),
            source_info.backing,
        )
    }

    /// Permanently remove rights from an owned handle.
    pub fn drop_rights(
        &mut self,
        caller: AddressSpaceId,
        handle: CapabilityHandle,
        rights: Rights,
    ) -> Result<Rights, CapabilityError> {
        let slot = self.valid_slot(handle)?;
        let info = self.entries[slot].info;
        if info.owner != caller {
            return Err(CapabilityError::AccessDenied)
        }
        let remaining = Rights::from_bits(info.rights.bits() & !rights.bits())
            .expect("masked capability rights");
        if remaining.is_empty() {
            return Err(CapabilityError::EmptyRights)
        }
        self.entries[slot].info.rights = remaining;
        audit_event!(
            Level::Info,
            EventField::unsigned(field::OPERATION, 4),
            EventField::unsigned(field::CAPABILITY, handle.raw()),
            EventField::unsigned(field::CALLER, caller.raw() as u64),
            EventField::unsigned(field::RIGHTS, remaining.bits() as u64),
        );
        Ok(remaining)
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

    /// Replace the resource policy for a live capability. Only an owner with
    /// CONTROL may change a tenant's limits. Child capabilities inherit the
    /// policy that was active on their parent when they were delegated.
    pub fn configure_quota(
        &mut self,
        caller: AddressSpaceId,
        handle: CapabilityHandle,
        policy: QuotaPolicy,
    ) -> Result<(), CapabilityError> {
        let slot = self.valid_slot(handle)?;
        let info = self.entries[slot].info;
        if info.owner != caller || !info.rights.contains(Rights::CONTROL) {
            return Err(CapabilityError::AccessDenied)
        }
        if !policy.is_valid() {
            return Err(CapabilityError::InvalidQuota)
        }
        self.quotas[slot].configure(policy);
        Ok(())
    }

    pub fn quota_policy(
        &self,
        caller: AddressSpaceId,
        handle: CapabilityHandle,
    ) -> Result<QuotaPolicy, CapabilityError> {
        let slot = self.valid_slot(handle)?;
        if self.entries[slot].info.owner != caller {
            return Err(CapabilityError::AccessDenied)
        }
        Ok(self.quotas[slot].policy())
    }

    pub fn quota_usage(
        &self,
        caller: AddressSpaceId,
        handle: CapabilityHandle,
    ) -> Result<QuotaUsage, CapabilityError> {
        let slot = self.valid_slot(handle)?;
        if self.entries[slot].info.owner != caller {
            return Err(CapabilityError::AccessDenied)
        }
        Ok(self.quotas[slot].usage())
    }

    pub fn consume_quota(
        &self,
        caller: AddressSpaceId,
        handle: CapabilityHandle,
        resource: QuotaResource,
        now_us: u64,
        amount: u64,
    ) -> Result<QuotaDecision, CapabilityError> {
        let slot = self.valid_slot(handle)?;
        if self.entries[slot].info.owner != caller {
            return Err(CapabilityError::AccessDenied)
        }
        Ok(self.quotas[slot].consume(resource, now_us, amount))
    }

    pub fn refund_quota(
        &self,
        caller: AddressSpaceId,
        handle: CapabilityHandle,
        resource: QuotaResource,
        amount: u64,
    ) -> Result<(), CapabilityError> {
        let slot = self.valid_slot(handle)?;
        if self.entries[slot].info.owner != caller {
            return Err(CapabilityError::AccessDenied)
        }
        self.quotas[slot].refund(resource, amount);
        Ok(())
    }

    pub fn release_memory(
        &self,
        caller: AddressSpaceId,
        handle: CapabilityHandle,
        amount: u64,
    ) -> Result<(), CapabilityError> {
        let slot = self.valid_slot(handle)?;
        if self.entries[slot].info.owner != caller {
            return Err(CapabilityError::AccessDenied)
        }
        self.quotas[slot].release_memory(amount);
        Ok(())
    }

    pub fn links(
        &self,
        caller: AddressSpaceId,
        handle: CapabilityHandle,
    ) -> Result<CapabilityLinks, CapabilityError> {
        self.authorize_handle(caller, handle, Rights::NONE)?;
        let slot = self.valid_slot(handle)?;
        let entry = &self.entries[slot];
        Ok(CapabilityLinks {
            parent: entry.info.parent,
            first_child: self.handle_for_slot(entry.first_child),
            next_sibling: self.handle_for_slot(entry.next_sibling),
        })
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
                self.vacate(slot);
                revoked += 1
            }
        }
        audit_event!(
            Level::Info,
            EventField::unsigned(field::OPERATION, 2),
            EventField::unsigned(field::CAPABILITY, authority.raw()),
            EventField::unsigned(field::CALLER, caller.raw() as u64),
            EventField::unsigned(field::LENGTH, revoked as u64),
        );
        self.trace_revocation(authority, 2);
        Ok(revoked)
    }

    /// Revoke descendants and notify the mapper/fabric before handles vanish.
    pub fn revoke_with_hook<H: CapabilityRevocationHook>(
        &mut self,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        hook: &mut H,
    ) -> Result<usize, CapabilityError> {
        self.authorize_handle(caller, authority, Rights::REVOKE)?;
        let mut descendants = [false; CAPACITY];
        for (slot, descendant) in descendants.iter_mut().enumerate() {
            *descendant = self.entries[slot].occupied && self.is_descendant(slot, authority)
        }
        let mut revoked = 0;
        for (slot, descendant) in descendants.iter().enumerate() {
            if *descendant {
                hook.revoke(self.entries[slot].info);
                self.vacate(slot);
                revoked += 1
            }
        }
        audit_event!(
            Level::Info,
            EventField::unsigned(field::OPERATION, 2),
            EventField::unsigned(field::CAPABILITY, authority.raw()),
            EventField::unsigned(field::CALLER, caller.raw() as u64),
            EventField::unsigned(field::LENGTH, revoked as u64),
        );
        self.trace_revocation(authority, 2);
        Ok(revoked)
    }

    /// Revoke a distributed-memory capability subtree after checking that the
    /// authority names the requested remote resource.
    pub fn revoke_remote_memory(
        &mut self,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        resource: ResourceId,
    ) -> Result<usize, CapabilityError> {
        self.authorize(
            caller,
            authority,
            CapabilityObject::DistributedResource(resource),
            Rights::REVOKE,
        )?;
        self.revoke_descendants(caller, authority)
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
        self.vacate(slot);
        audit_event!(
            Level::Info,
            EventField::unsigned(field::OPERATION, 2),
            EventField::unsigned(field::CAPABILITY, handle.raw()),
            EventField::unsigned(field::CALLER, caller.raw() as u64),
            EventField::unsigned(field::LENGTH, (revoked + 1) as u64),
        );
        self.trace_revocation(handle, 2);
        Ok(revoked + 1)
    }

    pub fn used(&self) -> usize {
        self.entries.iter().filter(|entry| entry.occupied).count()
    }

    /// Snapshot every live descriptor for trusted diagnostics such as
    /// `synos-top`. User-space visibility is still decided by the service that
    /// owns this kernel capability space.
    pub fn entries(
        &self,
    ) -> impl Iterator<Item = (CapabilityHandle, CapabilityInfo)> + '_ {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.occupied)
            .map(|(slot, entry)| {
                (
                    CapabilityHandle::from_parts(slot, entry.generation),
                    entry.info,
                )
            })
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
        backing: Option<PhysicalRange>,
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
        let parent_slot = match parent {
            Some(handle) => self.valid_slot(handle)?,
            None => NO_DESCRIPTOR,
        };
        let next_sibling = if parent_slot == NO_DESCRIPTOR {
            NO_DESCRIPTOR
        } else {
            self.entries[parent_slot].first_child
        };
        let inherited_policy = if parent_slot == NO_DESCRIPTOR {
            QuotaPolicy::default()
        } else {
            self.quotas[parent_slot].policy()
        };
        self.entries[slot] = CapabilityDescriptorPage {
            generation,
            occupied: true,
            info: CapabilityInfo {
                owner,
                object,
                rights,
                parent,
                backing,
            },
            parent_slot,
            first_child: NO_DESCRIPTOR,
            next_sibling,
        };
        self.quotas[slot].configure(inherited_policy);
        if parent_slot != NO_DESCRIPTOR {
            self.entries[parent_slot].first_child = slot
        }
        audit_event!(
            Level::Info,
            EventField::unsigned(field::OPERATION, 1),
            EventField::unsigned(field::CAPABILITY, handle.raw()),
            EventField::unsigned(field::OWNER, owner.raw() as u64),
            EventField::unsigned(field::RIGHTS, rights.bits() as u64),
        );
        emit_capability_trace(
            Level::Info,
            CapabilityDomain::Kernel,
            CapabilityTraceStage::Created,
            handle.raw(),
            1,
        );
        Ok(handle)
    }

    fn authorize_handle(
        &self,
        caller: AddressSpaceId,
        handle: CapabilityHandle,
        required: Rights,
    ) -> Result<CapabilityInfo, CapabilityError> {
        let slot = match self.valid_slot(handle) {
            Ok(slot) => slot,
            Err(error) => {
                audit_event!(
                    Level::Warn,
                    EventField::unsigned(field::OPERATION, 3),
                    EventField::unsigned(field::CAPABILITY, handle.raw()),
                    EventField::unsigned(field::CALLER, caller.raw() as u64),
                    EventField::status(error.status()),
                );
                return Err(error)
            }
        };
        let info = self.entries[slot].info;
        if info.owner != caller || !info.rights.contains(required) {
            audit_event!(
                Level::Warn,
                EventField::unsigned(field::OPERATION, 3),
                EventField::unsigned(field::CAPABILITY, handle.raw()),
                EventField::unsigned(field::CALLER, caller.raw() as u64),
                EventField::status(Status::ACCESS_DENIED),
            );
            return Err(CapabilityError::AccessDenied);
        }
        audit_event!(
            Level::Trace,
            EventField::unsigned(field::OPERATION, 3),
            EventField::unsigned(field::CAPABILITY, handle.raw()),
            EventField::unsigned(field::CALLER, caller.raw() as u64),
            EventField::status(Status::NORMAL),
        );
        emit_capability_trace(
            Level::Trace,
            CapabilityDomain::Kernel,
            CapabilityTraceStage::KernelIpc,
            handle.raw(),
            3,
        );
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

    fn handle_for_slot(&self, slot: usize) -> Option<CapabilityHandle> {
        self.entries
            .get(slot)
            .filter(|entry| entry.occupied)
            .map(|entry| CapabilityHandle::from_parts(slot, entry.generation))
    }

    fn vacate(&mut self, slot: usize) {
        let parent = self.entries[slot].parent_slot;
        let sibling = self.entries[slot].next_sibling;
        if let Some(parent_entry) = self.entries.get(parent) {
            let mut child = parent_entry.first_child;
            if child == slot {
                self.entries[parent].first_child = sibling
            } else {
                for _ in 0..CAPACITY {
                    if child == NO_DESCRIPTOR {
                        break;
                    }
                    let next = self.entries[child].next_sibling;
                    if next == slot {
                        self.entries[child].next_sibling = sibling;
                        break;
                    }
                    child = next;
                }
            }
        }
        self.entries[slot].occupied = false;
        self.quotas[slot].configure(QuotaPolicy::default());
        self.entries[slot].parent_slot = NO_DESCRIPTOR;
        self.entries[slot].first_child = NO_DESCRIPTOR;
        self.entries[slot].next_sibling = NO_DESCRIPTOR;
    }

    fn revoke_descendants_unchecked(&mut self, authority: CapabilityHandle) -> usize {
        let mut descendants = [false; CAPACITY];
        for (slot, descendant) in descendants.iter_mut().enumerate() {
            *descendant = self.entries[slot].occupied && self.is_descendant(slot, authority)
        }

        let mut revoked = 0;
        for (slot, descendant) in descendants.iter().enumerate() {
            if *descendant {
                self.vacate(slot);
                revoked += 1
            }
        }
        revoked
    }

    fn revoke_descendants(
        &mut self,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
    ) -> Result<usize, CapabilityError> {
        let mut descendants = [false; CAPACITY];
        for (slot, descendant) in descendants.iter_mut().enumerate() {
            *descendant = self.entries[slot].occupied && self.is_descendant(slot, authority)
        }

        let mut revoked = 0;
        for (slot, descendant) in descendants.iter().enumerate() {
            if *descendant {
                self.vacate(slot);
                revoked += 1
            }
        }
        audit_event!(
            Level::Info,
            EventField::unsigned(field::OPERATION, 2),
            EventField::unsigned(field::CAPABILITY, authority.raw()),
            EventField::unsigned(field::CALLER, caller.raw() as u64),
            EventField::unsigned(field::LENGTH, revoked as u64),
        );
        self.trace_revocation(authority, 2);
        Ok(revoked)
    }

    fn trace_revocation(&self, handle: CapabilityHandle, operation: u16) {
        emit_capability_trace(
            Level::Info,
            CapabilityDomain::Kernel,
            CapabilityTraceStage::Revoked,
            handle.raw(),
            operation,
        );
    }
}

impl<const CAPACITY: usize> Default for CapabilitySpace<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
