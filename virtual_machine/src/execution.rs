//! Translation and execution engine for the emulated CPU.
//!
//! The engine keeps decoded straight-line blocks instead of decoding every
//! instruction repeatedly. Hot blocks are promoted to the compiled form. The
//! compiled form is a portable decoded IR block, not host machine code, so it
//! works on every host supported by Rust and keeps all guest memory and device
//! accesses inside the existing CPU executor.

use crate::cpu::decoder::{DecodedInstruction, Operand};
use crate::cpu::{Cpu, CpuError, CpuMode, PrivilegeLevel};
use crate::devices::{InterruptController, PortBus};
use crate::firmware::bios::BiosContext;
use crate::memory::Mmu;
use std::collections::HashMap;

const DEFAULT_BLOCK_SIZE: usize = 32;
const DEFAULT_HOT_THRESHOLD: u64 = 1_024;
const DEFAULT_CACHE_CAPACITY: usize = 4_096;

/// Controls translation cache and hot-block behavior.
#[derive(Clone, Debug)]
pub struct ExecutionEngineConfig {
    /// Maximum number of instructions translated into one block.
    pub max_block_instructions: usize,
    /// Number of executions before a loop block is promoted to compiled IR.
    pub hot_threshold: u64,
    /// Maximum number of translated blocks retained in the cache.
    pub cache_capacity: usize,
    /// Enable hot-block promotion.
    pub enable_jit: bool,
    /// Collect per-instruction and per-block counters.
    pub enable_profiling: bool,
}

impl Default for ExecutionEngineConfig {
    fn default() -> Self {
        Self {
            max_block_instructions: DEFAULT_BLOCK_SIZE,
            hot_threshold: DEFAULT_HOT_THRESHOLD,
            cache_capacity: DEFAULT_CACHE_CAPACITY,
            enable_jit: true,
            enable_profiling: true,
        }
    }
}

/// Aggregate execution counters.
#[derive(Clone, Debug, Default)]
pub struct ExecutionStats {
    pub instructions: u64,
    pub translated_blocks: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub compiled_blocks: u64,
    pub cache_evictions: u64,
}

/// Profile for one translated block.
#[derive(Clone, Debug, Default)]
pub struct BlockProfile {
    pub start: u64,
    pub executions: u64,
    pub instructions: u64,
    pub compiled: bool,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct BlockKey {
    rip: u64,
    mode: CpuMode,
    privilege: PrivilegeLevel,
    cr3: u64,
}

#[derive(Clone, Debug)]
struct TranslationBlock {
    instructions: Vec<DecodedInstruction>,
    loop_block: bool,
    compiled: bool,
    last_used: u64,
}

/// Dynamic translation and execution engine used by [`crate::Vm`].
pub struct ExecutionEngine {
    config: ExecutionEngineConfig,
    cache: HashMap<BlockKey, TranslationBlock>,
    profiles: HashMap<u64, BlockProfile>,
    instruction_counts: HashMap<u64, u64>,
    stats: ExecutionStats,
    clock: u64,
    observed_code_version: u64,
    profile_hook: Option<Box<dyn FnMut(&ExecutionStats)>>,
}

impl ExecutionEngine {
    pub fn new() -> Self {
        Self::with_config(ExecutionEngineConfig::default())
    }

    pub fn with_config(config: ExecutionEngineConfig) -> Self {
        Self {
            config,
            cache: HashMap::new(),
            profiles: HashMap::new(),
            instruction_counts: HashMap::new(),
            stats: ExecutionStats::default(),
            clock: 0,
            observed_code_version: 0,
            profile_hook: None,
        }
    }

    pub fn config(&self) -> &ExecutionEngineConfig {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut ExecutionEngineConfig {
        &mut self.config
    }

    pub fn stats(&self) -> &ExecutionStats {
        &self.stats
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    pub fn block_profile(&self, rip: u64) -> Option<&BlockProfile> {
        self.profiles.get(&rip)
    }

    pub fn instruction_count(&self, rip: u64) -> u64 {
        self.instruction_counts.get(&rip).copied().unwrap_or(0)
    }

    /// Install a callback invoked after each engine dispatch. The callback
    /// receives a snapshot view and may export counters to a monitor.
    pub fn set_profile_hook<F>(&mut self, hook: F)
    where
        F: FnMut(&ExecutionStats) + 'static,
    {
        self.profile_hook = Some(Box::new(hook));
    }

    pub fn clear_profile_hook(&mut self) {
        self.profile_hook = None;
    }

    /// Drop translated code and counters. Guest RAM is left untouched.
    pub fn reset(&mut self) {
        self.cache.clear();
        self.profiles.clear();
        self.instruction_counts.clear();
        self.stats = ExecutionStats::default();
        self.clock = 0;
        self.observed_code_version = 0;
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }

    /// Execute up to `max_instructions`, returning the number actually run.
    /// A dispatch normally executes a complete translated block. It stops at
    /// control-flow, interrupt, privilege, self-modifying-code, and device
    /// boundaries so the VM can service devices between dispatches.
    pub fn execute(
        &mut self,
        cpu: &mut Cpu,
        mmu: &mut Mmu,
        intc: &mut InterruptController,
        ports: &mut PortBus,
        bios: &mut BiosContext,
        max_instructions: usize,
    ) -> Result<usize, CpuError> {
        if max_instructions == 0 || cpu.state.halted {
            return Ok(0);
        }

        self.invalidate_if_guest_code_changed(mmu);
        self.clock = self.clock.wrapping_add(1);

        let key = BlockKey {
            rip: cpu.state.rip,
            mode: cpu.state.mode,
            privilege: cpu.state.privilege,
            cr3: cpu.state.cr3,
        };
        let block = self.get_block(key, cpu, mmu)?;
        let block_len = block.instructions.len().min(max_instructions);
        let mut executed = 0usize;
        let initial_code_version = mmu.code_version();

        for instruction in block.instructions.iter().take(block_len) {
            if cpu.state.halted || cpu.state.rip != instruction.ip {
                break;
            }

            let mode = cpu.state.mode;
            cpu.prepare_instruction();
            cpu.execute_decoded(instruction, mmu, intc, ports, bios)?;
            executed += 1;
            self.record_instruction(instruction.ip, key.rip, block.compiled);

            if mmu.code_version() != initial_code_version
                || cpu.state.mode != mode
                || is_block_boundary(instruction)
            {
                break;
            }
        }

        if executed == 0 && !cpu.state.halted {
            self.clear_cache();
        }
        self.notify_profile_hook();
        Ok(executed)
    }

    fn invalidate_if_guest_code_changed(&mut self, mmu: &Mmu) {
        let version = mmu.code_version();
        if version != self.observed_code_version {
            self.cache.clear();
            self.observed_code_version = version;
        }
    }

    fn get_block(
        &mut self,
        key: BlockKey,
        cpu: &Cpu,
        mmu: &Mmu,
    ) -> Result<TranslationBlock, CpuError> {
        if let Some(block) = self.cache.get_mut(&key) {
            self.stats.cache_hits += 1;
            self.clock = self.clock.wrapping_add(1);
            block.last_used = self.clock;
            let profile = self.profiles.entry(key.rip).or_default();
            profile.executions += 1;
            if self.config.enable_jit
                && block.loop_block
                && !block.compiled
                && profile.executions >= self.config.hot_threshold
            {
                block.compiled = true;
                profile.compiled = true;
                self.stats.compiled_blocks += 1;
            }
            return Ok(block.clone());
        }

        self.stats.cache_misses += 1;
        let instructions = self.translate_block(key.rip, cpu, mmu)?;
        let block = TranslationBlock {
            loop_block: is_loop_block(key.rip, &instructions),
            instructions,
            compiled: false,
            last_used: self.clock,
        };
        self.stats.translated_blocks += 1;
        self.profiles.entry(key.rip).or_insert_with(|| BlockProfile {
            start: key.rip,
            ..BlockProfile::default()
        });
        if let Some(profile) = self.profiles.get_mut(&key.rip) {
            profile.executions += 1;
        }
        self.insert_block(key, block.clone());
        Ok(block)
    }

    fn translate_block(
        &self,
        rip: u64,
        cpu: &Cpu,
        mmu: &Mmu,
    ) -> Result<Vec<DecodedInstruction>, CpuError> {
        let limit = self.config.max_block_instructions.max(1);
        let mut ip = rip;
        let mut instructions = Vec::with_capacity(limit);
        for _ in 0..limit {
            let instruction = cpu.decode_instruction(ip, mmu)?;
            ip = instruction.next_ip;
            let boundary = is_block_boundary(&instruction);
            instructions.push(instruction);
            if boundary {
                break;
            }
        }
        Ok(instructions)
    }

    fn insert_block(&mut self, key: BlockKey, block: TranslationBlock) {
        let capacity = self.config.cache_capacity.max(1);
        if self.cache.len() >= capacity && !self.cache.contains_key(&key) {
            if let Some(old_key) = self
                .cache
                .iter()
                .min_by_key(|(_, block)| block.last_used)
                .map(|(key, _)| *key)
            {
                self.cache.remove(&old_key);
                self.stats.cache_evictions += 1;
            }
        }
        self.cache.insert(key, block);
    }

    fn record_instruction(&mut self, rip: u64, block_start: u64, compiled: bool) {
        self.stats.instructions += 1;
        if self.config.enable_profiling {
            *self.instruction_counts.entry(rip).or_default() += 1;
            let profile = self.profiles.entry(block_start).or_insert_with(|| BlockProfile {
                start: block_start,
                ..BlockProfile::default()
            });
            profile.instructions += 1;
            profile.compiled |= compiled;
        }
    }

    fn notify_profile_hook(&mut self) {
        if let Some(hook) = self.profile_hook.as_mut() {
            hook(&self.stats);
        }
    }
}

impl Default for ExecutionEngine {
    fn default() -> Self {
        Self::new()
    }
}

fn is_block_boundary(instruction: &DecodedInstruction) -> bool {
    matches!(
        instruction.mnemonic,
        "JMP"
            | "JCC"
            | "CALL"
            | "CALLF"
            | "RET"
            | "RETF"
            | "INT"
            | "INT3"
            | "IRET"
            | "HLT"
            | "SYSCALL"
            | "SYSRET"
            | "SYSENTER"
            | "SYSEXIT"
            | "LOOP"
            | "LOOPE"
            | "LOOPNE"
            | "JRCXZ"
            | "IN"
            | "OUT"
            | "INSB"
            | "INSW"
            | "INSD"
            | "INSQ"
            | "OUTSB"
            | "OUTSW"
            | "OUTSD"
            | "OUTSQ"
            | "WRMSR"
            | "RDMSR"
            | "STI"
            | "CLI"
    )
}

fn is_loop_block(start: u64, instructions: &[DecodedInstruction]) -> bool {
    let Some(last) = instructions.last() else {
        return false;
    };
    if !matches!(
        last.mnemonic,
        "JMP" | "JCC" | "LOOP" | "LOOPE" | "LOOPNE" | "JRCXZ"
    ) {
        return false;
    }
    match last.operands.first() {
        Some(Operand::Relative(relative)) => {
            last.next_ip.wrapping_add(*relative as i64 as u64) <= start
        }
        _ => false,
    }
}
