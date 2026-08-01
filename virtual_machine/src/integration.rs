//! End-to-end checks for the SynOS kernel and its core kernel services.

use synos_kernel::{
    AddressSpaceId, CapabilityObject, CapabilitySpace, ExecutionMode, PhysicalRange, Rights,
    Scheduler, SchedulingPolicy,
};
use synos_kernel::ipc::{Channel, Message};
use synos_kernel::ipc::{ChannelId, SharedRegionId};

use crate::{CpuMode, Vm, VmConfig, VmError, VmRunReport};

const CR0_PAGING: u64 = 1 << 31;

#[derive(Debug)]
pub enum IntegrationError {
    Vm(VmError),
    InvalidConfiguration,
    Scheduler(synos_kernel::SchedulerError),
    Capability(synos_kernel::CapabilityError),
    Ipc(synos_kernel::ipc::IpcError),
    InvalidKernelState,
}

impl From<VmError> for IntegrationError {
    fn from(error: VmError) -> Self {
        Self::Vm(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynosIntegrationReport {
    pub vm: VmRunReport,
    pub kernel_booted: bool,
    pub paging_ready: bool,
    pub scheduler_ready: bool,
    pub capabilities_ready: bool,
    pub ipc_ready: bool,
    pub boot_output: String,
}

/// Boot the supplied kernel for a bounded number of instructions and exercise
/// the same scheduler, capability, memory, and IPC APIs used by kernel code.
pub fn run_synos_integration(
    mut config: VmConfig,
    max_steps: u64,
) -> Result<SynosIntegrationReport, IntegrationError> {
    if config.kernel_path.is_none() || max_steps == 0 {
        return Err(IntegrationError::InvalidConfiguration);
    }

    config.max_steps = Some(max_steps);
    let mut vm = Vm::with_config(config);
    let vm_report = vm.run_for_steps(max_steps)?;

    let boot_output = vm
        .serial()
        .map(|serial| String::from_utf8_lossy(serial.borrow().output()).into_owned())
        .unwrap_or_default();
    let cpu = &vm.cpu().state;
    let paging_ready = cpu.mode == CpuMode::Long64
        && cpu.cr3 != 0
        && cpu.cr0 & CR0_PAGING != 0;
    // The serial line may still be inside one formatted write when the
    // bounded run ends. Long-mode execution at the loaded kernel address is
    // the reliable handoff signal; serial text is retained for diagnostics.
    let kernel_booted = boot_output.contains("SynOS kernel bootstrap")
        || (paging_ready && vm_report.rip >= crate::KERNEL_LOAD_ADDR);

    let (scheduler_ready, capabilities_ready) = exercise_scheduler_and_capabilities()?;
    let ipc_ready = exercise_ipc()?;

    Ok(SynosIntegrationReport {
        vm: vm_report,
        kernel_booted,
        paging_ready,
        scheduler_ready,
        capabilities_ready,
        ipc_ready,
        boot_output,
    })
}

fn exercise_scheduler_and_capabilities() -> Result<(bool, bool), IntegrationError> {
    let owner = AddressSpaceId::KERNEL;
    let mut capabilities = CapabilitySpace::<32>::new();
    let authority = capabilities
        .mint_root(owner, CapabilityObject::AddressSpace(owner), Rights::ALL)
        .map_err(IntegrationError::Capability)?;

    let mut scheduler = Scheduler::new();
    let first = scheduler
        .create(
            &capabilities,
            owner,
            authority,
            owner,
            ExecutionMode::Kernel,
            SchedulingPolicy::Cooperative,
            0x1000,
            0x8000,
        )
        .map_err(IntegrationError::Scheduler)?;
    let second = scheduler
        .create(
            &capabilities,
            owner,
            authority,
            owner,
            ExecutionMode::Kernel,
            SchedulingPolicy::Cooperative,
            0x2000,
            0x9000,
        )
        .map_err(IntegrationError::Scheduler)?;
    let first_switch = scheduler
        .dispatch()
        .ok_or(IntegrationError::InvalidKernelState)?;
    scheduler
        .yield_current()
        .map_err(IntegrationError::Scheduler)?;
    let scheduler_ready = first_switch.next == second
        && scheduler.current() == Some(first)
        && scheduler.thread(second).is_ok();

    let backing = PhysicalRange::new(0x20_0000, 0x4000)
        .ok_or(IntegrationError::InvalidKernelState)?;
    let region_memory = PhysicalRange::new(0x20_1000, 0x1000)
        .ok_or(IntegrationError::InvalidKernelState)?;
    let untyped = capabilities
        .mint_untyped(owner, backing, Rights::ALL)
        .map_err(IntegrationError::Capability)?;
    let region = SharedRegionId::new(1).ok_or(IntegrationError::InvalidKernelState)?;
    let shared = capabilities
        .retype_memory(
            owner,
            untyped,
            owner,
            region,
            region_memory,
            Rights::READ.union(Rights::WRITE).union(Rights::MAP),
        )
        .map_err(IntegrationError::Capability)?;
    capabilities
        .authorize_mapping(owner, shared, region, true, false)
        .map_err(IntegrationError::Capability)?;
    let revoked = capabilities
        .revoke(owner, untyped)
        .map_err(IntegrationError::Capability)?;
    let capabilities_ready = revoked == 1 && capabilities.used() == 2;

    Ok((scheduler_ready, capabilities_ready))
}

fn exercise_ipc() -> Result<bool, IntegrationError> {
    let owner = AddressSpaceId::KERNEL;
    let mut capabilities = CapabilitySpace::<8>::new();
    let channel_id = ChannelId::new(1).ok_or(IntegrationError::InvalidKernelState)?;
    let endpoint = capabilities
        .mint_root(
            owner,
            CapabilityObject::IpcChannel(channel_id),
            Rights::SEND.union(Rights::RECEIVE),
        )
        .map_err(IntegrationError::Capability)?;
    let channel = Channel::<4>::new(channel_id);
    let mut message = Message::EMPTY;
    message.label = 0x5349_4e54;
    message.words = [1, 2, 3, 4];
    channel
        .try_send(&capabilities, owner, endpoint, None, message)
        .map_err(IntegrationError::Ipc)?;
    let received = channel
        .try_receive(&capabilities, owner, endpoint)
        .map_err(IntegrationError::Ipc)?;
    Ok(received.label == message.label && received.words == message.words)
}
