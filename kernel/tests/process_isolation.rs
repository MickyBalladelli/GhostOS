use std::vec::Vec;

use synos_app::{
    ImageArchitecture, Mapping, MappingRequest, NativeSpawnRequest, ProcessArguments,
    ProcessBackend, ProcessContext, ProcessLimits, ProcessState, RuntimeSegment,
    SegmentPermissions, StackRequest, TlsRequest,
};
use synos_kernel::{
    AddressSpaceId, CapabilityObject, CapabilitySpace, MemoryAccess, PageTableRoot,
    KernelProcessBackend, ProcessMemory, Rights, Scheduler, USER_SPACE_END,
};

const CAPABILITIES: usize = 16;

struct MemoryFixture {
    next_mapping: u64,
    created: Vec<AddressSpaceId>,
    destroyed: Vec<AddressSpaceId>,
    mapped_segments: Vec<(AddressSpaceId, RuntimeSegment)>,
    released: Vec<(AddressSpaceId, Mapping)>,
}

impl MemoryFixture {
    fn new() -> Self {
        Self {
            next_mapping: 0,
            created: Vec::new(),
            destroyed: Vec::new(),
            mapped_segments: Vec::new(),
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
        Ok(PageTableRoot::new(frame).expect("valid page-table frame"))
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
        address_space: AddressSpaceId,
        _mapping: Mapping,
        segment: RuntimeSegment,
        _source: &[u8],
    ) -> Result<(), Self::Error> {
        self.mapped_segments.push((address_space, segment));
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
        Ok(USER_SPACE_END - request.size)
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
        Ok(synos_kernel::USER_SPACE_START + 0x4000_0000)
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
    bytes: Vec<u8>,
}

impl ImageFixture {
    fn new() -> Self {
        let mut bytes = vec![0u8; 0x1001];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 1;
        bytes[6] = 1;
        bytes[16..18].copy_from_slice(&3u16.to_le_bytes());
        bytes[18..20].copy_from_slice(&62u16.to_le_bytes());
        bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
        bytes[24..32].copy_from_slice(&0x1000u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
        bytes[52..54].copy_from_slice(&64u16.to_le_bytes());
        bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
        bytes[56..58].copy_from_slice(&1u16.to_le_bytes());

        let program = &mut bytes[64..120];
        program[0..4].copy_from_slice(&1u32.to_le_bytes());
        program[4..8].copy_from_slice(&5u32.to_le_bytes());
        program[8..16].copy_from_slice(&0x1000u64.to_le_bytes());
        program[16..24].copy_from_slice(&0x1000u64.to_le_bytes());
        program[32..40].copy_from_slice(&1u64.to_le_bytes());
        program[40..48].copy_from_slice(&1u64.to_le_bytes());
        program[48..56].copy_from_slice(&0x1000u64.to_le_bytes());
        bytes[0x1000] = 0xf4;
        Self { bytes }
    }
}

fn spawn_request<'a>(image: &'a ImageFixture) -> NativeSpawnRequest<'a> {
    NativeSpawnRequest {
        image: &image.bytes,
        architecture: ImageArchitecture::X86_64,
        expected_payload: None,
        heap_bytes: synos_app::DEFAULT_HEAP_BYTES,
        arguments: ProcessArguments {
            argv: &[],
            environment: &[],
        },
        limits: ProcessLimits::DEFAULT,
    }
}

fn backend() -> KernelProcessBackend<'static, MemoryFixture, 2, CAPABILITIES> {
    let scheduler = Box::leak(Box::new(Scheduler::new()));
    let capabilities = Box::leak(Box::new(CapabilitySpace::<CAPABILITIES>::new()));
    let authority = capabilities
        .mint_root(
            AddressSpaceId::KERNEL,
            CapabilityObject::AddressSpace(AddressSpaceId::KERNEL),
            Rights::ALL,
        )
        .expect("kernel authority");
    synos_kernel::KernelProcessBackend::new(
        scheduler,
        capabilities,
        AddressSpaceId::KERNEL,
        authority,
        MemoryFixture::new(),
    )
    .expect("process backend")
}

#[test]
fn native_processes_get_distinct_page_tables_and_private_mappings() {
    let image = ImageFixture::new();
    let mut backend = backend();
    let first = backend.spawn(spawn_request(&image)).expect("first process");
    let second = backend.spawn(spawn_request(&image)).expect("second process");

    let first_space = backend.memory().created[0];
    let second_space = backend.memory().created[1];
    let first_root = backend.address_space(first_space).unwrap().root();
    let second_root = backend.address_space(second_space).unwrap().root();
    let first_segment = backend
        .memory()
        .mapped_segments
        .iter()
        .find(|(owner, _)| *owner == first_space)
        .map(|(_, segment)| *segment)
        .expect("first process segment");
    let second_segment = backend
        .memory()
        .mapped_segments
        .iter()
        .find(|(owner, _)| *owner == second_space)
        .map(|(_, segment)| *segment)
        .expect("second process segment");

    assert_ne!(first, second);
    assert_ne!(first_root, second_root);
    assert_ne!(first_segment.address, second_segment.address);
    assert!(backend
        .address_space(first_space)
        .unwrap()
        .can_access(first_segment.address, 1, MemoryAccess::Read));
    assert!(backend
        .address_space(first_space)
        .unwrap()
        .can_access(first_segment.address, 1, MemoryAccess::Execute));
    assert!(!backend
        .address_space(first_space)
        .unwrap()
        .can_access(first_segment.address, 1, MemoryAccess::Write));
    assert!(!backend
        .address_space(first_space)
        .unwrap()
        .can_access(second_segment.address, 1, MemoryAccess::Read));
    assert!(!backend
        .address_space(first_space)
        .unwrap()
        .can_access(second_segment.address, 1, MemoryAccess::Write));
    assert!(!backend
        .address_space(first_space)
        .unwrap()
        .can_access(second_segment.address, 1, MemoryAccess::Execute));
    assert!(backend
        .address_space(second_space)
        .unwrap()
        .can_access(second_segment.address, 1, MemoryAccess::Read));
}

#[test]
fn fencing_one_process_reclaims_only_its_isolation_resources() {
    let image = ImageFixture::new();
    let mut backend = backend();
    let first = backend.spawn(spawn_request(&image)).expect("first process");
    let second = backend.spawn(spawn_request(&image)).expect("second process");
    let first_space = backend.memory().created[0];
    let second_space = backend.memory().created[1];

    backend.fence(first).expect("fence first process");

    assert!(backend.address_space(first_space).is_err());
    assert!(backend.address_space(second_space).is_ok());
    assert_eq!(backend.status(second).unwrap().state, ProcessState::Running);
    assert_eq!(backend.memory().destroyed, vec![first_space]);
    assert!(backend
        .memory()
        .released
        .iter()
        .any(|(owner, _)| *owner == first_space));
    assert!(!backend
        .memory()
        .released
        .iter()
        .any(|(owner, _)| *owner == second_space));
}
