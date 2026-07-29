use synos_fabric::{
    Access, AddressRange, NodeId, PAGE_SIZE,
    dsm::{CoherenceDirectory, DsmPageState},
    memory::{
        GlobalAddressSpace, LeaseOwner, LeaseTable, MemoryKind, Transport,
    },
};

use crate::InspectError;

pub const MAX_MEMORY_NODES: usize = 16;
pub const MAX_CXL_LEASES: usize = 32;
pub const MAX_DSM_ALLOCATIONS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryNodeSample {
    pub node: NodeId,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub wired_bytes: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CxlLeaseSample {
    pub host_node: NodeId,
    pub owner_node: NodeId,
    pub range: AddressRange,
    pub writable: bool,
    pub expires_at_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DsmPageSample {
    pub host_node: NodeId,
    pub owner_node: NodeId,
    pub range: AddressRange,
    pub resident_pages: u32,
    pub shared_pages: u32,
    pub fault_count: u64,
}

#[derive(Clone, Copy)]
pub struct MemoryReport {
    sampled_at_us: u64,
    nodes: [Option<MemoryNodeSample>; MAX_MEMORY_NODES],
    cxl_leases: [Option<CxlLeaseSample>; MAX_CXL_LEASES],
    dsm_allocations: [Option<DsmPageSample>; MAX_DSM_ALLOCATIONS],
}

impl MemoryReport {
    pub const fn new() -> Self {
        Self {
            sampled_at_us: 0,
            nodes: [None; MAX_MEMORY_NODES],
            cxl_leases: [None; MAX_CXL_LEASES],
            dsm_allocations: [None; MAX_DSM_ALLOCATIONS],
        }
    }

    pub const fn sampled_at_us(&self) -> u64 {
        self.sampled_at_us
    }

    pub fn set_sampled_at_us(&mut self, sampled_at_us: u64) {
        self.sampled_at_us = sampled_at_us
    }

    pub fn nodes(&self) -> impl Iterator<Item = MemoryNodeSample> + '_ {
        self.nodes.iter().flatten().copied()
    }

    pub fn cxl_leases(&self) -> impl Iterator<Item = CxlLeaseSample> + '_ {
        self.cxl_leases.iter().flatten().copied()
    }

    pub fn dsm_allocations(&self) -> impl Iterator<Item = DsmPageSample> + '_ {
        self.dsm_allocations.iter().flatten().copied()
    }

    pub fn push_node(&mut self, sample: MemoryNodeSample) -> Result<(), InspectError> {
        if sample.total_bytes == 0
            || sample.free_bytes > sample.total_bytes
            || sample.wired_bytes > sample.total_bytes - sample.free_bytes
            || self.nodes().any(|entry| entry.node == sample.node)
        {
            return Err(InspectError::InvalidSample)
        }
        insert(&mut self.nodes, sample)
    }

    pub fn push_cxl_lease(&mut self, sample: CxlLeaseSample) -> Result<(), InspectError> {
        if sample.range.length == 0 || sample.expires_at_us == 0 {
            return Err(InspectError::InvalidSample)
        }
        insert(&mut self.cxl_leases, sample)
    }

    pub fn push_dsm_allocation(&mut self, sample: DsmPageSample) -> Result<(), InspectError> {
        if sample.range.length == 0
            || sample.resident_pages == 0
            || sample.shared_pages > sample.resident_pages
        {
            return Err(InspectError::InvalidSample)
        }
        insert(&mut self.dsm_allocations, sample)
    }

    /// Import live fabric pools, generation-checked CXL leases, and software
    /// DSM directory pages into one coherent diagnostic report.
    pub fn append_fabric<
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
        const PAGES: usize,
    >(
        &mut self,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &LeaseTable<LEASES>,
        directory: &CoherenceDirectory<PAGES>,
        directory_node: NodeId,
        now_us: u64,
    ) -> Result<(), InspectError> {
        for pool in space.pools().filter(|pool| pool.kind == MemoryKind::Ram) {
            let leased = leases
                .leases()
                .filter(|lease| lease.pool == pool.id && lease.expires_at_us > now_us)
                .fold(0u64, |bytes, lease| bytes.saturating_add(lease.range.length));
            if let Some(entry) = self
                .nodes
                .iter_mut()
                .flatten()
                .find(|entry| entry.node == pool.node)
            {
                entry.total_bytes = entry.total_bytes.saturating_add(pool.global.length);
                entry.free_bytes = entry
                    .free_bytes
                    .saturating_add(pool.global.length.saturating_sub(leased))
            } else {
                self.push_node(MemoryNodeSample {
                    node: pool.node,
                    total_bytes: pool.global.length,
                    free_bytes: pool.global.length.saturating_sub(leased),
                    wired_bytes: 0,
                })?
            }
        }

        for lease in leases.leases().filter(|lease| lease.expires_at_us > now_us) {
            let Some(pool) = space.pool(lease.pool) else {
                continue
            };
            if pool.transport != Transport::Cxl {
                continue
            }
            let owner_node = match lease.owner {
                LeaseOwner::Node(node) => node,
                LeaseOwner::Service(_) => pool.node,
            };
            self.push_cxl_lease(CxlLeaseSample {
                host_node: pool.node,
                owner_node,
                range: lease.range,
                writable: lease.rights.permits(Access::Write),
                expires_at_us: lease.expires_at_us,
            })?
        }

        for page in directory
            .pages()
            .filter(|page| page.state != DsmPageState::Invalid)
        {
            let owner_node = page
                .owner
                .or_else(|| {
                    NodeId::new(page.sharers.trailing_zeros() + 1)
                        .filter(|_| page.sharers != 0)
                })
                .unwrap_or(directory_node);
            self.push_dsm_allocation(DsmPageSample {
                host_node: directory_node,
                owner_node,
                range: AddressRange::new(page.page_address, PAGE_SIZE)
                    .map_err(|_| InspectError::InvalidSample)?,
                resident_pages: 1,
                shared_pages: u32::from(page.state == DsmPageState::Shared),
                fault_count: 0,
            })?
        }
        Ok(())
    }

    pub fn clear(&mut self) {
        *self = Self::new()
    }

    pub(crate) fn retain_node(&mut self, node: NodeId) {
        for entry in &mut self.nodes {
            if entry.is_some_and(|sample| sample.node != node) {
                *entry = None
            }
        }
        for entry in &mut self.cxl_leases {
            if entry.is_some_and(|sample| sample.host_node != node) {
                *entry = None
            }
        }
        for entry in &mut self.dsm_allocations {
            if entry.is_some_and(|sample| sample.host_node != node) {
                *entry = None
            }
        }
    }
}

impl Default for MemoryReport {
    fn default() -> Self {
        Self::new()
    }
}

fn insert<T: Copy, const CAPACITY: usize>(
    entries: &mut [Option<T>; CAPACITY],
    sample: T,
) -> Result<(), InspectError> {
    let slot = entries
        .iter_mut()
        .find(|entry| entry.is_none())
        .ok_or(InspectError::Capacity)?;
    *slot = Some(sample);
    Ok(())
}
