#![forbid(unsafe_code)]

use ghostos_init::ProcessId;
use ghostos_ipc::{
    validate_inheritable_descriptors, DescriptorInheritance, DescriptorInheritanceError,
    InheritableDescriptor,
};
use ghostos_status::{IntoStatus, Status};

pub const MAX_HOT_SWAP_DESCRIPTORS: usize = ghostos_ipc::MAX_INHERITABLE_DESCRIPTORS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotSwapRequest<'a> {
    pub service: u64,
    pub old_process: ProcessId,
    pub old_generation: u32,
    pub image: u128,
    pub descriptors: &'a [InheritableDescriptor],
}

impl<'a> HotSwapRequest<'a> {
    pub const fn new(
        service: u64,
        old_process: ProcessId,
        old_generation: u32,
        image: u128,
        descriptors: &'a [InheritableDescriptor],
    ) -> Self {
        Self {
            service,
            old_process,
            old_generation,
            image,
            descriptors,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplacementSpawnRequest<'a> {
    pub service: u64,
    pub image: u128,
    pub generation: u32,
    pub descriptors: &'a [InheritableDescriptor],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HotSwapReceipt {
    pub service: u64,
    pub old_process: ProcessId,
    pub replacement_process: ProcessId,
    pub old_generation: u32,
    pub replacement_generation: u32,
    pub inherited_descriptors: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HotSwapError {
    InvalidRequest,
    DescriptorCapacity,
    DuplicateDescriptor,
    ReplacementSpawnFailed,
    DescriptorInheritanceFailed,
    ReplacementNotReady,
    SwitchFailed,
    ReplacementCleanupFailed,
    DrainFailed,
    RetireFailed,
}

impl IntoStatus for HotSwapError {
    fn status(self) -> Status {
        match self {
            Self::InvalidRequest | Self::DuplicateDescriptor => Status::INVALID_ARGUMENT,
            Self::DescriptorCapacity => Status::NO_SPACE,
            Self::ReplacementSpawnFailed
            | Self::DescriptorInheritanceFailed
            | Self::ReplacementNotReady
            | Self::SwitchFailed
            | Self::ReplacementCleanupFailed
            | Self::DrainFailed
            | Self::RetireFailed => Status::BUSY,
        }
    }
}

/// Operations supplied by Ring 0 or the service supervisor.
///
/// The order enforced by [`HotSwapCoordinator::replace`] is the availability
/// guarantee: the old process serves traffic while the replacement starts,
/// receives inherited descriptors, and reports ready. Routing changes only at
/// `switch_service`, after which old requests are drained before old-process
/// capabilities are revoked.
pub trait HotSwapRuntime: DescriptorInheritance {
    fn spawn_replacement(
        &mut self,
        request: ReplacementSpawnRequest<'_>,
    ) -> Result<ProcessId, Self::Error>;

    fn replacement_ready(&mut self, process: ProcessId) -> Result<(), Self::Error>;

    fn switch_service(
        &mut self,
        service: u64,
        expected_old: ProcessId,
        replacement: ProcessId,
    ) -> Result<(), Self::Error>;

    fn drain_process(&mut self, process: ProcessId) -> Result<(), Self::Error>;

    fn fence_process(&mut self, process: ProcessId) -> Result<(), Self::Error>;
}

pub struct HotSwapCoordinator;

impl HotSwapCoordinator {
    pub const fn new() -> Self {
        Self
    }

    pub fn replace<R: HotSwapRuntime>(
        &mut self,
        runtime: &mut R,
        request: HotSwapRequest<'_>,
    ) -> Result<HotSwapReceipt, HotSwapError> {
        validate_request(request)?;

        let replacement_generation = request.old_generation.wrapping_add(1).max(1);
        let replacement = runtime
            .spawn_replacement(ReplacementSpawnRequest {
                service: request.service,
                image: request.image,
                generation: replacement_generation,
                descriptors: request.descriptors,
            })
            .map_err(|_| HotSwapError::ReplacementSpawnFailed)?;

        if replacement == request.old_process || replacement.raw() == 0 {
            // Never fence here: a buggy runtime may have returned the old
            // process, and fencing it would interrupt the service we protect.
            return Err(HotSwapError::InvalidRequest);
        }

        if runtime
            .inherit_descriptors(
                request.old_process.raw(),
                replacement.raw(),
                request.descriptors,
            )
            .is_err()
        {
            return cleanup_replacement(runtime, replacement, HotSwapError::DescriptorInheritanceFailed);
        }

        if runtime.replacement_ready(replacement).is_err() {
            return cleanup_replacement(runtime, replacement, HotSwapError::ReplacementNotReady);
        }

        if runtime
            .switch_service(request.service, request.old_process, replacement)
            .is_err()
        {
            return cleanup_replacement(runtime, replacement, HotSwapError::SwitchFailed);
        }

        if runtime.drain_process(request.old_process).is_err() {
            return Err(HotSwapError::DrainFailed);
        }
        if runtime.fence_process(request.old_process).is_err() {
            return Err(HotSwapError::RetireFailed);
        }

        Ok(HotSwapReceipt {
            service: request.service,
            old_process: request.old_process,
            replacement_process: replacement,
            old_generation: request.old_generation,
            replacement_generation,
            inherited_descriptors: request.descriptors.len(),
        })
    }
}

impl Default for HotSwapCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

fn validate_request(request: HotSwapRequest<'_>) -> Result<(), HotSwapError> {
    if request.service == 0 || request.old_process.raw() == 0 || request.image == 0 {
        return Err(HotSwapError::InvalidRequest);
    }
    validate_inheritable_descriptors(request.descriptors).map_err(|error| match error {
        DescriptorInheritanceError::Capacity => HotSwapError::DescriptorCapacity,
        DescriptorInheritanceError::Duplicate => HotSwapError::DuplicateDescriptor,
    })
}

fn cleanup_replacement<R: HotSwapRuntime>(
    runtime: &mut R,
    replacement: ProcessId,
    error: HotSwapError,
) -> Result<HotSwapReceipt, HotSwapError> {
    if runtime.fence_process(replacement).is_err() {
        return Err(HotSwapError::ReplacementCleanupFailed);
    }
    Err(error)
}
