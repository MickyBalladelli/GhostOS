use ghostos_fabric::{
    Access, NodeId,
    memory::{GlobalAddressSpace, LeaseTable},
};

use crate::{
    Error, RequestId,
    allocator::{AllocationHandle, ModelAddress, UnifiedAllocator},
    kv_cache::{KvCacheHandle, KvCachePool},
};

pub const DEFAULT_INFERENCE_CAPACITY: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct InferenceHandle(u64);

impl InferenceHandle {
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
pub enum InferenceState {
    Replicating,
    Running,
    Degraded,
    Complete,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecoveryRecord {
    pub request: RequestId,
    pub model: AllocationHandle,
    pub kv_cache: KvCacheHandle,
    pub next_token: u64,
    pub rng_state: u64,
    pub epoch: u64,
    pub primary: NodeId,
    pub replica: NodeId,
    checksum: u32,
}

impl RecoveryRecord {
    pub const WIRE_BYTES: usize = 68;
    const MAGIC: [u8; 4] = *b"SYNR";
    const VERSION: u8 = 1;

    fn new(
        request: RequestId,
        model: AllocationHandle,
        kv_cache: KvCacheHandle,
        next_token: u64,
        rng_state: u64,
        epoch: u64,
        primary: NodeId,
        replica: NodeId,
    ) -> Self {
        let mut record = Self {
            request,
            model,
            kv_cache,
            next_token,
            rng_state,
            epoch,
            primary,
            replica,
            checksum: 0,
        };
        record.checksum = record.calculate_checksum();
        record
    }

    pub fn encode(self) -> [u8; Self::WIRE_BYTES] {
        let mut bytes = [0; Self::WIRE_BYTES];
        bytes[0..4].copy_from_slice(&Self::MAGIC);
        bytes[4] = Self::VERSION;
        bytes[8..16].copy_from_slice(&self.request.raw().to_be_bytes());
        bytes[16..24].copy_from_slice(&self.model.raw().to_be_bytes());
        bytes[24..32].copy_from_slice(&self.kv_cache.raw().to_be_bytes());
        bytes[32..40].copy_from_slice(&self.next_token.to_be_bytes());
        bytes[40..48].copy_from_slice(&self.rng_state.to_be_bytes());
        bytes[48..56].copy_from_slice(&self.epoch.to_be_bytes());
        bytes[56..60].copy_from_slice(&self.primary.raw().to_be_bytes());
        bytes[60..64].copy_from_slice(&self.replica.raw().to_be_bytes());
        bytes[64..68].copy_from_slice(&self.checksum.to_be_bytes());
        bytes
    }

    pub fn decode(bytes: [u8; Self::WIRE_BYTES]) -> Result<Self, Error> {
        if bytes[0..4] != Self::MAGIC || bytes[4] != Self::VERSION {
            return Err(Error::CorruptRecoveryRecord)
        }
        let record = Self {
            request: RequestId::new(read_u64(&bytes, 8))
                .ok_or(Error::CorruptRecoveryRecord)?,
            model: AllocationHandle::from_raw(read_u64(&bytes, 16))
                .ok_or(Error::CorruptRecoveryRecord)?,
            kv_cache: KvCacheHandle::from_raw(read_u64(&bytes, 24))
                .ok_or(Error::CorruptRecoveryRecord)?,
            next_token: read_u64(&bytes, 32),
            rng_state: read_u64(&bytes, 40),
            epoch: read_u64(&bytes, 48),
            primary: NodeId::new(read_u32(&bytes, 56))
                .ok_or(Error::CorruptRecoveryRecord)?,
            replica: NodeId::new(read_u32(&bytes, 60))
                .ok_or(Error::CorruptRecoveryRecord)?,
            checksum: read_u32(&bytes, 64),
        };
        if record.primary == record.replica
            || record.epoch == 0
            || record.checksum != record.calculate_checksum()
        {
            return Err(Error::CorruptRecoveryRecord)
        }
        Ok(record)
    }

    fn calculate_checksum(self) -> u32 {
        let mut copy = self;
        copy.checksum = 0;
        let bytes = copy.encode();
        crc32(&bytes[..64])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointWrite {
    pub primary: NodeId,
    pub replica: NodeId,
    pub epoch: u64,
    pub bytes: [u8; RecoveryRecord::WIRE_BYTES],
}

#[derive(Clone, Copy)]
struct InferenceEntry {
    occupied: bool,
    generation: u32,
    state: InferenceState,
    committed: Option<RecoveryRecord>,
    pending: Option<RecoveryRecord>,
    failed_nodes: u64,
}

impl InferenceEntry {
    const EMPTY: Self = Self {
        occupied: false,
        generation: 0,
        state: InferenceState::Complete,
        committed: None,
        pending: None,
        failed_nodes: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InferenceInfo {
    pub handle: InferenceHandle,
    pub request: RequestId,
    pub state: InferenceState,
    pub committed_epoch: u64,
    pub next_token: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryDegradationHandle {
    pub inference: InferenceHandle,
    pub request: RequestId,
    pub model: AllocationHandle,
    pub kv_cache: KvCacheHandle,
    pub next_token: u64,
    pub rng_state: u64,
    pub committed_epoch: u64,
    pub journal_node: NodeId,
}

impl MemoryDegradationHandle {
    pub fn resolve_model<
        const ALLOCATIONS: usize,
        const EXTENTS: usize,
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
    >(
        self,
        offset: u64,
        access: Access,
        now_us: u64,
        allocator: &UnifiedAllocator<ALLOCATIONS, EXTENTS>,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &LeaseTable<LEASES>,
    ) -> Result<ModelAddress, Error> {
        allocator.resolve(self.model, offset, access, now_us, space, leases)
    }

    pub fn resolve_kv<
        const CACHES: usize,
        const SEGMENTS: usize,
        const ALLOCATIONS: usize,
        const EXTENTS: usize,
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
    >(
        self,
        token: u64,
        byte_in_token: u64,
        access: Access,
        now_us: u64,
        caches: &KvCachePool<CACHES, SEGMENTS>,
        allocator: &UnifiedAllocator<ALLOCATIONS, EXTENTS>,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &LeaseTable<LEASES>,
    ) -> Result<ModelAddress, Error> {
        caches.resolve(
            self.kv_cache,
            token,
            byte_in_token,
            access,
            now_us,
            allocator,
            space,
            leases,
        )
    }
}

/// Dual-journal request ledger for token-boundary inference recovery.
///
/// A checkpoint becomes visible only after both journal nodes acknowledge it.
/// If either node fails, the committed record and mirrored KV handle continue
/// through `MemoryDegradationHandle` without changing application identity.
pub struct InferenceLedger<const CAPACITY: usize = DEFAULT_INFERENCE_CAPACITY> {
    entries: [InferenceEntry; CAPACITY],
}

impl<const CAPACITY: usize> InferenceLedger<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            entries: [InferenceEntry::EMPTY; CAPACITY],
        }
    }

    pub fn begin(
        &mut self,
        request: RequestId,
        model: AllocationHandle,
        kv_cache: KvCacheHandle,
        primary: NodeId,
        replica: NodeId,
        rng_state: u64,
    ) -> Result<(InferenceHandle, CheckpointWrite), Error> {
        if primary == replica {
            return Err(Error::NoFailoverReplica)
        }
        if self.entries.iter().any(|entry| {
            entry.occupied
                && entry
                    .committed
                    .or(entry.pending)
                    .is_some_and(|record| record.request == request)
        }) {
            return Err(Error::InvalidHandle)
        }
        let slot = self
            .entries
            .iter()
            .position(|entry| !entry.occupied)
            .ok_or(Error::Capacity)?;
        let generation = self.entries[slot].generation.wrapping_add(1).max(1);
        let handle = InferenceHandle::from_parts(slot, generation);
        let record = RecoveryRecord::new(
            request,
            model,
            kv_cache,
            0,
            rng_state,
            1,
            primary,
            replica,
        );
        self.entries[slot] = InferenceEntry {
            occupied: true,
            generation,
            state: InferenceState::Replicating,
            committed: None,
            pending: Some(record),
            failed_nodes: 0,
        };
        Ok((handle, checkpoint_write(record)))
    }

    pub fn prepare_checkpoint(
        &mut self,
        handle: InferenceHandle,
        next_token: u64,
        rng_state: u64,
    ) -> Result<CheckpointWrite, Error> {
        let slot = self.valid_slot(handle)?;
        let entry = &mut self.entries[slot];
        if entry.pending.is_some() {
            return Err(Error::StaleCheckpoint)
        }
        let committed = entry.committed.ok_or(Error::StaleCheckpoint)?;
        if next_token < committed.next_token {
            return Err(Error::StaleCheckpoint)
        }
        let pending = RecoveryRecord::new(
            committed.request,
            committed.model,
            committed.kv_cache,
            next_token,
            rng_state,
            committed.epoch.saturating_add(1),
            committed.primary,
            committed.replica,
        );
        entry.pending = Some(pending);
        entry.state = InferenceState::Replicating;
        Ok(checkpoint_write(pending))
    }

    pub fn acknowledge_checkpoint(
        &mut self,
        handle: InferenceHandle,
        epoch: u64,
        primary_acknowledged: bool,
        replica_acknowledged: bool,
    ) -> Result<InferenceInfo, Error> {
        let slot = self.valid_slot(handle)?;
        let entry = &mut self.entries[slot];
        let pending = entry.pending.ok_or(Error::StaleCheckpoint)?;
        if pending.epoch != epoch {
            return Err(Error::StaleCheckpoint)
        }
        if !primary_acknowledged || !replica_acknowledged {
            return Err(Error::NoFailoverReplica)
        }
        entry.committed = Some(pending);
        entry.pending = None;
        entry.state = if entry.failed_nodes & node_bit(pending.primary)? != 0
            || entry.failed_nodes & node_bit(pending.replica)? != 0
        {
            InferenceState::Degraded
        } else {
            InferenceState::Running
        };
        self.info(handle)
    }

    pub fn prepare_replica_repair(
        &mut self,
        handle: InferenceHandle,
        replacement: NodeId,
    ) -> Result<CheckpointWrite, Error> {
        let slot = self.valid_slot(handle)?;
        let entry = &mut self.entries[slot];
        if entry.pending.is_some() {
            return Err(Error::StaleCheckpoint)
        }
        let committed = entry.committed.ok_or(Error::StaleCheckpoint)?;
        let primary_failed = entry.failed_nodes & node_bit(committed.primary)? != 0;
        let replica_failed = entry.failed_nodes & node_bit(committed.replica)? != 0;
        let survivor = match (primary_failed, replica_failed) {
            (false, true) | (false, false) => committed.primary,
            (true, false) => committed.replica,
            (true, true) => return Err(Error::NoFailoverReplica),
        };
        if replacement == survivor || entry.failed_nodes & node_bit(replacement)? != 0 {
            return Err(Error::NoFailoverReplica)
        }
        let pending = RecoveryRecord::new(
            committed.request,
            committed.model,
            committed.kv_cache,
            committed.next_token,
            committed.rng_state,
            committed.epoch.saturating_add(1),
            survivor,
            replacement,
        );
        entry.pending = Some(pending);
        entry.state = InferenceState::Replicating;
        Ok(checkpoint_write(pending))
    }

    pub fn fail_node(
        &mut self,
        handle: InferenceHandle,
        failed: NodeId,
    ) -> Result<MemoryDegradationHandle, Error> {
        let slot = self.valid_slot(handle)?;
        let entry = &mut self.entries[slot];
        let bit = node_bit(failed)?;
        entry.failed_nodes |= bit;
        entry.pending = None;
        let committed = entry.committed.ok_or(Error::NoFailoverReplica)?;
        let primary_failed = entry.failed_nodes & node_bit(committed.primary)? != 0;
        let replica_failed = entry.failed_nodes & node_bit(committed.replica)? != 0;
        let journal_node = match (primary_failed, replica_failed) {
            (false, _) => committed.primary,
            (true, false) => committed.replica,
            (true, true) => return Err(Error::NoFailoverReplica),
        };
        entry.state = InferenceState::Degraded;
        Ok(MemoryDegradationHandle {
            inference: handle,
            request: committed.request,
            model: committed.model,
            kv_cache: committed.kv_cache,
            next_token: committed.next_token,
            rng_state: committed.rng_state,
            committed_epoch: committed.epoch,
            journal_node,
        })
    }

    pub fn info(&self, handle: InferenceHandle) -> Result<InferenceInfo, Error> {
        let entry = &self.entries[self.valid_slot(handle)?];
        let record = entry.committed.or(entry.pending).ok_or(Error::RequestNotFound)?;
        Ok(InferenceInfo {
            handle,
            request: record.request,
            state: entry.state,
            committed_epoch: entry.committed.map_or(0, |value| value.epoch),
            next_token: entry.committed.map_or(0, |value| value.next_token),
        })
    }

    pub fn restore(
        &mut self,
        bytes: [u8; RecoveryRecord::WIRE_BYTES],
        local_journal_node: NodeId,
        failed_node: NodeId,
    ) -> Result<(InferenceHandle, MemoryDegradationHandle), Error> {
        let record = RecoveryRecord::decode(bytes)?;
        if local_journal_node != record.primary && local_journal_node != record.replica {
            return Err(Error::NoFailoverReplica)
        }
        if failed_node != record.primary && failed_node != record.replica {
            return Err(Error::InvalidRange)
        }
        if local_journal_node == failed_node {
            return Err(Error::NoFailoverReplica)
        }
        let slot = self
            .entries
            .iter()
            .position(|entry| !entry.occupied)
            .ok_or(Error::Capacity)?;
        let generation = self.entries[slot].generation.wrapping_add(1).max(1);
        let handle = InferenceHandle::from_parts(slot, generation);
        self.entries[slot] = InferenceEntry {
            occupied: true,
            generation,
            state: InferenceState::Degraded,
            committed: Some(record),
            pending: None,
            failed_nodes: node_bit(failed_node)?,
        };
        Ok((
            handle,
            MemoryDegradationHandle {
                inference: handle,
                request: record.request,
                model: record.model,
                kv_cache: record.kv_cache,
                next_token: record.next_token,
                rng_state: record.rng_state,
                committed_epoch: record.epoch,
                journal_node: local_journal_node,
            },
        ))
    }

    pub fn complete(&mut self, handle: InferenceHandle) -> Result<(), Error> {
        let slot = self.valid_slot(handle)?;
        self.entries[slot].state = InferenceState::Complete;
        self.entries[slot].occupied = false;
        Ok(())
    }

    fn valid_slot(&self, handle: InferenceHandle) -> Result<usize, Error> {
        let slot = handle.slot();
        let entry = self.entries.get(slot).ok_or(Error::InvalidHandle)?;
        if !entry.occupied || entry.generation != handle.generation() {
            return Err(Error::RequestNotFound)
        }
        Ok(slot)
    }
}

impl<const CAPACITY: usize> Default for InferenceLedger<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

fn checkpoint_write(record: RecoveryRecord) -> CheckpointWrite {
    CheckpointWrite {
        primary: record.primary,
        replica: record.replica,
        epoch: record.epoch,
        bytes: record.encode(),
    }
}

fn node_bit(node: NodeId) -> Result<u64, Error> {
    if node.raw() > 64 {
        Err(Error::Capacity)
    } else {
        Ok(1 << (node.raw() - 1))
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = crc >> 1 ^ 0xedb8_8320 & mask
        }
    }
    !crc
}
