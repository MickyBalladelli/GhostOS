//! VM checkpoints, portable serialization, page diffs, and snapshot chains.

use crate::cpu::{CpuMode, CpuState, DescriptorTableRegister, PrivilegeLevel, SegmentRegister};
use crate::devices::{InterruptControllerState, LocalApicState};
use crate::firmware::bios::BiosState;
use crate::memory::{MmuState, PAGE_SIZE};
use crate::Vm;
use std::fs;
use std::path::Path;

const MAGIC: &[u8; 8] = b"SYNOVM01";
pub const SNAPSHOT_FORMAT_VERSION: u32 = 1;
const MAX_ITEMS: usize = 16 * 1024 * 1024;

pub type SnapshotId = u64;

#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    #[error("snapshot I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid snapshot format")]
    InvalidFormat,
    #[error("unsupported snapshot version {0}")]
    VersionMismatch(u32),
    #[error("snapshot belongs to a VM with {expected} bytes of RAM, not {actual}")]
    IncompatibleMemory { expected: usize, actual: usize },
    #[error("snapshot state is invalid")]
    InvalidState,
    #[error("snapshot checksum does not match its base")]
    ChecksumMismatch,
    #[error("snapshot {0} does not exist in the chain")]
    MissingSnapshot(SnapshotId),
}

/// Complete guest checkpoint. Host-side translation caches and network
/// backends are deliberately excluded; they are rebuilt or remain attached
/// to the VM topology after restore.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VmSnapshot {
    pub format_version: u32,
    pub memory_size: usize,
    pub cpu: CpuState,
    pub mmu: MmuState,
    pub interrupt_controller: InterruptControllerState,
    pub apic: LocalApicState,
    pub bios_state: BiosState,
    pub bios_ivt: [u16; 256],
    pub bios_bda: [u8; 256],
    pub bios_ega: [u8; 32 * 4],
    pub bios_reset_vector: u64,
}

impl VmSnapshot {
    pub fn capture(vm: &Vm) -> Self {
        Self {
            format_version: SNAPSHOT_FORMAT_VERSION,
            memory_size: vm.mmu.ram_size(),
            cpu: vm.cpu.state,
            mmu: vm.mmu.snapshot_state(),
            interrupt_controller: vm.interrupt_controller.snapshot_state(),
            apic: vm.apic.borrow().snapshot_state(),
            bios_state: vm.bios.context.state,
            bios_ivt: vm.bios.context.ivt,
            bios_bda: vm.bios.context.bda,
            bios_ega: vm.bios.context.ega,
            bios_reset_vector: vm.bios.reset_vector,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.bytes(MAGIC);
        writer.u32(self.format_version);
        writer.u64(self.memory_size as u64);
        encode_cpu(&mut writer, &self.cpu);
        encode_mmu(&mut writer, &self.mmu);
        encode_interrupt_controller(&mut writer, &self.interrupt_controller);
        encode_apic(&mut writer, &self.apic);
        writer.u8(encode_bios_state(self.bios_state));
        for value in self.bios_ivt {
            writer.u16(value);
        }
        writer.bytes(&self.bios_bda);
        writer.bytes(&self.bios_ega);
        writer.u64(self.bios_reset_vector);
        writer.finish()
    }

    pub fn serialize(&self) -> Vec<u8> {
        self.to_bytes()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SnapshotError> {
        let mut reader = Reader::new(bytes);
        if reader.bytes_exact(8)? != MAGIC {
            return Err(SnapshotError::InvalidFormat)
        }
        let format_version = reader.u32()?;
        if format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(SnapshotError::VersionMismatch(format_version))
        }
        let memory_size = reader.u64()? as usize;
        let cpu = decode_cpu(&mut reader)?;
        let mmu = decode_mmu(&mut reader)?;
        if mmu.ram.len() != memory_size {
            return Err(SnapshotError::InvalidFormat)
        }
        let interrupt_controller = decode_interrupt_controller(&mut reader)?;
        let apic = decode_apic(&mut reader)?;
        let bios_state = decode_bios_state(reader.u8()?)?;
        let mut bios_ivt = [0u16; 256];
        for value in &mut bios_ivt {
            *value = reader.u16()?
        }
        let bios_bda = reader.array::<256>()?;
        let bios_ega = reader.array::<128>()?;
        let bios_reset_vector = reader.u64()?;
        if !reader.is_empty() {
            return Err(SnapshotError::InvalidFormat)
        }
        Ok(Self {
            format_version,
            memory_size,
            cpu,
            mmu,
            interrupt_controller,
            apic,
            bios_state,
            bios_ivt,
            bios_bda,
            bios_ega,
            bios_reset_vector,
        })
    }

    pub fn deserialize(bytes: &[u8]) -> Result<Self, SnapshotError> {
        Self::from_bytes(bytes)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), SnapshotError> {
        fs::write(path, self.to_bytes()).map_err(SnapshotError::Io)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, SnapshotError> {
        Self::from_bytes(&fs::read(path)?)
    }

    pub fn checksum(&self) -> u64 {
        checksum(&self.to_bytes())
    }

    /// Build a page-level delta from `self` to `target`.
    pub fn diff(&self, target: &Self) -> Result<SnapshotDiff, SnapshotError> {
        if self.memory_size != target.memory_size {
            return Err(SnapshotError::IncompatibleMemory {
                expected: self.memory_size,
                actual: target.memory_size,
            })
        }
        let mut changed_pages = Vec::new();
        for (page, (before, after)) in self
            .mmu
            .ram
            .chunks(PAGE_SIZE)
            .zip(target.mmu.ram.chunks(PAGE_SIZE))
            .enumerate()
        {
            if before != after {
                changed_pages.push(SnapshotPage {
                    page: page as u64,
                    data: after.to_vec(),
                })
            }
        }
        let mut target_mmu = target.mmu.clone();
        target_mmu.ram.clear();
        Ok(SnapshotDiff {
            base_checksum: self.checksum(),
            target_memory_size: target.memory_size,
            target_cpu: target.cpu,
            target_mmu,
            target_interrupt_controller: target.interrupt_controller.clone(),
            target_apic: target.apic.clone(),
            target_bios_state: target.bios_state,
            target_bios_ivt: target.bios_ivt,
            target_bios_bda: target.bios_bda,
            target_bios_ega: target.bios_ega,
            target_bios_reset_vector: target.bios_reset_vector,
            changed_pages,
        })
    }
}

impl VmSnapshot {
    pub fn restore_into(&self, vm: &mut Vm) -> Result<(), SnapshotError> {
        if self.format_version != SNAPSHOT_FORMAT_VERSION {
            return Err(SnapshotError::VersionMismatch(self.format_version))
        }
        if vm.mmu.ram_size() != self.memory_size {
            return Err(SnapshotError::IncompatibleMemory {
                expected: vm.mmu.ram_size(),
                actual: self.memory_size,
            })
        }
        vm.mmu
            .restore_state(&self.mmu)
            .map_err(|_| SnapshotError::InvalidState)?;
        vm.cpu.state = self.cpu;
        vm.interrupt_controller.restore_state(&self.interrupt_controller);
        vm.apic.borrow_mut().restore_state(&self.apic);
        vm.bios.context.state = self.bios_state;
        vm.bios.context.ivt = self.bios_ivt;
        vm.bios.context.bda = self.bios_bda;
        vm.bios.context.ega = self.bios_ega;
        vm.bios.reset_vector = self.bios_reset_vector;
        vm.initialized = self.bios_state != BiosState::Reset;
        vm.execution.clear_cache();
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotPage {
    pub page: u64,
    pub data: Vec<u8>,
}

/// A diff stores changed RAM pages and complete non-RAM state. This keeps
/// chains small when a guest mostly reuses its memory while preserving exact
/// restore semantics for CPU, paging, firmware, and interrupt state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotDiff {
    pub base_checksum: u64,
    pub target_memory_size: usize,
    pub target_cpu: CpuState,
    pub target_mmu: MmuState,
    pub target_interrupt_controller: InterruptControllerState,
    pub target_apic: LocalApicState,
    pub target_bios_state: BiosState,
    pub target_bios_ivt: [u16; 256],
    pub target_bios_bda: [u8; 256],
    pub target_bios_ega: [u8; 128],
    pub target_bios_reset_vector: u64,
    pub changed_pages: Vec<SnapshotPage>,
}

impl SnapshotDiff {
    pub fn apply_to(&self, base: &VmSnapshot) -> Result<VmSnapshot, SnapshotError> {
        if base.checksum() != self.base_checksum {
            return Err(SnapshotError::ChecksumMismatch)
        }
        if base.memory_size != self.target_memory_size {
            return Err(SnapshotError::IncompatibleMemory {
                expected: base.memory_size,
                actual: self.target_memory_size,
            })
        }
        let mut mmu = self.target_mmu.clone();
        mmu.ram = base.mmu.ram.clone();
        for page in &self.changed_pages {
            let start = page
                .page
                .checked_mul(PAGE_SIZE as u64)
                .ok_or(SnapshotError::InvalidState)? as usize;
            let end = start
                .checked_add(page.data.len())
                .ok_or(SnapshotError::InvalidState)?;
            if end > mmu.ram.len() || page.data.len() > PAGE_SIZE {
                return Err(SnapshotError::InvalidState)
            }
            mmu.ram[start..end].copy_from_slice(&page.data)
        }
        Ok(VmSnapshot {
            format_version: SNAPSHOT_FORMAT_VERSION,
            memory_size: self.target_memory_size,
            cpu: self.target_cpu,
            mmu,
            interrupt_controller: self.target_interrupt_controller.clone(),
            apic: self.target_apic.clone(),
            bios_state: self.target_bios_state,
            bios_ivt: self.target_bios_ivt,
            bios_bda: self.target_bios_bda,
            bios_ega: self.target_bios_ega,
            bios_reset_vector: self.target_bios_reset_vector,
        })
    }
}

/// Parent-linked snapshot store. Only the first checkpoint owns full RAM;
/// later checkpoints retain page diffs and can be materialized on demand.
#[derive(Clone, Debug)]
pub struct SnapshotChain {
    base: VmSnapshot,
    records: Vec<(SnapshotId, SnapshotId, SnapshotDiff)>,
    next_id: SnapshotId,
}

impl SnapshotChain {
    pub fn new(base: VmSnapshot) -> Self {
        Self {
            base,
            records: Vec::new(),
            next_id: 1,
        }
    }

    pub fn base(&self) -> &VmSnapshot {
        &self.base
    }

    pub fn checkpoint(&mut self, snapshot: VmSnapshot) -> Result<SnapshotId, SnapshotError> {
        let parent = self.latest_id();
        let parent_snapshot = self.snapshot(parent)?;
        let diff = parent_snapshot.diff(&snapshot)?;
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.records.push((id, parent, diff));
        Ok(id)
    }

    pub fn latest_id(&self) -> SnapshotId {
        self.records.last().map(|record| record.0).unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.records.len() + 1
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn snapshot(&self, id: SnapshotId) -> Result<VmSnapshot, SnapshotError> {
        if id == 0 {
            return Ok(self.base.clone())
        }
        let (_, parent, diff) = self
            .records
            .iter()
            .find(|record| record.0 == id)
            .ok_or(SnapshotError::MissingSnapshot(id))?;
        let parent_snapshot = self.snapshot(*parent)?;
        diff.apply_to(&parent_snapshot)
    }

    pub fn restore_into(&self, id: SnapshotId, vm: &mut Vm) -> Result<(), SnapshotError> {
        self.snapshot(id)?.restore_into(vm)
    }
}

fn checksum(bytes: &[u8]) -> u64 {
    let mut value = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x1000_0000_01b3)
    }
    value
}

fn encode_bios_state(state: BiosState) -> u8 {
    match state {
        BiosState::Reset => 0,
        BiosState::Initialized => 1,
        BiosState::UefiInitialized => 2,
        BiosState::Running => 3,
        BiosState::Halted => 4,
    }
}

fn decode_bios_state(value: u8) -> Result<BiosState, SnapshotError> {
    match value {
        0 => Ok(BiosState::Reset),
        1 => Ok(BiosState::Initialized),
        2 => Ok(BiosState::UefiInitialized),
        3 => Ok(BiosState::Running),
        4 => Ok(BiosState::Halted),
        _ => Err(SnapshotError::InvalidFormat),
    }
}

fn encode_cpu(writer: &mut Writer, state: &CpuState) {
    for value in [
        state.rax, state.rbx, state.rcx, state.rdx, state.rsi, state.rdi, state.rbp,
        state.rsp, state.r8, state.r9, state.r10, state.r11, state.r12, state.r13,
        state.r14, state.r15, state.rip, state.rflags, state.cr0, state.cr2, state.cr3,
        state.cr4, state.efer, state.ist_stack, state.star, state.lstar, state.fs_base,
        state.gs_base,
    ] {
        writer.u64(value)
    }
    for segment in [state.cs, state.ds, state.es, state.fs, state.gs, state.ss] {
        encode_segment(writer, &segment)
    }
    encode_descriptor(writer, &state.gdtr);
    encode_descriptor(writer, &state.idtr);
    writer.u8(encode_mode(state.mode));
    writer.u8(encode_privilege(state.privilege));
    writer.bool(state.halted);
    writer.bool(state.interrupt_shadow)
}

fn decode_cpu(reader: &mut Reader<'_>) -> Result<CpuState, SnapshotError> {
    let mut values = [0u64; 28];
    for value in &mut values {
        *value = reader.u64()?
    }
    let cs = decode_segment(reader)?;
    let ds = decode_segment(reader)?;
    let es = decode_segment(reader)?;
    let fs = decode_segment(reader)?;
    let gs = decode_segment(reader)?;
    let ss = decode_segment(reader)?;
    let gdtr = decode_descriptor(reader)?;
    let idtr = decode_descriptor(reader)?;
    let mode = decode_mode(reader.u8()?)?;
    let privilege = decode_privilege(reader.u8()?)?;
    let halted = reader.bool()?;
    let interrupt_shadow = reader.bool()?;
    Ok(CpuState {
        rax: values[0], rbx: values[1], rcx: values[2], rdx: values[3], rsi: values[4],
        rdi: values[5], rbp: values[6], rsp: values[7], r8: values[8], r9: values[9],
        r10: values[10], r11: values[11], r12: values[12], r13: values[13], r14: values[14],
        r15: values[15], rip: values[16], rflags: values[17], cr0: values[18], cr2: values[19],
        cr3: values[20], cr4: values[21], efer: values[22], ist_stack: values[23], star: values[24],
        lstar: values[25], fs_base: values[26], gs_base: values[27], cs, ds, es, fs, gs, ss,
        gdtr, idtr, mode, privilege, halted, interrupt_shadow,
    })
}

fn encode_segment(writer: &mut Writer, segment: &SegmentRegister) {
    writer.u16(segment.selector);
    writer.u64(segment.base);
    writer.u32(segment.limit);
    writer.u16(segment.attributes)
}

fn decode_segment(reader: &mut Reader<'_>) -> Result<SegmentRegister, SnapshotError> {
    Ok(SegmentRegister {
        selector: reader.u16()?,
        base: reader.u64()?,
        limit: reader.u32()?,
        attributes: reader.u16()?,
    })
}

fn encode_descriptor(writer: &mut Writer, descriptor: &DescriptorTableRegister) {
    writer.u64(descriptor.base);
    writer.u16(descriptor.limit)
}

fn decode_descriptor(reader: &mut Reader<'_>) -> Result<DescriptorTableRegister, SnapshotError> {
    Ok(DescriptorTableRegister {
        base: reader.u64()?,
        limit: reader.u16()?,
    })
}

fn encode_mode(mode: CpuMode) -> u8 {
    match mode {
        CpuMode::Real16 => 0,
        CpuMode::Protected16 => 1,
        CpuMode::Protected32 => 2,
        CpuMode::Long64 => 3,
    }
}

fn decode_mode(value: u8) -> Result<CpuMode, SnapshotError> {
    match value {
        0 => Ok(CpuMode::Real16),
        1 => Ok(CpuMode::Protected16),
        2 => Ok(CpuMode::Protected32),
        3 => Ok(CpuMode::Long64),
        _ => Err(SnapshotError::InvalidFormat),
    }
}

fn encode_privilege(privilege: PrivilegeLevel) -> u8 {
    match privilege {
        PrivilegeLevel::Ring0 => 0,
        PrivilegeLevel::Ring3 => 3,
    }
}

fn decode_privilege(value: u8) -> Result<PrivilegeLevel, SnapshotError> {
    match value {
        0 => Ok(PrivilegeLevel::Ring0),
        3 => Ok(PrivilegeLevel::Ring3),
        _ => Err(SnapshotError::InvalidFormat),
    }
}

fn encode_mmu(writer: &mut Writer, state: &MmuState) {
    writer.bytes_vec(&state.ram);
    writer.usize_vec(&state.free_frames);
    writer.u64(state.code_version);
    writer.u64_pairs(&state.identity);
    writer.u64_u64_u64_bool(&state.cow_pages);
    writer.u64_usize_pairs(&state.mapped_frames);
    writer.u64_pairs(&state.mapped_pages);
    writer.u64_vec(&state.ballooned_frames);
    writer.option_u64(state.zero_page);
    writer.usize(state.overcommitted_pages);
    writer.usize(state.overcommit_limit);
    writer.bool(state.paging_enabled);
    writer.u64(state.cr3);
    writer.bool(state.privilege)
}

fn decode_mmu(reader: &mut Reader<'_>) -> Result<MmuState, SnapshotError> {
    Ok(MmuState {
        ram: reader.bytes_vec()?,
        free_frames: reader.usize_vec()?,
        code_version: reader.u64()?,
        identity: reader.u64_pairs()?,
        cow_pages: reader.u64_u64_u64_bool()?,
        mapped_frames: reader.u64_usize_pairs()?,
        mapped_pages: reader.u64_pairs()?,
        ballooned_frames: reader.u64_vec()?,
        zero_page: reader.option_u64()?,
        overcommitted_pages: reader.usize()?,
        overcommit_limit: reader.usize()?,
        paging_enabled: reader.bool()?,
        cr3: reader.u64()?,
        privilege: reader.bool()?,
    })
}

fn encode_interrupt_controller(writer: &mut Writer, state: &InterruptControllerState) {
    writer.u64(state.idt_base);
    writer.u16(state.idt_limit);
    writer.u64_pairs_u8(&state.irq_routing);
    writer.bool(state.pic_mapped)
}

fn decode_interrupt_controller(
    reader: &mut Reader<'_>,
) -> Result<InterruptControllerState, SnapshotError> {
    Ok(InterruptControllerState {
        idt_base: reader.u64()?,
        idt_limit: reader.u16()?,
        irq_routing: reader.u64_pairs_u8()?,
        pic_mapped: reader.bool()?,
    })
}

fn encode_apic(writer: &mut Writer, state: &LocalApicState) {
    writer.u64(state.base);
    writer.bool(state.enabled);
    for value in [
        state.id, state.version, state.tpr, state.ppr, state.ldr, state.dfr, state.svr,
    ] {
        writer.u32(value)
    }
    for table in [state.irr, state.isr, state.tmr, state.level_pending] {
        for value in table {
            writer.u32(value)
        }
    }
    for value in [
        state.esr, state.icr_hi, state.icr_lo, state.lvt_timer, state.lvt_thermal,
        state.lvt_perfmon, state.lvt_lint0, state.lvt_lint1, state.lvt_error,
        state.timer_initial_count, state.timer_current_count, state.timer_divide,
    ] {
        writer.u32(value)
    }
    writer.bool(state.timer_running);
    writer.option_u64(state.timer_last_ns);
    writer.bool(state.timer_fired)
}

fn decode_apic(reader: &mut Reader<'_>) -> Result<LocalApicState, SnapshotError> {
    let base = reader.u64()?;
    let enabled = reader.bool()?;
    let mut values = [0u32; 7];
    for value in &mut values {
        *value = reader.u32()?
    }
    let mut tables = [[0u32; 8]; 4];
    for table in &mut tables {
        for value in table {
            *value = reader.u32()?
        }
    }
    let mut device = [0u32; 12];
    for value in &mut device {
        *value = reader.u32()?
    }
    Ok(LocalApicState {
        base,
        enabled,
        id: values[0],
        version: values[1],
        tpr: values[2],
        ppr: values[3],
        ldr: values[4],
        dfr: values[5],
        svr: values[6],
        irr: tables[0],
        isr: tables[1],
        tmr: tables[2],
        level_pending: tables[3],
        esr: device[0],
        icr_hi: device[1],
        icr_lo: device[2],
        lvt_timer: device[3],
        lvt_thermal: device[4],
        lvt_perfmon: device[5],
        lvt_lint0: device[6],
        lvt_lint1: device[7],
        lvt_error: device[8],
        timer_initial_count: device[9],
        timer_current_count: device[10],
        timer_divide: device[11],
        timer_running: reader.bool()?,
        timer_last_ns: reader.option_u64()?,
        timer_fired: reader.bool()?,
    })
}

struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }

    fn bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value)
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value)
    }

    fn bool(&mut self, value: bool) {
        self.u8(value as u8)
    }

    fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes())
    }

    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes())
    }

    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes())
    }

    fn usize(&mut self, value: usize) {
        self.u64(value as u64)
    }

    fn len(&mut self, value: usize) {
        self.usize(value)
    }

    fn bytes_vec(&mut self, value: &[u8]) {
        self.len(value.len());
        self.bytes(value)
    }

    fn usize_vec(&mut self, value: &[usize]) {
        self.len(value.len());
        for item in value {
            self.usize(*item)
        }
    }

    fn u64_vec(&mut self, value: &[u64]) {
        self.len(value.len());
        for item in value {
            self.u64(*item)
        }
    }

    fn u64_pairs(&mut self, value: &[(u64, u64)]) {
        self.len(value.len());
        for &(a, b) in value {
            self.u64(a);
            self.u64(b)
        }
    }

    fn u64_pairs_u8(&mut self, value: &[(u8, u64)]) {
        self.len(value.len());
        for &(a, b) in value {
            self.u8(a);
            self.u64(b)
        }
    }

    fn u64_usize_pairs(&mut self, value: &[(u64, usize)]) {
        self.len(value.len());
        for &(a, b) in value {
            self.u64(a);
            self.usize(b)
        }
    }

    fn u64_u64_u64_bool(&mut self, value: &[(u64, u64, u64, bool)]) {
        self.len(value.len());
        for &(a, b, c, d) in value {
            self.u64(a);
            self.u64(b);
            self.u64(c);
            self.bool(d)
        }
    }

    fn option_u64(&mut self, value: Option<u64>) {
        match value {
            Some(value) => {
                self.bool(true);
                self.u64(value)
            }
            None => self.bool(false),
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], SnapshotError> {
        let end = self.offset.checked_add(len).ok_or(SnapshotError::InvalidFormat)?;
        if end > self.bytes.len() {
            return Err(SnapshotError::InvalidFormat)
        }
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }

    fn bytes_exact(&mut self, len: usize) -> Result<&'a [u8], SnapshotError> {
        self.take(len)
    }

    fn u8(&mut self) -> Result<u8, SnapshotError> {
        Ok(self.take(1)?[0])
    }

    fn bool(&mut self) -> Result<bool, SnapshotError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(SnapshotError::InvalidFormat),
        }
    }

    fn u16(&mut self) -> Result<u16, SnapshotError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, SnapshotError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, SnapshotError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn usize(&mut self) -> Result<usize, SnapshotError> {
        usize::try_from(self.u64()?).map_err(|_| SnapshotError::InvalidFormat)
    }

    fn count(&mut self) -> Result<usize, SnapshotError> {
        let count = self.usize()?;
        if count > MAX_ITEMS {
            return Err(SnapshotError::InvalidFormat)
        }
        Ok(count)
    }

    fn bytes_vec(&mut self) -> Result<Vec<u8>, SnapshotError> {
        let len = self.count()?;
        Ok(self.take(len)?.to_vec())
    }

    fn usize_vec(&mut self) -> Result<Vec<usize>, SnapshotError> {
        let count = self.count()?;
        (0..count).map(|_| self.usize()).collect()
    }

    fn u64_vec(&mut self) -> Result<Vec<u64>, SnapshotError> {
        let count = self.count()?;
        (0..count).map(|_| self.u64()).collect()
    }

    fn u64_pairs(&mut self) -> Result<Vec<(u64, u64)>, SnapshotError> {
        let count = self.count()?;
        (0..count)
            .map(|_| Ok((self.u64()?, self.u64()?)))
            .collect()
    }

    fn u64_pairs_u8(&mut self) -> Result<Vec<(u8, u64)>, SnapshotError> {
        let count = self.count()?;
        (0..count)
            .map(|_| Ok((self.u8()?, self.u64()?)))
            .collect()
    }

    fn u64_usize_pairs(&mut self) -> Result<Vec<(u64, usize)>, SnapshotError> {
        let count = self.count()?;
        (0..count)
            .map(|_| Ok((self.u64()?, self.usize()?)))
            .collect()
    }

    fn u64_u64_u64_bool(&mut self) -> Result<Vec<(u64, u64, u64, bool)>, SnapshotError> {
        let count = self.count()?;
        (0..count)
            .map(|_| Ok((self.u64()?, self.u64()?, self.u64()?, self.bool()?)))
            .collect()
    }

    fn option_u64(&mut self) -> Result<Option<u64>, SnapshotError> {
        if self.bool()? {
            Ok(Some(self.u64()?))
        } else {
            Ok(None)
        }
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], SnapshotError> {
        self.take(N)?.try_into().map_err(|_| SnapshotError::InvalidFormat)
    }
}
