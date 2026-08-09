//! Legacy BIOS: real-mode entry, POST, and INT 10h/13h/15h/16h/19h services.

use crate::cpu::{CpuMode, CpuState, PrivilegeLevel, SegmentRegister};
use crate::devices::DisplayState;
use crate::firmware::uefi::{UefiContext, UEFI_CALL_VECTOR};
use crate::memory::Mmu;
use crate::replay::SharedReplay;
use std::cell::RefCell;
use std::rc::Rc;

pub const BIOS_ROM_BASE: u64 = 0xF0000;
pub const BIOS_ROM_SIZE: usize = 0x10000;
pub const BIOS_ENTRY_LINEAR: u64 = 0xFE05B;
const BIOS_ENTRY_OFFSET: usize = 0xE05B;
pub const RESET_VECTOR_LINEAR: u64 = 0xFFFF0;
const RESET_VECTOR_ROM_OFFSET: usize = 0xFFF0;
pub const MBR_LOAD_ADDR: u64 = 0x7C00;
const CHS_HEADS: u64 = 16;
const CHS_SECTORS: u64 = 63;

fn bios_rom() -> Vec<u8> {
    let mut rom = vec![0u8; BIOS_ROM_SIZE];
    let stub: [u8; 13] = [
        0xFA, 0x66, 0x31, 0xC0, 0x8E, 0xD8, 0x8E, 0xC0, 0x8E, 0xD0, 0x66, 0xBC, 0x00,
    ];
    rom[BIOS_ENTRY_OFFSET] = 0xFA;
    // mov sp, 0x7C00 requires 4 bytes: 66 BC 00 7C
    rom[BIOS_ENTRY_OFFSET..BIOS_ENTRY_OFFSET + 12]
        .copy_from_slice(&[0xFA, 0x66, 0x31, 0xC0, 0x8E, 0xD8, 0x8E, 0xC0, 0x8E, 0xD0, 0x66, 0xBC]);
    rom[BIOS_ENTRY_OFFSET + 12] = 0x00;
    rom[BIOS_ENTRY_OFFSET + 13] = 0x7C;
    rom[BIOS_ENTRY_OFFSET + 14] = 0xFB; // sti
    rom[BIOS_ENTRY_OFFSET + 15] = 0xCD; // int 0x19
    rom[BIOS_ENTRY_OFFSET + 16] = 0x19;
    rom[BIOS_ENTRY_OFFSET + 17] = 0xF4; // hlt
    let _ = stub;
    rom[RESET_VECTOR_ROM_OFFSET..RESET_VECTOR_ROM_OFFSET + 5]
        .copy_from_slice(&[0xEA, 0x5B, 0xE0, 0x00, 0xF0]);
    rom[0xFFFE] = 0x55;
    rom[0xFFFF] = 0xAA;
    rom
}

pub struct BiosContext {
    pub state: BiosState,
    pub ivt: [u16; 256],
    pub bda: [u8; 256],
    pub ega: [u8; 32 * 4],
    display: Rc<RefCell<DisplayState>>,
    boot_image: Option<Vec<u8>>,
    memory_size: usize,
    /// UEFI firmware context, shared with the executor through `call_int`
    /// so vector 0xE0 (UEFI service dispatch) reaches the boot services.
    pub uefi: Option<UefiContext>,
    replay: Option<SharedReplay>,
    replay_instruction_ip: Option<u64>,
}

impl BiosContext {
    pub fn new() -> Self {
        Self {
            state: BiosState::Reset,
            ivt: [0; 256],
            bda: [0; 256],
            ega: [0; 32 * 4],
            display: Rc::new(RefCell::new(DisplayState::new())),
            boot_image: None,
            memory_size: 128 * 1024 * 1024,
            uefi: None,
            replay: None,
            replay_instruction_ip: None,
        }
    }

    pub fn attach_replay(&mut self, replay: SharedReplay) {
        self.replay = Some(replay.clone());
        if let Some(uefi) = &mut self.uefi {
            uefi.attach_replay(replay);
        }
    }

    pub fn set_replay_instruction_ip(&mut self, instruction_ip: Option<u64>) {
        self.replay_instruction_ip = instruction_ip;
        if let Some(uefi) = &mut self.uefi {
            uefi.set_replay_instruction_ip(instruction_ip);
        }
    }

    pub fn set_display(&mut self, display: Rc<RefCell<DisplayState>>) {
        self.display = display.clone();
        if let Some(uefi) = &mut self.uefi {
            uefi.set_display(display);
        }
    }

    pub fn display(&self) -> Rc<RefCell<DisplayState>> {
        self.display.clone()
    }

    pub fn set_boot_image(&mut self, image: Vec<u8>) {
        self.boot_image = Some(image);
    }

    pub fn set_memory_size(&mut self, size: usize) {
        self.memory_size = size;
        if let Some(uefi) = &mut self.uefi {
            uefi.set_memory_size(size);
        }
    }

    pub fn memory_size(&self) -> usize {
        self.memory_size
    }

    pub fn init_bios(&mut self) -> Result<(), BiosError> {
        self.state = BiosState::Initialized;
        Ok(())
    }

    pub fn init_uefi(&mut self) -> Result<(), BiosError> {
        if self.uefi.is_none() {
            let mut uefi = UefiContext::new();
            uefi.set_display(self.display.clone());
            uefi.set_memory_size(self.memory_size);
            if let Some(replay) = &self.replay {
                uefi.attach_replay(replay.clone());
            }
            self.uefi = Some(uefi);
        }
        self.state = BiosState::UefiInitialized;
        Ok(())
    }

    pub fn reset(&mut self) {
        self.state = BiosState::Reset;
        self.ivt = [0; 256];
        self.bda = [0; 256];
        self.ega = [0; 32 * 4];
        if let Some(uefi) = &mut self.uefi {
            uefi.reset();
        }
        self.replay_instruction_ip = None;
    }

    pub fn call_int(
        &mut self,
        int_num: u8,
        cpu: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), BiosError> {
        // UEFI service dispatch: the firmware emits `int 0xE0` inside each
        // boot/runtime service stub (mov eax,id; int 0xE0; ret). Route it to
        // the UEFI context when UEFI firmware is active, otherwise ignore.
        if int_num == UEFI_CALL_VECTOR {
            if let Some(uefi) = &mut self.uefi {
                uefi.dispatch(cpu, mmu);
            }
            return Ok(());
        }
        match int_num {
            0x10 => self.video_service(cpu, mmu),
            0x13 => self.disk_service(cpu, mmu),
            0x15 => self.system_service(cpu, mmu),
            0x16 => self.keyboard_service(cpu),
            0x19 => self.bootstrap_service(cpu, mmu),
            _ => Ok(()),
        }
    }

    fn video_service(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) -> Result<(), BiosError> {
        self.display.borrow_mut().int10(cpu, mmu);
        Ok(())
    }

    // INT 13h — disk
    fn disk_service(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) -> Result<(), BiosError> {
        clear_cf(cpu);
        let ah = ((cpu.rax >> 8) & 0xFF) as u8;
        match ah {
            0x00 | 0x01 | 0x0E => set_ah(cpu, 0),
            0x02 => self.int13_read_chs(cpu, mmu),
            0x08 => {
                set_ah(cpu, 0);
                cpu.rdx = (cpu.rdx & !0xFF00) | (((CHS_HEADS - 1) as u64) << 8);
                cpu.rcx = (cpu.rcx & !0xFFFF) | (CHS_SECTORS - 1) as u64;
                cpu.rdx = (cpu.rdx & !0xFF) | 0x01;
            }
            0x15 => {
                set_ah(cpu, 3);
                let sectors = self
                    .boot_image
                    .as_ref()
                    .map(|i| (i.len() / 512) as u64)
                    .unwrap_or(0);
                cpu.rcx = (cpu.rcx & !0xFFFF) | (sectors & 0xFFFF);
                cpu.rdx = (cpu.rdx & !0xFFFF) | ((sectors >> 16) & 0xFFFF);
            }
            0x41 => self.int13_edd_check(cpu),
            0x42 => self.int13_edd_read(cpu, mmu),
            _ => {
                set_cf(cpu);
                set_ah(cpu, 0x01);
            }
        }
        Ok(())
    }

    fn int13_read_chs(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let Some(image) = &self.boot_image else {
            set_cf(cpu);
            set_ah(cpu, 0x80);
            return;
        };
        let count = (cpu.rax & 0xFF) as u64;
        let ch = ((cpu.rcx >> 8) & 0xFF) as u64;
        let cl = (cpu.rcx & 0xFF) as u64;
        let dh = ((cpu.rdx >> 8) & 0xFF) as u64;
        let cyl = ch | ((cl & 0xC0) << 2);
        let sect = cl & 0x3F;
        let lba = if sect == 0 {
            (cyl * CHS_HEADS + dh) * CHS_SECTORS
        } else {
            (cyl * CHS_HEADS + dh) * CHS_SECTORS + (sect - 1)
        };
        let available = (image.len() / 512) as u64;
        let to_read = count.min(available.saturating_sub(lba));
        if to_read == 0 {
            set_cf(cpu);
            set_ah(cpu, 0x02);
            return;
        }
        let offset = (lba * 512) as usize;
        let bytes = &image[offset..offset + (to_read as usize) * 512];
        let dest = ((cpu.es.selector as u64) << 4) + (cpu.rbx & 0xFFFF);
        let _ = mmu.write_phys(dest, bytes);
        clear_cf(cpu);
        set_ah(cpu, 0);
    }

    fn int13_edd_check(&self, cpu: &mut CpuState) {
        let dl = (cpu.rdx & 0xFF) as u8;
        if dl & 0x80 == 0 {
            set_cf(cpu);
            set_ah(cpu, 0x01);
            return;
        }
        clear_cf(cpu);
        set_ah(cpu, 0x30);
        cpu.rbx = (cpu.rbx & !0xFFFF) | 0xAA55;
        cpu.rcx = (cpu.rcx & !0xFFFF) | 0x0003;
    }

    fn int13_edd_read(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let dap_addr = ((cpu.ds.selector as u64) << 4) + (cpu.rsi & 0xFFFF);
        let Some(image) = &self.boot_image else {
            set_cf(cpu);
            set_ah(cpu, 0x80);
            return;
        };
        let header = mmu.read_phys(dap_addr, 16).unwrap_or_default();
        if header[0] < 0x10 {
            set_cf(cpu);
            set_ah(cpu, 0x01);
            return;
        }
        let count = header[2] as u64;
        let seg = u16::from_le_bytes([header[4], header[5]]);
        let off = u16::from_le_bytes([header[6], header[7]]);
        let lba = u64::from_le_bytes(header[8..16].try_into().unwrap());
        let dest = ((seg as u64) << 4) + off as u64;
        let available = (image.len() / 512) as u64;
        let to_read = count.min(available.saturating_sub(lba));
        if to_read == 0 {
            set_cf(cpu);
            set_ah(cpu, 0x02);
            return;
        }
        let offset = (lba * 512) as usize;
        let bytes = &image[offset..offset + (to_read as usize) * 512];
        let _ = mmu.write_phys(dest, bytes);
        clear_cf(cpu);
        set_ah(cpu, 0);
    }

    // INT 15h — system services
    fn system_service(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) -> Result<(), BiosError> {
        clear_cf(cpu);
        let ah = ((cpu.rax >> 8) & 0xFF) as u8;
        match ah {
            0x20 | 0x86 | 0xC0 => set_ah(cpu, 0),
            0x88 => {
                let above = self.memory_size.saturating_sub(0x100000);
                let kb = (above / 1024).min(0xFFFF) as u64;
                cpu.rax = (cpu.rax & !0xFFFF) | (kb & 0xFFFF);
                clear_cf(cpu);
            }
            0xE8 | 0xE9 => self.int15_e820(cpu, mmu),
            _ => {
                set_cf(cpu);
                set_ah(cpu, 0x86);
            }
        }
        Ok(())
    }

    fn int15_e820(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        if cpu.rdx & 0xFFFF_FFFF != 0x534D_4150 {
            set_cf(cpu);
            set_ah(cpu, 0x86);
            return;
        }
        let dest = ((cpu.es.selector as u64) << 4) + (cpu.rdi & 0xFFFF);
        let ecx = (cpu.rcx & 0xFFFF_FFFF) as usize;
        if ecx < 20 {
            set_cf(cpu);
            set_ah(cpu, 0x86);
            return;
        }
        let ebx = (cpu.rbx & 0xFFFF_FFFF) as usize;
        let mem = self.memory_size as u64;
        let entries: [(u64, u64, u32); 3] = [
            (0, mem.min(0xE0000), 1),
            (0xE0000, 0x20000, 2),
            (0x100000, mem.saturating_sub(0x100000), 1),
        ];
        if ebx >= entries.len() {
            cpu.rbx = 0;
            set_ah(cpu, 0);
            return;
        }
        let (base, len, ty) = entries[ebx];
        let mut buf = [0u8; 24];
        buf[0..8].copy_from_slice(&base.to_le_bytes());
        buf[8..16].copy_from_slice(&len.to_le_bytes());
        buf[16..20].copy_from_slice(&ty.to_le_bytes());
        buf[20..24].copy_from_slice(&1u32.to_le_bytes());
        let _ = mmu.write_phys(dest, &buf[..ecx.min(24)]);
        cpu.rbx = ((ebx + 1) as u64) & 0xFFFF_FFFF;
        set_ah(cpu, 0);
        clear_cf(cpu);
    }

    // INT 16h — keyboard
    fn keyboard_service(&mut self, cpu: &mut CpuState) -> Result<(), BiosError> {
        match ((cpu.rax >> 8) & 0xFF) as u8 {
            0x00 => {
                cpu.rax &= !0xFFFF;
                cpu.rflags |= 1 << 6; // ZF
            }
            0x01 => cpu.rflags |= 1 << 6,
            _ => cpu.rax &= !0xFF,
        }
        Ok(())
    }

    // INT 19h — MBR boot
    fn bootstrap_service(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) -> Result<(), BiosError> {
        let Some(image) = &self.boot_image else {
            cpu.halted = true;
            return Ok(());
        };
        if image.len() < 512 {
            cpu.halted = true;
            return Ok(());
        }
        let _ = mmu.write_phys(MBR_LOAD_ADDR, &image[..512]);
        let seg = SegmentRegister {
            selector: 0,
            base: 0,
            limit: 0xFFFF,
            attributes: 0x93,
        };
        cpu.cs = seg;
        cpu.ds = seg;
        cpu.es = seg;
        cpu.ss = seg;
        cpu.mode = CpuMode::Real16;
        cpu.privilege = PrivilegeLevel::Ring0;
        cpu.rip = MBR_LOAD_ADDR;
        cpu.rsp = MBR_LOAD_ADDR;
        cpu.rflags = (cpu.rflags | (1 << 9)) | 0x2;
        mmu.set_privilege(false);
        Ok(())
    }
}

impl Default for BiosContext {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BiosState {
    Reset,
    Initialized,
    UefiInitialized,
    Running,
    Halted,
}

impl Default for BiosState {
    fn default() -> Self {
        BiosState::Reset
    }
}

#[derive(Debug)]
pub enum BiosError {
    InitFailed,
    InvalidCall,
    UnknownInterrupt,
    NotImplemented,
}

fn clear_cf(cpu: &mut CpuState) {
    cpu.rflags &= !1;
}

fn set_cf(cpu: &mut CpuState) {
    cpu.rflags |= 1;
}

fn set_ah(cpu: &mut CpuState, v: u8) {
    cpu.rax = (cpu.rax & !0xFF00) | ((v as u64) << 8);
}

pub struct Bios {
    pub context: BiosContext,
    pub reset_vector: u64,
}

impl Bios {
    pub fn new() -> Self {
        Self {
            context: BiosContext::new(),
            reset_vector: RESET_VECTOR_LINEAR,
        }
    }

    pub fn set_boot_image(&mut self, image: Vec<u8>) {
        self.context.set_boot_image(image);
    }

    pub fn set_memory_size(&mut self, size: usize) {
        self.context.set_memory_size(size);
    }

    pub fn init(&mut self, mmu: &mut Mmu, cpu: &mut CpuState) -> Result<(), BiosError> {
        self.post(mmu, cpu)?;
        let seg = SegmentRegister {
            selector: 0xF000,
            base: 0,
            limit: 0xFFFF,
            attributes: 0x9B,
        };
        cpu.cs = seg;
        cpu.mode = CpuMode::Real16;
        cpu.privilege = PrivilegeLevel::Ring0;
        cpu.rip = BIOS_ENTRY_LINEAR;
        cpu.rsp = MBR_LOAD_ADDR;
        cpu.rflags = (cpu.rflags | (1 << 9)) | 0x2;
        mmu.set_privilege(false);
        self.context.state = BiosState::Running;
        Ok(())
    }

    pub fn post(&mut self, mmu: &mut Mmu, cpu: &mut CpuState) -> Result<(), BiosError> {
        let rom = bios_rom();
        let _ = mmu.write_phys(BIOS_ROM_BASE, &rom);
        let mut ivt = [0u8; 0x400];
        for slot in (0..0x400usize).step_by(4) {
            ivt[slot] = 0x53;
            ivt[slot + 1] = 0xFF;
            ivt[slot + 2] = 0x00;
            ivt[slot + 3] = 0xF0;
        }
        let _ = mmu.write_phys(0, &ivt);
        let mut bda = [0u8; 256];
        bda[0x80] = 0x03;
        bda[0x81] = 0xF8;
        bda[0x10] = 0x21;
        bda[0x13] = 0x80;
        bda[0x14] = 0x02;
        bda[0x49] = 0x03;
        bda[0x4A] = 80;
        bda[0x4B] = 0;
        bda[0x4C] = 0xA0;
        bda[0x4D] = 0x0F;
        let _ = mmu.write_phys(0x0400, &bda);
        cpu.rax = 0x0003;
        self.context.video_service(cpu, mmu)?;
        self.context.state = BiosState::Initialized;
        Ok(())
    }

    pub fn reset(&mut self) {
        self.context.reset();
    }
}

impl Default for Bios {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_mmu() -> Mmu {
        Mmu::new(8 * 1024 * 1024)
    }

    #[test]
    fn rom_has_signature_and_reset_vector() {
        let rom = bios_rom();
        assert_eq!(rom[RESET_VECTOR_ROM_OFFSET], 0xEA);
        assert_eq!(rom[0xFFFE], 0x55);
        assert_eq!(rom[0xFFFF], 0xAA);
        assert_eq!(rom[BIOS_ENTRY_OFFSET], 0xFA);
    }

    #[test]
    fn post_installs_rom_ivt_and_bda() {
        let mut bios = Bios::new();
        let mut mmu = mem_mmu();
        let mut cpu = CpuState::default();
        bios.post(&mut mmu, &mut cpu).unwrap();
        assert_eq!(mmu.read_phys(0xFFFFE, 2).unwrap(), [0x55, 0xAA]);
        assert_eq!(mmu.read_phys(0x00, 4).unwrap(), [0x53, 0xFF, 0x00, 0xF0]);
        assert_eq!(mmu.read_phys(0x0410, 1).unwrap(), [0x21]);
    }

    #[test]
    fn init_places_cpu_at_rom_entry() {
        let mut bios = Bios::new();
        let mut mmu = mem_mmu();
        let mut cpu = CpuState::default();
        bios.init(&mut mmu, &mut cpu).unwrap();
        assert_eq!(cpu.mode, CpuMode::Real16);
        assert_eq!(cpu.rip, BIOS_ENTRY_LINEAR);
    }

    #[test]
    fn int13_reads_chs_sector() {
        let mut ctx = BiosContext::new();
        let mut img = Vec::new();
        for s in 0..4 {
            img.extend_from_slice(&[s as u8; 512]);
        }
        let boot = img.clone();
        ctx.set_boot_image(img);
        let mut mmu = mem_mmu();
        let mut cpu = CpuState::default();
        cpu.rax = 0x0201;
        cpu.es.selector = 0x1000;
        cpu.rbx = 0;
        cpu.rcx = 0x0002;
        cpu.rdx = 0;
        ctx.call_int(0x13, &mut cpu, &mut mmu).unwrap();
        assert_eq!(cpu.rflags & 1, 0);
        assert_eq!(mmu.read_phys(0x10000, 512).unwrap(), boot[512..1024]);
    }

    #[test]
    fn int15_e820_returns_memory_map() {
        let mut ctx = BiosContext::new();
        ctx.set_memory_size(8 * 1024 * 1024);
        let mut mmu = mem_mmu();
        let mut cpu = CpuState::default();
        let mut bases = Vec::new();
        let mut ebx = 0usize;
        loop {
            cpu.rax = 0xE820;
            cpu.rdx = 0x534D_4150;
            cpu.es.selector = 0x1000;
            cpu.rdi = 0x0200;
            cpu.rcx = 24;
            cpu.rbx = ebx as u64;
            ctx.call_int(0x15, &mut cpu, &mut mmu).unwrap();
            ebx = (cpu.rbx & 0xFFFF_FFFF) as usize;
            if ebx == 0 {
                break;
            }
            let entry = mmu.read_phys(0x10200, 24).unwrap();
            let base = u64::from_le_bytes(entry[0..8].try_into().unwrap());
            bases.push(base);
        }
        assert_eq!(bases, [0x0000_0000, 0x000E_0000, 0x0010_0000]);
    }

    #[test]
    fn int19_loads_mbr_to_7c00() {
        let mut ctx = BiosContext::new();
        let img = vec![0xAAu8; 512];
        let boot = img.clone();
        ctx.set_boot_image(img);
        let mut mmu = mem_mmu();
        let mut cpu = CpuState::default();
        ctx.call_int(0x19, &mut cpu, &mut mmu).unwrap();
        assert_eq!(cpu.rip, MBR_LOAD_ADDR);
        assert_eq!(cpu.mode, CpuMode::Real16);
        assert_eq!(mmu.read_phys(MBR_LOAD_ADDR, 512).unwrap(), boot);
    }
}
