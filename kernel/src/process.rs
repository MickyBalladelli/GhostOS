//! Kernel-owned adapter for native application images and process lifetime.

use core::convert::TryFrom;

use ghostos_app::{
    ImageLoadRequest, ImageMapper, LoadedImage, Mapping, MappingRequest, NativeExecRequest,
    NativeSpawnRequest, ProcessArguments, ProcessBackend, ProcessContext, ProcessExit,
    ProcessLimits, ProcessState, ProcessStatus, ProcessUsage,
    RuntimeSegment, SegmentPermissions, StackRequest, TlsRequest, load_image, DEFAULT_GUARD_PAGES,
    DEFAULT_STACK_BYTES, LoaderError,
};
use ghostos_init::{CrashReason, ExitReason, ProcessId, SpawnRequest, SupervisorRuntime};
use ghostos_status::{IntoStatus, Status};
use ghostos_system_model::ContentId;

use crate::{
    AddressSpaceId, AddressSpaceTable, CapabilityHandle, CapabilityObject, CapabilitySpace,
    Context, PageTableRoot, Rights, Scheduler, SchedulingPolicy, ThreadId,
};

pub const DEFAULT_KERNEL_PROCESS_CAPACITY: usize = 64;

/// Verified native image information supplied by the package/service loader.
#[derive(Clone, Copy)]
pub struct NativeServiceImage<'a> {
    pub bytes: &'a [u8],
    pub architecture: ghostos_app::ImageArchitecture,
    pub heap_bytes: u64,
    pub limits: ProcessLimits,
}

/// Resolves the image identity carried by a service manifest to verified ELF
/// bytes. The kernel never loads an image by name or from an untrusted path.
pub trait ServiceImageProvider {
    fn image(&self, image_id: u128) -> Option<NativeServiceImage<'_>>;
}

/// Real kernel process runtime for [`ghostos_init::Supervisor`].
///
/// Service restart calls `spawn` again after the supervisor's backoff. Service
/// fencing calls the kernel process backend, which stops the thread, releases
/// its mappings and address space, and deletes its authority capability.
pub struct KernelSupervisorRuntime<'a, M, I, const CAPACITY: usize = DEFAULT_KERNEL_PROCESS_CAPACITY,
    const MAX_CAPABILITIES: usize = { crate::MAX_CAPABILITIES }>
where
    M: ProcessMemory,
    I: ServiceImageProvider,
{
    backend: &'a mut KernelProcessBackend<'a, M, CAPACITY, MAX_CAPABILITIES>,
    images: &'a I,
}

impl<'a, M, I, const CAPACITY: usize, const MAX_CAPABILITIES: usize>
    KernelSupervisorRuntime<'a, M, I, CAPACITY, MAX_CAPABILITIES>
where
    M: ProcessMemory,
    I: ServiceImageProvider,
{
    pub fn new(
        backend: &'a mut KernelProcessBackend<'a, M, CAPACITY, MAX_CAPABILITIES>,
        images: &'a I,
    ) -> Self {
        Self { backend, images }
    }

    pub fn backend(&self) -> &KernelProcessBackend<'a, M, CAPACITY, MAX_CAPABILITIES> {
        self.backend
    }
}

impl<M, I, const CAPACITY: usize, const MAX_CAPABILITIES: usize> SupervisorRuntime
    for KernelSupervisorRuntime<'_, M, I, CAPACITY, MAX_CAPABILITIES>
where
    M: ProcessMemory,
    I: ServiceImageProvider,
{
    type Error = KernelProcessError;

    fn spawn(&mut self, request: SpawnRequest) -> Result<ProcessId, Self::Error> {
        if request.image_id == 0 || request.capability_profile == 0 {
            return Err(KernelProcessError::InvalidTransition)
        }
        let image = self
            .images
            .image(request.image_id)
            .ok_or(KernelProcessError::NotFound)?;
        self.backend.spawn(NativeSpawnRequest {
            image: image.bytes,
            architecture: image.architecture,
            expected_payload: None,
            heap_bytes: image.heap_bytes,
            arguments: ProcessArguments {
                argv: &[],
                environment: &[],
            },
            limits: image.limits,
        })
    }

    fn fence_process(&mut self, process: ProcessId) -> Result<(), Self::Error> {
        self.backend.fence(process)
    }
}

/// Privileged memory operations needed by the application image loader.
///
/// An implementation owns the page tables and physical frames for an address
/// space. The process backend below supplies the address-space identity and
/// keeps the ELF parser independent from those privileged details.
pub trait ProcessMemory {
    type Error;

    /// Allocate and initialize a distinct user page-table root. The returned
    /// root must not alias the kernel root or another live process root.
    fn create_address_space(
        &mut self,
        address_space: AddressSpaceId,
    ) -> Result<PageTableRoot, Self::Error>;
    fn destroy_address_space(&mut self, address_space: AddressSpaceId);

    fn reserve(
        &mut self,
        address_space: AddressSpaceId,
        request: MappingRequest,
    ) -> Result<Mapping, Self::Error>;
    fn map_segment(
        &mut self,
        address_space: AddressSpaceId,
        mapping: Mapping,
        segment: RuntimeSegment,
        source: &[u8],
    ) -> Result<(), Self::Error>;
    fn zero_fill(
        &mut self,
        address_space: AddressSpaceId,
        mapping: Mapping,
        address: u64,
        length: u64,
    ) -> Result<(), Self::Error>;
    fn apply_relative_relocation(
        &mut self,
        address_space: AddressSpaceId,
        mapping: Mapping,
        address: u64,
        addend: i64,
        addend_from_memory: bool,
    ) -> Result<(), Self::Error>;
    fn protect(
        &mut self,
        address_space: AddressSpaceId,
        mapping: Mapping,
        address: u64,
        length: u64,
        permissions: SegmentPermissions,
    ) -> Result<(), Self::Error>;
    fn allocate_stack(
        &mut self,
        address_space: AddressSpaceId,
        mapping: Mapping,
        request: StackRequest,
        arguments: ProcessArguments<'_>,
    ) -> Result<u64, Self::Error>;
    fn allocate_heap(
        &mut self,
        address_space: AddressSpaceId,
        mapping: Mapping,
        size: u64,
    ) -> Result<u64, Self::Error>;
    fn allocate_tls(
        &mut self,
        address_space: AddressSpaceId,
        mapping: Mapping,
        request: TlsRequest,
        source: &[u8],
    ) -> Result<u64, Self::Error>;
    fn install_context(
        &mut self,
        address_space: AddressSpaceId,
        mapping: Mapping,
        context: ProcessContext,
    ) -> Result<(), Self::Error>;
    fn release(&mut self, address_space: AddressSpaceId, mapping: Mapping);
}

struct KernelImageMapper<'a, M, const CAPACITY: usize> {
    memory: &'a mut M,
    address_spaces: &'a mut AddressSpaceTable<CAPACITY>,
    address_space: AddressSpaceId,
}

impl<'a, M, const CAPACITY: usize> KernelImageMapper<'a, M, CAPACITY> {
    fn new(
        memory: &'a mut M,
        address_spaces: &'a mut AddressSpaceTable<CAPACITY>,
        address_space: AddressSpaceId,
    ) -> Self {
        Self {
            memory,
            address_spaces,
            address_space,
        }
    }
}

impl<M: ProcessMemory, const CAPACITY: usize> ImageMapper
    for KernelImageMapper<'_, M, CAPACITY>
{
    type Error = ();

    fn reserve(&mut self, request: MappingRequest) -> Result<Mapping, Self::Error> {
        let mut request = request;
        if !request.fixed && request.preferred_base.is_none() {
            request.preferred_base = self
                .address_spaces
                .get_mut(self.address_space)
                .map_err(|_| ())?
                .randomized_hint(request.size, request.alignment)
        }
        let mapping = self
            .memory
            .reserve(self.address_space, request)
            .map_err(|_| ())?;
        if self
            .address_spaces
            .get_mut(self.address_space)
            .map_err(|_| ())?
            .claim(mapping)
            .is_err()
        {
            self.memory.release(self.address_space, mapping);
            return Err(())
        }
        Ok(mapping)
    }

    fn map_segment(
        &mut self,
        mapping: Mapping,
        segment: RuntimeSegment,
        source: &[u8],
    ) -> Result<(), Self::Error> {
        self.memory
            .map_segment(self.address_space, mapping, segment, source)
            .map_err(|_| ())?;
        self.address_spaces
            .get_mut(self.address_space)
            .map_err(|_| ())?
            .map_segment(mapping, segment, source.len())
            .map_err(|_| ())
    }

    fn zero_fill(
        &mut self,
        mapping: Mapping,
        address: u64,
        length: u64,
    ) -> Result<(), Self::Error> {
        self.address_spaces
            .get(self.address_space)
            .map_err(|_| ())?
            .zero_fill(mapping, address, length)
            .map_err(|_| ())?;
        self.memory
            .zero_fill(self.address_space, mapping, address, length)
            .map_err(|_| ())
    }

    fn apply_relative_relocation(
        &mut self,
        mapping: Mapping,
        address: u64,
        addend: i64,
        addend_from_memory: bool,
    ) -> Result<(), Self::Error> {
        self.address_spaces
            .get(self.address_space)
            .map_err(|_| ())?
            .relocate(mapping, address)
            .map_err(|_| ())?;
        self.memory.apply_relative_relocation(
            self.address_space,
            mapping,
            address,
            addend,
            addend_from_memory,
        )
        .map_err(|_| ())
    }

    fn protect(
        &mut self,
        mapping: Mapping,
        address: u64,
        length: u64,
        permissions: SegmentPermissions,
    ) -> Result<(), Self::Error> {
        self.memory
            .protect(self.address_space, mapping, address, length, permissions)
            .map_err(|_| ())?;
        self.address_spaces
            .get_mut(self.address_space)
            .map_err(|_| ())?
            .protect(mapping, address, length, permissions)
            .map_err(|_| ())
    }

    fn allocate_stack(
        &mut self,
        mapping: Mapping,
        request: StackRequest,
        arguments: ProcessArguments<'_>,
    ) -> Result<u64, Self::Error> {
        let stack = self
            .memory
            .allocate_stack(self.address_space, mapping, request, arguments)
            .map_err(|_| ())?;
        let base = stack
            .checked_sub(request.size)
            .unwrap_or_default();
        self.address_spaces
            .get_mut(self.address_space)
            .map_err(|_| ())?
            .record_stack(mapping, base, request.size, request.guard_pages)
            .map_err(|_| ())?;
        Ok(stack)
    }

    fn allocate_heap(&mut self, mapping: Mapping, size: u64) -> Result<u64, Self::Error> {
        let base = self
            .memory
            .allocate_heap(self.address_space, mapping, size)
            .map_err(|_| ())?;
        self.address_spaces
            .get_mut(self.address_space)
            .map_err(|_| ())?
            .record_region(mapping, base, size, SegmentPermissions::READ.union(SegmentPermissions::WRITE))
            .map_err(|_| ())?;
        Ok(base)
    }

    fn allocate_tls(
        &mut self,
        mapping: Mapping,
        request: TlsRequest,
        source: &[u8],
    ) -> Result<u64, Self::Error> {
        let base = self
            .memory
            .allocate_tls(self.address_space, mapping, request, source)
            .map_err(|_| ())?;
        self.address_spaces
            .get_mut(self.address_space)
            .map_err(|_| ())?
            .record_region(mapping, base, request.memory_size, SegmentPermissions::READ)
            .map_err(|_| ())?;
        Ok(base)
    }

    fn install_context(
        &mut self,
        mapping: Mapping,
        context: ProcessContext,
    ) -> Result<(), Self::Error> {
        self.address_spaces
            .get(self.address_space)
            .map_err(|_| ())?
            .install_context(context)
            .map_err(|_| ())?;
        self.memory
            .install_context(self.address_space, mapping, context)
            .map_err(|_| ())
    }

    fn release(&mut self, mapping: Mapping) {
        self.memory.release(self.address_space, mapping);
        if let Ok(space) = self.address_spaces.get_mut(self.address_space) {
            space.release(mapping)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KernelProcessError {
    Capacity,
    Capability,
    Scheduler,
    Memory,
    Loader(LoaderError),
    NotFound,
    InvalidTransition,
}

impl IntoStatus for KernelProcessError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::Capability => Status::ACCESS_DENIED,
            Self::Loader(error) => error.status(),
            Self::NotFound => Status::NOT_FOUND,
            Self::InvalidTransition => Status::INVALID_ARGUMENT,
            Self::Scheduler | Self::Memory => Status::BUSY,
        }
    }
}

const C_PROCESS_CAPACITY: usize = 64;

#[repr(C)]
#[derive(Clone, Copy)]
struct CProcessRecord {
    process_id: u64,
    thread_id: u32,
    address_space: u32,
    authority: u64,
    mapping_present: bool,
    mapping_base: u64,
    mapping_size: u64,
    limit_memory: u64,
    limit_cpu_time: u64,
    limit_deadline: u64,
    limit_cancel_grace: u64,
    usage_memory: u64,
    usage_cpu_time: u64,
    exit_present: bool,
    exit_status: i32,
    exit_kind: u8,
    crash_kind: u8,
    cancel_present: bool,
    cancel_time: u64,
}

impl CProcessRecord {
    const EMPTY: Self = Self {
        process_id: 0,
        thread_id: 0,
        address_space: 0,
        authority: 0,
        mapping_present: false,
        mapping_base: 0,
        mapping_size: 0,
        limit_memory: 0,
        limit_cpu_time: 0,
        limit_deadline: 0,
        limit_cancel_grace: 0,
        usage_memory: 0,
        usage_cpu_time: 0,
        exit_present: false,
        exit_status: 0,
        exit_kind: 0,
        crash_kind: 0,
        cancel_present: false,
        cancel_time: 0,
    };
}

#[repr(C)]
struct CProcessTable {
    capacity: usize,
    generations: [u32; C_PROCESS_CAPACITY],
    slots: [CProcessRecord; C_PROCESS_CAPACITY],
}

unsafe extern "C" {
    fn ghostos_process_table_init(table: *mut CProcessTable, capacity: usize);
    fn ghostos_process_free_slot(table: *const CProcessTable) -> usize;
    fn ghostos_process_new_identity(table: *mut CProcessTable, slot: usize, process: *mut u64, address_space: *mut u32) -> u32;
    fn ghostos_process_find(table: *const CProcessTable, process: u64) -> usize;
    fn ghostos_process_get(table: *const CProcessTable, slot: usize, out: *mut CProcessRecord) -> bool;
    fn ghostos_process_publish(table: *mut CProcessTable, slot: usize, record: *const CProcessRecord) -> u32;
    fn ghostos_process_exec(table: *mut CProcessTable, slot: usize, base: u64, size: u64, usage_memory: u64) -> u32;
    fn ghostos_process_cancel(table: *mut CProcessTable, slot: usize, now_us: u64) -> u32;
    fn ghostos_process_finish(table: *mut CProcessTable, slot: usize, status: i32, exit_kind: u8, crash_kind: u8) -> u32;
    fn ghostos_process_clear(table: *mut CProcessTable, slot: usize);
}

/// Kernel implementation of [`ProcessBackend`].
///
/// Every successful spawn follows the same order: the Ring 3 loader validates
/// and maps the image, the kernel creates a user thread for its returned
/// context, and only then does the backend publish the process id.
pub struct KernelProcessBackend<
    'a,
    M,
    const CAPACITY: usize = DEFAULT_KERNEL_PROCESS_CAPACITY,
    const MAX_CAPABILITIES: usize = { crate::MAX_CAPABILITIES },
> {
    scheduler: &'a mut Scheduler,
    capabilities: &'a mut CapabilitySpace<MAX_CAPABILITIES>,
    memory: M,
    caller: AddressSpaceId,
    processes: CProcessTable,
    address_spaces: AddressSpaceTable<CAPACITY>,
}

impl<
        'a,
        M: ProcessMemory,
        const CAPACITY: usize,
        const MAX_CAPABILITIES: usize,
    > KernelProcessBackend<'a, M, CAPACITY, MAX_CAPABILITIES>
{
    pub fn new(
        scheduler: &'a mut Scheduler,
        capabilities: &'a mut CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        authority: CapabilityHandle,
        memory: M,
    ) -> Result<Self, KernelProcessError> {
        let address_space_authorized = capabilities
            .authorize(
                caller,
                authority,
                CapabilityObject::AddressSpace(caller),
                Rights::CREATE,
            )
            .is_ok();
        let system_control_authorized = capabilities
            .authorize(caller, authority, CapabilityObject::SystemControl, Rights::CONTROL)
            .is_ok();
        if !address_space_authorized && !system_control_authorized {
            return Err(KernelProcessError::Capability)
        }
        if CAPACITY > C_PROCESS_CAPACITY {
            return Err(KernelProcessError::Capacity)
        }
        let mut processes = CProcessTable {
            capacity: 0,
            generations: [0; C_PROCESS_CAPACITY],
            slots: [CProcessRecord::EMPTY; C_PROCESS_CAPACITY],
        };
        // SAFETY: C initializes the fixed-size table with the checked capacity.
        unsafe { ghostos_process_table_init(&mut processes, CAPACITY) };
        Ok(Self {
            scheduler,
            capabilities,
            memory,
            caller,
            processes,
            address_spaces: AddressSpaceTable::new(),
        })
    }

    pub fn memory(&self) -> &M {
        &self.memory
    }

    pub fn memory_mut(&mut self) -> &mut M {
        &mut self.memory
    }

    pub fn scheduler(&self) -> &Scheduler {
        self.scheduler
    }

    pub fn scheduler_mut(&mut self) -> &mut Scheduler {
        self.scheduler
    }

    pub fn address_space(
        &self,
        address_space: AddressSpaceId,
    ) -> Result<&crate::AddressSpace, KernelProcessError> {
        self.address_spaces
            .get(address_space)
            .map_err(|_| KernelProcessError::NotFound)
    }

    /// Transfer control to a scheduled user thread. A hardware exception or
    /// interrupt returns through the architecture's saved user frame.
    pub fn enter(&self, process: ProcessId) -> Result<(), KernelProcessError> {
        let index = self.slot_index(process)?;
        let record = self.record(index)?;
        let thread = ThreadId::new(record.thread_id).ok_or(KernelProcessError::NotFound)?;
        let address_space = AddressSpaceId::new(record.address_space).ok_or(KernelProcessError::NotFound)?;
        let context = self
            .scheduler
            .thread(thread)
            .map_err(|_| KernelProcessError::Scheduler)?
            .context;
        let root = self
            .address_spaces
            .get(address_space)
            .map_err(|_| KernelProcessError::NotFound)?
            .root();
        crate::arch::enter_user(&context, root)
    }

    /// Return the kernel's lifecycle view of one process.
    pub fn status(&self, process: ProcessId) -> Result<ProcessStatus, KernelProcessError> {
        let index = self.slot_index(process)?;
        let record = self.record(index)?;
        let exit = record_exit(record)?;
        let state = match (record.exit_present, record.cancel_present) {
            (true, _) => ProcessState::Exited,
            (false, true) => ProcessState::Cancelling { requested_at_us: record.cancel_time },
            (false, false) => ProcessState::Running,
        };
        Ok(ProcessStatus {
            process,
            state,
            limits: ProcessLimits {
                memory_bytes: record.limit_memory,
                cpu_time_us: record.limit_cpu_time,
                deadline_us: record.limit_deadline,
                cancel_grace_us: record.limit_cancel_grace,
            },
            usage: ProcessUsage { memory_bytes: record.usage_memory, cpu_time_us: record.usage_cpu_time },
            exit,
        })
    }

    /// Record a faulting user process, persist a bounded crash capsule, then
    /// remove its runnable thread and address-space resources.
    pub fn crash(
        &mut self,
        process: ProcessId,
        reason: CrashReason,
        fault_address: u64,
    ) -> Result<(), KernelProcessError> {
        self.finish_with_fault(
            process,
            ProcessExit {
                status: -1,
                reason: ExitReason::Crash(reason),
            },
            fault_address,
        )
    }

    /// Record a user-mode exit and release its thread, mapping, and authority.
    pub fn exit(&mut self, process: ProcessId, status: i32) -> Result<(), KernelProcessError> {
        let reason = if status == 0 {
            ExitReason::Clean
        } else {
            ExitReason::Crash(CrashReason::UnexpectedExit)
        };
        self.finish(process, ProcessExit { status, reason })
    }

    fn free_slot(&self) -> Result<usize, KernelProcessError> {
        // SAFETY: C reads the initialized process table.
        let slot = unsafe { ghostos_process_free_slot(&self.processes) };
        (slot < self.processes.capacity).then_some(slot).ok_or(KernelProcessError::Capacity)
    }

    fn slot_index(&self, process: ProcessId) -> Result<usize, KernelProcessError> {
        // SAFETY: C reads the initialized process table.
        let slot = unsafe { ghostos_process_find(&self.processes, process.raw()) };
        (slot < self.processes.capacity).then_some(slot).ok_or(KernelProcessError::NotFound)
    }

    fn new_identity(&mut self, slot: usize) -> Result<(ProcessId, AddressSpaceId), KernelProcessError> {
        let mut process_raw = 0;
        let mut address_space_raw = 0;
        // SAFETY: C updates only the selected generation counter and writes both IDs.
        let error = unsafe {
            ghostos_process_new_identity(&mut self.processes, slot, &mut process_raw, &mut address_space_raw)
        };
        if error != 0 {
            return Err(KernelProcessError::Capacity)
        }
        let process = ProcessId::new(process_raw).ok_or(KernelProcessError::Capacity)?;
        let address_space = AddressSpaceId::new(address_space_raw).ok_or(KernelProcessError::Capacity)?;
        Ok((process, address_space))
    }

    fn record(&self, slot: usize) -> Result<CProcessRecord, KernelProcessError> {
        let mut record = CProcessRecord::EMPTY;
        // SAFETY: C copies one occupied process record into this output value.
        if unsafe { ghostos_process_get(&self.processes, slot, &mut record) } {
            Ok(record)
        } else {
            Err(KernelProcessError::NotFound)
        }
    }

    fn load(
        &mut self,
        address_space: AddressSpaceId,
        image: &[u8],
        architecture: ghostos_app::ImageArchitecture,
        expected_payload: Option<ContentId>,
        heap_bytes: u64,
        arguments: ProcessArguments<'_>,
    ) -> Result<LoadedImage, KernelProcessError> {
        let mut mapper = KernelImageMapper::new(
            &mut self.memory,
            &mut self.address_spaces,
            address_space,
        );
        load_image(
            &mut mapper,
            ImageLoadRequest {
                bytes: image,
                architecture,
                expected_payload,
                heap_bytes,
                stack: StackRequest {
                    size: DEFAULT_STACK_BYTES,
                    guard_pages: DEFAULT_GUARD_PAGES,
                    executable: false,
                },
                arguments,
            },
        )
        .map_err(KernelProcessError::Loader)
    }

    fn install_thread(
        &mut self,
        address_space: AddressSpaceId,
        authority: CapabilityHandle,
        context: ProcessContext,
    ) -> Result<ThreadId, KernelProcessError> {
        let entry = usize::try_from(context.entry).map_err(|_| KernelProcessError::Scheduler)?;
        let stack = usize::try_from(context.stack_pointer).map_err(|_| KernelProcessError::Scheduler)?;
        let root = self
            .address_spaces
            .get(address_space)
            .map_err(|_| KernelProcessError::Scheduler)?
            .root();
        self.scheduler
            .create_user(
                self.capabilities,
                self.caller,
                authority,
                address_space,
                root,
                SchedulingPolicy::Cooperative,
                entry,
                stack,
            )
            .map_err(|_| KernelProcessError::Scheduler)
    }

    fn finish(&mut self, process: ProcessId, exit: ProcessExit) -> Result<(), KernelProcessError> {
        self.finish_with_fault(process, exit, 0)
    }

    fn finish_with_fault(
        &mut self,
        process: ProcessId,
        exit: ProcessExit,
        fault_address: u64,
    ) -> Result<(), KernelProcessError> {
        let index = self.slot_index(process)?;
        let record = self.record(index)?;
        if record.exit_present {
            return Err(KernelProcessError::InvalidTransition)
        }
        let thread = ThreadId::new(record.thread_id).ok_or(KernelProcessError::NotFound)?;
        let address_space = AddressSpaceId::new(record.address_space).ok_or(KernelProcessError::NotFound)?;
        let authority = CapabilityHandle::from_raw(record.authority).ok_or(KernelProcessError::NotFound)?;
        if let ExitReason::Crash(reason) = exit.reason {
            self.report_crash(index, reason, fault_address)?;
        }
        self.scheduler
            .stop(self.capabilities, self.caller, authority, thread)
            .map_err(|_| KernelProcessError::Scheduler)?;
        if record.mapping_present {
            let mapping = Mapping { base: record.mapping_base, size: record.mapping_size };
            self.memory.release(address_space, mapping)
        }
        self.address_spaces
            .destroy(address_space)
            .map_err(|_| KernelProcessError::NotFound)?;
        self.memory.destroy_address_space(address_space);
        self.capabilities
            .delete(self.caller, authority)
            .map_err(|_| KernelProcessError::Capability)?;
        let (exit_kind, crash_kind) = encode_exit(exit.reason);
        // SAFETY: C commits the completed process transition after resource teardown.
        map_c_process_error(unsafe {
            ghostos_process_finish(&mut self.processes, index, exit.status, exit_kind, crash_kind)
        })
    }

    fn report_crash(
        &self,
        index: usize,
        reason: CrashReason,
        fault_address: u64,
    ) -> Result<(), KernelProcessError> {
        let record = self.record(index)?;
        let thread = ThreadId::new(record.thread_id).ok_or(KernelProcessError::NotFound)?;
        let context = self
            .scheduler
            .thread(thread)
            .map_err(|_| KernelProcessError::Scheduler)?
            .context;
        let mut registers = crate::crash::RegisterState::empty();
        registers.instruction_pointer = context.instruction_pointer as u64;
        registers.stack_pointer = context.stack_pointer as u64;
        for (destination, value) in registers.general.iter_mut().zip(context.registers) {
            *destination = value as u64;
        }
        let reason_code = match reason {
            CrashReason::Panic => 1,
            CrashReason::ProtectionFault => 2,
            CrashReason::IllegalInstruction => 3,
            CrashReason::Watchdog => 4,
            CrashReason::UnexpectedExit => 5,
        };
        crate::crash::capture_and_persist(
            registers,
            fault_address,
            Status::CORRUPT,
            0x100 | reason_code,
            Some(self.scheduler),
        );
        Ok(())
    }
}

impl<
        M: ProcessMemory,
        const CAPACITY: usize,
        const MAX_CAPABILITIES: usize,
    > ProcessBackend for KernelProcessBackend<'_, M, CAPACITY, MAX_CAPABILITIES>
{
    type Error = KernelProcessError;

    fn spawn(&mut self, request: NativeSpawnRequest<'_>) -> Result<ProcessId, Self::Error> {
        request
            .limits
            .validate()
            .map_err(|_| KernelProcessError::InvalidTransition)?;
        let index = self.free_slot()?;
        let (process, address_space) = self.new_identity(index)?;
        let root = self
            .memory
            .create_address_space(address_space)
            .map_err(|_| KernelProcessError::Memory)?;
        if self
            .address_spaces
            .create_with_aslr_seed(
                address_space,
                root,
                crate::random::next_u64().unwrap_or(0),
            )
            .is_err()
        {
            self.memory.destroy_address_space(address_space);
            return Err(KernelProcessError::Capacity)
        }
        let process_authority = self
            .capabilities
            .mint_root(
                self.caller,
                CapabilityObject::AddressSpace(address_space),
                Rights::ALL,
            )
            .map_err(|_| {
                let _ = self.address_spaces.destroy(address_space);
                self.memory.destroy_address_space(address_space);
                KernelProcessError::Capability
            })?;
        let loaded = match self.load(
            address_space,
            request.image,
            request.architecture,
            request.expected_payload,
            request.heap_bytes,
            request.arguments,
        ) {
            Ok(loaded) => loaded,
            Err(error) => {
                let _ = self.capabilities.delete(self.caller, process_authority);
                let _ = self.address_spaces.destroy(address_space);
                self.memory.destroy_address_space(address_space);
                return Err(error)
            }
        };
        let thread = match self.install_thread(address_space, process_authority, loaded.context) {
            Ok(thread) => thread,
            Err(error) => {
                self.memory.release(address_space, loaded.mapping);
                let _ = self.capabilities.delete(self.caller, process_authority);
                let _ = self.address_spaces.destroy(address_space);
                self.memory.destroy_address_space(address_space);
                return Err(error)
            }
        };
        let usage_memory = loaded.mapping.size
            .saturating_add(DEFAULT_STACK_BYTES)
            .saturating_add(request.heap_bytes);
        let record = CProcessRecord {
            process_id: process.raw(),
            thread_id: thread.raw(),
            address_space: address_space.raw(),
            authority: process_authority.raw(),
            mapping_present: true,
            mapping_base: loaded.mapping.base,
            mapping_size: loaded.mapping.size,
            limit_memory: request.limits.memory_bytes,
            limit_cpu_time: request.limits.cpu_time_us,
            limit_deadline: request.limits.deadline_us,
            limit_cancel_grace: request.limits.cancel_grace_us,
            usage_memory,
            usage_cpu_time: 0,
            exit_present: false,
            exit_status: 0,
            exit_kind: 0,
            crash_kind: 0,
            cancel_present: false,
            cancel_time: 0,
        };
        // SAFETY: C copies the initialized process record into the reserved slot.
        map_c_process_error(unsafe { ghostos_process_publish(&mut self.processes, index, &record) })?;
        Ok(process)
    }

    fn exec(
        &mut self,
        process: ProcessId,
        request: NativeExecRequest<'_>,
    ) -> Result<(), Self::Error> {
        let index = self.slot_index(process)?;
        let record = self.record(index)?;
        if record.exit_present {
            return Err(KernelProcessError::InvalidTransition)
        }
        let address_space = AddressSpaceId::new(record.address_space).ok_or(KernelProcessError::NotFound)?;
        let thread = ThreadId::new(record.thread_id).ok_or(KernelProcessError::NotFound)?;
        let loaded = self.load(
            address_space,
            request.image,
            request.architecture,
            request.expected_payload,
            request.heap_bytes,
            request.arguments,
        )?;
        let entry = match usize::try_from(loaded.context.entry) {
            Ok(entry) => entry,
            Err(_) => {
                self.memory.release(address_space, loaded.mapping);
                return Err(KernelProcessError::Scheduler)
            }
        };
        let stack = match usize::try_from(loaded.context.stack_pointer) {
            Ok(stack) => stack,
            Err(_) => {
                self.memory.release(address_space, loaded.mapping);
                return Err(KernelProcessError::Scheduler)
            }
        };
        if let Ok(context) = self.scheduler.context_mut(thread) {
            *context = Context::new(entry, stack);
        } else {
            self.memory.release(address_space, loaded.mapping);
            return Err(KernelProcessError::Scheduler)
        }
        if record.mapping_present {
            let mapping = Mapping { base: record.mapping_base, size: record.mapping_size };
            self.memory.release(address_space, mapping)
        }
        let usage_memory = loaded.mapping.size
            .saturating_add(DEFAULT_STACK_BYTES)
            .saturating_add(request.heap_bytes);
        // SAFETY: C updates mapping and cancellation state for the live process.
        map_c_process_error(unsafe {
            ghostos_process_exec(&mut self.processes, index, loaded.mapping.base, loaded.mapping.size, usage_memory)
        })
    }

    fn wait(&mut self, process: ProcessId) -> Result<Option<ProcessExit>, Self::Error> {
        let index = self.slot_index(process)?;
        record_exit(self.record(index)?)
    }

    fn usage(&mut self, process: ProcessId) -> Result<ProcessUsage, Self::Error> {
        let index = self.slot_index(process)?;
        let record = self.record(index)?;
        Ok(ProcessUsage { memory_bytes: record.usage_memory, cpu_time_us: record.usage_cpu_time })
    }

    fn request_cancel(&mut self, process: ProcessId) -> Result<(), Self::Error> {
        let index = self.slot_index(process)?;
        // SAFETY: C records the first cancellation request for the live process.
        map_c_process_error(unsafe {
            ghostos_process_cancel(&mut self.processes, index, self.scheduler.clock())
        })
    }

    fn fence(&mut self, process: ProcessId) -> Result<(), Self::Error> {
        self.finish(
            process,
            ProcessExit {
                status: -1,
                reason: ExitReason::Crash(CrashReason::Watchdog),
            },
        )?;
        let index = self.slot_index(process)?;
        // SAFETY: C clears the fenced process record, keeping its generation counter.
        unsafe { ghostos_process_clear(&mut self.processes, index) };
        Ok(())
    }
}

fn map_c_process_error(error: u32) -> Result<(), KernelProcessError> {
    match error {
        0 => Ok(()),
        1 => Err(KernelProcessError::Capacity),
        2 => Err(KernelProcessError::NotFound),
        _ => Err(KernelProcessError::InvalidTransition),
    }
}

fn encode_exit(reason: ExitReason) -> (u8, u8) {
    match reason {
        ExitReason::Clean => (1, 0),
        ExitReason::Crash(reason) => {
            let reason = match reason {
                CrashReason::Panic => 1,
                CrashReason::ProtectionFault => 2,
                CrashReason::IllegalInstruction => 3,
                CrashReason::Watchdog => 4,
                CrashReason::UnexpectedExit => 5,
            };
            (2, reason)
        }
    }
}

fn record_exit(record: CProcessRecord) -> Result<Option<ProcessExit>, KernelProcessError> {
    if !record.exit_present {
        return Ok(None)
    }
    let reason = match record.exit_kind {
        1 => ExitReason::Clean,
        2 => ExitReason::Crash(match record.crash_kind {
            1 => CrashReason::Panic,
            2 => CrashReason::ProtectionFault,
            3 => CrashReason::IllegalInstruction,
            4 => CrashReason::Watchdog,
            5 => CrashReason::UnexpectedExit,
            _ => return Err(KernelProcessError::InvalidTransition),
        }),
        _ => return Err(KernelProcessError::InvalidTransition),
    };
    Ok(Some(ProcessExit { status: record.exit_status, reason }))
}
