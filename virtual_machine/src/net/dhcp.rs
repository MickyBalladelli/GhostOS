//! Bounded DHCP server attached to the deterministic Ethernet segment.

use super::backend::{DeterministicPort, DeterministicSegment, NetBackend};
use super::mac::MacAddress;
use super::packet::{pad_frame, NetError, ETHERNET_HEADER_LEN};
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

unsafe extern "C" {
    fn ghostos_vm_dhcp_write_option(
        output: *mut u8,
        output_capacity: usize,
        cursor: usize,
        code: u8,
        value: *const u8,
        value_length: usize,
        output_cursor: *mut usize,
    ) -> bool;
    fn ghostos_vm_net_ipv4_to_number(address: *const u8) -> u32;
    fn ghostos_vm_net_ipv4_from_number(value: u32, address: *mut u8);
    fn ghostos_vm_net_checksum(bytes: *const u8, length: usize) -> u16;
}

pub const DHCP_SERVER_MAC: MacAddress = MacAddress::ghostos_default(0xD0);
pub const DHCP_SERVER_PORT: u16 = 67;
pub const DHCP_CLIENT_PORT: u16 = 68;
pub const MAX_DHCP_SERVER_LEASES: usize = 256;
pub const MAX_DHCP_SERVER_RESERVATIONS: usize = 32;
pub const MAX_DHCP_SERVER_PACKETS_PER_POLL: usize = 32;
pub const DHCP_MAGIC_COOKIE: u32 = 0x6382_5363;

const DHCP_DISCOVER: u8 = 1;
const DHCP_OFFER: u8 = 2;
const DHCP_REQUEST: u8 = 3;
const DHCP_ACK: u8 = 5;
const DHCP_NAK: u8 = 6;
const DHCP_RELEASE: u8 = 7;
const DHCP_OPTION_MESSAGE_TYPE: u8 = 53;
const DHCP_OPTION_REQUESTED_IP: u8 = 50;
const DHCP_OPTION_SERVER_ID: u8 = 54;
const DHCP_OPTION_SUBNET_MASK: u8 = 1;
const DHCP_OPTION_ROUTER: u8 = 3;
const DHCP_OPTION_DNS: u8 = 6;
const DHCP_OPTION_LEASE_TIME: u8 = 51;
const DHCP_OPTION_RENEWAL_TIME: u8 = 58;
const DHCP_OPTION_REBINDING_TIME: u8 = 59;
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
        let start = ipv4_number(self.pool_start);
        let end = ipv4_number(self.pool_end);
        if start > end || start == 0 || end == u32::MAX {
            return Err(DhcpConfigError::InvalidPool);
        }
        let size = end.saturating_sub(start).saturating_add(1);
        if size as usize > MAX_DHCP_SERVER_LEASES {
            return Err(DhcpConfigError::PoolTooLarge);
        }
        if self.lease_time_secs < 3 {
            return Err(DhcpConfigError::InvalidLeaseDuration);
        }
        if self.reservations.len() > MAX_DHCP_SERVER_RESERVATIONS {
            return Err(DhcpConfigError::TooManyReservations);
        }
        for (index, reservation) in self.reservations.iter().enumerate() {
            if !self.address_in_pool(reservation.address) {
                return Err(DhcpConfigError::ReservationOutsidePool);
            }
            if self
                .reservations
                .iter()
                .skip(index + 1)
                .any(|other| other.mac == reservation.mac || other.address == reservation.address)
            {
                return Err(DhcpConfigError::DuplicateReservation);
            }
        }
        Ok(())
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

    fn address_in_pool(&self, address: [u8; 4]) -> bool {
        let value = ipv4_number(address);
        value >= ipv4_number(self.pool_start) && value <= ipv4_number(self.pool_end)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DhcpLeaseInfo {
    pub mac: MacAddress,
    pub address: [u8; 4],
    pub expires_at_ms: u64,
}

#[derive(Clone, Copy)]
struct Lease {
    info: DhcpLeaseInfo,
    offered: bool,
}

#[derive(Clone, Copy)]
struct DhcpRequest {
    source_mac: MacAddress,
    mac: MacAddress,
    xid: u32,
    flags: u16,
    ciaddr: [u8; 4],
    message_type: u8,
    requested_ip: Option<[u8; 4]>,
    server_id: Option<[u8; 4]>,
}

/// DHCP server running as a synthetic port on a deterministic segment.
pub struct DeterministicDhcpServer {
    port: DeterministicPort,
    config: DhcpServerConfig,
    leases: Vec<Lease>,
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
        Ok(Self {
            port,
            config,
            leases: Vec::new(),
        })
    }

    pub fn config(&self) -> &DhcpServerConfig {
        &self.config
    }

    pub fn leases(&self) -> Vec<DhcpLeaseInfo> {
        self.leases.iter().map(|lease| lease.info).collect()
    }

    pub fn lease_for(&self, mac: MacAddress) -> Option<DhcpLeaseInfo> {
        self.leases
            .iter()
            .find(|lease| lease.info.mac == mac)
            .map(|lease| lease.info)
    }

    pub fn poll(&mut self, now_ms: u64) -> Result<usize, NetError> {
        self.expire(now_ms);
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
        let request = decode_request(frame)?;
        if request.source_mac != request.mac {
            return None;
        }
        if request.server_id.is_some_and(|server| server != self.config.server_ip) {
            return None;
        }
        match request.message_type {
            DHCP_DISCOVER => {
                let address = self.choose_address(request.mac, request.requested_ip, now_ms)?;
                self.set_lease(request.mac, address, now_ms, true);
                Some(encode_reply(&self.config, request, DHCP_OFFER, address))
            }
            DHCP_REQUEST => {
                let requested = request.requested_ip.or_else(|| nonzero(request.ciaddr));
                let address = self.choose_requested(request.mac, requested, now_ms);
                match address {
                    Some(address) => {
                        self.set_lease(request.mac, address, now_ms, false);
                        Some(encode_reply(&self.config, request, DHCP_ACK, address))
                    }
                    None => Some(encode_reply(&self.config, request, DHCP_NAK, [0; 4])),
                }
            }
            DHCP_RELEASE => {
                self.leases.retain(|lease| lease.info.mac != request.mac);
                None
            }
            _ => None,
        }
    }

    fn choose_requested(
        &self,
        mac: MacAddress,
        requested: Option<[u8; 4]>,
        now_ms: u64,
    ) -> Option<[u8; 4]> {
        let reservation = self.reservation_for(mac);
        if reservation.is_some_and(|address| requested.is_some_and(|wanted| wanted != address)) {
            return None;
        }
        if let Some(address) = reservation {
            return self.address_available(address, mac, now_ms).then_some(address);
        }
        if let Some(address) = requested {
            return self
                .config
                .address_in_pool(address)
                .then(|| self.address_available(address, mac, now_ms).then_some(address))
                .flatten();
        }
        self.choose_address(mac, None, now_ms)
    }

    fn choose_address(
        &self,
        mac: MacAddress,
        requested: Option<[u8; 4]>,
        now_ms: u64,
    ) -> Option<[u8; 4]> {
        if let Some(address) = self.reservation_for(mac) {
            return self.address_available(address, mac, now_ms).then_some(address);
        }
        if let Some(existing) = self
            .leases
            .iter()
            .find(|lease| lease.info.mac == mac && lease.info.expires_at_ms > now_ms)
            .map(|lease| lease.info.address)
        {
            return Some(existing);
        }
        if let Some(requested) = requested {
            if self.config.address_in_pool(requested)
                && self.address_available(requested, mac, now_ms)
            {
                return Some(requested);
            }
        }
        let start = ipv4_number(self.config.pool_start);
        let end = ipv4_number(self.config.pool_end);
        for value in start..=end {
            let address = ipv4_bytes(value);
            if !self.is_reserved(address) && self.address_available(address, mac, now_ms) {
                return Some(address);
            }
        }
        None
    }

    fn address_available(&self, address: [u8; 4], mac: MacAddress, now_ms: u64) -> bool {
        !self.leases.iter().any(|lease| {
            lease.info.address == address
                && lease.info.mac != mac
                && lease.info.expires_at_ms > now_ms
        })
    }

    fn is_reserved(&self, address: [u8; 4]) -> bool {
        self.config
            .reservations
            .iter()
            .any(|reservation| reservation.address == address)
    }

    fn reservation_for(&self, mac: MacAddress) -> Option<[u8; 4]> {
        self.config
            .reservations
            .iter()
            .find(|reservation| reservation.mac == mac)
            .map(|reservation| reservation.address)
    }

    fn set_lease(&mut self, mac: MacAddress, address: [u8; 4], now_ms: u64, offered: bool) {
        let expires_at_ms = now_ms.saturating_add(self.config.lease_time_secs as u64 * 1_000);
        if let Some(lease) = self.leases.iter_mut().find(|lease| lease.info.mac == mac) {
            lease.info.address = address;
            lease.info.expires_at_ms = expires_at_ms;
            lease.offered = offered;
            return;
        }
        if self.leases.len() < MAX_DHCP_SERVER_LEASES {
            self.leases.push(Lease {
                info: DhcpLeaseInfo {
                    mac,
                    address,
                    expires_at_ms,
                },
                offered,
            });
        }
    }

    fn expire(&mut self, now_ms: u64) {
        self.leases
            .retain(|lease| lease.info.expires_at_ms > now_ms);
    }
}

fn decode_request(frame: &[u8]) -> Option<DhcpRequest> {
    if frame.len() < ETHERNET_HEADER_LEN + 20 + 8 + 240 {
        return None;
    }
    if frame[12..14] != [0x08, 0x00] {
        return None;
    }
    let ip = ETHERNET_HEADER_LEN;
    if frame[ip] >> 4 != 4 || frame[ip] & 0x0F < 5 {
        return None;
    }
    let ip_header_len = (frame[ip] as usize & 0x0F) * 4;
    if frame.len() < ip + ip_header_len + 8 {
        return None;
    }
    if frame[ip + 9] != 17 {
        return None;
    }
    let total_len = u16::from_be_bytes([frame[ip + 2], frame[ip + 3]]) as usize;
    if total_len < ip_header_len + 8 || ip + total_len > frame.len() {
        return None;
    }
    let udp = ip + ip_header_len;
    if u16::from_be_bytes([frame[udp], frame[udp + 1]]) != DHCP_CLIENT_PORT
        || u16::from_be_bytes([frame[udp + 2], frame[udp + 3]]) != DHCP_SERVER_PORT
    {
        return None;
    }
    let udp_len = u16::from_be_bytes([frame[udp + 4], frame[udp + 5]]) as usize;
    if udp_len < 8 + 240 || udp + udp_len > ip + total_len {
        return None;
    }
    let payload = &frame[udp + 8..udp + udp_len];
    if payload[0] != 1
        || payload[1] != 1
        || payload[2] != 6
        || u32::from_be_bytes([payload[236], payload[237], payload[238], payload[239]])
            != DHCP_MAGIC_COOKIE
    {
        return None;
    }
    let mut request = DhcpRequest {
        source_mac: MacAddress::from_bytes(&frame[6..12])?,
        mac: MacAddress::from_bytes(&payload[28..34])?,
        xid: u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]),
        flags: u16::from_be_bytes([payload[10], payload[11]]),
        ciaddr: [payload[12], payload[13], payload[14], payload[15]],
        message_type: 0,
        requested_ip: None,
        server_id: None,
    };
    let mut cursor = 240;
    while cursor < payload.len() {
        let code = payload[cursor];
        cursor += 1;
        if code == DHCP_OPTION_END {
            break;
        }
        if code == 0 {
            continue;
        }
        let length = *payload.get(cursor)? as usize;
        cursor += 1;
        let value = payload.get(cursor..cursor.checked_add(length)?)?;
        cursor += length;
        match code {
            DHCP_OPTION_MESSAGE_TYPE if value.len() == 1 => request.message_type = value[0],
            DHCP_OPTION_REQUESTED_IP if value.len() == 4 => {
                request.requested_ip = Some([value[0], value[1], value[2], value[3]])
            }
            DHCP_OPTION_SERVER_ID if value.len() == 4 => {
                request.server_id = Some([value[0], value[1], value[2], value[3]])
            }
            _ => {}
        }
    }
    (request.message_type != 0).then_some(request)
}

fn encode_reply(
    config: &DhcpServerConfig,
    request: DhcpRequest,
    message_type: u8,
    address: [u8; 4],
) -> Vec<u8> {
    let mut dhcp = [0u8; 576];
    dhcp[0] = 2;
    dhcp[1] = 1;
    dhcp[2] = 6;
    dhcp[4..8].copy_from_slice(&request.xid.to_be_bytes());
    dhcp[10..12].copy_from_slice(&request.flags.to_be_bytes());
    dhcp[16..20].copy_from_slice(&address);
    dhcp[20..24].copy_from_slice(&config.server_ip);
    dhcp[28..34].copy_from_slice(&request.mac.to_bytes());
    dhcp[236..240].copy_from_slice(&DHCP_MAGIC_COOKIE.to_be_bytes());
    let mut cursor = 240;
    cursor = write_option(&mut dhcp, cursor, DHCP_OPTION_MESSAGE_TYPE, &[message_type]);
    if message_type != DHCP_NAK {
        cursor = write_option(&mut dhcp, cursor, DHCP_OPTION_SUBNET_MASK, &config.subnet_mask);
        cursor = write_option(&mut dhcp, cursor, DHCP_OPTION_ROUTER, &config.gateway);
        cursor = write_option(&mut dhcp, cursor, DHCP_OPTION_DNS, &config.dns);
        cursor = write_option(
            &mut dhcp,
            cursor,
            DHCP_OPTION_LEASE_TIME,
            &config.lease_time_secs.to_be_bytes(),
        );
        let t1 = (config.lease_time_secs / 2).max(1);
        let t2 = (config.lease_time_secs.saturating_mul(7) / 8)
            .max(t1.saturating_add(1))
            .min(config.lease_time_secs.saturating_sub(1));
        cursor = write_option(&mut dhcp, cursor, DHCP_OPTION_RENEWAL_TIME, &t1.to_be_bytes());
        cursor = write_option(&mut dhcp, cursor, DHCP_OPTION_REBINDING_TIME, &t2.to_be_bytes());
    }
    cursor = write_option(&mut dhcp, cursor, DHCP_OPTION_SERVER_ID, &config.server_ip);
    dhcp[cursor] = DHCP_OPTION_END;
    let dhcp_len = cursor + 1;

    let ip_len = 20 + 8 + dhcp_len;
    let mut frame = vec![0u8; ETHERNET_HEADER_LEN + ip_len];
    frame[..6].fill(0xFF);
    frame[6..12].copy_from_slice(&DHCP_SERVER_MAC.to_bytes());
    frame[12..14].copy_from_slice(&[0x08, 0x00]);
    let ip = ETHERNET_HEADER_LEN;
    frame[ip] = 0x45;
    frame[ip + 2..ip + 4].copy_from_slice(&(ip_len as u16).to_be_bytes());
    frame[ip + 6..ip + 8].copy_from_slice(&0x4000u16.to_be_bytes());
    frame[ip + 8] = 64;
    frame[ip + 9] = 17;
    frame[ip + 12..ip + 16].copy_from_slice(&config.server_ip);
    frame[ip + 16..ip + 20].copy_from_slice(&[255, 255, 255, 255]);
    let ip_checksum = checksum(&frame[ip..ip + 20]);
    frame[ip + 10..ip + 12].copy_from_slice(&ip_checksum.to_be_bytes());
    let udp = ip + 20;
    frame[udp..udp + 2].copy_from_slice(&DHCP_SERVER_PORT.to_be_bytes());
    frame[udp + 2..udp + 4].copy_from_slice(&DHCP_CLIENT_PORT.to_be_bytes());
    frame[udp + 4..udp + 6].copy_from_slice(&((8 + dhcp_len) as u16).to_be_bytes());
    frame[udp + 8..udp + 8 + dhcp_len].copy_from_slice(&dhcp[..dhcp_len]);
    let mut pseudo = Vec::with_capacity(12 + 8 + dhcp_len);
    pseudo.extend_from_slice(&config.server_ip);
    pseudo.extend_from_slice(&[255, 255, 255, 255]);
    pseudo.extend_from_slice(&[0, 17]);
    pseudo.extend_from_slice(&((8 + dhcp_len) as u16).to_be_bytes());
    pseudo.extend_from_slice(&frame[udp..udp + 8 + dhcp_len]);
    let udp_checksum = checksum(&pseudo);
    frame[udp + 6..udp + 8].copy_from_slice(&udp_checksum.to_be_bytes());
    pad_frame(&frame)
}

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

fn checksum(bytes: &[u8]) -> u16 {
    unsafe { ghostos_vm_net_checksum(bytes.as_ptr(), bytes.len()) }
}

fn nonzero(address: [u8; 4]) -> Option<[u8; 4]> {
    (address != [0; 4]).then_some(address)
}

fn ipv4_number(address: [u8; 4]) -> u32 {
    unsafe { ghostos_vm_net_ipv4_to_number(address.as_ptr()) }
}

fn ipv4_bytes(address: u32) -> [u8; 4] {
    let mut bytes = [0; 4];
    unsafe { ghostos_vm_net_ipv4_from_number(address, bytes.as_mut_ptr()) }
    bytes
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
