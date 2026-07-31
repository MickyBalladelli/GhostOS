use crate::cpu::CpuState;
use crate::devices::DisplayState;
use crate::memory::Mmu;
use std::cell::RefCell;
use std::rc::Rc;

pub struct BiosContext {
    pub state: BiosState,
    pub ivt: [u16; 256],
    pub bda: [u8; 256],
    pub ega: [u8; 32 * 4],
    display: Rc<RefCell<DisplayState>>,
}

impl BiosContext {
    pub fn new() -> Self {
        Self {
            state: BiosState::Reset,
            ivt: [0; 256],
            bda: [0; 256],
            ega: [0; 32 * 4],
            display: Rc::new(RefCell::new(DisplayState::new())),
        }
    }

    pub fn set_display(&mut self, display: Rc<RefCell<DisplayState>>) {
        self.display = display;
    }

    pub fn display(&self) -> Rc<RefCell<DisplayState>> {
        self.display.clone()
    }

    pub fn init_bios(&mut self) -> Result<(), BiosError> {
        self.state = BiosState::Initialized;
        Ok(())
    }

    pub fn init_uefi(&mut self) -> Result<(), BiosError> {
        self.state = BiosState::UefiInitialized;
        Ok(())
    }

    pub fn reset(&mut self) {
        self.state = BiosState::Reset;
        self.ivt = [0; 256];
        self.bda = [0; 256];
        self.ega = [0; 32 * 4];
    }

    pub fn call_int(
        &mut self,
        int_num: u8,
        cpu: &mut CpuState,
        mmu: &mut Mmu,
    ) -> Result<(), BiosError> {
        match int_num {
            0x10 => self.video_service(cpu, mmu),
            0x13 => self.disk_service(),
            0x15 => self.system_service(),
            0x16 => self.keyboard_service(),
            _ => Ok(()),
        }
    }

    fn video_service(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) -> Result<(), BiosError> {
        self.display.borrow_mut().int10(cpu, mmu);
        Ok(())
    }

    fn disk_service(&mut self) -> Result<(), BiosError> {
        Ok(())
    }

    fn system_service(&mut self) -> Result<(), BiosError> {
        Ok(())
    }

    fn keyboard_service(&mut self) -> Result<(), BiosError> {
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
            reset_vector: 0xFFFF_0000,
        }
    }

    pub fn init(&mut self, _mmu: &mut Mmu, _cpu: &mut CpuState) -> Result<(), BiosError> {
        self.context.init_bios()?;
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