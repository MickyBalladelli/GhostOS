use core::sync::atomic::{AtomicU8, AtomicU64, Ordering};

use crate::{
    LogicalName, MAX_NAME_BYTES,
    logical::{
        LogicalError, LogicalScope, LogicalTarget, LogicalTargetKind, MAX_LOGICAL_VALUE_BYTES,
        ResolvedLogicalName,
    },
};

pub const DEFAULT_FAST_LOGICAL_CAPACITY: usize = 32;

struct AtomicLogicalSlot {
    version: AtomicU64,
    hash: AtomicU64,
    name_length: AtomicU8,
    target_kind: AtomicU8,
    target_length: AtomicU8,
    name: [AtomicU8; MAX_NAME_BYTES],
    target: [AtomicU8; MAX_LOGICAL_VALUE_BYTES],
}

impl AtomicLogicalSlot {
    const fn new() -> Self {
        Self {
            version: AtomicU64::new(0),
            hash: AtomicU64::new(0),
            name_length: AtomicU8::new(0),
            target_kind: AtomicU8::new(0),
            target_length: AtomicU8::new(0),
            name: [const { AtomicU8::new(0) }; MAX_NAME_BYTES],
            target: [const { AtomicU8::new(0) }; MAX_LOGICAL_VALUE_BYTES],
        }
    }

    fn clear(&self) {
        self.version.fetch_add(1, Ordering::AcqRel);
        self.hash.store(0, Ordering::Relaxed);
        self.version.fetch_add(1, Ordering::Release);
    }

    fn write(&self, hash: u64, name: LogicalName, target: LogicalTarget) {
        self.version.fetch_add(1, Ordering::AcqRel);
        let name_bytes = name.as_str().as_bytes();
        let target_bytes = target.as_str().as_bytes();
        self.name_length
            .store(name_bytes.len() as u8, Ordering::Relaxed);
        self.target_length
            .store(target_bytes.len() as u8, Ordering::Relaxed);
        self.target_kind.store(
            match target.kind {
                LogicalTargetKind::File => 1,
                LogicalTargetKind::Device => 2,
                LogicalTargetKind::IpcChannel => 3,
            },
            Ordering::Relaxed,
        );
        for (slot, byte) in self.name.iter().zip(name_bytes) {
            slot.store(byte.to_ascii_lowercase(), Ordering::Relaxed)
        }
        for (slot, byte) in self.target.iter().zip(target_bytes) {
            slot.store(*byte, Ordering::Relaxed)
        }
        self.hash.store(hash, Ordering::Relaxed);
        self.version.fetch_add(1, Ordering::Release);
    }

    fn read(&self, hash: u64, name: &str) -> Option<LogicalTarget> {
        loop {
            let before = self.version.load(Ordering::Acquire);
            if before & 1 != 0 {
                core::hint::spin_loop();
                continue;
            }
            let stored_hash = self.hash.load(Ordering::Relaxed);
            if stored_hash == 0 || stored_hash != hash {
                return None;
            }
            let name_length = self.name_length.load(Ordering::Relaxed) as usize;
            if name_length != name.len()
                || self.name[..name_length]
                    .iter()
                    .zip(name.as_bytes())
                    .any(|(stored, wanted)| {
                        stored.load(Ordering::Relaxed) != wanted.to_ascii_lowercase()
                    })
            {
                return None;
            }
            let length = self.target_length.load(Ordering::Relaxed) as usize;
            let mut bytes = [0; MAX_LOGICAL_VALUE_BYTES];
            for (byte, stored) in bytes.iter_mut().zip(&self.target).take(length) {
                *byte = stored.load(Ordering::Relaxed)
            }
            let kind = match self.target_kind.load(Ordering::Relaxed) {
                1 => LogicalTargetKind::File,
                2 => LogicalTargetKind::Device,
                3 => LogicalTargetKind::IpcChannel,
                _ => return None,
            };
            if self.version.load(Ordering::Acquire) != before {
                continue;
            }
            let value = core::str::from_utf8(&bytes[..length]).ok()?;
            return LogicalTarget::new(kind, value).ok();
        }
    }
}

pub struct AtomicLogicalTable<const CAPACITY: usize = DEFAULT_FAST_LOGICAL_CAPACITY> {
    slots: [AtomicLogicalSlot; CAPACITY],
}

impl<const CAPACITY: usize> AtomicLogicalTable<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            slots: [const { AtomicLogicalSlot::new() }; CAPACITY],
        }
    }

    pub(crate) fn clear(&self) {
        for slot in &self.slots {
            slot.clear()
        }
    }

    pub(crate) fn insert(
        &self,
        name: LogicalName,
        target: LogicalTarget,
    ) -> Result<(), LogicalError> {
        if CAPACITY == 0 {
            return Err(LogicalError::TableFull);
        }
        let hash = logical_hash(name.as_str());
        let start = hash as usize % CAPACITY;
        for probe in 0..CAPACITY {
            let slot = &self.slots[(start + probe) % CAPACITY];
            let stored_hash = slot.hash.load(Ordering::Acquire);
            if stored_hash == 0 || slot.read(hash, name.as_str()).is_some() {
                slot.write(hash, name, target);
                return Ok(());
            }
        }
        Err(LogicalError::TableFull)
    }

    fn resolve(&self, scope: LogicalScope, name: &str) -> Option<ResolvedLogicalName> {
        if CAPACITY == 0 {
            return None;
        }
        let hash = logical_hash(name);
        let start = hash as usize % CAPACITY;
        for probe in 0..CAPACITY {
            let slot = &self.slots[(start + probe) % CAPACITY];
            if slot.hash.load(Ordering::Acquire) == 0 {
                return None;
            }
            if let Some(target) = slot.read(hash, name) {
                let name = LogicalName::new(name).ok()?;
                return Some(ResolvedLogicalName {
                    scope,
                    name,
                    target,
                });
            }
        }
        None
    }
}

impl<const CAPACITY: usize> Default for AtomicLogicalTable<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-process read-only view of process and system logical-name tables.
///
/// The publisher flips one epoch around a complete rebuild. Readers use
/// atomic open-addressed tables and retry if publication overlaps a lookup.
pub struct LogicalFastPath<
    const PROCESS: usize = DEFAULT_FAST_LOGICAL_CAPACITY,
    const SYSTEM: usize = DEFAULT_FAST_LOGICAL_CAPACITY,
> {
    epoch: AtomicU64,
    process_id: AtomicU64,
    process: AtomicLogicalTable<PROCESS>,
    system: AtomicLogicalTable<SYSTEM>,
}

impl<const PROCESS: usize, const SYSTEM: usize> LogicalFastPath<PROCESS, SYSTEM> {
    pub const fn new() -> Self {
        Self {
            epoch: AtomicU64::new(0),
            process_id: AtomicU64::new(0),
            process: AtomicLogicalTable::new(),
            system: AtomicLogicalTable::new(),
        }
    }

    pub(crate) fn begin_publish(&self, process_id: u64) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        self.process_id.store(process_id, Ordering::Relaxed);
        self.process.clear();
        self.system.clear();
    }

    pub(crate) fn insert_process(
        &self,
        name: LogicalName,
        target: LogicalTarget,
    ) -> Result<(), LogicalError> {
        self.process.insert(name, target)
    }

    pub(crate) fn insert_system(
        &self,
        name: LogicalName,
        target: LogicalTarget,
    ) -> Result<(), LogicalError> {
        self.system.insert(name, target)
    }

    pub(crate) fn finish_publish(&self) {
        self.epoch.fetch_add(1, Ordering::Release);
    }

    pub(crate) fn abort_publish(&self) {
        self.process.clear();
        self.system.clear();
        self.finish_publish();
    }

    pub fn resolve(
        &self,
        process_id: u64,
        name: &str,
    ) -> Result<ResolvedLogicalName, LogicalError> {
        LogicalName::new(name).map_err(|_| LogicalError::InvalidName)?;
        loop {
            let before = self.epoch.load(Ordering::Acquire);
            if before & 1 != 0 {
                core::hint::spin_loop();
                continue;
            }
            let result = if self.process_id.load(Ordering::Relaxed) == process_id {
                self.process
                    .resolve(LogicalScope::Process(process_id), name)
                    .or_else(|| self.system.resolve(LogicalScope::System, name))
            } else {
                self.system.resolve(LogicalScope::System, name)
            };
            if self.epoch.load(Ordering::Acquire) == before {
                return result.ok_or(LogicalError::NotFound);
            }
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }
}

impl<const PROCESS: usize, const SYSTEM: usize> Default for LogicalFastPath<PROCESS, SYSTEM> {
    fn default() -> Self {
        Self::new()
    }
}

fn logical_hash(name: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in name.bytes() {
        hash ^= byte.to_ascii_lowercase() as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash.max(1)
}
