#![no_std]
#![forbid(unsafe_code)]

//! Self-healing Ring 3 daemon supervision.
//!
//! Health signals are lock-free and fixed-capacity. Clean daemon state is
//! pinned in SynFS CoW checkpoints. Recovery and replacement operations call
//! platform adapters for privileged process and IPC work.

mod recovery;
mod telemetry;

pub use recovery::{
    DEFAULT_RECOVERY_CAPACITY, RecoveryCoordinator, RecoveryError, RecoveryReceipt,
    RecoveryRuntime, RecoverySpawnRequest, RecoveryStatus,
};
pub use telemetry::{
    DEFAULT_HEALTH_CAPACITY, HealthConfig, HealthError, HealthFault, HealthMonitor, HealthReport,
    HealthState, HealthToken,
};

pub use synos_update::{
    HotSwapCoordinator, HotSwapError, HotSwapReceipt, HotSwapRequest, HotSwapRuntime,
    InheritableDescriptor, KernelPatchCoordinator, KernelPatchError, KernelPatchReceipt,
    KernelPatchRecord, KernelPatchRequest, KernelPatchRuntime, MicrokernelPatchCoordinator,
    ReplacementSpawnRequest,
};

/// Combined supervisor used by `synos-heal`.
pub struct SelfHealingSupervisor<
    const HEALTH_CAPACITY: usize = DEFAULT_HEALTH_CAPACITY,
    const RECOVERY_CAPACITY: usize = DEFAULT_RECOVERY_CAPACITY,
> {
    health: HealthMonitor<HEALTH_CAPACITY>,
    recovery: RecoveryCoordinator<RECOVERY_CAPACITY>,
    hot_swap: HotSwapCoordinator,
}

impl<const HEALTH_CAPACITY: usize, const RECOVERY_CAPACITY: usize>
    SelfHealingSupervisor<HEALTH_CAPACITY, RECOVERY_CAPACITY>
{
    pub const fn new() -> Self {
        Self {
            health: HealthMonitor::new(),
            recovery: RecoveryCoordinator::new(),
            hot_swap: HotSwapCoordinator::new(),
        }
    }

    pub const fn health(&self) -> &HealthMonitor<HEALTH_CAPACITY> {
        &self.health
    }

    pub const fn health_mut(&mut self) -> &mut HealthMonitor<HEALTH_CAPACITY> {
        &mut self.health
    }

    pub const fn recovery(&self) -> &RecoveryCoordinator<RECOVERY_CAPACITY> {
        &self.recovery
    }

    pub const fn recovery_mut(&mut self) -> &mut RecoveryCoordinator<RECOVERY_CAPACITY> {
        &mut self.recovery
    }

    pub fn recover<const MAX_BLOCKS: usize, R: RecoveryRuntime>(
        &mut self,
        service: u64,
        crashed_process: synos_init::ProcessId,
        filesystem: &synos_synfs::SynFs<MAX_BLOCKS>,
        capability: synos_synfs::RmsMapHandle,
        runtime: &mut R,
    ) -> Result<RecoveryReceipt, RecoveryError> {
        self.recovery
            .recover(service, crashed_process, filesystem, capability, runtime)
    }

    pub fn hot_swap<R: HotSwapRuntime>(
        &mut self,
        runtime: &mut R,
        request: HotSwapRequest<'_>,
    ) -> Result<HotSwapReceipt, HotSwapError> {
        self.hot_swap.replace(runtime, request)
    }
}

impl<const HEALTH_CAPACITY: usize, const RECOVERY_CAPACITY: usize> Default
    for SelfHealingSupervisor<HEALTH_CAPACITY, RECOVERY_CAPACITY>
{
    fn default() -> Self {
        Self::new()
    }
}

pub type DaemonSupervisor<const HEALTH_CAPACITY: usize = DEFAULT_HEALTH_CAPACITY> =
    SelfHealingSupervisor<HEALTH_CAPACITY, DEFAULT_RECOVERY_CAPACITY>;
