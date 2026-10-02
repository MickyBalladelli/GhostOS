//! Packet backends: the pluggable [`NetBackend`] trait, isolated loopback,
//! and deterministic shared Ethernet segments.

use crate::net::mac::{mac_matches, MacAddress};
use crate::net::packet::{pad_frame, NetError, ETHERNET_FRAME_MAX, ETHERNET_HEADER_LEN};
use crate::net::dhcp::DeterministicVmNetwork;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::rc::Rc;

unsafe extern "C" {
    fn ghostos_vm_net_host_frame_encode(
        packet: *const u8,
        packet_length: usize,
        output: *mut u8,
        output_capacity: usize,
        output_length: *mut usize,
    ) -> bool;
    fn ghostos_vm_net_host_frame_decode(
        frame: *const u8,
        frame_length: usize,
        packet_offset: *mut usize,
        packet_length: *mut usize,
    ) -> bool;
    fn ghostos_vm_net_validate_packet(packet_length: usize) -> u32;
    fn ghostos_vm_net_segment_accepts(
        destination: *const u8,
        length: usize,
        port_mac: *const MacAddress,
        connected: bool,
        admin_up: bool,
    ) -> bool;
}

fn validate_packet(packet: &[u8]) -> Result<(), NetError> {
    match unsafe { ghostos_vm_net_validate_packet(packet.len()) } {
        0 => Ok(()),
        1 => Err(NetError::Truncated),
        _ => Err(NetError::PacketTooLarge),
    }
}

/// Host-facing network selection for a VM.
///
/// Loopback is intentionally not a VM configuration option. It remains
/// available through [`LoopbackHub`] for isolated backend tests only.
#[derive(Clone)]
pub enum NetworkBackendConfig {
    /// Bounded, reproducible Ethernet segment for deterministic VM tests.
    Deterministic,
    /// Shared deterministic VM fixture. The fixture owns the DHCP server and
    /// allocates distinct MAC pairs as VMs attach.
    DeterministicShared {
        network: Rc<RefCell<DeterministicVmNetwork>>,
    },
    /// User-mode networking transport. A host-side gateway receives the
    /// Ethernet frames over UDP and performs the NAT/user-mode service.
    UserNat { bind: SocketAddr, peer: SocketAddr },
    /// Attach the guest directly to a host Ethernet interface.
    Bridged { interface: String },
}

impl Default for NetworkBackendConfig {
    fn default() -> Self {
        Self::Deterministic
    }
}

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

/// Host transport used by user-mode/NAT and bridged VM networking.
pub struct HostNetworkBackend {
    transport: HostTransport,
    mac: MacAddress,
    admin_up: bool,
    promiscuous: bool,
    tx_packets: usize,
    rx_packets: usize,
}

enum HostTransport {
    Udp(UdpSocket),
    #[cfg(target_os = "linux")]
    Raw {
        fd: libc::c_int,
        interface: String,
    },
}

impl HostNetworkBackend {
    /// Open a UDP Ethernet frame transport for a user-mode/NAT gateway.
    pub fn user_nat(bind: SocketAddr, peer: SocketAddr, mac: MacAddress) -> io::Result<Self> {
        let socket = UdpSocket::bind(bind)?;
        socket.connect(peer)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            transport: HostTransport::Udp(socket),
            mac,
            admin_up: true,
            promiscuous: false,
            tx_packets: 0,
            rx_packets: 0,
        })
    }

    /// Open a host Ethernet interface for bridged networking.
    #[cfg(target_os = "linux")]
    pub fn bridged(interface: &str, mac: MacAddress) -> io::Result<Self> {
        use std::ffi::CString;
        use std::mem;
        let name = CString::new(interface.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid interface name"))?;
        let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
        if index == 0 {
            return Err(io::Error::last_os_error());
        }
        let protocol = (libc::ETH_P_ALL as u16).to_be() as i32;
        let fd = unsafe {
            libc::socket(
                libc::AF_PACKET,
                libc::SOCK_RAW | libc::SOCK_NONBLOCK,
                protocol,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut address: libc::sockaddr_ll = unsafe { mem::zeroed() };
        address.sll_family = libc::AF_PACKET as u16;
        address.sll_protocol = (libc::ETH_P_ALL as u16).to_be();
        address.sll_ifindex = index as i32;
        let result = unsafe {
            libc::bind(
                fd,
                (&address as *const libc::sockaddr_ll).cast(),
                mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(error);
        }

        Ok(Self {
            transport: HostTransport::Raw {
                fd,
                interface: interface.to_string(),
            },
            mac,
            admin_up: true,
            promiscuous: false,
            tx_packets: 0,
            rx_packets: 0,
        })
    }

    #[cfg(not(target_os = "linux"))]
    pub fn bridged(_interface: &str, _mac: MacAddress) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "bridged Ethernet backend requires Linux AF_PACKET support",
        ))
    }

    fn transport_link_up(&self) -> bool {
        match &self.transport {
            HostTransport::Udp(_) => true,
            #[cfg(target_os = "linux")]
            HostTransport::Raw { interface, .. } => {
                let carrier = std::fs::read_to_string(format!(
                    "/sys/class/net/{interface}/carrier"
                ));
                carrier
                    .map(|state| state.trim() == "1")
                    .unwrap_or_else(|_| {
                        std::fs::read_to_string(format!(
                            "/sys/class/net/{interface}/operstate"
                        ))
                        .map(|state| state.trim() != "down")
                        .unwrap_or(false)
                    })
            }
        }
    }

    fn transmit_udp(socket: &UdpSocket, packet: &[u8]) -> Result<(), NetError> {
        let mut frame_length = 0;
        if !unsafe {
            ghostos_vm_net_host_frame_encode(
                packet.as_ptr(), packet.len(), std::ptr::null_mut(), 0, &mut frame_length,
            )
        } {
            return Err(NetError::BackendUnavailable);
        }
        let mut frame = vec![0; frame_length];
        if !unsafe {
            ghostos_vm_net_host_frame_encode(
                packet.as_ptr(), packet.len(), frame.as_mut_ptr(), frame.len(), &mut frame_length,
            )
        } {
            return Err(NetError::BackendUnavailable);
        }
        socket.send(&frame).map_err(|error| match error.kind() {
            io::ErrorKind::WouldBlock => NetError::QueueFull,
            _ => NetError::BackendUnavailable,
        })?;
        Ok(())
    }
}

impl Drop for HostNetworkBackend {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if let HostTransport::Raw { fd, .. } = &self.transport {
            unsafe { libc::close(*fd) };
        }
    }
}

impl NetBackend for HostNetworkBackend {
    fn transmit(&mut self, packet: &[u8]) -> Result<(), NetError> {
        if !self.admin_up {
            return Err(NetError::AdminDown);
        }
        if !self.link_up() {
            return Err(NetError::LinkDown);
        }
        validate_packet(packet)?;
        match &self.transport {
            HostTransport::Udp(socket) => Self::transmit_udp(socket, packet)?,
            #[cfg(target_os = "linux")]
            HostTransport::Raw { fd, .. } => {
                let frame = pad_frame(packet);
                let sent = unsafe {
                    libc::send(fd, frame.as_ptr().cast(), frame.len(), libc::MSG_DONTWAIT)
                };
                if sent < 0 {
                    return Err(match io::Error::last_os_error().kind() {
                        io::ErrorKind::WouldBlock => NetError::QueueFull,
                        _ => NetError::BackendUnavailable,
                    });
                }
                if sent as usize != frame.len() {
                    return Err(NetError::BackendUnavailable);
                }
            }
        }
        self.tx_packets = self.tx_packets.saturating_add(1);
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        if !self.admin_up {
            return Err(NetError::AdminDown);
        }
        if !self.link_up() {
            return Err(NetError::LinkDown);
        }
        let packet = match &self.transport {
            HostTransport::Udp(socket) => {
                let mut frame = [0u8; 4 + ETHERNET_FRAME_MAX];
                match socket.recv(&mut frame) {
                    Ok(length) if length >= 4 => {
                        let mut packet_offset = 0;
                        let mut packet_length = 0;
                        if !unsafe {
                            ghostos_vm_net_host_frame_decode(
                                frame.as_ptr(), length, &mut packet_offset, &mut packet_length,
                            )
                        } {
                            return Ok(None);
                        }
                        frame[packet_offset..packet_offset + packet_length].to_vec()
                    }
                    Ok(_) => return Ok(None),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                    Err(_) => return Err(NetError::BackendUnavailable),
                }
            }
            #[cfg(target_os = "linux")]
            HostTransport::Raw { fd, .. } => {
                let mut frame = [0u8; ETHERNET_FRAME_MAX];
                let length = unsafe {
                    libc::recv(fd, frame.as_mut_ptr().cast(), frame.len(), libc::MSG_DONTWAIT)
                };
                if length < 0 {
                    if io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock {
                        return Ok(None);
                    }
                    return Err(NetError::BackendUnavailable);
                }
                frame[..length as usize].to_vec()
            }
        };
        if packet.len() < ETHERNET_HEADER_LEN {
            return Ok(None);
        }
        if !mac_matches(&packet[..6], &self.mac, self.promiscuous) {
            return Ok(None);
        }
        self.rx_packets = self.rx_packets.saturating_add(1);
        Ok(Some(packet))
    }

    fn link_up(&self) -> bool {
        self.transport_link_up()
    }

    fn admin_up(&self) -> bool {
        self.admin_up
    }

    fn set_admin_up(&mut self, up: bool) {
        self.admin_up = up;
    }

    fn queue_state(&self) -> NetQueueState {
        NetQueueState {
            rx_packets: 0,
            tx_packets: self.tx_packets,
        }
    }

    fn set_promiscuous(&mut self, enabled: bool) {
        self.promiscuous = enabled;
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
    connected: bool,
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
    drop_next: usize,
}

impl DeterministicSegment {
    pub fn new(max_ports: usize) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            ports: Vec::with_capacity(max_ports),
            up: true,
            max_queue: 256,
            tx_packets: 0,
            drop_next: 0,
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

    /// Bound the per-port queue for saturation and backpressure tests.
    pub fn set_max_queue(&mut self, max_queue: usize) {
        self.max_queue = max_queue;
        for port in &mut self.ports {
            while port.queue.len() > max_queue {
                port.queue.pop_front();
            }
        }
    }

    /// Drop the next `count` frames after validation, without making loss
    /// look like a backend failure.
    pub fn drop_next(&mut self, count: usize) {
        self.drop_next = count;
    }

    /// Disconnect one port while keeping its slot and MAC identity stable.
    pub fn disconnect(&mut self, mac: MacAddress) -> bool {
        let Some(port) = self.ports.iter_mut().find(|port| port.mac == mac) else {
            return false;
        };
        port.connected = false;
        port.queue.clear();
        true
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
            connected: true,
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
        if !source.connected {
            return Err(NetError::BackendUnavailable);
        }
        if !source.admin_up {
            return Err(NetError::AdminDown);
        }
        let recipients: Vec<usize> = self
            .ports
            .iter()
            .enumerate()
            .filter(|(index, port)| {
                *index != from
                    && unsafe {
                        ghostos_vm_net_segment_accepts(
                            packet.as_ptr(), packet.len(), &port.mac, port.connected, port.admin_up,
                        )
                    }
            })
            .map(|(index, _)| index)
            .collect();
        if recipients
            .iter()
            .any(|index| self.ports[*index].queue.len() >= self.max_queue)
        {
            return Err(NetError::QueueFull);
        }
        if self.drop_next > 0 {
            self.drop_next -= 1;
            self.tx_packets = self.tx_packets.saturating_add(1);
            return Ok(())
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
        if !port.connected {
            return Err(NetError::BackendUnavailable);
        }
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
        let first_mac = MacAddress::ghostos_default(0x60);
        let second_mac = MacAddress::ghostos_default(0x61);
        let third_mac = MacAddress::ghostos_default(0x62);
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
        let mac = MacAddress::ghostos_default(0x63);
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
