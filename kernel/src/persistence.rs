use ghostos_boot_protocol::{
    GHOSTOS_PERSISTENCE_COMMAND_PORT, GHOSTOS_PERSISTENCE_DATA_PORT,
    GHOSTOS_PERSISTENCE_FLUSH, GHOSTOS_PERSISTENCE_LENGTH_PORT, GHOSTOS_PERSISTENCE_LOAD,
    GHOSTOS_PERSISTENCE_MAX_BYTES, GHOSTOS_PERSISTENCE_SAVE,
};

const PERSISTENCE_CONTAINER_MAGIC: [u8; 8] = *b"SYNREC01";
const PERSISTENCE_CONTAINER_VERSION: u16 = 1;
const PERSISTENCE_CONTAINER_HEADER_BYTES: usize = 24;
const MAX_CRASH_BYTES: usize = 1024;
const MAX_CONTAINER_BYTES: usize = PERSISTENCE_CONTAINER_HEADER_BYTES
    + crate::boot_diagnostics::MAX_BOOT_DIAGNOSTIC_BYTES
    + MAX_CRASH_BYTES;

struct PersistentRecords {
    boot: [u8; crate::boot_diagnostics::MAX_BOOT_DIAGNOSTIC_BYTES],
    boot_length: usize,
    crash: [u8; MAX_CRASH_BYTES],
    crash_length: usize,
}

impl PersistentRecords {
    const fn empty() -> Self {
        Self {
            boot: [0; crate::boot_diagnostics::MAX_BOOT_DIAGNOSTIC_BYTES],
            boot_length: 0,
            crash: [0; MAX_CRASH_BYTES],
            crash_length: 0,
        }
    }
}

pub struct PersistentStore;

impl PersistentStore {
    pub const fn new() -> Self {
        Self
    }

    #[allow(dead_code)]
    pub fn load(&self, bytes: &mut [u8]) -> Option<usize> {
        if bytes.len() > GHOSTOS_PERSISTENCE_MAX_BYTES {
            return None
        }
        io_out8(GHOSTOS_PERSISTENCE_COMMAND_PORT, GHOSTOS_PERSISTENCE_LOAD);
        let length = io_in32(GHOSTOS_PERSISTENCE_LENGTH_PORT) as usize;
        if length > bytes.len() {
            return None
        }
        for byte in &mut bytes[..length] {
            *byte = io_in8(GHOSTOS_PERSISTENCE_DATA_PORT);
        }
        Some(length)
    }

    pub fn save(&self, bytes: &[u8]) {
        if bytes.len() > GHOSTOS_PERSISTENCE_MAX_BYTES {
            return
        }
        io_out8(GHOSTOS_PERSISTENCE_COMMAND_PORT, GHOSTOS_PERSISTENCE_SAVE);
        io_out32(GHOSTOS_PERSISTENCE_LENGTH_PORT, bytes.len() as u32);
        for byte in bytes {
            io_out8(GHOSTOS_PERSISTENCE_DATA_PORT, *byte);
        }
        io_out8(GHOSTOS_PERSISTENCE_COMMAND_PORT, GHOSTOS_PERSISTENCE_FLUSH);
    }

    pub fn load_boot_diagnostic(&self, bytes: &mut [u8]) -> Option<usize> {
        let records = self.load_records();
        if records.boot_length == 0 || records.boot_length > bytes.len() {
            return None
        }
        bytes[..records.boot_length].copy_from_slice(&records.boot[..records.boot_length]);
        Some(records.boot_length)
    }

    pub fn save_boot_diagnostic(&self, bytes: &[u8]) {
        if bytes.len() > crate::boot_diagnostics::MAX_BOOT_DIAGNOSTIC_BYTES {
            return
        }
        let mut records = self.load_records();
        records.boot[..bytes.len()].copy_from_slice(bytes);
        records.boot[bytes.len()..].fill(0);
        records.boot_length = bytes.len();
        self.save_records(&records)
    }

    pub fn save_crash_capsule(&self, bytes: &[u8]) {
        if bytes.len() > MAX_CRASH_BYTES {
            return
        }
        let mut records = self.load_records();
        records.crash[..bytes.len()].copy_from_slice(bytes);
        records.crash[bytes.len()..].fill(0);
        records.crash_length = bytes.len();
        self.save_records(&records)
    }

    fn load_records(&self) -> PersistentRecords {
        let mut stored = [0; MAX_CONTAINER_BYTES];
        let Some(length) = self.load(&mut stored) else {
            return PersistentRecords::empty()
        };
        if length >= PERSISTENCE_CONTAINER_HEADER_BYTES
            && stored[..8] == PERSISTENCE_CONTAINER_MAGIC
            && u16::from_le_bytes([stored[8], stored[9]]) == PERSISTENCE_CONTAINER_VERSION
            && u16::from_le_bytes([stored[10], stored[11]])
                == PERSISTENCE_CONTAINER_HEADER_BYTES as u16
        {
            let boot_length = u32::from_le_bytes(stored[12..16].try_into().unwrap()) as usize;
            let crash_length = u32::from_le_bytes(stored[16..20].try_into().unwrap()) as usize;
            let payload_length = boot_length.saturating_add(crash_length);
            let total_length = PERSISTENCE_CONTAINER_HEADER_BYTES.saturating_add(payload_length);
            if boot_length <= crate::boot_diagnostics::MAX_BOOT_DIAGNOSTIC_BYTES
                && crash_length <= MAX_CRASH_BYTES
                && total_length == length
                && u32::from_le_bytes(stored[20..24].try_into().unwrap())
                    == persistence_checksum(&stored[PERSISTENCE_CONTAINER_HEADER_BYTES..total_length])
            {
                let mut records = PersistentRecords::empty();
                records.boot_length = boot_length;
                records.crash_length = crash_length;
                records.boot[..boot_length]
                    .copy_from_slice(&stored[PERSISTENCE_CONTAINER_HEADER_BYTES..PERSISTENCE_CONTAINER_HEADER_BYTES + boot_length]);
                records.crash[..crash_length].copy_from_slice(
                    &stored[PERSISTENCE_CONTAINER_HEADER_BYTES + boot_length..total_length],
                );
                return records
            }
        }

        let mut records = PersistentRecords::empty();
        if length >= 8 && stored[..8] == *b"SYNCRSH1" {
            records.crash_length = length.min(MAX_CRASH_BYTES);
            records.crash[..records.crash_length].copy_from_slice(&stored[..records.crash_length]);
        } else if length >= 8 && stored[..8] == crate::boot_diagnostics::BOOT_DIAGNOSTIC_MAGIC {
            records.boot_length = length.min(crate::boot_diagnostics::MAX_BOOT_DIAGNOSTIC_BYTES);
            records.boot[..records.boot_length].copy_from_slice(&stored[..records.boot_length]);
        }
        records
    }

    fn save_records(&self, records: &PersistentRecords) {
        let total_length = PERSISTENCE_CONTAINER_HEADER_BYTES
            .saturating_add(records.boot_length)
            .saturating_add(records.crash_length);
        if total_length > MAX_CONTAINER_BYTES {
            return
        }
        let mut stored = [0; MAX_CONTAINER_BYTES];
        stored[..8].copy_from_slice(&PERSISTENCE_CONTAINER_MAGIC);
        stored[8..10].copy_from_slice(&PERSISTENCE_CONTAINER_VERSION.to_le_bytes());
        stored[10..12].copy_from_slice(&(PERSISTENCE_CONTAINER_HEADER_BYTES as u16).to_le_bytes());
        stored[12..16].copy_from_slice(&(records.boot_length as u32).to_le_bytes());
        stored[16..20].copy_from_slice(&(records.crash_length as u32).to_le_bytes());
        let payload_end = PERSISTENCE_CONTAINER_HEADER_BYTES + records.boot_length;
        stored[PERSISTENCE_CONTAINER_HEADER_BYTES..payload_end]
            .copy_from_slice(&records.boot[..records.boot_length]);
        stored[payload_end..total_length]
            .copy_from_slice(&records.crash[..records.crash_length]);
        let checksum = persistence_checksum(&stored[PERSISTENCE_CONTAINER_HEADER_BYTES..total_length]);
        stored[20..24].copy_from_slice(&checksum.to_le_bytes());
        self.save(&stored[..total_length])
    }
}

fn persistence_checksum(bytes: &[u8]) -> u32 {
    let mut checksum = 0x811c_9dc5_u32;
    for byte in bytes {
        checksum ^= u32::from(*byte);
        checksum = checksum.wrapping_mul(16_777_619);
    }
    checksum
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
#[allow(dead_code)]
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
#[allow(dead_code)]
fn io_in32(_port: u16) -> u32 {
    0
}

#[cfg(all(
    target_arch = "x86_64",
    any(target_os = "none", target_os = "uefi")
))]
#[allow(dead_code)]
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
#[allow(dead_code)]
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
