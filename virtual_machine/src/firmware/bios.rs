//! Legacy BIOS: real-mode entry, POST, and INT 10h/13h/15h/16h/19h services.

use crate::cpu::{CpuMode, CpuState, PrivilegeLevel, SegmentRegister};
use crate::devices::DisplayState;
use crate::firmware::uefi::{UefiContext, UEFI_CALL_VECTOR};
use crate::memory::Mmu;
use crate::replay::SharedReplay;
use std::cell::RefCell;
use std::rc::Rc;
use std::ffi::c_void;

pub const BIOS_ROM_BASE: u64 = 0xF0000;
pub const BIOS_ROM_SIZE: usize = 0x10000;
pub const BIOS_ENTRY_LINEAR: u64 = 0xFE05B;
#[cfg(test)]
const BIOS_ENTRY_OFFSET: usize = 0xE05B;
pub const RESET_VECTOR_LINEAR: u64 = 0xFFFF0;
#[cfg(test)]
const RESET_VECTOR_ROM_OFFSET: usize = 0xFFF0;
pub const MBR_LOAD_ADDR: u64 = 0x7C00;

#[repr(C)]
struct CRegisters {
    rax: u64, rbx: u64, rcx: u64, rdx: u64, rsi: u64, rdi: u64, rflags: u64,
    ds: u16, es: u16,
}
#[repr(C)]
struct CIo {
    read: unsafe extern "C" fn(*mut c_void, u64, *mut u8, usize) -> bool,
    write: unsafe extern "C" fn(*mut c_void, u64, *const u8, usize) -> bool,
    context: *mut c_void,
}
unsafe extern "C" {
    fn ghostos_vm_bios_rom(output: *mut u8);
    fn ghostos_vm_bios_post_tables(ivt: *mut u8, bda: *mut u8);
    fn ghostos_vm_bios_service(vector: u8, registers: *mut CRegisters, image: *const u8,
        image_length: usize, memory_size: u64, io: *const CIo);
}
unsafe extern "C" fn read_memory(raw: *mut c_void, addr: u64, out: *mut u8, len: usize) -> bool {
    let mmu = unsafe { &mut *raw.cast::<Mmu>() };
    match mmu.read_phys(addr, len) {
        Ok(bytes) => {
            if len != 0 { unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, len) }; }
            true
        }
        Err(_) => false,
    }
}
unsafe extern "C" fn write_memory(raw: *mut c_void, addr: u64, data: *const u8, len: usize) -> bool {
    let mmu = unsafe { &mut *raw.cast::<Mmu>() };
    let bytes = if len == 0 { &[] } else { unsafe { std::slice::from_raw_parts(data, len) } };
    mmu.write_phys(addr, bytes).is_ok()
}
fn bios_rom() -> Vec<u8> {
    let mut rom = vec![0; BIOS_ROM_SIZE];
    unsafe { ghostos_vm_bios_rom(rom.as_mut_ptr()) };
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
            0x13 | 0x15 | 0x16 => self.legacy_service(int_num, cpu, mmu),
            0x19 => self.bootstrap_service(cpu, mmu),
            _ => Ok(()),
        }
    }

    fn video_service(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) -> Result<(), BiosError> {
        self.display.borrow_mut().int10(cpu, mmu);
        Ok(())
    }

    fn legacy_service(&mut self, vector: u8, cpu: &mut CpuState, mmu: &mut Mmu) -> Result<(), BiosError> {
        let mut registers = CRegisters { rax: cpu.rax, rbx: cpu.rbx, rcx: cpu.rcx,
            rdx: cpu.rdx, rsi: cpu.rsi, rdi: cpu.rdi, rflags: cpu.rflags,
            ds: cpu.ds.selector, es: cpu.es.selector };
        let (image, length) = self.boot_image.as_ref()
            .map_or((std::ptr::null(), 0), |image| (image.as_ptr(), image.len()));
        let io = CIo { read: read_memory, write: write_memory, context: (mmu as *mut Mmu).cast() };
        unsafe { ghostos_vm_bios_service(vector, &mut registers, image, length, self.memory_size as u64, &io) };
        cpu.rax = registers.rax; cpu.rbx = registers.rbx; cpu.rcx = registers.rcx;
        cpu.rdx = registers.rdx; cpu.rsi = registers.rsi; cpu.rdi = registers.rdi;
        cpu.rflags = registers.rflags;
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
        let mut bda = [0u8; 256];
        unsafe { ghostos_vm_bios_post_tables(ivt.as_mut_ptr(), bda.as_mut_ptr()) };
        let _ = mmu.write_phys(0, &ivt);
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
