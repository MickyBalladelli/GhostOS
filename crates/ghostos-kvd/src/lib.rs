#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::{sync::Arc, vec, vec::Vec};
use core::fmt;
use ghostos_fabric::NodeId;
use ghostos_status::{IntoStatus, Status};
use ghostos_ghostfs::{CheckpointInfo, Error as SynFsError, SynFs};

pub const KVD_STATE_PATH: &str = "SYS$SYSTEM:KVD_STATE.DAT;1";
pub const DEFAULT_TABLE_CAPACITY: usize = 256;
pub const DEFAULT_CAPABILITY_CAPACITY: usize = 64;
pub const DEFAULT_SNAPSHOT_CAPACITY: usize = 16;
pub const MAX_SCOPE_BYTES: usize = 64;
pub const MAX_RESP_ARGS: usize = 32;

const EMPTY: u8 = 0;
const OCCUPIED: u8 = 1;
const TOMBSTONE: u8 = 2;
const STATE_MAGIC: &[u8; 8] = b"SYNKVD01";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KvdError {
    AccessDenied,
    BufferTooSmall { required: usize },
    Capacity,
    InvalidCapability,
    InvalidKey,
    InvalidState,
    InvalidValue,
    NotFound,
    Protocol,
    SnapshotCapacity,
    SnapshotNotFound,
    SynFs(SynFsError),
    TransactionAborted,
}

impl From<SynFsError> for KvdError {
    fn from(error: SynFsError) -> Self {
        Self::SynFs(error)
    }
}

impl IntoStatus for KvdError {
    fn status(self) -> Status {
        match self {
            Self::AccessDenied | Self::InvalidCapability => Status::ACCESS_DENIED,
            Self::BufferTooSmall { .. } | Self::Capacity | Self::SnapshotCapacity => {
                Status::NO_SPACE
            }
            Self::NotFound | Self::SnapshotNotFound => Status::NOT_FOUND,
            Self::SynFs(error) => error.status(),
            Self::TransactionAborted => Status::BUSY,
            Self::InvalidKey
            | Self::InvalidState
            | Self::InvalidValue
            | Self::Protocol => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct CacheRights(u8);

impl CacheRights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const DELETE: Self = Self(1 << 2);
    pub const ADMIN: Self = Self(1 << 3);
    pub const ALL: Self = Self(0x0f);

    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::ALL.0 == 0 { Some(Self(bits)) } else { None }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct Keyspace {
    bytes: [u8; MAX_SCOPE_BYTES],
    len: u8,
}

impl Keyspace {
    pub const fn sys() -> Self {
        Self::from_static(b"sys/")
    }

    pub const fn job() -> Self {
        Self::from_static(b"job/")
    }

    pub const fn app() -> Self {
        Self::from_static(b"app/")
    }

    pub const fn from_static(prefix: &[u8]) -> Self {
        let mut bytes = [0; MAX_SCOPE_BYTES];
        let mut index = 0;
        while index < prefix.len() && index < MAX_SCOPE_BYTES {
            bytes[index] = prefix[index];
            index += 1;
        }
        Self { bytes, len: index as u8 }
    }

    pub fn new(prefix: &[u8]) -> Result<Self, KvdError> {
        if prefix.is_empty() || prefix.len() > MAX_SCOPE_BYTES {
            return Err(KvdError::InvalidKey)
        }
        let mut keyspace = Self::from_static(prefix);
        if !prefix.ends_with(b"/") {
            if keyspace.len as usize == MAX_SCOPE_BYTES {
                return Err(KvdError::InvalidKey)
            }
            keyspace.bytes[keyspace.len as usize] = b'/';
            keyspace.len += 1;
        }
        Ok(keyspace)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    pub fn contains(&self, key: &[u8]) -> bool {
        key.starts_with(self.as_bytes()) && key.len() > self.len as usize
    }
}

impl fmt::Debug for Keyspace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Keyspace").field(&self.as_bytes()).finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct CacheCapability(u64);

impl CacheCapability {
    const fn from_parts(slot: usize, generation: u32) -> Self {
        Self(((generation as u64) << 32) | slot as u64)
    }

    pub const fn from_raw(raw: u64) -> Option<Self> {
        if raw >> 32 == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    const fn slot(self) -> usize {
        self.0 as u32 as usize
    }

    const fn generation(self) -> u32 {
        (self.0 >> 32) as u32
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CapabilitySlot {
    generation: u32,
    active: bool,
    owner: u64,
    rights: CacheRights,
    scope: Keyspace,
}

impl CapabilitySlot {
    const EMPTY: Self = Self {
        generation: 0,
        active: false,
        owner: 0,
        rights: CacheRights::NONE,
        scope: Keyspace::from_static(b"x"),
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvictionPolicy {
    None,
    Lru,
    Lfu,
    Ttl,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoryTier {
    Local,
    Cxl(NodeId),
    Vram(NodeId),
    Layer2(NodeId),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TierBudget {
    pub tier: MemoryTier,
    pub capacity_bytes: usize,
    pub used_bytes: usize,
}

#[derive(Clone, Debug)]
pub struct MemoryPool {
    budgets: Vec<TierBudget>,
}

impl MemoryPool {
    pub fn local(capacity_bytes: usize) -> Self {
        Self {
            budgets: vec![TierBudget {
                tier: MemoryTier::Local,
                capacity_bytes,
                used_bytes: 0,
            }],
        }
    }

    pub fn add_tier(&mut self, tier: MemoryTier, capacity_bytes: usize) -> Result<(), KvdError> {
        if capacity_bytes == 0 || self.budgets.iter().any(|budget| budget.tier == tier) {
            return Err(KvdError::InvalidValue)
        }
        self.budgets.push(TierBudget { tier, capacity_bytes, used_bytes: 0 });
        Ok(())
    }

    pub fn budgets(&self) -> &[TierBudget] {
        &self.budgets
    }

    fn reserve(&mut self, bytes: usize) -> Option<MemoryTier> {
        self.budgets.iter_mut().find_map(|budget| {
            let available = budget.capacity_bytes.saturating_sub(budget.used_bytes);
            if available < bytes { return None }
            budget.used_bytes += bytes;
            Some(budget.tier)
        })
    }

    fn release(&mut self, tier: MemoryTier, bytes: usize) {
        if let Some(budget) = self.budgets.iter_mut().find(|budget| budget.tier == tier) {
            budget.used_bytes = budget.used_bytes.saturating_sub(bytes)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadHandle {
    slot: usize,
    generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotId(u64);

impl SnapshotId {
    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotInfo {
    pub id: SnapshotId,
    pub generation: u64,
    pub entries: usize,
    pub bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheStats {
    pub entries: usize,
    pub bytes: usize,
    pub capacity_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PutResult {
    pub replaced: bool,
    pub evicted: bool,
    pub bytes: usize,
    pub tier: MemoryTier,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EntryMeta {
    expires_at: Option<u64>,
    last_access: u64,
    frequency: u64,
    generation: u64,
    tier: MemoryTier,
}

#[derive(Clone, Debug)]
struct Entry {
    key: Vec<u8>,
    value: Vec<u8>,
    meta: EntryMeta,
}

#[derive(Clone, Debug)]
struct TableSlot {
    state: u8,
    entry: Option<Arc<Entry>>,
}

impl TableSlot {
    fn empty() -> Self {
        Self { state: EMPTY, entry: None }
    }
}

#[derive(Clone, Debug)]
struct Snapshot {
    info: SnapshotInfo,
    slots: Vec<TableSlot>,
}

pub struct CacheEngine<
    const CAPABILITIES: usize = DEFAULT_CAPABILITY_CAPACITY,
    const SNAPSHOTS: usize = DEFAULT_SNAPSHOT_CAPACITY,
> {
    slots: Vec<TableSlot>,
    entries: usize,
    bytes: usize,
    capacity_bytes: usize,
    policy: EvictionPolicy,
    now: u64,
    next_generation: u64,
    next_snapshot: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
    capabilities: [CapabilitySlot; CAPABILITIES],
    snapshots: [Option<Snapshot>; SNAPSHOTS],
    memory: MemoryPool,
}

impl<const CAPABILITIES: usize, const SNAPSHOTS: usize>
    CacheEngine<CAPABILITIES, SNAPSHOTS>
{
    pub fn new(capacity_bytes: usize) -> Result<Self, KvdError> {
        Self::with_capacity(DEFAULT_TABLE_CAPACITY, capacity_bytes, EvictionPolicy::Lru)
    }

    pub fn with_capacity(
        table_capacity: usize,
        capacity_bytes: usize,
        policy: EvictionPolicy,
    ) -> Result<Self, KvdError> {
        if table_capacity < 2 || capacity_bytes == 0 {
            return Err(KvdError::InvalidValue)
        }
        let mut slots = Vec::with_capacity(table_capacity);
        for _ in 0..table_capacity { slots.push(TableSlot::empty()) }
        Ok(Self {
            slots,
            entries: 0,
            bytes: 0,
            capacity_bytes,
            policy,
            now: 0,
            next_generation: 1,
            next_snapshot: 1,
            hits: 0,
            misses: 0,
            evictions: 0,
            capabilities: [CapabilitySlot::EMPTY; CAPABILITIES],
            snapshots: [const { None }; SNAPSHOTS],
            memory: MemoryPool::local(capacity_bytes),
        })
    }

    pub fn memory_pool(&self) -> &MemoryPool {
        &self.memory
    }

    pub fn set_memory_pool(&mut self, mut memory: MemoryPool) -> Result<(), KvdError> {
        if memory.budgets.iter().map(|budget| budget.capacity_bytes).sum::<usize>()
            < self.bytes
        {
            return Err(KvdError::Capacity)
        }
        for slot in &self.slots {
            if let Some(entry) = &slot.entry {
                let Some(_) = memory.reserve(entry.value.len()) else {
                    return Err(KvdError::Capacity)
                };
            }
        }
        self.memory = memory;
        Ok(())
    }

    pub fn set_policy(&mut self, policy: EvictionPolicy) {
        self.policy = policy
    }

    /// Evict entries until at least `required_bytes` are free.
    ///
    /// The memory manager can call this when a microkernel pressure event
    /// arrives, before allocating another shared page.
    pub fn memory_pressure(&mut self, required_bytes: usize, now: u64) -> usize {
        self.evict_expired(now);
        let before = self.evictions;
        while self.capacity_bytes.saturating_sub(self.bytes) < required_bytes {
            if !self.evict_one(None) { break }
        }
        (self.evictions - before) as usize
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            entries: self.entries,
            bytes: self.bytes,
            capacity_bytes: self.capacity_bytes,
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            generation: self.next_generation,
        }
    }

    pub fn issue_capability(
        &mut self,
        owner: u64,
        scope: Keyspace,
        rights: CacheRights,
    ) -> Result<CacheCapability, KvdError> {
        if owner == 0 || rights.is_empty() {
            return Err(KvdError::InvalidValue)
        }
        let (index, slot) = self
            .capabilities
            .iter_mut()
            .enumerate()
            .find(|(_, slot)| !slot.active)
            .ok_or(KvdError::Capacity)?;
        slot.generation = slot.generation.wrapping_add(1).max(1);
        slot.active = true;
        slot.owner = owner;
        slot.rights = rights;
        slot.scope = scope;
        Ok(CacheCapability::from_parts(index, slot.generation))
    }

    pub fn delegate_capability(
        &mut self,
        owner: u64,
        parent: CacheCapability,
        child_owner: u64,
        rights: CacheRights,
        scope: Keyspace,
    ) -> Result<CacheCapability, KvdError> {
        let parent_slot = self.authorize_capability(owner, parent, CacheRights::ADMIN, None)?;
        if child_owner == 0
            || rights.is_empty()
            || !parent_slot.rights.contains(rights)
            || !parent_slot.scope.contains(scope.as_bytes())
        {
            return Err(KvdError::AccessDenied)
        }
        self.issue_capability(child_owner, scope, rights)
    }

    pub fn revoke_capability(
        &mut self,
        owner: u64,
        authority: CacheCapability,
        target: CacheCapability,
    ) -> Result<(), KvdError> {
        self.authorize_capability(owner, authority, CacheRights::ADMIN, None)?;
        let slot = self.capability_slot_mut(target)?;
        slot.active = false;
        Ok(())
    }

    pub fn put(
        &mut self,
        owner: u64,
        capability: CacheCapability,
        key: &[u8],
        value: &[u8],
        now: u64,
        ttl_us: Option<u64>,
    ) -> Result<PutResult, KvdError> {
        self.authorize_capability(owner, capability, CacheRights::WRITE, Some(key))?;
        validate_key_value(key, value)?;
        self.now = now;
        self.evict_expired(now);
        let expires_at = match ttl_us {
            Some(ttl) => Some(now.checked_add(ttl).ok_or(KvdError::InvalidValue)?),
            None => None,
        };
        if ttl_us == Some(0) { return Err(KvdError::InvalidValue) }
        let existing = self.find_key(key);
        let old_bytes = existing
            .and_then(|index| self.slots[index].entry.as_ref())
            .map_or(0, |entry| entry.value.len());
        let required = value.len().saturating_sub(old_bytes);
        let mut evicted = false;
        while self.bytes.saturating_add(required) > self.capacity_bytes {
            if !self.evict_one(existing) { return Err(KvdError::Capacity) }
            evicted = true;
        }
        if existing.is_none() && self.slots.iter().all(|slot| slot.state == OCCUPIED) {
            if !self.evict_one(None) { return Err(KvdError::Capacity) }
            evicted = true;
        }
        let tier = if let Some(index) = existing {
            let old = self.slots[index].entry.as_ref().ok_or(KvdError::InvalidState)?;
            let tier = old.meta.tier;
            let old_length = old.value.len();
            self.memory.release(tier, old_length);
            let Some(tier) = self.memory.reserve(value.len()) else {
                let _ = self.memory.reserve(old_length);
                return Err(KvdError::Capacity)
            };
            tier
        } else {
            self.memory.reserve(value.len()).ok_or(KvdError::Capacity)?
        };
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        let entry = Arc::new(Entry {
            key: key.to_vec(),
            value: value.to_vec(),
            meta: EntryMeta {
                expires_at,
                last_access: now,
                frequency: 1,
                generation,
                tier,
            },
        });
        let index = existing.unwrap_or_else(|| self.insertion_slot(key));
        if self.slots[index].state != OCCUPIED {
            self.entries += 1;
        } else if let Some(old) = self.slots[index].entry.take() {
            self.bytes = self.bytes.saturating_sub(old.value.len());
        }
        self.slots[index] = TableSlot { state: OCCUPIED, entry: Some(entry) };
        self.bytes += value.len();
        Ok(PutResult { replaced: existing.is_some(), evicted, bytes: value.len(), tier })
    }

    pub fn get(
        &mut self,
        owner: u64,
        capability: CacheCapability,
        key: &[u8],
        now: u64,
    ) -> Result<ReadHandle, KvdError> {
        self.authorize_capability(owner, capability, CacheRights::READ, Some(key))?;
        self.now = now;
        self.evict_expired(now);
        let Some(index) = self.find_key(key) else {
            self.misses = self.misses.saturating_add(1);
            return Err(KvdError::NotFound)
        };
        let old = self.slots[index].entry.as_ref().ok_or(KvdError::InvalidState)?;
        let mut updated = (**old).clone();
        updated.meta.last_access = now;
        updated.meta.frequency = updated.meta.frequency.saturating_add(1);
        let generation = updated.meta.generation;
        self.slots[index].entry = Some(Arc::new(updated));
        self.hits = self.hits.saturating_add(1);
        Ok(ReadHandle { slot: index, generation })
    }

    pub fn read(&self, handle: ReadHandle) -> Result<&[u8], KvdError> {
        let slot = self.slots.get(handle.slot).ok_or(KvdError::InvalidCapability)?;
        let entry = slot.entry.as_ref().ok_or(KvdError::InvalidCapability)?;
        if slot.state != OCCUPIED || entry.meta.generation != handle.generation {
            return Err(KvdError::InvalidCapability)
        }
        Ok(entry.value.as_slice())
    }

    pub fn delete(
        &mut self,
        owner: u64,
        capability: CacheCapability,
        key: &[u8],
        now: u64,
    ) -> Result<bool, KvdError> {
        self.authorize_capability(owner, capability, CacheRights::DELETE, Some(key))?;
        self.now = now;
        let Some(index) = self.find_key(key) else { return Ok(false) };
        self.remove_at(index);
        Ok(true)
    }

    pub fn contains(
        &mut self,
        owner: u64,
        capability: CacheCapability,
        key: &[u8],
        now: u64,
    ) -> Result<bool, KvdError> {
        self.authorize_capability(owner, capability, CacheRights::READ, Some(key))?;
        self.now = now;
        self.evict_expired(now);
        Ok(self.find_key(key).is_some())
    }

    pub fn expire(
        &mut self,
        owner: u64,
        capability: CacheCapability,
        key: &[u8],
        now: u64,
        ttl_us: u64,
    ) -> Result<bool, KvdError> {
        self.authorize_capability(owner, capability, CacheRights::WRITE, Some(key))?;
        let Some(index) = self.find_key(key) else { return Ok(false) };
        let entry = self.slots[index].entry.as_ref().ok_or(KvdError::InvalidState)?;
        let mut updated = (**entry).clone();
        updated.meta.expires_at = Some(now.checked_add(ttl_us).ok_or(KvdError::InvalidValue)?);
        self.slots[index].entry = Some(Arc::new(updated));
        Ok(true)
    }

    pub fn ttl_us(
        &mut self,
        owner: u64,
        capability: CacheCapability,
        key: &[u8],
        now: u64,
    ) -> Result<Option<u64>, KvdError> {
        self.authorize_capability(owner, capability, CacheRights::READ, Some(key))?;
        self.evict_expired(now);
        let Some(index) = self.find_key(key) else { return Err(KvdError::NotFound) };
        let entry = self.slots[index].entry.as_ref().ok_or(KvdError::InvalidState)?;
        Ok(entry.meta.expires_at.map(|expiry| expiry.saturating_sub(now)))
    }

    pub fn evict_expired(&mut self, now: u64) -> usize {
        self.now = now;
        let mut removed = 0;
        for index in 0..self.slots.len() {
            let expired = self.slots[index]
                .entry
                .as_ref()
                .and_then(|entry| entry.meta.expires_at)
                .is_some_and(|expiry| expiry <= now);
            if expired {
                self.remove_at(index);
                self.evictions = self.evictions.saturating_add(1);
                removed += 1;
            }
        }
        removed
    }

    pub fn snapshot(&mut self) -> Result<SnapshotInfo, KvdError> {
        let slot = self
            .snapshots
            .iter_mut()
            .find(|snapshot| snapshot.is_none())
            .ok_or(KvdError::SnapshotCapacity)?;
        let id = SnapshotId(self.next_snapshot);
        self.next_snapshot = self.next_snapshot.wrapping_add(1).max(1);
        let info = SnapshotInfo {
            id,
            generation: self.next_generation,
            entries: self.entries,
            bytes: self.bytes,
        };
        *slot = Some(Snapshot { info, slots: self.slots.clone() });
        Ok(info)
    }

    pub fn restore(&mut self, id: SnapshotId) -> Result<SnapshotInfo, KvdError> {
        let snapshot = self
            .snapshots
            .iter()
            .flatten()
            .find(|snapshot| snapshot.info.id == id)
            .ok_or(KvdError::SnapshotNotFound)?
            .clone();
        self.release_all_memory();
        self.slots = snapshot.slots;
        self.entries = snapshot.info.entries;
        self.bytes = snapshot.info.bytes;
        for slot in &self.slots {
            if let Some(entry) = &slot.entry {
                self.memory.reserve(entry.value.len()).ok_or(KvdError::Capacity)?;
            }
        }
        Ok(snapshot.info)
    }

    pub fn release_snapshot(&mut self, id: SnapshotId) -> Result<(), KvdError> {
        let slot = self
            .snapshots
            .iter_mut()
            .find(|snapshot| snapshot.as_ref().is_some_and(|snapshot| snapshot.info.id == id))
            .ok_or(KvdError::SnapshotNotFound)?;
        *slot = None;
        Ok(())
    }

    pub fn transaction(&mut self) -> Result<Transaction<'_, CAPABILITIES, SNAPSHOTS>, KvdError> {
        let snapshot = self.snapshot()?.id;
        Ok(Transaction { engine: self, snapshot, committed: false })
    }

    pub fn checkpoint<const BLOCKS: usize>(
        &mut self,
        filesystem: &mut SynFs<BLOCKS>,
    ) -> Result<CheckpointInfo, KvdError> {
        let mut image = Vec::new();
        self.encode_state(&mut image)?;
        filesystem.write(KVD_STATE_PATH, &image)?;
        Ok(filesystem.create_checkpoint()?)
    }

    pub fn load_from_ghostfs<const BLOCKS: usize>(
        &mut self,
        filesystem: &SynFs<BLOCKS>,
        image: &mut [u8],
    ) -> Result<usize, KvdError> {
        let read = filesystem.read(KVD_STATE_PATH, image)?;
        self.decode_state(&image[..read.bytes_read])?;
        Ok(read.bytes_read)
    }

    pub fn redis_gateway<'engine>(
        &'engine mut self,
        owner: u64,
        capability: CacheCapability,
    ) -> RedisGateway<'engine, CAPABILITIES, SNAPSHOTS> {
        RedisGateway { engine: self, owner, capability }
    }

    fn authorize_capability(
        &self,
        owner: u64,
        capability: CacheCapability,
        required: CacheRights,
        key: Option<&[u8]>,
    ) -> Result<&CapabilitySlot, KvdError> {
        let slot = self
            .capabilities
            .get(capability.slot())
            .ok_or(KvdError::InvalidCapability)?;
        if !slot.active
            || slot.generation != capability.generation()
            || slot.owner != owner
            || !slot.rights.contains(required)
            || key.is_some_and(|key| !slot.scope.contains(key))
        {
            return Err(if slot.active { KvdError::AccessDenied } else { KvdError::InvalidCapability })
        }
        Ok(slot)
    }

    fn capability_slot_mut(&mut self, capability: CacheCapability) -> Result<&mut CapabilitySlot, KvdError> {
        let slot = self.capabilities.get_mut(capability.slot()).ok_or(KvdError::InvalidCapability)?;
        if !slot.active || slot.generation != capability.generation() {
            return Err(KvdError::InvalidCapability)
        }
        Ok(slot)
    }

    fn find_key(&self, key: &[u8]) -> Option<usize> {
        let start = hash(key) as usize % self.slots.len();
        for step in 0..self.slots.len() {
            let index = (start + step) % self.slots.len();
            let slot = &self.slots[index];
            if slot.state == EMPTY { return None }
            if slot.state == OCCUPIED && slot.entry.as_ref().is_some_and(|entry| entry.key == key) {
                return Some(index)
            }
        }
        None
    }

    fn insertion_slot(&self, key: &[u8]) -> usize {
        let start = hash(key) as usize % self.slots.len();
        let mut tombstone = None;
        for step in 0..self.slots.len() {
            let index = (start + step) % self.slots.len();
            match self.slots[index].state {
                EMPTY => return tombstone.unwrap_or(index),
                TOMBSTONE if tombstone.is_none() => tombstone = Some(index),
                _ => {}
            }
        }
        tombstone.unwrap_or(0)
    }

    fn evict_one(&mut self, protected: Option<usize>) -> bool {
        let mut victim: Option<usize> = None;
        for (index, slot) in self.slots.iter().enumerate() {
            if Some(index) == protected || slot.state != OCCUPIED { continue }
            let entry = match &slot.entry { Some(entry) => entry, None => continue };
            let better = victim.is_none_or(|old| {
                let old_entry = self.slots[old].entry.as_ref().expect("occupied entry");
                match self.policy {
                    EvictionPolicy::Lru => entry.meta.last_access < old_entry.meta.last_access,
                    EvictionPolicy::Lfu => entry.meta.frequency < old_entry.meta.frequency,
                    EvictionPolicy::Ttl => entry.meta.expires_at.unwrap_or(u64::MAX)
                        < old_entry.meta.expires_at.unwrap_or(u64::MAX),
                    EvictionPolicy::None => false,
                }
            });
            if better { victim = Some(index) }
        }
        let Some(index) = victim else { return false };
        self.remove_at(index);
        self.evictions = self.evictions.saturating_add(1);
        true
    }

    fn remove_at(&mut self, index: usize) {
        if let Some(entry) = self.slots[index].entry.take() {
            self.bytes = self.bytes.saturating_sub(entry.value.len());
            self.memory.release(entry.meta.tier, entry.value.len());
            self.entries = self.entries.saturating_sub(1);
            self.slots[index].state = TOMBSTONE;
        }
    }

    fn release_all_memory(&mut self) {
        for budget in &mut self.memory.budgets { budget.used_bytes = 0 }
    }

    fn encode_state(&self, output: &mut Vec<u8>) -> Result<(), KvdError> {
        output.extend_from_slice(STATE_MAGIC);
        push_u64(output, self.entries as u64);
        for slot in &self.slots {
            let Some(entry) = &slot.entry else { continue };
            push_u32(output, entry.key.len() as u32);
            push_u32(output, entry.value.len() as u32);
            push_u64(output, entry.meta.expires_at.unwrap_or(0));
            output.extend_from_slice(&entry.key);
            output.extend_from_slice(&entry.value);
        }
        Ok(())
    }

    fn decode_state(&mut self, input: &[u8]) -> Result<(), KvdError> {
        if input.len() < 16 || &input[..8] != STATE_MAGIC {
            return Err(KvdError::InvalidState)
        }
        let mut cursor = 8;
        let count = read_u64(input, &mut cursor)? as usize;
        self.release_all_memory();
        self.slots.fill(TableSlot::empty());
        self.entries = 0;
        self.bytes = 0;
        for _ in 0..count {
            let key_len = read_u32(input, &mut cursor)? as usize;
            let value_len = read_u32(input, &mut cursor)? as usize;
            let expiry = read_u64(input, &mut cursor)?;
            let key = take(input, &mut cursor, key_len)?;
            let value = take(input, &mut cursor, value_len)?;
            if self.find_key(key).is_some() { return Err(KvdError::InvalidState) }
            if self.bytes + value.len() > self.capacity_bytes { return Err(KvdError::Capacity) }
            let tier = self.memory.reserve(value.len()).ok_or(KvdError::Capacity)?;
            let generation = self.next_generation;
            self.next_generation = self.next_generation.wrapping_add(1).max(1);
            let index = self.insertion_slot(key);
            self.slots[index] = TableSlot {
                state: OCCUPIED,
                entry: Some(Arc::new(Entry {
                    key: key.to_vec(),
                    value: value.to_vec(),
                    meta: EntryMeta {
                        expires_at: (expiry != 0).then_some(expiry),
                        last_access: self.now,
                        frequency: 1,
                        generation,
                        tier,
                    },
                })),
            };
            self.entries += 1;
            self.bytes += value.len();
        }
        if cursor != input.len() { return Err(KvdError::InvalidState) }
        Ok(())
    }
}

impl<const CAPABILITIES: usize, const SNAPSHOTS: usize> Default
    for CacheEngine<CAPABILITIES, SNAPSHOTS>
{
    fn default() -> Self {
        Self::new(1024 * 1024).expect("valid default KVD configuration")
    }
}

pub struct Transaction<'engine, const CAPABILITIES: usize, const SNAPSHOTS: usize> {
    engine: &'engine mut CacheEngine<CAPABILITIES, SNAPSHOTS>,
    snapshot: SnapshotId,
    committed: bool,
}

impl<const CAPABILITIES: usize, const SNAPSHOTS: usize>
    Transaction<'_, CAPABILITIES, SNAPSHOTS>
{
    pub fn put(
        &mut self,
        owner: u64,
        capability: CacheCapability,
        key: &[u8],
        value: &[u8],
        now: u64,
        ttl_us: Option<u64>,
    ) -> Result<PutResult, KvdError> {
        self.engine.put(owner, capability, key, value, now, ttl_us)
    }

    pub fn delete(
        &mut self,
        owner: u64,
        capability: CacheCapability,
        key: &[u8],
        now: u64,
    ) -> Result<bool, KvdError> {
        self.engine.delete(owner, capability, key, now)
    }

    pub fn commit(mut self) -> Result<(), KvdError> {
        self.engine.release_snapshot(self.snapshot)?;
        self.committed = true;
        Ok(())
    }

    pub fn abort(mut self) -> Result<(), KvdError> {
        self.engine.restore(self.snapshot)?;
        self.engine.release_snapshot(self.snapshot)?;
        self.committed = true;
        Ok(())
    }
}

impl<const CAPABILITIES: usize, const SNAPSHOTS: usize> Drop
    for Transaction<'_, CAPABILITIES, SNAPSHOTS>
{
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.engine.restore(self.snapshot);
            let _ = self.engine.release_snapshot(self.snapshot);
        }
    }
}

pub struct RedisGateway<'engine, const CAPABILITIES: usize, const SNAPSHOTS: usize> {
    engine: &'engine mut CacheEngine<CAPABILITIES, SNAPSHOTS>,
    owner: u64,
    capability: CacheCapability,
}

impl<const CAPABILITIES: usize, const SNAPSHOTS: usize>
    RedisGateway<'_, CAPABILITIES, SNAPSHOTS>
{
    pub fn execute(
        &mut self,
        request: &[u8],
        now: u64,
        response: &mut [u8],
    ) -> Result<usize, KvdError> {
        let mut args = [None; MAX_RESP_ARGS];
        let count = parse_resp_command(request, &mut args)?;
        if count == 0 { return Err(KvdError::Protocol) }
        let command = args[0].ok_or(KvdError::Protocol)?;
        if command.eq_ignore_ascii_case(b"PING") {
            return write_simple(response, b"PONG");
        }
        if command.eq_ignore_ascii_case(b"GET") && count == 2 {
            let key = args[1].ok_or(KvdError::Protocol)?;
            return match self.engine.get(self.owner, self.capability, key, now) {
                Ok(handle) => write_bulk(response, self.engine.read(handle)?),
                Err(KvdError::NotFound) => write_null(response),
                Err(error) => Err(error),
            }
        }
        if command.eq_ignore_ascii_case(b"SET") && count >= 3 {
            let key = args[1].ok_or(KvdError::Protocol)?;
            let value = args[2].ok_or(KvdError::Protocol)?;
            let mut ttl = None;
            let mut only_if_absent = false;
            let mut only_if_present = false;
            let mut index = 3;
            while index < count {
                let option = args[index].ok_or(KvdError::Protocol)?;
                if index + 1 < count && option.eq_ignore_ascii_case(b"EX") {
                    ttl = Some(parse_u64(args[index + 1].ok_or(KvdError::Protocol)?)?.checked_mul(1_000_000).ok_or(KvdError::InvalidValue)?);
                    index += 2;
                } else if index + 1 < count && option.eq_ignore_ascii_case(b"PX") {
                    ttl = Some(parse_u64(args[index + 1].ok_or(KvdError::Protocol)?)?.checked_mul(1_000).ok_or(KvdError::InvalidValue)?);
                    index += 2;
                } else if option.eq_ignore_ascii_case(b"NX") {
                    if only_if_present { return Err(KvdError::Protocol) }
                    only_if_absent = true;
                    index += 1;
                } else if option.eq_ignore_ascii_case(b"XX") {
                    if only_if_absent { return Err(KvdError::Protocol) }
                    only_if_present = true;
                    index += 1;
                } else {
                    return Err(KvdError::Protocol)
                }
            }
            self.engine.evict_expired(now);
            let exists = self.engine.find_key(key).is_some();
            if (only_if_absent && exists) || (only_if_present && !exists) {
                return write_null(response)
            }
            self.engine.put(self.owner, self.capability, key, value, now, ttl)?;
            return write_simple(response, b"OK");
        }
        if command.eq_ignore_ascii_case(b"DEL") && count >= 2 {
            let mut deleted = 0u64;
            for key in args[1..count].iter().flatten() {
                if self.engine.delete(self.owner, self.capability, key, now)? { deleted += 1 }
            }
            return write_integer(response, deleted);
        }
        if command.eq_ignore_ascii_case(b"EXISTS") && count == 2 {
            let exists = self.engine.contains(self.owner, self.capability, args[1].ok_or(KvdError::Protocol)?, now)?;
            return write_integer(response, u64::from(exists));
        }
        if command.eq_ignore_ascii_case(b"EXPIRE") && count == 3 {
            let ttl = parse_u64(args[2].ok_or(KvdError::Protocol)?)?;
            let changed = self.engine.expire(self.owner, self.capability, args[1].ok_or(KvdError::Protocol)?, now, ttl.saturating_mul(1_000_000))?;
            return write_integer(response, u64::from(changed));
        }
        if command.eq_ignore_ascii_case(b"TTL") && count == 2 {
            let key = args[1].ok_or(KvdError::Protocol)?;
            return match self.engine.ttl_us(self.owner, self.capability, key, now) {
                Ok(Some(ttl)) => write_signed_integer(response, (ttl / 1_000_000) as i64),
                Ok(None) => write_signed_integer(response, -1),
                Err(KvdError::NotFound) => write_signed_integer(response, -2),
                Err(error) => Err(error),
            }
        }
        if command.eq_ignore_ascii_case(b"DBSIZE") && count == 1 {
            return write_integer(response, self.engine.stats().entries as u64);
        }
        Err(KvdError::Protocol)
    }
}

fn validate_key_value(key: &[u8], value: &[u8]) -> Result<(), KvdError> {
    if key.is_empty() || key.len() > u32::MAX as usize { return Err(KvdError::InvalidKey) }
    if value.len() > u32::MAX as usize { return Err(KvdError::InvalidValue) }
    Ok(())
}

fn hash(bytes: &[u8]) -> u64 {
    let mut value = 0xcbf29ce484222325_u64;
    for byte in bytes {
        value ^= *byte as u64;
        value = value.wrapping_mul(0x100000001b3);
    }
    value
}

fn push_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes())
}

fn push_u64(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes())
}

fn read_u32(input: &[u8], cursor: &mut usize) -> Result<u32, KvdError> {
    let bytes = take(input, cursor, 4)?;
    Ok(u32::from_le_bytes(bytes.try_into().map_err(|_| KvdError::InvalidState)?))
}

fn read_u64(input: &[u8], cursor: &mut usize) -> Result<u64, KvdError> {
    let bytes = take(input, cursor, 8)?;
    Ok(u64::from_le_bytes(bytes.try_into().map_err(|_| KvdError::InvalidState)?))
}

fn take<'a>(input: &'a [u8], cursor: &mut usize, length: usize) -> Result<&'a [u8], KvdError> {
    let end = cursor.checked_add(length).ok_or(KvdError::InvalidState)?;
    let bytes = input.get(*cursor..end).ok_or(KvdError::InvalidState)?;
    *cursor = end;
    Ok(bytes)
}

fn parse_resp_command<'a>(
    input: &'a [u8],
    output: &mut [Option<&'a [u8]>; MAX_RESP_ARGS],
) -> Result<usize, KvdError> {
    if input.first() != Some(&b'*') { return Err(KvdError::Protocol) }
    let mut cursor = 1;
    let count = parse_line_u64(input, &mut cursor)? as usize;
    if count == 0 || count > output.len() { return Err(KvdError::Protocol) }
    for item in output.iter_mut().take(count) {
        if take(input, &mut cursor, 1)? != b"$" { return Err(KvdError::Protocol) }
        let length = parse_line_u64(input, &mut cursor)? as usize;
        *item = Some(take(input, &mut cursor, length)?);
        if take(input, &mut cursor, 2)? != b"\r\n" { return Err(KvdError::Protocol) }
    }
    if cursor != input.len() { return Err(KvdError::Protocol) }
    Ok(count)
}

fn parse_line_u64(input: &[u8], cursor: &mut usize) -> Result<u64, KvdError> {
    let start = *cursor;
    while *cursor + 1 < input.len() && input[*cursor..*cursor + 2] != *b"\r\n" { *cursor += 1 }
    if *cursor + 1 >= input.len() { return Err(KvdError::Protocol) }
    let value = parse_u64(&input[start..*cursor])?;
    *cursor += 2;
    Ok(value)
}

fn parse_u64(input: &[u8]) -> Result<u64, KvdError> {
    if input.is_empty() { return Err(KvdError::Protocol) }
    let mut value = 0u64;
    for byte in input {
        if !byte.is_ascii_digit() { return Err(KvdError::Protocol) }
        value = value.checked_mul(10).and_then(|value| value.checked_add((byte - b'0') as u64)).ok_or(KvdError::InvalidValue)?;
    }
    Ok(value)
}

fn write_simple(output: &mut [u8], value: &[u8]) -> Result<usize, KvdError> {
    let required = value.len() + 3;
    if output.len() < required { return Err(KvdError::BufferTooSmall { required }) }
    output[0] = b'+';
    output[1..1 + value.len()].copy_from_slice(value);
    output[1 + value.len()..required].copy_from_slice(b"\r\n");
    Ok(required)
}

fn write_null(output: &mut [u8]) -> Result<usize, KvdError> {
    if output.len() < 5 { return Err(KvdError::BufferTooSmall { required: 5 }) }
    output[..5].copy_from_slice(b"$-1\r\n");
    Ok(5)
}

fn write_bulk(output: &mut [u8], value: &[u8]) -> Result<usize, KvdError> {
    let digits = decimal_len(value.len() as u64);
    let required = 1 + digits + 2 + value.len() + 2;
    if output.len() < required { return Err(KvdError::BufferTooSmall { required }) }
    output[0] = b'$';
    write_decimal(&mut output[1..1 + digits], value.len() as u64);
    output[1 + digits..1 + digits + 2].copy_from_slice(b"\r\n");
    let start = 1 + digits + 2;
    output[start..start + value.len()].copy_from_slice(value);
    output[start + value.len()..required].copy_from_slice(b"\r\n");
    Ok(required)
}

fn write_integer(output: &mut [u8], value: u64) -> Result<usize, KvdError> {
    write_signed_integer(output, value as i64)
}

fn write_signed_integer(output: &mut [u8], value: i64) -> Result<usize, KvdError> {
    let negative = value < 0;
    let magnitude = value.unsigned_abs();
    let digits = decimal_len(magnitude);
    let required = 1 + usize::from(negative) + digits + 2;
    if output.len() < required { return Err(KvdError::BufferTooSmall { required }) }
    output[0] = b':';
    let start = 1 + usize::from(negative);
    if negative { output[1] = b'-' }
    write_decimal(&mut output[start..start + digits], magnitude);
    output[start + digits..required].copy_from_slice(b"\r\n");
    Ok(required)
}

fn decimal_len(mut value: u64) -> usize {
    let mut length = 1;
    while value >= 10 { value /= 10; length += 1 }
    length
}

fn write_decimal(output: &mut [u8], mut value: u64) {
    for index in (0..output.len()).rev() {
        output[index] = b'0' + (value % 10) as u8;
        value /= 10;
    }
}
