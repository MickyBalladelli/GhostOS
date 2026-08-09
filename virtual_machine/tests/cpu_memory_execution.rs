//! Direct deterministic coverage for the CPU, MMU, and translated execution engine.

use std::cell::RefCell;
use std::rc::Rc;

use synos_vm::cpu::decoder::{InstructionDecodeError, InstructionDecoder, Operand};
use synos_vm::devices::{ApicTrigger, Device, DeviceError, InterruptController, LocalApic, PortBus};
use synos_vm::firmware::bios::BiosContext;
use synos_vm::{
    Cpu, CpuError, CpuMode, ExecutionEngine, ExecutionEngineConfig, LargePageSize, Mmu,
    PageFlags, PrivilegeLevel, PAGE_SIZE,
};

const CODE: u64 = 0x1000;

fn decode(bytes: &[u8]) -> Result<synos_vm::cpu::decoder::DecodedInstruction, InstructionDecodeError> {
    let mut mmu = Mmu::new(2 * 1024 * 1024);
    mmu.write_phys(CODE, bytes).expect("write decoder fixture");
    InstructionDecoder::new().decode(CODE, &mmu)
}

fn cpu_with(bytes: &[u8]) -> (Cpu, Mmu, InterruptController, PortBus, BiosContext) {
    let mut cpu = Cpu::new();
    let mut mmu = Mmu::new(16 * 1024 * 1024);
    mmu.write_phys(CODE, bytes).expect("write execution fixture");
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

#[test]
fn decoder_covers_prefix_addressing_far_operands_and_malformed_streams() {
    let rex = decode(&[0x49, 0x89, 0xC0]).expect("decode REX register operands");
    assert!(rex.has_rex);
    assert!(matches!(rex.operands.as_slice(), [Operand::Register(8), Operand::Register(0)]));

    let sib = decode(&[0x64, 0x48, 0x8B, 0x04, 0x25, 0x78, 0x56, 0x34, 0x12])
        .expect("decode segment override and SIB");
    assert_eq!(sib.opsize, 64);
    match &sib.operands[..] {
        [Operand::Register(0), Operand::Memory(memory)] => {
            assert_eq!(memory.segment, 4);
            assert_eq!(memory.base, None);
            assert_eq!(memory.index, None);
            assert_eq!(memory.displacement, 0x1234_5678);
        }
        _ => panic!("unexpected SIB operands"),
    }

    let addr32 = decode(&[0x67, 0x8B, 0x44, 0x88, 0x7F]).expect("decode address-size override");
    assert_eq!(addr32.addrsize, 32);
    match &addr32.operands[..] {
        [Operand::Register(0), Operand::Memory(memory)] => {
            assert_eq!(memory.base, Some(0));
            assert_eq!(memory.index, Some(1));
            assert_eq!(memory.scale, 4);
            assert_eq!(memory.displacement, 0x7F);
        }
        _ => panic!("unexpected address-size operands"),
    }

    let rip = decode(&[0x48, 0x8B, 0x05, 0x10, 0, 0, 0]).expect("decode RIP-relative operand");
    assert!(matches!(&rip.operands[..], [Operand::Register(0), Operand::Memory(memory)] if memory.rip_relative));

    let far = decode(&[0xEA, 0x34, 0x12, 0, 0, 0x78, 0x56]).expect("decode far operand");
    assert!(matches!(far.operands.as_slice(), [Operand::Far { offset: 0x1234, selector: 0x5678 }]));

    assert!(matches!(decode(&[0x66; 16]), Err(InstructionDecodeError::InvalidPrefix)));
    let mut short = Mmu::new(2 * PAGE_SIZE);
    short.write_byte((2 * PAGE_SIZE - 1) as u64, 0xB8).expect("write partial instruction");
    assert!(matches!(
        InstructionDecoder::new().decode((2 * PAGE_SIZE - 1) as u64, &short),
        Err(InstructionDecodeError::OutOfMemory)
    ));
    assert!(matches!(decode(&[0x0F, 0x38]), Err(InstructionDecodeError::InvalidOpcode)));
}

#[test]
fn executor_covers_alu_flags_branches_stack_calls_strings_control_msr_and_errors() {
    let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = cpu_with(&[
        0x48, 0x83, 0xC0, 0x01, // add rax, 1
        0x48, 0x83, 0xE8, 0x02, // sub rax, 2
        0x75, 0x04, // jne over the next add
        0x48, 0x83, 0xC0, 0x10,
        0x48, 0x83, 0xC0, 0x20,
    ]);
    cpu.state.rax = 2;
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("add");
    assert_eq!(cpu.state.rax, 3);
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("sub");
    assert_eq!(cpu.state.rax, 1);
    assert_eq!(cpu.state.rflags & (1 << 6), 0);
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("branch");
    assert_eq!(cpu.rip(), CODE + 14);

    let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = cpu_with(&[0x50, 0x5B]);
    cpu.state.rax = 0xDEAD_BEEF;
    cpu.state.rsp = 0x8000;
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("push");
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("pop");
    assert_eq!(cpu.state.rbx, 0xDEAD_BEEF);
    assert_eq!(cpu.state.rsp, 0x8000);

    let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = cpu_with(&[0xE8, 0x05, 0, 0, 0]);
    mmu.write_phys(CODE + 10, &[0xC3]).expect("write return");
    cpu.state.rsp = 0x8000;
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("call");
    assert_eq!(cpu.rip(), CODE + 10);
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("return");
    assert_eq!(cpu.rip(), CODE + 5);
    assert_eq!(cpu.state.rsp, 0x8000);

    let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = cpu_with(&[0xF3, 0xA4]);
    cpu.state.rsi = 0x3000;
    cpu.state.rdi = 0x4000;
    cpu.state.rcx = 4;
    mmu.write_bytes(0x3000, b"copy").expect("write string source");
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("rep movsb");
    assert_eq!(mmu.read_bytes(0x4000, 4).expect("read string destination"), b"copy");
    assert_eq!(cpu.state.rcx, 0);

    let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = cpu_with(&[0x0F, 0x22, 0xD8]);
    let new_cr3 = 0x4000;
    cpu.state.rax = new_cr3;
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("mov cr3");
    assert_eq!(cpu.state.cr3, new_cr3);

    let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = cpu_with(&[0x0F, 0x21, 0xC0]);
    cpu.state.rax = u64::MAX;
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("mov from debug register");
    assert_eq!(cpu.state.rax, 0);

    let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = cpu_with(&[0x0F, 0x30, 0x0F, 0x32]);
    cpu.state.rcx = 0xC000_0100;
    cpu.state.rax = 0x5566_7788;
    cpu.state.rdx = 0x1122_3344;
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("wrmsr FS base");
    assert_eq!(cpu.state.fs_base, 0x1122_3344_5566_7788);
    cpu.state.rax = 0;
    cpu.state.rdx = 0;
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("rdmsr FS base");
    assert_eq!(cpu.state.rax, 0x5566_7788);
    assert_eq!(cpu.state.rdx, 0x1122_3344);

    let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = cpu_with(&[0xF7, 0xF0]);
    cpu.state.rax = 0;
    cpu.state.rdx = 1;
    assert!(matches!(
        step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios),
        Err(CpuError::DivideError)
    ));
    let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = cpu_with(&[0x0F, 0x0B]);
    assert!(matches!(
        step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios),
        Err(CpuError::InstructionDecodeError)
    ));
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

#[test]
fn cpu_modes_privilege_segments_interrupts_sti_shadow_and_hlt_wakeup() {
    let reset = Cpu::new();
    assert_eq!(reset.state.rax, 0);
    assert_eq!(reset.state.rsp, 0);
    assert_eq!(reset.rip(), 0xFFF0);
    assert_eq!(reset.state.rflags, 0x2);
    assert!(!reset.state.halted);

    let mut mmu = Mmu::new(16 * 1024 * 1024);
    let mut cpu = Cpu::new();
    cpu.enter_protected_mode(&mut mmu, 0x3000).expect("protected mode");
    assert_eq!(cpu.mode(), CpuMode::Protected32);
    assert_eq!(cpu.state.cs.limit, 0xFFFF_FFFF);
    assert_eq!(cpu.state.ds.limit, 0xFFFF_FFFF);
    cpu.state.efer |= 1 << 8;
    cpu.enter_long_mode(&mut mmu, 0x4000).expect("long mode");
    assert_eq!(cpu.mode(), CpuMode::Long64);
    assert!(cpu.state.cs.attributes & 0x2000 != 0);
    cpu.state.cr0 &= !(1 << 31);
    cpu.update_paging_state(&mut mmu).expect("leave long mode");
    assert_eq!(cpu.mode(), CpuMode::Protected32);

    let (mut cpu, mut mmu, mut intc, mut ports, mut bios) = cpu_with(&[0xFB, 0x90, 0xF4]);
    cpu.state.rflags &= !(1 << 9);
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("sti");
    assert_ne!(cpu.state.rflags & (1 << 9), 0);
    assert!(cpu.state.interrupt_shadow);
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("shadowed instruction");
    assert!(!cpu.state.interrupt_shadow);
    step(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios).expect("hlt");
    assert!(cpu.state.halted);

    install_gate(&mut mmu, &mut intc, 0x20, 0x7000);
    cpu.handle_interrupt(0x20, &mut mmu, &mut intc).expect("wake and deliver IRQ");
    assert!(!cpu.state.halted);
    assert_eq!(cpu.rip(), 0x7000);
    assert_eq!(cpu.privilege(), PrivilegeLevel::Ring0);

    let mut apic = LocalApic::new(0);
    apic.signal(0x30, ApicTrigger::Edge);
    apic.signal(0x40, ApicTrigger::Edge);
    assert_eq!(apic.pending_vector(), Some(0x40));
    apic.accept_pending(0x40);
    apic.eoi();
    assert_eq!(apic.pending_vector(), Some(0x30));

    install_gate(&mut mmu, &mut intc, 14, 0x7100);
    cpu.state.rsp = 0x9000;
    cpu.raise_page_fault(0x1234_5678, false, true, false, &mut mmu, &mut intc)
        .expect("deliver page fault");
    assert_eq!(cpu.state.cr2, 0x1234_5000);
    assert_eq!(cpu.rip(), 0x7100);
    assert_eq!(mmu.read_u64(cpu.state.rsp).expect("read page fault code"), 2);

    cpu.state.privilege = PrivilegeLevel::Ring3;
    cpu.state.cs.selector = 0x1B;
    cpu.state.ss.selector = 0x23;
    cpu.state.rsp = 0xA000;
    install_gate(&mut mmu, &mut intc, 0x21, 0x7200);
    cpu.handle_interrupt(0x21, &mut mmu, &mut intc).expect("ring transition");
    assert_eq!(cpu.privilege(), PrivilegeLevel::Ring0);
    assert_eq!(cpu.rip(), 0x7200);
}

#[derive(Default)]
struct RegisterDevice {
    value: u64,
}

impl Device for RegisterDevice {
    fn read(&self, _address: u64, _size: u8) -> Result<u64, DeviceError> {
        Ok(self.value)
    }

    fn write(&mut self, _address: u64, value: u64, _size: u8) -> Result<(), DeviceError> {
        self.value = value;
        Ok(())
    }

    fn reset(&mut self) {
        self.value = 0;
    }
}

#[test]
fn mmu_covers_frames_paging_permissions_large_pages_mmio_cow_bounds_and_stats() {
    let mut mmu = Mmu::new(16 * 1024 * 1024);
    let initial = mmu.memory_stats();
    let reusable = mmu.alloc_frame().expect("allocate reusable frame");
    mmu.free_frame(reusable);
    assert_eq!(mmu.alloc_frame().expect("reuse frame"), reusable);
    let frame = mmu.alloc_zeroed_frame().expect("allocate frame");
    mmu.write_phys(frame, &[0xAA]).expect("seed frame");
    mmu.map_page(0x4000, frame, PageFlags::PRESENT | PageFlags::WRITABLE)
        .expect("map page");
    mmu.set_paging(true, mmu.cr3());
    mmu.write_byte(0x4003, 0x5A).expect("write mapped page");
    assert_eq!(mmu.read_byte(0x4003).expect("read mapped page"), 0x5A);
    assert!(mmu.write_byte(0x5000, 1).is_err());
    mmu.set_privilege(true);
    assert!(mmu.read_byte(0x4000).is_err());
    mmu.set_privilege(false);

    mmu.map_2mb_page(0x20_0000, 0x20_0000, PageFlags::PRESENT | PageFlags::WRITABLE)
        .expect("map 2 MiB page");
    assert_eq!(
        mmu.read_byte(0x20_0123).expect("read large page"),
        0
    );
    assert!(matches!(
        mmu.map_large_page(0x21_0000, 0x20_0000, LargePageSize::TwoMiB, PageFlags::PRESENT),
        Err(synos_vm::MemoryError::AlignmentError)
    ));

    mmu.attach_mmio(0xF000, 8, Box::new(RegisterDevice::default()));
    mmu.set_paging(false, 0);
    mmu.write_to_addr(0xF001, 0xCAFE, 2).expect("write unaligned MMIO");
    assert_eq!(mmu.read_from_addr(0xF001, 2).expect("read MMIO"), 0xCAFE);
    assert!(mmu.read_byte(u64::MAX).is_err());

    let cow_frame = mmu.alloc_frame().expect("allocate COW frame");
    mmu.map_page(0x8000, cow_frame, PageFlags::PRESENT | PageFlags::WRITABLE)
        .expect("map COW source");
    mmu.set_paging(true, mmu.cr3());
    mmu.write_byte(0x8000, 0x11).expect("seed COW source");
    mmu.clone_cow_page(0x8000, 0x9000).expect("clone COW page");
    assert!(mmu.is_cow_page(0x8000));
    mmu.write_byte(0x9000, 0x22).expect("resolve COW write");
    assert_eq!(mmu.read_byte(0x8000).expect("read source after COW"), 0x11);
    assert_eq!(mmu.read_byte(0x9000).expect("read destination after COW"), 0x22);

    mmu.set_overcommit_limit(1);
    mmu.map_overcommit_page(0xA000, PageFlags::PRESENT).expect("map lazy page");
    assert_eq!(mmu.overcommitted_pages(), 1);
    assert!(mmu.map_overcommit_page(0xB000, PageFlags::PRESENT).is_err());
    mmu.write_byte(0xA000, 0x33).expect("resolve lazy page");
    assert_eq!(mmu.overcommitted_pages(), 0);

    let balloon = mmu.alloc_frame().expect("allocate balloon frame");
    mmu.balloon_inflate(&[balloon]).expect("inflate balloon");
    assert_eq!(mmu.memory_stats().ballooned_frames, 1);
    mmu.balloon_deflate(&[balloon]).expect("deflate balloon");
    assert_eq!(mmu.memory_stats().ballooned_frames, 0);
    assert!(mmu.memory_stats().total_frames >= initial.total_frames);
    mmu.reset();
    assert_eq!(mmu.memory_stats().free_frames, mmu.memory_stats().total_frames);
}

#[test]
fn execution_engine_profiles_blocks_invalidates_code_honors_limits_and_resets_cleanly() {
    let mut mmu = Mmu::new(4 * 1024 * 1024);
    mmu.write_phys(CODE, &[0xE2, 0xFE]).expect("write loop");
    let mut cpu = Cpu::new();
    cpu.set_rip(CODE);
    cpu.state.rcx = 4;
    let mut intc = InterruptController::new();
    let mut ports = PortBus::new();
    let mut bios = BiosContext::new();
    let hook_calls = Rc::new(RefCell::new(0u64));
    let hook_counter = hook_calls.clone();
    let mut engine = ExecutionEngine::with_config(ExecutionEngineConfig {
        max_block_instructions: 1,
        hot_threshold: 2,
        enable_jit: true,
        enable_profiling: true,
        cache_capacity: 2,
    });
    engine.set_profile_hook(move |_| *hook_counter.borrow_mut() += 1);

    for _ in 0..4 {
        assert_eq!(
            engine
                .execute(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios, 1)
                .expect("execute bounded loop"),
            1
        );
    }
    assert!(engine.stats().cache_misses >= 1);
    assert!(engine.stats().cache_hits >= 1);
    assert!(engine.stats().compiled_blocks >= 1);
    assert!(engine.instruction_count(CODE) >= 1);
    assert!(engine.block_profile(CODE).is_some_and(|profile| profile.compiled));
    assert_eq!(*hook_calls.borrow(), 4);

    let misses_before_write = engine.stats().cache_misses;
    cpu.set_rip(CODE);
    mmu.write_byte(CODE, 0x90).expect("self-modifying code write");
    assert_eq!(
        engine
            .execute(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios, 1)
            .expect("execute invalidated code"),
        1
    );
    assert!(engine.stats().cache_misses > misses_before_write);

    cpu.set_rip(0x3000);
    mmu.write_phys(0x3000, &[0x0F, 0x38]).expect("write invalid code");
    assert!(engine
        .execute(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios, 1)
        .is_err());
    engine.reset();
    assert_eq!(engine.cache_len(), 0);
    assert_eq!(engine.stats().instructions, 0);
    assert_eq!(engine.execute(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios, 0).unwrap(), 0);

    engine.config_mut().enable_jit = false;
    cpu.set_rip(CODE);
    cpu.state.rcx = 1;
    mmu.write_phys(CODE, &[0xE2, 0xFE]).expect("restore loop");
    engine
        .execute(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios, 1)
        .expect("execute without JIT");
    assert_eq!(engine.stats().compiled_blocks, 0);
}

#[test]
fn translation_cache_invalidation_proof_matrix() {
    let mut mmu = Mmu::new(8 * 1024 * 1024);
    let alternate = 0x2000;
    let alias = 0x3000;
    mmu.write_phys(CODE, &[0x90]).expect("initial code");
    mmu.write_phys(alternate, &[0xF4]).expect("alternate code");
    let executable = PageFlags::PRESENT | PageFlags::WRITABLE;
    mmu.map_page(CODE, CODE, executable).expect("map initial code");
    mmu.set_paging(true, mmu.cr3());

    let mut cpu = Cpu::new();
    cpu.set_rip(CODE);
    let mut intc = InterruptController::new();
    let mut ports = PortBus::new();
    let mut bios = BiosContext::new();
    let mut engine = ExecutionEngine::with_config(ExecutionEngineConfig {
        max_block_instructions: 4,
        hot_threshold: 100,
        cache_capacity: 8,
        enable_jit: true,
        enable_profiling: false,
    });

    engine
        .execute(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios, 1)
        .expect("execute initial mapping");
    let misses_before_remap = engine.stats().cache_misses;

    mmu.write_phys(alternate, &[0xF4]).expect("write remapped code");
    mmu.map_page(alternate, alternate, executable).expect("map alternate page");
    mmu.map_page(CODE, alternate, executable).expect("remap code page");
    cpu.state.halted = false;
    cpu.set_rip(CODE);
    engine
        .execute(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios, 1)
        .expect("execute remapped code");
    assert!(cpu.state.halted);
    assert!(engine.stats().cache_misses > misses_before_remap);

    mmu.map_page(CODE, CODE, executable | PageFlags::NX)
        .expect("make code non-executable");
    cpu.state.halted = false;
    cpu.set_rip(CODE);
    let misses_before_permission = engine.stats().cache_misses;
    assert!(engine
        .execute(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios, 1)
        .is_err());
    assert!(engine.stats().cache_misses > misses_before_permission);

    mmu.map_page(CODE, CODE, executable)
        .expect("restore executable permission");
    mmu.map_page(alias, CODE, executable)
        .expect("map code alias");
    cpu.state.halted = false;
    cpu.set_rip(CODE);
    mmu.write_phys(CODE, &[0x90]).expect("restore aliased code");
    engine
        .execute(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios, 1)
        .expect("execute code before alias write");
    let misses_before_alias_write = engine.stats().cache_misses;
    mmu.write_byte(alias, 0xF4).expect("write through code alias");
    cpu.state.halted = false;
    cpu.set_rip(CODE);
    engine
        .execute(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios, 1)
        .expect("execute aliased self-modification");
    assert!(cpu.state.halted);
    assert!(engine.stats().cache_misses > misses_before_alias_write);

    mmu.set_paging(false, 0);
    mmu.write_phys(CODE, &[0xCD, 0x20]).expect("interrupt instruction");
    mmu.write_phys(0x6000, &[0xF4]).expect("interrupt handler");
    install_gate(&mut mmu, &mut intc, 0x20, 0x6000);
    cpu.set_idtr(0x2000, 0x0FFF, &mut intc);
    cpu.state.rsp = 0x9000;
    cpu.state.halted = false;
    cpu.set_rip(CODE);
    assert_eq!(
        engine
            .execute(&mut cpu, &mut mmu, &mut intc, &mut ports, &mut bios, 4)
            .expect("execute interrupt boundary"),
        1
    );
    assert_eq!(cpu.rip(), 0x6000);

    engine.reset();
    assert_eq!(engine.cache_len(), 0);
    assert_eq!(engine.stats().instructions, 0);
}
