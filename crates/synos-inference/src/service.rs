use synos_fabric::{
    NodeId,
    memory::{GlobalAddressSpace, LeaseTable, MemoryKind},
};
use synos_llm::{
    RequestId,
    allocator::{AllocationHandle, UnifiedAllocator},
    inference::{
        CheckpointWrite, InferenceHandle, InferenceInfo, InferenceLedger, MemoryDegradationHandle,
    },
    kv_cache::{KvCacheHandle, KvCacheInfo, KvCachePool},
};

use crate::{Error, protocol::ModelName};

pub const DEFAULT_MODEL_CAPACITY: usize = 32;
pub const DEFAULT_ACTIVE_CAPACITY: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ModelRegistration {
    pub name: ModelName,
    pub allocation: AllocationHandle,
    pub compute_node: NodeId,
    pub primary_journal: NodeId,
    pub replica_journal: NodeId,
    pub kv_memory: MemoryKind,
    pub bytes_per_token: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ExecutionHandle(u64);

impl ExecutionHandle {
    const fn from_parts(slot: usize, generation: u32) -> Self {
        Self((generation as u64) << 32 | slot as u64)
    }

    pub const fn from_raw(raw: u64) -> Option<Self> {
        if raw >> 32 == 0 {
            None
        } else {
            Some(Self(raw))
        }
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
pub struct ExecutionInfo {
    pub handle: ExecutionHandle,
    pub request: RequestId,
    pub model: ModelName,
    pub inference: InferenceInfo,
    pub kv_cache: KvCacheInfo,
    pub prompt_tokens: u64,
    pub max_generated_tokens: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionStart {
    pub info: ExecutionInfo,
    pub checkpoint: CheckpointWrite,
}

#[derive(Clone, Copy)]
struct ExecutionEntry {
    occupied: bool,
    generation: u32,
    request: Option<RequestId>,
    model_slot: u16,
    inference: Option<InferenceHandle>,
    kv_cache: Option<KvCacheHandle>,
    prompt_tokens: u64,
    max_generated_tokens: u64,
}

impl ExecutionEntry {
    const EMPTY: Self = Self {
        occupied: false,
        generation: 0,
        request: None,
        model_slot: 0,
        inference: None,
        kv_cache: None,
        prompt_tokens: 0,
        max_generated_tokens: 0,
    };
}

/// Request lifecycle above `synos-llm`.
///
/// Models name pooled RAM/VRAM allocations. Starting a request reserves a
/// mirrored, growable KV cache in the model-selected memory tier and creates a
/// dual-journal recovery record before model code receives the execution.
pub struct ClusterInferenceService<
    const MODELS: usize = DEFAULT_MODEL_CAPACITY,
    const ACTIVE: usize = DEFAULT_ACTIVE_CAPACITY,
    const CACHES: usize = DEFAULT_ACTIVE_CAPACITY,
    const SEGMENTS: usize = 32,
    const LEDGER: usize = DEFAULT_ACTIVE_CAPACITY,
> {
    models: [Option<ModelRegistration>; MODELS],
    executions: [ExecutionEntry; ACTIVE],
    caches: KvCachePool<CACHES, SEGMENTS>,
    ledger: InferenceLedger<LEDGER>,
}

impl<
    const MODELS: usize,
    const ACTIVE: usize,
    const CACHES: usize,
    const SEGMENTS: usize,
    const LEDGER: usize,
> ClusterInferenceService<MODELS, ACTIVE, CACHES, SEGMENTS, LEDGER>
{
    pub const fn new() -> Self {
        Self {
            models: [None; MODELS],
            executions: [ExecutionEntry::EMPTY; ACTIVE],
            caches: KvCachePool::new(),
            ledger: InferenceLedger::new(),
        }
    }

    pub fn register_model<const ALLOCATIONS: usize, const EXTENTS: usize>(
        &mut self,
        model: ModelRegistration,
        allocator: &UnifiedAllocator<ALLOCATIONS, EXTENTS>,
    ) -> Result<(), Error> {
        if model.bytes_per_token == 0
            || model.primary_journal == model.replica_journal
            || self
                .models
                .iter()
                .flatten()
                .any(|registered| registered.name == model.name)
        {
            return if self
                .models
                .iter()
                .flatten()
                .any(|registered| registered.name == model.name)
            {
                Err(Error::DuplicateModel)
            } else {
                Err(Error::InvalidRequest)
            };
        }
        allocator.info(model.allocation)?;
        let slot = self
            .models
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(Error::Capacity)?;
        *slot = Some(model);
        Ok(())
    }

    pub fn unregister_model(&mut self, name: &str) -> Result<(), Error> {
        let model_slot = self.model_slot(name)?;
        if self
            .executions
            .iter()
            .any(|entry| entry.occupied && entry.model_slot as usize == model_slot)
        {
            return Err(Error::Capacity);
        }
        self.models[model_slot] = None;
        Ok(())
    }

    pub fn model_count(&self) -> usize {
        self.models.iter().flatten().count()
    }

    pub fn model_at(&self, ordinal: usize) -> Option<ModelRegistration> {
        self.models.iter().flatten().nth(ordinal).copied()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start<
        const ALLOCATIONS: usize,
        const EXTENTS: usize,
        const POOLS: usize,
        const OVERRIDES: usize,
        const LEASES: usize,
    >(
        &mut self,
        request: RequestId,
        model_name: &str,
        prompt_tokens: u64,
        max_generated_tokens: u64,
        rng_state: u64,
        now_us: u64,
        lease_duration_us: u64,
        allocator: &mut UnifiedAllocator<ALLOCATIONS, EXTENTS>,
        space: &GlobalAddressSpace<POOLS, OVERRIDES>,
        leases: &mut LeaseTable<LEASES>,
    ) -> Result<ExecutionStart, Error> {
        if prompt_tokens == 0 || max_generated_tokens == 0 || lease_duration_us == 0 {
            return Err(Error::InvalidRequest);
        }
        if self
            .executions
            .iter()
            .any(|entry| entry.occupied && entry.request == Some(request))
        {
            return Err(Error::InvalidRequest);
        }
        let total_tokens = prompt_tokens
            .checked_add(max_generated_tokens)
            .ok_or(Error::InvalidRequest)?;
        let model_slot = self.model_slot(model_name)?;
        let model = self.models[model_slot].ok_or(Error::ModelNotFound)?;
        let execution_slot = self
            .executions
            .iter()
            .position(|entry| !entry.occupied)
            .ok_or(Error::Capacity)?;
        let cache = self.caches.open_in_memory(
            request,
            model.compute_node,
            model.kv_memory,
            model.bytes_per_token,
            total_tokens,
            allocator,
            space,
            leases,
            now_us,
            lease_duration_us,
        )?;
        let (inference, checkpoint) = match self.ledger.begin(
            request,
            model.allocation,
            cache.handle,
            model.primary_journal,
            model.replica_journal,
            rng_state,
        ) {
            Ok(started) => started,
            Err(error) => {
                let _ = self.caches.close(cache.handle, allocator, leases);
                return Err(error.into());
            }
        };
        let generation = self.executions[execution_slot]
            .generation
            .wrapping_add(1)
            .max(1);
        let handle = ExecutionHandle::from_parts(execution_slot, generation);
        self.executions[execution_slot] = ExecutionEntry {
            occupied: true,
            generation,
            request: Some(request),
            model_slot: model_slot as u16,
            inference: Some(inference),
            kv_cache: Some(cache.handle),
            prompt_tokens,
            max_generated_tokens,
        };
        Ok(ExecutionStart {
            info: self.info(handle)?,
            checkpoint,
        })
    }

    pub fn acknowledge_checkpoint(
        &mut self,
        handle: ExecutionHandle,
        epoch: u64,
        primary_acknowledged: bool,
        replica_acknowledged: bool,
    ) -> Result<ExecutionInfo, Error> {
        let entry = self.entry(handle)?;
        let inference = entry.inference.ok_or(Error::RequestNotFound)?;
        self.ledger.acknowledge_checkpoint(
            inference,
            epoch,
            primary_acknowledged,
            replica_acknowledged,
        )?;
        self.info(handle)
    }

    pub fn prepare_token_checkpoint(
        &mut self,
        handle: ExecutionHandle,
        generated_tokens: u64,
        rng_state: u64,
    ) -> Result<CheckpointWrite, Error> {
        let entry = *self.entry(handle)?;
        if generated_tokens > entry.max_generated_tokens {
            return Err(Error::InvalidRequest);
        }
        let inference = entry.inference.ok_or(Error::RequestNotFound)?;
        let cache = entry.kv_cache.ok_or(Error::RequestNotFound)?;
        let committed_tokens = entry
            .prompt_tokens
            .checked_add(generated_tokens)
            .ok_or(Error::InvalidRequest)?;
        let cache_info = self.caches.info(cache)?;
        if committed_tokens < cache_info.committed_tokens
            || committed_tokens > cache_info.reserved_tokens
        {
            return Err(Error::StaleSequence);
        }
        let checkpoint = self
            .ledger
            .prepare_checkpoint(inference, generated_tokens, rng_state)?;
        self.caches.commit_tokens(cache, committed_tokens)?;
        Ok(checkpoint)
    }

    pub fn fail_node(
        &mut self,
        handle: ExecutionHandle,
        failed: NodeId,
    ) -> Result<MemoryDegradationHandle, Error> {
        let inference = self
            .entry(handle)?
            .inference
            .ok_or(Error::RequestNotFound)?;
        self.ledger.fail_node(inference, failed).map_err(Into::into)
    }

    pub fn prepare_replica_repair(
        &mut self,
        handle: ExecutionHandle,
        replacement: NodeId,
    ) -> Result<CheckpointWrite, Error> {
        let inference = self
            .entry(handle)?
            .inference
            .ok_or(Error::RequestNotFound)?;
        self.ledger
            .prepare_replica_repair(inference, replacement)
            .map_err(Into::into)
    }

    pub fn info(&self, handle: ExecutionHandle) -> Result<ExecutionInfo, Error> {
        let entry = self.entry(handle)?;
        let model = self
            .models
            .get(entry.model_slot as usize)
            .and_then(|model| *model)
            .ok_or(Error::ModelNotFound)?;
        let inference = self
            .ledger
            .info(entry.inference.ok_or(Error::RequestNotFound)?)?;
        let kv_cache = self
            .caches
            .info(entry.kv_cache.ok_or(Error::RequestNotFound)?)?;
        Ok(ExecutionInfo {
            handle,
            request: entry.request.ok_or(Error::RequestNotFound)?,
            model: model.name,
            inference,
            kv_cache,
            prompt_tokens: entry.prompt_tokens,
            max_generated_tokens: entry.max_generated_tokens,
        })
    }

    pub fn finish<const ALLOCATIONS: usize, const EXTENTS: usize, const LEASES: usize>(
        &mut self,
        handle: ExecutionHandle,
        allocator: &mut UnifiedAllocator<ALLOCATIONS, EXTENTS>,
        leases: &mut LeaseTable<LEASES>,
    ) -> Result<(), Error> {
        let slot = self.valid_slot(handle)?;
        let entry = self.executions[slot];
        self.caches.close(
            entry.kv_cache.ok_or(Error::RequestNotFound)?,
            allocator,
            leases,
        )?;
        self.ledger
            .complete(entry.inference.ok_or(Error::RequestNotFound)?)?;
        self.executions[slot].occupied = false;
        Ok(())
    }

    fn model_slot(&self, name: &str) -> Result<usize, Error> {
        self.models
            .iter()
            .position(|entry| entry.is_some_and(|model| model.name.as_str() == name))
            .ok_or(Error::ModelNotFound)
    }

    fn entry(&self, handle: ExecutionHandle) -> Result<&ExecutionEntry, Error> {
        Ok(&self.executions[self.valid_slot(handle)?])
    }

    fn valid_slot(&self, handle: ExecutionHandle) -> Result<usize, Error> {
        let slot = handle.slot();
        let entry = self.executions.get(slot).ok_or(Error::RequestNotFound)?;
        if !entry.occupied || entry.generation != handle.generation() {
            return Err(Error::RequestNotFound);
        }
        Ok(slot)
    }
}

impl<
    const MODELS: usize,
    const ACTIVE: usize,
    const CACHES: usize,
    const SEGMENTS: usize,
    const LEDGER: usize,
> Default for ClusterInferenceService<MODELS, ACTIVE, CACHES, SEGMENTS, LEDGER>
{
    fn default() -> Self {
        Self::new()
    }
}
