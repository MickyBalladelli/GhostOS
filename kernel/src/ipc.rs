use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicUsize, Ordering};

use crate::capability::{CapabilityHandle, CapabilityObject, CapabilitySpace, Rights};
use crate::task::AddressSpaceId;

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

struct Slot {
    sequence: AtomicUsize,
    message: UnsafeCell<Message>,
}

impl Slot {
    const fn new(sequence: usize) -> Self {
        Self {
            sequence: AtomicUsize::new(sequence),
            message: UnsafeCell::new(Message::EMPTY),
        }
    }
}

// Access to the cell is owned by a queue position before it is read or written.
unsafe impl Sync for Slot {}

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
                        // Safety: the successful CAS gives this producer exclusive ownership.
                        unsafe {
                            slot.message.get().write(message);
                        }
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
                        // Safety: the successful CAS gives this consumer exclusive ownership.
                        let message = unsafe { slot.message.get().read() };
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
