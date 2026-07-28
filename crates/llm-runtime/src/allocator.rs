use synos_fabric::{
    Access, AddressRange, NodeId, PAGE_SIZE, PageFault,
    memory::{
        GlobalAddressSpace, LeaseHandle, LeaseRights, LeaseTable, MemoryKind,
        LeaseOwner, MemoryMapping, ResolvedAddress, Transport,
    },
};

use crate::Error;

pub const DEFAULT_ALLOCATION_CAPACITY: usize = 64;
pub const DEFAULT_EXTENTS_PER_ALLOCATION: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct AllocationHandle(u64);

impl AllocationHandle {
    const fn from_parts(slot: usize, generation: u32) -> Self {
        Self((generation as u64) << 32 | slot as u64)
    }

    pub const fn from_raw(raw: u64) -> Option<Self> {
        if raw >> 32 == 0 { None } else { Some(Self(raw)) }
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
pub struct AllocationPolicy {
    pub compute_node: NodeId,
    pub lease_owner: LeaseOwner,
    pub memory_kind: MemoryKind,
    pub allow_local: bool,
    pub allow_cxl: bool,
    pub allow_layer2: bool,
    pub require_mirror: bool,
}

impl AllocationPolicy {
    /// One process sees local and attached CXL memory as one allocation.
    pub const fn single_node(compute_node: NodeId, memory_kind: MemoryKind) -> Self {
        Self {
            compute_node,
            lease_owner: LeaseOwner::Node(compute_node),
            memory_kind,
            allow_local: true,
            allow_cxl: true,
            allow_layer2: false,
            require_mirror: false,
        }
    }

    /// Long-context allocations may spill into networked legacy nodes.
    pub const fn cluster(compute_node: NodeId, memory_kind: MemoryKind) -> Self {
        Self {
            compute_node,
            lease_owner: LeaseOwner::Node(compute_node),
            memory_kind,
            allow_local: true,
            allow_cxl: true,
            allow_layer2: true,
            require_mirror: false,
        }
    }

    pub const fn resilient_cluster(
        compute_node: NodeId,
        memory_kind: MemoryKind,
        service_id: u64,
    ) -> Self {
        Self {
            compute_node,
            lease_owner: LeaseOwner::Service(service_id),
            memory_kind,
            allow_local: true,
            allow_cxl: true,
            allow_layer2: true,
            require_mirror: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct AllocationExtent {
    lease: LeaseHandle,
    fabric: AddressRange,
    logical_offset: u64,
}

impl AllocationExtent {
    const EMPTY: Option<Self> = None;
}

#[derive(Clone, Copy)]
struct AllocationEntry<const EXTENTS: usize> {
    occupied: bool,
    generation: u32,
    owner: LeaseOwner,
    virtual_range: AddressRange,
    extents: [Option<AllocationExtent>; EXTENTS],
    extent_count: u16,
}

impl<const EXTENTS: usize> AllocationEntry<EXTENTS> {
    const EMPTY: Self = Self {
        occupied: false,
        generation: 0,
        owner: LeaseOwner::Node(NodeId::LOCAL),
        virtual_range: AddressRange {
            start: 0,
            length: 1,
        },
        extents: [AllocationExtent::EMPTY; EXTENTS],
        extent_count: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AllocationInfo {
    pub handle: AllocationHandle,
    pub virtual_range: AddressRange,
    pub extent_count: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelAddress {
    pub virtual_address: u64,
    pub fabric_address: u64,
    pub allocation_offset: u64,
    pub lease: LeaseHandle,
    pub resolved: ResolvedAddress,
    pub mapping: MemoryMapping,
}

/// Presents many physical fabric leases as one contiguous process allocation.
///
/// No tensor or pipeline topology reaches the caller. Page mapping code uses
/// `resolve` to back each virtual page from local RAM, CXL, or layer-2 memory.
pub struct UnifiedAllocator<
    const ALLOCATIONS: usize = DEFAULT_ALLOCATION_CAPACITY,
    const EXTENTS: usize = DEFAULT_EXTENTS_PER_ALLOCATION,
> {
    virtual_window: AddressRange,
    allocations: [AllocationEntry<EXTENTS>; ALLOCATIONS],
}

impl<const ALLOCATIONS: usize, const EXTENTS: usize> UnifiedAllocator<ALLOCATIONS, EXTENTS> {
    pub const fn new(virtual_window: AddressRange) -> Self {
        Self {
            virtual_window,
            allocations: [AllocationEntry::EMPTY; ALLOCATIONS],
        }
    }

    pub fn allocate<
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
    >(
        &mut self,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &mut LeaseTable<LEASES>,
        bytes: u64,
        alignment: u64,
        policy: AllocationPolicy,
        now_us: u64,
        lease_duration_us: u64,
    ) -> Result<AllocationInfo, Error> {
        let length = page_align(bytes)?;
        if alignment < PAGE_SIZE || !alignment.is_power_of_two() {
            return Err(Error::InvalidRange)
        }
        let virtual_range = self.find_virtual_range(length, alignment)?;
        let slot_index = self
            .allocations
            .iter()
            .position(|entry| !entry.occupied)
            .ok_or(Error::Capacity)?;

        let mut extents = [AllocationExtent::EMPTY; EXTENTS];
        let mut extent_count = 0usize;
        let mut allocated = 0u64;

        for transport in [Transport::Local, Transport::Cxl, Transport::Layer2] {
            if !policy_accepts_transport(policy, transport) {
                continue
            }
            for pool in space.pools().filter(|pool| {
                pool.kind == policy.memory_kind
                    && pool.transport == transport
                    && (transport != Transport::Local || pool.node == policy.compute_node)
                    && (!policy.require_mirror || pool.mirror.is_some())
                    && !space.is_node_failed(pool.node)
            }) {
                loop {
                    if allocated == length {
                        break
                    }
                    if extent_count == EXTENTS {
                        self.rollback(leases, policy.lease_owner, &extents);
                        return Err(Error::Capacity)
                    }
                    let free = match leases.largest_free_range(
                        space,
                        pool.id,
                        alignment,
                        now_us,
                    ) {
                        Ok(range) => range,
                        Err(synos_fabric::Error::Capacity) => break,
                        Err(error) => {
                            self.rollback(leases, policy.lease_owner, &extents);
                            return Err(error.into())
                        }
                    };
                    let chunk = core::cmp::min(length - allocated, free.length);
                    let (lease, fabric) = match leases.allocate(
                        space,
                        policy.lease_owner,
                        pool.id,
                        chunk,
                        alignment,
                        LeaseRights::READ_WRITE,
                        now_us,
                        lease_duration_us,
                    ) {
                        Ok(result) => result,
                        Err(error) => {
                            self.rollback(leases, policy.lease_owner, &extents);
                            return Err(error.into())
                        }
                    };
                    extents[extent_count] = Some(AllocationExtent {
                        lease,
                        fabric,
                        logical_offset: allocated,
                    });
                    extent_count += 1;
                    allocated += fabric.length;
                }
            }
            if allocated == length {
                break
            }
        }

        if allocated != length {
            self.rollback(leases, policy.lease_owner, &extents);
            return Err(Error::Capacity)
        }
        let generation = self.allocations[slot_index]
            .generation
            .wrapping_add(1)
            .max(1);
        let handle = AllocationHandle::from_parts(slot_index, generation);
        self.allocations[slot_index] = AllocationEntry {
            occupied: true,
            generation,
            owner: policy.lease_owner,
            virtual_range,
            extents,
            extent_count: extent_count as u16,
        };
        Ok(AllocationInfo {
            handle,
            virtual_range,
            extent_count: extent_count as u16,
        })
    }

    pub fn info(&self, handle: AllocationHandle) -> Result<AllocationInfo, Error> {
        let entry = self.entry(handle)?;
        Ok(AllocationInfo {
            handle,
            virtual_range: entry.virtual_range,
            extent_count: entry.extent_count,
        })
    }

    pub fn resolve<
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
    >(
        &self,
        handle: AllocationHandle,
        offset: u64,
        access: Access,
        now_us: u64,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &LeaseTable<LEASES>,
    ) -> Result<ModelAddress, Error> {
        let entry = self.entry(handle)?;
        if offset >= entry.virtual_range.length {
            return Err(Error::InvalidRange)
        }
        let extent = entry
            .extents
            .iter()
            .flatten()
            .find(|extent| {
                offset >= extent.logical_offset
                    && offset < extent.logical_offset + extent.fabric.length
            })
            .ok_or(Error::AllocationNotFound)?;
        let fabric_address = extent.fabric.start + offset - extent.logical_offset;
        leases.authorize(extent.lease, entry.owner, fabric_address, access, now_us)?;
        let mapping = space.resolve_mapping(fabric_address)?;
        Ok(ModelAddress {
            virtual_address: entry.virtual_range.start + offset,
            fabric_address,
            allocation_offset: offset,
            lease: extent.lease,
            resolved: mapping.source(),
            mapping,
        })
    }

    pub fn resolve_fault<
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
    >(
        &self,
        handle: AllocationHandle,
        fault: PageFault,
        now_us: u64,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &LeaseTable<LEASES>,
    ) -> Result<ModelAddress, Error> {
        let entry = self.entry(handle)?;
        if !entry.virtual_range.contains(fault.virtual_address) {
            return Err(Error::InvalidRange)
        }
        self.resolve(
            handle,
            fault.virtual_address - entry.virtual_range.start,
            fault.access,
            now_us,
            space,
            leases,
        )
    }

    pub fn renew<const LEASES: usize>(
        &self,
        handle: AllocationHandle,
        leases: &mut LeaseTable<LEASES>,
        now_us: u64,
        duration_us: u64,
    ) -> Result<(), Error> {
        let entry = self.entry(handle)?;
        for extent in entry.extents.iter().flatten() {
            leases.renew(extent.lease, entry.owner, now_us, duration_us)?
        }
        Ok(())
    }

    pub fn release<const LEASES: usize>(
        &mut self,
        handle: AllocationHandle,
        leases: &mut LeaseTable<LEASES>,
    ) -> Result<(), Error> {
        let slot = self.valid_slot(handle)?;
        let entry = self.allocations[slot];
        for extent in entry.extents.iter().flatten() {
            leases.release(extent.lease, entry.owner)?
        }
        self.allocations[slot].occupied = false;
        Ok(())
    }

    fn entry(&self, handle: AllocationHandle) -> Result<&AllocationEntry<EXTENTS>, Error> {
        Ok(&self.allocations[self.valid_slot(handle)?])
    }

    fn valid_slot(&self, handle: AllocationHandle) -> Result<usize, Error> {
        let slot = handle.slot();
        let entry = self
            .allocations
            .get(slot)
            .ok_or(Error::InvalidHandle)?;
        if !entry.occupied || entry.generation != handle.generation() {
            return Err(Error::AllocationNotFound)
        }
        Ok(slot)
    }

    fn find_virtual_range(&self, length: u64, alignment: u64) -> Result<AddressRange, Error> {
        let mut cursor = align_up(self.virtual_window.start, alignment)?;
        loop {
            let end = cursor.checked_add(length).ok_or(Error::InvalidRange)?;
            if end > self.virtual_window.end() {
                return Err(Error::Capacity)
            }
            let conflict = self
                .allocations
                .iter()
                .filter(|entry| entry.occupied)
                .find(|entry| {
                    entry.virtual_range.start < end
                        && cursor < entry.virtual_range.end()
                });
            if let Some(conflict) = conflict {
                cursor = align_up(conflict.virtual_range.end(), alignment)?;
            } else {
                return Ok(AddressRange {
                    start: cursor,
                    length,
                })
            }
        }
    }

    fn rollback<const LEASES: usize>(
        &self,
        leases: &mut LeaseTable<LEASES>,
        owner: LeaseOwner,
        extents: &[Option<AllocationExtent>; EXTENTS],
    ) {
        for extent in extents.iter().flatten() {
            let _ = leases.release(extent.lease, owner);
        }
    }
}

fn policy_accepts_transport(policy: AllocationPolicy, transport: Transport) -> bool {
    match transport {
        Transport::Local => policy.allow_local,
        Transport::Cxl => policy.allow_cxl,
        Transport::Layer2 => policy.allow_layer2,
    }
}

fn page_align(bytes: u64) -> Result<u64, Error> {
    if bytes == 0 {
        return Err(Error::InvalidRange)
    }
    align_up(bytes, PAGE_SIZE)
}

fn align_up(value: u64, alignment: u64) -> Result<u64, Error> {
    value
        .checked_add(alignment - 1)
        .map(|sum| sum & !(alignment - 1))
        .ok_or(Error::InvalidRange)
}
