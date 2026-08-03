use synos_fabric::{Access, AddressRange, NodeId, memory::LeaseTable};
use synos_llm::{Error as AllocationError, allocator::{AllocationInfo, AllocationPolicy, ModelAddress, UnifiedAllocator}};

/// Memory placement policy for a legacy container.
///
/// The container receives one virtual range. The allocator backs its pages
/// with local RAM, CXL, or software DSM extents without copying the workload
/// image between tiers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContainerMemoryPolicy {
    pub compute_node: NodeId,
    pub service_id: u64,
    pub allow_local: bool,
    pub allow_cxl: bool,
    pub allow_software_dsm: bool,
    pub require_mirror: bool,
}

impl ContainerMemoryPolicy {
    pub const fn new(compute_node: NodeId, service_id: u64) -> Option<Self> {
        if service_id == 0 {
            return None;
        }
        Some(Self {
            compute_node,
            service_id,
            allow_local: true,
            allow_cxl: true,
            allow_software_dsm: false,
            require_mirror: false,
        })
    }

    pub const fn with_software_dsm(mut self, enabled: bool) -> Self {
        self.allow_software_dsm = enabled;
        self
    }

    pub const fn with_local(mut self, enabled: bool) -> Self {
        self.allow_local = enabled;
        self
    }

    pub const fn with_cxl(mut self, enabled: bool) -> Self {
        self.allow_cxl = enabled;
        self
    }

    pub const fn with_mirror(mut self, required: bool) -> Self {
        self.require_mirror = required;
        self
    }

    fn allocation_policy(self) -> Result<AllocationPolicy, ZeroCopyMemoryError> {
        if !self.allow_local && !self.allow_cxl && !self.allow_software_dsm {
            return Err(ZeroCopyMemoryError::NoTransport);
        }
        Ok(AllocationPolicy {
            compute_node: self.compute_node,
            lease_owner: synos_fabric::memory::LeaseOwner::Service(self.service_id),
            memory_kind: synos_fabric::memory::MemoryKind::Ram,
            allow_local: self.allow_local,
            allow_cxl: self.allow_cxl,
            allow_layer2: self.allow_software_dsm,
            require_mirror: self.require_mirror,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ZeroCopyMemoryError {
    NoTransport,
    Allocation(AllocationError),
}

impl From<AllocationError> for ZeroCopyMemoryError {
    fn from(error: AllocationError) -> Self {
        Self::Allocation(error)
    }
}

/// Fixed-capacity virtual-memory broker for container address spaces.
///
/// `UnifiedAllocator` preserves the virtual address presented to the guest
/// while `GlobalAddressSpace` resolves each page to its current local, CXL, or
/// software-DSM backing. The broker owns leases, not byte buffers.
pub struct ZeroCopyContainerMemory<
    const ALLOCATIONS: usize = { synos_llm::allocator::DEFAULT_ALLOCATION_CAPACITY },
    const EXTENTS: usize = { synos_llm::allocator::DEFAULT_EXTENTS_PER_ALLOCATION },
> {
    allocator: UnifiedAllocator<ALLOCATIONS, EXTENTS>,
}

impl<const ALLOCATIONS: usize, const EXTENTS: usize>
    ZeroCopyContainerMemory<ALLOCATIONS, EXTENTS>
{
    pub const fn new(virtual_window: AddressRange) -> Self {
        Self {
            allocator: UnifiedAllocator::new(virtual_window),
        }
    }

    pub fn allocate<const POOLS: usize, const OVERRIDES: usize, const LEASES: usize>(
        &mut self,
        space: &synos_fabric::memory::GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &mut LeaseTable<LEASES>,
        bytes: u64,
        alignment: u64,
        policy: ContainerMemoryPolicy,
        now_us: u64,
        lease_duration_us: u64,
    ) -> Result<AllocationInfo, ZeroCopyMemoryError> {
        self.allocator
            .allocate(
                space,
                leases,
                bytes,
                alignment,
                policy.allocation_policy()?,
                now_us,
                lease_duration_us,
            )
            .map_err(Into::into)
    }

    pub fn resolve<const POOLS: usize, const OVERRIDES: usize, const LEASES: usize>(
        &self,
        allocation: synos_llm::allocator::AllocationHandle,
        offset: u64,
        access: Access,
        now_us: u64,
        space: &synos_fabric::memory::GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &LeaseTable<LEASES>,
    ) -> Result<ModelAddress, ZeroCopyMemoryError> {
        self.allocator
            .resolve(allocation, offset, access, now_us, space, leases)
            .map_err(Into::into)
    }

    pub fn renew<const LEASES: usize>(
        &self,
        allocation: synos_llm::allocator::AllocationHandle,
        leases: &mut LeaseTable<LEASES>,
        now_us: u64,
        lease_duration_us: u64,
    ) -> Result<(), ZeroCopyMemoryError> {
        self.allocator
            .renew(allocation, leases, now_us, lease_duration_us)
            .map_err(Into::into)
    }

    pub fn release<const LEASES: usize>(
        &mut self,
        allocation: synos_llm::allocator::AllocationHandle,
        leases: &mut LeaseTable<LEASES>,
    ) -> Result<(), ZeroCopyMemoryError> {
        self.allocator
            .release(allocation, leases)
            .map_err(Into::into)
    }

    pub const fn allocator(&self) -> &UnifiedAllocator<ALLOCATIONS, EXTENTS> {
        &self.allocator
    }
}
