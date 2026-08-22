use std::vec::Vec;

use ghostos_app::{
    ImageArchitecture, Mapping, MappingRequest, ProcessArguments, ProcessContext, ProcessLimits,
    RuntimeSegment, SegmentPermissions, StackRequest, TlsRequest, DEFAULT_HEAP_BYTES,
};
use ghostos_init::{
    CrashReason, ExitReason, RestartPolicy, ServiceId, ServiceKind, ServiceName, ServiceSpec,
    ServiceState, Supervisor, SupervisorEvent,
};
use ghostos_kernel::{
    AddressSpaceId, CapabilityObject, CapabilitySpace, KernelProcessBackend, KernelSupervisorRuntime,
    NativeServiceImage, PageTableRoot, ProcessMemory, Rights, Scheduler,
};

const CAPABILITIES: usize = 64;

struct MemoryFixture {
    next_mapping: u64,
    created: Vec<AddressSpaceId>,
    destroyed: Vec<AddressSpaceId>,
    released: Vec<(AddressSpaceId, Mapping)>,
}

impl MemoryFixture {
    fn new() -> Self {
        Self {
            next_mapping: 0,
            created: Vec::new(),
            destroyed: Vec::new(),
            released: Vec::new(),
        }
    }
}

impl ProcessMemory for MemoryFixture {
    type Error = ();

    fn create_address_space(
        &mut self,
        address_space: AddressSpaceId,
    ) -> Result<PageTableRoot, Self::Error> {
        self.created.push(address_space);
        let frame = (self.created.len() as u64 + 1) * 0x1000;
        Ok(PageTableRoot::new(frame).unwrap())
    }

    fn destroy_address_space(&mut self, address_space: AddressSpaceId) {
        self.destroyed.push(address_space);
    }

    fn reserve(
        &mut self,
        _address_space: AddressSpaceId,
        request: MappingRequest,
    ) -> Result<Mapping, Self::Error> {
        self.next_mapping += 1;
        Ok(Mapping {
            base: 0x0000_0080_0000_0000 + self.next_mapping * 0x0100_0000,
            size: request.size,
        })
    }

    fn map_segment(
        &mut self,
        _address_space: AddressSpaceId,
        _mapping: Mapping,
        _segment: RuntimeSegment,
        _source: &[u8],
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn zero_fill(
        &mut self,
        _address_space: AddressSpaceId,
        _mapping: Mapping,
        _address: u64,
        _length: u64,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn apply_relative_relocation(
        &mut self,
        _address_space: AddressSpaceId,
        _mapping: Mapping,
        _address: u64,
        _addend: i64,
        _addend_from_memory: bool,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn protect(
        &mut self,
        _address_space: AddressSpaceId,
        _mapping: Mapping,
        _address: u64,
        _length: u64,
        _permissions: SegmentPermissions,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn allocate_stack(
        &mut self,
        _address_space: AddressSpaceId,
        _mapping: Mapping,
        request: StackRequest,
        _arguments: ProcessArguments<'_>,
    ) -> Result<u64, Self::Error> {
        Ok(ghostos_kernel::USER_SPACE_END - request.size)
    }

    fn allocate_heap(
        &mut self,
        _address_space: AddressSpaceId,
        mapping: Mapping,
        _size: u64,
    ) -> Result<u64, Self::Error> {
        Ok(mapping.base + mapping.size + 0x1000)
    }

    fn allocate_tls(
        &mut self,
        _address_space: AddressSpaceId,
        _mapping: Mapping,
        _request: TlsRequest,
        _source: &[u8],
    ) -> Result<u64, Self::Error> {
        Ok(ghostos_kernel::USER_SPACE_START + 0x4000_0000)
    }

    fn install_context(
        &mut self,
        _address_space: AddressSpaceId,
        _mapping: Mapping,
        _context: ProcessContext,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn release(&mut self, address_space: AddressSpaceId, mapping: Mapping) {
        self.released.push((address_space, mapping));
    }
}

struct ImageFixture {
    image: Vec<u8>,
}

impl ImageFixture {
    fn new() -> Self {
        let mut image = vec![0u8; 0x1001];
        image[0..4].copy_from_slice(b"\x7fELF");
        image[4] = 2;
        image[5] = 1;
        image[6] = 1;
        image[16..18].copy_from_slice(&3u16.to_le_bytes());
        image[18..20].copy_from_slice(&62u16.to_le_bytes());
        image[20..24].copy_from_slice(&1u32.to_le_bytes());
        image[24..32].copy_from_slice(&0x1000u64.to_le_bytes());
        image[32..40].copy_from_slice(&64u64.to_le_bytes());
        image[52..54].copy_from_slice(&64u16.to_le_bytes());
        image[54..56].copy_from_slice(&56u16.to_le_bytes());
        image[56..58].copy_from_slice(&1u16.to_le_bytes());

        let program = &mut image[64..120];
        program[0..4].copy_from_slice(&1u32.to_le_bytes());
        program[4..8].copy_from_slice(&5u32.to_le_bytes());
        program[8..16].copy_from_slice(&0x1000u64.to_le_bytes());
        program[16..24].copy_from_slice(&0x1000u64.to_le_bytes());
        program[32..40].copy_from_slice(&1u64.to_le_bytes());
        program[40..48].copy_from_slice(&1u64.to_le_bytes());
        program[48..56].copy_from_slice(&0x1000u64.to_le_bytes());
        image[0x1000] = 0xf4;
        Self { image }
    }
}

impl ghostos_kernel::ServiceImageProvider for ImageFixture {
    fn image(&self, image_id: u128) -> Option<NativeServiceImage<'_>> {
        (image_id == 1).then_some(NativeServiceImage {
            bytes: &self.image,
            architecture: ImageArchitecture::X86_64,
            heap_bytes: DEFAULT_HEAP_BYTES,
            limits: ProcessLimits::DEFAULT,
        })
    }
}

fn service(restart: RestartPolicy) -> ServiceSpec {
    ServiceSpec {
        id: ServiceId::new(1).unwrap(),
        name: ServiceName::new("integration-service").unwrap(),
        kind: ServiceKind::System,
        image_id: 1,
        capability_profile: 1,
        restart,
    }
}

#[test]
fn service_start_loads_a_ring3_image_into_a_private_process() {
    let mut scheduler = Scheduler::new();
    let mut capabilities = CapabilitySpace::<CAPABILITIES>::new();
    let images = ImageFixture::new();
    let authority = capabilities
        .mint_root(
            AddressSpaceId::KERNEL,
            CapabilityObject::AddressSpace(AddressSpaceId::KERNEL),
            Rights::ALL,
        )
        .unwrap();
    let mut backend = KernelProcessBackend::<_, 1, CAPABILITIES>::new(
        &mut scheduler,
        &mut capabilities,
        AddressSpaceId::KERNEL,
        authority,
        MemoryFixture::new(),
    )
    .unwrap();
    let mut runtime = KernelSupervisorRuntime::new(&mut backend, &images);
    let mut supervisor = Supervisor::<1>::new();
    let service = service(RestartPolicy::NEVER);
    supervisor.register(service).unwrap();

    let process = match supervisor.start(service.id, &mut runtime).unwrap() {
        SupervisorEvent::Started {
            process,
            generation,
            ..
        } => {
            assert_eq!(generation, 1);
            process
        }
        event => panic!("unexpected service start event: {event:?}"),
    };

    assert_eq!(supervisor.status(service.id).unwrap().state, ServiceState::Running);
    assert_eq!(runtime.backend().status(process).unwrap().process, process);
    assert!(runtime
        .backend()
        .address_space(AddressSpaceId::new(1).unwrap())
        .is_ok());
    assert_eq!(runtime.backend().memory().created, vec![AddressSpaceId::new(1).unwrap()]);
}

#[test]
fn service_restart_fences_the_old_process_and_starts_a_new_generation() {
    let mut scheduler = Scheduler::new();
    let mut capabilities = CapabilitySpace::<CAPABILITIES>::new();
    let images = ImageFixture::new();
    let authority = capabilities
        .mint_root(
            AddressSpaceId::KERNEL,
            CapabilityObject::AddressSpace(AddressSpaceId::KERNEL),
            Rights::ALL,
        )
        .unwrap();
    let mut backend = KernelProcessBackend::<_, 1, CAPABILITIES>::new(
        &mut scheduler,
        &mut capabilities,
        AddressSpaceId::KERNEL,
        authority,
        MemoryFixture::new(),
    )
    .unwrap();
    let mut runtime = KernelSupervisorRuntime::new(&mut backend, &images);
    let mut supervisor = Supervisor::<1>::new();
    let service = service(RestartPolicy::on_failure(1, 100, 10, 10).unwrap());
    supervisor.register(service).unwrap();
    let first = match supervisor.start(service.id, &mut runtime).unwrap() {
        SupervisorEvent::Started { process, .. } => process,
        event => panic!("unexpected service start event: {event:?}"),
    };

    assert!(matches!(
        supervisor.report_exit(
            first,
            ExitReason::Crash(CrashReason::ProtectionFault),
            0,
            &mut runtime,
        ),
        Ok(SupervisorEvent::RestartScheduled { at_us: 10, .. })
    ));
    assert_eq!(runtime.backend().memory().destroyed.len(), 1);

    let second = match supervisor.tick(10, &mut runtime).unwrap() {
        Some(SupervisorEvent::Started {
            process,
            generation,
            ..
        }) => {
            assert_eq!(generation, 2);
            process
        }
        event => panic!("unexpected service restart event: {event:?}"),
    };

    assert_ne!(first, second);
    assert_eq!(supervisor.status(service.id).unwrap().state, ServiceState::Running);
    assert!(runtime
        .backend()
        .address_space(AddressSpaceId::new(1).unwrap())
        .is_ok());
    assert_eq!(runtime.backend().memory().created.len(), 2);
    assert_eq!(runtime.backend().memory().destroyed.len(), 1);
}
