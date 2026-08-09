//! Differential CPU checks against small architectural reference models.
//!
//! The reference models intentionally cover only the instruction/state under
//! test. They make the expected x86 result explicit without depending on the
//! host CPU or an external emulator.

use std::cell::RefCell;
use std::rc::Rc;

use synos_vm::devices::{Device, DeviceError, InterruptController, PortBus, PortDevice};
use synos_vm::firmware::bios::BiosContext;
use synos_vm::{
    Cpu, CpuError, CpuMode, ExecutionEngine, ExecutionEngineConfig, Mmu, PageFlags,
    PrivilegeLevel,
};

const CODE: u64 = 0x1000;
const CF: u64 = 1 << 0;
const PF: u64 = 1 << 2;
const AF: u64 = 1 << 4;
const ZF: u64 = 1 << 6;
const SF: u64 = 1 << 7;
const IF: u64 = 1 << 9;
const DF: u64 = 1 << 10;
const OF: u64 = 1 << 11;
const ARITHMETIC_FLAGS: u64 = CF | PF | AF | ZF | SF | OF;

fn fixture(bytes: &[u8]) -> (Cpu, Mmu, InterruptController, PortBus, BiosContext) {
    let mut mmu = Mmu::new(16 * 1024 * 1024);
    mmu.write_phys(CODE, bytes).expect("write CPU fixture");
    let mut cpu = Cpu::new();
    cpu.set_rip(CODE);
    (
        cpu,
        mmu,
        InterruptController::new(),
        PortBus::new(),
        BiosContext::new(),
    )
}

fn step(
    cpu: &mut Cpu,
    mmu: &mut Mmu,
    intc: &mut InterruptController,
    ports: &mut PortBus,
    bios: &mut BiosContext,
) -> Result<(), CpuError> {
    cpu.step(mmu, intc, ports, bios)
}

struct DifferentialMachine {
    cpu: Cpu,
    mmu: Mmu,
    intc: InterruptController,
    ports: PortBus,
    bios: BiosContext,
}

impl DifferentialMachine {
    fn new(program: &[u8]) -> Self {
        let mut mmu = Mmu::new(2 * 1024 * 1024);
        mmu.write_phys(CODE, program).expect("write differential program");
        let mut cpu = Cpu::new();
        cpu.set_rip(CODE);
        Self {
            cpu,
            mmu,
            intc: InterruptController::new(),
            ports: PortBus::new(),
            bios: BiosContext::new(),
        }
    }

    fn interpreter_step(&mut self) -> Result<(), CpuError> {
        self.cpu.step(
            &mut self.mmu,
            &mut self.intc,
            &mut self.ports,
            &mut self.bios,
        )
    }

    fn translated_step(
        &mut self,
        engine: &mut ExecutionEngine,
        budget: usize,
    ) -> Result<usize, CpuError> {
        engine.execute(
            &mut self.cpu,
            &mut self.mmu,
            &mut self.intc,
            &mut self.ports,
            &mut self.bios,
            budget,
        )
    }

    fn memory(&self) -> Vec<u8> {
        self.mmu
            .read_phys(0, self.mmu.ram_size())
            .expect("read differential memory")
    }
}

fn translated_engine(hot_threshold: u64) -> ExecutionEngine {
    ExecutionEngine::with_config(ExecutionEngineConfig {
        max_block_instructions: 16,
        hot_threshold,
        cache_capacity: 32,
        enable_jit: true,
        enable_profiling: true,
    })
}

fn run_interpreter(
    machine: &mut DifferentialMachine,
    instruction_budget: usize,
) -> Result<usize, CpuError> {
    let mut executed = 0;
    while executed < instruction_budget && !machine.cpu.state.halted {
        machine.interpreter_step()?;
        executed += 1;
    }
    Ok(executed)
}

fn run_translated(
    machine: &mut DifferentialMachine,
    engine: &mut ExecutionEngine,
    instruction_budget: usize,
) -> Result<usize, CpuError> {
    let mut executed = 0;
    while executed < instruction_budget && !machine.cpu.state.halted {
        let count = machine.translated_step(engine, instruction_budget - executed)?;
        if count == 0 {
            break
        }
        executed += count;
    }
    Ok(executed)
}

fn assert_architectural_equivalence(
    interpreter: &DifferentialMachine,
    translated: &DifferentialMachine,
) {
    assert_eq!(translated.cpu.state, interpreter.cpu.state, "CPU state diverged");
    assert_eq!(translated.memory(), interpreter.memory(), "guest memory diverged");
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PortEffect {
    Read { port: u16, size: u8 },
    Write { port: u16, value: u64, size: u8 },
}

struct TracePort {
    effects: Rc<RefCell<Vec<PortEffect>>>,
    read_value: u64,
}

impl PortDevice for TracePort {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        self.effects.borrow_mut().push(PortEffect::Read { port, size });
        Ok(self.read_value)
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        self.effects
            .borrow_mut()
            .push(PortEffect::Write { port, value, size });
        Ok(())
    }

    fn reset(&mut self) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MmioEffect {
    Read { address: u64, size: u8 },
    Write { address: u64, value: u64, size: u8 },
}

struct TraceMmio {
    effects: Rc<RefCell<Vec<MmioEffect>>>,
    read_value: u64,
}

impl Device for TraceMmio {
    fn read(&self, address: u64, size: u8) -> Result<u64, DeviceError> {
        self.effects
            .borrow_mut()
            .push(MmioEffect::Read { address, size });
        Ok(self.read_value)
    }

    fn write(&mut self, address: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        self.effects
            .borrow_mut()
            .push(MmioEffect::Write { address, value, size });
        Ok(())
    }

    fn reset(&mut self) {}
}

#[test]
fn interpreter_and_translated_paths_match_cpu_memory_and_compiled_loops() {
    let program = [
        0x48, 0xB8, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
        0x50,
        0x5B,
        0x48, 0x83, 0xC3, 0x01,
        0x53,
        0x59,
        0xF4,
    ];
    let mut interpreter = DifferentialMachine::new(&program);
    let mut translated = DifferentialMachine::new(&program);
    interpreter.cpu.state.rsp = 0x8000;
    translated.cpu.state.rsp = 0x8000;
    let mut engine = translated_engine(2);

    assert_eq!(run_interpreter(&mut interpreter, 7), Ok(7));
    assert_eq!(run_translated(&mut translated, &mut engine, 7), Ok(7));
    assert_architectural_equivalence(&interpreter, &translated);

    let loop_program = [0xE2, 0xFE, 0xF4];
    let mut interpreter = DifferentialMachine::new(&loop_program);
    let mut translated = DifferentialMachine::new(&loop_program);
    interpreter.cpu.state.rcx = 4;
    translated.cpu.state.rcx = 4;
    let mut engine = translated_engine(2);

    assert_eq!(run_interpreter(&mut interpreter, 5), Ok(5));
    assert_eq!(run_translated(&mut translated, &mut engine, 5), Ok(5));
    assert_architectural_equivalence(&interpreter, &translated);
    assert!(engine.stats().compiled_blocks >= 1);
}

#[test]
fn interpreter_and_translated_paths_match_exception_timing_and_state() {
    let mut interpreter = DifferentialMachine::new(&[0x90, 0x0F, 0x38]);
    let mut translated = DifferentialMachine::new(&[0x90, 0x0F, 0x38]);
    let mut engine = translated_engine(2);

    assert_eq!(interpreter.interpreter_step(), Ok(()));
    assert_eq!(translated.translated_step(&mut engine, 1), Ok(1));
    assert_architectural_equivalence(&interpreter, &translated);
    assert_eq!(
        interpreter.interpreter_step(),
        Err(CpuError::InstructionDecodeError)
    );
    assert_eq!(
        translated.translated_step(&mut engine, 1),
        Err(CpuError::InstructionDecodeError)
    );
    assert_architectural_equivalence(&interpreter, &translated);

    for (program, expected) in [
        (&[0xF7, 0xF0][..], CpuError::DivideError),
        (&[0x0F, 0x34][..], CpuError::UnsupportedInstruction),
    ] {
        let mut interpreter = DifferentialMachine::new(program);
        let mut translated = DifferentialMachine::new(program);
        interpreter.cpu.state.rdx = 1;
        translated.cpu.state.rdx = 1;
        interpreter.cpu.state.interrupt_shadow = true;
        translated.cpu.state.interrupt_shadow = true;
        let mut engine = translated_engine(2);

        assert_eq!(interpreter.interpreter_step(), Err(expected));
        assert_eq!(translated.translated_step(&mut engine, 1), Err(expected));
        assert_architectural_equivalence(&interpreter, &translated);
    }
}

#[test]
fn interpreter_and_translated_paths_match_ordered_device_effects() {
    const PORT: u16 = 0x400;
    let mut interpreter = DifferentialMachine::new(&[0xEC, 0xEE, 0xF4]);
    let mut translated = DifferentialMachine::new(&[0xEC, 0xEE, 0xF4]);
    interpreter.cpu.state.rdx = PORT as u64;
    translated.cpu.state.rdx = PORT as u64;

    let interpreter_effects = Rc::new(RefCell::new(Vec::new()));
    let translated_effects = Rc::new(RefCell::new(Vec::new()));
    interpreter.ports.attach(
        PORT,
        1,
        Box::new(TracePort {
            effects: interpreter_effects.clone(),
            read_value: 0x5A,
        }),
    );
    translated.ports.attach(
        PORT,
        1,
        Box::new(TracePort {
            effects: translated_effects.clone(),
            read_value: 0x5A,
        }),
    );
    let mut engine = translated_engine(2);

    assert_eq!(run_interpreter(&mut interpreter, 3), Ok(3));
    assert_eq!(run_translated(&mut translated, &mut engine, 3), Ok(3));
    assert_architectural_equivalence(&interpreter, &translated);
    assert_eq!(*translated_effects.borrow(), *interpreter_effects.borrow());
    assert_eq!(
        *translated_effects.borrow(),
        vec![
            PortEffect::Read { port: PORT, size: 1 },
            PortEffect::Write {
                port: PORT,
                value: 0x5A,
                size: 1,
            },
        ]
    );

    const MMIO: u64 = 0x1_0000;
    const MMIO_VALUE: u64 = 0x1122_3344_5566_7788;
    let program = [
        0x48, 0x8B, 0x04, 0x25, 0x00, 0x00, 0x01, 0x00,
        0x48, 0x89, 0x04, 0x25, 0x08, 0x00, 0x01, 0x00,
        0xF4,
    ];
    let mut interpreter = DifferentialMachine::new(&program);
    let mut translated = DifferentialMachine::new(&program);
    let interpreter_effects = Rc::new(RefCell::new(Vec::new()));
    let translated_effects = Rc::new(RefCell::new(Vec::new()));
    interpreter.mmu.attach_mmio(
        MMIO,
        16,
        Box::new(TraceMmio {
            effects: interpreter_effects.clone(),
            read_value: MMIO_VALUE,
        }),
    );
    translated.mmu.attach_mmio(
        MMIO,
        16,
        Box::new(TraceMmio {
            effects: translated_effects.clone(),
            read_value: MMIO_VALUE,
        }),
    );
    let mut engine = translated_engine(2);

    assert_eq!(run_interpreter(&mut interpreter, 3), Ok(3));
    assert_eq!(run_translated(&mut translated, &mut engine, 3), Ok(3));
    assert_architectural_equivalence(&interpreter, &translated);
    assert_eq!(*translated_effects.borrow(), *interpreter_effects.borrow());
    assert_eq!(
        *translated_effects.borrow(),
        vec![
            MmioEffect::Read {
                address: MMIO,
                size: 8,
            },
            MmioEffect::Write {
                address: MMIO + 8,
                value: MMIO_VALUE,
                size: 8,
            },
        ]
    );
}

fn even_parity(value: u8) -> bool {
    value.count_ones() % 2 == 0
}

fn reference_arithmetic_flags(lhs: u64, rhs: u64, result: u64, subtract: bool) -> u64 {
    let sign = 1u64 << 63;
    let mut flags = 0x2;
    if if subtract { lhs < rhs } else { lhs > u64::MAX - rhs } {
        flags |= CF;
    }
    if ((lhs ^ rhs ^ result) & 0x10) != 0 {
        flags |= AF;
    }
    if result == 0 {
        flags |= ZF;
    }
    if result & sign != 0 {
        flags |= SF;
    }
    if even_parity(result as u8) {
        flags |= PF;
    }
    let overflow = if subtract {
        ((lhs ^ rhs) & (lhs ^ result) & sign) != 0
    } else {
        (!(lhs ^ rhs) & (lhs ^ result) & sign) != 0
    };
    if overflow {
        flags |= OF;
    }
    flags
}

fn reference_logic_flags(result: u64) -> u64 {
    let mut flags = 0x2;
    if result == 0 {
        flags |= ZF;
    }
    if result & (1u64 << 63) != 0 {
        flags |= SF;
    }
    if even_parity(result as u8) {
        flags |= PF;
    }
    flags
}

#[test]
fn differential_integer_instructions_and_flags() {
    let cases = [
        ("add", [0x48, 0x01, 0xD8], false),
        ("sub", [0x48, 0x29, 0xD8], true),
    ];

    for (name, bytes, subtract) in cases {
        let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = fixture(&bytes);
        let lhs = 0x8000_0000_0000_0000;
        let rhs = 1;
        cpu.state.rax = lhs;
        cpu.state.rbx = rhs;
        cpu.state.rflags = 0x202;
        step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios)
            .unwrap_or_else(|error| panic!("{name} failed: {error:?}"));

        let expected = if subtract {
            lhs.wrapping_sub(rhs)
        } else {
            lhs.wrapping_add(rhs)
        };
        let expected_flags = reference_arithmetic_flags(lhs, rhs, expected, subtract);
        assert_eq!(cpu.state.rax, expected, "{name} result");
        assert_eq!(
            cpu.state.rflags & ARITHMETIC_FLAGS,
            expected_flags & ARITHMETIC_FLAGS,
            "{name} flags"
        );
    }

    let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = fixture(&[0x48, 0x31, 0xD8]);
    cpu.state.rax = 0xFFFF_0000_FFFF_0000;
    cpu.state.rbx = 0xFFFF_0000_FFFF_0000;
    cpu.state.rflags = 0x202 | CF | OF;
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("xor");
    assert_eq!(cpu.state.rax, 0);
    assert_eq!(
        cpu.state.rflags & ARITHMETIC_FLAGS,
        reference_logic_flags(0) & ARITHMETIC_FLAGS
    );

    let (mut cpu, mut mmu, mut intc, mut ports, mut bios) =
        fixture(&[0x48, 0xC1, 0xE0, 0x01]);
    cpu.state.rax = 0x8000_0000_0000_0001;
    cpu.state.rflags = 0x202;
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("shl");
    let expected = 0x2u64;
    assert_eq!(cpu.state.rax, expected);
    assert_ne!(cpu.state.rflags & CF, 0);
    assert_ne!(cpu.state.rflags & OF, 0);
    assert_eq!(
        cpu.state.rflags & (PF | ZF | SF),
        reference_logic_flags(expected) & (PF | ZF | SF)
    );

    let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = fixture(&[
        0x48, 0xB8, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
    ]);
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("mov immediate");
    assert_eq!(cpu.state.rax, 0x1122_3344_5566_7788);
}

#[test]
fn differential_string_operation_matches_reference_forward_and_backward() {
    for backwards in [false, true] {
        let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = fixture(&[0xF3, 0xA4]);
        let source = b"diff";
        let source_start = 0x3000;
        let destination_start = 0x4000;
        mmu.write_bytes(source_start, source).expect("write string source");
        cpu.state.rsi = if backwards {
            source_start + source.len() as u64 - 1
        } else {
            source_start
        };
        cpu.state.rdi = if backwards {
            destination_start + source.len() as u64 - 1
        } else {
            destination_start
        };
        cpu.state.rcx = source.len() as u64;
        if backwards {
            cpu.state.rflags |= DF;
        }

        step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("rep movsb");

        let mut reference = vec![0u8; source.len()];
        let mut rsi = if backwards {
            source_start + source.len() as u64 - 1
        } else {
            source_start
        };
        let mut rdi = if backwards {
            destination_start + source.len() as u64 - 1
        } else {
            destination_start
        };
        let delta: i64 = if backwards { -1 } else { 1 };
        for _ in source {
            let source_index = (rsi - source_start) as usize;
            let destination_index = (rdi - destination_start) as usize;
            reference[destination_index] = source[source_index];
            rsi = (rsi as i64 + delta) as u64;
            rdi = (rdi as i64 + delta) as u64;
        }
        assert_eq!(
            mmu.read_bytes(destination_start, source.len()).expect("read destination"),
            reference
        );
        assert_eq!(cpu.state.rcx, 0);
        assert_eq!(cpu.state.rsi, rsi);
        assert_eq!(cpu.state.rdi, rdi);
    }
}

#[test]
fn differential_segmentation_and_paging_faults_match_reference() {
    let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = fixture(&[
        0x64, 0x48, 0x8B, 0x04, 0x25, 0x10, 0, 0, 0,
    ]);
    let fs_base = 0x2000;
    let offset = 0x10;
    let expected_address = fs_base + offset;
    let expected_value = 0xCAFEBABE_D15EA5E5u64;
    cpu.state.fs_base = fs_base;
    mmu.write_phys(expected_address, &expected_value.to_le_bytes())
        .expect("write segmented value");
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("segmented load");
    assert_eq!(cpu.state.rax, expected_value);

    let mut paged = Mmu::new(16 * 1024 * 1024);
    let code_frame = paged.alloc_frame().expect("allocate code frame");
    paged.write_phys(code_frame, &[0x90]).expect("write NX code");
    paged
        .map_page(
            CODE,
            code_frame,
            PageFlags::PRESENT | PageFlags::WRITABLE | PageFlags::NX,
        )
        .expect("map NX page");
    paged.set_paging(true, paged.cr3());
    let mut faulting_cpu = Cpu::new();
    faulting_cpu.set_rip(CODE);
    let mut intc = InterruptController::new();
    let mut ports = PortBus::new();
    let mut bios = BiosContext::new();
    assert!(matches!(
        faulting_cpu.step(&mut paged, &mut intc, &mut ports, &mut bios),
        Err(CpuError::InstructionDecodeError)
    ));
}

#[test]
fn differential_privilege_interrupt_and_reset_state() {
    let mut cpu = Cpu::new();
    let reset = cpu.state;
    assert_eq!(reset.rip, 0xFFF0);
    assert_eq!(reset.rflags, 0x2);
    assert_eq!(reset.mode, CpuMode::Real16);
    assert_eq!(reset.privilege, PrivilegeLevel::Ring0);
    assert!(!reset.halted);

    let mut mmu = Mmu::new(16 * 1024 * 1024);
    cpu.enter_protected_mode(&mut mmu, 0).expect("protected mode");
    assert_eq!(cpu.mode(), CpuMode::Protected32);
    assert_eq!(cpu.state.cs.selector, 0x08);
    assert_eq!(cpu.state.ds.selector, 0x10);

    cpu.state.efer |= 1 << 8;
    cpu.enter_long_mode(&mut mmu, 0x4000).expect("long mode");
    assert_eq!(cpu.mode(), CpuMode::Long64);
    assert_ne!(cpu.state.cs.attributes & 0x2000, 0);

    let mut mmu = Mmu::new(16 * 1024 * 1024);
    let mut cpu = Cpu::new();
    let mut intc = InterruptController::new();
    install_gate(&mut mmu, &mut intc, 0x20, 0x7000);
    cpu.state.rip = 0x1234;
    cpu.state.rsp = 0x9000;
    cpu.state.rflags = 0x202;
    cpu.state.cs.selector = 0x1B;
    cpu.state.ss.selector = 0x23;
    cpu.state.privilege = PrivilegeLevel::Ring3;
    cpu.state.halted = true;
    cpu.handle_interrupt(0x20, &mut mmu, &mut intc).expect("interrupt");

    let expected_rsp = 0x9000 - 5 * 8;
    assert_eq!(cpu.state.rip, 0x7000);
    assert_eq!(cpu.state.rsp, expected_rsp);
    assert_eq!(cpu.state.privilege, PrivilegeLevel::Ring0);
    assert_eq!(cpu.state.rflags & IF, 0);
    assert!(!cpu.state.halted);
    assert_eq!(mmu.read_u64(expected_rsp).expect("saved RIP"), 0x1234);
    assert_eq!(mmu.read_u64(expected_rsp + 8).expect("saved CS"), 0x1B);
    assert_eq!(mmu.read_u64(expected_rsp + 16).expect("saved flags"), 0x202);
    assert_eq!(mmu.read_u64(expected_rsp + 24).expect("saved RSP"), 0x9000);
    assert_eq!(mmu.read_u64(expected_rsp + 32).expect("saved SS"), 0x23);

    cpu.reset();
    assert_eq!(cpu.state, synos_vm::cpu::CpuState::default());
}

fn install_gate(mmu: &mut Mmu, intc: &mut InterruptController, vector: u8, offset: u64) {
    let base = 0x2000;
    intc.set_idt(base, 0x0FFF);
    let entry = base + vector as u64 * 16;
    let mut gate = [0u8; 16];
    gate[0..2].copy_from_slice(&(offset as u16).to_le_bytes());
    gate[2..4].copy_from_slice(&0x08u16.to_le_bytes());
    gate[5] = 0x8E;
    gate[6..8].copy_from_slice(&((offset >> 16) as u16).to_le_bytes());
    gate[8..12].copy_from_slice(&((offset >> 32) as u32).to_le_bytes());
    mmu.write_phys(entry, &gate).expect("write IDT gate");
}
