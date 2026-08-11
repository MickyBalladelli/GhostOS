use smoltcp::phy::ChecksumCapabilities;
use smoltcp::wire::{
    EthernetAddress, EthernetFrame, EthernetProtocol, EthernetRepr, IpAddress, IpProtocol,
    Ipv4Address, Ipv4Packet, Ipv4Repr, UdpPacket, UdpRepr, ETHERNET_HEADER_LEN,
    IPV4_HEADER_LEN, UDP_HEADER_LEN,
};

use crate::packet::{PacketError, QueueDevice};
use crate::{
    DhcpError, DhcpTransport, MAX_DHCP_PACKET, MAX_INTERFACE_NAME, DHCP_CLIENT_PORT,
    DHCP_SERVER_PORT,
};

pub const DHCP_BROADCAST_MAC: [u8; 6] = [0xff; 6];
pub const DHCP_BROADCAST_IPV4: [u8; 4] = [255, 255, 255, 255];
pub const DHCP_UNSPECIFIED_IPV4: [u8; 4] = [0, 0, 0, 0];
pub const DHCP_MIN_ETHERNET_FRAME: usize = 60;

pub struct DhcpIngress<'a> {
    pub source_mac: [u8; 6],
    pub source_ip: [u8; 4],
    pub destination_ip: [u8; 4],
    pub payload: &'a [u8],
}

/// Raw DHCP transport over the NIC packet queues.
///
/// This adapter owns no socket state. It emits complete Ethernet II frames
/// and accepts only validated IPv4/UDP DHCP replies for this interface.
pub struct EthernetDhcpTransport<'a, const CAPACITY: usize, const MTU: usize> {
    device: &'a mut QueueDevice<CAPACITY, MTU>,
    interface: [u8; MAX_INTERFACE_NAME],
    interface_len: u8,
    mac: [u8; 6],
}

impl<'a, const CAPACITY: usize, const MTU: usize>
    EthernetDhcpTransport<'a, CAPACITY, MTU>
{
    pub fn new(
        device: &'a mut QueueDevice<CAPACITY, MTU>,
        interface: &str,
        mac: [u8; 6],
    ) -> Result<Self, DhcpError> {
        if interface.is_empty() || interface.len() > MAX_INTERFACE_NAME || mac == [0; 6] {
            return Err(DhcpError::InvalidInterface);
        }
        let mut name = [0; MAX_INTERFACE_NAME];
        name[..interface.len()].copy_from_slice(interface.as_bytes());
        Ok(Self {
            device,
            interface: name,
            interface_len: interface.len() as u8,
            mac,
        })
    }

    pub fn interface_name(&self) -> &str {
        core::str::from_utf8(&self.interface[..self.interface_len as usize]).unwrap_or("")
    }

    pub const fn mac(&self) -> [u8; 6] {
        self.mac
    }

    pub fn receive<F>(&mut self, receive: F) -> Result<bool, DhcpError>
    where
        F: FnOnce(DhcpIngress<'_>) -> Result<(), DhcpError>,
    {
        let mac = self.mac;
        let reader = match self
            .device
            .ingress
            .dequeue_filtered(|frame| parse_frame(frame, mac).is_some())
        {
            Ok(reader) => reader,
            Err(PacketError::Empty) => return Ok(false),
            Err(PacketError::Full | PacketError::FrameTooLarge | PacketError::Capability(_)) => {
                return Err(DhcpError::Network(crate::DhcpNetworkError::BackendUnavailable))
            }
        };
        let ingress = parse_frame(reader.frame(), mac).ok_or(DhcpError::InvalidPacket)?;
        receive(ingress)?;
        Ok(true)
    }

    fn validate_send(
        &self,
        interface: &str,
        src_mac: [u8; 6],
        dst_ip: [u8; 4],
        dst_mac: [u8; 6],
        src_port: u16,
        dst_port: u16,
        payload: &[u8],
    ) -> Result<usize, DhcpError> {
        if interface != self.interface_name() || src_mac != self.mac {
            return Err(DhcpError::InvalidInterface);
        }
        if src_port != DHCP_CLIENT_PORT || dst_port != DHCP_SERVER_PORT {
            return Err(DhcpError::InvalidPacket);
        }
        if dst_mac == [0; 6] || payload.len() > MAX_DHCP_PACKET {
            return Err(DhcpError::InvalidPacket);
        }
        if dst_ip == DHCP_BROADCAST_IPV4 && dst_mac != DHCP_BROADCAST_MAC {
            return Err(DhcpError::InvalidPacket);
        }
        let wire_len = ETHERNET_HEADER_LEN + IPV4_HEADER_LEN + UDP_HEADER_LEN + payload.len();
        let frame_len = wire_len.max(DHCP_MIN_ETHERNET_FRAME);
        if frame_len > MTU {
            return Err(DhcpError::Capacity);
        }
        Ok(frame_len)
    }
}

impl<const CAPACITY: usize, const MTU: usize> DhcpTransport
    for EthernetDhcpTransport<'_, CAPACITY, MTU>
{
    fn send_udp(
        &mut self,
        interface: &str,
        src_mac: [u8; 6],
        src_ip: [u8; 4],
        dst_ip: [u8; 4],
        dst_mac: [u8; 6],
        src_port: u16,
        dst_port: u16,
        payload: &[u8],
    ) -> Result<(), DhcpError> {
        let frame_len = self.validate_send(
            interface, src_mac, dst_ip, dst_mac, src_port, dst_port, payload,
        )?;
        let mut writer = self
            .device
            .egress
            .reserve()
            .map_err(|_| DhcpError::Network(crate::DhcpNetworkError::QueueFull))?;
        {
            let buffer = writer.buffer();
            buffer[..frame_len].fill(0);
            let ethernet_repr = EthernetRepr {
                src_addr: EthernetAddress(src_mac),
                dst_addr: EthernetAddress(dst_mac),
                ethertype: EthernetProtocol::Ipv4,
            };
            let mut ethernet = EthernetFrame::new_unchecked(&mut buffer[..frame_len]);
            ethernet_repr.emit(&mut ethernet);

            let source = Ipv4Address::from_octets(src_ip);
            let destination = Ipv4Address::from_octets(dst_ip);
            let source_ip = IpAddress::Ipv4(source);
            let destination_ip = IpAddress::Ipv4(destination);
            let mut ipv4 = Ipv4Packet::new_unchecked(&mut buffer[ETHERNET_HEADER_LEN..frame_len]);
            Ipv4Repr {
                src_addr: source,
                dst_addr: destination,
                next_header: IpProtocol::Udp,
                payload_len: UDP_HEADER_LEN + payload.len(),
                hop_limit: 64,
            }
            .emit(&mut ipv4, &ChecksumCapabilities::default());
            let mut udp = UdpPacket::new_unchecked(ipv4.payload_mut());
            UdpRepr { src_port, dst_port }.emit(
                &mut udp,
                &source_ip,
                &destination_ip,
                payload.len(),
                |output| output.copy_from_slice(payload),
                &ChecksumCapabilities::default(),
            );
        }
        writer
            .commit(frame_len)
            .map_err(|_| DhcpError::Capacity)
    }
}

fn parse_frame<'a>(frame: &'a [u8], local_mac: [u8; 6]) -> Option<DhcpIngress<'a>> {
    let ethernet = EthernetFrame::new_checked(frame).ok()?;
    let destination_mac = ethernet.dst_addr().0;
    if destination_mac != local_mac && destination_mac != DHCP_BROADCAST_MAC {
        return None;
    }
    if ethernet.ethertype() != EthernetProtocol::Ipv4 {
        return None;
    }
    let ipv4 = Ipv4Packet::new_checked(ethernet.payload()).ok()?;
    let checksum_caps = ChecksumCapabilities::default();
    let ipv4_repr = Ipv4Repr::parse(&ipv4, &checksum_caps).ok()?;
    if ipv4_repr.next_header != IpProtocol::Udp {
        return None;
    }
    let source_ip = IpAddress::Ipv4(ipv4_repr.src_addr);
    let destination_ip = IpAddress::Ipv4(ipv4_repr.dst_addr);
    let udp = UdpPacket::new_checked(ipv4.payload()).ok()?;
    let udp_repr = UdpRepr::parse(&udp, &source_ip, &destination_ip, &checksum_caps).ok()?;
    if udp_repr.src_port != DHCP_SERVER_PORT || udp_repr.dst_port != DHCP_CLIENT_PORT {
        return None;
    }
    Some(DhcpIngress {
        source_mac: ethernet.src_addr().0,
        source_ip: ipv4_repr.src_addr.octets(),
        destination_ip: ipv4_repr.dst_addr.octets(),
        payload: udp.payload(),
    })
}
