//! Capability-gated IPv4 DHCP client with bounded DORA handling.
//!
//! The client binds to one Ethernet interface, validates transaction identity
//! and lease options, and applies accepted leases through a runtime bridge so
//! previous static configuration can be restored on failure or expiry.

use crate::capture::{CaptureDirection, PacketCapture, MAX_CAPTURE_RECORDS};
use crate::firewall::{
    CapabilityRight, Direction, FirewallRule, Ipv4Cidr, PortRange, Protocol, RateLimit, RuleAction,
};
use crate::transport::DhcpIngress;
use synos_time_sync::MonotonicClock;

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
const OPT_CLASSLESS_ROUTE: u8 = 121;
const OPT_END: u8 = 255;

/// Initial discover/request backoff schedule in milliseconds (bounded).
pub const BACKOFF_MS: [u64; 5] = [4_000, 8_000, 16_000, 32_000, 64_000];
pub const MAX_DISCOVER_ATTEMPTS: u8 = 8;
pub const RETRY_JITTER_PERCENT: u64 = 50;
pub const MAX_RETRY_DELAY_MS: u64 = BACKOFF_MS[BACKOFF_MS.len() - 1];
pub const DHCP_LEASE_RECORD_VERSION: u8 = 1;
pub const DHCP_LEASE_RECORD_BYTES: usize = 166;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DhcpNetworkError {
    MissingNic,
    LinkDown,
    QueueFull,
    AdminDown,
    BackendUnavailable,
}

impl DhcpNetworkError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::MissingNic => "NET_MISSING_NIC",
            Self::LinkDown => "NET_LINK_DOWN",
            Self::QueueFull => "NET_QUEUE_FULL",
            Self::AdminDown => "NET_ADMIN_DOWN",
            Self::BackendUnavailable => "NET_BACKEND_UNAVAILABLE",
        }
    }
}

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
    Network(DhcpNetworkError),
}

impl DhcpError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::AccessDenied => "DHCP_ACCESS_DENIED",
            Self::Capacity => "DHCP_CAPACITY",
            Self::InvalidPacket => "DHCP_INVALID_PACKET",
            Self::InvalidLease => "DHCP_INVALID_LEASE",
            Self::InvalidState => "DHCP_INVALID_STATE",
            Self::InvalidInterface => "DHCP_INVALID_INTERFACE",
            Self::NoOffer => "DHCP_NO_OFFER",
            Self::ConflictingOffer => "DHCP_CONFLICTING_OFFER",
            Self::ServerUnavailable => "DHCP_SERVER_UNAVAILABLE",
            Self::Runtime => "DHCP_RUNTIME",
            Self::Network(error) => error.code(),
        }
    }
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
    Error = 7,
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
            Self::Error => "error",
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
pub struct DhcpRoute {
    pub destination: [u8; 4],
    pub prefix_len: u8,
    pub gateway: [u8; 4],
}

impl DhcpRoute {
    pub const DEFAULT: Self = Self {
        destination: [0; 4],
        prefix_len: 0,
        gateway: [0; 4],
    };
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
    pub routes: [DhcpRoute; MAX_DHCP_ROUTES],
    pub route_count: u8,
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

/// Fixed-size, versioned lease metadata suitable for durable storage.
/// Remaining times are stored relative to the persistence point so a reboot
/// does not mistake a reset monotonic clock for lease time remaining.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DhcpLeaseRecord {
    pub version: u8,
    pub interface: [u8; MAX_INTERFACE_NAME],
    pub interface_len: u8,
    pub client_mac: [u8; 6],
    pub server_mac: Option<[u8; 6]>,
    pub lease: DhcpLease,
    pub remaining_lease_ms: u64,
    pub remaining_t1_ms: u64,
    pub remaining_t2_ms: u64,
}

impl DhcpLeaseRecord {
    pub fn encode(self, output: &mut [u8]) -> Result<usize, DhcpError> {
        if output.len() < DHCP_LEASE_RECORD_BYTES {
            return Err(DhcpError::Capacity);
        }
        validate_lease_record(self)?;
        let mut cursor = 0;
        record_put_u8(output, &mut cursor, self.version)?;
        record_put_u8(output, &mut cursor, self.interface_len)?;
        record_put_bytes(output, &mut cursor, &self.interface)?;
        record_put_bytes(output, &mut cursor, &self.client_mac)?;
        match self.server_mac {
            Some(mac) => {
                record_put_u8(output, &mut cursor, 1)?;
                record_put_bytes(output, &mut cursor, &mac)?;
            }
            None => {
                record_put_u8(output, &mut cursor, 0)?;
                record_put_bytes(output, &mut cursor, &[0; 6])?;
            }
        }
        record_put_u64(output, &mut cursor, self.remaining_lease_ms)?;
        record_put_u64(output, &mut cursor, self.remaining_t1_ms)?;
        record_put_u64(output, &mut cursor, self.remaining_t2_ms)?;
        encode_lease_record_lease(output, &mut cursor, self.lease)?;
        if cursor != DHCP_LEASE_RECORD_BYTES {
            return Err(DhcpError::Capacity);
        }
        Ok(cursor)
    }

    pub fn decode(input: &[u8]) -> Result<Self, DhcpError> {
        if input.len() != DHCP_LEASE_RECORD_BYTES {
            return Err(DhcpError::InvalidLease);
        }
        let mut cursor = 0;
        let version = record_get_u8(input, &mut cursor)?;
        let interface_len = record_get_u8(input, &mut cursor)?;
        let mut interface = [0; MAX_INTERFACE_NAME];
        interface.copy_from_slice(record_get_bytes(input, &mut cursor, MAX_INTERFACE_NAME)?);
        let mut client_mac = [0; 6];
        client_mac.copy_from_slice(record_get_bytes(input, &mut cursor, 6)?);
        let server_mac_present = record_get_u8(input, &mut cursor)?;
        let mut server_mac_bytes = [0; 6];
        server_mac_bytes.copy_from_slice(record_get_bytes(input, &mut cursor, 6)?);
        let server_mac = match server_mac_present {
            0 => None,
            1 => Some(server_mac_bytes),
            _ => return Err(DhcpError::InvalidLease),
        };
        let remaining_lease_ms = record_get_u64(input, &mut cursor)?;
        let remaining_t1_ms = record_get_u64(input, &mut cursor)?;
        let remaining_t2_ms = record_get_u64(input, &mut cursor)?;
        let lease = decode_lease_record_lease(input, &mut cursor)?;
        let record = Self {
            version,
            interface,
            interface_len,
            client_mac,
            server_mac,
            lease,
            remaining_lease_ms,
            remaining_t1_ms,
            remaining_t2_ms,
        };
        validate_lease_record(record)?;
        if cursor != DHCP_LEASE_RECORD_BYTES {
            return Err(DhcpError::InvalidLease);
        }
        Ok(record)
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
    pub routes: [DhcpRoute; MAX_DHCP_ROUTES],
    pub route_count: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DhcpInterfaceState {
    pub enabled: bool,
    pub link_up: bool,
    pub configured: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DhcpLeaseApplication {
    pub lease: DhcpLease,
    pub interface: DhcpInterfaceState,
}

/// Applies or restores the complete interface configuration through the
/// network configuration runtime. Implementations of `apply_lease_atomically`
/// must commit address, routes, DNS, timers, identity, and interface state as
/// one transaction or leave the previous configuration untouched.
pub trait DhcpLeaseRuntime {
    fn apply_lease(&mut self, interface: &str, lease: &DhcpLease) -> Result<(), DhcpError>;
    fn restore_static(&mut self, interface: &str, snapshot: &StaticSnapshot) -> Result<(), DhcpError>;

    /// Check a candidate lease while the currently published configuration is
    /// still active. This must not publish the candidate.
    fn health_check_lease(
        &mut self,
        _interface: &str,
        _application: &DhcpLeaseApplication,
    ) -> Result<(), DhcpError> {
        Ok(())
    }

    fn apply_lease_atomically(
        &mut self,
        interface: &str,
        application: &DhcpLeaseApplication,
    ) -> Result<(), DhcpError> {
        self.apply_lease(interface, &application.lease)
    }

    /// Reconcile DHCP-owned address, routes, and DNS from the old lease to
    /// the candidate lease. Implementations must leave non-DHCP routes and
    /// DNS entries untouched, including entries with the same value as a
    /// DHCP entry.
    fn reconcile_dhcp_lease(
        &mut self,
        interface: &str,
        _previous: Option<&DhcpLease>,
        application: &DhcpLeaseApplication,
    ) -> Result<(), DhcpError> {
        self.apply_lease_atomically(interface, application)
    }

    /// Remove only state owned by the lease. The preserved snapshot is the
    /// previous static interface state, not permission to delete unrelated
    /// routes or DNS entries.
    fn remove_dhcp_state(
        &mut self,
        interface: &str,
        _lease: &DhcpLease,
        preserved: Option<&StaticSnapshot>,
    ) -> Result<(), DhcpError> {
        if let Some(snapshot) = preserved {
            self.restore_static(interface, snapshot)?;
        }
        Ok(())
    }
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

/// DHCP transport decorator that retains the exact bounded UDP packets sent
/// by the client. The caller records replies after validating their arrival.
pub struct CapturingDhcpTransport<T, const CAPACITY: usize = MAX_CAPTURE_RECORDS> {
    pub transport: T,
    pub capture: PacketCapture<CAPACITY>,
    pub now_ms: u64,
}

impl<T, const CAPACITY: usize> CapturingDhcpTransport<T, CAPACITY> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            capture: PacketCapture::new(),
            now_ms: 0,
        }
    }

    pub fn set_timestamp(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
    }
}

impl<T: DhcpTransport, const CAPACITY: usize> DhcpTransport
    for CapturingDhcpTransport<T, CAPACITY>
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
        self.transport.send_udp(
            interface, src_mac, src_ip, dst_ip, dst_mac, src_port, dst_port, payload,
        )?;
        self.capture.record_packet(
            self.now_ms,
            CaptureDirection::Egress,
            src_mac,
            dst_mac,
            src_ip,
            dst_ip,
            src_port,
            dst_port,
            payload,
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DhcpClientView {
    pub interface: [u8; MAX_INTERFACE_NAME],
    pub interface_len: u8,
    pub state: DhcpClientState,
    pub enabled: bool,
    pub link_up: bool,
    pub xid: u32,
    pub attempt: u8,
    pub lease: Option<DhcpLease>,
    pub bound_at_ms: Option<u64>,
    pub next_action_ms: Option<u64>,
    pub preserved: Option<StaticSnapshot>,
    pub network_error: Option<DhcpNetworkError>,
    pub server_mac: Option<[u8; 6]>,
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
    enabled: bool,
    link_up: bool,
    authorized: bool,
    xid: u32,
    attempt: u8,
    next_action_ms: Option<u64>,
    selected: Option<PendingOffer>,
    lease: Option<DhcpLease>,
    bound_at_ms: Option<u64>,
    preserved: Option<StaticSnapshot>,
    last_network_error: Option<DhcpNetworkError>,
    server_mac: Option<[u8; 6]>,
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
            enabled: true,
            link_up: true,
            authorized: false,
            xid: 0,
            attempt: 0,
            next_action_ms: Some(0),
            selected: None,
            lease: None,
            bound_at_ms: None,
            preserved: None,
            last_network_error: None,
            server_mac: None,
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

    pub const fn next_action_ms(&self) -> Option<u64> {
        self.next_action_ms
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
            enabled: self.enabled,
            link_up: self.link_up,
            xid: self.xid,
            attempt: self.attempt,
            lease: self.lease,
            bound_at_ms: self.bound_at_ms,
            next_action_ms: self.next_action_ms,
            preserved: self.preserved,
            network_error: self.last_network_error,
            server_mac: self.server_mac,
        }
    }

    pub const fn network_error(&self) -> Option<DhcpNetworkError> {
        self.last_network_error
    }

    pub const fn server_mac(&self) -> Option<[u8; 6]> {
        self.server_mac
    }

    pub fn take_network_error(&mut self) -> Option<DhcpNetworkError> {
        self.last_network_error.take()
    }

    /// Stop the DHCP scheduler and publish the first network failure. The
    /// caller may clear the failure by changing the NIC state or calling
    /// `start` after the backend is healthy again.
    pub fn report_network_error(&mut self, error: DhcpNetworkError) {
        if self.last_network_error.is_none() {
            self.last_network_error = Some(error);
        }
        self.selected = None;
        self.next_action_ms = None;
        self.state = DhcpClientState::Error;
    }

    pub fn preserve_static(&mut self, snapshot: StaticSnapshot) {
        self.preserved = Some(snapshot);
    }

    pub fn persist_lease(&self, now_ms: u64) -> Result<DhcpLeaseRecord, DhcpError> {
        let lease = self.lease.ok_or(DhcpError::InvalidState)?;
        let bound_at_ms = self.bound_at_ms.ok_or(DhcpError::InvalidState)?;
        let expires_at_ms = lease.expires_at_ms(bound_at_ms);
        if now_ms >= expires_at_ms {
            return Err(DhcpError::InvalidLease);
        }
        let record = DhcpLeaseRecord {
            version: DHCP_LEASE_RECORD_VERSION,
            interface: self.interface,
            interface_len: self.interface_len,
            client_mac: self.mac,
            server_mac: self.server_mac,
            lease,
            remaining_lease_ms: expires_at_ms.saturating_sub(now_ms),
            remaining_t1_ms: lease.renew_at_ms(bound_at_ms).saturating_sub(now_ms),
            remaining_t2_ms: lease.rebind_at_ms(bound_at_ms).saturating_sub(now_ms),
        };
        validate_lease_record(record)?;
        Ok(record)
    }

    pub fn recover_lease(
        &mut self,
        record: DhcpLeaseRecord,
        now_ms: u64,
        elapsed_since_persist_ms: u64,
        expected_server_id: Option<[u8; 4]>,
        expected_server_mac: Option<[u8; 6]>,
    ) -> Result<(), DhcpError> {
        self.require_auth()?;
        validate_lease_record(record)?;
        if record.version != DHCP_LEASE_RECORD_VERSION
            || record.interface_len != self.interface_len
            || record.interface[..record.interface_len as usize]
                != self.interface[..self.interface_len as usize]
            || record.client_mac != self.mac
        {
            return Err(DhcpError::InvalidInterface);
        }
        if expected_server_id.is_some_and(|server_id| server_id != record.lease.server_id)
            || expected_server_mac.is_some_and(|server_mac| record.server_mac != Some(server_mac))
        {
            return Err(DhcpError::ConflictingOffer);
        }
        if elapsed_since_persist_ms >= record.remaining_lease_ms {
            return Err(DhcpError::InvalidLease);
        }
        let remaining_lease_ms = record
            .remaining_lease_ms
            .saturating_sub(elapsed_since_persist_ms);
        let lease_duration_ms = (record.lease.lease_time_secs as u64).saturating_mul(1_000);
        let lease_age_ms = lease_duration_ms.saturating_sub(remaining_lease_ms);
        self.lease = Some(record.lease);
        self.bound_at_ms = Some(now_ms.saturating_sub(lease_age_ms));
        self.server_mac = record.server_mac;
        self.selected = None;
        self.attempt = 0;
        self.last_network_error = None;
        self.state = DhcpClientState::InitReboot;
        self.next_action_ms = Some(now_ms);
        Ok(())
    }

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Changes administrative state without changing physical carrier state.
    /// A disabled client retains its lease for safe init-reboot recovery, but
    /// never schedules or transmits DHCP packets.
    pub fn set_enabled(&mut self, enabled: bool, now_ms: u64) {
        self.enabled = enabled;
        if !enabled {
            self.report_network_error(DhcpNetworkError::AdminDown);
            return
        }
        if !self.link_up {
            self.next_action_ms = None;
            return
        }
        self.last_network_error = None;
        self.state = if self.lease.is_some() {
            DhcpClientState::InitReboot
        } else {
            DhcpClientState::Init
        };
        self.attempt = 0;
        self.selected = None;
        self.next_action_ms = Some(now_ms);
    }

    pub fn set_enabled_with_clock<C: MonotonicClock>(&mut self, enabled: bool, clock: &C) {
        self.set_enabled(enabled, clock.now_us() / 1_000)
    }

    pub fn set_link(&mut self, up: bool, now_ms: u64) {
        self.link_up = up;
        if !up || !self.enabled {
            self.report_network_error(if up {
                DhcpNetworkError::AdminDown
            } else {
                DhcpNetworkError::LinkDown
            });
            return;
        }
        self.last_network_error = None;
        self.state = if self.lease.is_some() {
            DhcpClientState::InitReboot
        } else {
            DhcpClientState::Init
        };
        self.attempt = 0;
        self.selected = None;
        self.next_action_ms = Some(now_ms);
    }

    pub fn set_link_with_clock<C: MonotonicClock>(&mut self, up: bool, clock: &C) {
        self.set_link(up, clock.now_us() / 1_000)
    }

    pub fn start(&mut self, now_ms: u64) -> Result<(), DhcpError> {
        self.require_auth()?;
        if !self.enabled || !self.link_up {
            return Err(DhcpError::InvalidState);
        }
        self.last_network_error = None;
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

    pub fn start_with_clock<C: MonotonicClock>(&mut self, clock: &C) -> Result<(), DhcpError> {
        self.start(clock.now_us() / 1_000)
    }

    pub fn poll<T: DhcpTransport, R: DhcpLeaseRuntime>(
        &mut self,
        now_ms: u64,
        transport: &mut T,
        runtime: &mut R,
    ) -> Result<(), DhcpError> {
        self.require_auth()?;
        if !self.enabled || !self.link_up {
            return Ok(());
        }
        if let (Some(lease), Some(bound_at)) = (self.lease, self.bound_at_ms) {
            if now_ms >= lease.expires_at_ms(bound_at) {
                return self.expire(now_ms, runtime);
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
            DhcpClientState::Bound | DhcpClientState::Error => Ok(()),
        };
        match result {
            Err(DhcpError::ServerUnavailable) => {
                // Keep the last-known-good lease during a transient outage.
                let _ = self.restore_fallback(runtime);
            }
            Err(DhcpError::Network(error)) => {
                let _ = self.restore_fallback(runtime);
                self.report_network_error(error);
            }
            _ => {}
        }
        result
    }

    pub fn poll_with_clock<C: MonotonicClock, T: DhcpTransport, R: DhcpLeaseRuntime>(
        &mut self,
        clock: &C,
        transport: &mut T,
        runtime: &mut R,
    ) -> Result<(), DhcpError> {
        self.poll(clock.now_us() / 1_000, transport, runtime)
    }

    pub fn handle_packet<R: DhcpLeaseRuntime>(
        &mut self,
        packet: &[u8],
        now_ms: u64,
        runtime: &mut R,
    ) -> Result<(), DhcpError> {
        self.require_auth()?;
        if !self.enabled || !self.link_up {
            return Err(DhcpError::InvalidState)
        }
        let message = DhcpMessage::decode(packet)?;
        if message.xid != self.xid || message.chaddr != self.mac {
            return Err(DhcpError::InvalidPacket);
        }
        let result = match message.message_type {
            DhcpMessageType::Offer => self.handle_offer(message, now_ms),
            DhcpMessageType::Ack => self.handle_ack(message, now_ms, runtime),
            DhcpMessageType::Nak => self.handle_nak(now_ms, runtime),
            _ => Err(DhcpError::InvalidPacket),
        };
        if let Err(DhcpError::Network(error)) = result {
            self.report_network_error(error);
        }
        result
    }

    pub fn handle_frame<R: DhcpLeaseRuntime>(
        &mut self,
        frame: DhcpIngress<'_>,
        now_ms: u64,
        runtime: &mut R,
    ) -> Result<(), DhcpError> {
        let source_mac = frame.source_mac;
        if !self.accept_server_mac(source_mac) {
            return Err(DhcpError::ConflictingOffer);
        }
        let result = self.handle_packet(frame.payload, now_ms, runtime);
        if result.is_ok() && source_mac != [0; 6] && source_mac != [0xff; 6] {
            self.server_mac = Some(source_mac);
        }
        result
    }

    fn accept_server_mac(&self, source_mac: [u8; 6]) -> bool {
        if source_mac == [0; 6] || source_mac == [0xff; 6] {
            return true;
        }
        let Some(server_mac) = self.server_mac else {
            return true;
        };
        matches!(self.state, DhcpClientState::Rebinding | DhcpClientState::InitReboot)
            || source_mac == server_mac
    }

    pub fn handle_packet_with_clock<C: MonotonicClock, R: DhcpLeaseRuntime>(
        &mut self,
        packet: &[u8],
        clock: &C,
        runtime: &mut R,
    ) -> Result<(), DhcpError> {
        self.handle_packet(packet, clock.now_us() / 1_000, runtime)
    }

    pub fn release<T: DhcpTransport, R: DhcpLeaseRuntime>(
        &mut self,
        now_ms: u64,
        transport: &mut T,
        runtime: &mut R,
    ) -> Result<(), DhcpError> {
        self.require_auth()?;
        if !self.enabled || !self.link_up {
            return Err(DhcpError::InvalidState)
        }
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
            self.server_mac.unwrap_or([0xff; 6]),
            DHCP_CLIENT_PORT,
            DHCP_SERVER_PORT,
            &packet[..length],
        )?;
        self.clear_lease(runtime)?;
        self.state = DhcpClientState::Init;
        self.next_action_ms = Some(now_ms);
        Ok(())
    }

    pub fn release_with_clock<C: MonotonicClock, T: DhcpTransport, R: DhcpLeaseRuntime>(
        &mut self,
        clock: &C,
        transport: &mut T,
        runtime: &mut R,
    ) -> Result<(), DhcpError> {
        self.release(clock.now_us() / 1_000, transport, runtime)
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
        let lease = message.into_lease()?;
        if self.state == DhcpClientState::Bound {
            return if self.lease == Some(lease) {
                Ok(())
            } else {
                Err(DhcpError::ConflictingOffer)
            };
        }
        if !matches!(
            self.state,
            DhcpClientState::Requesting
                | DhcpClientState::InitReboot
                | DhcpClientState::Renewing
                | DhcpClientState::Rebinding
        ) {
            return Err(DhcpError::InvalidState);
        }
        if let Some(selected) = self.selected {
            if selected.offer.server_id != lease.server_id || selected.offer.yiaddr != lease.yiaddr {
                return Err(DhcpError::ConflictingOffer);
            }
        }
        if self.state == DhcpClientState::Renewing
            && self.lease.is_some_and(|current| current.server_id != lease.server_id)
        {
            return Err(DhcpError::ConflictingOffer);
        }
        if self.state == DhcpClientState::InitReboot
            && self.lease.is_some_and(|current| current.address != lease.address)
        {
            return Err(DhcpError::ConflictingOffer);
        }
        let application = DhcpLeaseApplication {
            lease,
            interface: DhcpInterfaceState {
                enabled: self.enabled,
                link_up: self.link_up,
                configured: true,
            },
        };
        runtime.health_check_lease(self.interface_name(), &application)?;
        let previous = self.lease;
        runtime.reconcile_dhcp_lease(self.interface_name(), previous.as_ref(), &application)?;
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
            if self.lease.is_none() {
                self.clear_lease(runtime)?;
            }
            self.selected = None;
            self.state = DhcpClientState::Init;
            self.attempt = 0;
            self.next_action_ms = Some(now_ms);
            Ok(())
        } else {
            Err(DhcpError::InvalidState)
        }
    }

    fn expire<R: DhcpLeaseRuntime>(
        &mut self,
        now_ms: u64,
        runtime: &mut R,
    ) -> Result<(), DhcpError> {
        self.clear_lease(runtime)?;
        self.state = DhcpClientState::Init;
        self.attempt = 0;
        self.selected = None;
        self.next_action_ms = Some(now_ms);
        Ok(())
    }

    fn clear_lease<R: DhcpLeaseRuntime>(&mut self, runtime: &mut R) -> Result<(), DhcpError> {
        if let Some(lease) = self.lease {
            runtime.remove_dhcp_state(
                self.interface_name(),
                &lease,
                self.preserved.as_ref(),
            )?;
        }
        self.lease = None;
        self.bound_at_ms = None;
        self.server_mac = None;
        Ok(())
    }

    fn restore_fallback<R: DhcpLeaseRuntime>(&mut self, runtime: &mut R) -> Result<(), DhcpError> {
        if self.lease.is_none() {
            if let Some(snapshot) = self.preserved {
                runtime.restore_static(self.interface_name(), &snapshot)?;
            }
        }
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
            // A failed REQUESTING or INIT-REBOOT exchange must restart with
            // discovery instead of leaving the client permanently idle.
            self.selected = None;
            self.state = DhcpClientState::Init;
            self.attempt = 0;
            self.next_action_ms = Some(now_ms);
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
            self.server_mac.unwrap_or([0xff; 6]),
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
        let base = BACKOFF_MS[index];
        let jitter_window = base.saturating_mul(RETRY_JITTER_PERCENT) / 100;
        let jitter = u64::from(retry_jitter(self.txid_seed, self.attempt))
            % jitter_window.saturating_add(1);
        let delay = base.saturating_sub(jitter).max(1);
        self.next_action_ms = Some(now_ms.saturating_add(delay));
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
    routes: [DhcpRoute; MAX_DHCP_ROUTES],
    route_count: u8,
    classless_routes: bool,
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
            routes: [DhcpRoute::DEFAULT; MAX_DHCP_ROUTES],
            route_count: 0,
            classless_routes: false,
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
                &[
                    OPT_SUBNET_MASK,
                    OPT_ROUTER,
                    OPT_DNS,
                    OPT_LEASE_TIME,
                    OPT_RENEWAL_TIME,
                    OPT_REBINDING_TIME,
                    OPT_CLASSLESS_ROUTE,
                ],
            )?;
        }
        if cursor >= output.len() {
            return Err(DhcpError::Capacity);
        }
        output[cursor] = OPT_END;
        Ok(cursor + 1)
    }

    fn decode(input: &[u8]) -> Result<Self, DhcpError> {
        if input.len() < 241
            || input.len() > MAX_DHCP_PACKET
            || input[1] != 1
            || input[2] != 6
        {
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
        let mut ended = false;
        while index < input.len() {
            let code = input[index];
            index += 1;
            if code == OPT_END {
                ended = true;
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
            let end = index
                .checked_add(length)
                .ok_or(DhcpError::InvalidPacket)?;
            if end > input.len() {
                return Err(DhcpError::InvalidPacket);
            }
            let value = &input[index..end];
            index = end;
            match code {
                OPT_MESSAGE_TYPE => {
                    if length != 1 || message_type.is_some() {
                        return Err(DhcpError::InvalidPacket);
                    }
                    message_type = Some(
                        DhcpMessageType::from_raw(value[0])
                            .ok_or(DhcpError::InvalidPacket)?,
                    );
                }
                OPT_SUBNET_MASK => {
                    if length != 4 || message.subnet_mask.is_some() {
                        return Err(DhcpError::InvalidPacket);
                    }
                    message.subnet_mask = Some([value[0], value[1], value[2], value[3]]);
                }
                OPT_ROUTER => {
                    if length != 4 || message.gateway.is_some() {
                        return Err(DhcpError::InvalidPacket);
                    }
                    message.gateway = Some([value[0], value[1], value[2], value[3]]);
                }
                OPT_DNS => {
                    if length == 0
                        || length % 4 != 0
                        || length / 4 > MAX_DHCP_DNS_SERVERS
                        || message.dns_count != 0
                    {
                        return Err(DhcpError::InvalidPacket);
                    }
                    let mut count = 0u8;
                    let mut dns = [[0; 4]; MAX_DHCP_DNS_SERVERS];
                    let mut offset = 0;
                    while offset < value.len() {
                        dns[count as usize] =
                            [value[offset], value[offset + 1], value[offset + 2], value[offset + 3]];
                        count += 1;
                        offset += 4;
                    }
                    message.dns = dns;
                    message.dns_count = count;
                }
                OPT_LEASE_TIME => {
                    if length != 4 || message.lease_time_secs.is_some() {
                        return Err(DhcpError::InvalidPacket);
                    }
                    message.lease_time_secs =
                        Some(u32::from_be_bytes([value[0], value[1], value[2], value[3]]));
                }
                OPT_RENEWAL_TIME => {
                    if length != 4 || message.t1_secs.is_some() {
                        return Err(DhcpError::InvalidPacket);
                    }
                    message.t1_secs =
                        Some(u32::from_be_bytes([value[0], value[1], value[2], value[3]]));
                }
                OPT_REBINDING_TIME => {
                    if length != 4 || message.t2_secs.is_some() {
                        return Err(DhcpError::InvalidPacket);
                    }
                    message.t2_secs =
                        Some(u32::from_be_bytes([value[0], value[1], value[2], value[3]]));
                }
                OPT_CLASSLESS_ROUTE => {
                    if value.is_empty() || message.classless_routes {
                        return Err(DhcpError::InvalidPacket);
                    }
                    message.classless_routes = true;
                    let mut cursor = 0usize;
                    while cursor < value.len() {
                        if message.route_count as usize >= MAX_DHCP_ROUTES {
                            return Err(DhcpError::Capacity);
                        }
                        let prefix_len = value[cursor];
                        cursor += 1;
                        if prefix_len > 32 {
                            return Err(DhcpError::InvalidPacket);
                        }
                        let destination_bytes = (prefix_len as usize).div_ceil(8);
                        let end = cursor
                            .checked_add(destination_bytes + 4)
                            .ok_or(DhcpError::InvalidPacket)?;
                        if end > value.len() {
                            return Err(DhcpError::InvalidPacket);
                        }
                        let mut destination = [0; 4];
                        destination[..destination_bytes]
                            .copy_from_slice(&value[cursor..cursor + destination_bytes]);
                        cursor += destination_bytes;
                        let gateway = [
                            value[cursor],
                            value[cursor + 1],
                            value[cursor + 2],
                            value[cursor + 3],
                        ];
                        cursor += 4;
                        let index = message.route_count as usize;
                        message.routes[index] = DhcpRoute {
                            destination,
                            prefix_len,
                            gateway,
                        };
                        message.route_count += 1;
                    }
                }
                OPT_SERVER_ID => {
                    if length != 4 {
                        return Err(DhcpError::InvalidPacket);
                    }
                    let server_id = [value[0], value[1], value[2], value[3]];
                    if let Some(previous) = message.server_id {
                        return if previous == server_id {
                            Err(DhcpError::InvalidPacket)
                        } else {
                            Err(DhcpError::ConflictingOffer)
                        };
                    }
                    message.server_id = Some(server_id);
                }
                OPT_REQUESTED_IP => {
                    if length != 4 || message.requested_ip.is_some() {
                        return Err(DhcpError::InvalidPacket);
                    }
                    message.requested_ip = Some([value[0], value[1], value[2], value[3]]);
                }
                _ => {}
            }
        }
        if !ended {
            return Err(DhcpError::InvalidPacket);
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
        let t1 = self.t1_secs.unwrap_or(lease_time / 2);
        let t2 = self.t2_secs.unwrap_or(lease_time.saturating_mul(7) / 8);
        if t1 == 0 || t2 == 0 || t1 >= lease_time || t2 > lease_time || t1 > t2 {
            return Err(DhcpError::InvalidLease);
        }
        if let Some(gateway) = self.gateway {
            validate_gateway(gateway, self.yiaddr, subnet_mask)?;
        }
        let mut routes = self.routes;
        let mut route_count = self.route_count;
        let gateway = if self.classless_routes {
            routes[..route_count as usize]
                .iter()
                .find(|route| route.prefix_len == 0)
                .map(|route| route.gateway)
        } else if let Some(gateway) = self.gateway {
            if route_count >= MAX_DHCP_ROUTES as u8 {
                return Err(DhcpError::Capacity);
            }
            routes[route_count as usize] = DhcpRoute {
                destination: [0; 4],
                prefix_len: 0,
                gateway,
            };
            route_count += 1;
            Some(gateway)
        } else {
            None
        };
        let offer = DhcpOffer {
            xid: self.xid,
            yiaddr: self.yiaddr,
            server_id,
            subnet_mask,
            gateway,
            dns: self.dns,
            dns_count: self.dns_count,
            lease_time_secs: lease_time,
            t1_secs: t1,
            t2_secs: t2,
            chaddr: self.chaddr,
            routes,
            route_count,
        };
        validate_offer(offer)?;
        Ok(offer)
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
            routes: offer.routes,
            route_count: offer.route_count,
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

fn validate_lease_record(record: DhcpLeaseRecord) -> Result<(), DhcpError> {
    if record.version != DHCP_LEASE_RECORD_VERSION
        || record.interface_len == 0
        || record.interface_len as usize > MAX_INTERFACE_NAME
        || core::str::from_utf8(&record.interface[..record.interface_len as usize]).is_err()
        || record.interface[record.interface_len as usize..]
            .iter()
            .any(|byte| *byte != 0)
        || !valid_mac(record.client_mac)
        || record.server_mac.is_some_and(|mac| !valid_mac(mac))
        || record.remaining_t1_ms > record.remaining_lease_ms
        || record.remaining_t2_ms > record.remaining_lease_ms
    {
        return Err(DhcpError::InvalidLease);
    }
    validate_lease(record.lease)
}

fn validate_lease(lease: DhcpLease) -> Result<(), DhcpError> {
    if lease.address == [0; 4]
        || lease.address != lease.yiaddr
        || !valid_unicast_ipv4(lease.server_id)
        || !valid_subnet_mask(lease.subnet_mask)
        || lease.lease_time_secs == 0
        || lease.t1_secs == 0
        || lease.t1_secs >= lease.lease_time_secs
        || lease.t2_secs < lease.t1_secs
        || lease.t2_secs > lease.lease_time_secs
        || lease.dns_count > MAX_DHCP_DNS_SERVERS as u8
        || lease.route_count > MAX_DHCP_ROUTES as u8
    {
        return Err(DhcpError::InvalidLease);
    }
    if !valid_host_on_subnet(lease.address, lease.address, lease.subnet_mask) {
        return Err(DhcpError::InvalidLease);
    }
    if let Some(gateway) = lease.gateway {
        validate_gateway(gateway, lease.address, lease.subnet_mask)?;
    }
    let subnet_broadcast = subnet_broadcast(lease.address, lease.subnet_mask);
    let has_directed_broadcast = ipv4_value(lease.subnet_mask).count_ones() <= 30;
    for (index, route) in lease
        .routes
        .iter()
        .take(lease.route_count as usize)
        .enumerate()
    {
        if route.prefix_len > 32
            || !valid_unicast_ipv4(route.gateway)
            || !valid_host_on_subnet(route.gateway, lease.address, lease.subnet_mask)
            || !canonical_route_destination(route.destination, route.prefix_len)
            || (has_directed_broadcast && route.destination == subnet_broadcast)
        {
            return Err(DhcpError::InvalidLease);
        }
        if lease.routes[..index]
            .iter()
            .any(|previous| {
                previous.destination == route.destination
                    && previous.prefix_len == route.prefix_len
            })
        {
            return Err(DhcpError::InvalidLease);
        }
    }
    for dns in lease.dns.iter().take(lease.dns_count as usize) {
        if !valid_unicast_ipv4(*dns)
            || (has_directed_broadcast && *dns == subnet_broadcast)
        {
            return Err(DhcpError::InvalidLease);
        }
    }
    Ok(())
}

fn validate_offer(offer: DhcpOffer) -> Result<(), DhcpError> {
    validate_lease(DhcpLease {
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
        routes: offer.routes,
        route_count: offer.route_count,
    })
}

fn valid_mac(mac: [u8; 6]) -> bool {
    mac != [0; 6] && mac != [0xff; 6]
}

fn valid_unicast_ipv4(address: [u8; 4]) -> bool {
    address != [0; 4] && address != [255; 4] && address[0] < 224
}

fn valid_subnet_mask(mask: [u8; 4]) -> bool {
    let mut saw_zero = false;
    for byte in mask {
        for bit in [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01] {
            if byte & bit == 0 {
                saw_zero = true;
            } else if saw_zero {
                return false;
            }
        }
    }
    true
}

fn validate_gateway(
    gateway: [u8; 4],
    interface_address: [u8; 4],
    subnet_mask: [u8; 4],
) -> Result<(), DhcpError> {
    if !valid_host_on_subnet(gateway, interface_address, subnet_mask) {
        return Err(DhcpError::InvalidLease);
    }
    Ok(())
}

fn valid_host_on_subnet(
    address: [u8; 4],
    interface_address: [u8; 4],
    subnet_mask: [u8; 4],
) -> bool {
    if !valid_unicast_ipv4(address) || !valid_subnet_mask(subnet_mask) {
        return false;
    }
    let mask = ipv4_value(subnet_mask);
    let interface = ipv4_value(interface_address);
    let value = ipv4_value(address);
    if value & mask != interface & mask {
        return false;
    }
    let prefix_len = mask.count_ones();
    prefix_len > 30 || (value != (interface & mask) && value != ((interface & mask) | !mask))
}

fn canonical_route_destination(destination: [u8; 4], prefix_len: u8) -> bool {
    if !valid_unicast_ipv4(destination) && prefix_len != 0 {
        return false;
    }
    let mask = prefix_mask(prefix_len);
    ipv4_value(destination) & mask == ipv4_value(destination)
}

fn prefix_mask(prefix_len: u8) -> u32 {
    if prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_len)
    }
}

fn subnet_broadcast(address: [u8; 4], subnet_mask: [u8; 4]) -> [u8; 4] {
    (ipv4_value(address) | !ipv4_value(subnet_mask)).to_be_bytes()
}

fn ipv4_value(address: [u8; 4]) -> u32 {
    u32::from_be_bytes(address)
}

fn encode_lease_record_lease(
    output: &mut [u8],
    cursor: &mut usize,
    lease: DhcpLease,
) -> Result<(), DhcpError> {
    record_put_bytes(output, cursor, &lease.address)?;
    record_put_bytes(output, cursor, &lease.subnet_mask)?;
    match lease.gateway {
        Some(gateway) => {
            record_put_u8(output, cursor, 1)?;
            record_put_bytes(output, cursor, &gateway)?;
        }
        None => {
            record_put_u8(output, cursor, 0)?;
            record_put_bytes(output, cursor, &[0; 4])?;
        }
    }
    for dns in lease.dns {
        record_put_bytes(output, cursor, &dns)?;
    }
    record_put_u8(output, cursor, lease.dns_count)?;
    record_put_bytes(output, cursor, &lease.server_id)?;
    record_put_u32(output, cursor, lease.lease_time_secs)?;
    record_put_u32(output, cursor, lease.t1_secs)?;
    record_put_u32(output, cursor, lease.t2_secs)?;
    record_put_bytes(output, cursor, &lease.yiaddr)?;
    for route in lease.routes {
        record_put_bytes(output, cursor, &route.destination)?;
        record_put_u8(output, cursor, route.prefix_len)?;
        record_put_bytes(output, cursor, &route.gateway)?;
    }
    record_put_u8(output, cursor, lease.route_count)
}

fn decode_lease_record_lease(
    input: &[u8],
    cursor: &mut usize,
) -> Result<DhcpLease, DhcpError> {
    let mut address = [0; 4];
    address.copy_from_slice(record_get_bytes(input, cursor, 4)?);
    let mut subnet_mask = [0; 4];
    subnet_mask.copy_from_slice(record_get_bytes(input, cursor, 4)?);
    let gateway_present = record_get_u8(input, cursor)?;
    let mut gateway_bytes = [0; 4];
    gateway_bytes.copy_from_slice(record_get_bytes(input, cursor, 4)?);
    let gateway = match gateway_present {
        0 => None,
        1 => Some(gateway_bytes),
        _ => return Err(DhcpError::InvalidLease),
    };
    let mut dns = [[0; 4]; MAX_DHCP_DNS_SERVERS];
    for entry in &mut dns {
        entry.copy_from_slice(record_get_bytes(input, cursor, 4)?);
    }
    let dns_count = record_get_u8(input, cursor)?;
    let mut server_id = [0; 4];
    server_id.copy_from_slice(record_get_bytes(input, cursor, 4)?);
    let lease_time_secs = record_get_u32(input, cursor)?;
    let t1_secs = record_get_u32(input, cursor)?;
    let t2_secs = record_get_u32(input, cursor)?;
    let mut yiaddr = [0; 4];
    yiaddr.copy_from_slice(record_get_bytes(input, cursor, 4)?);
    let mut routes = [DhcpRoute::DEFAULT; MAX_DHCP_ROUTES];
    for route in &mut routes {
        let mut destination = [0; 4];
        destination.copy_from_slice(record_get_bytes(input, cursor, 4)?);
        let prefix_len = record_get_u8(input, cursor)?;
        let mut gateway = [0; 4];
        gateway.copy_from_slice(record_get_bytes(input, cursor, 4)?);
        *route = DhcpRoute {
            destination,
            prefix_len,
            gateway,
        };
    }
    let route_count = record_get_u8(input, cursor)?;
    Ok(DhcpLease {
        address,
        subnet_mask,
        gateway,
        dns,
        dns_count,
        server_id,
        lease_time_secs,
        t1_secs,
        t2_secs,
        yiaddr,
        routes,
        route_count,
    })
}

fn record_put_bytes(output: &mut [u8], cursor: &mut usize, bytes: &[u8]) -> Result<(), DhcpError> {
    let end = cursor
        .checked_add(bytes.len())
        .ok_or(DhcpError::Capacity)?;
    if end > output.len() {
        return Err(DhcpError::Capacity);
    }
    output[*cursor..end].copy_from_slice(bytes);
    *cursor = end;
    Ok(())
}

fn record_put_u8(output: &mut [u8], cursor: &mut usize, value: u8) -> Result<(), DhcpError> {
    record_put_bytes(output, cursor, &[value])
}

fn record_put_u32(output: &mut [u8], cursor: &mut usize, value: u32) -> Result<(), DhcpError> {
    record_put_bytes(output, cursor, &value.to_be_bytes())
}

fn record_put_u64(output: &mut [u8], cursor: &mut usize, value: u64) -> Result<(), DhcpError> {
    record_put_bytes(output, cursor, &value.to_be_bytes())
}

fn record_get_bytes<'a>(
    input: &'a [u8],
    cursor: &mut usize,
    length: usize,
) -> Result<&'a [u8], DhcpError> {
    let end = cursor
        .checked_add(length)
        .ok_or(DhcpError::InvalidLease)?;
    if end > input.len() {
        return Err(DhcpError::InvalidLease);
    }
    let bytes = &input[*cursor..end];
    *cursor = end;
    Ok(bytes)
}

fn record_get_u8(input: &[u8], cursor: &mut usize) -> Result<u8, DhcpError> {
    Ok(record_get_bytes(input, cursor, 1)?[0])
}

fn record_get_u32(input: &[u8], cursor: &mut usize) -> Result<u32, DhcpError> {
    let bytes = record_get_bytes(input, cursor, 4)?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn record_get_u64(input: &[u8], cursor: &mut usize) -> Result<u64, DhcpError> {
    let bytes = record_get_bytes(input, cursor, 8)?;
    Ok(u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

fn retry_jitter(seed: u32, attempt: u8) -> u32 {
    let mut value = seed
        .wrapping_add(u32::from(attempt).wrapping_mul(0x85EB_CA6B))
        .wrapping_add(0xC2B2_AE35);
    value ^= value >> 16;
    value = value.wrapping_mul(0x7FEB_352D);
    value ^= value >> 15;
    value
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
