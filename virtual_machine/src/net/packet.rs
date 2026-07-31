//! Packet buffer management.

use std::collections::VecDeque;
use std::fmt;

pub const ETHERNET_FRAME_MAX: usize = 1518;
pub const ETHERNET_FRAME_MIN: usize = 60;
pub const ETHERNET_HEADER_LEN: usize = 14;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetError {
    PacketTooLarge,
    Truncated,
    QueueFull,
    LinkDown,
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let msg = match self {
            NetError::PacketTooLarge => "packet exceeds maximum Ethernet frame size",
            NetError::Truncated => "packet too short to contain an Ethernet header",
            NetError::QueueFull => "receive queue full, packet dropped",
            NetError::LinkDown => "network link is down",
        };
        write!(f, "{}", msg)
    }
}

impl std::error::Error for NetError {}

pub struct PacketQueue {
    queue: VecDeque<Vec<u8>>,
    max_packets: usize,
    max_bytes: usize,
    bytes: usize,
}

impl PacketQueue {
    pub fn new(max_packets: usize, max_bytes: usize) -> Self {
        Self { queue: VecDeque::with_capacity(max_packets.min(1024)), max_packets, max_bytes, bytes: 0 }
    }

    pub fn push(&mut self, packet: Vec<u8>) -> bool {
        if packet.len() > self.max_bytes || self.queue.len() >= self.max_packets {
            return false;
        }
        self.bytes += packet.len();
        self.queue.push_back(packet);
        true
    }

    pub fn pop(&mut self) -> Option<Vec<u8>> {
        let p = self.queue.pop_front()?;
        self.bytes -= p.len();
        Some(p)
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn peek(&self) -> Option<&[u8]> {
        self.queue.front().map(|p| p.as_slice())
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn clear(&mut self) {
        self.queue.clear();
        self.bytes = 0;
    }
}

pub fn pad_frame(packet: &[u8]) -> Vec<u8> {
    if packet.len() >= ETHERNET_FRAME_MIN {
        packet.to_vec()
    } else {
        let mut p = packet.to_vec();
        p.resize(ETHERNET_FRAME_MIN, 0);
        p
    }
}