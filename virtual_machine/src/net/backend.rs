//! Packet backends: the pluggable [`NetBackend`] trait, isolated loopback,
//! and deterministic shared Ethernet segments.

use crate::net::mac::{mac_matches, MacAddress};
use crate::net::packet::{pad_frame, NetError, ETHERNET_FRAME_MAX, ETHERNET_HEADER_LEN};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub trait NetBackend {
    fn transmit(&mut self, packet: &[u8]) -> Result<(), NetError>;
    fn receive(&mut self) -> Result<Option<Vec<u8>>, NetError>;
    fn link_up(&self) -> bool;
    fn admin_up(&self) -> bool {
        true
    }
    fn set_admin_up(&mut self, _up: bool) {}
    fn queue_state(&self) -> NetQueueState {
        NetQueueState::EMPTY
    }
    fn set_promiscuous(&mut self, enabled: bool) {
        let _ = enabled;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetQueueState {
    pub rx_packets: usize,
    pub tx_packets: usize,
}

impl NetQueueState {
    pub const EMPTY: Self = Self {
        rx_packets: 0,
        tx_packets: 0,
    };
}

pub struct LoopbackHub {
    queues: [VecDeque<Vec<u8>>; 2],
    up: bool,
    tx_packets: usize,
}

impl LoopbackHub {
    pub fn new() -> Self {
        Self {
            queues: [VecDeque::new(), VecDeque::new()],
            up: true,
            tx_packets: 0,
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
        self.tx_packets = self.tx_packets.saturating_add(1);
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
    admin_up: bool,
}

impl LoopbackPort {
    pub fn new(hub: Rc<RefCell<LoopbackHub>>, index: usize, mac: MacAddress) -> Self {
        assert!(index < 2, "loopback hub has exactly two ports");
        Self {
            hub,
            index,
            mac,
            promiscuous: false,
            admin_up: true,
        }
    }

    pub fn mac(&self) -> MacAddress {
        self.mac
    }
}

impl NetBackend for LoopbackPort {
    fn transmit(&mut self, packet: &[u8]) -> Result<(), NetError> {
        if !self.admin_up {
            return Err(NetError::AdminDown);
        }
        if !self.hub.borrow().up {
            return Err(NetError::LinkDown);
        }
        if packet.len() < ETHERNET_HEADER_LEN {
            return Err(NetError::Truncated);
        }
        self.hub.borrow_mut().deliver(self.index, packet)
    }

    fn receive(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        if !self.admin_up {
            return Err(NetError::AdminDown);
        }
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

    fn admin_up(&self) -> bool {
        self.admin_up
    }

    fn set_admin_up(&mut self, up: bool) {
        self.admin_up = up;
    }

    fn queue_state(&self) -> NetQueueState {
        let hub = self.hub.borrow();
        NetQueueState {
            rx_packets: hub.queues[self.index].len(),
            tx_packets: hub.tx_packets,
        }
    }

    fn set_promiscuous(&mut self, enabled: bool) {
        self.promiscuous = enabled;
    }
}

struct SegmentPort {
    mac: MacAddress,
    queue: VecDeque<Vec<u8>>,
    admin_up: bool,
    promiscuous: bool,
}

/// Deterministic, bounded shared L2 segment for VM integration tests.
///
/// The segment has one physical carrier shared by all ports. Frames are
/// delivered to every other port for broadcast/multicast and to the matching
/// port for unicast. A segment with no peer accepts transmission and drops
/// the frame, matching an unattached cable without inventing a peer.
pub struct DeterministicSegment {
    ports: Vec<SegmentPort>,
    up: bool,
    max_queue: usize,
    tx_packets: usize,
}

impl DeterministicSegment {
    pub fn new(max_ports: usize) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            ports: Vec::with_capacity(max_ports),
            up: true,
            max_queue: 256,
            tx_packets: 0,
        }))
    }

    pub fn set_link_up(&mut self, up: bool) {
        self.up = up;
        if !up {
            for port in &mut self.ports {
                port.queue.clear();
            }
        }
    }

    pub fn link_up(&self) -> bool {
        self.up
    }

    pub fn connect(
        segment: Rc<RefCell<Self>>,
        mac: MacAddress,
    ) -> Result<DeterministicPort, NetError> {
        let mut segment_ref = segment.borrow_mut();
        if segment_ref.ports.iter().any(|port| port.mac == mac) {
            return Err(NetError::BackendUnavailable);
        }
        let index = segment_ref.ports.len();
        if index == segment_ref.ports.capacity() {
            return Err(NetError::BackendUnavailable);
        }
        segment_ref.ports.push(SegmentPort {
            mac,
            queue: VecDeque::new(),
            admin_up: true,
            promiscuous: false,
        });
        drop(segment_ref);
        Ok(DeterministicPort {
            segment,
            index,
            mac,
        })
    }

    pub fn queued_packets(&self, port: usize) -> usize {
        self.ports.get(port).map_or(0, |port| port.queue.len())
    }

    fn transmit(&mut self, from: usize, packet: &[u8]) -> Result<(), NetError> {
        if !self.up {
            return Err(NetError::LinkDown);
        }
        if packet.len() < ETHERNET_HEADER_LEN {
            return Err(NetError::Truncated);
        }
        if packet.len() > ETHERNET_FRAME_MAX {
            return Err(NetError::PacketTooLarge);
        }
        let Some(source) = self.ports.get(from) else {
            return Err(NetError::BackendUnavailable);
        };
        if !source.admin_up {
            return Err(NetError::AdminDown);
        }
        let destination = MacAddress::from_bytes(&packet[..6]).ok_or(NetError::Truncated)?;
        let recipients: Vec<usize> = self
            .ports
            .iter()
            .enumerate()
            .filter(|(index, port)| {
                *index != from
                    && port.admin_up
                    && (destination.is_broadcast()
                        || destination.is_multicast()
                        || port.mac == destination)
            })
            .map(|(index, _)| index)
            .collect();
        if recipients
            .iter()
            .any(|index| self.ports[*index].queue.len() >= self.max_queue)
        {
            return Err(NetError::QueueFull);
        }
        let frame = pad_frame(packet);
        for index in recipients {
            self.ports[index].queue.push_back(frame.clone());
        }
        self.tx_packets = self.tx_packets.saturating_add(1);
        Ok(())
    }
}

pub struct DeterministicPort {
    segment: Rc<RefCell<DeterministicSegment>>,
    index: usize,
    mac: MacAddress,
}

impl DeterministicPort {
    pub fn mac(&self) -> MacAddress {
        self.mac
    }
}

impl NetBackend for DeterministicPort {
    fn transmit(&mut self, packet: &[u8]) -> Result<(), NetError> {
        self.segment.borrow_mut().transmit(self.index, packet)
    }

    fn receive(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        let mut segment = self.segment.borrow_mut();
        if !segment.up {
            return Err(NetError::LinkDown);
        }
        let port = segment
            .ports
            .get_mut(self.index)
            .ok_or(NetError::BackendUnavailable)?;
        if !port.admin_up {
            return Err(NetError::AdminDown);
        }
        while let Some(packet) = port.queue.pop_front() {
            if packet.len() >= ETHERNET_HEADER_LEN
                && mac_matches(&packet[..6], &self.mac, port.promiscuous)
            {
                return Ok(Some(packet));
            }
        }
        Ok(None)
    }

    fn link_up(&self) -> bool {
        self.segment.borrow().up
    }

    fn admin_up(&self) -> bool {
        self.segment
            .borrow()
            .ports
            .get(self.index)
            .is_some_and(|port| port.admin_up)
    }

    fn set_admin_up(&mut self, up: bool) {
        if let Some(port) = self.segment.borrow_mut().ports.get_mut(self.index) {
            port.admin_up = up;
            if !up {
                port.queue.clear();
            }
        }
    }

    fn queue_state(&self) -> NetQueueState {
        let segment = self.segment.borrow();
        NetQueueState {
            rx_packets: segment.queued_packets(self.index),
            tx_packets: segment.tx_packets,
        }
    }

    fn set_promiscuous(&mut self, enabled: bool) {
        if let Some(port) = self.segment.borrow_mut().ports.get_mut(self.index) {
            port.promiscuous = enabled;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(destination: MacAddress, source: MacAddress) -> Vec<u8> {
        let mut frame = vec![0; ETHERNET_HEADER_LEN];
        frame[..6].copy_from_slice(&destination.to_bytes());
        frame[6..12].copy_from_slice(&source.to_bytes());
        frame
    }

    #[test]
    fn shared_segment_delivers_unicast_and_broadcast() {
        let segment = DeterministicSegment::new(3);
        let first_mac = MacAddress::synos_default(0x60);
        let second_mac = MacAddress::synos_default(0x61);
        let third_mac = MacAddress::synos_default(0x62);
        let mut first = DeterministicSegment::connect(segment.clone(), first_mac).unwrap();
        let mut second = DeterministicSegment::connect(segment.clone(), second_mac).unwrap();
        let mut third = DeterministicSegment::connect(segment, third_mac).unwrap();

        first.transmit(&frame(second_mac, first_mac)).unwrap();
        assert!(second.receive().unwrap().is_some());
        assert!(third.receive().unwrap().is_none());

        first.transmit(&frame(MacAddress::BROADCAST, first_mac)).unwrap();
        assert!(second.receive().unwrap().is_some());
        assert!(third.receive().unwrap().is_some());
    }

    #[test]
    fn administrative_state_is_distinct_from_carrier_state() {
        let segment = DeterministicSegment::new(1);
        let mac = MacAddress::synos_default(0x63);
        let mut port = DeterministicSegment::connect(segment.clone(), mac).unwrap();
        assert!(port.link_up());
        port.set_admin_up(false);
        assert!(port.link_up());
        assert!(!port.admin_up());
        assert_eq!(port.transmit(&frame(MacAddress::BROADCAST, mac)), Err(NetError::AdminDown));
        segment.borrow_mut().set_link_up(false);
        assert!(!port.link_up());
    }
}
