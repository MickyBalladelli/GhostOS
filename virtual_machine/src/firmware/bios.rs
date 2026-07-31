use crate::cpu::Cpu;
use crate::memory::Mmu;

pub struct BiosContext {
    pub state: BiosState,
    pub ivt: [u16; 256],
    pub bda: [u8; 256],
    pub ega: [u8; 32 * 4],
}

impl BiosContext {
    pub fn new() -> Self {
        Self {
            state: BiosState::Reset,
            ivt: [0; 256],
            bda: [0; 256],
            ega: [0; 32 * 4],
        }
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

    pub fn call_int(&mut self, int_num: u8, _cpu: &mut Cpu, _mmu: &mut Mmu) -> Result<(), BiosError> {
        match int_num {
            0x10 => self.video_service(),
            0x13 => self.disk_service(),
            0x15 => self.system_service(),
            0x16 => self.keyboard_service(),
            _ => Ok(()),
        }
    }

    fn video_service(&mut self) -> Result<(), BiosError> {
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

    pub fn init(&mut self, _mmu: &mut Mmu, _cpu: &mut Cpu) -> Result<(), BiosError> {
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