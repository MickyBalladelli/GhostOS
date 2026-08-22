use ghostos_fabric::{
    Access, PAGE_SIZE,
    memory::{GlobalAddressSpace, LeaseTable, MemoryMapping},
};

use crate::{
    Error,
    allocator::{AllocationHandle, ModelAddress, UnifiedAllocator},
};

pub const DEFAULT_PREFETCH_STREAMS: usize = 32;
pub const DEFAULT_PREFETCH_QUEUE: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PrefetchRequest {
    pub allocation: AllocationHandle,
    pub offset: u64,
    pub address: ModelAddress,
}

#[derive(Clone, Copy)]
struct Stream {
    allocation: u64,
    last_page: u64,
    sequential_reads: u8,
}

impl Stream {
    const EMPTY: Self = Self {
        allocation: 0,
        last_page: 0,
        sequential_reads: 0,
    };
}

/// Bounded predictor feeding an asynchronous page-fetch worker.
///
/// The allocator never waits for I/O here. After consecutive page reads it
/// resolves future pages and places only non-local work on a fixed queue.
pub struct PredictivePrefetcher<
    const STREAMS: usize = DEFAULT_PREFETCH_STREAMS,
    const QUEUE: usize = DEFAULT_PREFETCH_QUEUE,
> {
    streams: [Stream; STREAMS],
    queue: [Option<PrefetchRequest>; QUEUE],
    head: usize,
    length: usize,
    trigger_reads: u8,
    distance_pages: u8,
}

impl<const STREAMS: usize, const QUEUE: usize> PredictivePrefetcher<STREAMS, QUEUE> {
    pub const fn new(trigger_reads: u8, distance_pages: u8) -> Self {
        Self {
            streams: [Stream::EMPTY; STREAMS],
            queue: [None; QUEUE],
            head: 0,
            length: 0,
            trigger_reads,
            distance_pages,
        }
    }

    pub fn observe<
        const ALLOCATIONS: usize,
        const EXTENTS: usize,
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
    >(
        &mut self,
        allocator: &UnifiedAllocator<ALLOCATIONS, EXTENTS>,
        allocation: AllocationHandle,
        offset: u64,
        access: Access,
        now_us: u64,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &LeaseTable<LEASES>,
    ) -> Result<usize, Error> {
        if access == Access::Write || STREAMS == 0 || QUEUE == 0 {
            return Ok(0);
        }

        let page = offset & !(PAGE_SIZE - 1);
        let slot = self
            .streams
            .iter()
            .position(|stream| stream.allocation == allocation.raw())
            .or_else(|| {
                self.streams
                    .iter()
                    .position(|stream| stream.allocation == 0)
            })
            .unwrap_or_else(|| {
                self.streams
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, stream)| stream.sequential_reads)
                    .map_or(0, |(slot, _)| slot)
            });
        let stream = &mut self.streams[slot];
        if stream.allocation != allocation.raw() {
            *stream = Stream {
                allocation: allocation.raw(),
                last_page: page,
                sequential_reads: 1,
            };
            return Ok(0);
        }

        stream.sequential_reads = if page == stream.last_page.saturating_add(PAGE_SIZE) {
            stream.sequential_reads.saturating_add(1)
        } else if page == stream.last_page {
            stream.sequential_reads
        } else {
            1
        };
        stream.last_page = page;
        if stream.sequential_reads < self.trigger_reads.max(2) {
            return Ok(0);
        }

        let mut queued = 0;
        for distance in 1..=self.distance_pages {
            if self.length == QUEUE {
                break;
            }
            let Some(next_offset) = page.checked_add(PAGE_SIZE * u64::from(distance)) else {
                break;
            };
            let address = match allocator.resolve(
                allocation,
                next_offset,
                Access::Read,
                now_us,
                space,
                leases,
            ) {
                Ok(address) => address,
                Err(Error::InvalidRange | Error::AllocationNotFound) => break,
                Err(error) => return Err(error),
            };
            if matches!(
                address.mapping,
                MemoryMapping::Direct { source, .. }
                    if source.transport == ghostos_fabric::memory::Transport::Local
            ) || self.contains(allocation, next_offset)
            {
                continue;
            }
            let tail = (self.head + self.length) % QUEUE;
            self.queue[tail] = Some(PrefetchRequest {
                allocation,
                offset: next_offset,
                address,
            });
            self.length += 1;
            queued += 1;
        }
        Ok(queued)
    }

    pub fn pop(&mut self) -> Option<PrefetchRequest> {
        if self.length == 0 || QUEUE == 0 {
            return None;
        }
        let request = self.queue[self.head].take();
        self.head = (self.head + 1) % QUEUE;
        self.length -= 1;
        request
    }

    pub const fn pending(&self) -> usize {
        self.length
    }

    fn contains(&self, allocation: AllocationHandle, offset: u64) -> bool {
        self.queue
            .iter()
            .flatten()
            .any(|request| request.allocation == allocation && request.offset == offset)
    }
}

impl<const STREAMS: usize, const QUEUE: usize> Default for PredictivePrefetcher<STREAMS, QUEUE> {
    fn default() -> Self {
        Self::new(2, 4)
    }
}
