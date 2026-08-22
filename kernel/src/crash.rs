use core::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, AtomicU8, Ordering};

use ghostos_observability::{CorrelationId, SECURITY_AUDIT};
use ghostos_status::Status;

use crate::capability::{CapabilityObject, CapabilitySpace};
use crate::persistence::PersistentStore;
use crate::scheduler::Scheduler;

pub const CAPSULE_MAGIC: [u8; 8] = *b"SYNCRSH1";
pub const CAPSULE_VERSION: u16 = 1;
pub const MAX_CAPABILITY_RECORDS: usize = 8;
pub const MAX_AUDIT_IDS: usize = 8;
pub const MAX_CAPSULE_BYTES: usize = 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RegisterState {
    pub general: [u64; 16],
    pub instruction_pointer: u64,
    pub stack_pointer: u64,
    pub flags: u64,
    pub fault_address: u64,
}

impl RegisterState {
    pub const fn empty() -> Self {
        Self {
            general: [0; 16],
            instruction_pointer: 0,
            stack_pointer: 0,
            flags: 0,
            fault_address: 0,
        }
    }

    pub const fn with_fault_address(mut self, fault_address: u64) -> Self {
        self.fault_address = fault_address;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityRecord {
    pub redacted_handle: u64,
    pub redacted_owner: u32,
    pub rights: u16,
    pub object_kind: u8,
}

impl CapabilityRecord {
    const EMPTY: Self = Self {
        redacted_handle: 0,
        redacted_owner: 0,
        rights: 0,
        object_kind: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityContext {
    pub active: u16,
    pub capacity: u16,
    pub records: [CapabilityRecord; MAX_CAPABILITY_RECORDS],
    pub record_count: u8,
}

impl CapabilityContext {
    const EMPTY: Self = Self {
        active: 0,
        capacity: 0,
        records: [CapabilityRecord::EMPTY; MAX_CAPABILITY_RECORDS],
        record_count: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SchedulerState {
    pub clock: u64,
    pub current_thread: u32,
    pub current_cpu: u8,
    pub state_counts: [u16; 5],
    pub current_instruction_pointer: u64,
    pub current_stack_pointer: u64,
}

impl SchedulerState {
    const EMPTY: Self = Self {
        clock: 0,
        current_thread: 0,
        current_cpu: 0,
        state_counts: [0; 5],
        current_instruction_pointer: 0,
        current_stack_pointer: 0,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuildIdentity {
    pub package: u64,
    pub version: u64,
    pub target: u64,
}

const fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(0x100000001b3);
        index += 1;
    }
    hash
}

const fn target_identity() -> u64 {
    if cfg!(target_arch = "x86_64") {
        hash_bytes(b"x86_64")
    } else if cfg!(target_arch = "aarch64") {
        hash_bytes(b"aarch64")
    } else if cfg!(target_arch = "riscv64") {
        hash_bytes(b"riscv64")
    } else {
        hash_bytes(b"unsupported")
    }
}

pub const BUILD_IDENTITY: BuildIdentity = BuildIdentity {
    package: hash_bytes(env!("CARGO_PKG_NAME").as_bytes()),
    version: hash_bytes(env!("CARGO_PKG_VERSION").as_bytes()),
    target: target_identity(),
};

static CRASH_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static CAPABILITY_CONTEXT_SEQUENCE: AtomicU32 = AtomicU32::new(0);
static CAPABILITY_ACTIVE: AtomicU16 = AtomicU16::new(0);
static CAPABILITY_CAPACITY: AtomicU16 = AtomicU16::new(0);
static CAPABILITY_RECORD_COUNT: AtomicU8 = AtomicU8::new(0);
static CAPABILITY_HANDLES: [AtomicU64; MAX_CAPABILITY_RECORDS] =
    [const { AtomicU64::new(0) }; MAX_CAPABILITY_RECORDS];
static CAPABILITY_OWNERS: [AtomicU32; MAX_CAPABILITY_RECORDS] =
    [const { AtomicU32::new(0) }; MAX_CAPABILITY_RECORDS];
static CAPABILITY_RIGHTS: [AtomicU16; MAX_CAPABILITY_RECORDS] =
    [const { AtomicU16::new(0) }; MAX_CAPABILITY_RECORDS];
static CAPABILITY_OBJECTS: [AtomicU8; MAX_CAPABILITY_RECORDS] =
    [const { AtomicU8::new(0) }; MAX_CAPABILITY_RECORDS];

pub(crate) fn publish_capability_context<const CAPACITY: usize>(
    capabilities: &CapabilitySpace<CAPACITY>,
) {
    CAPABILITY_CONTEXT_SEQUENCE.fetch_add(1, Ordering::AcqRel);
    CAPABILITY_ACTIVE.store(capabilities.used() as u16, Ordering::Relaxed);
    CAPABILITY_CAPACITY.store(CAPACITY.min(u16::MAX as usize) as u16, Ordering::Relaxed);

    let mut count = 0;
    for (handle, info) in capabilities.entries() {
        if count == MAX_CAPABILITY_RECORDS {
            break
        }
        CAPABILITY_HANDLES[count].store(redact_u64(handle.raw()), Ordering::Relaxed);
        CAPABILITY_OWNERS[count].store(redact_u32(info.owner.raw()), Ordering::Relaxed);
        CAPABILITY_RIGHTS[count].store(info.rights.bits(), Ordering::Relaxed);
        CAPABILITY_OBJECTS[count].store(object_kind(info.object), Ordering::Relaxed);
        count += 1;
    }
    CAPABILITY_RECORD_COUNT.store(count as u8, Ordering::Relaxed);
    CAPABILITY_CONTEXT_SEQUENCE.fetch_add(1, Ordering::Release);
}

fn capability_context() -> CapabilityContext {
    for _ in 0..2 {
        let before = CAPABILITY_CONTEXT_SEQUENCE.load(Ordering::Acquire);
        if before & 1 != 0 {
            continue
        }
        let mut context = CapabilityContext {
            active: CAPABILITY_ACTIVE.load(Ordering::Relaxed),
            capacity: CAPABILITY_CAPACITY.load(Ordering::Relaxed),
            records: [CapabilityRecord::EMPTY; MAX_CAPABILITY_RECORDS],
            record_count: CAPABILITY_RECORD_COUNT
                .load(Ordering::Relaxed)
                .min(MAX_CAPABILITY_RECORDS as u8),
        };
        for index in 0..context.record_count as usize {
            context.records[index] = CapabilityRecord {
                redacted_handle: CAPABILITY_HANDLES[index].load(Ordering::Relaxed),
                redacted_owner: CAPABILITY_OWNERS[index].load(Ordering::Relaxed),
                rights: CAPABILITY_RIGHTS[index].load(Ordering::Relaxed),
                object_kind: CAPABILITY_OBJECTS[index].load(Ordering::Relaxed),
            }
        }
        if CAPABILITY_CONTEXT_SEQUENCE.load(Ordering::Acquire) == before {
            return context
        }
    }
    CapabilityContext::EMPTY
}

fn object_kind(object: CapabilityObject) -> u8 {
    match object {
        CapabilityObject::UntypedMemory(_) => 1,
        CapabilityObject::Mmio(_) => 11,
        CapabilityObject::MemoryRegion(_) => 2,
        CapabilityObject::AddressSpace(_) => 3,
        CapabilityObject::Thread(_) => 4,
        CapabilityObject::SystemControl => 5,
        CapabilityObject::NetworkDiagnostic => 6,
        CapabilityObject::IpcChannel(_) => 7,
        CapabilityObject::DistributedResource(_) => 8,
        CapabilityObject::LogicalNamespace { .. } => 9,
        CapabilityObject::DmaDevice(_) => 10,
    }
}

fn redact_u64(value: u64) -> u64 {
    let mut hash = 0xd6e8feb86659fd93;
    let mut shift = 0;
    while shift < 64 {
        hash ^= (value >> shift) & 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
        shift += 8;
    }
    hash
}

fn redact_u32(value: u32) -> u32 {
    (redact_u64(value as u64) ^ (redact_u64(value as u64) >> 32)) as u32
}

pub fn capture_and_persist(
    mut registers: RegisterState,
    fault_address: u64,
    status: Status,
    reason: u16,
    scheduler: Option<&Scheduler>,
) {
    if CRASH_IN_PROGRESS
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return
    }

    registers.fault_address = fault_address;
    let scheduler = scheduler.map_or(SchedulerState::EMPTY, Scheduler::crash_snapshot);
    let mut audit_events = [None; MAX_AUDIT_IDS];
    let audit_count = SECURITY_AUDIT.copy_recent(&mut audit_events);
    let mut audit_ids = [CorrelationId::NONE; MAX_AUDIT_IDS];
    for (destination, event) in audit_ids.iter_mut().zip(audit_events.into_iter()) {
        if let Some(event) = event {
            *destination = event.correlation;
        }
    }

    let capsule = CrashCapsule {
        reason,
        status: status.raw() as u64,
        build: BUILD_IDENTITY,
        registers,
        capabilities: capability_context(),
        scheduler,
        audit_ids,
        audit_count: audit_count as u8,
    };
    let mut bytes = [0; MAX_CAPSULE_BYTES];
    if let Some(length) = capsule.encode(&mut bytes) {
        PersistentStore::new().save_crash_capsule(&bytes[..length])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CrashCapsule {
    pub reason: u16,
    pub status: u64,
    pub build: BuildIdentity,
    pub registers: RegisterState,
    pub capabilities: CapabilityContext,
    pub scheduler: SchedulerState,
    pub audit_ids: [CorrelationId; MAX_AUDIT_IDS],
    pub audit_count: u8,
}

impl CrashCapsule {
    pub fn encode(&self, destination: &mut [u8]) -> Option<usize> {
        let mut writer = Writer::new(destination);
        writer.bytes(&CAPSULE_MAGIC)?;
        writer.u16(CAPSULE_VERSION)?;
        let length_offset = writer.position();
        writer.u16(0)?;
        writer.u16(self.reason)?;
        writer.u16(0)?;
        writer.u64(self.status)?;
        writer.u64(self.build.package)?;
        writer.u64(self.build.version)?;
        writer.u64(self.build.target)?;

        writer.u64(self.registers.fault_address)?;
        writer.u64(self.registers.instruction_pointer)?;
        writer.u64(self.registers.stack_pointer)?;
        writer.u64(self.registers.flags)?;
        for register in self.registers.general {
            writer.u64(register)?;
        }

        writer.u16(self.capabilities.active)?;
        writer.u16(self.capabilities.capacity)?;
        writer.u8(self.capabilities.record_count)?;
        writer.u8(0)?;
        for record in self.capabilities.records {
            writer.u64(record.redacted_handle)?;
            writer.u32(record.redacted_owner)?;
            writer.u16(record.rights)?;
            writer.u8(record.object_kind)?;
            writer.u8(0)?;
        }

        writer.u64(self.scheduler.clock)?;
        writer.u32(self.scheduler.current_thread)?;
        writer.u8(self.scheduler.current_cpu)?;
        for count in self.scheduler.state_counts {
            writer.u16(count)?;
        }
        writer.u64(self.scheduler.current_instruction_pointer)?;
        writer.u64(self.scheduler.current_stack_pointer)?;

        writer.u8(self.audit_count.min(MAX_AUDIT_IDS as u8))?;
        writer.bytes(&[0; 7])?;
        for audit_id in self.audit_ids {
            writer.u128(audit_id.raw())?;
        }

        let length = writer.position();
        writer.patch_u16(length_offset, length as u16)?;
        Some(length)
    }
}

struct Writer<'a> {
    destination: &'a mut [u8],
    position: usize,
}

impl<'a> Writer<'a> {
    const fn new(destination: &'a mut [u8]) -> Self {
        Self {
            destination,
            position: 0,
        }
    }

    const fn position(&self) -> usize {
        self.position
    }

    fn bytes(&mut self, bytes: &[u8]) -> Option<()> {
        let end = self.position.checked_add(bytes.len())?;
        self.destination.get_mut(self.position..end)?.copy_from_slice(bytes);
        self.position = end;
        Some(())
    }

    fn u8(&mut self, value: u8) -> Option<()> {
        self.bytes(&[value])
    }

    fn u16(&mut self, value: u16) -> Option<()> {
        self.bytes(&value.to_le_bytes())
    }

    fn u32(&mut self, value: u32) -> Option<()> {
        self.bytes(&value.to_le_bytes())
    }

    fn u64(&mut self, value: u64) -> Option<()> {
        self.bytes(&value.to_le_bytes())
    }

    fn u128(&mut self, value: u128) -> Option<()> {
        self.bytes(&value.to_le_bytes())
    }

    fn patch_u16(&mut self, offset: usize, value: u16) -> Option<()> {
        let end = offset.checked_add(2)?;
        self.destination.get_mut(offset..end)?.copy_from_slice(&value.to_le_bytes());
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capsule_is_bounded_and_contains_build_identity() {
        let capsule = CrashCapsule {
            reason: 7,
            status: Status::CORRUPT.raw() as u64,
            build: BUILD_IDENTITY,
            registers: RegisterState::empty().with_fault_address(0xdead),
            capabilities: CapabilityContext::EMPTY,
            scheduler: SchedulerState::EMPTY,
            audit_ids: [CorrelationId::from_raw(9); MAX_AUDIT_IDS],
            audit_count: 1,
        };
        let mut bytes = [0; MAX_CAPSULE_BYTES];
        let length = capsule.encode(&mut bytes).expect("capsule fits");
        assert!(length < MAX_CAPSULE_BYTES);
        assert_eq!(&bytes[..CAPSULE_MAGIC.len()], &CAPSULE_MAGIC);
        assert!(bytes.windows(8).any(|window| window == &BUILD_IDENTITY.package.to_le_bytes()));
    }

    #[test]
    fn capability_redaction_does_not_store_raw_handle() {
        let mut capabilities = CapabilitySpace::<2>::new();
        let handle = capabilities
            .mint_root(
                crate::AddressSpaceId::KERNEL,
                CapabilityObject::SystemControl,
                crate::Rights::CONTROL,
            )
            .expect("root capability");
        publish_capability_context(&capabilities);
        let context = capability_context();
        assert_eq!(context.active, 1);
        assert_ne!(context.records[0].redacted_handle, handle.raw());
    }
}
