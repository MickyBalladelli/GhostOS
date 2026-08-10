use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PacketError {
    Empty,
    Full,
    FrameTooLarge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SlotState {
    Free,
    Loaned,
    Ready,
}

struct PacketSlot<const MTU: usize> {
    bytes: [u8; MTU],
    length: usize,
    state: SlotState,
}

impl<const MTU: usize> PacketSlot<MTU> {
    const EMPTY: Self = Self {
        bytes: [0; MTU],
        length: 0,
        state: SlotState::Free,
    };
}

/// Fixed packet storage shared by a driver loop and the TCP/IP stack.
///
/// Producers reserve a slot and DMA or write directly into it. Consumers read
/// that same slot and release it. Packet payloads are never copied between
/// queue entries.
pub struct PacketQueue<const CAPACITY: usize, const MTU: usize> {
    slots: [PacketSlot<MTU>; CAPACITY],
    reserve_cursor: usize,
    receive_cursor: usize,
    drops: u64,
}

impl<const CAPACITY: usize, const MTU: usize> PacketQueue<CAPACITY, MTU> {
    pub const fn new() -> Self {
        Self {
            slots: [const { PacketSlot::EMPTY }; CAPACITY],
            reserve_cursor: 0,
            receive_cursor: 0,
            drops: 0,
        }
    }

    pub fn reserve(&mut self) -> Result<PacketWriter<'_, MTU>, PacketError> {
        if CAPACITY == 0 || MTU == 0 {
            self.drops = self.drops.saturating_add(1);
            return Err(PacketError::Full)
        }
        let index = (0..CAPACITY)
            .map(|distance| (self.reserve_cursor + distance) % CAPACITY)
            .find(|index| self.slots[*index].state == SlotState::Free)
            .ok_or_else(|| {
                self.drops = self.drops.saturating_add(1);
                PacketError::Full
            })?;
        self.reserve_cursor = (index + 1) % CAPACITY;
        let slot = &mut self.slots[index];
        slot.state = SlotState::Loaned;
        slot.length = 0;
        Ok(PacketWriter { slot })
    }

    pub fn dequeue(&mut self) -> Result<PacketReader<'_, MTU>, PacketError> {
        if CAPACITY == 0 {
            return Err(PacketError::Empty)
        }
        let index = (0..CAPACITY)
            .map(|distance| (self.receive_cursor + distance) % CAPACITY)
            .find(|index| self.slots[*index].state == SlotState::Ready)
            .ok_or(PacketError::Empty)?;
        self.receive_cursor = (index + 1) % CAPACITY;
        let slot = &mut self.slots[index];
        slot.state = SlotState::Loaned;
        Ok(PacketReader { slot })
    }

    /// Dequeues only frames accepted by the Ring 3 firewall callback. Rejected
    /// frames are released immediately, so the packet storage stays zero-copy.
    pub fn dequeue_filtered(
        &mut self,
        mut accept: impl FnMut(&[u8]) -> bool,
    ) -> Result<PacketReader<'_, MTU>, PacketError> {
        if CAPACITY == 0 {
            return Err(PacketError::Empty)
        }
        let accepted = (0..CAPACITY).find_map(|distance| {
            let index = (self.receive_cursor + distance) % CAPACITY;
            let slot = &self.slots[index];
            (slot.state == SlotState::Ready && accept(&slot.bytes[..slot.length]))
                .then_some((distance, index))
        });
        for distance in 0..CAPACITY {
            let index = (self.receive_cursor + distance) % CAPACITY;
            let inspected = accepted.is_none_or(|(accepted_distance, _)| distance <= accepted_distance);
            if inspected
                && self.slots[index].state == SlotState::Ready
                && accepted.is_none_or(|(_, accepted_index)| accepted_index != index)
            {
                self.slots[index].length = 0;
                self.slots[index].state = SlotState::Free;
            }
        }
        if let Some((_, index)) = accepted {
            self.receive_cursor = (index + 1) % CAPACITY;
            let slot = &mut self.slots[index];
            slot.state = SlotState::Loaned;
            return Ok(PacketReader { slot })
        }
        Err(PacketError::Empty)
    }

    pub fn pending(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.state == SlotState::Ready)
            .count()
    }

    pub fn available(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.state == SlotState::Free)
            .count()
    }

    pub const fn drops(&self) -> u64 {
        self.drops
    }
}

impl<const CAPACITY: usize, const MTU: usize> Default for PacketQueue<CAPACITY, MTU> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PacketWriter<'a, const MTU: usize> {
    slot: &'a mut PacketSlot<MTU>,
}

impl<const MTU: usize> PacketWriter<'_, MTU> {
    pub fn buffer(&mut self) -> &mut [u8] {
        &mut self.slot.bytes
    }

    pub fn capacity(&self) -> usize {
        MTU
    }

    pub fn commit(self, length: usize) -> Result<(), PacketError> {
        if length > MTU {
            return Err(PacketError::FrameTooLarge)
        }
        self.slot.length = length;
        self.slot.state = SlotState::Ready;
        Ok(())
    }
}

impl<const MTU: usize> Drop for PacketWriter<'_, MTU> {
    fn drop(&mut self) {
        if self.slot.state == SlotState::Loaned {
            self.slot.length = 0;
            self.slot.state = SlotState::Free
        }
    }
}

pub struct PacketReader<'a, const MTU: usize> {
    slot: &'a mut PacketSlot<MTU>,
}

impl<const MTU: usize> PacketReader<'_, MTU> {
    pub fn frame(&self) -> &[u8] {
        &self.slot.bytes[..self.slot.length]
    }
}

impl<const MTU: usize> Drop for PacketReader<'_, MTU> {
    fn drop(&mut self) {
        self.slot.length = 0;
        self.slot.state = SlotState::Free
    }
}

/// `smoltcp` device backed by loaned packet slots.
///
/// A Ring 3 NIC driver fills `ingress` slots and drains `egress` slots.
pub struct QueueDevice<const CAPACITY: usize, const MTU: usize> {
    pub ingress: PacketQueue<CAPACITY, MTU>,
    pub egress: PacketQueue<CAPACITY, MTU>,
}

impl<const CAPACITY: usize, const MTU: usize> QueueDevice<CAPACITY, MTU> {
    pub const fn new() -> Self {
        Self {
            ingress: PacketQueue::new(),
            egress: PacketQueue::new(),
        }
    }

    pub fn queue_depth(&self) -> (usize, usize) {
        (self.ingress.pending(), self.egress.pending())
    }

    pub const fn queue_drops(&self) -> u64 {
        self.ingress.drops().saturating_add(self.egress.drops())
    }
}

pub trait QueueMetrics {
    fn queue_depth(&self) -> (usize, usize);
    fn queue_drops(&self) -> u64;
}

impl<const CAPACITY: usize, const MTU: usize> QueueMetrics for QueueDevice<CAPACITY, MTU> {
    fn queue_depth(&self) -> (usize, usize) {
        self.queue_depth()
    }

    fn queue_drops(&self) -> u64 {
        self.queue_drops()
    }
}

impl<const CAPACITY: usize, const MTU: usize> Default for QueueDevice<CAPACITY, MTU> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct QueueRxToken<'a, const MTU: usize>(PacketReader<'a, MTU>);
pub struct QueueTxToken<'a, const MTU: usize>(PacketWriter<'a, MTU>);

impl<const MTU: usize> RxToken for QueueRxToken<'_, MTU> {
    fn consume<R, F>(self, receive: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        receive(self.0.frame())
    }
}

impl<const MTU: usize> TxToken for QueueTxToken<'_, MTU> {
    fn consume<R, F>(mut self, length: usize, transmit: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let length = length.min(self.0.buffer().len());
        let result = match self.0.buffer().get_mut(..length) {
            Some(buffer) => transmit(buffer),
            None => transmit(&mut []),
        };
        let _ = self.0.commit(length);
        result
    }
}

impl<const CAPACITY: usize, const MTU: usize> Device for QueueDevice<CAPACITY, MTU> {
    type RxToken<'a>
        = QueueRxToken<'a, MTU>
    where
        Self: 'a;
    type TxToken<'a>
        = QueueTxToken<'a, MTU>
    where
        Self: 'a;

    fn receive(
        &mut self,
        _timestamp: Instant,
    ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let transmit = self.egress.reserve().ok()?;
        let receive = self.ingress.dequeue().ok()?;
        Some((QueueRxToken(receive), QueueTxToken(transmit)))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        self.egress.reserve().ok().map(QueueTxToken)
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut capabilities = DeviceCapabilities::default();
        capabilities.max_transmission_unit = MTU;
        capabilities.max_burst_size = Some(CAPACITY);
        capabilities.medium = Medium::Ethernet;
        capabilities
    }
}
