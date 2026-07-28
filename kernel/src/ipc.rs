use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use crate::capability::{CapabilityHandle, CapabilityObject, CapabilitySpace, Rights};
use crate::task::AddressSpaceId;
use synos_status::{IntoStatus, Severity, Status, facility};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SharedBuffer {
    pub region: SharedRegionId,
    pub offset: u32,
    pub length: u32,
    pub writable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Message {
    pub label: u64,
    pub buffer: Option<SharedBuffer>,
    pub words: [u64; 4],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityTransfer {
    pub delegated: CapabilityHandle,
    pub receiver: AddressSpaceId,
    pub rights: Rights,
}

impl Message {
    pub const EMPTY: Self = Self {
        label: 0,
        buffer: None,
        words: [0; 4],
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IpcError {
    Full,
    Empty,
    AccessDenied,
}

impl IntoStatus for IpcError {
    fn status(self) -> Status {
        match self {
            Self::Full => Status::BUSY,
            Self::Empty => Status::new(Severity::Information, facility::KERNEL, 2, 0)
                .expect("valid IPC status"),
            Self::AccessDenied => Status::ACCESS_DENIED,
        }
    }
}

struct Slot {
    sequence: AtomicUsize,
    message: AtomicMessage,
}

impl Slot {
    const fn new(sequence: usize) -> Self {
        Self {
            sequence: AtomicUsize::new(sequence),
            message: AtomicMessage::new(),
        }
    }
}

struct AtomicMessage {
    label: AtomicU64,
    buffer_present: AtomicBool,
    buffer_region: AtomicU32,
    buffer_offset: AtomicU32,
    buffer_length: AtomicU32,
    buffer_writable: AtomicBool,
    words: [AtomicU64; 4],
}

impl AtomicMessage {
    const fn new() -> Self {
        Self {
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

    fn write(&self, message: Message) {
        self.label.store(message.label, Ordering::Relaxed);
        if let Some(buffer) = message.buffer {
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
        for (word, value) in self.words.iter().zip(message.words) {
            word.store(value, Ordering::Relaxed)
        }
    }

    fn read(&self) -> Message {
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
        Message {
            label: self.label.load(Ordering::Relaxed),
            buffer,
            words,
        }
    }
}

/// A bounded, non-blocking MPMC channel.
///
/// Messages carry a shared-region descriptor instead of copying payload bytes.
/// The sender and receiver address spaces must already map that region.
pub struct Channel<const CAPACITY: usize> {
    id: ChannelId,
    enqueue_position: AtomicUsize,
    dequeue_position: AtomicUsize,
    slots: [Slot; CAPACITY],
}

impl<const CAPACITY: usize> Channel<CAPACITY> {
    pub fn new(id: ChannelId) -> Self {
        assert!(CAPACITY >= 2);
        Self {
            id,
            enqueue_position: AtomicUsize::new(0),
            dequeue_position: AtomicUsize::new(0),
            slots: core::array::from_fn(Slot::new),
        }
    }

    pub const fn id(&self) -> ChannelId {
        self.id
    }

    pub fn try_send<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        buffer_authority: Option<CapabilityHandle>,
        message: Message,
    ) -> Result<(), IpcError> {
        capabilities
            .authorize(
                caller,
                endpoint,
                CapabilityObject::IpcChannel(self.id),
                Rights::SEND,
            )
            .map_err(|_| IpcError::AccessDenied)?;

        if let Some(buffer) = message.buffer {
            let handle = buffer_authority.ok_or(IpcError::AccessDenied)?;
            capabilities
                .authorize_mapping(caller, handle, buffer.region, buffer.writable, false)
                .map_err(|_| IpcError::AccessDenied)?;
        }

        self.enqueue(message)
    }

    /// Atomically attenuate a capability and attach its handle to a zero-copy
    /// IPC message. The delegated handle is placed in word 3.
    #[allow(clippy::too_many_arguments)]
    pub fn try_send_delegated<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &mut CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        buffer_authority: Option<CapabilityHandle>,
        source: CapabilityHandle,
        receiver: AddressSpaceId,
        rights: Rights,
        mut message: Message,
    ) -> Result<CapabilityTransfer, IpcError> {
        capabilities
            .authorize(
                caller,
                endpoint,
                CapabilityObject::IpcChannel(self.id),
                Rights::SEND,
            )
            .map_err(|_| IpcError::AccessDenied)?;
        if let Some(buffer) = message.buffer {
            let handle = buffer_authority.ok_or(IpcError::AccessDenied)?;
            capabilities
                .authorize_mapping(caller, handle, buffer.region, buffer.writable, false)
                .map_err(|_| IpcError::AccessDenied)?
        }
        let delegated = capabilities
            .delegate(caller, source, receiver, rights)
            .map_err(|_| IpcError::AccessDenied)?;
        message.words[3] = delegated.raw();
        if let Err(error) = self.enqueue(message) {
            let _ = capabilities.delete(receiver, delegated);
            return Err(error)
        }
        Ok(CapabilityTransfer {
            delegated,
            receiver,
            rights,
        })
    }

    /// Validate capabilities once, then use the mapped ring without syscalls.
    pub fn map_sender<'a, const MAX_CAPABILITIES: usize>(
        &'a self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        ring_memory: CapabilityHandle,
        ring_region: SharedRegionId,
    ) -> Result<MappedSender<'a, CAPACITY>, IpcError> {
        capabilities
            .authorize(
                caller,
                endpoint,
                CapabilityObject::IpcChannel(self.id),
                Rights::SEND,
            )
            .map_err(|_| IpcError::AccessDenied)?;
        capabilities
            .authorize_mapping(caller, ring_memory, ring_region, true, false)
            .map_err(|_| IpcError::AccessDenied)?;
        Ok(MappedSender {
            channel: self,
            ring_region,
        })
    }

    /// Validate capabilities once, then consume the read-only shared ring.
    pub fn map_receiver<'a, const MAX_CAPABILITIES: usize>(
        &'a self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
        ring_memory: CapabilityHandle,
        ring_region: SharedRegionId,
    ) -> Result<MappedReceiver<'a, CAPACITY>, IpcError> {
        capabilities
            .authorize(
                caller,
                endpoint,
                CapabilityObject::IpcChannel(self.id),
                Rights::RECEIVE,
            )
            .map_err(|_| IpcError::AccessDenied)?;
        capabilities
            .authorize_mapping(caller, ring_memory, ring_region, false, false)
            .map_err(|_| IpcError::AccessDenied)?;
        Ok(MappedReceiver {
            channel: self,
            ring_region,
        })
    }

    fn enqueue(&self, message: Message) -> Result<(), IpcError> {
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
                        slot.message.write(message);
                        slot.sequence
                            .store(position.wrapping_add(1), Ordering::Release);
                        return Ok(());
                    }
                    Err(observed) => position = observed,
                }
            } else if difference < 0 {
                return Err(IpcError::Full);
            } else {
                position = self.enqueue_position.load(Ordering::Relaxed)
            }
        }
    }

    pub fn try_receive<const MAX_CAPABILITIES: usize>(
        &self,
        capabilities: &CapabilitySpace<MAX_CAPABILITIES>,
        caller: AddressSpaceId,
        endpoint: CapabilityHandle,
    ) -> Result<Message, IpcError> {
        capabilities
            .authorize(
                caller,
                endpoint,
                CapabilityObject::IpcChannel(self.id),
                Rights::RECEIVE,
            )
            .map_err(|_| IpcError::AccessDenied)?;

        self.dequeue()
    }

    fn dequeue(&self) -> Result<Message, IpcError> {
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
                        let message = slot.message.read();
                        slot.sequence
                            .store(position.wrapping_add(CAPACITY), Ordering::Release);
                        return Ok(message);
                    }
                    Err(observed) => position = observed,
                }
            } else if difference < 0 {
                return Err(IpcError::Empty);
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

/// Direct producer view over a capability-mapped shared-memory ring.
///
/// Setup enters the kernel once. Steady-state sends are atomic memory
/// operations and may only reference the ring's own mapped region.
pub struct MappedSender<'a, const CAPACITY: usize> {
    channel: &'a Channel<CAPACITY>,
    ring_region: SharedRegionId,
}

impl<const CAPACITY: usize> MappedSender<'_, CAPACITY> {
    pub fn try_send(&self, message: Message) -> Result<(), IpcError> {
        if message
            .buffer
            .is_some_and(|buffer| buffer.region != self.ring_region)
        {
            return Err(IpcError::AccessDenied);
        }
        self.channel.enqueue(message)
    }

    pub const fn region(&self) -> SharedRegionId {
        self.ring_region
    }
}

/// Direct consumer view over the read-only side of a shared-memory ring.
pub struct MappedReceiver<'a, const CAPACITY: usize> {
    channel: &'a Channel<CAPACITY>,
    ring_region: SharedRegionId,
}

impl<const CAPACITY: usize> MappedReceiver<'_, CAPACITY> {
    pub fn try_receive(&self) -> Result<Message, IpcError> {
        self.channel.dequeue()
    }

    pub const fn region(&self) -> SharedRegionId {
        self.ring_region
    }
}
