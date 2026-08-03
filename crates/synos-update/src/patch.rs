use synos_status::{IntoStatus, Severity, Status, facility};

pub const DEFAULT_KERNEL_PATCH_CAPACITY: usize = 32;
pub const MAX_KERNEL_PATCH_BATCH: usize = 16;

/// A request to redirect one Ring 0 function to a replacement implementation.
///
/// `expected_generation` makes an update single-writer: a patch prepared
/// against an older kernel generation cannot be applied after another patch
/// has changed the live dispatch table. The architecture runtime must also
/// authenticate the replacement and prove that both addresses belong to the
/// trusted kernel image before returning from [`KernelPatchRuntime::validate`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelPatchRequest {
    pub patch_id: u64,
    pub target: usize,
    pub replacement: usize,
    pub expected_generation: u64,
}

impl KernelPatchRequest {
    pub const fn new(
        patch_id: u64,
        target: usize,
        replacement: usize,
        expected_generation: u64,
    ) -> Self {
        Self {
            patch_id,
            target,
            replacement,
            expected_generation,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelPatchRecord {
    pub patch_id: u64,
    pub target: usize,
    pub replacement: usize,
    pub generation: u64,
    batch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KernelPatchReceipt {
    pub previous_generation: u64,
    pub generation: u64,
    pub patches: usize,
    pub batch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KernelPatchError {
    InvalidRequest,
    Capacity,
    DuplicatePatch,
    DuplicateTarget,
    AlreadyPatched,
    StaleGeneration,
    ValidationFailed,
    QuiesceFailed,
    RedirectFailed,
    RollbackFailed,
    ResumeFailed,
    NoActivePatch,
}

impl IntoStatus for KernelPatchError {
    fn status(self) -> Status {
        match self {
            Self::InvalidRequest
            | Self::DuplicatePatch
            | Self::DuplicateTarget
            | Self::AlreadyPatched
            | Self::StaleGeneration => Status::INVALID_ARGUMENT,
            Self::Capacity => Status::NO_SPACE,
            Self::ValidationFailed => Status::ACCESS_DENIED,
            Self::QuiesceFailed
            | Self::RedirectFailed
            | Self::RollbackFailed
            | Self::ResumeFailed
            | Self::NoActivePatch => Status::new(Severity::Error, facility::KERNEL, 8, 0)
                .expect("valid kernel patch status"),
        }
    }
}

/// Ring 0 operations needed by the portable patch transaction.
///
/// An implementation must make `quiesce` a stop-the-world barrier: no CPU may
/// execute kernel text while redirects are installed or restored. It must
/// validate the target range, replacement image signature, ABI compatibility,
/// and instruction encoding before changing memory. `redirect` and `restore`
/// must be atomic from the point of view of resumed CPUs, normally by writing
/// a pre-validated patchpoint or function-table slot and flushing instruction
/// caches before `resume`. If `resume` fails, the runtime must leave CPUs
/// quiesced so the coordinator can restore the old redirects safely.
pub trait KernelPatchRuntime {
    type Error;

    fn validate(&mut self, request: KernelPatchRequest) -> Result<(), Self::Error>;
    fn quiesce(&mut self) -> Result<(), Self::Error>;
    fn redirect(&mut self, request: KernelPatchRequest) -> Result<(), Self::Error>;
    fn restore(&mut self, request: KernelPatchRequest) -> Result<(), Self::Error>;
    fn resume(&mut self) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy)]
struct PatchSlot {
    record: Option<KernelPatchRecord>,
}

impl PatchSlot {
    const EMPTY: Self = Self { record: None };
}

/// Applies bounded, generation-checked Ring 0 redirects as one transaction.
///
/// The coordinator owns policy and failure handling. It never writes executable
/// memory itself; that unsafe, architecture-specific work stays behind
/// [`KernelPatchRuntime`]. A failed batch leaves all redirects at their old
/// values, unless the runtime reports that both rollback or resume could not
/// be completed.
pub struct KernelPatchCoordinator<const CAPACITY: usize = DEFAULT_KERNEL_PATCH_CAPACITY> {
    slots: [PatchSlot; CAPACITY],
    generation: u64,
    next_batch: u64,
    last_batch: Option<u64>,
}

pub type MicrokernelPatchCoordinator<const CAPACITY: usize = DEFAULT_KERNEL_PATCH_CAPACITY> =
    KernelPatchCoordinator<CAPACITY>;

impl<const CAPACITY: usize> KernelPatchCoordinator<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            slots: [PatchSlot::EMPTY; CAPACITY],
            generation: 1,
            next_batch: 1,
            last_batch: None,
        }
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn records(&self) -> impl Iterator<Item = KernelPatchRecord> + '_ {
        self.slots.iter().filter_map(|slot| slot.record)
    }

    pub fn apply<R: KernelPatchRuntime>(
        &mut self,
        runtime: &mut R,
        requests: &[KernelPatchRequest],
    ) -> Result<KernelPatchReceipt, KernelPatchError> {
        self.validate_batch(requests)?;

        for &request in requests {
            runtime
                .validate(request)
                .map_err(|_| KernelPatchError::ValidationFailed)?;
        }

        runtime
            .quiesce()
            .map_err(|_| KernelPatchError::QuiesceFailed)?;

        let mut applied = 0;
        while applied < requests.len() {
            if runtime.redirect(requests[applied]).is_err() {
                return self.abort_batch(
                    runtime,
                    requests,
                    applied,
                    KernelPatchError::RedirectFailed,
                );
            }
            applied += 1;
        }

        if runtime.resume().is_err() {
            return self.abort_batch(
                runtime,
                requests,
                requests.len(),
                KernelPatchError::ResumeFailed,
            );
        }

        let previous_generation = self.generation;
        let generation = self.next_generation();
        let batch = self.next_batch();
        let start = self.first_empty().expect("capacity checked above");
        for (offset, request) in requests.iter().copied().enumerate() {
            self.slots[start + offset].record = Some(KernelPatchRecord {
                patch_id: request.patch_id,
                target: request.target,
                replacement: request.replacement,
                generation,
                batch,
            });
        }
        self.last_batch = Some(batch);

        Ok(KernelPatchReceipt {
            previous_generation,
            generation,
            patches: requests.len(),
            batch,
        })
    }

    pub fn rollback_last<R: KernelPatchRuntime>(
        &mut self,
        runtime: &mut R,
    ) -> Result<KernelPatchReceipt, KernelPatchError> {
        let batch = self.last_batch.ok_or(KernelPatchError::NoActivePatch)?;
        let mut requests = [KernelPatchRequest::new(0, 0, 0, 0); MAX_KERNEL_PATCH_BATCH];
        let mut count = 0;
        for slot in &self.slots {
            if let Some(record) = slot.record.filter(|record| record.batch == batch) {
                if count == requests.len() {
                    return Err(KernelPatchError::Capacity);
                }
                requests[count] = KernelPatchRequest::new(
                    record.patch_id,
                    record.target,
                    record.replacement,
                    record.generation,
                );
                count += 1;
            }
        }
        if count == 0 {
            return Err(KernelPatchError::NoActivePatch);
        }

        runtime
            .quiesce()
            .map_err(|_| KernelPatchError::QuiesceFailed)?;

        let mut removed = 0;
        while removed < count {
            let index = count - removed - 1;
            if runtime.restore(requests[index]).is_err() {
                return self.abort_rollback(runtime, &requests, count, removed);
            }
            removed += 1;
        }

        if runtime.resume().is_err() {
            return self.abort_rollback(runtime, &requests, count, count);
        }

        for slot in &mut self.slots {
            if slot.record.is_some_and(|record| record.batch == batch) {
                slot.record = None;
            }
        }
        self.last_batch = self
            .slots
            .iter()
            .filter_map(|slot| slot.record.map(|record| record.batch))
            .max();
        let previous_generation = self.generation;
        let generation = self.next_generation();
        Ok(KernelPatchReceipt {
            previous_generation,
            generation,
            patches: count,
            batch,
        })
    }

    fn validate_batch(&self, requests: &[KernelPatchRequest]) -> Result<(), KernelPatchError> {
        if requests.is_empty() || requests.len() > MAX_KERNEL_PATCH_BATCH {
            return Err(KernelPatchError::InvalidRequest);
        }
        if requests.len()
            > CAPACITY.saturating_sub(
                self.slots
                    .iter()
                    .filter(|slot| slot.record.is_some())
                    .count(),
            )
        {
            return Err(KernelPatchError::Capacity);
        }

        for (index, request) in requests.iter().copied().enumerate() {
            if request.patch_id == 0
                || request.target == 0
                || request.replacement == 0
                || request.target == request.replacement
            {
                return Err(KernelPatchError::InvalidRequest);
            }
            if request.expected_generation != self.generation {
                return Err(KernelPatchError::StaleGeneration);
            }
            if requests[..index]
                .iter()
                .any(|previous| previous.patch_id == request.patch_id)
            {
                return Err(KernelPatchError::DuplicatePatch);
            }
            if requests[..index]
                .iter()
                .any(|previous| previous.target == request.target)
            {
                return Err(KernelPatchError::DuplicateTarget);
            }
            if self
                .slots
                .iter()
                .filter_map(|slot| slot.record)
                .any(|record| {
                    record.patch_id == request.patch_id || record.target == request.target
                })
            {
                return Err(KernelPatchError::AlreadyPatched);
            }
        }
        Ok(())
    }

    fn abort_batch<R: KernelPatchRuntime>(
        &self,
        runtime: &mut R,
        requests: &[KernelPatchRequest],
        applied: usize,
        error: KernelPatchError,
    ) -> Result<KernelPatchReceipt, KernelPatchError> {
        let mut rollback_failed = false;
        for index in (0..applied).rev() {
            if runtime.restore(requests[index]).is_err() {
                rollback_failed = true;
            }
        }
        let resume_failed = runtime.resume().is_err();
        if rollback_failed {
            Err(KernelPatchError::RollbackFailed)
        } else if resume_failed {
            Err(KernelPatchError::ResumeFailed)
        } else {
            Err(error)
        }
    }

    fn abort_rollback<R: KernelPatchRuntime>(
        &self,
        runtime: &mut R,
        requests: &[KernelPatchRequest; MAX_KERNEL_PATCH_BATCH],
        count: usize,
        removed: usize,
    ) -> Result<KernelPatchReceipt, KernelPatchError> {
        let mut recovery_failed = false;
        for request in requests.iter().skip(count - removed).take(removed) {
            if runtime.redirect(*request).is_err() {
                recovery_failed = true;
            }
        }
        let resume_failed = runtime.resume().is_err();
        if recovery_failed || resume_failed {
            Err(KernelPatchError::RollbackFailed)
        } else {
            let _ = count;
            Err(KernelPatchError::RollbackFailed)
        }
    }

    fn first_empty(&self) -> Option<usize> {
        self.slots.iter().position(|slot| slot.record.is_none())
    }

    fn next_generation(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1).max(1);
        self.generation
    }

    fn next_batch(&mut self) -> u64 {
        let batch = self.next_batch;
        self.next_batch = self.next_batch.wrapping_add(1).max(1);
        batch
    }
}

impl<const CAPACITY: usize> Default for KernelPatchCoordinator<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}
