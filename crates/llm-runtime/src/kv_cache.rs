use synos_fabric::{
    Access, NodeId, PAGE_SIZE,
    memory::{
        GlobalAddressSpace, LeaseTable, MemoryKind, Migration, MigrationCopy,
        MigrationPlanner,
    },
};

use crate::{
    Error, RequestId,
    allocator::{
        AllocationHandle, AllocationPolicy, ModelAddress, UnifiedAllocator,
    },
};

pub const DEFAULT_CACHE_CAPACITY: usize = 128;
pub const DEFAULT_SEGMENTS_PER_CACHE: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct KvCacheHandle(u64);

impl KvCacheHandle {
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

#[derive(Clone, Copy)]
struct CacheSegment {
    allocation: AllocationHandle,
    first_token: u64,
    token_capacity: u64,
}

#[derive(Clone, Copy)]
struct CacheEntry<const SEGMENTS: usize> {
    occupied: bool,
    generation: u32,
    request: Option<RequestId>,
    owner: NodeId,
    memory_kind: MemoryKind,
    bytes_per_token: u64,
    reserved_tokens: u64,
    committed_tokens: u64,
    segments: [Option<CacheSegment>; SEGMENTS],
    segment_count: u16,
}

impl<const SEGMENTS: usize> CacheEntry<SEGMENTS> {
    const EMPTY: Self = Self {
        occupied: false,
        generation: 0,
        request: None,
        owner: NodeId::LOCAL,
        memory_kind: MemoryKind::Ram,
        bytes_per_token: 0,
        reserved_tokens: 0,
        committed_tokens: 0,
        segments: [None; SEGMENTS],
        segment_count: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KvCacheInfo {
    pub handle: KvCacheHandle,
    pub request: RequestId,
    pub bytes_per_token: u64,
    pub reserved_tokens: u64,
    pub committed_tokens: u64,
    pub segment_count: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KvCacheRebalance {
    pub cache: KvCacheHandle,
    pub allocation: AllocationHandle,
    pub allocation_offset: u64,
    pub migration: Migration,
    pub copy: MigrationCopy,
}

/// Growable KV cache whose segments may live in local, CXL, or layer-2 RAM.
///
/// Token addressing stays stable when the cache grows. The runtime, not the
/// model framework, chooses the physical tier for every new segment.
pub struct KvCachePool<
    const CACHES: usize = DEFAULT_CACHE_CAPACITY,
    const SEGMENTS: usize = DEFAULT_SEGMENTS_PER_CACHE,
> {
    caches: [CacheEntry<SEGMENTS>; CACHES],
}

impl<const CACHES: usize, const SEGMENTS: usize> KvCachePool<CACHES, SEGMENTS> {
    pub const fn new() -> Self {
        Self {
            caches: [CacheEntry::EMPTY; CACHES],
        }
    }

    pub fn open<
        const ALLOCATIONS: usize,
        const EXTENTS: usize,
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
    >(
        &mut self,
        request: RequestId,
        owner: NodeId,
        bytes_per_token: u64,
        initial_tokens: u64,
        allocator: &mut UnifiedAllocator<ALLOCATIONS, EXTENTS>,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &mut LeaseTable<LEASES>,
        now_us: u64,
        lease_duration_us: u64,
    ) -> Result<KvCacheInfo, Error> {
        self.open_in_memory(
            request,
            owner,
            MemoryKind::Ram,
            bytes_per_token,
            initial_tokens,
            allocator,
            space,
            leases,
            now_us,
            lease_duration_us,
        )
    }

    pub fn open_in_memory<
        const ALLOCATIONS: usize,
        const EXTENTS: usize,
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
    >(
        &mut self,
        request: RequestId,
        owner: NodeId,
        memory_kind: MemoryKind,
        bytes_per_token: u64,
        initial_tokens: u64,
        allocator: &mut UnifiedAllocator<ALLOCATIONS, EXTENTS>,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &mut LeaseTable<LEASES>,
        now_us: u64,
        lease_duration_us: u64,
    ) -> Result<KvCacheInfo, Error> {
        if bytes_per_token == 0 || initial_tokens == 0 {
            return Err(Error::InvalidRange)
        }
        if self
            .caches
            .iter()
            .any(|entry| entry.occupied && entry.request == Some(request))
        {
            return Err(Error::InvalidHandle)
        }
        let slot = self
            .caches
            .iter()
            .position(|entry| !entry.occupied)
            .ok_or(Error::Capacity)?;
        let allocation = allocator.allocate(
            space,
            leases,
            allocation_bytes(bytes_per_token, initial_tokens)?,
            PAGE_SIZE,
            AllocationPolicy::resilient_cluster(
                owner,
                memory_kind,
                request.raw(),
            ),
            now_us,
            lease_duration_us,
        )?;
        let generation = self.caches[slot].generation.wrapping_add(1).max(1);
        let handle = KvCacheHandle::from_parts(slot, generation);
        let mut segments = [None; SEGMENTS];
        if SEGMENTS == 0 {
            allocator.release(allocation.handle, leases)?;
            return Err(Error::Capacity)
        }
        segments[0] = Some(CacheSegment {
            allocation: allocation.handle,
            first_token: 0,
            token_capacity: initial_tokens,
        });
        self.caches[slot] = CacheEntry {
            occupied: true,
            generation,
            request: Some(request),
            owner,
            memory_kind,
            bytes_per_token,
            reserved_tokens: initial_tokens,
            committed_tokens: 0,
            segments,
            segment_count: 1,
        };
        Ok(self.info(handle)?)
    }

    pub fn grow<
        const ALLOCATIONS: usize,
        const EXTENTS: usize,
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
    >(
        &mut self,
        handle: KvCacheHandle,
        additional_tokens: u64,
        allocator: &mut UnifiedAllocator<ALLOCATIONS, EXTENTS>,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &mut LeaseTable<LEASES>,
        now_us: u64,
        lease_duration_us: u64,
    ) -> Result<KvCacheInfo, Error> {
        let slot = self.valid_slot(handle)?;
        if additional_tokens == 0
            || self.caches[slot].segment_count as usize >= SEGMENTS
        {
            return Err(Error::Capacity)
        }
        let entry = self.caches[slot];
        let allocation = allocator.allocate(
            space,
            leases,
            allocation_bytes(entry.bytes_per_token, additional_tokens)?,
            PAGE_SIZE,
            AllocationPolicy::resilient_cluster(
                entry.owner,
                entry.memory_kind,
                entry.request.ok_or(Error::CacheNotFound)?.raw(),
            ),
            now_us,
            lease_duration_us,
        )?;
        let segment_slot = self.caches[slot].segment_count as usize;
        self.caches[slot].segments[segment_slot] = Some(CacheSegment {
            allocation: allocation.handle,
            first_token: entry.reserved_tokens,
            token_capacity: additional_tokens,
        });
        self.caches[slot].segment_count += 1;
        self.caches[slot].reserved_tokens = self.caches[slot]
            .reserved_tokens
            .checked_add(additional_tokens)
            .ok_or(Error::InvalidRange)?;
        self.info(handle)
    }

    pub fn commit_tokens(
        &mut self,
        handle: KvCacheHandle,
        total_tokens: u64,
    ) -> Result<(), Error> {
        let slot = self.valid_slot(handle)?;
        let entry = &mut self.caches[slot];
        if total_tokens < entry.committed_tokens || total_tokens > entry.reserved_tokens {
            return Err(Error::InvalidRange)
        }
        entry.committed_tokens = total_tokens;
        Ok(())
    }

    pub fn resolve<
        const ALLOCATIONS: usize,
        const EXTENTS: usize,
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
    >(
        &self,
        handle: KvCacheHandle,
        token: u64,
        byte_in_token: u64,
        access: Access,
        now_us: u64,
        allocator: &UnifiedAllocator<ALLOCATIONS, EXTENTS>,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &LeaseTable<LEASES>,
    ) -> Result<ModelAddress, Error> {
        let entry = self.entry(handle)?;
        if token >= entry.reserved_tokens || byte_in_token >= entry.bytes_per_token {
            return Err(Error::InvalidRange)
        }
        let segment = entry
            .segments
            .iter()
            .flatten()
            .find(|segment| {
                token >= segment.first_token
                    && token < segment.first_token + segment.token_capacity
            })
            .ok_or(Error::CacheNotFound)?;
        let local_token = token - segment.first_token;
        let offset = local_token
            .checked_mul(entry.bytes_per_token)
            .and_then(|value| value.checked_add(byte_in_token))
            .ok_or(Error::InvalidRange)?;
        allocator.resolve(
            segment.allocation,
            offset,
            access,
            now_us,
            space,
            leases,
        )
    }

    pub fn info(&self, handle: KvCacheHandle) -> Result<KvCacheInfo, Error> {
        let entry = self.entry(handle)?;
        Ok(KvCacheInfo {
            handle,
            request: entry.request.ok_or(Error::CacheNotFound)?,
            bytes_per_token: entry.bytes_per_token,
            reserved_tokens: entry.reserved_tokens,
            committed_tokens: entry.committed_tokens,
            segment_count: entry.segment_count,
        })
    }

    /// Return the next hot KV-cache page that should move to a lower-latency
    /// pool. The cursor is a page ordinal across committed cache segments.
    /// The returned copy descriptor lets the transport worker move and verify
    /// the page before the caller commits the redirect.
    pub fn next_rebalance<
        const TRACKING: usize,
        const ALLOCATIONS: usize,
        const EXTENTS: usize,
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
    >(
        &self,
        handle: KvCacheHandle,
        cursor: &mut u64,
        planner: &MigrationPlanner<TRACKING>,
        allocator: &UnifiedAllocator<ALLOCATIONS, EXTENTS>,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &LeaseTable<LEASES>,
        now_us: u64,
    ) -> Result<Option<KvCacheRebalance>, Error> {
        let entry = self.entry(handle)?;
        let committed_tokens = entry.committed_tokens;
        let mut remaining_pages = *cursor;
        for segment in entry.segments.iter().flatten() {
            if segment.first_token >= committed_tokens {
                break
            }
            let token_count = core::cmp::min(
                segment.token_capacity,
                committed_tokens - segment.first_token,
            );
            let bytes = allocation_bytes(entry.bytes_per_token, token_count)?;
            let segment_pages = bytes
                .checked_add(PAGE_SIZE - 1)
                .ok_or(Error::InvalidRange)?
                / PAGE_SIZE;
            if remaining_pages >= segment_pages {
                remaining_pages -= segment_pages;
                continue
            }
            let allocation_offset = remaining_pages
                .checked_mul(PAGE_SIZE)
                .ok_or(Error::InvalidRange)?;
            *cursor = cursor.saturating_add(1);
            let address = allocator.resolve(
                segment.allocation,
                allocation_offset,
                Access::Read,
                now_us,
                space,
                leases,
            )?;
            if let Some(migration) = planner.recommend(space, address.fabric_address) {
                let copy = space.begin_migration(migration)?;
                return Ok(Some(KvCacheRebalance {
                    cache: handle,
                    allocation: segment.allocation,
                    allocation_offset,
                    migration,
                    copy,
                }))
            }
            remaining_pages = 0;
        }
        Ok(None)
    }

    /// Rebalance committed KV pages. The callback owns the actual copy for
    /// CXL or DSM and must return true only after destination verification.
    pub fn rebalance<
        const TRACKING: usize,
        const ALLOCATIONS: usize,
        const EXTENTS: usize,
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
        F,
    >(
        &self,
        handle: KvCacheHandle,
        planner: &mut MigrationPlanner<TRACKING>,
        allocator: &UnifiedAllocator<ALLOCATIONS, EXTENTS>,
        space: &mut GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &LeaseTable<LEASES>,
        now_us: u64,
        max_pages: usize,
        mut copy_and_verify: F,
    ) -> Result<usize, Error>
    where
        F: FnMut(MigrationCopy) -> Result<bool, Error>,
    {
        let mut cursor = 0;
        let mut migrated = 0;
        while migrated < max_pages {
            let Some(plan) = self.next_rebalance(
                handle,
                &mut cursor,
                planner,
                allocator,
                space,
                leases,
                now_us,
            )?
            else {
                break
            };
            if copy_and_verify(plan.copy)? {
                planner.commit(space, plan.migration)?;
                migrated += 1
            }
        }
        Ok(migrated)
    }

    pub fn close<
        const ALLOCATIONS: usize,
        const EXTENTS: usize,
        const LEASES: usize,
    >(
        &mut self,
        handle: KvCacheHandle,
        allocator: &mut UnifiedAllocator<ALLOCATIONS, EXTENTS>,
        leases: &mut LeaseTable<LEASES>,
    ) -> Result<(), Error> {
        let slot = self.valid_slot(handle)?;
        let entry = self.caches[slot];
        for segment in entry.segments.iter().flatten() {
            allocator.release(segment.allocation, leases)?
        }
        self.caches[slot].occupied = false;
        Ok(())
    }

    fn entry(&self, handle: KvCacheHandle) -> Result<&CacheEntry<SEGMENTS>, Error> {
        Ok(&self.caches[self.valid_slot(handle)?])
    }

    fn valid_slot(&self, handle: KvCacheHandle) -> Result<usize, Error> {
        let slot = handle.slot();
        let entry = self.caches.get(slot).ok_or(Error::InvalidHandle)?;
        if !entry.occupied || entry.generation != handle.generation() {
            return Err(Error::CacheNotFound)
        }
        Ok(slot)
    }
}

impl<const CACHES: usize, const SEGMENTS: usize> Default for KvCachePool<CACHES, SEGMENTS> {
    fn default() -> Self {
        Self::new()
    }
}

fn allocation_bytes(bytes_per_token: u64, tokens: u64) -> Result<u64, Error> {
    bytes_per_token
        .checked_mul(tokens)
        .ok_or(Error::InvalidRange)
}
