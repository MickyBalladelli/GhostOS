//! Borrowed CPU/MMU/device adapters for the native block dispatcher.
use super::*;
use std::ffi::c_void;

#[repr(C)]
struct State { rip: u64, code_version: u64, translation_version: u64, mode: u32, halted: bool }
#[repr(C)]
struct Instruction { ip: u64, next_ip: u64, mnemonic: *const u8, mnemonic_length: usize }
impl Instruction {
    fn from_decoded(value: &DecodedInstruction) -> Self {
        Self { ip: value.ip, next_ip: value.next_ip, mnemonic: value.mnemonic.as_ptr(), mnemonic_length: value.mnemonic.len() }
    }
}
#[repr(C)]
struct ExecutionHost {
    state: unsafe extern "C-unwind" fn(*mut c_void, *mut State),
    instruction: unsafe extern "C-unwind" fn(*mut c_void, usize, *mut Instruction),
    source_valid: unsafe extern "C-unwind" fn(*mut c_void, usize) -> bool,
    step: unsafe extern "C-unwind" fn(*mut c_void, usize) -> bool,
    record: unsafe extern "C-unwind" fn(*mut c_void, usize),
}
struct Dispatch<'a> {
    engine: &'a mut ExecutionEngine,
    cpu: &'a mut Cpu,
    mmu: &'a mut Mmu,
    intc: &'a mut InterruptController,
    ports: &'a mut PortBus,
    bios: &'a mut BiosContext,
    block: &'a TranslationBlock,
    start: u64,
    error: Option<CpuError>,
}
unsafe extern "C-unwind" fn state(context: *mut c_void, output: *mut State) {
    let host = &*(context as *const Dispatch<'_>);
    output.write(State { rip: host.cpu.state.rip, code_version: host.mmu.code_version(),
        translation_version: host.mmu.translation_version(), mode: host.cpu.state.mode as u32, halted: host.cpu.state.halted });
}
unsafe extern "C-unwind" fn instruction(context: *mut c_void, index: usize, output: *mut Instruction) {
    let host = &*(context as *const Dispatch<'_>);
    output.write(Instruction::from_decoded(&host.block.instructions[index]));
}
unsafe extern "C-unwind" fn source_valid(context: *mut c_void, index: usize) -> bool {
    let host = &*(context as *const Dispatch<'_>);
    host.block.instruction_source_is_valid(&host.block.instructions[index], host.mmu)
}
unsafe extern "C-unwind" fn step(context: *mut c_void, index: usize) -> bool {
    let host = &mut *(context as *mut Dispatch<'_>);
    let instruction = &host.block.instructions[index];
    let shadow = host.cpu.state.interrupt_shadow;
    host.cpu.prepare_instruction();
    host.mmu.set_replay_instruction_ip(Some(instruction.ip));
    host.ports.set_replay_instruction_ip(Some(instruction.ip));
    host.bios.set_replay_instruction_ip(Some(instruction.ip));
    let result = host.cpu.execute_decoded(instruction, host.mmu, host.intc, host.ports, host.bios);
    host.mmu.set_replay_instruction_ip(None);
    host.ports.set_replay_instruction_ip(None);
    host.bios.set_replay_instruction_ip(None);
    if matches!(result, Err(CpuError::UnsupportedInstruction)) { host.cpu.state.interrupt_shadow = shadow; }
    match result { Ok(()) => true, Err(error) => { host.error = Some(error); false } }
}
unsafe extern "C-unwind" fn record(context: *mut c_void, index: usize) {
    let host = &mut *(context as *mut Dispatch<'_>);
    host.engine.record_instruction(host.block.instructions[index].ip, host.start, host.block.compiled);
}

pub(super) fn run(engine: &mut ExecutionEngine, cpu: &mut Cpu, mmu: &mut Mmu,
    intc: &mut InterruptController, ports: &mut PortBus, bios: &mut BiosContext,
    block: &TranslationBlock, start: u64, maximum: usize) -> Result<usize, CpuError> {
    let mut context = Dispatch { engine, cpu, mmu, intc, ports, bios, block, start, error: None };
    let callbacks = ExecutionHost { state, instruction, source_valid, step, record };
    let mut executed = 0;
    let mut clear_cache = false;
    // C borrows the context synchronously, retains no pointers, and indexes
    // only the supplied block length. C-unwind preserves Rust callback panics.
    let success = unsafe { ghostos_vm_execution_run(&callbacks, &mut context as *mut _ as *mut c_void,
        block.instructions.len(), maximum, &mut executed, &mut clear_cache) };
    if !success { return Err(context.error.take().expect("CPU callback error")); }
    if clear_cache { context.engine.clear_cache(); }
    context.engine.notify_profile_hook();
    Ok(executed)
}

#[repr(C)]
struct TranslationHost {
    decode: unsafe extern "C-unwind" fn(*mut c_void, u64, *mut Instruction) -> bool,
    finish: unsafe extern "C-unwind" fn(*mut c_void, u64, usize) -> bool,
}
struct Translation<'a> {
    cpu: &'a Cpu,
    mmu: &'a mut Mmu,
    instructions: Vec<DecodedInstruction>,
    bytes: Vec<u8>,
    error: Option<CpuError>,
}
unsafe extern "C-unwind" fn decode(context: *mut c_void, ip: u64, output: *mut Instruction) -> bool {
    let host = &mut *(context as *mut Translation<'_>);
    match host.cpu.decode_instruction(ip, host.mmu) {
        Ok(instruction) => {
            output.write(Instruction::from_decoded(&instruction));
            host.instructions.push(instruction);
            true
        },
        Err(error) => { host.error = Some(error); false }
    }
}
unsafe extern "C-unwind" fn finish(context: *mut c_void, start: u64, length: usize) -> bool {
    let host = &mut *(context as *mut Translation<'_>);
    match host.mmu.read_bytes(start, length) {
        Ok(bytes) => { host.bytes = bytes; host.mmu.mark_code_range(start, length); host.error = None; true },
        Err(_) => { host.error = Some(CpuError::MemoryAccessError); false }
    }
}
pub(super) fn translate(cpu: &Cpu, mmu: &mut Mmu, start: u64, maximum: usize) -> Result<(Vec<DecodedInstruction>, Vec<u8>), CpuError> {
    let mut context = Translation { cpu, mmu, instructions: Vec::with_capacity(maximum.max(1)), bytes: Vec::new(), error: None };
    let callbacks = TranslationHost { decode, finish };
    let success = unsafe { ghostos_vm_execution_translate(&callbacks, &mut context as *mut _ as *mut c_void, start, maximum) };
    if success { Ok((context.instructions, context.bytes)) }
    else { Err(context.error.expect("translation callback error")) }
}
pub(super) fn source_range(start: u64, bytes: usize, ip: u64, next_ip: u64) -> Option<std::ops::Range<usize>> {
    let mut offset = 0;
    let mut length = 0;
    unsafe { ghostos_vm_execution_source_range(start, bytes, ip, next_ip, &mut offset, &mut length) }.then(|| offset..offset + length)
}
pub(super) fn observe_version(observed: &mut u64, current: u64) -> bool {
    unsafe { ghostos_vm_execution_observe_version(observed, current) }
}
pub(super) fn promote(enabled: bool, block: &mut TranslationBlock, threshold: u64) -> bool {
    unsafe { ghostos_vm_execution_promote(enabled, block.loop_block, block.compiled, &mut block.hot_executions, threshold) }
}
pub(super) fn retune(hits: u64, misses: u64) -> bool {
    unsafe { ghostos_vm_execution_retune(hits, misses) }
}
pub(super) fn loop_block(start: u64, instructions: &[DecodedInstruction]) -> bool {
    let Some(last) = instructions.last() else { return false; };
    let displacement = match last.operands.first() { Some(Operand::Relative(value)) => Some(*value as i64 as u64), _ => None };
    unsafe { ghostos_vm_execution_loop(start, &Instruction::from_decoded(last), displacement.is_some(), displacement.unwrap_or(0)) }
}

extern "C-unwind" {
    fn ghostos_vm_execution_run(host: *const ExecutionHost, context: *mut c_void, count: usize, maximum: usize, executed: *mut usize, clear_cache: *mut bool) -> bool;
    fn ghostos_vm_execution_translate(host: *const TranslationHost, context: *mut c_void, start: u64, maximum: usize) -> bool;
    fn ghostos_vm_execution_source_range(start: u64, bytes: usize, ip: u64, next_ip: u64, offset: *mut usize, length: *mut usize) -> bool;
    fn ghostos_vm_execution_observe_version(observed: *mut u64, current: u64) -> bool;
    fn ghostos_vm_execution_promote(enabled: bool, loop_block: bool, compiled: bool, hot: *mut u64, threshold: u64) -> bool;
    fn ghostos_vm_execution_retune(hits: u64, misses: u64) -> bool;
    fn ghostos_vm_execution_loop(start: u64, last: *const Instruction, relative: bool, displacement: u64) -> bool;
}

#[repr(C)]
struct CacheKey { rip: u64, cr3: u64, mode: u32, privilege: u32 }
impl From<BlockKey> for CacheKey {
    fn from(key: BlockKey) -> Self {
        Self { rip: key.rip, cr3: key.cr3, mode: key.mode as u32, privilege: key.privilege as u32 }
    }
}

/// Owns the native hash table and FIFO, including the boxed block payloads.
/// Returned references borrow this owner, so no native mutation can invalidate
/// a live Rust reference. Removal transfers the box back to Rust exactly once.
pub(super) struct Cache(std::ptr::NonNull<c_void>);
unsafe extern "C" fn destroy_block(payload: *mut c_void) {
    drop(Box::from_raw(payload as *mut TranslationBlock));
}
impl Cache {
    pub(super) fn new() -> Self {
        Self(std::ptr::NonNull::new(unsafe { ghostos_vm_execution_cache_new(destroy_block) })
            .expect("allocate native translation cache"))
    }
    pub(super) fn len(&self) -> usize {
        unsafe { ghostos_vm_execution_cache_length(self.0.as_ptr()) }
    }
    pub(super) fn get(&self, key: &BlockKey) -> Option<&TranslationBlock> {
        let pointer = unsafe { ghostos_vm_execution_cache_get(self.0.as_ptr(), &CacheKey::from(*key)) };
        unsafe { (pointer as *const TranslationBlock).as_ref() }
    }
    pub(super) fn get_mut(&mut self, key: &BlockKey) -> Option<&mut TranslationBlock> {
        let pointer = unsafe { ghostos_vm_execution_cache_get(self.0.as_ptr(), &CacheKey::from(*key)) };
        unsafe { (pointer as *mut TranslationBlock).as_mut() }
    }
    fn take(pointer: *mut c_void) -> Option<Box<TranslationBlock>> {
        if pointer.is_null() { None } else { Some(unsafe { Box::from_raw(pointer as *mut TranslationBlock) }) }
    }
    pub(super) fn remove(&mut self, key: &BlockKey) -> Option<Box<TranslationBlock>> {
        Self::take(unsafe { ghostos_vm_execution_cache_remove(self.0.as_ptr(), &CacheKey::from(*key)) })
    }
    pub(super) fn forget_order(&mut self, key: &BlockKey) {
        unsafe { ghostos_vm_execution_cache_forget_order(self.0.as_ptr(), &CacheKey::from(*key)); }
    }
    pub(super) fn evict(&mut self, key: &BlockKey, capacity: usize) -> Option<Box<TranslationBlock>> {
        Self::take(unsafe { ghostos_vm_execution_cache_evict(self.0.as_ptr(), &CacheKey::from(*key), capacity) })
    }
    pub(super) fn insert(&mut self, key: BlockKey, block: TranslationBlock) {
        let pointer = Box::into_raw(Box::new(block));
        let success = unsafe { ghostos_vm_execution_cache_insert(self.0.as_ptr(), &CacheKey::from(key), pointer as *mut c_void) };
        if !success {
            // On native allocation failure the cache has not taken ownership.
            drop(unsafe { Box::from_raw(pointer) });
            panic!("allocate native translation cache entry");
        }
    }
    pub(super) fn clear(&mut self, clear_order: bool) {
        unsafe { ghostos_vm_execution_cache_clear(self.0.as_ptr(), clear_order); }
    }
}
impl Drop for Cache {
    fn drop(&mut self) { unsafe { ghostos_vm_execution_cache_free(self.0.as_ptr()); } }
}
extern "C" {
    fn ghostos_vm_execution_cache_new(destroy: unsafe extern "C" fn(*mut c_void)) -> *mut c_void;
    fn ghostos_vm_execution_cache_free(cache: *mut c_void);
    fn ghostos_vm_execution_cache_clear(cache: *mut c_void, clear_order: bool);
    fn ghostos_vm_execution_cache_length(cache: *const c_void) -> usize;
    fn ghostos_vm_execution_cache_get(cache: *const c_void, key: *const CacheKey) -> *mut c_void;
    fn ghostos_vm_execution_cache_remove(cache: *mut c_void, key: *const CacheKey) -> *mut c_void;
    fn ghostos_vm_execution_cache_forget_order(cache: *mut c_void, key: *const CacheKey);
    fn ghostos_vm_execution_cache_evict(cache: *mut c_void, key: *const CacheKey, capacity: usize) -> *mut c_void;
    fn ghostos_vm_execution_cache_insert(cache: *mut c_void, key: *const CacheKey, payload: *mut c_void) -> bool;
}

/// Owns two native maps. Mutation requires an exclusive owner borrow, including
/// calls that create entries; lookups never create or relocate borrowed values.
pub(super) struct Profiles(std::ptr::NonNull<c_void>);
impl Profiles {
    pub(super) fn new() -> Self {
        Self(std::ptr::NonNull::new(unsafe { ghostos_vm_execution_profiles_new() })
            .expect("allocate native execution profiles"))
    }
    pub(super) fn get(&self, rip: u64) -> Option<&BlockProfile> {
        unsafe { ghostos_vm_execution_profiles_get(self.0.as_ptr(), rip, 0, false).as_ref() }
    }
    pub(super) fn execution(&mut self, rip: u64, initial_start: u64) {
        profile_result(unsafe { ghostos_vm_execution_profiles_execution(self.0.as_ptr(), rip, initial_start, cfg!(debug_assertions)) });
    }
    pub(super) fn compiled(&mut self, rip: u64) {
        unsafe { ghostos_vm_execution_profiles_compiled(self.0.as_ptr(), rip); }
    }
    pub(super) fn record(&mut self, rip: u64, block_start: u64, compiled: bool) {
        profile_result(unsafe { ghostos_vm_execution_profiles_record(self.0.as_ptr(), rip, block_start, compiled, cfg!(debug_assertions)) });
    }
    pub(super) fn count(&self, rip: u64) -> u64 {
        unsafe { ghostos_vm_execution_profiles_counter(self.0.as_ptr(), rip, false).as_ref() }.copied().unwrap_or(0)
    }
    pub(super) fn clear(&mut self) { unsafe { ghostos_vm_execution_profiles_clear(self.0.as_ptr()); } }
}
impl Drop for Profiles {
    fn drop(&mut self) { unsafe { ghostos_vm_execution_profiles_free(self.0.as_ptr()); } }
}
extern "C" {
    fn ghostos_vm_execution_profiles_new() -> *mut c_void;
    fn ghostos_vm_execution_profiles_free(profiles: *mut c_void);
    fn ghostos_vm_execution_profiles_clear(profiles: *mut c_void);
    fn ghostos_vm_execution_profiles_get(profiles: *mut c_void, rip: u64, initial_start: u64, create: bool) -> *mut BlockProfile;
    fn ghostos_vm_execution_profiles_counter(profiles: *mut c_void, rip: u64, create: bool) -> *mut u64;
}

fn profile_result(result: u32) {
    match result {
        0 => {},
        1 => panic!("allocate native execution profile/counter"),
        2 => panic!("attempt to add with overflow"),
        _ => unreachable!("native execution profile result"),
    }
}
pub(super) fn increment(counter: &mut u64) {
    assert!(unsafe { ghostos_vm_execution_increment(counter, cfg!(debug_assertions)) }, "attempt to add with overflow");
}
extern "C" {
    fn ghostos_vm_execution_increment(counter: *mut u64, checked: bool) -> bool;
    fn ghostos_vm_execution_profiles_execution(profiles: *mut c_void, rip: u64, initial_start: u64, checked: bool) -> u32;
    fn ghostos_vm_execution_profiles_compiled(profiles: *mut c_void, rip: u64);
    fn ghostos_vm_execution_profiles_record(profiles: *mut c_void, rip: u64, block_start: u64, compiled: bool, checked: bool) -> u32;
}
