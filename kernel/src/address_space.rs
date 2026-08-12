//! Kernel-owned Ring 3 address-space records.

use synos_app::{Mapping, MappingRequest, ProcessContext, RuntimeSegment, SegmentPermissions};

use crate::capability::{CapabilityHandle, PhysicalRange};
use crate::task::AddressSpaceId;

pub const PAGE_SIZE: u64 = 4096;
/// Low supervisor identity map used during early kernel execution.
pub const KERNEL_SPACE_START: u64 = 0;
pub const KERNEL_SPACE_END: u64 = 4 * 1024 * 1024 * 1024;
/// User mappings begin in their own x86_64 PML4 slot. The low slot stays
/// supervisor-only, so user code cannot reach the kernel identity map.
pub const USER_SPACE_START: u64 = 0x0000_0080_0000_0000;
pub const USER_SPACE_END: u64 = 0x0000_7fff_ffff_f000;
pub const MAX_ADDRESS_SPACE_REGIONS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VirtualAddressLayout {
    pub kernel_start: u64,
    pub kernel_end: u64,
    pub user_start: u64,
    pub user_end: u64,
}

impl VirtualAddressLayout {
    pub const fn contains_user(self, address: u64, size: u64) -> bool {
        address >= self.user_start
            && match address.checked_add(size) {
                Some(end) => end <= self.user_end,
                None => false,
            }
    }

    pub const fn contains_kernel(self, address: u64, size: u64) -> bool {
        address >= self.kernel_start
            && match address.checked_add(size) {
                Some(end) => end <= self.kernel_end,
                None => false,
            }
    }
}

pub const VIRTUAL_ADDRESS_LAYOUT: VirtualAddressLayout = VirtualAddressLayout {
    kernel_start: KERNEL_SPACE_START,
    kernel_end: KERNEL_SPACE_END,
    user_start: USER_SPACE_START,
    user_end: USER_SPACE_END,
};

pub const fn is_user_range(address: u64, size: u64) -> bool {
    VIRTUAL_ADDRESS_LAYOUT.contains_user(address, size)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageTableRoot {
    frame: u64,
}

impl PageTableRoot {
    pub const fn new(frame: u64) -> Option<Self> {
        if frame == 0 || frame % PAGE_SIZE != 0 {
            None
        } else {
            Some(Self { frame })
        }
    }

    pub const fn frame(self) -> u64 {
        self.frame
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressSpaceError {
    Capacity,
    InvalidRoot,
    InvalidRequest,
    InvalidMapping,
    MappingOverflow,
    MappingConflict,
    NotFound,
    InvalidContext,
    NotCopyOnWrite,
}

#[derive(Clone, Copy)]
struct Region {
    owner: Mapping,
    base: u64,
    size: u64,
    permissions: Option<SegmentPermissions>,
    backing: Option<PhysicalRange>,
    authority: u64,
    copy_on_write: bool,
}

impl Region {
    const fn end(self) -> Option<u64> {
        self.base.checked_add(self.size)
    }

    const fn overlaps(self, other_base: u64, other_size: u64) -> bool {
        let Some(end) = self.end() else {
            return true;
        };
        let Some(other_end) = other_base.checked_add(other_size) else {
            return true;
        };
        self.base < other_end && other_base < end
    }

    const fn contains(self, address: u64, size: u64) -> bool {
        let Some(end) = self.end() else {
            return false;
        };
        let Some(requested_end) = address.checked_add(size) else {
            return false;
        };
        address >= self.base && requested_end <= end
    }
}

#[derive(Clone, Copy)]
pub struct AddressSpace {
    id: AddressSpaceId,
    root: PageTableRoot,
    regions: [Option<Region>; MAX_ADDRESS_SPACE_REGIONS],
}

impl AddressSpace {
    pub fn new(id: AddressSpaceId, root: PageTableRoot) -> Result<Self, AddressSpaceError> {
        if id == AddressSpaceId::KERNEL {
            return Err(AddressSpaceError::InvalidRequest)
        }
        Ok(Self {
            id,
            root,
            regions: [None; MAX_ADDRESS_SPACE_REGIONS],
        })
    }

    pub const fn id(self) -> AddressSpaceId {
        self.id
    }

    pub const fn root(self) -> PageTableRoot {
        self.root
    }

    pub fn reserve(&mut self, request: MappingRequest) -> Result<Mapping, AddressSpaceError> {
        if request.size == 0
            || request.alignment < PAGE_SIZE
            || !request.alignment.is_power_of_two()
        {
            return Err(AddressSpaceError::InvalidRequest)
        }
        let base = if request.fixed {
            let base = request.preferred_base.ok_or(AddressSpaceError::InvalidRequest)?;
            if base % request.alignment != 0 {
                return Err(AddressSpaceError::InvalidRequest)
            }
            base
        } else {
            let mut candidate = request
                .preferred_base
                .unwrap_or(USER_SPACE_START)
                .max(USER_SPACE_START);
            let mut selected = None;
            for _ in 0..MAX_ADDRESS_SPACE_REGIONS {
                candidate = align_up(candidate, request.alignment)?;
                if is_user_range(candidate, request.size)
                    && !self.overlaps(candidate, request.size)
                {
                    selected = Some(candidate);
                    break;
                }
                candidate = self
                    .next_after(candidate, request.size)
                    .ok_or(AddressSpaceError::MappingOverflow)?;
            }
            selected.ok_or(AddressSpaceError::Capacity)?
        };
        if !is_user_range(base, request.size) || self.overlaps(base, request.size)
        {
            return Err(if request.fixed {
                AddressSpaceError::MappingConflict
            } else {
                AddressSpaceError::MappingOverflow
            })
        }
        let mapping = Mapping {
            base,
            size: request.size,
        };
        self.claim(mapping)?;
        Ok(mapping)
    }

    pub fn claim(&mut self, mapping: Mapping) -> Result<(), AddressSpaceError> {
        if mapping.size == 0
            || !is_user_range(mapping.base, mapping.size)
            || self.overlaps(mapping.base, mapping.size)
        {
            return Err(AddressSpaceError::MappingConflict)
        }
        self.insert(Region {
            owner: mapping,
            base: mapping.base,
            size: mapping.size,
            permissions: None,
            backing: None,
            authority: 0,
            copy_on_write: false,
        })?;
        Ok(())
    }

    pub fn map_backing(
        &mut self,
        authority: CapabilityHandle,
        backing: PhysicalRange,
        writable: bool,
    ) -> Result<Mapping, AddressSpaceError> {
        if backing.start % PAGE_SIZE != 0
            || backing.length == 0
            || backing.length % PAGE_SIZE != 0
        {
            return Err(AddressSpaceError::InvalidMapping)
        }
        let mapping = self.reserve(MappingRequest {
            size: backing.length,
            alignment: PAGE_SIZE,
            preferred_base: None,
            fixed: false,
        })?;
        let Some(region) = self
            .regions
            .iter_mut()
            .flatten()
            .find(|region| region.owner == mapping && region.permissions.is_none())
        else {
            self.release(mapping);
            return Err(AddressSpaceError::InvalidMapping)
        };
        region.permissions = Some(if writable {
            SegmentPermissions::READ.union(SegmentPermissions::WRITE)
        } else {
            SegmentPermissions::READ
        });
        region.backing = Some(backing);
        region.authority = authority.raw();
        region.copy_on_write = false;
        Ok(mapping)
    }

    /// Make a child address-space record sharing this space's mappings.
    /// Writable regions become read-only COW regions in both records. The
    /// caller must retain each physical backing in its [`CowManager`].
    pub fn clone_cow(
        &mut self,
        child_id: AddressSpaceId,
        child_root: PageTableRoot,
    ) -> Result<Self, AddressSpaceError> {
        if child_id == AddressSpaceId::KERNEL || child_id == self.id {
            return Err(AddressSpaceError::InvalidRequest)
        }
        let mut child = *self;
        child.id = child_id;
        child.root = child_root;
        for region in self.regions.iter_mut().flatten() {
            if let Some(permissions) = region.permissions
                && permissions.writable()
                && region.backing.is_some()
            {
                region.permissions = Some(permissions.without(SegmentPermissions::WRITE));
                region.copy_on_write = true;
            }
        }
        child.regions = self.regions;
        Ok(child)
    }

    pub fn cow_mapping(&self, address: u64) -> Result<(u64, u64), AddressSpaceError> {
        let page = address & !(PAGE_SIZE - 1);
        let region = self
            .regions
            .iter()
            .flatten()
            .find(|region| region.copy_on_write && region.contains(page, PAGE_SIZE))
            .ok_or(AddressSpaceError::NotCopyOnWrite)?;
        let backing = region.backing.ok_or(AddressSpaceError::NotCopyOnWrite)?;
        let offset = page
            .checked_sub(region.base)
            .ok_or(AddressSpaceError::InvalidMapping)?;
        let frame = backing
            .start
            .checked_add(offset)
            .ok_or(AddressSpaceError::InvalidMapping)?;
        Ok((page, frame))
    }

    pub fn can_replace_cow_page(&self, address: u64) -> Result<(), AddressSpaceError> {
        let page = address & !(PAGE_SIZE - 1);
        let region = self
            .regions
            .iter()
            .flatten()
            .find(|region| {
                region.copy_on_write
                    && region.backing.is_some()
                    && region.contains(page, PAGE_SIZE)
            })
            .ok_or(AddressSpaceError::NotCopyOnWrite)?;
        let end = region.end().ok_or(AddressSpaceError::MappingOverflow)?;
        let right_base = page
            .checked_add(PAGE_SIZE)
            .ok_or(AddressSpaceError::MappingOverflow)?;
        let extra = usize::from(page != region.base)
            + usize::from(end.saturating_sub(right_base) != 0);
        if self.regions.iter().filter(|slot| slot.is_none()).count() < extra {
            Err(AddressSpaceError::Capacity)
        } else {
            Ok(())
        }
    }

    /// Replace one COW page with a private writable frame.
    pub fn replace_cow_page(
        &mut self,
        address: u64,
        new_frame: u64,
    ) -> Result<PhysicalRange, AddressSpaceError> {
        if new_frame % PAGE_SIZE != 0 {
            return Err(AddressSpaceError::InvalidMapping)
        }
        let page = address & !(PAGE_SIZE - 1);
        let index = self
            .regions
            .iter()
            .position(|region| {
                region.is_some_and(|region| {
                    region.copy_on_write
                        && region.backing.is_some()
                        && region.contains(page, PAGE_SIZE)
                })
            })
            .ok_or(AddressSpaceError::NotCopyOnWrite)?;
        let original = self.regions[index].ok_or(AddressSpaceError::NotCopyOnWrite)?;
        let backing = original.backing.ok_or(AddressSpaceError::NotCopyOnWrite)?;
        let offset = page
            .checked_sub(original.base)
            .ok_or(AddressSpaceError::InvalidMapping)?;
        let old_frame = backing
            .start
            .checked_add(offset)
            .ok_or(AddressSpaceError::InvalidMapping)?;
        let end = original.end().ok_or(AddressSpaceError::MappingOverflow)?;
        let right_base = page
            .checked_add(PAGE_SIZE)
            .ok_or(AddressSpaceError::MappingOverflow)?;
        let left_size = page - original.base;
        let right_size = end.saturating_sub(right_base);
        let extra = usize::from(left_size != 0) + usize::from(right_size != 0);
        let free = self.regions.iter().filter(|region| region.is_none()).count();
        if free < extra {
            return Err(AddressSpaceError::Capacity)
        }
        let private = Region {
            owner: original.owner,
            base: page,
            size: PAGE_SIZE,
            permissions: original
                .permissions
                .map(|permissions| permissions.union(SegmentPermissions::WRITE)),
            backing: Some(PhysicalRange::new(new_frame, PAGE_SIZE).ok_or(
                AddressSpaceError::InvalidMapping,
            )?),
            authority: original.authority,
            copy_on_write: false,
        };
        let left = (left_size != 0).then_some(Region {
            owner: original.owner,
            base: original.base,
            size: left_size,
            permissions: original.permissions,
            backing: Some(PhysicalRange::new(backing.start, left_size).ok_or(
                AddressSpaceError::InvalidMapping,
            )?),
            authority: original.authority,
            copy_on_write: true,
        });
        let right = (right_size != 0).then_some(Region {
            owner: original.owner,
            base: right_base,
            size: right_size,
            permissions: original.permissions,
            backing: Some(PhysicalRange::new(
                backing.start + offset + PAGE_SIZE,
                right_size,
            )
            .ok_or(AddressSpaceError::InvalidMapping)?),
            authority: original.authority,
            copy_on_write: true,
        });
        self.regions[index] = left.or(Some(private));
        if left.is_some() {
            let slot = self
                .regions
                .iter()
                .position(Option::is_none)
                .ok_or(AddressSpaceError::Capacity)?;
            self.regions[slot] = Some(private);
        }
        if let Some(right) = right {
            let slot = self
                .regions
                .iter()
                .position(Option::is_none)
                .ok_or(AddressSpaceError::Capacity)?;
            self.regions[slot] = Some(right);
        }
        Ok(PhysicalRange::new(old_frame, PAGE_SIZE).ok_or(AddressSpaceError::InvalidMapping)?)
    }

    pub fn unmap_backing(
        &mut self,
        authority: CapabilityHandle,
        address: u64,
        length: u64,
    ) -> Result<PhysicalRange, AddressSpaceError> {
        if address % PAGE_SIZE != 0 || length == 0 || length % PAGE_SIZE != 0 {
            return Err(AddressSpaceError::InvalidMapping)
        }
        let index = self
            .regions
            .iter()
            .position(|region| {
                region.is_some_and(|region| {
                    region.base == address
                        && region.size == length
                        && region.authority == authority.raw()
                        && region.backing.is_some()
                })
            })
            .ok_or(AddressSpaceError::NotFound)?;
        self.regions[index]
            .take()
            .and_then(|region| region.backing)
            .ok_or(AddressSpaceError::InvalidMapping)
    }

    pub fn map_segment(
        &mut self,
        mapping: Mapping,
        segment: RuntimeSegment,
        source_length: usize,
    ) -> Result<(), AddressSpaceError> {
        if segment.memory_size == 0
            || segment.file_size > segment.memory_size
            || segment.file_size as usize != source_length
            || segment.permissions.writable() && segment.permissions.executable()
            || !self.owns(mapping, segment.address, segment.memory_size)
        {
            return Err(AddressSpaceError::InvalidMapping)
        }
        self.insert(Region {
            owner: mapping,
            base: segment.address,
            size: segment.memory_size,
            permissions: Some(segment.permissions),
            backing: None,
            authority: 0,
            copy_on_write: false,
        })
    }

    pub fn zero_fill(
        &self,
        mapping: Mapping,
        address: u64,
        length: u64,
    ) -> Result<(), AddressSpaceError> {
        if length == 0 || !self.owns(mapping, address, length) {
            return Err(AddressSpaceError::InvalidMapping)
        }
        Ok(())
    }

    pub fn relocate(
        &self,
        mapping: Mapping,
        address: u64,
    ) -> Result<(), AddressSpaceError> {
        if !self.owns(mapping, address, core::mem::size_of::<u64>() as u64) {
            return Err(AddressSpaceError::InvalidMapping)
        }
        Ok(())
    }

    pub fn protect(
        &mut self,
        mapping: Mapping,
        address: u64,
        length: u64,
        permissions: SegmentPermissions,
    ) -> Result<(), AddressSpaceError> {
        if length == 0
            || permissions.writable() && permissions.executable()
            || !self.owns(mapping, address, length)
        {
            return Err(AddressSpaceError::InvalidMapping)
        }
        self.insert(Region {
            owner: mapping,
            base: address,
            size: length,
            permissions: Some(permissions),
            backing: None,
            authority: 0,
            copy_on_write: false,
        })
    }

    pub fn record_region(
        &mut self,
        mapping: Mapping,
        base: u64,
        size: u64,
        permissions: SegmentPermissions,
    ) -> Result<(), AddressSpaceError> {
        if size == 0
            || permissions.writable() && permissions.executable()
            || !is_user_range(base, size)
        {
            return Err(AddressSpaceError::InvalidMapping)
        }
        self.insert(Region {
            owner: mapping,
            base,
            size,
            permissions: Some(permissions),
            backing: None,
            authority: 0,
            copy_on_write: false,
        })
    }

    pub fn install_context(&self, context: ProcessContext) -> Result<(), AddressSpaceError> {
        if !self.contains(context.entry, 1)
            || !self.contains(context.stack_pointer.saturating_sub(1), 1)
            || !self.contains(context.heap_base, context.heap_size)
            || context.tls_pointer.is_some_and(|tls| !self.contains(tls, 1))
        {
            return Err(AddressSpaceError::InvalidContext)
        }
        Ok(())
    }

    pub fn release(&mut self, mapping: Mapping) {
        for region in &mut self.regions {
            if region.is_some_and(|region| region.owner == mapping) {
                *region = None
            }
        }
    }

    fn contains(&self, address: u64, size: u64) -> bool {
        self.regions
            .iter()
            .flatten()
            .any(|region| region.permissions.is_some() && region.contains(address, size))
    }

    fn owns(&self, mapping: Mapping, address: u64, size: u64) -> bool {
        self.regions
            .iter()
            .flatten()
            .any(|region| region.owner == mapping && region.contains(address, size))
    }

    fn overlaps(&self, base: u64, size: u64) -> bool {
        self.regions
            .iter()
            .flatten()
            .any(|region| region.overlaps(base, size))
    }

    fn next_after(&self, base: u64, size: u64) -> Option<u64> {
        self.regions
            .iter()
            .flatten()
            .filter(|region| region.overlaps(base, size))
            .filter_map(|region| region.end())
            .max()
    }

    fn insert(&mut self, region: Region) -> Result<(), AddressSpaceError> {
        let slot = self
            .regions
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(AddressSpaceError::Capacity)?;
        *slot = Some(region);
        Ok(())
    }
}

pub struct AddressSpaceTable<const CAPACITY: usize> {
    spaces: [Option<AddressSpace>; CAPACITY],
}

impl<const CAPACITY: usize> AddressSpaceTable<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            spaces: [None; CAPACITY],
        }
    }

    pub fn create(
        &mut self,
        id: AddressSpaceId,
        root: PageTableRoot,
    ) -> Result<(), AddressSpaceError> {
        if self
            .spaces
            .iter()
            .flatten()
            .any(|space| space.id() == id || space.root() == root)
        {
            return Err(AddressSpaceError::MappingConflict)
        }
        let slot = self
            .spaces
            .iter_mut()
            .find(|space| space.is_none())
            .ok_or(AddressSpaceError::Capacity)?;
        *slot = Some(AddressSpace::new(id, root)?);
        Ok(())
    }

    pub fn get(&self, id: AddressSpaceId) -> Result<&AddressSpace, AddressSpaceError> {
        self.spaces
            .iter()
            .flatten()
            .find(|space| space.id() == id)
            .ok_or(AddressSpaceError::NotFound)
    }

    pub fn get_mut(&mut self, id: AddressSpaceId) -> Result<&mut AddressSpace, AddressSpaceError> {
        self.spaces
            .iter_mut()
            .flatten()
            .find(|space| space.id() == id)
            .ok_or(AddressSpaceError::NotFound)
    }

    pub fn destroy(&mut self, id: AddressSpaceId) -> Result<(), AddressSpaceError> {
        let space = self
            .spaces
            .iter_mut()
            .find(|space| space.is_some_and(|space| space.id() == id))
            .ok_or(AddressSpaceError::NotFound)?;
        *space = None;
        Ok(())
    }
}

impl<const CAPACITY: usize> Default for AddressSpaceTable<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn align_up(value: u64, alignment: u64) -> Result<u64, AddressSpaceError> {
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
        .ok_or(AddressSpaceError::MappingOverflow)
}
