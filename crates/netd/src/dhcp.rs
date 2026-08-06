//! Capability-gated IPv4 DHCP client with bounded DORA handling.
//!
//! The client binds to one Ethernet interface, validates transaction identity
//! and lease options, and applies accepted leases through a runtime bridge so
//! previous static configuration can be restored on failure or expiry.

use crate::firewall::{
    CapabilityRight, Direction, FirewallRule, Ipv4Cidr, PortRange, Protocol, RateLimit, RuleAction,
};

pub const DHCP_CLIENT_PORT: u16 = 68;
pub const DHCP_SERVER_PORT: u16 = 67;
pub const MAX_DHCP_PACKET: usize = 576;
pub const MAX_DHCP_DNS_SERVERS: usize = 2;
pub const MAX_DHCP_ROUTES: usize = 4;
pub const MAX_INTERFACE_NAME: usize = 48;
pub const DHCP_MAGIC_COOKIE: u32 = 0x6382_5363;

const OPT_SUBNET_MASK: u8 = 1;
const OPT_ROUTER: u8 = 3;
const OPT_DNS: u8 = 6;
const OPT_REQUESTED_IP: u8 = 50;
const OPT_LEASE_TIME: u8 = 51;
const OPT_MESSAGE_TYPE: u8 = 53;
const OPT_SERVER_ID: u8 = 54;
const OPT_PARAMETER_REQUEST: u8 = 55;
const OPT_RENEWAL_TIME: u8 = 58;
const OPT_REBINDING_TIME: u8 = 59;
const OPT_END: u8 = 255;

/// Initial discover/request backoff schedule in milliseconds (bounded).
pub const BACKOFF_MS: [u64; 5] = [4_000, 8_000, 16_000, 32_000, 64_000];
pub const MAX_DISCOVER_ATTEMPTS: u8 = 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DhcpError {
    AccessDenied,
    Capacity,
    InvalidPacket,
    InvalidLease,
    InvalidState,
    InvalidInterface,
    NoOffer,
    ConflictingOffer,
    ServerUnavailable,
    Runtime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DhcpMessageType {
    Discover = 1,
    Offer = 2,
    Request = 3,
    Decline = 4,
    Ack = 5,
    Nak = 6,
    Release = 7,
    Inform = 8,
}

impl DhcpMessageType {
    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Discover),
            2 => Some(Self::Offer),
            3 => Some(Self::Request),
            4 => Some(Self::Decline),
            5 => Some(Self::Ack),
            6 => Some(Self::Nak),
            7 => Some(Self::Release),
            8 => Some(Self::Inform),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum DhcpClientState {
    Init = 0,
    Selecting = 1,
    Requesting = 2,
    Bound = 3,
    Renewing = 4,
    Rebinding = 5,
    InitReboot = 6,
}

impl DhcpClientState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Selecting => "selecting",
            Self::Requesting => "requesting",
            Self::Bound => "bound",
            Self::Renewing => "renewing",
            Self::Rebinding => "rebinding",
            Self::InitReboot => "init-reboot",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StaticSnapshot {
    pub address: [u8; 4],
    pub gateway: Option<[u8; 4]>,
    pub subnet_mask: Option<[u8; 4]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DhcpLease {
    pub address: [u8; 4],
    pub subnet_mask: [u8; 4],
    pub gateway: Option<[u8; 4]>,
    pub dns: [[u8; 4]; MAX_DHCP_DNS_SERVERS],
    pub dns_count: u8,
    pub server_id: [u8; 4],
    pub lease_time_secs: u32,
    pub t1_secs: u32,
    pub t2_secs: u32,
    pub yiaddr: [u8; 4],
}

impl DhcpLease {
    pub const fn expires_at_ms(self, bound_at_ms: u64) -> u64 {
        bound_at_ms.saturating_add((self.lease_time_secs as u64).saturating_mul(1_000))
    }

    pub const fn renew_at_ms(self, bound_at_ms: u64) -> u64 {
        bound_at_ms.saturating_add((self.t1_secs as u64).saturating_mul(1_000))
    }

    pub const fn rebind_at_ms(self, bound_at_ms: u64) -> u64 {
        bound_at_ms.saturating_add((self.t2_secs as u64).saturating_mul(1_000))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DhcpOffer {
    pub xid: u32,
    pub yiaddr: [u8; 4],
    pub server_id: [u8; 4],
    pub subnet_mask: [u8; 4],
    pub gateway: Option<[u8; 4]>,
    pub dns: [[u8; 4]; MAX_DHCP_DNS_SERVERS],
    pub dns_count: u8,
    pub lease_time_secs: u32,
    pub t1_secs: u32,
    pub t2_secs: u32,
    pub chaddr: [u8; 6],
}

/// Applies or restores interface addressing through the network configuration runtime.
pub trait DhcpLeaseRuntime {
    fn apply_lease(&mut self, interface: &str, lease: &DhcpLease) -> Result<(), DhcpError>;
    fn restore_static(&mut self, interface: &str, snapshot: &StaticSnapshot) -> Result<(), DhcpError>;
}

/// Bound UDP/Ethernet transport used by the DHCP client.
pub trait DhcpTransport {
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
    ) -> Result<(), DhcpError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DhcpClientView {
    pub interface: [u8; MAX_INTERFACE_NAME],
    pub interface_len: u8,
    pub state: DhcpClientState,
    pub link_up: bool,
    pub xid: u32,
    pub attempt: u8,
    pub lease: Option<DhcpLease>,
    pub bound_at_ms: Option<u64>,
    pub next_action_ms: Option<u64>,
    pub preserved: Option<StaticSnapshot>,
}

impl DhcpClientView {
    pub fn interface_name(&self) -> &str {
        core::str::from_utf8(&self.interface[..self.interface_len as usize]).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingOffer {
    offer: DhcpOffer,
}

pub struct DhcpClient {
    interface: [u8; MAX_INTERFACE_NAME],
    interface_len: u8,
    mac: [u8; 6],
    state: DhcpClientState,
    link_up: bool,
    authorized: bool,
    xid: u32,
    attempt: u8,
    next_action_ms: Option<u64>,
    selected: Option<PendingOffer>,
    lease: Option<DhcpLease>,
    bound_at_ms: Option<u64>,
    preserved: Option<StaticSnapshot>,
    txid_seed: u32,
}

impl DhcpClient {
    pub fn new(interface: &str, mac: [u8; 6], xid_seed: u32) -> Result<Self, DhcpError> {
        if interface.is_empty() || interface.len() > MAX_INTERFACE_NAME || mac == [0; 6] {
            return Err(DhcpError::InvalidInterface);
        }
        let mut name = [0; MAX_INTERFACE_NAME];
        name[..interface.len()].copy_from_slice(interface.as_bytes());
        Ok(Self {
            interface: name,
            interface_len: interface.len() as u8,
            mac,
            state: DhcpClientState::Init,
            link_up: true,
            authorized: false,
            xid: 0,
            attempt: 0,
            next_action_ms: Some(0),
            selected: None,
            lease: None,
            bound_at_ms: None,
            preserved: None,
            txid_seed: xid_seed,
        })
    }

    pub fn authorize(&mut self, rights: u8) -> Result<(), DhcpError> {
        let required = CapabilityRight::Raw as u8 | CapabilityRight::Ingress as u8;
        if rights & required != required {
            return Err(DhcpError::AccessDenied);
        }
        self.authorized = true;
        Ok(())
    }

    pub const fn is_authorized(&self) -> bool {
        self.authorized
    }

    pub fn interface_name(&self) -> &str {
        core::str::from_utf8(&self.interface[..self.interface_len as usize]).unwrap_or("")
    }

    pub const fn state(&self) -> DhcpClientState {
        self.state
    }

    pub const fn lease(&self) -> Option<DhcpLease> {
        self.lease
    }

    pub const fn preserved(&self) -> Option<StaticSnapshot> {
        self.preserved
    }

    pub fn view(&self) -> DhcpClientView {
        DhcpClientView {
            interface: self.interface,
            interface_len: self.interface_len,
            state: self.state,
            link_up: self.link_up,
            xid: self.xid,
            attempt: self.attempt,
            lease: self.lease,
            bound_at_ms: self.bound_at_ms,
            next_action_ms: self.next_action_ms,
            preserved: self.preserved,
        }
    }

    pub fn preserve_static(&mut self, snapshot: StaticSnapshot) {
        self.preserved = Some(snapshot);
    }

    pub fn set_link(&mut self, up: bool, now_ms: u64) {
        self.link_up = up;
        if !up {
            self.next_action_ms = None;
            return;
        }
        match self.state {
            DhcpClientState::Bound | DhcpClientState::Renewing | DhcpClientState::Rebinding
                if self.lease.is_some() =>
            {
                self.state = DhcpClientState::InitReboot;
                self.attempt = 0;
                self.next_action_ms = Some(now_ms);
            }
            _ => {
                self.state = DhcpClientState::Init;
                self.attempt = 0;
                self.selected = None;
                self.next_action_ms = Some(now_ms);
            }
        }
    }

    pub fn start(&mut self, now_ms: u64) -> Result<(), DhcpError> {
        self.require_auth()?;
        if !self.link_up {
            return Err(DhcpError::InvalidState);
        }
        self.state = if self.lease.is_some() {
            DhcpClientState::InitReboot
        } else {
            DhcpClientState::Init
        };
        self.attempt = 0;
        self.selected = None;
        self.next_action_ms = Some(now_ms);
        Ok(())
    }

    pub fn poll<T: DhcpTransport, R: DhcpLeaseRuntime>(
        &mut self,
        now_ms: u64,
        transport: &mut T,
        runtime: &mut R,
    ) -> Result<(), DhcpError> {
        self.require_auth()?;
        if !self.link_up {
            return Ok(());
        }
        if let (Some(lease), Some(bound_at)) = (self.lease, self.bound_at_ms) {
            if now_ms >= lease.expires_at_ms(bound_at) {
                return self.expire(runtime);
            }
            match self.state {
                DhcpClientState::Bound if now_ms >= lease.renew_at_ms(bound_at) => {
                    self.state = DhcpClientState::Renewing;
                    self.attempt = 0;
                    self.next_action_ms = Some(now_ms);
                }
                DhcpClientState::Renewing if now_ms >= lease.rebind_at_ms(bound_at) => {
                    self.state = DhcpClientState::Rebinding;
                    self.attempt = 0;
                    self.next_action_ms = Some(now_ms);
                }
                _ => {}
            }
        }
        let Some(due) = self.next_action_ms else {
            return Ok(());
        };
        if now_ms < due {
            return Ok(());
        }
        let result = match self.state {
            DhcpClientState::Init | DhcpClientState::Selecting => self.send_discover(now_ms, transport),
            DhcpClientState::Requesting => self.send_request(now_ms, transport, false),
            DhcpClientState::InitReboot => self.send_request(now_ms, transport, true),
            DhcpClientState::Renewing => self.send_renew(now_ms, transport),
            DhcpClientState::Rebinding => self.send_rebind(now_ms, transport),
            DhcpClientState::Bound => Ok(()),
        };
        if result == Err(DhcpError::ServerUnavailable) {
            // Never leave a half-applied DHCP attempt in place; restore static.
            let _ = self.clear_lease(runtime);
        }
        result
    }

    pub fn handle_packet<R: DhcpLeaseRuntime>(
        &mut self,
        packet: &[u8],
        now_ms: u64,
        runtime: &mut R,
    ) -> Result<(), DhcpError> {
        self.require_auth()?;
        let message = DhcpMessage::decode(packet)?;
        if message.xid != self.xid || message.chaddr != self.mac {
            return Err(DhcpError::InvalidPacket);
        }
        match message.message_type {
            DhcpMessageType::Offer => self.handle_offer(message, now_ms),
            DhcpMessageType::Ack => self.handle_ack(message, now_ms, runtime),
            DhcpMessageType::Nak => self.handle_nak(now_ms, runtime),
            _ => Err(DhcpError::InvalidPacket),
        }
    }

    pub fn release<T: DhcpTransport, R: DhcpLeaseRuntime>(
        &mut self,
        now_ms: u64,
        transport: &mut T,
        runtime: &mut R,
    ) -> Result<(), DhcpError> {
        self.require_auth()?;
        let Some(lease) = self.lease else {
            return Err(DhcpError::InvalidState);
        };
        let mut message = DhcpMessage::new(DhcpMessageType::Release, self.xid, self.mac);
        message.ciaddr = lease.address;
        message.server_id = Some(lease.server_id);
        let mut packet = [0; MAX_DHCP_PACKET];
        let length = message.encode(&mut packet)?;
        transport.send_udp(
            self.interface_name(),
            self.mac,
            lease.address,
            lease.server_id,
            [0xff; 6],
            DHCP_CLIENT_PORT,
            DHCP_SERVER_PORT,
            &packet[..length],
        )?;
        self.clear_lease(runtime)?;
        self.state = DhcpClientState::Init;
        self.next_action_ms = Some(now_ms);
        Ok(())
    }

    fn handle_offer(&mut self, message: DhcpMessage, now_ms: u64) -> Result<(), DhcpError> {
        let offer = message.into_offer()?;
        if let Some(selected) = self.selected {
            if selected.offer.server_id != offer.server_id || selected.offer.yiaddr != offer.yiaddr
            {
                return Err(DhcpError::ConflictingOffer);
            }
            return Ok(());
        }
        if !matches!(
            self.state,
            DhcpClientState::Init | DhcpClientState::Selecting
        ) {
            return Err(DhcpError::InvalidState);
        }
        self.selected = Some(PendingOffer { offer });
        self.state = DhcpClientState::Requesting;
        self.attempt = 0;
        self.next_action_ms = Some(now_ms);
        Ok(())
    }

    fn handle_ack<R: DhcpLeaseRuntime>(
        &mut self,
        message: DhcpMessage,
        now_ms: u64,
        runtime: &mut R,
    ) -> Result<(), DhcpError> {
        if !matches!(
            self.state,
            DhcpClientState::Requesting
                | DhcpClientState::InitReboot
                | DhcpClientState::Renewing
                | DhcpClientState::Rebinding
        ) {
            return Err(DhcpError::InvalidState);
        }
        let lease = message.into_lease()?;
        if let Some(selected) = self.selected {
            if selected.offer.server_id != lease.server_id || selected.offer.yiaddr != lease.yiaddr {
                return Err(DhcpError::ConflictingOffer);
            }
        }
        runtime.apply_lease(self.interface_name(), &lease)?;
        self.lease = Some(lease);
        self.bound_at_ms = Some(now_ms);
        self.selected = None;
        self.attempt = 0;
        self.state = DhcpClientState::Bound;
        self.next_action_ms = Some(lease.renew_at_ms(now_ms));
        Ok(())
    }

    fn handle_nak<R: DhcpLeaseRuntime>(
        &mut self,
        now_ms: u64,
        runtime: &mut R,
    ) -> Result<(), DhcpError> {
        if matches!(
            self.state,
            DhcpClientState::Requesting
                | DhcpClientState::InitReboot
                | DhcpClientState::Renewing
                | DhcpClientState::Rebinding
        ) {
            self.clear_lease(runtime)?;
            self.selected = None;
            self.state = DhcpClientState::Init;
            self.attempt = 0;
            self.next_action_ms = Some(now_ms);
            Ok(())
        } else {
            Err(DhcpError::InvalidState)
        }
    }

    fn expire<R: DhcpLeaseRuntime>(&mut self, runtime: &mut R) -> Result<(), DhcpError> {
        self.clear_lease(runtime)?;
        self.state = DhcpClientState::Init;
        self.attempt = 0;
        self.selected = None;
        self.next_action_ms = Some(0);
        Ok(())
    }

    fn clear_lease<R: DhcpLeaseRuntime>(&mut self, runtime: &mut R) -> Result<(), DhcpError> {
        if let Some(snapshot) = self.preserved {
            runtime.restore_static(self.interface_name(), &snapshot)?;
        }
        self.lease = None;
        self.bound_at_ms = None;
        Ok(())
    }

    fn send_discover<T: DhcpTransport>(
        &mut self,
        now_ms: u64,
        transport: &mut T,
    ) -> Result<(), DhcpError> {
        if self.attempt >= MAX_DISCOVER_ATTEMPTS {
            self.next_action_ms = None;
            self.state = DhcpClientState::Init;
            return Err(DhcpError::ServerUnavailable);
        }
        self.xid = next_xid(self.txid_seed, self.attempt);
        self.state = DhcpClientState::Selecting;
        let mut message = DhcpMessage::new(DhcpMessageType::Discover, self.xid, self.mac);
        message.flags = 0x8000;
        message.parameter_request = true;
        let mut packet = [0; MAX_DHCP_PACKET];
        let length = message.encode(&mut packet)?;
        transport.send_udp(
            self.interface_name(),
            self.mac,
            [0; 4],
            [255, 255, 255, 255],
            [0xff; 6],
            DHCP_CLIENT_PORT,
            DHCP_SERVER_PORT,
            &packet[..length],
        )?;
        self.schedule_retry(now_ms);
        Ok(())
    }

    fn send_request<T: DhcpTransport>(
        &mut self,
        now_ms: u64,
        transport: &mut T,
        init_reboot: bool,
    ) -> Result<(), DhcpError> {
        if self.attempt >= MAX_DISCOVER_ATTEMPTS {
            self.next_action_ms = None;
            return Err(DhcpError::ServerUnavailable);
        }
        let (yiaddr, server_id) = if init_reboot {
            let lease = self.lease.ok_or(DhcpError::InvalidState)?;
            (lease.address, None)
        } else {
            let selected = self.selected.ok_or(DhcpError::NoOffer)?;
            (selected.offer.yiaddr, Some(selected.offer.server_id))
        };
        if init_reboot {
            self.xid = next_xid(self.txid_seed, self.attempt.saturating_add(16));
        }
        let mut message = DhcpMessage::new(DhcpMessageType::Request, self.xid, self.mac);
        message.flags = 0x8000;
        message.requested_ip = Some(yiaddr);
        message.server_id = server_id;
        message.parameter_request = true;
        let mut packet = [0; MAX_DHCP_PACKET];
        let length = message.encode(&mut packet)?;
        transport.send_udp(
            self.interface_name(),
            self.mac,
            [0; 4],
            [255, 255, 255, 255],
            [0xff; 6],
            DHCP_CLIENT_PORT,
            DHCP_SERVER_PORT,
            &packet[..length],
        )?;
        if !init_reboot {
            self.state = DhcpClientState::Requesting;
        }
        self.schedule_retry(now_ms);
        Ok(())
    }

    fn send_renew<T: DhcpTransport>(
        &mut self,
        now_ms: u64,
        transport: &mut T,
    ) -> Result<(), DhcpError> {
        let lease = self.lease.ok_or(DhcpError::InvalidState)?;
        self.xid = next_xid(self.txid_seed, self.attempt.saturating_add(32));
        let mut message = DhcpMessage::new(DhcpMessageType::Request, self.xid, self.mac);
        message.ciaddr = lease.address;
        message.parameter_request = true;
        let mut packet = [0; MAX_DHCP_PACKET];
        let length = message.encode(&mut packet)?;
        transport.send_udp(
            self.interface_name(),
            self.mac,
            lease.address,
            lease.server_id,
            [0xff; 6],
            DHCP_CLIENT_PORT,
            DHCP_SERVER_PORT,
            &packet[..length],
        )?;
        self.schedule_retry(now_ms);
        Ok(())
    }

    fn send_rebind<T: DhcpTransport>(
        &mut self,
        now_ms: u64,
        transport: &mut T,
    ) -> Result<(), DhcpError> {
        let lease = self.lease.ok_or(DhcpError::InvalidState)?;
        self.xid = next_xid(self.txid_seed, self.attempt.saturating_add(48));
        let mut message = DhcpMessage::new(DhcpMessageType::Request, self.xid, self.mac);
        message.ciaddr = lease.address;
        message.flags = 0x8000;
        message.parameter_request = true;
        let mut packet = [0; MAX_DHCP_PACKET];
        let length = message.encode(&mut packet)?;
        transport.send_udp(
            self.interface_name(),
            self.mac,
            lease.address,
            [255, 255, 255, 255],
            [0xff; 6],
            DHCP_CLIENT_PORT,
            DHCP_SERVER_PORT,
            &packet[..length],
        )?;
        self.schedule_retry(now_ms);
        Ok(())
    }

    fn schedule_retry(&mut self, now_ms: u64) {
        let index = core::cmp::min(self.attempt as usize, BACKOFF_MS.len() - 1);
        self.next_action_ms = Some(now_ms.saturating_add(BACKOFF_MS[index]));
        self.attempt = self.attempt.saturating_add(1);
    }

    fn require_auth(&self) -> Result<(), DhcpError> {
        if self.authorized {
            Ok(())
        } else {
            Err(DhcpError::AccessDenied)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DhcpMessage {
    message_type: DhcpMessageType,
    xid: u32,
    flags: u16,
    ciaddr: [u8; 4],
    yiaddr: [u8; 4],
    siaddr: [u8; 4],
    giaddr: [u8; 4],
    chaddr: [u8; 6],
    subnet_mask: Option<[u8; 4]>,
    gateway: Option<[u8; 4]>,
    dns: [[u8; 4]; MAX_DHCP_DNS_SERVERS],
    dns_count: u8,
    lease_time_secs: Option<u32>,
    t1_secs: Option<u32>,
    t2_secs: Option<u32>,
    server_id: Option<[u8; 4]>,
    requested_ip: Option<[u8; 4]>,
    parameter_request: bool,
}

impl DhcpMessage {
    fn new(message_type: DhcpMessageType, xid: u32, mac: [u8; 6]) -> Self {
        Self {
            message_type,
            xid,
            flags: 0,
            ciaddr: [0; 4],
            yiaddr: [0; 4],
            siaddr: [0; 4],
            giaddr: [0; 4],
            chaddr: mac,
            subnet_mask: None,
            gateway: None,
            dns: [[0; 4]; MAX_DHCP_DNS_SERVERS],
            dns_count: 0,
            lease_time_secs: None,
            t1_secs: None,
            t2_secs: None,
            server_id: None,
            requested_ip: None,
            parameter_request: false,
        }
    }

    fn encode(self, output: &mut [u8]) -> Result<usize, DhcpError> {
        if output.len() < 240 {
            return Err(DhcpError::Capacity);
        }
        output[..240].fill(0);
        output[0] = 1; // BOOTREQUEST
        output[1] = 1; // Ethernet
        output[2] = 6;
        output[4..8].copy_from_slice(&self.xid.to_be_bytes());
        output[10..12].copy_from_slice(&self.flags.to_be_bytes());
        output[12..16].copy_from_slice(&self.ciaddr);
        output[16..20].copy_from_slice(&self.yiaddr);
        output[20..24].copy_from_slice(&self.siaddr);
        output[24..28].copy_from_slice(&self.giaddr);
        output[28..34].copy_from_slice(&self.chaddr);
        output[236..240].copy_from_slice(&DHCP_MAGIC_COOKIE.to_be_bytes());
        let mut cursor = 240;
        cursor = write_option(output, cursor, OPT_MESSAGE_TYPE, &[self.message_type as u8])?;
        if let Some(ip) = self.requested_ip {
            cursor = write_option(output, cursor, OPT_REQUESTED_IP, &ip)?;
        }
        if let Some(server) = self.server_id {
            cursor = write_option(output, cursor, OPT_SERVER_ID, &server)?;
        }
        if self.parameter_request {
            cursor = write_option(
                output,
                cursor,
                OPT_PARAMETER_REQUEST,
                &[OPT_SUBNET_MASK, OPT_ROUTER, OPT_DNS, OPT_LEASE_TIME, OPT_RENEWAL_TIME, OPT_REBINDING_TIME],
            )?;
        }
        if cursor >= output.len() {
            return Err(DhcpError::Capacity);
        }
        output[cursor] = OPT_END;
        Ok(cursor + 1)
    }

    fn decode(input: &[u8]) -> Result<Self, DhcpError> {
        if input.len() < 241 || input[1] != 1 || input[2] != 6 {
            return Err(DhcpError::InvalidPacket);
        }
        let magic = u32::from_be_bytes([input[236], input[237], input[238], input[239]]);
        if magic != DHCP_MAGIC_COOKIE {
            return Err(DhcpError::InvalidPacket);
        }
        let mut message = Self::new(DhcpMessageType::Offer, 0, [0; 6]);
        message.xid = u32::from_be_bytes([input[4], input[5], input[6], input[7]]);
        message.flags = u16::from_be_bytes([input[10], input[11]]);
        message.ciaddr = [input[12], input[13], input[14], input[15]];
        message.yiaddr = [input[16], input[17], input[18], input[19]];
        message.siaddr = [input[20], input[21], input[22], input[23]];
        message.giaddr = [input[24], input[25], input[26], input[27]];
        message.chaddr = [input[28], input[29], input[30], input[31], input[32], input[33]];

        let mut index = 240;
        let mut message_type = None;
        while index < input.len() {
            let code = input[index];
            index += 1;
            if code == OPT_END {
                break;
            }
            if code == 0 {
                continue;
            }
            if index >= input.len() {
                return Err(DhcpError::InvalidPacket);
            }
            let length = input[index] as usize;
            index += 1;
            if index + length > input.len() {
                return Err(DhcpError::InvalidPacket);
            }
            let value = &input[index..index + length];
            index += length;
            match code {
                OPT_MESSAGE_TYPE if length == 1 => {
                    message_type = DhcpMessageType::from_raw(value[0]);
                }
                OPT_SUBNET_MASK if length == 4 => {
                    message.subnet_mask = Some([value[0], value[1], value[2], value[3]]);
                }
                OPT_ROUTER if length >= 4 => {
                    message.gateway = Some([value[0], value[1], value[2], value[3]]);
                }
                OPT_DNS => {
                    let mut count = 0u8;
                    let mut dns = [[0; 4]; MAX_DHCP_DNS_SERVERS];
                    let mut offset = 0;
                    while offset + 4 <= value.len() && count < MAX_DHCP_DNS_SERVERS as u8 {
                        dns[count as usize] =
                            [value[offset], value[offset + 1], value[offset + 2], value[offset + 3]];
                        count += 1;
                        offset += 4;
                    }
                    message.dns = dns;
                    message.dns_count = count;
                }
                OPT_LEASE_TIME if length == 4 => {
                    message.lease_time_secs =
                        Some(u32::from_be_bytes([value[0], value[1], value[2], value[3]]));
                }
                OPT_RENEWAL_TIME if length == 4 => {
                    message.t1_secs =
                        Some(u32::from_be_bytes([value[0], value[1], value[2], value[3]]));
                }
                OPT_REBINDING_TIME if length == 4 => {
                    message.t2_secs =
                        Some(u32::from_be_bytes([value[0], value[1], value[2], value[3]]));
                }
                OPT_SERVER_ID if length == 4 => {
                    message.server_id = Some([value[0], value[1], value[2], value[3]]);
                }
                OPT_REQUESTED_IP if length == 4 => {
                    message.requested_ip = Some([value[0], value[1], value[2], value[3]]);
                }
                _ => {}
            }
        }
        message.message_type = message_type.ok_or(DhcpError::InvalidPacket)?;
        // Server replies are BOOTREPLY.
        if input[0] != 2 {
            return Err(DhcpError::InvalidPacket);
        }
        Ok(message)
    }

    fn into_offer(self) -> Result<DhcpOffer, DhcpError> {
        if self.message_type != DhcpMessageType::Offer {
            return Err(DhcpError::InvalidPacket);
        }
        let lease_time = self.lease_time_secs.filter(|value| *value > 0).ok_or(DhcpError::InvalidLease)?;
        let subnet_mask = self.subnet_mask.ok_or(DhcpError::InvalidLease)?;
        let server_id = self.server_id.ok_or(DhcpError::InvalidLease)?;
        if self.yiaddr == [0; 4] {
            return Err(DhcpError::InvalidLease);
        }
        let t1 = self.t1_secs.unwrap_or(lease_time / 2).max(1);
        let t2 = self
            .t2_secs
            .unwrap_or(lease_time.saturating_mul(7) / 8)
            .max(t1);
        if t1 >= lease_time || t2 > lease_time || t1 > t2 {
            return Err(DhcpError::InvalidLease);
        }
        Ok(DhcpOffer {
            xid: self.xid,
            yiaddr: self.yiaddr,
            server_id,
            subnet_mask,
            gateway: self.gateway,
            dns: self.dns,
            dns_count: self.dns_count,
            lease_time_secs: lease_time,
            t1_secs: t1,
            t2_secs: t2,
            chaddr: self.chaddr,
        })
    }

    fn into_lease(self) -> Result<DhcpLease, DhcpError> {
        if self.message_type != DhcpMessageType::Ack {
            return Err(DhcpError::InvalidPacket);
        }
        let mut normalized = self;
        normalized.message_type = DhcpMessageType::Offer;
        let offer = normalized.into_offer()?;
        Ok(DhcpLease {
            address: offer.yiaddr,
            subnet_mask: offer.subnet_mask,
            gateway: offer.gateway,
            dns: offer.dns,
            dns_count: offer.dns_count,
            server_id: offer.server_id,
            lease_time_secs: offer.lease_time_secs,
            t1_secs: offer.t1_secs,
            t2_secs: offer.t2_secs,
            yiaddr: offer.yiaddr,
        })
    }
}

fn write_option(output: &mut [u8], cursor: usize, code: u8, value: &[u8]) -> Result<usize, DhcpError> {
    let end = cursor
        .checked_add(2 + value.len())
        .ok_or(DhcpError::Capacity)?;
    if end > output.len() {
        return Err(DhcpError::Capacity);
    }
    output[cursor] = code;
    output[cursor + 1] = value.len() as u8;
    output[cursor + 2..end].copy_from_slice(value);
    Ok(end)
}

fn next_xid(seed: u32, attempt: u8) -> u32 {
    seed.wrapping_mul(0x9E37_79B9).wrapping_add(u32::from(attempt)).wrapping_add(1)
}

/// Firewall rules and rate limits for DHCP client traffic on UDP 67/68.
pub fn dhcp_client_firewall_rules() -> [FirewallRule; 2] {
    let rate = RateLimit::new(8, 1_000);
    [
        FirewallRule {
            direction: Some(Direction::Egress),
            protocol: Some(Protocol::Udp),
            source: None,
            destination: Some(Ipv4Cidr::ANY),
            source_ports: PortRange::new(DHCP_CLIENT_PORT, DHCP_CLIENT_PORT),
            destination_ports: PortRange::new(DHCP_SERVER_PORT, DHCP_SERVER_PORT),
            action: RuleAction::Allow,
            stateful: false,
            capability: Some(CapabilityRight::Raw),
            rate_limit: rate,
        },
        FirewallRule {
            direction: Some(Direction::Ingress),
            protocol: Some(Protocol::Udp),
            source: None,
            destination: Some(Ipv4Cidr::ANY),
            source_ports: PortRange::new(DHCP_SERVER_PORT, DHCP_SERVER_PORT),
            destination_ports: PortRange::new(DHCP_CLIENT_PORT, DHCP_CLIENT_PORT),
            action: RuleAction::Allow,
            stateful: false,
            capability: Some(CapabilityRight::Ingress),
            rate_limit: rate,
        },
    ]
}

pub fn install_dhcp_client_rules<const RULES: usize>(
    policy: &mut crate::firewall::FirewallPolicy<RULES>,
) -> Result<(), crate::firewall::FirewallError> {
    for rule in dhcp_client_firewall_rules() {
        policy.add_rule(rule)?;
    }
    Ok(())
}

/// Deterministic DHCP server fixture used by tests.
pub struct DhcpServerFixture {
    pub server_id: [u8; 4],
    pub offered_ip: [u8; 4],
    pub subnet_mask: [u8; 4],
    pub gateway: [u8; 4],
    pub dns: [u8; 4],
    pub lease_time_secs: u32,
    pub t1_secs: u32,
    pub t2_secs: u32,
    pub mac: [u8; 6],
}

impl DhcpServerFixture {
    pub fn respond(&self, request: &[u8]) -> Result<[u8; MAX_DHCP_PACKET], DhcpError> {
        let message = DhcpMessage::decode_request(request)?;
        if message.chaddr != self.mac {
            return Err(DhcpError::InvalidPacket);
        }
        let reply_type = match message.message_type {
            DhcpMessageType::Discover => DhcpMessageType::Offer,
            DhcpMessageType::Request => DhcpMessageType::Ack,
            _ => return Err(DhcpError::InvalidPacket),
        };
        let mut reply = DhcpMessage::new(reply_type, message.xid, self.mac);
        reply.yiaddr = self.offered_ip;
        reply.siaddr = self.server_id;
        reply.subnet_mask = Some(self.subnet_mask);
        reply.gateway = Some(self.gateway);
        reply.dns = [self.dns, [0; 4]];
        reply.dns_count = 1;
        reply.lease_time_secs = Some(self.lease_time_secs);
        reply.t1_secs = Some(self.t1_secs);
        reply.t2_secs = Some(self.t2_secs);
        reply.server_id = Some(self.server_id);
        let mut packet = [0; MAX_DHCP_PACKET];
        let length = reply.encode_reply(&mut packet)?;
        let mut out = [0; MAX_DHCP_PACKET];
        out[..length].copy_from_slice(&packet[..length]);
        Ok(out)
    }

    pub fn nak(&self, request: &[u8]) -> Result<[u8; MAX_DHCP_PACKET], DhcpError> {
        let message = DhcpMessage::decode_request(request)?;
        let mut reply = DhcpMessage::new(DhcpMessageType::Nak, message.xid, self.mac);
        reply.server_id = Some(self.server_id);
        let mut packet = [0; MAX_DHCP_PACKET];
        let length = reply.encode_reply(&mut packet)?;
        let mut out = [0; MAX_DHCP_PACKET];
        out[..length].copy_from_slice(&packet[..length]);
        Ok(out)
    }
}

impl DhcpMessage {
    fn decode_request(input: &[u8]) -> Result<Self, DhcpError> {
        if input.len() < 241 || input[0] != 1 {
            return Err(DhcpError::InvalidPacket);
        }
        // Temporarily treat as reply decode by cloning into a buffer with op=2.
        let mut buffer = [0; MAX_DHCP_PACKET];
        let length = core::cmp::min(input.len(), MAX_DHCP_PACKET);
        buffer[..length].copy_from_slice(&input[..length]);
        buffer[0] = 2;
        Self::decode(&buffer[..length])
    }

    fn encode_reply(self, output: &mut [u8]) -> Result<usize, DhcpError> {
        let length = self.encode(output)?;
        output[0] = 2; // BOOTREPLY
        // Re-encode options after flipping op — encode already wrote options.
        // Ensure lease options are present for Offer/Ack.
        if matches!(
            self.message_type,
            DhcpMessageType::Offer | DhcpMessageType::Ack
        ) {
            // encode() only writes request options; rewrite reply options.
            output[240..].fill(0);
            let mut cursor = 240;
            cursor = write_option(output, cursor, OPT_MESSAGE_TYPE, &[self.message_type as u8])?;
            if let Some(mask) = self.subnet_mask {
                cursor = write_option(output, cursor, OPT_SUBNET_MASK, &mask)?;
            }
            if let Some(gateway) = self.gateway {
                cursor = write_option(output, cursor, OPT_ROUTER, &gateway)?;
            }
            if self.dns_count > 0 {
                cursor = write_option(output, cursor, OPT_DNS, &self.dns[0])?;
            }
            if let Some(lease) = self.lease_time_secs {
                cursor = write_option(output, cursor, OPT_LEASE_TIME, &lease.to_be_bytes())?;
            }
            if let Some(t1) = self.t1_secs {
                cursor = write_option(output, cursor, OPT_RENEWAL_TIME, &t1.to_be_bytes())?;
            }
            if let Some(t2) = self.t2_secs {
                cursor = write_option(output, cursor, OPT_REBINDING_TIME, &t2.to_be_bytes())?;
            }
            if let Some(server) = self.server_id {
                cursor = write_option(output, cursor, OPT_SERVER_ID, &server)?;
            }
            if cursor >= output.len() {
                return Err(DhcpError::Capacity);
            }
            output[cursor] = OPT_END;
            return Ok(cursor + 1);
        }
        if self.message_type == DhcpMessageType::Nak {
            output[240..].fill(0);
            let mut cursor = 240;
            cursor = write_option(output, cursor, OPT_MESSAGE_TYPE, &[DhcpMessageType::Nak as u8])?;
            if let Some(server) = self.server_id {
                cursor = write_option(output, cursor, OPT_SERVER_ID, &server)?;
            }
            output[cursor] = OPT_END;
            return Ok(cursor + 1);
        }
        Ok(length)
    }
}

/// Format an IPv4 address into `buffer`, returning the written length.
pub fn format_ipv4(address: [u8; 4], buffer: &mut [u8]) -> Result<usize, DhcpError> {
    let mut cursor = 0;
    for (index, octet) in address.iter().enumerate() {
        if index != 0 {
            if cursor >= buffer.len() {
                return Err(DhcpError::Capacity);
            }
            buffer[cursor] = b'.';
            cursor += 1;
        }
        let mut value = *octet;
        let mut digits = [0u8; 3];
        let mut count = 0;
        loop {
            digits[count] = b'0' + (value % 10);
            count += 1;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        while count != 0 {
            count -= 1;
            if cursor >= buffer.len() {
                return Err(DhcpError::Capacity);
            }
            buffer[cursor] = digits[count];
            cursor += 1;
        }
    }
    Ok(cursor)
}
