//! Packet backends: the pluggable [`NetBackend`] trait and a two-port
//! loopback hub connecting the emulated NICs.

use crate::net::mac::{mac_matches, MacAddress};
use crate::net::packet::{pad_frame, NetError, ETHERNET_FRAME_MAX, ETHERNET_HEADER_LEN};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub trait NetBackend {
    fn transmit(&mut self, packet: &[u8]) -> Result<(), NetError>;
    fn receive(&mut self) -> Result<Option<Vec<u8>>, NetError>;
    fn link_up(&self) -> bool;
    fn set_promiscuous(&mut self, enabled: bool) {
        let _ = enabled;
    }
}

pub struct LoopbackHub {
    queues: [VecDeque<Vec<u8>>; 2],
    up: bool,
}

impl LoopbackHub {
    pub fn new() -> Self {
        Self {
            queues: [VecDeque::new(), VecDeque::new()],
            up: true,
        }
    }

    /// Enable or disable delivery for both ports.
    pub fn set_link_up(&mut self, up: bool) {
        self.up = up
    }

    pub fn link_up(&self) -> bool {
        self.up
    }

    /// Return the number of frames waiting for a port.
    pub fn queued_packets(&self, port: usize) -> usize {
        self.queues.get(port).map_or(0, VecDeque::len)
    }

    pub fn clear(&mut self) {
        for queue in &mut self.queues {
            queue.clear()
        }
    }

    fn deliver(&mut self, from: usize, packet: &[u8]) -> Result<(), NetError> {
        if packet.len() > ETHERNET_FRAME_MAX {
            return Err(NetError::PacketTooLarge);
        }
        let to = 1 - from;
        if self.queues[to].len() >= 256 {
            return Err(NetError::QueueFull);
        }
        self.queues[to].push_back(pad_frame(packet));
        Ok(())
    }
}

impl Default for LoopbackHub {
    fn default() -> Self {
        Self::new()
    }
}

pub struct LoopbackPort {
    hub: Rc<RefCell<LoopbackHub>>,
    index: usize,
    mac: MacAddress,
    promiscuous: bool,
}

impl LoopbackPort {
    pub fn new(hub: Rc<RefCell<LoopbackHub>>, index: usize, mac: MacAddress) -> Self {
        assert!(index < 2, "loopback hub has exactly two ports");
        Self {
            hub,
            index,
            mac,
            promiscuous: false,
        }
    }

    pub fn mac(&self) -> MacAddress {
        self.mac
    }
}

impl NetBackend for LoopbackPort {
    fn transmit(&mut self, packet: &[u8]) -> Result<(), NetError> {
        if !self.hub.borrow().up {
            return Err(NetError::LinkDown);
        }
        if packet.len() < ETHERNET_HEADER_LEN {
            return Err(NetError::Truncated);
        }
        self.hub.borrow_mut().deliver(self.index, packet)
    }

    fn receive(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        if !self.hub.borrow().up {
            return Err(NetError::LinkDown);
        }
        let mut hub = self.hub.borrow_mut();
        while let Some(packet) = hub.queues[self.index].pop_front() {
            if packet.len() >= ETHERNET_HEADER_LEN
                && mac_matches(&packet[..6], &self.mac, self.promiscuous)
            {
                return Ok(Some(packet));
            }
        }
        Ok(None)
    }

    fn link_up(&self) -> bool {
        self.hub.borrow().up
    }

    fn set_promiscuous(&mut self, enabled: bool) {
        self.promiscuous = enabled;
    }
}
