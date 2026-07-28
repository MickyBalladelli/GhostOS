use synos_fabric::{
    Access, NodeId, PAGE_SIZE,
    memory::{GlobalAddressSpace, LeaseTable, MemoryKind},
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
                MemoryKind::Ram,
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
                MemoryKind::Ram,
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
