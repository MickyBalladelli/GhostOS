pub mod bios;
pub mod uefi;

pub use bios::{
    Bios, BiosContext, BiosError, BiosState, BIOS_ENTRY_LINEAR, BIOS_ROM_BASE, BIOS_ROM_SIZE,
    MBR_LOAD_ADDR, RESET_VECTOR_LINEAR,
};
pub use uefi::{
    UefiContext, UefiError, UefiState, UEFI_CALL_VECTOR, UEFI_CHILD_IMAGE_BASE, UEFI_IMAGE_BASE,
    UEFI_MEMORY_MAP_BASE, UEFI_MEMORY_MAP_SIZE, UEFI_STACK_TOP, UEFI_TABLES_BASE,
};

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
        let mut context = BiosContext::new();
        if mode == FirmwareMode::Uefi {
            let mut uefi = UefiContext::new();
            uefi.set_display(context.display());
            uefi.set_memory_size(context.memory_size());
            context.uefi = Some(uefi);
        }
        Self {
            mode,
            context,
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
