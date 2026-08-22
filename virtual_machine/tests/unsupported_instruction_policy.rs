//! The VM's unsupported-instruction policy is intentionally explicit.
//!
//! A valid but unimplemented instruction is a propagated VM error. It does
//! not become a guest #UD, and it does not silently halt or advance the CPU.
//! Malformed bytes remain a separate decode error.

use ghostos_vm::devices::{InterruptController, PortBus};
use ghostos_vm::firmware::bios::BiosContext;
use ghostos_vm::{Cpu, CpuError, CpuMode, Mmu};

const CODE: u64 = 0x1000;

fn assert_unsupported_is_vm_error(mode: CpuMode) {
    let mut mmu = Mmu::new(4 * 1024 * 1024);
    mmu.write_phys(CODE, &[0x0F, 0x34]).expect("write SYSENTER");
    let mut cpu = Cpu::new();
    cpu.state.mode = mode;
    cpu.state.rip = CODE;
    cpu.state.rax = 0x1234;
    cpu.state.rflags = 0x202;
    cpu.state.interrupt_shadow = true;
    let before = cpu.state;
    let mut intc = InterruptController::new();
    let mut ports = PortBus::new();
    let mut bios = BiosContext::new();

    let result = cpu.step(&mut mmu, &mut intc, &mut ports, &mut bios);

    assert_eq!(result, Err(CpuError::UnsupportedInstruction));
    assert_eq!(cpu.state, before, "unsupported instruction changed CPU state");
    assert!(!cpu.state.halted);
}

#[test]
fn unsupported_instruction_is_vm_error_in_bios_real_mode() {
    assert_unsupported_is_vm_error(CpuMode::Real16);
}

#[test]
fn unsupported_instruction_is_vm_error_in_long_mode() {
    assert_unsupported_is_vm_error(CpuMode::Long64);
}

#[test]
fn malformed_instruction_remains_decode_error() {
    let mut mmu = Mmu::new(4 * 1024 * 1024);
    mmu.write_phys(CODE, &[0x0F, 0x38]).expect("write malformed opcode");
    let mut cpu = Cpu::new();
    cpu.set_rip(CODE);
    let mut intc = InterruptController::new();
    let mut ports = PortBus::new();
    let mut bios = BiosContext::new();

    assert_eq!(
        cpu.step(&mut mmu, &mut intc, &mut ports, &mut bios),
        Err(CpuError::InstructionDecodeError)
    );
    assert_eq!(cpu.rip(), CODE);
    assert!(!cpu.state.halted);
}
