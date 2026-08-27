use ghostos_debug::{Error, coredump::{CoreDumpEngine, CoreDumpRequest, CoreDumpRuntime, FrozenPage}, gdb::{DebugAuthority, DebugOperation, DebugRuntime, DebugToken, DsmFault, GdbStub, RegisterFile, StopReason}, probes::{Instruction, ProbeProgram, ProbeRecorder, ProbeSample, ProbeVm, SampleField}};
use ghostos_fabric::NodeId;
use ghostos_init::{CrashReason, ProcessId};
use ghostos_ghostfs::SynFs;

fn packet(payload: &[u8]) -> Vec<u8> {
    let checksum = payload.iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
    let mut result = Vec::with_capacity(payload.len() + 4);
    result.push(b'$');
    result.extend_from_slice(payload);
    result.extend_from_slice(format!("#{checksum:02x}").as_bytes());
    result
}

#[test]
fn probes_execute_bounded_arithmetic_and_reject_backward_jumps() {
    let mut program = ProbeProgram::new();
    program.push(Instruction::load_field(0, SampleField::Value0)).unwrap();
    program.push(Instruction::load_immediate(1, 2)).unwrap();
    program.push(Instruction::binary(ghostos_debug::probes::Opcode::Add, 0, 1)).unwrap();
    program.push(Instruction::EMIT).unwrap();
    program.push(Instruction::HALT).unwrap();
    let sample = ProbeSample::ipc_stream(7, NodeId::LOCAL, 4, 40, 2, 0);
    let mut vm = ProbeVm::new(program, ProbeRecorder::<2>::new()).unwrap();
    vm.run(sample).unwrap();
    assert_eq!(vm.sink().records().next().unwrap().value, 42);

    let mut invalid = ProbeProgram::new();
    invalid.push(Instruction::jump_if_zero(0, 0)).unwrap();
    invalid.push(Instruction::HALT).unwrap();
    assert!(matches!(ProbeVm::new(invalid, ProbeRecorder::<1>::new()), Err(Error::InvalidProgram)));
}

struct Runtime {
    registers: RegisterFile,
    memory: [u8; 8],
    continued: bool,
    stepped: bool,
}

impl Default for Runtime {
    fn default() -> Self {
        Self {
            registers: RegisterFile::empty(),
            memory: [0; 8],
            continued: false,
            stepped: false,
        }
    }
}

impl DebugRuntime for Runtime {
    fn stop_reason(&mut self) -> StopReason { StopReason::Signal(11) }
    fn registers(&mut self) -> Result<RegisterFile, Error> { Ok(self.registers) }
    fn write_registers(&mut self, registers: RegisterFile) -> Result<(), Error> { self.registers = registers; Ok(()) }
    fn read_memory(&mut self, _address: u64, destination: &mut [u8]) -> Result<usize, Error> {
        let count = destination.len().min(self.memory.len());
        destination[..count].copy_from_slice(&self.memory[..count]);
        Ok(count)
    }
    fn write_memory(&mut self, _address: u64, bytes: &[u8]) -> Result<(), Error> {
        self.memory[..bytes.len()].copy_from_slice(bytes);
        Ok(())
    }
    fn continue_execution(&mut self) -> Result<(), Error> { self.continued = true; Ok(()) }
    fn step_execution(&mut self) -> Result<(), Error> { self.stepped = true; Ok(()) }
    fn dsm_faults(&mut self, _destination: &mut [DsmFault]) -> usize { 0 }
}

struct ReadOnly;

impl DebugAuthority for ReadOnly {
    fn permits(&self, _token: DebugToken, operation: DebugOperation) -> bool {
        operation == DebugOperation::Read
    }
}

#[test]
fn gdb_handles_partial_frames_and_enforces_capabilities() {
    let process_registers = RegisterFile::from_slice(&[0x11, 0x22]).unwrap();
    let runtime = Runtime { registers: process_registers, memory: [1, 2, 3, 4, 5, 6, 7, 8], ..Runtime::default() };
    let token = DebugToken::new(9).unwrap();
    let mut stub = GdbStub::new(runtime, ReadOnly, token);
    let frame = packet(b"m0,4");
    let mut output = [0; 64];
    assert_eq!(stub.ingest(&frame[..3], &mut output), Ok(None));
    let length = stub.ingest(&frame[3..], &mut output).unwrap().unwrap();
    assert_eq!(&output[..length], b"+$01020304#0a");
    let length = stub.ingest(&packet(b"M0,1:ff"), &mut output).unwrap().unwrap();
    assert_eq!(&output[..length], b"+$E03#a8");
}

struct Frozen {
    registers: RegisterFile,
    pages: Vec<Vec<u8>>,
}

impl ghostos_debug::coredump::FrozenProcess for Frozen {
    fn registers(&self) -> RegisterFile { self.registers }
    fn page_count(&self) -> usize { self.pages.len() }
    fn read_page(&self, index: usize, destination: &mut [u8]) -> Result<FrozenPage, Error> {
        let page = &self.pages[index];
        destination[..page.len()].copy_from_slice(page);
        Ok(FrozenPage { virtual_address: 0x4000 + index as u64 * 4096, length: page.len() })
    }
}

struct FreezeRuntime;

impl CoreDumpRuntime for FreezeRuntime {
    type Frozen = Frozen;
    fn freeze(&mut self, _process: ProcessId, _reason: CrashReason) -> Result<Self::Frozen, Error> {
        Ok(Frozen { registers: RegisterFile::from_slice(&[1, 2]).unwrap(), pages: vec![vec![9, 8, 7]] })
    }
}

#[test]
fn coredump_commits_metadata_and_page_payload() {
    let mut filesystem = SynFs::<64>::new();
    let process = ProcessId::new(3).unwrap();
    let receipt = CoreDumpEngine::capture(
        &mut FreezeRuntime,
        &mut filesystem,
        CoreDumpRequest { process, reason: CrashReason::Panic, timestamp_us: 99, directory: "/cores/3" },
    ).unwrap();
    assert_eq!(receipt.pages, 1);
    let mut metadata = [0; ghostos_debug::coredump::CORE_METADATA_BYTES];
    filesystem.read("/cores/3/META", &mut metadata).unwrap();
    assert_eq!(&metadata[..8], b"SYNCORE1");
    let mut page = [0; 32];
    let read = filesystem.read("/cores/3/PAGE-00000000", &mut page).unwrap();
    assert_eq!(read.bytes_read, 19);
    assert_eq!(&page[16..19], &[9, 8, 7]);
}
