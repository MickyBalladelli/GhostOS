use crate::{Access, AddressRange, Error, NodeId, PAGE_SIZE};
use synos_observability::{
    CorrelationId, EventField, EventKind, Level, TraceEvent, emit, field,
    next_correlation_id,
};

pub const DEFAULT_POOL_CAPACITY: usize = 64;
pub const DEFAULT_LEASE_CAPACITY: usize = 256;
pub const DEFAULT_PAGE_TRACKING_CAPACITY: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryKind {
    Ram,
    Vram,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Transport {
    Local,
    Cxl,
    Layer2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct PoolId(u32);

impl PoolId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryPool {
    pub id: PoolId,
    pub node: NodeId,
    pub mirror: Option<NodeId>,
    pub kind: MemoryKind,
    pub transport: Transport,
    /// Cluster-global addresses exposed to callers.
    pub global: AddressRange,
    /// Address at the owning node or CXL device.
    pub backing_start: u64,
    pub latency_ns: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedAddress {
    pub node: NodeId,
    pub transport: Transport,
    pub backing_address: u64,
    pub failed_over: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MigrationCopy {
    pub migration: Migration,
    pub source: ResolvedAddress,
    pub target: ResolvedAddress,
}

/// How a resolved fabric address may be consumed by the page mapper.
///
/// Local and CXL memory are directly mapped. Layer-2 memory is never exposed
/// as coherent RAM: it is fetched as a complete page into a local NUMA cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryMapping {
    Direct {
        source: ResolvedAddress,
        correlation: CorrelationId,
    },
    RemotePage {
        source: ResolvedAddress,
        correlation: CorrelationId,
        global_page: u64,
        backing_page: u64,
        bytes: u32,
    },
}

impl MemoryMapping {
    pub const fn source(self) -> ResolvedAddress {
        match self {
            Self::Direct { source, .. } | Self::RemotePage { source, .. } => source,
        }
    }

    pub const fn correlation(self) -> CorrelationId {
        match self {
            Self::Direct { correlation, .. }
            | Self::RemotePage { correlation, .. } => correlation,
        }
    }

    pub const fn is_cache_coherent(self) -> bool {
        matches!(
            self,
            Self::Direct {
                source: ResolvedAddress {
                    transport: Transport::Local | Transport::Cxl,
                    ..
                },
                ..
            }
        )
    }
}

#[derive(Clone, Copy)]
struct PageOverride {
    global_page: u64,
    target_pool: PoolId,
    target_backing_page: u64,
}

pub struct GlobalAddressSpace<
    const POOLS: usize = DEFAULT_POOL_CAPACITY,
    const OVERRIDES: usize = DEFAULT_PAGE_TRACKING_CAPACITY,
> {
    pools: [Option<MemoryPool>; POOLS],
    draining: [bool; POOLS],
    overrides: [Option<PageOverride>; OVERRIDES],
    failed_nodes: u64,
}

impl<const POOLS: usize, const OVERRIDES: usize> GlobalAddressSpace<POOLS, OVERRIDES> {
    pub const fn new() -> Self {
        Self {
            pools: [None; POOLS],
            draining: [false; POOLS],
            overrides: [None; OVERRIDES],
            failed_nodes: 0,
        }
    }

    pub fn add_pool(&mut self, pool: MemoryPool) -> Result<(), Error> {
        if pool.global.start % PAGE_SIZE != 0
            || pool.global.length % PAGE_SIZE != 0
            || pool.backing_start % PAGE_SIZE != 0
            || pool.latency_ns == 0
            || pool.mirror == Some(pool.node)
        {
            return Err(Error::Alignment)
        }
        if self.pools.iter().flatten().any(|existing| {
            existing.id == pool.id
                || existing.global.overlaps(pool.global)
                || (existing.node == pool.node
                    && existing.backing_start
                        < pool.backing_start.saturating_add(pool.global.length)
                    && pool.backing_start
                        < existing.backing_start.saturating_add(existing.global.length))
        }) {
            return Err(Error::AddressConflict)
        }
        let slot = self
            .pools
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(Error::Capacity)?;
        *slot = Some(pool);
        Ok(())
    }

    pub fn begin_pool_drain(&mut self, id: PoolId) -> Result<(), Error> {
        let index = self
            .pools
            .iter()
            .position(|entry| entry.is_some_and(|pool| pool.id == id))
            .ok_or(Error::DeviceNotFound)?;
        self.draining[index] = true;
        Ok(())
    }

    pub fn cancel_pool_drain(&mut self, id: PoolId) -> Result<(), Error> {
        let index = self
            .pools
            .iter()
            .position(|entry| entry.is_some_and(|pool| pool.id == id))
            .ok_or(Error::DeviceNotFound)?;
        self.draining[index] = false;
        Ok(())
    }

    pub fn is_pool_draining(&self, id: PoolId) -> bool {
        self.pools
            .iter()
            .position(|entry| entry.is_some_and(|pool| pool.id == id))
            .is_some_and(|index| self.draining[index])
    }

    pub fn remove_pool(&mut self, id: PoolId) -> Result<MemoryPool, Error> {
        let index = self
            .pools
            .iter()
            .position(|entry| entry.is_some_and(|pool| pool.id == id))
            .ok_or(Error::DeviceNotFound)?;
        if !self.draining[index] {
            return Err(Error::Busy)
        }
        let pool = self.pools[index].take().ok_or(Error::DeviceNotFound)?;
        self.draining[index] = false;
        for redirect in &mut self.overrides {
            if redirect.is_some_and(|entry| {
                entry.target_pool == id || pool.global.contains(entry.global_page)
            }) {
                *redirect = None
            }
        }
        Ok(pool)
    }

    pub fn pools(&self) -> impl Iterator<Item = &MemoryPool> {
        self.pools.iter().flatten()
    }

    pub fn pool(&self, id: PoolId) -> Option<&MemoryPool> {
        self.pools().find(|pool| pool.id == id)
    }

    /// Resolve the pool currently owning a page. This follows a committed
    /// migration redirect, so a later migration never copies from stale
    /// ownership metadata.
    pub fn page_pool(&self, global_page: u64) -> Option<&MemoryPool> {
        let global_page = global_page & !(PAGE_SIZE - 1);
        self.overrides
            .iter()
            .flatten()
            .find(|entry| entry.global_page == global_page)
            .and_then(|entry| self.pool(entry.target_pool))
            .or_else(|| self.pools().find(|pool| pool.global.contains(global_page)))
    }

    pub fn resolve(&self, global_address: u64) -> Result<ResolvedAddress, Error> {
        let global_page = global_address & !(PAGE_SIZE - 1);
        let page_offset = global_address & (PAGE_SIZE - 1);
        if let Some(redirect) = self
            .overrides
            .iter()
            .flatten()
            .find(|entry| entry.global_page == global_page)
        {
            let pool = self.pool(redirect.target_pool).ok_or(Error::InvalidAddress)?;
            return self.resolve_node(
                pool,
                redirect.target_backing_page + page_offset,
            )
        }
        let pool = self
            .pools()
            .find(|pool| pool.global.contains(global_address))
            .ok_or(Error::InvalidAddress)?;
        self.resolve_node(
            pool,
            pool.backing_start + global_address - pool.global.start,
        )
    }

    /// Preserve transport semantics for the page mapper.
    ///
    /// CXL HDM windows remain direct mappings. Ethernet pools return a
    /// page-aligned block request for the local NUMA page cache.
    pub fn resolve_mapping(&self, global_address: u64) -> Result<MemoryMapping, Error> {
        let source = self.resolve(global_address)?;
        let correlation = match source.transport {
            Transport::Local => CorrelationId::NONE,
            Transport::Cxl | Transport::Layer2 => {
                next_correlation_id(source.node.raw())
            }
        };
        let mapping = match source.transport {
            Transport::Local | Transport::Cxl => MemoryMapping::Direct {
                source,
                correlation,
            },
            Transport::Layer2 => MemoryMapping::RemotePage {
                source,
                correlation,
                global_page: global_address & !(PAGE_SIZE - 1),
                backing_page: source.backing_address & !(PAGE_SIZE - 1),
                bytes: PAGE_SIZE as u32,
            },
        };
        if !correlation.is_none() {
            emit(
                TraceEvent::new(Level::Trace, EventKind::RemoteMemory)
                    .on_node(source.node.raw())
                    .correlated(correlation)
                    .with_field(EventField::unsigned(
                        field::ADDRESS,
                        global_address,
                    ))
                    .with_field(EventField::unsigned(
                        field::OPERATION,
                        source.transport as u64,
                    )),
            )
        }
        Ok(mapping)
    }

    pub fn mark_node_failed(&mut self, node: NodeId) -> Result<(), Error> {
        let bit = node_bit(node).ok_or(Error::Capacity)?;
        self.failed_nodes |= bit;
        Ok(())
    }

    pub fn mark_node_alive(&mut self, node: NodeId) -> Result<(), Error> {
        let bit = node_bit(node).ok_or(Error::Capacity)?;
        self.failed_nodes &= !bit;
        Ok(())
    }

    pub fn is_node_failed(&self, node: NodeId) -> bool {
        node_bit(node).is_some_and(|bit| self.failed_nodes & bit != 0)
    }

    pub fn redirect_page(
        &mut self,
        global_page: u64,
        target_pool: PoolId,
        target_backing_page: u64,
    ) -> Result<(), Error> {
        if global_page % PAGE_SIZE != 0 || target_backing_page % PAGE_SIZE != 0 {
            return Err(Error::Alignment)
        }
        let target = self.pool(target_pool).ok_or(Error::DeviceNotFound)?;
        if self.is_pool_draining(target_pool) || self.is_node_failed(target.node) {
            return Err(Error::Busy)
        }
        if self.page_pool(global_page).is_none() {
            return Err(Error::InvalidAddress)
        }
        if target_backing_page < target.backing_start
            || target_backing_page.saturating_add(PAGE_SIZE)
                > target.backing_start.saturating_add(target.global.length)
        {
            return Err(Error::InvalidAddress)
        }
        if self.overrides.iter().flatten().any(|entry| {
            entry.global_page != global_page
                && entry.target_pool == target_pool
                && entry.target_backing_page == target_backing_page
        }) {
            return Err(Error::Busy)
        }
        let slot_index = self
            .overrides
            .iter()
            .position(|entry| entry.is_some_and(|item| item.global_page == global_page))
            .or_else(|| self.overrides.iter().position(Option::is_none))
            .ok_or(Error::Capacity)?;
        self.overrides[slot_index] = Some(PageOverride {
            global_page,
            target_pool,
            target_backing_page,
        });
        Ok(())
    }

    /// Prepare a page copy. The caller copies the page using the returned
    /// transport addresses and calls `MigrationPlanner::commit` only after
    /// verifying the destination contents.
    pub fn begin_migration(&self, migration: Migration) -> Result<MigrationCopy, Error> {
        let source_pool = self
            .page_pool(migration.global_page)
            .ok_or(Error::InvalidAddress)?;
        if source_pool.id != migration.source {
            return Err(Error::Busy)
        }
        let target_pool = self.pool(migration.target).ok_or(Error::DeviceNotFound)?;
        if self.is_pool_draining(target_pool.id) || self.is_node_failed(target_pool.node) {
            return Err(Error::Busy)
        }
        if self.overrides.iter().flatten().any(|entry| {
            entry.global_page != migration.global_page
                && entry.target_pool == migration.target
                && entry.target_backing_page == migration.target_backing_page
        }) {
            return Err(Error::Busy)
        }
        let source = self.resolve(migration.global_page)?;
        let target = self.resolve_backing(target_pool, migration.target_backing_page)?;
        Ok(MigrationCopy {
            migration,
            source,
            target,
        })
    }

    fn resolve_backing(
        &self,
        pool: &MemoryPool,
        backing_address: u64,
    ) -> Result<ResolvedAddress, Error> {
        if backing_address % PAGE_SIZE != 0
            || backing_address < pool.backing_start
            || backing_address.saturating_add(PAGE_SIZE)
                > pool.backing_start.saturating_add(pool.global.length)
        {
            return Err(Error::InvalidAddress)
        }
        self.resolve_node(pool, backing_address)
    }

    fn free_backing_page(&self, pool: &MemoryPool) -> Option<u64> {
        let end = pool.backing_start.checked_add(pool.global.length)?;
        let mut page = pool.backing_start;
        while page.checked_add(PAGE_SIZE)? <= end {
            let used = self.overrides.iter().flatten().any(|entry| {
                entry.target_pool == pool.id && entry.target_backing_page == page
            });
            if !used {
                return Some(page)
            }
            page = page.checked_add(PAGE_SIZE)?;
        }
        None
    }

    fn resolve_node(
        &self,
        pool: &MemoryPool,
        backing_address: u64,
    ) -> Result<ResolvedAddress, Error> {
        if !self.is_node_failed(pool.node) {
            return Ok(ResolvedAddress {
                node: pool.node,
                transport: pool.transport,
                backing_address,
                failed_over: false,
            })
        }
        let mirror = pool.mirror.ok_or(Error::NodeFailed)?;
        if self.is_node_failed(mirror) {
            return Err(Error::NodeFailed)
        }
        Ok(ResolvedAddress {
            node: mirror,
            transport: pool.transport,
            backing_address,
            failed_over: true,
        })
    }
}

impl<const POOLS: usize, const OVERRIDES: usize> Default
    for GlobalAddressSpace<POOLS, OVERRIDES>
{
    fn default() -> Self {
        Self::new()
    }
}

const fn node_bit(node: NodeId) -> Option<u64> {
    let raw = node.raw();
    if raw > 64 { None } else { Some(1 << (raw - 1)) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaseRights(u8);

impl LeaseRights {
    pub const READ: Self = Self(1);
    pub const WRITE: Self = Self(2);
    pub const READ_WRITE: Self = Self(3);

    pub const fn permits(self, access: Access) -> bool {
        match access {
            Access::Read => self.0 & Self::READ.0 != 0,
            Access::Write => self.0 & Self::WRITE.0 != 0,
            Access::Execute => false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeaseOwner {
    Node(NodeId),
    Service(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct LeaseHandle(u64);

impl LeaseHandle {
    const fn from_parts(slot: usize, generation: u32) -> Self {
        Self((generation as u64) << 32 | slot as u64)
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
pub struct LeaseInfo {
    pub handle: LeaseHandle,
    pub owner: LeaseOwner,
    pub pool: PoolId,
    pub range: AddressRange,
    pub rights: LeaseRights,
    pub expires_at_us: u64,
}

#[derive(Clone, Copy)]
struct Lease {
    occupied: bool,
    generation: u32,
    owner: LeaseOwner,
    pool: PoolId,
    range: AddressRange,
    rights: LeaseRights,
    expires_at_us: u64,
}

impl Lease {
    const VACANT: Self = Self {
        occupied: false,
        generation: 0,
        owner: LeaseOwner::Node(NodeId::LOCAL),
        pool: PoolId(0),
        range: AddressRange {
            start: 0,
            length: 1,
        },
        rights: LeaseRights::READ,
        expires_at_us: 0,
    };
}

/// Dynamic, generation-checked leases over global RAM or VRAM ranges.
pub struct LeaseTable<const CAPACITY: usize = DEFAULT_LEASE_CAPACITY> {
    leases: [Lease; CAPACITY],
}

impl<const CAPACITY: usize> LeaseTable<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            leases: [Lease::VACANT; CAPACITY],
        }
    }

    pub fn allocate<const POOLS: usize, const OVERRIDES: usize>(
        &mut self,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        owner: LeaseOwner,
        pool_id: PoolId,
        length: u64,
        alignment: u64,
        rights: LeaseRights,
        now_us: u64,
        duration_us: u64,
    ) -> Result<(LeaseHandle, AddressRange), Error> {
        self.expire(now_us);
        let pool = space.pool(pool_id).ok_or(Error::DeviceNotFound)?;
        if space.is_pool_draining(pool_id) {
            return Err(Error::Busy)
        }
        if length == 0
            || alignment < PAGE_SIZE
            || !alignment.is_power_of_two()
            || length % PAGE_SIZE != 0
            || duration_us == 0
        {
            return Err(Error::Alignment)
        }
        let mut cursor = align_up(pool.global.start, alignment).ok_or(Error::InvalidRange)?;
        let pool_end = pool.global.end();
        loop {
            let end = cursor.checked_add(length).ok_or(Error::InvalidRange)?;
            if end > pool_end {
                return Err(Error::Capacity)
            }
            let candidate = AddressRange {
                start: cursor,
                length,
            };
            let conflict = self
                .leases
                .iter()
                .filter(|lease| lease.occupied && lease.pool == pool_id)
                .filter(|lease| lease.range.overlaps(candidate))
                .max_by_key(|lease| lease.range.end());
            if let Some(conflict) = conflict {
                cursor = align_up(conflict.range.end(), alignment)
                    .ok_or(Error::InvalidRange)?;
                continue
            }
            let slot_index = self
                .leases
                .iter()
                .position(|lease| !lease.occupied)
                .ok_or(Error::Capacity)?;
            let generation = self.leases[slot_index].generation.wrapping_add(1).max(1);
            self.leases[slot_index] = Lease {
                occupied: true,
                generation,
                owner,
                pool: pool_id,
                range: candidate,
                rights,
                expires_at_us: now_us.saturating_add(duration_us),
            };
            return Ok((
                LeaseHandle::from_parts(slot_index, generation),
                candidate,
            ))
        }
    }

    pub fn largest_free_range<const POOLS: usize, const OVERRIDES: usize>(
        &mut self,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        pool_id: PoolId,
        alignment: u64,
        now_us: u64,
    ) -> Result<AddressRange, Error> {
        self.expire(now_us);
        let pool = space.pool(pool_id).ok_or(Error::DeviceNotFound)?;
        if alignment < PAGE_SIZE || !alignment.is_power_of_two() {
            return Err(Error::Alignment)
        }
        let mut cursor = align_up(pool.global.start, alignment).ok_or(Error::InvalidRange)?;
        let mut largest = AddressRange {
            start: cursor,
            length: 0,
        };
        while cursor < pool.global.end() {
            let next = self
                .leases
                .iter()
                .filter(|lease| {
                    lease.occupied
                        && lease.pool == pool_id
                        && lease.range.end() > cursor
                })
                .min_by_key(|lease| lease.range.start);
            let gap_end = next
                .map(|lease| lease.range.start)
                .unwrap_or_else(|| pool.global.end());
            if gap_end > cursor {
                let length = (gap_end - cursor) / PAGE_SIZE * PAGE_SIZE;
                if length > largest.length {
                    largest = AddressRange {
                        start: cursor,
                        length,
                    }
                }
            }
            let Some(lease) = next else { break };
            cursor = align_up(
                core::cmp::max(cursor, lease.range.end()),
                alignment,
            )
            .ok_or(Error::InvalidRange)?;
        }
        if largest.length == 0 {
            Err(Error::Capacity)
        } else {
            Ok(largest)
        }
    }

    pub fn authorize(
        &self,
        handle: LeaseHandle,
        owner: LeaseOwner,
        address: u64,
        access: Access,
        now_us: u64,
    ) -> Result<(), Error> {
        let lease = self.valid(handle)?;
        if lease.owner != owner {
            return Err(Error::NotOwner)
        }
        if now_us >= lease.expires_at_us {
            return Err(Error::ExpiredLease)
        }
        if !lease.range.contains(address) || !lease.rights.permits(access) {
            return Err(Error::NotOwner)
        }
        Ok(())
    }

    pub fn renew(
        &mut self,
        handle: LeaseHandle,
        owner: LeaseOwner,
        now_us: u64,
        duration_us: u64,
    ) -> Result<(), Error> {
        let slot = self.valid_slot(handle)?;
        let lease = &mut self.leases[slot];
        if lease.owner != owner {
            return Err(Error::NotOwner)
        }
        if now_us >= lease.expires_at_us {
            lease.occupied = false;
            return Err(Error::ExpiredLease)
        }
        if duration_us == 0 {
            return Err(Error::InvalidRange)
        }
        lease.expires_at_us = now_us.saturating_add(duration_us);
        Ok(())
    }

    pub fn release(&mut self, handle: LeaseHandle, owner: LeaseOwner) -> Result<(), Error> {
        let slot = self.valid_slot(handle)?;
        if self.leases[slot].owner != owner {
            return Err(Error::NotOwner)
        }
        self.leases[slot].occupied = false;
        Ok(())
    }

    pub fn release_node(&mut self, node: NodeId) -> usize {
        let mut released = 0;
        for lease in &mut self.leases {
            if lease.occupied && lease.owner == LeaseOwner::Node(node) {
                lease.occupied = false;
                released += 1
            }
        }
        released
    }

    pub fn active_for_pool(&self, pool: PoolId) -> usize {
        self.leases
            .iter()
            .filter(|lease| lease.occupied && lease.pool == pool)
            .count()
    }

    pub fn leases(&self) -> impl Iterator<Item = LeaseInfo> + '_ {
        self.leases
            .iter()
            .enumerate()
            .filter(|(_, lease)| lease.occupied)
            .map(|(slot, lease)| LeaseInfo {
                handle: LeaseHandle::from_parts(slot, lease.generation),
                owner: lease.owner,
                pool: lease.pool,
                range: lease.range,
                rights: lease.rights,
                expires_at_us: lease.expires_at_us,
            })
    }

    pub fn revoke_pool(&mut self, pool: PoolId) -> usize {
        let mut revoked = 0;
        for lease in &mut self.leases {
            if lease.occupied && lease.pool == pool {
                lease.occupied = false;
                revoked += 1
            }
        }
        revoked
    }

    pub fn expire(&mut self, now_us: u64) -> usize {
        let mut expired = 0;
        for lease in &mut self.leases {
            if lease.occupied && now_us >= lease.expires_at_us {
                lease.occupied = false;
                expired += 1
            }
        }
        expired
    }

    fn valid(&self, handle: LeaseHandle) -> Result<&Lease, Error> {
        Ok(&self.leases[self.valid_slot(handle)?])
    }

    fn valid_slot(&self, handle: LeaseHandle) -> Result<usize, Error> {
        let slot = handle.slot();
        let lease = self.leases.get(slot).ok_or(Error::LeaseNotFound)?;
        if !lease.occupied || lease.generation != handle.generation() {
            return Err(Error::LeaseNotFound)
        }
        Ok(slot)
    }
}

impl<const CAPACITY: usize> Default for LeaseTable<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn align_up(value: u64, alignment: u64) -> Option<u64> {
    let mask = alignment - 1;
    match value.checked_add(mask) {
        Some(sum) => Some(sum & !mask),
        None => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Migration {
    pub global_page: u64,
    pub source: PoolId,
    pub target: PoolId,
    pub target_backing_page: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageAccess {
    pub global_page: u64,
    pub reads: u32,
    pub writes: u32,
    pub average_latency_ns: u32,
    pub samples: u32,
}

#[derive(Clone, Copy)]
struct PageMetric {
    global_page: u64,
    reads: u32,
    writes: u32,
    latency_total_ns: u64,
    samples: u32,
}

impl PageMetric {
    const EMPTY: Self = Self {
        global_page: u64::MAX,
        reads: 0,
        writes: 0,
        latency_total_ns: 0,
        samples: 0,
    };
}

/// Bounded access sampler that recommends migration to a lower-latency pool.
pub struct MigrationPlanner<const CAPACITY: usize = DEFAULT_PAGE_TRACKING_CAPACITY> {
    metrics: [PageMetric; CAPACITY],
    hot_accesses: u32,
    latency_saving_ns: u32,
}

impl<const CAPACITY: usize> MigrationPlanner<CAPACITY> {
    pub const fn new(hot_accesses: u32, latency_saving_ns: u32) -> Self {
        Self {
            metrics: [PageMetric::EMPTY; CAPACITY],
            hot_accesses,
            latency_saving_ns,
        }
    }

    pub fn record(
        &mut self,
        global_address: u64,
        access: Access,
        latency_ns: u32,
    ) -> Result<(), Error> {
        let page = global_address & !(PAGE_SIZE - 1);
        let slot_index = self
            .metrics
            .iter()
            .position(|metric| metric.global_page == page)
            .or_else(|| {
                self.metrics
                    .iter()
                    .position(|metric| metric.global_page == u64::MAX)
            })
            .ok_or(Error::Capacity)?;
        let slot = &mut self.metrics[slot_index];
        if slot.global_page == u64::MAX {
            slot.global_page = page
        }
        match access {
            Access::Read | Access::Execute => slot.reads = slot.reads.saturating_add(1),
            Access::Write => slot.writes = slot.writes.saturating_add(1),
        }
        slot.latency_total_ns = slot.latency_total_ns.saturating_add(latency_ns as u64);
        slot.samples = slot.samples.saturating_add(1);
        Ok(())
    }

    pub fn accesses(&self) -> impl Iterator<Item = PageAccess> + '_ {
        self.metrics
            .iter()
            .filter(|metric| metric.global_page != u64::MAX && metric.samples != 0)
            .map(|metric| PageAccess {
                global_page: metric.global_page,
                reads: metric.reads,
                writes: metric.writes,
                average_latency_ns: (metric.latency_total_ns / metric.samples as u64)
                    .min(u64::from(u32::MAX)) as u32,
                samples: metric.samples,
            })
    }

    pub fn recommend<const POOLS: usize, const OVERRIDES: usize>(
        &self,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        global_page: u64,
    ) -> Option<Migration> {
        let metric = self
            .metrics
            .iter()
            .find(|metric| metric.global_page == global_page)?;
        if metric.reads.saturating_add(metric.writes) < self.hot_accesses || metric.samples == 0 {
            return None
        }
        let source = space.page_pool(global_page)?;
        let observed = (metric.latency_total_ns / metric.samples as u64)
            .min(u64::from(u32::MAX)) as u32;
        let target = space
            .pools()
            .filter(|pool| {
                pool.id != source.id
                    && pool.kind == source.kind
                    && pool.global.length >= PAGE_SIZE
                    && !space.is_pool_draining(pool.id)
                    && !space.is_node_failed(pool.node)
                })
            .min_by_key(|pool| pool.latency_ns)?;
        if target.latency_ns.saturating_add(self.latency_saving_ns) >= observed {
            return None
        }
        let target_backing_page = space.free_backing_page(target)?;
        Some(Migration {
            global_page,
            source: source.id,
            target: target.id,
            target_backing_page,
        })
    }

    /// Atomically changes future resolution after the caller copies and
    /// verifies the page through its selected transport.
    pub fn commit<const POOLS: usize, const OVERRIDES: usize>(
        &mut self,
        space: &mut GlobalAddressSpace<POOLS, OVERRIDES>,
        migration: Migration,
    ) -> Result<(), Error> {
        space.begin_migration(migration)?;
        space.redirect_page(
            migration.global_page,
            migration.target,
            migration.target_backing_page,
        )?;
        if let Some(metric) = self
            .metrics
            .iter_mut()
            .find(|metric| metric.global_page == migration.global_page)
        {
            *metric = PageMetric::EMPTY
        }
        Ok(())
    }
}
