use ghostos_boot_protocol::GHOSTOS_PERSISTENCE_MAX_BYTES;

unsafe extern "C" {
    fn ghostos_persistence_load(bytes: *mut u8, capacity: usize, length: *mut usize) -> bool;
    fn ghostos_persistence_save(bytes: *const u8, length: usize);
}

const PERSISTENCE_CONTAINER_HEADER_BYTES: usize = 24;
const MAX_CRASH_BYTES: usize = 1024;
const MAX_CONTAINER_BYTES: usize = PERSISTENCE_CONTAINER_HEADER_BYTES
    + crate::boot_diagnostics::MAX_BOOT_DIAGNOSTIC_BYTES
    + MAX_CRASH_BYTES;

#[repr(C)]
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
        let mut length = 0;
        // SAFETY: C reads at most `bytes.len()` bytes and writes the returned length.
        if !unsafe { ghostos_persistence_load(bytes.as_mut_ptr(), bytes.len(), &mut length) } {
            return None
        }
        Some(length)
    }

    pub fn save(&self, bytes: &[u8]) {
        if bytes.len() > GHOSTOS_PERSISTENCE_MAX_BYTES {
            return
        }
        // SAFETY: C reads exactly the provided byte slice and checks the protocol limit.
        unsafe { ghostos_persistence_save(bytes.as_ptr(), bytes.len()) };
    }

    pub fn load_boot_diagnostic(&self, bytes: &mut [u8]) -> Option<usize> {
        let records = self.load_records();
        let mut length = 0;
        unsafe { ghostos_persistence_records_boot(&records, bytes.as_mut_ptr(), bytes.len(), &mut length) }
            .then_some(length)
    }

    pub fn save_boot_diagnostic(&self, bytes: &[u8]) {
        if bytes.len() > crate::boot_diagnostics::MAX_BOOT_DIAGNOSTIC_BYTES {
            return
        }
        let mut records = self.load_records();
        unsafe { ghostos_persistence_records_update(&mut records, false, bytes.as_ptr(), bytes.len()) };
        self.save_records(&records)
    }

    pub fn save_crash_capsule(&self, bytes: &[u8]) {
        if bytes.len() > MAX_CRASH_BYTES {
            return
        }
        let mut records = self.load_records();
        unsafe { ghostos_persistence_records_update(&mut records, true, bytes.as_ptr(), bytes.len()) };
        self.save_records(&records)
    }

    fn load_records(&self) -> PersistentRecords {
        let mut stored = [0; MAX_CONTAINER_BYTES];
        let Some(length) = self.load(&mut stored) else {
            return PersistentRecords::empty()
        };
        let mut records = PersistentRecords::empty();
        unsafe { ghostos_persistence_records_decode(stored.as_ptr(), length, &mut records) };
        records
    }

    fn save_records(&self, records: &PersistentRecords) {
        let mut stored = [0; MAX_CONTAINER_BYTES];
        let mut length = 0;
        if unsafe {
            ghostos_persistence_records_encode(records, stored.as_mut_ptr(), stored.len(), &mut length)
        } {
            self.save(&stored[..length])
        }
    }
}

const _: () = {
    assert!(crate::boot_diagnostics::MAX_BOOT_DIAGNOSTIC_BYTES == 56);
    assert!(core::mem::size_of::<PersistentRecords>() == 1096);
    assert!(core::mem::offset_of!(PersistentRecords, crash) == 64);
    assert!(core::mem::offset_of!(PersistentRecords, crash_length) == 1088);
};

unsafe extern "C" {
    fn ghostos_persistence_records_decode(bytes: *const u8, length: usize, records: *mut PersistentRecords);
    fn ghostos_persistence_records_encode(records: *const PersistentRecords, bytes: *mut u8,
        capacity: usize, length: *mut usize) -> bool;
    fn ghostos_persistence_records_update(records: *mut PersistentRecords, crash: bool,
        bytes: *const u8, length: usize) -> bool;
    fn ghostos_persistence_records_boot(records: *const PersistentRecords, bytes: *mut u8,
        capacity: usize, length: *mut usize) -> bool;
}
