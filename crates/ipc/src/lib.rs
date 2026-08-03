#![no_std]
#![forbid(unsafe_code)]

use core::mem::{align_of, size_of};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use zerocopy::{FromBytes, Immutable, IntoBytes, KnownLayout};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct ChannelId(u32);

impl ChannelId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct SharedRegionId(u32);

impl SharedRegionId {
    pub const fn new(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

pub const MAX_INHERITABLE_DESCRIPTORS: usize = 32;

/// An IPC object that may be handed to a replacement Ring 3 process.
///
/// Inheritance duplicates the process attachment. It does not revoke the
/// source attachment; the old process remains able to serve requests until
/// the supervisor performs the atomic service switch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InheritableDescriptor {
    Channel(ChannelId),
    SharedRegion(SharedRegionId),
}

impl InheritableDescriptor {
    pub const fn channel(raw: u32) -> Option<Self> {
        match ChannelId::new(raw) {
            Some(id) => Some(Self::Channel(id)),
            None => None,
        }
    }

    pub const fn shared_region(raw: u32) -> Option<Self> {
        match SharedRegionId::new(raw) {
            Some(id) => Some(Self::SharedRegion(id)),
            None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorInheritanceError {
    Capacity,
    Duplicate,
}

pub fn validate_inheritable_descriptors(
    descriptors: &[InheritableDescriptor],
) -> Result<(), DescriptorInheritanceError> {
    if descriptors.len() > MAX_INHERITABLE_DESCRIPTORS {
        return Err(DescriptorInheritanceError::Capacity);
    }
    for (index, descriptor) in descriptors.iter().enumerate() {
        if descriptors[..index].contains(descriptor) {
            return Err(DescriptorInheritanceError::Duplicate);
        }
    }
    Ok(())
}

/// Ring 0 hook used by a hot-swap coordinator to duplicate IPC ownership.
///
/// Process IDs are raw here so the IPC crate stays independent from the
/// process supervisor that owns them.
pub trait DescriptorInheritance {
    type Error;

    fn inherit_descriptors(
        &mut self,
        source_process: u64,
        target_process: u64,
        descriptors: &[InheritableDescriptor],
    ) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SharedBuffer {
    pub region: SharedRegionId,
    pub offset: u32,
    pub length: u32,
    pub writable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Envelope {
    pub correlation: u128,
    pub label: u64,
    pub buffer: Option<SharedBuffer>,
    pub words: [u64; 4],
}

impl Envelope {
    pub const EMPTY: Self = Self {
        correlation: 0,
        label: 0,
        buffer: None,
        words: [0; 4],
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RingError {
    Empty,
    Full,
}

struct Slot {
    sequence: AtomicUsize,
    envelope: AtomicEnvelope,
}

impl Slot {
    const fn new(sequence: usize) -> Self {
        Self {
            sequence: AtomicUsize::new(sequence),
            envelope: AtomicEnvelope::new(),
        }
    }
}

struct AtomicEnvelope {
    correlation_low: AtomicU64,
    correlation_high: AtomicU64,
    label: AtomicU64,
    buffer_present: AtomicBool,
    buffer_region: AtomicU32,
    buffer_offset: AtomicU32,
    buffer_length: AtomicU32,
    buffer_writable: AtomicBool,
    words: [AtomicU64; 4],
}

impl AtomicEnvelope {
    const fn new() -> Self {
        Self {
            correlation_low: AtomicU64::new(0),
            correlation_high: AtomicU64::new(0),
            label: AtomicU64::new(0),
            buffer_present: AtomicBool::new(false),
            buffer_region: AtomicU32::new(0),
            buffer_offset: AtomicU32::new(0),
            buffer_length: AtomicU32::new(0),
            buffer_writable: AtomicBool::new(false),
            words: [
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
                AtomicU64::new(0),
            ],
        }
    }

    fn write(&self, envelope: Envelope) {
        self.correlation_low
            .store(envelope.correlation as u64, Ordering::Relaxed);
        self.correlation_high
            .store((envelope.correlation >> 64) as u64, Ordering::Relaxed);
        self.label.store(envelope.label, Ordering::Relaxed);
        if let Some(buffer) = envelope.buffer {
            self.buffer_region
                .store(buffer.region.raw(), Ordering::Relaxed);
            self.buffer_offset.store(buffer.offset, Ordering::Relaxed);
            self.buffer_length.store(buffer.length, Ordering::Relaxed);
            self.buffer_writable
                .store(buffer.writable, Ordering::Relaxed);
            self.buffer_present.store(true, Ordering::Relaxed)
        } else {
            self.buffer_present.store(false, Ordering::Relaxed)
        }
        for (word, value) in self.words.iter().zip(envelope.words) {
            word.store(value, Ordering::Relaxed)
        }
    }

    fn read(&self) -> Envelope {
        let buffer = if self.buffer_present.load(Ordering::Relaxed) {
            SharedRegionId::new(self.buffer_region.load(Ordering::Relaxed)).map(|region| {
                SharedBuffer {
                    region,
                    offset: self.buffer_offset.load(Ordering::Relaxed),
                    length: self.buffer_length.load(Ordering::Relaxed),
                    writable: self.buffer_writable.load(Ordering::Relaxed),
                }
            })
        } else {
            None
        };
        let mut words = [0; 4];
        for (value, word) in words.iter_mut().zip(&self.words) {
            *value = word.load(Ordering::Relaxed)
        }
        Envelope {
            correlation: self.correlation_low.load(Ordering::Relaxed) as u128
                | ((self.correlation_high.load(Ordering::Relaxed) as u128) << 64),
            label: self.label.load(Ordering::Relaxed),
            buffer,
            words,
        }
    }
}

/// A bounded MPMC ring intended to live in capability-mapped shared pages.
///
/// The kernel validates the mapping once. Producers and consumers then move
/// only the fixed envelope through atomics; payload bytes remain in the shared
/// region named by the envelope.
pub struct Ring<const CAPACITY: usize> {
    enqueue_position: AtomicUsize,
    dequeue_position: AtomicUsize,
    slots: [Slot; CAPACITY],
}

impl<const CAPACITY: usize> Ring<CAPACITY> {
    pub fn new() -> Self {
        assert!(CAPACITY >= 2);
        Self {
            enqueue_position: AtomicUsize::new(0),
            dequeue_position: AtomicUsize::new(0),
            slots: core::array::from_fn(Slot::new),
        }
    }

    pub fn try_send(&self, envelope: Envelope) -> Result<(), RingError> {
        let mut position = self.enqueue_position.load(Ordering::Relaxed);
        loop {
            let slot = &self.slots[position % CAPACITY];
            let sequence = slot.sequence.load(Ordering::Acquire);
            let difference = sequence.wrapping_sub(position) as isize;

            if difference == 0 {
                match self.enqueue_position.compare_exchange_weak(
                    position,
                    position.wrapping_add(1),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        slot.envelope.write(envelope);
                        slot.sequence
                            .store(position.wrapping_add(1), Ordering::Release);
                        return Ok(());
                    }
                    Err(observed) => position = observed,
                }
            } else if difference < 0 {
                return Err(RingError::Full);
            } else {
                position = self.enqueue_position.load(Ordering::Relaxed)
            }
        }
    }

    pub fn try_receive(&self) -> Result<Envelope, RingError> {
        let mut position = self.dequeue_position.load(Ordering::Relaxed);
        loop {
            let slot = &self.slots[position % CAPACITY];
            let sequence = slot.sequence.load(Ordering::Acquire);
            let expected = position.wrapping_add(1);
            let difference = sequence.wrapping_sub(expected) as isize;

            if difference == 0 {
                match self.dequeue_position.compare_exchange_weak(
                    position,
                    expected,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        let envelope = slot.envelope.read();
                        slot.sequence
                            .store(position.wrapping_add(CAPACITY), Ordering::Release);
                        return Ok(envelope);
                    }
                    Err(observed) => position = observed,
                }
            } else if difference < 0 {
                return Err(RingError::Empty);
            } else {
                position = self.dequeue_position.load(Ordering::Relaxed)
            }
        }
    }

    pub fn pending(&self) -> usize {
        self.enqueue_position
            .load(Ordering::Acquire)
            .wrapping_sub(self.dequeue_position.load(Ordering::Acquire))
            .min(CAPACITY)
    }
}

impl<const CAPACITY: usize> Default for Ring<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArchiveError {
    BufferTooSmall,
    InvalidDescriptor,
    InvalidLayout,
    SchemaMismatch,
}

/// Fixed-capacity bump writer over a shared mapping.
///
/// Values are archived once into shared memory. Sending an `Envelope` then
/// passes only the descriptor, without heap allocation or payload copying.
pub struct SharedArena<'a> {
    region: SharedRegionId,
    bytes: &'a mut [u8],
    cursor: usize,
}

impl<'a> SharedArena<'a> {
    pub fn new(region: SharedRegionId, bytes: &'a mut [u8]) -> Self {
        Self {
            region,
            bytes,
            cursor: 0,
        }
    }

    pub fn reset(&mut self) {
        self.cursor = 0
    }

    pub fn archive<T>(&mut self, value: &T) -> Result<SharedBuffer, ArchiveError>
    where
        T: IntoBytes + Immutable + KnownLayout,
    {
        let alignment = align_of::<T>();
        let base = self.bytes.as_ptr() as usize;
        let address = base
            .checked_add(self.cursor)
            .ok_or(ArchiveError::BufferTooSmall)?
            .checked_add(alignment - 1)
            .ok_or(ArchiveError::BufferTooSmall)?
            & !(alignment - 1);
        let start = address
            .checked_sub(base)
            .ok_or(ArchiveError::BufferTooSmall)?;
        let end = start
            .checked_add(size_of::<T>())
            .ok_or(ArchiveError::BufferTooSmall)?;
        let offset = u32::try_from(start).map_err(|_| ArchiveError::BufferTooSmall)?;
        let length = u32::try_from(size_of::<T>()).map_err(|_| ArchiveError::BufferTooSmall)?;
        let destination = self
            .bytes
            .get_mut(start..end)
            .ok_or(ArchiveError::BufferTooSmall)?;
        destination.copy_from_slice(value.as_bytes());
        self.cursor = end;
        Ok(SharedBuffer {
            region: self.region,
            offset,
            length,
            writable: false,
        })
    }

    pub fn archive_structured<T>(
        &mut self,
        schema: u64,
        correlation: u128,
        value: &T,
        delegated_capability: Option<u64>,
    ) -> Result<Envelope, ArchiveError>
    where
        T: IntoBytes + Immutable + KnownLayout,
    {
        let payload = self.archive(value)?;
        Ok(structured_envelope(
            schema,
            correlation,
            payload,
            delegated_capability,
            checksum(value.as_bytes()),
        ))
    }
}

pub struct SharedView<'a> {
    region: SharedRegionId,
    bytes: &'a [u8],
}

impl<'a> SharedView<'a> {
    pub const fn new(region: SharedRegionId, bytes: &'a [u8]) -> Self {
        Self { region, bytes }
    }

    pub fn resolve<T>(&self, descriptor: SharedBuffer) -> Result<&'a T, ArchiveError>
    where
        T: FromBytes + Immutable + KnownLayout,
    {
        if descriptor.region != self.region || descriptor.writable {
            return Err(ArchiveError::InvalidDescriptor);
        }
        let start = descriptor.offset as usize;
        let end = start
            .checked_add(descriptor.length as usize)
            .ok_or(ArchiveError::InvalidDescriptor)?;
        let bytes = self
            .bytes
            .get(start..end)
            .ok_or(ArchiveError::InvalidDescriptor)?;
        T::ref_from_bytes(bytes).map_err(|_| ArchiveError::InvalidLayout)
    }

    pub fn resolve_envelope<T>(
        &self,
        envelope: Envelope,
        expected_schema: u64,
    ) -> Result<&'a T, ArchiveError>
    where
        T: FromBytes + IntoBytes + Immutable + KnownLayout,
    {
        let descriptor = validate_structured(envelope, expected_schema)?;
        let value: &T = self.resolve(descriptor)?;
        if checksum(value.as_bytes()) != envelope.words[1] {
            return Err(ArchiveError::InvalidDescriptor);
        }
        Ok(value)
    }
}

pub fn structured_envelope(
    schema: u64,
    correlation: u128,
    payload: SharedBuffer,
    delegated_capability: Option<u64>,
    payload_checksum: u64,
) -> Envelope {
    Envelope {
        correlation,
        label: schema,
        buffer: Some(payload),
        words: [
            PROTOCOL_VERSION as u64,
            payload_checksum,
            delegated_capability.unwrap_or(0),
            0,
        ],
    }
}

pub fn validate_structured(
    envelope: Envelope,
    expected_schema: u64,
) -> Result<SharedBuffer, ArchiveError> {
    if envelope.label != expected_schema {
        return Err(ArchiveError::SchemaMismatch);
    }
    if envelope.words[0] != PROTOCOL_VERSION as u64 {
        return Err(ArchiveError::InvalidDescriptor);
    }
    envelope.buffer.ok_or(ArchiveError::InvalidDescriptor)
}

pub fn checksum(bytes: &[u8]) -> u64 {
    let mut value = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        value ^= *byte as u64;
        value = value.wrapping_mul(0x0000_0100_0000_01b3)
    }
    value
}
