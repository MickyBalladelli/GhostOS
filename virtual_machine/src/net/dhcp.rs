//! Bounded DHCP server attached to the deterministic Ethernet segment.

use super::backend::{DeterministicPort, DeterministicSegment, NetBackend};
use super::mac::MacAddress;
use super::packet::NetError;
#[cfg(test)]
use super::packet::{pad_frame, ETHERNET_HEADER_LEN};
use std::cell::RefCell;
use std::fmt;
use std::ffi::c_void;
use std::rc::Rc;

unsafe extern "C" {
    fn ghostos_vm_dhcp_server_new(config: *const CConfig, reservations: *const CReservation, count: usize) -> *mut c_void;
    fn ghostos_vm_dhcp_server_free(server: *mut c_void);
    fn ghostos_vm_dhcp_server_expire(server: *mut c_void, now_ms: u64);
    fn ghostos_vm_dhcp_server_leases(server: *const c_void, leases: *mut CLease, capacity: usize) -> usize;
    fn ghostos_vm_dhcp_server_handle(server: *mut c_void, request: *const CDhcpRequest, now_ms: u64, address: *mut u8) -> u8;

    fn ghostos_vm_dhcp_validate_config(config: *const CConfig, reservations: *const CReservation, count: usize) -> u32;
    fn ghostos_vm_dhcp_encode_reply(config: *const CConfig, request: *const CDhcpRequest,
        message_type: u8, address: *const u8, output: *mut u8, capacity: usize, length: *mut usize) -> bool;
    fn ghostos_vm_dhcp_decode_request(
        frame: *const u8,
        frame_length: usize,
        request: *mut CDhcpRequest,
    ) -> bool;
    #[cfg(test)]
    fn ghostos_vm_dhcp_write_option(
        output: *mut u8,
        output_capacity: usize,
        cursor: usize,
        code: u8,
        value: *const u8,
        value_length: usize,
        output_cursor: *mut usize,
    ) -> bool;
}

#[repr(C)]
#[derive(Default)]
struct CDhcpRequest {
    source_mac: [u8; 6],
    mac: [u8; 6],
    xid: u32,
    flags: u16,
    ciaddr: [u8; 4],
    message_type: u8,
    requested_ip_present: u8,
    requested_ip: [u8; 4],
    server_id_present: u8,
    server_id: [u8; 4],
}

const _: [(); 36] = [(); std::mem::size_of::<CDhcpRequest>()];

#[repr(C)]
struct CConfig {
    server_ip: [u8; 4], pool_start: [u8; 4], pool_end: [u8; 4],
    subnet_mask: [u8; 4], gateway: [u8; 4], dns: [u8; 4], lease_time_secs: u32,
}
impl From<&DhcpServerConfig> for CConfig {
    fn from(config: &DhcpServerConfig) -> Self {
        Self { server_ip: config.server_ip, pool_start: config.pool_start, pool_end: config.pool_end,
            subnet_mask: config.subnet_mask, gateway: config.gateway, dns: config.dns,
            lease_time_secs: config.lease_time_secs }
    }
}
#[repr(C)]
struct CReservation { mac: [u8; 6], address: [u8; 4] }

pub const DHCP_SERVER_MAC: MacAddress = MacAddress::ghostos_default(0xD0);
pub const DHCP_SERVER_PORT: u16 = 67;
pub const DHCP_CLIENT_PORT: u16 = 68;
pub const MAX_DHCP_SERVER_LEASES: usize = 256;
pub const MAX_DHCP_SERVER_RESERVATIONS: usize = 32;
pub const MAX_DHCP_SERVER_PACKETS_PER_POLL: usize = 32;
pub const DHCP_MAGIC_COOKIE: u32 = 0x6382_5363;

#[cfg(test)]
const DHCP_DISCOVER: u8 = 1;
#[cfg(test)]
const DHCP_REQUEST: u8 = 3;
#[cfg(test)]
const DHCP_OPTION_MESSAGE_TYPE: u8 = 53;
#[cfg(test)]
const DHCP_OPTION_REQUESTED_IP: u8 = 50;
#[cfg(test)]
const DHCP_OPTION_SERVER_ID: u8 = 54;
#[cfg(test)]
const DHCP_OPTION_END: u8 = 255;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DhcpConfigError {
    InvalidPool,
    PoolTooLarge,
    InvalidLeaseDuration,
    TooManyReservations,
    DuplicateReservation,
    ReservationOutsidePool,
    SegmentUnavailable,
}

impl fmt::Display for DhcpConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidPool => "invalid DHCP address pool",
            Self::PoolTooLarge => "DHCP address pool is too large",
            Self::InvalidLeaseDuration => "DHCP lease duration must be at least three seconds",
            Self::TooManyReservations => "too many DHCP reservations",
            Self::DuplicateReservation => "duplicate DHCP reservation",
            Self::ReservationOutsidePool => "DHCP reservation is outside the address pool",
            Self::SegmentUnavailable => "DHCP server cannot attach to the segment",
        };
        f.write_str(message)
    }
}

impl std::error::Error for DhcpConfigError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DhcpReservation {
    pub mac: MacAddress,
    pub address: [u8; 4],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DhcpServerConfig {
    pub server_ip: [u8; 4],
    pub pool_start: [u8; 4],
    pub pool_end: [u8; 4],
    pub subnet_mask: [u8; 4],
    pub gateway: [u8; 4],
    pub dns: [u8; 4],
    pub lease_time_secs: u32,
    pub reservations: Vec<DhcpReservation>,
}

impl Default for DhcpServerConfig {
    fn default() -> Self {
        Self {
            server_ip: [10, 5, 0, 1],
            pool_start: [10, 5, 0, 100],
            pool_end: [10, 5, 0, 199],
            subnet_mask: [255, 255, 255, 0],
            gateway: [10, 5, 0, 1],
            dns: [10, 5, 0, 1],
            lease_time_secs: 3_600,
            reservations: Vec::new(),
        }
    }
}

impl DhcpServerConfig {
    pub fn validate(&self) -> Result<(), DhcpConfigError> {
        let config = CConfig::from(self);
        let reservations: Vec<CReservation> = self.reservations.iter()
            .map(|reservation| CReservation { mac: reservation.mac.to_bytes(), address: reservation.address }).collect();
        match unsafe { ghostos_vm_dhcp_validate_config(&config, reservations.as_ptr(), reservations.len()) } {
            0 => Ok(()),
            1 => Err(DhcpConfigError::InvalidPool),
            2 => Err(DhcpConfigError::PoolTooLarge),
            3 => Err(DhcpConfigError::InvalidLeaseDuration),
            4 => Err(DhcpConfigError::TooManyReservations),
            5 => Err(DhcpConfigError::DuplicateReservation),
            6 => Err(DhcpConfigError::ReservationOutsidePool),
            _ => unreachable!("invalid C DHCP configuration result"),
        }
    }

    pub fn add_reservation(
        &mut self,
        reservation: DhcpReservation,
    ) -> Result<(), DhcpConfigError> {
        if self.reservations.len() >= MAX_DHCP_SERVER_RESERVATIONS {
            return Err(DhcpConfigError::TooManyReservations);
        }
        self.reservations.push(reservation);
        if let Err(error) = self.validate() {
            self.reservations.pop();
            return Err(error);
        }
        Ok(())
    }


}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DhcpLeaseInfo {
    pub mac: MacAddress,
    pub address: [u8; 4],
    pub expires_at_ms: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CLease { mac: [u8; 6], address: [u8; 4], expires_at_ms: u64 }
const _: [(); 24] = [(); std::mem::size_of::<CLease>()];

/// DHCP server running as a synthetic port on a deterministic segment.
pub struct DeterministicDhcpServer {
    port: DeterministicPort,
    config: DhcpServerConfig,
    state: *mut c_void,
}

/// Shared bounded Ethernet fixture for multiple VMs.
///
/// The segment is EtherType agnostic. Ethernet frames carrying ARP, IPv4,
/// ICMP, UDP, or TCP therefore use the same deterministic delivery path.
pub struct DeterministicVmNetwork {
    segment: Rc<RefCell<DeterministicSegment>>,
    dhcp_server: Option<DeterministicDhcpServer>,
    attached_vms: usize,
}

impl DeterministicVmNetwork {
    pub fn new(
        dhcp_config: Option<DhcpServerConfig>,
    ) -> Result<Rc<RefCell<Self>>, DhcpConfigError> {
        let segment = DeterministicSegment::new(2_048);
        let dhcp_server = dhcp_config
            .map(|config| DeterministicDhcpServer::new(segment.clone(), config))
            .transpose()?;
        Ok(Rc::new(RefCell::new(Self {
            segment,
            dhcp_server,
            attached_vms: 0,
        })))
    }

    pub fn segment(&self) -> Rc<RefCell<DeterministicSegment>> {
        self.segment.clone()
    }

    pub fn attach_vm(
        &mut self,
    ) -> Result<
        (
            MacAddress,
            DeterministicPort,
            MacAddress,
            DeterministicPort,
        ),
        NetError,
    > {
        let slot = self.attached_vms;
        let base = u16::try_from(
            slot.checked_mul(2)
                .ok_or(NetError::BackendUnavailable)?,
        )
        .map_err(|_| NetError::BackendUnavailable)?;
        let e1000_mac = cluster_mac(base);
        let virtio_mac = cluster_mac(base.saturating_add(1));
        let e1000 = DeterministicSegment::connect(self.segment.clone(), e1000_mac)?;
        let virtio = DeterministicSegment::connect(self.segment.clone(), virtio_mac)?;
        self.attached_vms = self.attached_vms.saturating_add(1);
        Ok((e1000_mac, e1000, virtio_mac, virtio))
    }

    pub fn poll(&mut self, now_ms: u64) -> Result<usize, NetError> {
        self.dhcp_server
            .as_mut()
            .map_or(Ok(0), |server| server.poll(now_ms))
    }

    pub fn dhcp_server(&self) -> Option<&DeterministicDhcpServer> {
        self.dhcp_server.as_ref()
    }

    pub fn dhcp_server_mut(&mut self) -> Option<&mut DeterministicDhcpServer> {
        self.dhcp_server.as_mut()
    }
}

fn cluster_mac(slot: u16) -> MacAddress {
    MacAddress::new([0x52, 0x54, 0x00, 0x56, (slot >> 8) as u8, slot as u8])
}

impl DeterministicDhcpServer {
    pub fn new(
        segment: Rc<RefCell<DeterministicSegment>>,
        config: DhcpServerConfig,
    ) -> Result<Self, DhcpConfigError> {
        config.validate()?;
        let port = DeterministicSegment::connect(segment, DHCP_SERVER_MAC)
            .map_err(|_| DhcpConfigError::SegmentUnavailable)?;
        let c_config = CConfig::from(&config);
        let reservations: Vec<CReservation> = config.reservations.iter()
            .map(|reservation| CReservation { mac: reservation.mac.to_bytes(), address: reservation.address }).collect();
        let state = unsafe { ghostos_vm_dhcp_server_new(&c_config, reservations.as_ptr(), reservations.len()) };
        assert!(!state.is_null(), "C DHCP server allocation failed");
        Ok(Self { port, config, state })
    }

    pub fn config(&self) -> &DhcpServerConfig {
        &self.config
    }

    pub fn leases(&self) -> Vec<DhcpLeaseInfo> {
        let mut leases = [CLease::default(); MAX_DHCP_SERVER_LEASES];
        let count = unsafe { ghostos_vm_dhcp_server_leases(self.state, leases.as_mut_ptr(), leases.len()) };
        leases[..count].iter().map(|lease| DhcpLeaseInfo {
            mac: MacAddress(lease.mac), address: lease.address, expires_at_ms: lease.expires_at_ms,
        }).collect()
    }

    pub fn lease_for(&self, mac: MacAddress) -> Option<DhcpLeaseInfo> {
        self.leases().into_iter().find(|lease| lease.mac == mac)
    }

    pub fn poll(&mut self, now_ms: u64) -> Result<usize, NetError> {
        unsafe { ghostos_vm_dhcp_server_expire(self.state, now_ms) };
        let mut handled = 0;
        for _ in 0..MAX_DHCP_SERVER_PACKETS_PER_POLL {
            let Some(frame) = self.port.receive()? else {
                break;
            };
            if let Some(reply) = self.handle_frame(&frame, now_ms) {
                self.port.transmit(&reply)?;
                handled += 1;
            }
        }
        Ok(handled)
    }

    fn handle_frame(&mut self, frame: &[u8], now_ms: u64) -> Option<Vec<u8>> {
        let mut request = CDhcpRequest::default();
        if !unsafe { ghostos_vm_dhcp_decode_request(frame.as_ptr(), frame.len(), &mut request) } {
            return None;
        }
        let mut address = [0; 4];
        let message_type = unsafe { ghostos_vm_dhcp_server_handle(self.state, &request, now_ms, address.as_mut_ptr()) };
        (message_type != 0).then(|| encode_reply(&self.config, request, message_type, address))
    }
}

impl Drop for DeterministicDhcpServer {
    fn drop(&mut self) { unsafe { ghostos_vm_dhcp_server_free(self.state) }; }
}

fn encode_reply(
    config: &DhcpServerConfig,
    request: CDhcpRequest,
    message_type: u8,
    address: [u8; 4],
) -> Vec<u8> {
    let config = CConfig::from(config);
    let mut output = [0; 618];
    let mut length = 0;
    assert!(unsafe { ghostos_vm_dhcp_encode_reply(&config, &request, message_type,
        address.as_ptr(), output.as_mut_ptr(), output.len(), &mut length) });
    output[..length].to_vec()
}

#[cfg(test)]
fn write_option(output: &mut [u8], cursor: usize, code: u8, value: &[u8]) -> usize {
    let mut output_cursor = 0;
    assert!(unsafe {
        ghostos_vm_dhcp_write_option(
            output.as_mut_ptr(), output.len(), cursor, code, value.as_ptr(), value.len(),
            &mut output_cursor,
        )
    });
    output_cursor
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dhcp_packet(
        mac: MacAddress,
        xid: u32,
        message_type: u8,
        requested_ip: Option<[u8; 4]>,
        server_id: Option<[u8; 4]>,
    ) -> Vec<u8> {
        let mut payload = [0u8; 576];
        payload[0] = 1;
        payload[1] = 1;
        payload[2] = 6;
        payload[4..8].copy_from_slice(&xid.to_be_bytes());
        payload[10..12].copy_from_slice(&0x8000u16.to_be_bytes());
        payload[28..34].copy_from_slice(&mac.to_bytes());
        payload[236..240].copy_from_slice(&DHCP_MAGIC_COOKIE.to_be_bytes());
        let mut cursor = 240;
        cursor = write_option(
            &mut payload,
            cursor,
            DHCP_OPTION_MESSAGE_TYPE,
            &[message_type],
        );
        if let Some(address) = requested_ip {
            cursor = write_option(&mut payload, cursor, DHCP_OPTION_REQUESTED_IP, &address);
        }
        if let Some(server) = server_id {
            cursor = write_option(&mut payload, cursor, DHCP_OPTION_SERVER_ID, &server);
        }
        payload[cursor] = DHCP_OPTION_END;
        let payload_len = cursor + 1;
        let ip_len = 20 + 8 + payload_len;
        let mut frame = vec![0u8; ETHERNET_HEADER_LEN + ip_len];
        frame[..6].fill(0xFF);
        frame[6..12].copy_from_slice(&mac.to_bytes());
        frame[12..14].copy_from_slice(&[0x08, 0x00]);
        let ip = ETHERNET_HEADER_LEN;
        frame[ip] = 0x45;
        frame[ip + 2..ip + 4].copy_from_slice(&(ip_len as u16).to_be_bytes());
        frame[ip + 8] = 64;
        frame[ip + 9] = 17;
        frame[ip + 16..ip + 20].copy_from_slice(&[255, 255, 255, 255]);
        let udp = ip + 20;
        frame[udp..udp + 2].copy_from_slice(&DHCP_CLIENT_PORT.to_be_bytes());
        frame[udp + 2..udp + 4].copy_from_slice(&DHCP_SERVER_PORT.to_be_bytes());
        frame[udp + 4..udp + 6].copy_from_slice(&((8 + payload_len) as u16).to_be_bytes());
        frame[udp + 8..udp + 8 + payload_len].copy_from_slice(&payload[..payload_len]);
        pad_frame(&frame)
    }

    fn offer_for(port: &mut DeterministicPort, mac: MacAddress) -> Vec<u8> {
        for _ in 0..8 {
            let packet = port.receive().unwrap().expect("DHCP offer");
            let chaddr = &packet[14 + 20 + 8 + 28..14 + 20 + 8 + 34];
            if chaddr == mac.to_bytes() {
                return packet;
            }
        }
        panic!("DHCP offer for MAC not found")
    }

    #[test]
    fn shared_fixture_allocates_distinct_leases_and_carries_protocol_frames() {
        let network = DeterministicVmNetwork::new(Some(DhcpServerConfig::default())).unwrap();
        let (first_mac, mut first, second_mac, mut second) = network.borrow_mut().attach_vm().unwrap();

        first
            .transmit(&dhcp_packet(first_mac, 1, DHCP_DISCOVER, None, None))
            .unwrap();
        second
            .transmit(&dhcp_packet(second_mac, 2, DHCP_DISCOVER, None, None))
            .unwrap();
        network.borrow_mut().poll(0).unwrap();

        let first_offer = offer_for(&mut first, first_mac);
        let second_offer = offer_for(&mut second, second_mac);
        let first_ip = [
            first_offer[14 + 20 + 8 + 16],
            first_offer[14 + 20 + 8 + 17],
            first_offer[14 + 20 + 8 + 18],
            first_offer[14 + 20 + 8 + 19],
        ];
        let second_ip = [
            second_offer[14 + 20 + 8 + 16],
            second_offer[14 + 20 + 8 + 17],
            second_offer[14 + 20 + 8 + 18],
            second_offer[14 + 20 + 8 + 19],
        ];
        assert_ne!(first_ip, second_ip);

        first
            .transmit(&dhcp_packet(
                first_mac,
                1,
                DHCP_REQUEST,
                Some(first_ip),
                Some([10, 5, 0, 1]),
            ))
            .unwrap();
        second
            .transmit(&dhcp_packet(
                second_mac,
                2,
                DHCP_REQUEST,
                Some(second_ip),
                Some([10, 5, 0, 1]),
            ))
            .unwrap();
        network.borrow_mut().poll(1).unwrap();
        assert_eq!(network.borrow().dhcp_server().unwrap().lease_for(first_mac).unwrap().address, first_ip);
        assert_eq!(network.borrow().dhcp_server().unwrap().lease_for(second_mac).unwrap().address, second_ip);

        while first.receive().unwrap().is_some() {}
        while second.receive().unwrap().is_some() {}
        for (ether_type, protocol) in [
            (0x0806u16, 0u8), // ARP
            (0x0800, 1),      // IPv4 ICMP
            (0x0800, 17),     // IPv4 UDP
            (0x0800, 6),      // IPv4 TCP
        ] {
            let mut frame = vec![0u8; 60];
            frame[..6].copy_from_slice(&second_mac.to_bytes());
            frame[6..12].copy_from_slice(&first_mac.to_bytes());
            frame[12..14].copy_from_slice(&ether_type.to_be_bytes());
            if ether_type == 0x0800 {
                frame[14 + 9] = protocol;
            }
            first.transmit(&frame).unwrap();
            assert_eq!(second.receive().unwrap().unwrap()[12..14], frame[12..14]);
        }
    }
}
