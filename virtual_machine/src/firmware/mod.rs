pub mod bios;

pub use bios::{Bios, BiosContext, BiosState, BiosError};

#[derive(Debug)]
pub enum FirmwareError {
    InitFailed,
    InvalidState,
    BootFailed,
    UnknownMode,
}

impl From<BiosError> for FirmwareError {
    fn from(_: BiosError) -> Self {
        FirmwareError::InitFailed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareMode {
    Bios,
    Uefi,
}

pub struct Firmware {
    pub mode: FirmwareMode,
    pub context: BiosContext,
}

impl Firmware {
    pub fn new(mode: FirmwareMode) -> Self {
        Self {
            mode,
            context: BiosContext::new(),
        }
    }

    pub fn init(&mut self) -> Result<(), FirmwareError> {
        match self.mode {
            FirmwareMode::Bios => self.context.init_bios()?,
            FirmwareMode::Uefi => self.context.init_uefi()?,
        }
        Ok(())
    }

    pub fn reset(&mut self) {
        self.context.reset();
    }
}