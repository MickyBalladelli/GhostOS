//! Kernel-owned adapter for native application images and process lifetime.

use core::convert::TryFrom;

use synos_app::{
    ImageLoadRequest, ImageMapper, LoadedImage, Mapping, MappingRequest, NativeExecRequest,
    NativeSpawnRequest, ProcessArguments, ProcessBackend, ProcessContext, ProcessExit,
    ProcessUsage,
    RuntimeSegment, SegmentPermissions, StackRequest, TlsRequest, load_image, DEFAULT_GUARD_PAGES,
    DEFAULT_STACK_BYTES, LoaderError,
};
use synos_init::{CrashReason, ExitReason, ProcessId};
use synos_status::{IntoStatus, Status};
use synos_system_model::ContentId;

use crate::{
    AddressSpaceId, CapabilityHandle, CapabilityObject, CapabilitySpace, Context, ExecutionMode,
    Rights, Scheduler, SchedulingPolicy, ThreadId,
};

pub const DEFAULT_KERNEL_PROCESS_CAPACITY: usize = 64;

/// Privileged memory operations needed by the application image loader.
///
/// An implementation owns the page tables and physical frames for an address
/// space. The process backend below supplies the address-space identity and
/// keeps the ELF parser independent from those privileged details.
pub trait ProcessMemory {
    type Error;

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

struct KernelImageMapper<'a, M> {
    memory: &'a mut M,
    address_space: AddressSpaceId,
}

impl<'a, M> KernelImageMapper<'a, M> {
    fn new(memory: &'a mut M, address_space: AddressSpaceId) -> Self {
        Self {
            memory,
            address_space,
        }
    }
}

impl<M: ProcessMemory> ImageMapper for KernelImageMapper<'_, M> {
    type Error = M::Error;

    fn reserve(&mut self, request: MappingRequest) -> Result<Mapping, Self::Error> {
        self.memory.reserve(self.address_space, request)
    }

    fn map_segment(
        &mut self,
        mapping: Mapping,
        segment: RuntimeSegment,
        source: &[u8],
    ) -> Result<(), Self::Error> {
        self.memory
            .map_segment(self.address_space, mapping, segment, source)
    }

    fn zero_fill(
        &mut self,
        mapping: Mapping,
        address: u64,
        length: u64,
    ) -> Result<(), Self::Error> {
        self.memory
            .zero_fill(self.address_space, mapping, address, length)
    }

    fn apply_relative_relocation(
        &mut self,
        mapping: Mapping,
        address: u64,
        addend: i64,
        addend_from_memory: bool,
    ) -> Result<(), Self::Error> {
        self.memory.apply_relative_relocation(
            self.address_space,
            mapping,
            address,
            addend,
            addend_from_memory,
        )
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
    }

    fn allocate_stack(
        &mut self,
        mapping: Mapping,
        request: StackRequest,
        arguments: ProcessArguments<'_>,
    ) -> Result<u64, Self::Error> {
        self.memory
            .allocate_stack(self.address_space, mapping, request, arguments)
    }

    fn allocate_heap(&mut self, mapping: Mapping, size: u64) -> Result<u64, Self::Error> {
        self.memory.allocate_heap(self.address_space, mapping, size)
    }

    fn allocate_tls(
        &mut self,
        mapping: Mapping,
        request: TlsRequest,
        source: &[u8],
    ) -> Result<u64, Self::Error> {
        self.memory
            .allocate_tls(self.address_space, mapping, request, source)
    }

    fn install_context(
        &mut self,
        mapping: Mapping,
        context: ProcessContext,
    ) -> Result<(), Self::Error> {
        self.memory
            .install_context(self.address_space, mapping, context)
    }

    fn release(&mut self, mapping: Mapping) {
        self.memory.release(self.address_space, mapping)
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

#[derive(Clone, Copy)]
struct ProcessSlot {
    process: Option<ProcessId>,
    thread: Option<ThreadId>,
    address_space: Option<AddressSpaceId>,
    authority: Option<CapabilityHandle>,
    mapping: Option<Mapping>,
    usage: ProcessUsage,
    exit: Option<ProcessExit>,
    cancel_requested: bool,
}

impl ProcessSlot {
    const EMPTY: Self = Self {
        process: None,
        thread: None,
        address_space: None,
        authority: None,
        mapping: None,
        usage: ProcessUsage {
            memory_bytes: 0,
            cpu_time_us: 0,
        },
        exit: None,
        cancel_requested: false,
    };
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
    generations: [u32; CAPACITY],
    slots: [ProcessSlot; CAPACITY],
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
        Ok(Self {
            scheduler,
            capabilities,
            memory,
            caller,
            generations: [0; CAPACITY],
            slots: [ProcessSlot::EMPTY; CAPACITY],
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
        self.slots
            .iter()
            .position(|slot| slot.process.is_none())
            .ok_or(KernelProcessError::Capacity)
    }

    fn slot_index(&self, process: ProcessId) -> Result<usize, KernelProcessError> {
        self.slots
            .iter()
            .position(|slot| slot.process == Some(process))
            .ok_or(KernelProcessError::NotFound)
    }

    fn new_identity(&mut self, slot: usize) -> Result<(ProcessId, AddressSpaceId), KernelProcessError> {
        let generation = self.generations[slot].wrapping_add(1).max(1);
        self.generations[slot] = generation;
        let process_raw = ((generation as u64) << 32) | (slot as u64 + 1);
        let process = ProcessId::new(process_raw).ok_or(KernelProcessError::Capacity)?;
        let address_space = AddressSpaceId::new(process_raw as u32).ok_or(KernelProcessError::Capacity)?;
        Ok((process, address_space))
    }

    fn load(
        &mut self,
        address_space: AddressSpaceId,
        image: &[u8],
        architecture: synos_app::ImageArchitecture,
        expected_payload: Option<ContentId>,
        heap_bytes: u64,
        arguments: ProcessArguments<'_>,
    ) -> Result<LoadedImage, KernelProcessError> {
        let mut mapper = KernelImageMapper::new(&mut self.memory, address_space);
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
        self.scheduler
            .create(
                self.capabilities,
                self.caller,
                authority,
                address_space,
                ExecutionMode::User,
                SchedulingPolicy::Cooperative,
                entry,
                stack,
            )
            .map_err(|_| KernelProcessError::Scheduler)
    }

    fn finish(&mut self, process: ProcessId, exit: ProcessExit) -> Result<(), KernelProcessError> {
        let index = self.slot_index(process)?;
        let thread = self.slots[index].thread.ok_or(KernelProcessError::NotFound)?;
        let address_space = self.slots[index]
            .address_space
            .ok_or(KernelProcessError::NotFound)?;
        let authority = self.slots[index]
            .authority
            .ok_or(KernelProcessError::NotFound)?;
        self.scheduler
            .stop(self.capabilities, self.caller, authority, thread)
            .map_err(|_| KernelProcessError::Scheduler)?;
        if let Some(mapping) = self.slots[index].mapping {
            self.memory.release(address_space, mapping)
        }
        self.capabilities
            .delete(self.caller, authority)
            .map_err(|_| KernelProcessError::Capability)?;
        self.slots[index].exit = Some(exit);
        self.slots[index].mapping = None;
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
        let process_authority = self
            .capabilities
            .mint_root(
                self.caller,
                CapabilityObject::AddressSpace(address_space),
                Rights::ALL,
            )
            .map_err(|_| KernelProcessError::Capability)?;
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
                return Err(error)
            }
        };
        let thread = match self.install_thread(address_space, process_authority, loaded.context) {
            Ok(thread) => thread,
            Err(error) => {
                self.memory.release(address_space, loaded.mapping);
                let _ = self.capabilities.delete(self.caller, process_authority);
                return Err(error)
            }
        };
        self.slots[index] = ProcessSlot {
            process: Some(process),
            thread: Some(thread),
            address_space: Some(address_space),
            authority: Some(process_authority),
            mapping: Some(loaded.mapping),
            usage: ProcessUsage {
                memory_bytes: loaded
                    .mapping
                    .size
                    .saturating_add(DEFAULT_STACK_BYTES)
                    .saturating_add(request.heap_bytes),
                cpu_time_us: 0,
            },
            exit: None,
            cancel_requested: false,
        };
        Ok(process)
    }

    fn exec(
        &mut self,
        process: ProcessId,
        request: NativeExecRequest<'_>,
    ) -> Result<(), Self::Error> {
        let index = self.slot_index(process)?;
        if self.slots[index].exit.is_some() {
            return Err(KernelProcessError::InvalidTransition)
        }
        let address_space = self.slots[index]
            .address_space
            .ok_or(KernelProcessError::NotFound)?;
        let thread = self.slots[index].thread.ok_or(KernelProcessError::NotFound)?;
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
        if let Some(mapping) = self.slots[index].mapping {
            self.memory.release(address_space, mapping)
        }
        self.slots[index].mapping = Some(loaded.mapping);
        self.slots[index].usage.memory_bytes = loaded
            .mapping
            .size
            .saturating_add(DEFAULT_STACK_BYTES)
            .saturating_add(request.heap_bytes);
        self.slots[index].cancel_requested = false;
        Ok(())
    }

    fn wait(&mut self, process: ProcessId) -> Result<Option<ProcessExit>, Self::Error> {
        let index = self.slot_index(process)?;
        Ok(self.slots[index].exit)
    }

    fn usage(&mut self, process: ProcessId) -> Result<ProcessUsage, Self::Error> {
        let index = self.slot_index(process)?;
        Ok(self.slots[index].usage)
    }

    fn request_cancel(&mut self, process: ProcessId) -> Result<(), Self::Error> {
        let index = self.slot_index(process)?;
        if self.slots[index].exit.is_some() {
            return Err(KernelProcessError::InvalidTransition)
        }
        self.slots[index].cancel_requested = true;
        Ok(())
    }

    fn fence(&mut self, process: ProcessId) -> Result<(), Self::Error> {
        self.finish(
            process,
            ProcessExit {
                status: -1,
                reason: ExitReason::Crash(CrashReason::Watchdog),
            },
        )
    }
}
