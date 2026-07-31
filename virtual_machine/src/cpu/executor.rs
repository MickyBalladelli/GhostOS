use crate::cpu::{DecodedInstruction, CpuState};
use crate::memory::Mmu;
use crate::devices::InterruptController;
use crate::firmware::bios::BiosContext;
use crate::cpu::CpuError;

pub struct InstructionExecutor;

impl InstructionExecutor {
    pub fn new() -> Self {
        Self
    }

    pub fn execute(
        &self,
        instruction: &DecodedInstruction,
        state: &mut CpuState,
        mmu: &mut Mmu,
        intc: &mut InterruptController,
        bios: &mut BiosContext,
    ) -> Result<(), CpuError> {
        match instruction.mnemonic {
            "NOP" => {
                state.rip = instruction.next_ip;
            }
            "MOV" => {
                self.execute_mov(instruction, state, mmu)?;
            }
            "ADD" => {
                self.execute_add(instruction, state, mmu)?;
            }
            "PUSH" => {
                self.execute_push(instruction, state, mmu)?;
            }
            "POP" => {
                self.execute_pop(instruction, state, mmu)?;
            }
            "JMP" | "JCC" => {
                self.execute_jump(instruction, state, mmu)?;
            }
            "CALL" => {
                self.execute_call(instruction, state, mmu)?;
            }
            "RET" => {
                self.execute_ret(instruction, state, mmu)?;
            }
            "INT" => {
                self.execute_int(instruction, state, intc)?;
            }
            "IRET" => {
                self.execute_iret(instruction, state, intc)?;
            }
            "HLT" => {
                state.halted = true;
                state.rip = instruction.next_ip;
            }
            "LGDT" => {
                self.execute_lgdt(instruction, state)?;
            }
            "LIDT" => {
                self.execute_lidt(instruction, state)?;
            }
            "MOVZX" | "MOVSX" => {
                self.execute_movsx(instruction, state, mmu)?;
            }
            "LEA" => {
                self.execute_lea(instruction, state)?;
            }
            "CMP" => {
                self.execute_cmp(instruction, state, mmu)?;
            }
            "TEST" => {
                self.execute_test(instruction, state, mmu)?;
            }
            "AND" | "OR" | "XOR" | "SUB" => {
                self.execute_logical(instruction, state, mmu)?;
            }
            "SHL" | "SHR" | "SAR" | "ROL" | "ROR" => {
                self.execute_shift(instruction, state, mmu)?;
            }
            "MOVSD" | "MOVSQ" | "CMPSB" | "CMPSD" => {
                self.execute_string(instruction, state, mmu)?;
            }
            "IN" | "OUT" => {
                self.execute_io(instruction, state, mmu)?;
            }
            "INVD" | "WBINVD" => {
                self.execute_cache_ctrl(instruction, state)?;
            }
            "WRMSR" | "RDMSR" => {
                self.execute_msr(instruction, state)?;
            }
            "SYSCALL" | "SYSRET" => {
                self.execute_syscall(instruction, state)?;
            }
            "INVVPID" | "INVLPG" => {
                self.execute_invept(instruction, state, mmu)?;
            }
            _ => {
                state.rip = instruction.next_ip;
            }
        }

        Ok(())
    }

    fn execute_mov(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        state.rip += 1;
        Ok(())
    }

    fn execute_add(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        state.rip += 1;
        Ok(())
    }

    fn execute_push(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        state.rsp = state.rsp.saturating_sub(8);
        state.rip += 1;
        Ok(())
    }

    fn execute_pop(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        state.rsp = state.rsp.saturating_add(8);
        state.rip += 1;
        Ok(())
    }

    fn execute_jump(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        state.rip += 1;
        Ok(())
    }

    fn execute_call(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        state.rsp = state.rsp.saturating_sub(8);
        state.rip += 1;
        Ok(())
    }

    fn execute_ret(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        state.rsp = state.rsp.saturating_add(8);
        state.rip += 1;
        Ok(())
    }

    fn execute_int(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _intc: &mut InterruptController) -> Result<(), CpuError> {
        state.rip += 1;
        Ok(())
    }

    fn execute_iret(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _intc: &mut InterruptController) -> Result<(), CpuError> {
        state.rsp = state.rsp.saturating_add(8);
        state.rip += 1;
        Ok(())
    }

    fn execute_lgdt(&self, _instruction: &DecodedInstruction, _state: &mut CpuState) -> Result<(), CpuError> {
        Ok(())
    }

    fn execute_lidt(&self, _instruction: &DecodedInstruction, _state: &mut CpuState) -> Result<(), CpuError> {
        Ok(())
    }

    fn execute_movsx(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        state.rip += 1;
        Ok(())
    }

    fn execute_lea(&self, _instruction: &DecodedInstruction, state: &mut CpuState) -> Result<(), CpuError> {
        state.rip += 1;
        Ok(())
    }

    fn execute_cmp(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        state.rip += 1;
        Ok(())
    }

    fn execute_test(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        state.rip += 1;
        Ok(())
    }

    fn execute_logical(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        state.rip += 1;
        Ok(())
    }

    fn execute_shift(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        state.rip += 1;
        Ok(())
    }

    fn execute_string(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        state.rip += 1;
        Ok(())
    }

    fn execute_io(&self, _instruction: &DecodedInstruction, state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        state.rip += 1;
        Ok(())
    }

    fn execute_cache_ctrl(&self, _instruction: &DecodedInstruction, _state: &mut CpuState) -> Result<(), CpuError> {
        Ok(())
    }

    fn execute_msr(&self, _instruction: &DecodedInstruction, _state: &mut CpuState) -> Result<(), CpuError> {
        Ok(())
    }

    fn execute_syscall(&self, _instruction: &DecodedInstruction, _state: &mut CpuState) -> Result<(), CpuError> {
        Ok(())
    }

    fn execute_invept(&self, _instruction: &DecodedInstruction, _state: &mut CpuState, _mmu: &mut Mmu) -> Result<(), CpuError> {
        Ok(())
    }
}