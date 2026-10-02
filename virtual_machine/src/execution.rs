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
use ghostos_observability::{
    record_profile_sample, CacheEvent, CacheKind, CachePolicyReport, CachePolicyRegistry,
    ProfileDomain, ProfileSample,
};
use std::rc::Rc;

#[path = "execution_native.rs"]
mod native;

const DEFAULT_BLOCK_SIZE: usize = 32;
const DEFAULT_HOT_THRESHOLD: u64 = 1_024;
const DEFAULT_CACHE_CAPACITY: usize = 4_096;
const CACHE_WORKLOAD: u64 = 1;

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
            enable_profiling: false,
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
#[repr(C)]
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
    instructions: Rc<[DecodedInstruction]>,
    loop_block: bool,
    compiled: bool,
    hot_executions: u64,
    source_start: u64,
    source_bytes: Rc<[u8]>,
}

impl TranslationBlock {
    fn cache_bytes(&self) -> u64 {
        (self.source_bytes.len() as u64).saturating_add((self.instructions.len() * 16) as u64)
    }

    fn source_is_valid(&self, mmu: &Mmu) -> bool {
        mmu.bytes_equal(self.source_start, self.source_bytes.as_ref())
    }

    fn instruction_source_is_valid(&self, instruction: &DecodedInstruction, mmu: &Mmu) -> bool {
        let Some(range) = native::source_range(self.source_start, self.source_bytes.len(), instruction.ip, instruction.next_ip) else {
            return false
        };
        let source = &self.source_bytes[range];
        mmu.bytes_equal(instruction.ip, source)
    }
}

/// Dynamic translation and execution engine used by [`crate::Vm`].
pub struct ExecutionEngine {
    config: ExecutionEngineConfig,
    cache: native::Cache,
    profiles: native::Profiles,
    stats: ExecutionStats,
    cache_policy: CachePolicyRegistry<1>,
    observed_code_version: u64,
    observed_translation_version: u64,
    profile_hook: Option<Box<dyn FnMut(&ExecutionStats)>>,
}

impl ExecutionEngine {
    pub fn new() -> Self {
        Self::with_config(ExecutionEngineConfig::default())
    }

    pub fn with_config(config: ExecutionEngineConfig) -> Self {
        let mut cache_policy = CachePolicyRegistry::new();
        cache_policy
            .configure(
                CacheKind::VmTranslationBlocks,
                CACHE_WORKLOAD,
                CacheKind::VmTranslationBlocks.default_policy(),
            )
            .expect("built-in VM cache policy is valid");
        Self {
            config,
            cache: native::Cache::new(),
            profiles: native::Profiles::new(),
            stats: ExecutionStats::default(),
            cache_policy,
            observed_code_version: 0,
            observed_translation_version: 0,
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

    pub fn cache_policy_mut(&mut self) -> &mut CachePolicyRegistry<1> {
        &mut self.cache_policy
    }

    pub fn cache_report(&self) -> CachePolicyReport {
        self.cache_policy
            .reports()
            .next()
            .expect("VM translation cache policy is configured")
    }

    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    pub fn block_profile(&self, rip: u64) -> Option<&BlockProfile> {
        self.profiles.get(rip)
    }

    pub fn instruction_count(&self, rip: u64) -> u64 {
        self.profiles.count(rip)
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
        self.cache.clear(false);
        self.profiles.clear();
        self.stats = ExecutionStats::default();
        self.observed_code_version = 0;
        self.observed_translation_version = 0;
    }

    pub fn clear_cache(&mut self) {
        self.cache.clear(true);
        let _ = self
            .cache_policy
            .clear_bytes(CacheKind::VmTranslationBlocks, CACHE_WORKLOAD);
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

        mmu.refresh_page_table_pages();
        let code_changed = self.invalidate_if_guest_code_changed(mmu);
        let translation_changed = self.invalidate_if_guest_translation_changed(mmu);

        let key = BlockKey {
            rip: cpu.state.rip,
            mode: cpu.state.mode,
            privilege: cpu.state.privilege,
            cr3: cpu.state.cr3,
        };
        let block = self.get_block(key, cpu, mmu, code_changed || translation_changed)?;
        native::run(self, cpu, mmu, intc, ports, bios, &block, key.rip, max_instructions)
    }

    fn invalidate_if_guest_code_changed(&mut self, mmu: &Mmu) -> bool {
        native::observe_version(&mut self.observed_code_version, mmu.code_version())
    }

    fn invalidate_if_guest_translation_changed(&mut self, mmu: &Mmu) -> bool {
        native::observe_version(&mut self.observed_translation_version, mmu.translation_version())
    }

    fn get_block(
        &mut self,
        key: BlockKey,
        cpu: &Cpu,
        mmu: &mut Mmu,
        code_changed: bool,
    ) -> Result<TranslationBlock, CpuError> {
        let cached_block_is_valid = !code_changed
            || self
                .cache
                .get(&key)
                .is_some_and(|block| block.source_is_valid(mmu));
        if cached_block_is_valid {
            if let Some(block) = self.cache.get_mut(&key) {
                native::increment(&mut self.stats.cache_hits);
                let _ = self.cache_policy.observe(
                    CacheKind::VmTranslationBlocks,
                    CACHE_WORKLOAD,
                    CacheEvent::Hit,
                );
                let promote = native::promote(self.config.enable_jit, block, self.config.hot_threshold);
                if self.config.enable_profiling {
                    self.profiles.execution(key.rip, 0);
                }
                if promote {
                    block.compiled = true;
                    if self.config.enable_profiling {
                        self.profiles.compiled(key.rip);
                    }
                    native::increment(&mut self.stats.compiled_blocks);
                }
                let block = block.clone();
                self.retune_cache_if_due();
                return Ok(block);
            }
        } else {
            if let Some(block) = self.cache.remove(&key) {
                let _ = self.cache_policy.observe(
                    CacheKind::VmTranslationBlocks,
                    CACHE_WORKLOAD,
                    CacheEvent::StaleRejected,
                );
                let _ = self.cache_policy.observe(
                    CacheKind::VmTranslationBlocks,
                    CACHE_WORKLOAD,
                    CacheEvent::Eviction {
                        bytes: block.cache_bytes(),
                        cost_us: 0,
                    },
                );
            }
            self.cache.forget_order(&key);
        }

        native::increment(&mut self.stats.cache_misses);
        let _ = self.cache_policy.observe(
            CacheKind::VmTranslationBlocks,
            CACHE_WORKLOAD,
            CacheEvent::Miss,
        );
        self.retune_cache_if_due();
        let (instructions, source_bytes) = self.translate_block(key.rip, cpu, mmu)?;
        let loop_block = native::loop_block(key.rip, &instructions);
        let block = TranslationBlock {
            loop_block,
            instructions: Rc::from(instructions.into_boxed_slice()),
            compiled: false,
            hot_executions: 1,
            source_start: key.rip,
            source_bytes: Rc::from(source_bytes.into_boxed_slice()),
        };
        native::increment(&mut self.stats.translated_blocks);
        if self.config.enable_profiling {
            self.profiles.execution(key.rip, key.rip);
        }
        self.insert_block(key, block.clone());
        Ok(block)
    }

    fn translate_block(
        &self,
        rip: u64,
        cpu: &Cpu,
        mmu: &mut Mmu,
    ) -> Result<(Vec<DecodedInstruction>, Vec<u8>), CpuError> {
        native::translate(cpu, mmu, rip, self.config.max_block_instructions)
    }

    fn insert_block(&mut self, key: BlockKey, block: TranslationBlock) {
        if let Some(old_block) = self.cache.evict(&key, self.config.cache_capacity) {
            native::increment(&mut self.stats.cache_evictions);
            let _ = self.cache_policy.observe(
                CacheKind::VmTranslationBlocks,
                CACHE_WORKLOAD,
                CacheEvent::Eviction {
                    bytes: old_block.cache_bytes(),
                    cost_us: 0,
                },
            );
        }
        if !self
            .cache_policy
            .admit(
                CacheKind::VmTranslationBlocks,
                CACHE_WORKLOAD,
                block.cache_bytes(),
            )
            .unwrap_or(false)
        {
            return
        }
        self.cache.insert(key, block);
    }

    fn record_instruction(&mut self, rip: u64, block_start: u64, compiled: bool) {
        native::increment(&mut self.stats.instructions);
        if self.config.enable_profiling {
            self.profiles.record(rip, block_start, compiled);
        }
    }

    fn retune_cache_if_due(&mut self) {
        if native::retune(self.stats.cache_hits, self.stats.cache_misses) {
            let _ = self
                .cache_policy
                .retune(CacheKind::VmTranslationBlocks, CACHE_WORKLOAD);
        }
    }

    fn notify_profile_hook(&mut self) {
        if self.config.enable_profiling {
            record_profile_sample(ProfileSample::single(
                ProfileDomain::VmExecution,
                self.stats.instructions,
                0,
                0x8001,
            ));
        }
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
