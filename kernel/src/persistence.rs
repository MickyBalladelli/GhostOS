use synos_boot_protocol::{
    SYNOS_PERSISTENCE_COMMAND_PORT, SYNOS_PERSISTENCE_DATA_PORT,
    SYNOS_PERSISTENCE_FLUSH, SYNOS_PERSISTENCE_LENGTH_PORT, SYNOS_PERSISTENCE_LOAD,
    SYNOS_PERSISTENCE_MAX_BYTES, SYNOS_PERSISTENCE_SAVE,
};

pub struct PersistentStore;

impl PersistentStore {
    pub const fn new() -> Self {
        Self
    }

    pub fn load(&self, bytes: &mut [u8]) -> Option<usize> {
        if bytes.len() > SYNOS_PERSISTENCE_MAX_BYTES {
            return None
        }
        io_out8(SYNOS_PERSISTENCE_COMMAND_PORT, SYNOS_PERSISTENCE_LOAD);
        let length = io_in32(SYNOS_PERSISTENCE_LENGTH_PORT) as usize;
        if length > bytes.len() {
            return None
        }
        for byte in &mut bytes[..length] {
            *byte = io_in8(SYNOS_PERSISTENCE_DATA_PORT);
        }
        Some(length)
    }

    pub fn save(&self, bytes: &[u8]) {
        if bytes.len() > SYNOS_PERSISTENCE_MAX_BYTES {
            return
        }
        io_out8(SYNOS_PERSISTENCE_COMMAND_PORT, SYNOS_PERSISTENCE_SAVE);
        io_out32(SYNOS_PERSISTENCE_LENGTH_PORT, bytes.len() as u32);
        for byte in bytes {
            io_out8(SYNOS_PERSISTENCE_DATA_PORT, *byte);
        }
        io_out8(SYNOS_PERSISTENCE_COMMAND_PORT, SYNOS_PERSISTENCE_FLUSH);
    }
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn io_out8(port: u16, value: u8) {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") value,
            options(nomem, nostack, preserves_flags),
        )
    }
}

#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
)))]
fn io_out8(_port: u16, _value: u8) {}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn io_in32(port: u16) -> u32 {
    let value: u32;
    unsafe {
        core::arch::asm!(
            "in eax, dx",
            in("dx") port,
            out("eax") value,
            options(nomem, nostack, preserves_flags),
        )
    }
    value
}

#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
)))]
fn io_in32(_port: u16) -> u32 {
    0
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn io_in8(port: u16) -> u8 {
    let value: u8;
    unsafe {
        core::arch::asm!(
            "in al, dx",
            in("dx") port,
            out("al") value,
            options(nomem, nostack, preserves_flags),
        )
    }
    value
}

#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
)))]
fn io_in8(_port: u16) -> u8 {
    0
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
fn io_out32(port: u16, value: u32) {
    unsafe {
        core::arch::asm!(
            "out dx, eax",
            in("dx") port,
            in("eax") value,
            options(nomem, nostack, preserves_flags),
        )
    }
}

#[cfg(not(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
)))]
fn io_out32(_port: u16, _value: u32) {}
