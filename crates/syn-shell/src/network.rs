use core::fmt::Write;

use synos_observability::{EventField, Level, audit_event, field};
use synos_status::{Severity, Status, facility};
use synos_system_model::command::{
    ArgumentKind, ArgumentSpec, CommandSpec, OutputValue, StructuredOutput, MAX_OUTPUT_FIELDS,
};

use crate::{
    Error, Text, MAX_TOKEN_BYTES,
    interpreter::{CommandExecutor, ExecutionToken},
    parser::{CommandCall, CommandRegistry, RouteId, Value},
};

pub const SHOW_NETWORK_ROUTE: u16 = 60;
pub const SHOW_INTERFACES_ROUTE: u16 = 61;
pub const SHOW_ROUTES_ROUTE: u16 = 62;
pub const SET_HOSTNAME_ROUTE: u16 = 63;
pub const SET_INTERFACE_ROUTE: u16 = 64;
pub const SET_ROUTE_ROUTE: u16 = 65;
pub const PING_ROUTE: u16 = 66;
pub const SHOW_NEIGHBORS_ROUTE: u16 = 67;
pub const CLEAR_NEIGHBORS_ROUTE: u16 = 68;
pub const SHOW_DNS_ROUTE: u16 = 69;
pub const SET_DNS_ROUTE: u16 = 70;
pub const RESOLVE_ROUTE: u16 = 71;
pub const SHOW_SOCKETS_ROUTE: u16 = 72;
pub const SHOW_NETWORK_STATS_ROUTE: u16 = 73;
pub const TRACEROUTE_ROUTE: u16 = 74;
pub const SHOW_PACKETS_ROUTE: u16 = 75;

pub const MAX_NETWORK_OUTPUT_ROWS: usize = 4;
pub const MAX_NETWORK_LINK_EVENTS: usize = 4;
pub const MAX_NEIGHBOR_OUTPUT_ROWS: usize = 2;
pub const MAX_SOCKET_OUTPUT_ROWS: usize = 2;
pub const MAX_NETWORK_STATS_INTERFACES: usize = 2;
pub const MAX_TRACEROUTE_OUTPUT_HOPS: usize = 4;
pub const TRACEROUTE_MAX_HOPS: u8 = 8;
pub const TRACEROUTE_HOP_TIMEOUT_MS: u32 = 1_000;
pub const TRACEROUTE_PROBE_INTERVAL_MS: u32 = 100;
pub const TRACEROUTE_TOTAL_DEADLINE_MS: u32 = 10_000;
pub const MAX_DNS_SERVERS: usize = 3;
pub const MAX_DNS_SEARCH_DOMAINS: usize = 3;
pub const MAX_RESOLVE_ANSWERS: usize = 4;
pub const DEFAULT_RESOLVE_TIMEOUT_MS: u32 = 5_000;
pub const MAX_RESOLVE_TIMEOUT_MS: u32 = 30_000;
pub const MAX_PING_REPLY_OUTPUT: usize = 3;
pub const DEFAULT_PING_COUNT: u32 = MAX_PING_REPLY_OUTPUT as u32;
pub const MAX_PING_COUNT: u32 = MAX_PING_REPLY_OUTPUT as u32;
pub const DEFAULT_PING_TIMEOUT_MS: u32 = 1_000;
pub const MAX_PING_TIMEOUT_MS: u32 = 60_000;
pub const DEFAULT_PING_SIZE: u32 = 32;
pub const MAX_PING_SIZE: u32 = 256;
pub const MAX_PING_DNS_TIMEOUT_MS: u32 = 5_000;
pub const MAX_PING_TOTAL_TIMEOUT_MS: u32 = 120_000;
pub const PING_FIRST_SEQUENCE: u32 = 1;
pub const MAX_PACKET_CAPTURE_RECORDS: usize = 32;
pub const MAX_PACKET_OUTPUT_ROWS: usize = 2;
pub const MAX_PACKET_PAYLOAD_BYTES: usize = 32;
pub const DEFAULT_PACKET_MAX_RECORDS: u32 = MAX_PACKET_OUTPUT_ROWS as u32;
pub const MAX_PACKET_MAX_RECORDS: u32 = MAX_PACKET_CAPTURE_RECORDS as u32;
pub const PACKET_CAPTURE_EXPIRY_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkCommandHelp {
    pub name: &'static str,
    pub synopsis: &'static str,
    pub description: &'static str,
    pub aliases: &'static str,
    pub qualifiers: &'static str,
}

const NETWORK_COMMAND_HELP: &[NetworkCommandHelp] = &[
    NetworkCommandHelp {
        name: "SHOW-NETWORK",
        synopsis: "SHOW NETWORK",
        description: "Show network hostname and bounded interface and route counts.",
        aliases: "NETWORK",
        qualifiers: "",
    },
    NetworkCommandHelp {
        name: "SHOW-INTERFACES",
        synopsis: "SHOW INTERFACES [name]",
        description: "Show all interfaces or one named interface, including address mode, link state, and DHCP lease details.",
        aliases: "INTERFACES",
        qualifiers: "",
    },
    NetworkCommandHelp {
        name: "SHOW-ROUTES",
        synopsis: "SHOW ROUTES",
        description: "Show bounded network routes.",
        aliases: "ROUTES",
        qualifiers: "",
    },
    NetworkCommandHelp {
        name: "SET-HOSTNAME",
        synopsis: "SET HOSTNAME hostname",
        description: "Set the host name through the versioned network configuration.",
        aliases: "HOSTNAME",
        qualifiers: "",
    },
    NetworkCommandHelp {
        name: "SET-INTERFACE",
        synopsis: "SET INTERFACE name",
        description: "Change interface address mode, address, gateway, MTU, or enabled state.",
        aliases: "INTERFACE",
        qualifiers: "/ADDRESS /GATEWAY /MTU /ENABLE /DISABLE /DHCP /STATIC",
    },
    NetworkCommandHelp {
        name: "SET-ROUTE",
        synopsis: "SET ROUTE destination",
        description: "Add or replace a route through the versioned network configuration.",
        aliases: "ROUTE",
        qualifiers: "/GATEWAY /INTERFACE /METRIC",
    },
    NetworkCommandHelp {
        name: "PING",
        synopsis: "PING destination",
        description: "Send bounded ICMP echo requests through the network provider.",
        aliases: "",
        qualifiers: "/COUNT /TIMEOUT /SIZE /INTERFACE /SOURCE /IPV4 /IPV6",
    },
    NetworkCommandHelp {
        name: "SHOW-NEIGHBORS",
        synopsis: "SHOW NEIGHBORS",
        description: "Show bounded ARP and IPv6 neighbor cache entries.",
        aliases: "NEIGHBORS",
        qualifiers: "",
    },
    NetworkCommandHelp {
        name: "CLEAR-NEIGHBORS",
        synopsis: "CLEAR NEIGHBORS /CONFIRM",
        description: "Clear the neighbor cache only with explicit confirmation.",
        aliases: "",
        qualifiers: "/CONFIRM",
    },
    NetworkCommandHelp {
        name: "SHOW-DNS",
        synopsis: "SHOW DNS",
        description: "Show ordered DHCP or static DNS servers, search domains, and bounded query status.",
        aliases: "DNS",
        qualifiers: "",
    },
    NetworkCommandHelp {
        name: "SET-DNS",
        synopsis: "SET DNS /STATIC /SERVERS=addresses",
        description: "Set static resolver overrides or restore DHCP-owned DNS configuration.",
        aliases: "",
        qualifiers: "/SERVERS /SEARCH /DHCP /STATIC",
    },
    NetworkCommandHelp {
        name: "RESOLVE",
        synopsis: "RESOLVE hostname",
        description: "Resolve a hostname with bounded timeout and IPv4 or IPv6 selection.",
        aliases: "",
        qualifiers: "/TIMEOUT /IPV4 /IPV6",
    },
    NetworkCommandHelp {
        name: "SHOW-SOCKETS",
        synopsis: "SHOW SOCKETS",
        description: "Show bounded socket endpoints, ownership, state, queues, and lifetime.",
        aliases: "SOCKETS",
        qualifiers: "",
    },
    NetworkCommandHelp {
        name: "SHOW-NETWORK-STATS",
        synopsis: "SHOW NETWORK-STATS",
        description: "Show bounded interface, protocol, DHCP, and firewall counters with reset generations.",
        aliases: "NETWORK-STATS",
        qualifiers: "",
    },
    NetworkCommandHelp {
        name: "TRACEROUTE",
        synopsis: "TRACEROUTE destination",
        description: "Trace a bounded route using TTL-limited probes and ICMP time-exceeded replies.",
        aliases: "",
        qualifiers: "",
    },
    NetworkCommandHelp {
        name: "SHOW-PACKETS",
        synopsis: "SHOW PACKETS",
        description: "Show a capability-gated, bounded packet capture with filter and expiry metadata.",
        aliases: "PACKETS",
        qualifiers: "/INTERFACE /DIRECTION /PROTOCOL /MAX",
    },
];

pub fn command_help(name: &str) -> Option<&'static NetworkCommandHelp> {
    NETWORK_COMMAND_HELP
        .iter()
        .find(|entry| entry.name.eq_ignore_ascii_case(name))
        .or_else(|| {
            NETWORK_COMMAND_HELP.iter().find(|entry| {
                entry
                    .aliases
                    .split(',')
                    .map(str::trim)
                    .any(|alias| !alias.is_empty() && alias.eq_ignore_ascii_case(name))
            })
        })
}

pub type NetworkText = Text<MAX_TOKEN_BYTES>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PacketDirection {
    Ingress,
    Egress,
}

impl PacketDirection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ingress => "ingress",
            Self::Egress => "egress",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacketCaptureRequest<'a> {
    pub interface: Option<&'a str>,
    pub direction: Option<PacketDirection>,
    pub protocol: Option<&'a str>,
    pub max_records: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacketCaptureRecordView {
    pub sequence: u64,
    pub timestamp_ms: u64,
    pub interface: NetworkText,
    pub direction: PacketDirection,
    pub protocol: NetworkText,
    pub source: NetworkText,
    pub destination: NetworkText,
    pub source_port: Option<u16>,
    pub destination_port: Option<u16>,
    pub length: u32,
    pub original_length: u32,
    pub truncated: bool,
    pub expires_at_ms: u64,
    pub payload: NetworkText,
    pub payload_redacted: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacketCaptureView {
    pub generation: u64,
    pub record_count: u64,
    pub dropped_count: u64,
    pub expired_count: u64,
    pub capture_expires_at_ms: u64,
    pub records: [Option<PacketCaptureRecordView>; MAX_PACKET_OUTPUT_ROWS],
    pub next_record: Option<u64>,
}

impl PacketCaptureView {
    pub const EMPTY: Self = Self {
        generation: 0,
        record_count: 0,
        dropped_count: 0,
        expired_count: 0,
        capture_expires_at_ms: 0,
        records: [None; MAX_PACKET_OUTPUT_ROWS],
        next_record: None,
    };
}

pub struct PacketCaptureBuffer<const CAPACITY: usize = MAX_PACKET_CAPTURE_RECORDS> {
    records: [Option<PacketCaptureRecordView>; CAPACITY],
    length: usize,
    next_sequence: u64,
    generation: u64,
    dropped_count: u64,
    expired_count: u64,
    capture_expires_at_ms: u64,
}

impl<const CAPACITY: usize> PacketCaptureBuffer<CAPACITY> {
    pub const fn new() -> Self {
        Self {
            records: [None; CAPACITY],
            length: 0,
            next_sequence: 0,
            generation: 0,
            dropped_count: 0,
            expired_count: 0,
            capture_expires_at_ms: 0,
        }
    }

    pub fn record(
        &mut self,
        now_ms: u64,
        interface: &str,
        direction: PacketDirection,
        protocol: &str,
        source: &str,
        destination: &str,
        source_port: Option<u16>,
        destination_port: Option<u16>,
        payload: &[u8],
    ) -> Result<(), Status> {
        self.expire(now_ms);
        if CAPACITY == 0 || self.length == CAPACITY {
            self.dropped_count = self.dropped_count.saturating_add(1);
            return Ok(())
        }
        let mut preview = NetworkText::empty();
        preview.push_str("<redacted>").map_err(|_| Status::NO_SPACE)?;
        let record = PacketCaptureRecordView {
            sequence: self.next_sequence,
            timestamp_ms: now_ms,
            interface: NetworkText::new(interface).map_err(|_| Status::NO_SPACE)?,
            direction,
            protocol: NetworkText::new(protocol).map_err(|_| Status::NO_SPACE)?,
            source: NetworkText::new(source).map_err(|_| Status::NO_SPACE)?,
            destination: NetworkText::new(destination).map_err(|_| Status::NO_SPACE)?,
            source_port,
            destination_port,
            length: payload.len().min(MAX_PACKET_PAYLOAD_BYTES) as u32,
            original_length: payload.len().min(u32::MAX as usize) as u32,
            truncated: payload.len() > MAX_PACKET_PAYLOAD_BYTES,
            expires_at_ms: now_ms.saturating_add(PACKET_CAPTURE_EXPIRY_MS),
            payload: preview,
            payload_redacted: true,
        };
        self.records[self.length] = Some(record);
        self.length += 1;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.generation = self.generation.saturating_add(1);
        self.capture_expires_at_ms = self.records[..self.length]
            .iter()
            .flatten()
            .map(|record| record.expires_at_ms)
            .min()
            .unwrap_or(0);
        Ok(())
    }

    pub fn expire(&mut self, now_ms: u64) {
        if self.length == 0 {
            return
        }
        let mut retained = 0usize;
        let mut expired = 0u64;
        for index in 0..self.length {
            let Some(record) = self.records[index] else { continue };
            if now_ms >= record.expires_at_ms {
                expired = expired.saturating_add(1);
                continue
            }
            self.records[retained] = Some(record);
            retained += 1;
        }
        self.records[retained..self.length].fill(None);
        self.length = retained;
        self.expired_count = self.expired_count.saturating_add(expired);
        self.capture_expires_at_ms = self.records[..self.length]
            .iter()
            .flatten()
            .map(|record| record.expires_at_ms)
            .min()
            .unwrap_or(0);
        if expired != 0 {
            self.generation = self.generation.saturating_add(1);
        }
    }

    pub fn view(&mut self, now_ms: u64, request: PacketCaptureRequest<'_>) -> PacketCaptureView {
        self.expire(now_ms);
        let mut view = PacketCaptureView {
            generation: self.generation,
            record_count: 0,
            dropped_count: self.dropped_count,
            expired_count: self.expired_count,
            capture_expires_at_ms: self.capture_expires_at_ms,
            records: [None; MAX_PACKET_OUTPUT_ROWS],
            next_record: None,
        };
        let limit = request.max_records.min(MAX_PACKET_MAX_RECORDS) as usize;
        let mut matched = 0usize;
        for record in self.records[..self.length].iter().flatten() {
            if request.interface.is_some_and(|value| {
                !record.interface.as_str().eq_ignore_ascii_case(value)
            }) || request.direction.is_some_and(|value| record.direction != value)
                || request.protocol.is_some_and(|value| {
                    !record.protocol.as_str().eq_ignore_ascii_case(value)
                })
            {
                continue
            }
            view.record_count = view.record_count.saturating_add(1);
            if matched < limit && matched < MAX_PACKET_OUTPUT_ROWS {
                view.records[matched] = Some(*record);
                matched += 1;
            }
        }
        if view.record_count > matched as u64 {
            view.next_record = Some(matched as u64);
        }
        view
    }
}

impl<const CAPACITY: usize> Default for PacketCaptureBuffer<CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum InterfaceAddressMode {
    Static = 0,
    Dhcp = 1,
}

impl InterfaceAddressMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Dhcp => "dhcp",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DhcpLeaseView {
    pub state: NetworkText,
    pub transaction_id: Option<u32>,
    pub client_mac: Option<NetworkText>,
    pub attempt: Option<u8>,
    pub server: Option<NetworkText>,
    pub offered_address: Option<NetworkText>,
    pub bound_at_ms: Option<u64>,
    pub next_action_ms: Option<u64>,
    pub t1_at_ms: Option<u64>,
    pub t2_at_ms: Option<u64>,
    pub expires_at_ms: Option<u64>,
    pub failure_reason: Option<NetworkText>,
    pub last_packet_at_ms: Option<u64>,
    pub dns0: Option<NetworkText>,
    pub dns1: Option<NetworkText>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkQueueView {
    pub ready: bool,
    pub head: Option<u32>,
    pub tail: Option<u32>,
    pub capacity: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkLinkEvent {
    pub generation: u64,
    pub interface: NetworkText,
    pub link_up: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkInterfaceView {
    pub name: NetworkText,
    pub address: NetworkText,
    pub prefix_len: Option<u8>,
    pub mac: Option<NetworkText>,
    pub gateway: Option<NetworkText>,
    pub mtu: u32,
    pub enabled: bool,
    pub link_up: bool,
    pub rx_queue: Option<NetworkQueueView>,
    pub tx_queue: Option<NetworkQueueView>,
    pub mode: InterfaceAddressMode,
    pub dhcp: Option<DhcpLeaseView>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkRouteView {
    pub destination: NetworkText,
    pub gateway: NetworkText,
    pub interface: NetworkText,
    pub metric: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NeighborIpVersion {
    Ipv4,
    Ipv6,
}

impl NeighborIpVersion {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ipv4 => "ipv4",
            Self::Ipv6 => "ipv6",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NeighborState {
    Pending,
    Reachable,
    Stale,
    Failed,
    Permanent,
}

impl NeighborState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Reachable => "reachable",
            Self::Stale => "stale",
            Self::Failed => "failed",
            Self::Permanent => "permanent",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NeighborEntryView {
    pub interface: NetworkText,
    pub address: NetworkText,
    pub ip_version: NeighborIpVersion,
    pub hardware_address: Option<NetworkText>,
    pub state: NeighborState,
    pub last_seen_ms: u64,
    pub expires_at_ms: Option<u64>,
    pub attempts: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NeighborView {
    pub generation: u64,
    pub entry_count: u64,
    pub entries: [Option<NeighborEntryView>; MAX_NEIGHBOR_OUTPUT_ROWS],
    pub next_entry: Option<u64>,
}

impl NeighborView {
    pub const EMPTY: Self = Self {
        generation: 0,
        entry_count: 0,
        entries: [None; MAX_NEIGHBOR_OUTPUT_ROWS],
        next_entry: None,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DnsMode {
    Dhcp,
    Static,
}

impl DnsMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dhcp => "dhcp",
            Self::Static => "static",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DnsServerSource {
    Dhcp,
    Static,
}

impl DnsServerSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dhcp => "dhcp",
            Self::Static => "static",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DnsQueryStatus {
    Idle,
    Pending,
    Success,
    Timeout,
    Failed,
}

impl DnsQueryStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Pending => "pending",
            Self::Success => "success",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DnsServerView {
    pub address: NetworkText,
    pub source: DnsServerSource,
    pub order: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DnsView {
    pub generation: u64,
    pub mode: DnsMode,
    pub servers: [Option<DnsServerView>; MAX_DNS_SERVERS],
    pub server_count: u8,
    pub search_domains: [Option<NetworkText>; MAX_DNS_SEARCH_DOMAINS],
    pub search_count: u8,
    pub query_status: DnsQueryStatus,
    pub query_name: Option<NetworkText>,
    pub query_timeout_ms: u32,
}

impl DnsView {
    pub const EMPTY: Self = Self {
        generation: 0,
        mode: DnsMode::Dhcp,
        servers: [None; MAX_DNS_SERVERS],
        server_count: 0,
        search_domains: [None; MAX_DNS_SEARCH_DOMAINS],
        search_count: 0,
        query_status: DnsQueryStatus::Idle,
        query_name: None,
        query_timeout_ms: MAX_PING_DNS_TIMEOUT_MS,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DnsUpdate {
    pub mode: DnsMode,
    pub servers: [Option<NetworkText>; MAX_DNS_SERVERS],
    pub server_count: u8,
    pub search_domains: [Option<NetworkText>; MAX_DNS_SEARCH_DOMAINS],
    pub search_count: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolveResult {
    Success,
    Timeout,
    DnsFailure,
    PermissionDenied,
    Cancelled,
}

impl ResolveResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Timeout => "timeout",
            Self::DnsFailure => "dns-failure",
            Self::PermissionDenied => "permission-denied",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn status(self) -> Status {
        match self {
            Self::Success => Status::NORMAL,
            Self::Timeout => ping_status(106),
            Self::DnsFailure => ping_status(107),
            Self::PermissionDenied => Status::ACCESS_DENIED,
            Self::Cancelled => Status::CANCELLED,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolveRequest<'a> {
    pub hostname: &'a str,
    pub timeout_ms: u32,
    pub ip_version: Option<PingIpVersion>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolveAnswer {
    pub address: NetworkText,
    pub ip_version: PingIpVersion,
    pub ttl_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolveView {
    pub hostname: NetworkText,
    pub resolver: Option<NetworkText>,
    pub result: ResolveResult,
    pub timeout_ms: u32,
    pub elapsed_ms: u32,
    pub answers: [Option<ResolveAnswer>; MAX_RESOLVE_ANSWERS],
    pub answer_count: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketProtocol {
    Tcp,
    Udp,
    Icmp,
    Other,
}

impl SocketProtocol {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
            Self::Icmp => "icmp",
            Self::Other => "other",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketState {
    Closed,
    Listening,
    Connecting,
    Established,
    Closing,
}

impl SocketState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Listening => "listening",
            Self::Connecting => "connecting",
            Self::Established => "established",
            Self::Closing => "closing",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SocketEntryView {
    pub protocol: SocketProtocol,
    pub local_endpoint: NetworkText,
    pub remote_endpoint: Option<NetworkText>,
    pub owner: Option<NetworkText>,
    pub owner_redacted: bool,
    pub capability: u64,
    pub state: SocketState,
    pub rx_queue_bytes: u64,
    pub tx_queue_bytes: u64,
    pub lifetime_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SocketView {
    pub generation: u64,
    pub socket_count: u64,
    pub sockets: [Option<SocketEntryView>; MAX_SOCKET_OUTPUT_ROWS],
    pub next_socket: Option<u64>,
}

impl SocketView {
    pub const EMPTY: Self = Self {
        generation: 0,
        socket_count: 0,
        sockets: [None; MAX_SOCKET_OUTPUT_ROWS],
        next_socket: None,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkStatsInterfaceView {
    pub name: NetworkText,
    pub rx_packets: u64,
    pub rx_bytes: u64,
    pub tx_packets: u64,
    pub tx_bytes: u64,
    pub drops: u64,
    pub errors: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkStatsDhcpView {
    pub discovers: u64,
    pub offers: u64,
    pub retries: u64,
    pub failures: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkStatsArpView {
    pub requests: u64,
    pub replies: u64,
    pub failures: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkStatsIcmpView {
    pub received: u64,
    pub transmitted: u64,
    pub loss: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkStatsTransportView {
    pub received: u64,
    pub transmitted: u64,
    pub dropped: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkStatsFirewallView {
    pub allowed: u64,
    pub dropped: u64,
    pub rejected: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkStatsView {
    pub generation: u64,
    pub reset_generation: u64,
    pub interface_count: u64,
    pub interfaces: [Option<NetworkStatsInterfaceView>; MAX_NETWORK_STATS_INTERFACES],
    pub dhcp: NetworkStatsDhcpView,
    pub arp: NetworkStatsArpView,
    pub icmp: NetworkStatsIcmpView,
    pub udp: NetworkStatsTransportView,
    pub tcp: NetworkStatsTransportView,
    pub firewall: NetworkStatsFirewallView,
    pub next_interface: Option<u64>,
}

impl NetworkStatsView {
    pub const EMPTY: Self = Self {
        generation: 0,
        reset_generation: 0,
        interface_count: 0,
        interfaces: [None; MAX_NETWORK_STATS_INTERFACES],
        dhcp: NetworkStatsDhcpView {
            discovers: 0,
            offers: 0,
            retries: 0,
            failures: 0,
        },
        arp: NetworkStatsArpView {
            requests: 0,
            replies: 0,
            failures: 0,
        },
        icmp: NetworkStatsIcmpView {
            received: 0,
            transmitted: 0,
            loss: 0,
        },
        udp: NetworkStatsTransportView {
            received: 0,
            transmitted: 0,
            dropped: 0,
        },
        tcp: NetworkStatsTransportView {
            received: 0,
            transmitted: 0,
            dropped: 0,
        },
        firewall: NetworkStatsFirewallView {
            allowed: 0,
            dropped: 0,
            rejected: 0,
        },
        next_interface: None,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TracerouteResult {
    Complete,
    Timeout,
    Unreachable,
    NoRoute,
    PermissionDenied,
    RateLimited,
    MalformedReply,
    Cancelled,
}

impl TracerouteResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Timeout => "timeout",
            Self::Unreachable => "unreachable",
            Self::NoRoute => "no-route",
            Self::PermissionDenied => "permission-denied",
            Self::RateLimited => "rate-limited",
            Self::MalformedReply => "malformed-reply",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn status(self) -> Status {
        match self {
            Self::Complete => Status::NORMAL,
            Self::Timeout => ping_status(115),
            Self::Unreachable => ping_status(116),
            Self::NoRoute => ping_status(117),
            Self::PermissionDenied => Status::ACCESS_DENIED,
            Self::RateLimited => Status::BUSY,
            Self::MalformedReply => ping_status(118),
            Self::Cancelled => Status::CANCELLED,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TracerouteHopResult {
    TimeExceeded,
    DestinationReached,
    Timeout,
    Unreachable,
    NoRoute,
    RateLimited,
    MalformedReply,
}

impl TracerouteHopResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TimeExceeded => "time-exceeded",
            Self::DestinationReached => "destination-reached",
            Self::Timeout => "timeout",
            Self::Unreachable => "unreachable",
            Self::NoRoute => "no-route",
            Self::RateLimited => "rate-limited",
            Self::MalformedReply => "malformed-reply",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TracerouteRequest<'a> {
    pub destination: &'a str,
    pub max_hops: u8,
    pub hop_timeout_ms: u32,
    pub probe_interval_ms: u32,
    pub total_deadline_ms: u32,
}

impl TracerouteRequest<'_> {
    pub const fn defaults(destination: &str) -> TracerouteRequest<'_> {
        TracerouteRequest {
            destination,
            max_hops: TRACEROUTE_MAX_HOPS,
            hop_timeout_ms: TRACEROUTE_HOP_TIMEOUT_MS,
            probe_interval_ms: TRACEROUTE_PROBE_INTERVAL_MS,
            total_deadline_ms: TRACEROUTE_TOTAL_DEADLINE_MS,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TracerouteHop {
    pub ttl: u8,
    pub address: Option<NetworkText>,
    pub result: TracerouteHopResult,
    pub rtt_ms: Option<u64>,
    pub error: Option<NetworkText>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TracerouteView {
    pub destination: NetworkText,
    pub route_interface: Option<NetworkText>,
    pub route_gateway: Option<NetworkText>,
    pub result: TracerouteResult,
    pub max_hops: u8,
    pub hop_timeout_ms: u32,
    pub probe_interval_ms: u32,
    pub total_deadline_ms: u32,
    pub hop_count: u8,
    pub hops: [Option<TracerouteHop>; MAX_TRACEROUTE_OUTPUT_HOPS],
    pub next_hop: Option<u64>,
}

impl TracerouteView {
    pub fn failure(
        request: TracerouteRequest<'_>,
        result: TracerouteResult,
    ) -> Result<Self, Status> {
        Ok(Self {
            destination: NetworkText::new(request.destination).map_err(|_| Status::NO_SPACE)?,
            route_interface: None,
            route_gateway: None,
            result,
            max_hops: request.max_hops,
            hop_timeout_ms: request.hop_timeout_ms,
            probe_interval_ms: request.probe_interval_ms,
            total_deadline_ms: request.total_deadline_ms,
            hop_count: 0,
            hops: [None; MAX_TRACEROUTE_OUTPUT_HOPS],
            next_hop: None,
        })
    }
}

impl ResolveView {
    pub fn failure(request: ResolveRequest<'_>, result: ResolveResult) -> Result<Self, Status> {
        Ok(Self {
            hostname: NetworkText::new(request.hostname).map_err(|_| Status::NO_SPACE)?,
            resolver: None,
            result,
            timeout_ms: request.timeout_ms,
            elapsed_ms: 0,
            answers: [None; MAX_RESOLVE_ANSWERS],
            answer_count: 0,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkView {
    pub generation: u64,
    pub hostname: Option<NetworkText>,
    pub interface_count: u64,
    pub route_count: u64,
    pub interfaces: [Option<NetworkInterfaceView>; MAX_NETWORK_OUTPUT_ROWS],
    pub routes: [Option<NetworkRouteView>; MAX_NETWORK_OUTPUT_ROWS],
    pub link_events: [Option<NetworkLinkEvent>; MAX_NETWORK_LINK_EVENTS],
    pub next_interface: Option<u64>,
    pub next_route: Option<u64>,
}

impl NetworkView {
    pub const EMPTY: Self = Self {
        generation: 0,
        hostname: None,
        interface_count: 0,
        route_count: 0,
        interfaces: [None; MAX_NETWORK_OUTPUT_ROWS],
        routes: [None; MAX_NETWORK_OUTPUT_ROWS],
        link_events: [None; MAX_NETWORK_LINK_EVENTS],
        next_interface: None,
        next_route: None,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterfaceUpdate<'a> {
    pub name: &'a str,
    pub address: Option<&'a str>,
    pub gateway: Option<&'a str>,
    pub mtu: Option<u32>,
    pub enabled: Option<bool>,
    pub mode: Option<InterfaceAddressMode>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteUpdate<'a> {
    pub destination: &'a str,
    pub gateway: &'a str,
    pub interface: &'a str,
    pub metric: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PingIpVersion {
    Ipv4,
    Ipv6,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PingResult {
    Success,
    Timeout,
    Unreachable,
    NoRoute,
    LinkDown,
    DnsFailure,
    PermissionDenied,
    MalformedReply,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PingReply {
    pub sequence: u32,
    pub ttl: Option<u8>,
    pub payload_size: u32,
    pub rtt_ms: Option<u64>,
    pub error: Option<PingResult>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PingSummary {
    pub transmitted: u32,
    pub received: u32,
    pub minimum_rtt_ms: Option<u64>,
    pub average_rtt_ms: Option<u64>,
    pub maximum_rtt_ms: Option<u64>,
    pub replies: [Option<PingReply>; MAX_PING_REPLY_OUTPUT],
}

impl PingSummary {
    pub fn for_result(request: PingRequest<'_>, result: PingResult) -> Self {
        let received = if matches!(result, PingResult::Success) {
            request.count
        } else {
            0
        };
        let mut replies = [None; MAX_PING_REPLY_OUTPUT];
        let reply_count = core::cmp::min(request.count as usize, MAX_PING_REPLY_OUTPUT);
        let mut index = 0;
        while index < reply_count {
            replies[index] = Some(PingReply {
                sequence: PING_FIRST_SEQUENCE + index as u32,
                ttl: None,
                payload_size: request.size,
                rtt_ms: None,
                error: if matches!(result, PingResult::Success) {
                    None
                } else {
                    Some(result)
                },
            });
            index += 1;
        }
        Self {
            transmitted: request.count,
            received,
            minimum_rtt_ms: None,
            average_rtt_ms: None,
            maximum_rtt_ms: None,
            replies,
        }
    }

    pub const fn lost(self) -> u32 {
        self.transmitted.saturating_sub(self.received)
    }

    pub const fn loss_percent(self) -> u64 {
        if self.transmitted == 0 {
            0
        } else {
            self.lost() as u64 * 100 / self.transmitted as u64
        }
    }
}

impl PingResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Timeout => "timeout",
            Self::Unreachable => "unreachable",
            Self::NoRoute => "no-route",
            Self::LinkDown => "link-down",
            Self::DnsFailure => "dns-failure",
            Self::PermissionDenied => "permission-denied",
            Self::MalformedReply => "malformed-reply",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn status(self) -> Status {
        match self {
            Self::Success => Status::NORMAL,
            Self::Timeout => ping_status(100),
            Self::Unreachable => ping_status(101),
            Self::NoRoute => ping_status(102),
            Self::LinkDown => ping_status(103),
            Self::DnsFailure => ping_status(104),
            Self::PermissionDenied => Status::ACCESS_DENIED,
            Self::MalformedReply => ping_status(105),
            Self::Cancelled => Status::CANCELLED,
        }
    }

    pub const fn audit_code(self) -> u64 {
        match self {
            Self::Success => 1,
            Self::Timeout => 2,
            Self::Unreachable => 3,
            Self::NoRoute => 4,
            Self::LinkDown => 5,
            Self::DnsFailure => 6,
            Self::PermissionDenied => 7,
            Self::MalformedReply => 8,
            Self::Cancelled => 9,
        }
    }
}

fn ping_status(code: u16) -> Status {
    Status::new(Severity::Error, facility::NETWORK, code, 0).unwrap_or(Status::INTERNAL)
}

fn audit_identity(value: &str) -> u128 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash as u128
}

fn record_ping_request(context: PingAuditContext) {
    let level = if context.capability == 0 {
        Level::Warn
    } else {
        Level::Info
    };
    audit_event!(
        level,
        EventField::unsigned(field::OPERATION, PING_ROUTE as u64),
        EventField::unsigned(field::CAPABILITY, context.capability),
        EventField::identifier(field::PING_TARGET, context.target),
        EventField::identifier(field::PING_INTERFACE, context.interface),
    );
    audit_event!(
        level,
        EventField::unsigned(field::CAPABILITY, context.capability),
        EventField::identifier(field::PING_SOURCE, context.source),
        EventField::unsigned(field::PING_COUNT, context.count as u64),
        EventField::unsigned(field::PING_TIMEOUT, context.timeout_ms as u64),
    );
}

fn record_ping_result(context: PingAuditContext, result: PingResult) {
    let level = if matches!(result, PingResult::Success) {
        Level::Info
    } else {
        Level::Warn
    };
    audit_event!(
        level,
        EventField::unsigned(field::CAPABILITY, context.capability),
        EventField::identifier(field::PING_TARGET, context.target),
        EventField::unsigned(field::PING_RESULT, result.audit_code()),
        EventField::status(result.status()),
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PingRequest<'a> {
    pub destination: &'a str,
    pub count: u32,
    pub timeout_ms: u32,
    pub size: u32,
    pub interface: Option<&'a str>,
    pub source: Option<&'a str>,
    pub ip_version: Option<PingIpVersion>,
}

impl PingRequest<'_> {
    pub const fn dns_timeout_ms(self) -> u32 {
        if self.timeout_ms > MAX_PING_DNS_TIMEOUT_MS {
            MAX_PING_DNS_TIMEOUT_MS
        } else {
            self.timeout_ms
        }
    }

    pub const fn schedule(self) -> PingSchedule {
        let total_timeout_ms = self.timeout_ms.saturating_mul(self.count);
        PingSchedule {
            count: self.count,
            packet_timeout_ms: self.timeout_ms,
            total_timeout_ms: if total_timeout_ms > MAX_PING_TOTAL_TIMEOUT_MS {
                MAX_PING_TOTAL_TIMEOUT_MS
            } else {
                total_timeout_ms
            },
            first_sequence: PING_FIRST_SEQUENCE,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PingSchedule {
    pub count: u32,
    pub packet_timeout_ms: u32,
    pub total_timeout_ms: u32,
    pub first_sequence: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PingTarget {
    pub address: NetworkText,
    pub ip_version: PingIpVersion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResolvedPingRequest<'a> {
    pub request: PingRequest<'a>,
    pub target: PingTarget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct PingHandle(u64);

impl PingHandle {
    pub const fn new(raw: u64) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PingAuditContext {
    capability: u64,
    target: u128,
    interface: u128,
    source: u128,
    count: u32,
    timeout_ms: u32,
}

impl PingAuditContext {
    fn new(capability: u64, request: PingRequest<'_>) -> Self {
        Self {
            capability,
            target: audit_identity(request.destination),
            interface: request.interface.map_or(0, audit_identity),
            source: request.source.map_or(0, audit_identity),
            count: request.count,
            timeout_ms: request.timeout_ms,
        }
    }
}

/// Source of truth for network settings.
///
/// A system provider should validate the caller's network-administration
/// capability, create a new declarative configuration revision, stage it,
/// health-check it, commit it, and persist it before returning the new view.
pub trait NetworkSource {
    /// Prove the caller has the network-administration capability.
    fn authorize_mutation(&mut self) -> Result<(), Status>;

    /// Prove the caller has the network diagnostic capability and return its
    /// opaque audit handle.
    fn authorize_ping(&mut self, _request: ResolvedPingRequest<'_>) -> Result<u64, Status> {
        Err(Status::ACCESS_DENIED)
    }

    /// Prove the caller has the diagnostic capability before exposing packet
    /// metadata. Providers must expire old records before returning a view and
    /// must redact payloads and unauthorized endpoint identity.
    fn authorize_packet_capture(
        &mut self,
        _request: PacketCaptureRequest<'_>,
    ) -> Result<u64, Status> {
        Err(Status::ACCESS_DENIED)
    }

    fn show_packets(
        &mut self,
        _request: PacketCaptureRequest<'_>,
    ) -> Result<PacketCaptureView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn show_network(&mut self) -> Result<NetworkView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn show_interfaces(&mut self) -> Result<NetworkView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn show_routes(&mut self) -> Result<NetworkView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn show_neighbors(&mut self) -> Result<NeighborView, Status> {
        Err(Status::NOT_FOUND)
    }

    /// Clear dynamic and permanent neighbor entries after the shell has
    /// checked the explicit confirmation guard and mutation capability.
    fn clear_neighbors(&mut self) -> Result<u64, Status> {
        Err(Status::NOT_FOUND)
    }

    fn show_dns(&mut self) -> Result<DnsView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn set_dns(&mut self, _update: DnsUpdate) -> Result<DnsView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn resolve_hostname(&mut self, _request: ResolveRequest<'_>) -> Result<ResolveView, Status> {
        Err(Status::NOT_FOUND)
    }

    /// Return a bounded socket snapshot. Providers must omit owners the
    /// caller cannot inspect and set `owner_redacted` for those entries.
    fn show_sockets(&mut self) -> Result<SocketView, Status> {
        Err(Status::NOT_FOUND)
    }

    /// Return a bounded counter snapshot. `reset_generation` changes whenever
    /// counters are reset, so readers never combine values from two epochs.
    fn show_network_stats(&mut self) -> Result<NetworkStatsView, Status> {
        Err(Status::NOT_FOUND)
    }

    /// Run a bounded TTL trace. Providers must select the route before
    /// probing, rate-limit probes, honor the total deadline, and distinguish
    /// ICMP time-exceeded replies from destination replies.
    fn traceroute(
        &mut self,
        _request: TracerouteRequest<'_>,
    ) -> Result<TracerouteView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn set_hostname(&mut self, _hostname: &str) -> Result<NetworkView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn set_interface(&mut self, _update: InterfaceUpdate<'_>) -> Result<NetworkView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn set_route(&mut self, _update: RouteUpdate<'_>) -> Result<NetworkView, Status> {
        Err(Status::NOT_FOUND)
    }

    /// Resolve a hostname within `timeout_ms`. Providers must use a monotonic
    /// deadline and return a stable error when DNS cannot finish in bounds.
    fn resolve_ping_hostname(
        &mut self,
        _hostname: &str,
        _ip_version: PingIpVersion,
        _timeout_ms: u32,
    ) -> Result<PingTarget, Status> {
        Err(Status::NOT_FOUND)
    }

    fn resolve_ping_target(&mut self, request: PingRequest<'_>) -> Result<PingTarget, Status> {
        match resolve_literal_ipv4_target(request) {
            Ok(target) => Ok(target),
            Err(Status::NOT_FOUND) => self
                .resolve_ping_hostname(
                    request.destination,
                    request.ip_version.unwrap_or(PingIpVersion::Ipv4),
                    request.dns_timeout_ms(),
                )
                .map_err(map_ping_resolution_status),
            Err(status) => Err(status),
        }
    }

    /// Start a bounded ping session. The provider must send no more than
    /// `schedule.count` packets, start at `schedule.first_sequence`, enforce
    /// both deadlines, and keep the session non-blocking after this call.
    fn start_ping(
        &mut self,
        _request: ResolvedPingRequest<'_>,
        _schedule: PingSchedule,
    ) -> Result<PingHandle, Status> {
        Err(Status::NOT_FOUND)
    }

    /// Poll a ping session. `None` means still running; `Some` completes it.
    /// Completed output must use [`ping_result_output`] for one of the stable
    /// [`PingResult`] values.
    fn poll_ping(
        &mut self,
        _handle: PingHandle,
    ) -> Option<Result<StructuredOutput, Status>> {
        None
    }

    fn cancel_ping(&mut self, _handle: PingHandle) -> Result<(), Status> {
        Err(Status::NOT_FOUND)
    }

    /// Synchronous compatibility path. Completed output must use
    /// [`ping_result_output`] for a stable [`PingResult`].
    fn ping(&mut self, _request: ResolvedPingRequest<'_>) -> Result<StructuredOutput, Status> {
        Err(Status::NOT_FOUND)
    }
}

fn map_ping_resolution_status(status: Status) -> Status {
    match status {
        Status::NOT_FOUND => PingResult::DnsFailure.status(),
        Status::ACCESS_DENIED => PingResult::PermissionDenied.status(),
        _ => status,
    }
}

fn map_ping_provider_status(status: Status) -> Status {
    match status {
        Status::NOT_FOUND => PingResult::NoRoute.status(),
        Status::ACCESS_DENIED => PingResult::PermissionDenied.status(),
        _ => status,
    }
}

fn ping_result_from_output(output: &StructuredOutput) -> PingResult {
    for field in output.fields() {
        if field.name.as_str() != "result" {
            continue
        }
        let OutputValue::Text(value) = field.value else {
            break
        };
        return match value.as_str() {
            "success" => PingResult::Success,
            "timeout" => PingResult::Timeout,
            "unreachable" => PingResult::Unreachable,
            "no-route" => PingResult::NoRoute,
            "link-down" => PingResult::LinkDown,
            "dns-failure" => PingResult::DnsFailure,
            "permission-denied" => PingResult::PermissionDenied,
            "malformed-reply" => PingResult::MalformedReply,
            "cancelled" => PingResult::Cancelled,
            _ => ping_result_from_status(output.status()),
        }
    }
    ping_result_from_status(output.status())
}

fn ping_result_from_status(status: Status) -> PingResult {
    if status == Status::NORMAL {
        PingResult::Success
    } else if status == Status::ACCESS_DENIED {
        PingResult::PermissionDenied
    } else if status == Status::CANCELLED {
        PingResult::Cancelled
    } else if status == PingResult::Timeout.status() {
        PingResult::Timeout
    } else if status == PingResult::Unreachable.status() {
        PingResult::Unreachable
    } else if status == PingResult::NoRoute.status() {
        PingResult::NoRoute
    } else if status == PingResult::LinkDown.status() {
        PingResult::LinkDown
    } else if status == PingResult::DnsFailure.status() {
        PingResult::DnsFailure
    } else if status == PingResult::MalformedReply.status() {
        PingResult::MalformedReply
    } else {
        PingResult::NoRoute
    }
}

fn resolve_result_from_status(status: Status) -> ResolveResult {
    if status == Status::ACCESS_DENIED {
        ResolveResult::PermissionDenied
    } else if status == Status::CANCELLED {
        ResolveResult::Cancelled
    } else if status == ResolveResult::Timeout.status() {
        ResolveResult::Timeout
    } else {
        ResolveResult::DnsFailure
    }
}

fn traceroute_result_from_status(status: Status) -> TracerouteResult {
    if status == Status::ACCESS_DENIED {
        TracerouteResult::PermissionDenied
    } else if status == Status::BUSY {
        TracerouteResult::RateLimited
    } else if status == Status::CANCELLED {
        TracerouteResult::Cancelled
    } else if status == TracerouteResult::Timeout.status() {
        TracerouteResult::Timeout
    } else if status == TracerouteResult::MalformedReply.status() {
        TracerouteResult::MalformedReply
    } else if status == TracerouteResult::NoRoute.status() {
        TracerouteResult::NoRoute
    } else {
        TracerouteResult::Unreachable
    }
}

fn record_packet_capture_request(
    capability: u64,
    request: PacketCaptureRequest<'_>,
) {
    audit_event!(
        Level::Info,
        EventField::unsigned(field::OPERATION, SHOW_PACKETS_ROUTE as u64),
        EventField::unsigned(field::CAPABILITY, capability),
        EventField::unsigned(field::LENGTH, request.max_records as u64),
        EventField::identifier(
            field::TRANSPORT,
            request.protocol.map_or(0, audit_identity),
        ),
    );
}

pub fn register_network_commands<const CAPACITY: usize>(
    registry: &mut CommandRegistry<CAPACITY>,
) -> Result<(), Error> {
    registry.register(
        CommandSpec::new("SHOW-NETWORK", &[]).map_err(|_| Error::InvalidValue)?,
        route(SHOW_NETWORK_ROUTE),
    )?;
    registry.register(
        CommandSpec::new(
            "SHOW-INTERFACES",
            &[positional("INTERFACE", ArgumentKind::Text, false)?],
        )
        .map_err(|_| Error::InvalidValue)?,
        route(SHOW_INTERFACES_ROUTE),
    )?;
    registry.register(
        CommandSpec::new("SHOW-ROUTES", &[]).map_err(|_| Error::InvalidValue)?,
        route(SHOW_ROUTES_ROUTE),
    )?;
    registry.register(
        CommandSpec::new("SHOW-NEIGHBORS", &[]).map_err(|_| Error::InvalidValue)?,
        route(SHOW_NEIGHBORS_ROUTE),
    )?;
    let confirm = qualifier("CONFIRM", ArgumentKind::Boolean)?;
    registry.register(
        CommandSpec::new("CLEAR-NEIGHBORS", &[confirm])
            .map_err(|_| Error::InvalidValue)?,
        route(CLEAR_NEIGHBORS_ROUTE),
    )?;
    registry.register(
        CommandSpec::new("SHOW-DNS", &[]).map_err(|_| Error::InvalidValue)?,
        route(SHOW_DNS_ROUTE),
    )?;
    let servers = qualifier("SERVERS", ArgumentKind::Text)?;
    let search = qualifier("SEARCH", ArgumentKind::Text)?;
    let dhcp = qualifier("DHCP", ArgumentKind::Boolean)?;
    let static_mode = qualifier("STATIC", ArgumentKind::Boolean)?;
    registry.register(
        CommandSpec::new("SET-DNS", &[servers, search, dhcp, static_mode])
            .map_err(|_| Error::InvalidValue)?,
        route(SET_DNS_ROUTE),
    )?;
    let hostname = positional("HOSTNAME", ArgumentKind::Text, true)?;
    let timeout = qualifier("TIMEOUT", ArgumentKind::Integer)?;
    let ipv4 = qualifier("IPV4", ArgumentKind::Boolean)?;
    let ipv6 = qualifier("IPV6", ArgumentKind::Boolean)?;
    registry.register(
        CommandSpec::new("RESOLVE", &[hostname, timeout, ipv4, ipv6])
            .map_err(|_| Error::InvalidValue)?,
        route(RESOLVE_ROUTE),
    )?;
    registry.register(
        CommandSpec::new("SHOW-SOCKETS", &[]).map_err(|_| Error::InvalidValue)?,
        route(SHOW_SOCKETS_ROUTE),
    )?;
    registry.register(
        CommandSpec::new("SHOW-NETWORK-STATS", &[])
            .map_err(|_| Error::InvalidValue)?,
        route(SHOW_NETWORK_STATS_ROUTE),
    )?;
    let destination = positional("DESTINATION", ArgumentKind::Text, true)?;
    registry.register(
        CommandSpec::new("TRACEROUTE", &[destination])
            .map_err(|_| Error::InvalidValue)?,
        route(TRACEROUTE_ROUTE),
    )?;

    let interface = qualifier("INTERFACE", ArgumentKind::Text)?;
    let direction = qualifier("DIRECTION", ArgumentKind::Text)?;
    let protocol = qualifier("PROTOCOL", ArgumentKind::Text)?;
    let max = qualifier("MAX", ArgumentKind::Integer)?;
    registry.register(
        CommandSpec::new("SHOW-PACKETS", &[interface, direction, protocol, max])
            .map_err(|_| Error::InvalidValue)?,
        route(SHOW_PACKETS_ROUTE),
    )?;

    let hostname = positional("HOSTNAME", ArgumentKind::Text, true)?;
    registry.register(
        CommandSpec::new("SET-HOSTNAME", &[hostname]).map_err(|_| Error::InvalidValue)?,
        route(SET_HOSTNAME_ROUTE),
    )?;

    let interface = positional("INTERFACE", ArgumentKind::Text, true)?;
    let address = qualifier("ADDRESS", ArgumentKind::Text)?;
    let gateway = qualifier("GATEWAY", ArgumentKind::Text)?;
    let mtu = qualifier("MTU", ArgumentKind::Integer)?;
    let enable = qualifier("ENABLE", ArgumentKind::Boolean)?;
    let disable = qualifier("DISABLE", ArgumentKind::Boolean)?;
    let dhcp = qualifier("DHCP", ArgumentKind::Boolean)?;
    let static_mode = qualifier("STATIC", ArgumentKind::Boolean)?;
    registry.register(
        CommandSpec::new(
            "SET-INTERFACE",
            &[
                interface,
                address,
                gateway,
                mtu,
                enable,
                disable,
                dhcp,
                static_mode,
            ],
        )
        .map_err(|_| Error::InvalidValue)?,
        route(SET_INTERFACE_ROUTE),
    )?;

    let destination = positional("DESTINATION", ArgumentKind::Text, true)?;
    let gateway = qualifier("GATEWAY", ArgumentKind::Text)?;
    let interface = qualifier("INTERFACE", ArgumentKind::Text)?;
    let metric = qualifier("METRIC", ArgumentKind::Integer)?;
    registry.register(
        CommandSpec::new("SET-ROUTE", &[destination, gateway, interface, metric])
            .map_err(|_| Error::InvalidValue)?,
        route(SET_ROUTE_ROUTE),
    )?;

    let ping_destination = positional("DESTINATION", ArgumentKind::Text, true)?;
    let count = qualifier("COUNT", ArgumentKind::Integer)?;
    let timeout = qualifier("TIMEOUT", ArgumentKind::Integer)?;
    let size = qualifier("SIZE", ArgumentKind::Integer)?;
    let interface = qualifier("INTERFACE", ArgumentKind::Text)?;
    let source = qualifier("SOURCE", ArgumentKind::Text)?;
    let ipv4 = qualifier("IPV4", ArgumentKind::Boolean)?;
    let ipv6 = qualifier("IPV6", ArgumentKind::Boolean)?;
    registry.register(
        CommandSpec::new(
            "PING",
            &[
                ping_destination,
                count,
                timeout,
                size,
                interface,
                source,
                ipv4,
                ipv6,
            ],
        )
        .map_err(|_| Error::InvalidValue)?,
        route(PING_ROUTE),
    )
}

pub struct NetworkExecutor<Source, const CAPACITY: usize = 16> {
    source: Source,
    completions: [Option<NetworkCompletion>; CAPACITY],
}

enum NetworkCompletion {
    Ready(Result<StructuredOutput, Status>),
    Ping(PingHandle, PingAuditContext),
}

impl<Source, const CAPACITY: usize> NetworkExecutor<Source, CAPACITY> {
    pub fn new(source: Source) -> Self {
        Self {
            source,
            completions: [const { None }; CAPACITY],
        }
    }

    pub const fn source(&self) -> &Source {
        &self.source
    }

    pub const fn source_mut(&mut self) -> &mut Source {
        &mut self.source
    }
}

impl<Source: NetworkSource, const CAPACITY: usize> NetworkExecutor<Source, CAPACITY> {
    pub fn execute_command(&mut self, command: CommandCall) -> Result<StructuredOutput, Status> {
        dispatch_network_command(&mut self.source, command)
    }
}

/// Execute a network command against a [`NetworkSource`] without buffering completions.
pub fn dispatch_network_command<Source: NetworkSource>(
    source: &mut Source,
    command: CommandCall,
) -> Result<StructuredOutput, Status> {
    match command.route.raw() {
        SHOW_NETWORK_ROUTE => source.show_network().and_then(show_network_output),
        SHOW_INTERFACES_ROUTE => {
            let interface = command.get_text("INTERFACE");
            if interface.is_some_and(str::is_empty) {
                return Err(Status::INVALID_ARGUMENT);
            }
            source.show_interfaces().and_then(|view| match interface {
                Some(name) => show_interface_output(view, name),
                None => interfaces_output(view),
            })
        }
        SHOW_ROUTES_ROUTE => source.show_routes().and_then(routes_output),
        SHOW_NEIGHBORS_ROUTE => source.show_neighbors().and_then(neighbors_output),
        CLEAR_NEIGHBORS_ROUTE => {
            if !boolean(command.get("CONFIRM"))? {
                return Err(Status::INVALID_ARGUMENT)
            }
            source.authorize_mutation()?;
            source.clear_neighbors().and_then(clear_neighbors_output)
        }
        SHOW_DNS_ROUTE => source.show_dns().and_then(dns_output),
        SET_DNS_ROUTE => {
            let update = dns_update_request(&command)?;
            source.authorize_mutation()?;
            source.set_dns(update).and_then(dns_output)
        }
        RESOLVE_ROUTE => {
            let request = resolve_request(&command)?;
            match source.resolve_hostname(request) {
                Ok(view) => resolve_output(view),
                Err(status) => ResolveView::failure(request, resolve_result_from_status(status))
                    .and_then(resolve_output),
            }
        }
        SHOW_SOCKETS_ROUTE => source.show_sockets().and_then(sockets_output),
        SHOW_NETWORK_STATS_ROUTE => source
            .show_network_stats()
            .and_then(network_stats_output),
        TRACEROUTE_ROUTE => {
            let request = traceroute_request(&command)?;
            match source.traceroute(request) {
                Ok(view) => traceroute_output(view),
                Err(status) => TracerouteView::failure(request, traceroute_result_from_status(status))
                    .and_then(traceroute_output),
            }
        }
        SHOW_PACKETS_ROUTE => {
            let request = packet_capture_request(&command)?;
            let capability = source.authorize_packet_capture(request)?;
            record_packet_capture_request(capability, request);
            source.show_packets(request).and_then(packet_capture_output)
        }
        SET_HOSTNAME_ROUTE => {
            let hostname = command
                .get_text("HOSTNAME")
                .filter(|value| !value.is_empty())
                .ok_or(Status::INVALID_ARGUMENT)?;
            source.authorize_mutation()?;
            source
                .set_hostname(hostname)
                .and_then(|view| network_operation_output(view, "set-hostname"))
        }
        SET_INTERFACE_ROUTE => {
            let update = interface_update_request(&command)?;
            source.authorize_mutation()?;
            source
                .set_interface(update)
                .and_then(|view| set_interface_output(view, update.name))
        }
        SET_ROUTE_ROUTE => {
            let update = route_update_request(&command)?;
            source.authorize_mutation()?;
            source
                .set_route(update)
                .and_then(|view| network_operation_output(view, "set-route"))
        }
        PING_ROUTE => {
            let request = ping_request(&command)?;
            let target = match source.resolve_ping_target(request) {
                Ok(target) => target,
                Err(status) => {
                    let status = map_ping_resolution_status(status);
                    let audit = PingAuditContext::new(0, request);
                    record_ping_request(audit);
                    record_ping_result(audit, ping_result_from_status(status));
                    return Err(status)
                }
            };
            let resolved = ResolvedPingRequest { request, target };
            let capability = match source.authorize_ping(resolved) {
                Ok(capability) => capability,
                Err(status) => {
                    let audit = PingAuditContext::new(0, request);
                    record_ping_request(audit);
                    record_ping_result(audit, ping_result_from_status(status));
                    return Err(status)
                }
            };
            let audit = PingAuditContext::new(capability, request);
            record_ping_request(audit);
            match source.ping(resolved) {
                Ok(output) => {
                    record_ping_result(audit, ping_result_from_output(&output));
                    Ok(output)
                }
                Err(status) => {
                    let status = map_ping_provider_status(status);
                    record_ping_result(audit, ping_result_from_status(status));
                    Err(status)
                }
            }
        }
        _ => Err(Status::NOT_FOUND),
    }
}

impl<Source: NetworkSource, const CAPACITY: usize> CommandExecutor
    for NetworkExecutor<Source, CAPACITY>
{
    fn submit(
        &mut self,
        command: CommandCall,
        _pipeline_input: Option<&StructuredOutput>,
    ) -> Result<ExecutionToken, Error> {
        let slot = self
            .completions
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let completion = if command.route.raw() == PING_ROUTE {
            let request = ping_request(&command).map_err(Error::CommandFailed)?;
            let target = match self.source.resolve_ping_target(request) {
                Ok(target) => target,
                Err(status) => {
                    let status = map_ping_resolution_status(status);
                    let audit = PingAuditContext::new(0, request);
                    record_ping_request(audit);
                    record_ping_result(audit, ping_result_from_status(status));
                    return Err(Error::CommandFailed(status))
                }
            };
            let resolved = ResolvedPingRequest { request, target };
            let capability = match self.source.authorize_ping(resolved) {
                Ok(capability) => capability,
                Err(status) => {
                    let audit = PingAuditContext::new(0, request);
                    record_ping_request(audit);
                    record_ping_result(audit, ping_result_from_status(status));
                    return Err(Error::CommandFailed(status))
                }
            };
            let audit = PingAuditContext::new(capability, request);
            record_ping_request(audit);
            let handle = match self.source.start_ping(resolved, request.schedule()) {
                Ok(handle) => handle,
                Err(status) => {
                    let status = map_ping_provider_status(status);
                    record_ping_result(audit, ping_result_from_status(status));
                    return Err(Error::CommandFailed(status))
                }
            };
            NetworkCompletion::Ping(handle, audit)
        } else {
            NetworkCompletion::Ready(dispatch_network_command(&mut self.source, command))
        };
        self.completions[slot] = Some(completion);
        ExecutionToken::new((slot + 1) as u64).ok_or(Error::InvalidHandle)
    }

    fn poll(&mut self, token: ExecutionToken) -> Option<Result<StructuredOutput, Status>> {
        let slot = self
            .completions
            .get_mut(token.raw().checked_sub(1)? as usize)?;
        match slot.take()? {
            NetworkCompletion::Ready(result) => Some(result),
            NetworkCompletion::Ping(handle, audit) => match self.source.poll_ping(handle) {
                Some(Ok(output)) => {
                    record_ping_result(audit, ping_result_from_output(&output));
                    Some(Ok(output))
                }
                Some(Err(status)) => {
                    let status = map_ping_provider_status(status);
                    record_ping_result(audit, ping_result_from_status(status));
                    Some(Err(status))
                }
                None => {
                    *slot = Some(NetworkCompletion::Ping(handle, audit));
                    None
                }
            },
        }
    }

    fn cancel(&mut self, token: ExecutionToken) -> Result<(), Error> {
        let slot = self
            .completions
            .get_mut(token.raw().checked_sub(1).ok_or(Error::InvalidHandle)? as usize)
            .ok_or(Error::InvalidHandle)?;
        match slot.take() {
            Some(NetworkCompletion::Ready(_)) | None => Ok(()),
            Some(NetworkCompletion::Ping(handle, audit)) => {
                match self.source.cancel_ping(handle) {
                    Ok(()) => {
                        record_ping_result(audit, PingResult::Cancelled);
                        Ok(())
                    }
                    Err(status) => {
                        let status = map_ping_provider_status(status);
                        record_ping_result(audit, ping_result_from_status(status));
                        Err(Error::CommandFailed(status))
                    }
                }
            }
        }
    }
}

pub fn interface_update_request<'a>(
    command: &'a CommandCall,
) -> Result<InterfaceUpdate<'a>, Status> {
    let name = command
        .get_text("INTERFACE")
        .filter(|value| !value.is_empty())
        .ok_or(Status::INVALID_ARGUMENT)?;
    let address = optional_text(command, "ADDRESS")?;
    let gateway = optional_text(command, "GATEWAY")?;
    let mtu = optional_u32(command, "MTU")?;
    let enable = boolean(command.get("ENABLE"))?;
    let disable = boolean(command.get("DISABLE"))?;
    let dhcp = boolean(command.get("DHCP"))?;
    let static_mode = boolean(command.get("STATIC"))?;
    if enable && disable {
        return Err(Status::INVALID_ARGUMENT);
    }
    if dhcp && static_mode {
        return Err(Status::INVALID_ARGUMENT);
    }
    if dhcp && address.is_some() {
        return Err(Status::INVALID_ARGUMENT);
    }
    let enabled = match (enable, disable) {
        (true, false) => Some(true),
        (false, true) => Some(false),
        _ => None,
    };
    let mode = match (dhcp, static_mode) {
        (true, false) => Some(InterfaceAddressMode::Dhcp),
        (false, true) => Some(InterfaceAddressMode::Static),
        _ if address.is_some() => Some(InterfaceAddressMode::Static),
        _ => None,
    };
    if address.is_none()
        && gateway.is_none()
        && mtu.is_none()
        && enabled.is_none()
        && mode.is_none()
    {
        return Err(Status::INVALID_ARGUMENT);
    }
    if mtu.is_some_and(|value| !(576..=65_535).contains(&value)) {
        return Err(Status::INVALID_ARGUMENT);
    }
    Ok(InterfaceUpdate {
        name,
        address,
        gateway,
        mtu,
        enabled,
        mode,
    })
}

pub fn route_update_request<'a>(command: &'a CommandCall) -> Result<RouteUpdate<'a>, Status> {
    let destination = command
        .get_text("DESTINATION")
        .filter(|value| !value.is_empty())
        .ok_or(Status::INVALID_ARGUMENT)?;
    let gateway = command
        .get_text("GATEWAY")
        .filter(|value| !value.is_empty())
        .ok_or(Status::INVALID_ARGUMENT)?;
    let interface = command
        .get_text("INTERFACE")
        .filter(|value| !value.is_empty())
        .ok_or(Status::INVALID_ARGUMENT)?;
    let metric = optional_u32(command, "METRIC")?;
    Ok(RouteUpdate {
        destination,
        gateway,
        interface,
        metric,
    })
}

pub fn ping_request<'a>(command: &'a CommandCall) -> Result<PingRequest<'a>, Status> {
    let destination = command
        .get_text("DESTINATION")
        .filter(|value| !value.is_empty())
        .ok_or(Status::INVALID_ARGUMENT)?;
    let count = optional_u32(command, "COUNT")?.unwrap_or(DEFAULT_PING_COUNT);
    let timeout_ms = optional_u32(command, "TIMEOUT")?.unwrap_or(DEFAULT_PING_TIMEOUT_MS);
    let size = optional_u32(command, "SIZE")?.unwrap_or(DEFAULT_PING_SIZE);
    if !(1..=MAX_PING_COUNT).contains(&count)
        || !(1..=MAX_PING_TIMEOUT_MS).contains(&timeout_ms)
        || size > MAX_PING_SIZE
    {
        return Err(Status::INVALID_ARGUMENT)
    }
    let interface = optional_text(command, "INTERFACE")?;
    let source = optional_text(command, "SOURCE")?;
    let ipv4 = boolean(command.get("IPV4"))?;
    let ipv6 = boolean(command.get("IPV6"))?;
    if ipv4 && ipv6 {
        return Err(Status::INVALID_ARGUMENT)
    }
    let ip_version = match (ipv4, ipv6) {
        (true, false) => Some(PingIpVersion::Ipv4),
        (false, true) => Some(PingIpVersion::Ipv6),
        _ => None,
    };
    Ok(PingRequest {
        destination,
        count,
        timeout_ms,
        size,
        interface,
        source,
        ip_version,
    })
}

pub fn dns_update_request(command: &CommandCall) -> Result<DnsUpdate, Status> {
    let dhcp = boolean(command.get("DHCP"))?;
    let static_mode = boolean(command.get("STATIC"))?;
    if dhcp == static_mode {
        return Err(Status::INVALID_ARGUMENT)
    }
    let servers = match optional_text(command, "SERVERS")? {
        Some(value) => parse_dns_list::<MAX_DNS_SERVERS>(value, true)?,
        None => ([None; MAX_DNS_SERVERS], 0),
    };
    let search_domains = match optional_text(command, "SEARCH")? {
        Some(value) => parse_dns_list::<MAX_DNS_SEARCH_DOMAINS>(value, false)?,
        None => ([None; MAX_DNS_SEARCH_DOMAINS], 0),
    };
    if dhcp && servers.1 != 0 || static_mode && servers.1 == 0 {
        return Err(Status::INVALID_ARGUMENT)
    }
    Ok(DnsUpdate {
        mode: if dhcp { DnsMode::Dhcp } else { DnsMode::Static },
        servers: servers.0,
        server_count: servers.1,
        search_domains: search_domains.0,
        search_count: search_domains.1,
    })
}

pub fn resolve_request<'a>(command: &'a CommandCall) -> Result<ResolveRequest<'a>, Status> {
    let hostname = command
        .get_text("HOSTNAME")
        .filter(|value| !value.is_empty())
        .ok_or(Status::INVALID_ARGUMENT)?;
    let timeout_ms = optional_u32(command, "TIMEOUT")?.unwrap_or(DEFAULT_RESOLVE_TIMEOUT_MS);
    if !(1..=MAX_RESOLVE_TIMEOUT_MS).contains(&timeout_ms) {
        return Err(Status::INVALID_ARGUMENT)
    }
    let ipv4 = boolean(command.get("IPV4"))?;
    let ipv6 = boolean(command.get("IPV6"))?;
    if ipv4 && ipv6 {
        return Err(Status::INVALID_ARGUMENT)
    }
    Ok(ResolveRequest {
        hostname,
        timeout_ms,
        ip_version: match (ipv4, ipv6) {
            (true, false) => Some(PingIpVersion::Ipv4),
            (false, true) => Some(PingIpVersion::Ipv6),
            _ => None,
        },
    })
}

pub fn traceroute_request<'a>(
    command: &'a CommandCall,
) -> Result<TracerouteRequest<'a>, Status> {
    let destination = command
        .get_text("DESTINATION")
        .filter(|value| !value.is_empty())
        .ok_or(Status::INVALID_ARGUMENT)?;
    Ok(TracerouteRequest::defaults(destination))
}

pub fn packet_capture_request<'a>(
    command: &'a CommandCall,
) -> Result<PacketCaptureRequest<'a>, Status> {
    let interface = optional_text(command, "INTERFACE")?;
    let protocol = optional_text(command, "PROTOCOL")?;
    if interface.is_some_and(str::is_empty) || protocol.is_some_and(str::is_empty) {
        return Err(Status::INVALID_ARGUMENT)
    }
    let direction = match optional_text(command, "DIRECTION")? {
        None => None,
        Some(value) if value.eq_ignore_ascii_case("ingress") => Some(PacketDirection::Ingress),
        Some(value) if value.eq_ignore_ascii_case("egress") => Some(PacketDirection::Egress),
        Some(_) => return Err(Status::INVALID_ARGUMENT),
    };
    let max_records = optional_u32(command, "MAX")?.unwrap_or(DEFAULT_PACKET_MAX_RECORDS);
    if !(1..=MAX_PACKET_MAX_RECORDS).contains(&max_records) {
        return Err(Status::INVALID_ARGUMENT)
    }
    Ok(PacketCaptureRequest {
        interface,
        direction,
        protocol,
        max_records,
    })
}

fn parse_dns_list<const CAPACITY: usize>(
    value: &str,
    addresses: bool,
) -> Result<([Option<NetworkText>; CAPACITY], u8), Status> {
    let mut values = [None; CAPACITY];
    let mut count = 0usize;
    for item in value.split(',') {
        if item.is_empty()
            || count == CAPACITY
            || item.bytes().any(|byte| byte.is_ascii_whitespace())
            || (addresses && !valid_dns_server(item))
            || (!addresses && !valid_search_domain(item))
        {
            return Err(Status::INVALID_ARGUMENT)
        }
        values[count] = Some(NetworkText::new(item).map_err(|_| Status::NO_SPACE)?);
        count += 1;
    }
    Ok((values, count as u8))
}

fn valid_dns_server(value: &str) -> bool {
    if looks_like_ipv4_literal(value) {
        return parse_ipv4_literal(value).is_some()
    }
    value.contains(':')
        && value.bytes().any(|byte| byte.is_ascii_hexdigit())
        && value.bytes().all(|byte| byte == b':' || byte.is_ascii_hexdigit())
}

fn valid_search_domain(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_')
        })
        && !value.starts_with('.')
        && !value.ends_with('.')
}

pub fn resolve_literal_ipv4_target(
    request: PingRequest<'_>,
) -> Result<PingTarget, Status> {
    if !looks_like_ipv4_literal(request.destination) {
        return Err(Status::NOT_FOUND)
    }
    let address = parse_ipv4_literal(request.destination).ok_or(Status::INVALID_ARGUMENT)?;
    if request.ip_version == Some(PingIpVersion::Ipv6) {
        return Err(Status::INVALID_ARGUMENT)
    }
    let mut text = NetworkText::empty();
    write!(
        &mut text,
        "{}.{}.{}.{}",
        address[0], address[1], address[2], address[3]
    )
    .map_err(|_| Status::NO_SPACE)?;
    Ok(PingTarget {
        address: text,
        ip_version: PingIpVersion::Ipv4,
    })
}

pub fn ping_request_output(
    request: ResolvedPingRequest<'_>,
) -> Result<StructuredOutput, Status> {
    ping_result_output(request, PingResult::Success)
}

pub fn ping_result_output(
    request: ResolvedPingRequest<'_>,
    result: PingResult,
) -> Result<StructuredOutput, Status> {
    ping_summary_output(request, result, PingSummary::for_result(request.request, result))
}

pub fn ping_summary_output(
    request: ResolvedPingRequest<'_>,
    result: PingResult,
    summary: PingSummary,
) -> Result<StructuredOutput, Status> {
    if summary.received > summary.transmitted || summary.transmitted > request.request.count {
        return Err(Status::INVALID_ARGUMENT)
    }
    let reply_count = summary.replies.iter().filter(|reply| reply.is_some()).count();
    if reply_count as u32 > summary.transmitted {
        return Err(Status::INVALID_ARGUMENT)
    }
    if summary.received == 0
        && (summary.minimum_rtt_ms.is_some()
            || summary.average_rtt_ms.is_some()
            || summary.maximum_rtt_ms.is_some())
    {
        return Err(Status::INVALID_ARGUMENT)
    }
    if let (Some(minimum), Some(average), Some(maximum)) = (
        summary.minimum_rtt_ms,
        summary.average_rtt_ms,
        summary.maximum_rtt_ms,
    ) && (minimum > average || average > maximum)
    {
        return Err(Status::INVALID_ARGUMENT)
    }
    let mut output = StructuredOutput::new(result.status());
    insert_text(&mut output, "operation", "ping")?;
    insert_text(&mut output, "destination", request.request.destination)?;
    insert_text(&mut output, "address", request.target.address.as_str())?;
    insert_text(&mut output, "result", result.as_str())?;
    insert(&mut output, "result-status", OutputValue::Status(result.status()))?;
    insert(
        &mut output,
        "reply-count",
        OutputValue::Unsigned(reply_count as u64),
    )?;
    const REPLY_FIELDS: [[&str; 5]; MAX_PING_REPLY_OUTPUT] = [
        [
            "reply1-sequence",
            "reply1-ttl",
            "reply1-payload-size",
            "reply1-rtt-ms",
            "reply1-error",
        ],
        [
            "reply2-sequence",
            "reply2-ttl",
            "reply2-payload-size",
            "reply2-rtt-ms",
            "reply2-error",
        ],
        [
            "reply3-sequence",
            "reply3-ttl",
            "reply3-payload-size",
            "reply3-rtt-ms",
            "reply3-error",
        ],
    ];
    for (index, reply) in summary.replies.iter().enumerate() {
        let Some(reply) = reply else { continue };
        let fields = REPLY_FIELDS[index];
        insert(
            &mut output,
            fields[0],
            OutputValue::Unsigned(reply.sequence as u64),
        )?;
        if let Some(ttl) = reply.ttl {
            insert(&mut output, fields[1], OutputValue::Unsigned(ttl as u64))?;
        }
        insert(
            &mut output,
            fields[2],
            OutputValue::Unsigned(reply.payload_size as u64),
        )?;
        if let Some(rtt_ms) = reply.rtt_ms {
            insert(&mut output, fields[3], OutputValue::Unsigned(rtt_ms))?;
        }
        if let Some(error) = reply.error {
            insert_text(&mut output, fields[4], error.as_str())?;
        }
    }
    insert(
        &mut output,
        "transmitted",
        OutputValue::Unsigned(summary.transmitted as u64),
    )?;
    insert(
        &mut output,
        "received",
        OutputValue::Unsigned(summary.received as u64),
    )?;
    insert(
        &mut output,
        "lost",
        OutputValue::Unsigned(summary.lost() as u64),
    )?;
    insert(
        &mut output,
        "loss-percent",
        OutputValue::Unsigned(summary.loss_percent()),
    )?;
    if let Some(minimum) = summary.minimum_rtt_ms {
        insert(
            &mut output,
            "rtt-min-ms",
            OutputValue::Unsigned(minimum),
        )?;
    }
    if let Some(average) = summary.average_rtt_ms {
        insert(
            &mut output,
            "rtt-average-ms",
            OutputValue::Unsigned(average),
        )?;
    }
    if let Some(maximum) = summary.maximum_rtt_ms {
        insert(
            &mut output,
            "rtt-max-ms",
            OutputValue::Unsigned(maximum),
        )?;
    }
    insert(
        &mut output,
        "count",
        OutputValue::Unsigned(request.request.count as u64),
    )?;
    insert(
        &mut output,
        "timeout-ms",
        OutputValue::Unsigned(request.request.timeout_ms as u64),
    )?;
    insert(
        &mut output,
        "size",
        OutputValue::Unsigned(request.request.size as u64),
    )?;
    if let Some(interface) = request.request.interface {
        insert_text(&mut output, "interface", interface)?;
    }
    if let Some(source) = request.request.source {
        insert_text(&mut output, "source", source)?;
    }
    insert_text(
        &mut output,
        "ip-version",
        match request.target.ip_version {
            PingIpVersion::Ipv4 => "ipv4",
            PingIpVersion::Ipv6 => "ipv6",
        },
    )?;
    Ok(output)
}

fn looks_like_ipv4_literal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit() || byte == b'.')
}

fn parse_ipv4_literal(value: &str) -> Option<[u8; 4]> {
    let mut address = [0; 4];
    let mut octet = 0usize;
    let mut current = 0u16;
    let mut digits = 0usize;
    for byte in value.bytes() {
        if byte.is_ascii_digit() {
            if digits == 3 {
                return None
            }
            current = current
                .checked_mul(10)?
                .checked_add((byte - b'0') as u16)?;
            if current > u8::MAX as u16 {
                return None
            }
            digits += 1;
        } else if byte == b'.' {
            if digits == 0 || octet == 3 {
                return None
            }
            address[octet] = current as u8;
            octet += 1;
            current = 0;
            digits = 0;
        } else {
            return None
        }
    }
    if octet != 3 || digits == 0 {
        return None
    }
    address[3] = current as u8;
    Some(address)
}

fn optional_text<'a>(command: &'a CommandCall, name: &str) -> Result<Option<&'a str>, Status> {
    match command.get(name) {
        None => Ok(None),
        Some(Value::Text(_)) => command
            .get_text(name)
            .filter(|value| !value.is_empty())
            .map(Some)
            .ok_or(Status::INVALID_ARGUMENT),
        Some(_) => Err(Status::INVALID_ARGUMENT),
    }
}

fn optional_u32(command: &CommandCall, name: &str) -> Result<Option<u32>, Status> {
    match command.get(name) {
        None => Ok(None),
        Some(Value::Integer(value)) => u32::try_from(value)
            .map(Some)
            .map_err(|_| Status::INVALID_ARGUMENT),
        Some(_) => Err(Status::INVALID_ARGUMENT),
    }
}

fn boolean(value: Option<Value>) -> Result<bool, Status> {
    match value {
        None => Ok(false),
        Some(Value::Boolean(value)) => Ok(value),
        Some(_) => Err(Status::INVALID_ARGUMENT),
    }
}

pub fn network_output(view: NetworkView) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "show-network")?;
    insert(&mut output, "generation", OutputValue::Unsigned(view.generation))?;
    if let Some(hostname) = view.hostname {
        insert_text(&mut output, "hostname", hostname.as_str())?;
    }
    insert(
        &mut output,
        "interface-count",
        OutputValue::Unsigned(view.interface_count),
    )?;
    insert(
        &mut output,
        "route-count",
        OutputValue::Unsigned(view.route_count),
    )?;
    Ok(output)
}

pub fn neighbors_output(view: NeighborView) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "show-neighbors")?;
    insert(&mut output, "generation", OutputValue::Unsigned(view.generation))?;
    insert(
        &mut output,
        "entry-count",
        OutputValue::Unsigned(view.entry_count),
    )?;
    let mut used = output.fields().count();
    let mut omitted = None;
    for (index, entry) in view.entries.iter().flatten().enumerate() {
        let needed = 8;
        let remaining = view
            .entries
            .iter()
            .flatten()
            .skip(index + 1)
            .count()
            .saturating_add(usize::from(view.next_entry.is_some()));
        if used.saturating_add(needed).saturating_add(usize::from(remaining > 0))
            > MAX_OUTPUT_FIELDS
        {
            omitted = Some(index as u64);
            break
        }
        let fields = match index {
            0 => [
                "neighbor1-interface",
                "neighbor1-address",
                "neighbor1-ip-version",
                "neighbor1-hardware-address",
                "neighbor1-state",
                "neighbor1-last-seen-ms",
                "neighbor1-expires-ms",
                "neighbor1-attempts",
            ],
            1 => [
                "neighbor2-interface",
                "neighbor2-address",
                "neighbor2-ip-version",
                "neighbor2-hardware-address",
                "neighbor2-state",
                "neighbor2-last-seen-ms",
                "neighbor2-expires-ms",
                "neighbor2-attempts",
            ],
            _ => return Err(Status::INVALID_ARGUMENT),
        };
        insert_text(&mut output, fields[0], entry.interface.as_str())?;
        insert_text(&mut output, fields[1], entry.address.as_str())?;
        insert_text(&mut output, fields[2], entry.ip_version.as_str())?;
        if let Some(hardware_address) = entry.hardware_address {
            insert_text(&mut output, fields[3], hardware_address.as_str())?;
        }
        insert_text(&mut output, fields[4], entry.state.as_str())?;
        insert(
            &mut output,
            fields[5],
            OutputValue::Unsigned(entry.last_seen_ms),
        )?;
        if let Some(expires_at_ms) = entry.expires_at_ms {
            insert(
                &mut output,
                fields[6],
                OutputValue::Unsigned(expires_at_ms),
            )?;
        }
        insert(
            &mut output,
            fields[7],
            OutputValue::Unsigned(entry.attempts as u64),
        )?;
        used = used.saturating_add(needed);
    }
    if let Some(next) = omitted.or(view.next_entry) {
        insert(
            &mut output,
            "next-neighbor",
            OutputValue::Unsigned(next),
        )?;
    }
    Ok(output)
}

fn clear_neighbors_output(cleared: u64) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "clear-neighbors")?;
    insert(
        &mut output,
        "cleared-count",
        OutputValue::Unsigned(cleared),
    )?;
    Ok(output)
}

pub fn dns_output(view: DnsView) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "show-dns")?;
    insert(&mut output, "generation", OutputValue::Unsigned(view.generation))?;
    insert_text(&mut output, "mode", view.mode.as_str())?;
    insert(
        &mut output,
        "server-count",
        OutputValue::Unsigned(view.server_count as u64),
    )?;
    insert(
        &mut output,
        "search-count",
        OutputValue::Unsigned(view.search_count as u64),
    )?;
    insert(
        &mut output,
        "dhcp-owned",
        OutputValue::Boolean(matches!(view.mode, DnsMode::Dhcp)),
    )?;
    insert(
        &mut output,
        "static-override",
        OutputValue::Boolean(matches!(view.mode, DnsMode::Static)),
    )?;
    insert_text(&mut output, "query-status", view.query_status.as_str())?;
    insert(
        &mut output,
        "query-timeout-ms",
        OutputValue::Unsigned(view.query_timeout_ms as u64),
    )?;
    if let Some(query_name) = view.query_name {
        insert_text(&mut output, "query-name", query_name.as_str())?;
    }
    for (index, server) in view.servers.iter().flatten().enumerate() {
        let fields = match index {
            0 => ["server1-address", "server1-source", "server1-order"],
            1 => ["server2-address", "server2-source", "server2-order"],
            2 => ["server3-address", "server3-source", "server3-order"],
            _ => return Err(Status::INVALID_ARGUMENT),
        };
        insert_text(&mut output, fields[0], server.address.as_str())?;
        insert_text(&mut output, fields[1], server.source.as_str())?;
        insert(
            &mut output,
            fields[2],
            OutputValue::Unsigned(server.order as u64),
        )?;
    }
    for (index, domain) in view.search_domains.iter().flatten().enumerate() {
        let field = match index {
            0 => "search1-domain",
            1 => "search2-domain",
            2 => "search3-domain",
            _ => return Err(Status::INVALID_ARGUMENT),
        };
        insert_text(&mut output, field, domain.as_str())?;
    }
    Ok(output)
}

pub fn resolve_output(view: ResolveView) -> Result<StructuredOutput, Status> {
    let actual_count = view.answers.iter().filter(|answer| answer.is_some()).count();
    if actual_count != view.answer_count as usize
        || actual_count > MAX_RESOLVE_ANSWERS
        || (!matches!(view.result, ResolveResult::Success) && actual_count != 0)
    {
        return Err(Status::INVALID_ARGUMENT)
    }
    let mut output = StructuredOutput::new(view.result.status());
    insert_text(&mut output, "operation", "resolve")?;
    insert_text(&mut output, "hostname", view.hostname.as_str())?;
    insert_text(&mut output, "result", view.result.as_str())?;
    insert(
        &mut output,
        "result-status",
        OutputValue::Status(view.result.status()),
    )?;
    if let Some(resolver) = view.resolver {
        insert_text(&mut output, "resolver", resolver.as_str())?;
    }
    insert(
        &mut output,
        "timeout-ms",
        OutputValue::Unsigned(view.timeout_ms as u64),
    )?;
    insert(
        &mut output,
        "elapsed-ms",
        OutputValue::Unsigned(view.elapsed_ms as u64),
    )?;
    insert(
        &mut output,
        "answer-count",
        OutputValue::Unsigned(view.answer_count as u64),
    )?;
    for (index, answer) in view.answers.iter().flatten().enumerate() {
        let fields = match index {
            0 => ["answer1-address", "answer1-ip-version", "answer1-ttl-ms"],
            1 => ["answer2-address", "answer2-ip-version", "answer2-ttl-ms"],
            2 => ["answer3-address", "answer3-ip-version", "answer3-ttl-ms"],
            3 => ["answer4-address", "answer4-ip-version", "answer4-ttl-ms"],
            _ => return Err(Status::INVALID_ARGUMENT),
        };
        insert_text(&mut output, fields[0], answer.address.as_str())?;
        insert_text(&mut output, fields[1], match answer.ip_version {
            PingIpVersion::Ipv4 => "ipv4",
            PingIpVersion::Ipv6 => "ipv6",
        })?;
        insert(&mut output, fields[2], OutputValue::Unsigned(answer.ttl_ms))?;
    }
    Ok(output)
}

pub fn sockets_output(view: SocketView) -> Result<StructuredOutput, Status> {
    let actual_count = view.sockets.iter().filter(|socket| socket.is_some()).count();
    if actual_count > MAX_SOCKET_OUTPUT_ROWS || actual_count as u64 > view.socket_count {
        return Err(Status::INVALID_ARGUMENT)
    }
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "show-sockets")?;
    insert(&mut output, "generation", OutputValue::Unsigned(view.generation))?;
    insert(
        &mut output,
        "socket-count",
        OutputValue::Unsigned(view.socket_count),
    )?;
    let mut used = output.fields().count();
    let mut omitted = None;
    for (index, socket) in view.sockets.iter().flatten().enumerate() {
        let needed = 10;
        let remaining = view
            .sockets
            .iter()
            .flatten()
            .skip(index + 1)
            .count()
            .saturating_add(usize::from(view.next_socket.is_some()));
        if used.saturating_add(needed).saturating_add(usize::from(remaining > 0))
            > MAX_OUTPUT_FIELDS
        {
            omitted = Some(index as u64);
            break
        }
        let fields = match index {
            0 => [
                "socket1-protocol",
                "socket1-local-endpoint",
                "socket1-remote-endpoint",
                "socket1-owner",
                "socket1-owner-redacted",
                "socket1-capability",
                "socket1-state",
                "socket1-rx-queue-bytes",
                "socket1-tx-queue-bytes",
                "socket1-lifetime-ms",
            ],
            1 => [
                "socket2-protocol",
                "socket2-local-endpoint",
                "socket2-remote-endpoint",
                "socket2-owner",
                "socket2-owner-redacted",
                "socket2-capability",
                "socket2-state",
                "socket2-rx-queue-bytes",
                "socket2-tx-queue-bytes",
                "socket2-lifetime-ms",
            ],
            _ => return Err(Status::INVALID_ARGUMENT),
        };
        insert_text(&mut output, fields[0], socket.protocol.as_str())?;
        insert_text(&mut output, fields[1], socket.local_endpoint.as_str())?;
        if let Some(remote_endpoint) = socket.remote_endpoint {
            insert_text(&mut output, fields[2], remote_endpoint.as_str())?;
        }
        if !socket.owner_redacted {
            if let Some(owner) = socket.owner {
                insert_text(&mut output, fields[3], owner.as_str())?;
            }
        }
        insert(
            &mut output,
            fields[4],
            OutputValue::Boolean(socket.owner_redacted),
        )?;
        insert(
            &mut output,
            fields[5],
            OutputValue::Unsigned(socket.capability),
        )?;
        insert_text(&mut output, fields[6], socket.state.as_str())?;
        insert(
            &mut output,
            fields[7],
            OutputValue::Unsigned(socket.rx_queue_bytes),
        )?;
        insert(
            &mut output,
            fields[8],
            OutputValue::Unsigned(socket.tx_queue_bytes),
        )?;
        insert(
            &mut output,
            fields[9],
            OutputValue::Unsigned(socket.lifetime_ms),
        )?;
        used = used.saturating_add(needed);
    }
    if let Some(next) = omitted.or(view.next_socket) {
        insert(
            &mut output,
            "next-socket",
            OutputValue::Unsigned(next),
        )?;
    }
    Ok(output)
}

pub fn network_stats_output(view: NetworkStatsView) -> Result<StructuredOutput, Status> {
    let actual_count = view
        .interfaces
        .iter()
        .filter(|interface| interface.is_some())
        .count();
    if actual_count > MAX_NETWORK_STATS_INTERFACES || actual_count as u64 > view.interface_count {
        return Err(Status::INVALID_ARGUMENT)
    }
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "show-network-stats")?;
    insert(&mut output, "generation", OutputValue::Unsigned(view.generation))?;
    insert(
        &mut output,
        "reset-generation",
        OutputValue::Unsigned(view.reset_generation),
    )?;
    insert(
        &mut output,
        "interface-count",
        OutputValue::Unsigned(view.interface_count),
    )?;
    let mut used = output.fields().count();
    let mut omitted = None;
    for (index, interface) in view.interfaces.iter().flatten().enumerate() {
        let needed = 7;
        let fixed_counter_fields = 19;
        let remaining = view
            .interfaces
            .iter()
            .flatten()
            .skip(index + 1)
            .count()
            .saturating_add(usize::from(view.next_interface.is_some()));
        if used
            .saturating_add(needed)
            .saturating_add(fixed_counter_fields)
            .saturating_add(usize::from(remaining > 0))
            > MAX_OUTPUT_FIELDS
        {
            omitted = Some(index as u64);
            break
        }
        let fields = match index {
            0 => [
                "interface1-name",
                "interface1-rx-packets",
                "interface1-rx-bytes",
                "interface1-tx-packets",
                "interface1-tx-bytes",
                "interface1-drops",
                "interface1-errors",
            ],
            1 => [
                "interface2-name",
                "interface2-rx-packets",
                "interface2-rx-bytes",
                "interface2-tx-packets",
                "interface2-tx-bytes",
                "interface2-drops",
                "interface2-errors",
            ],
            _ => return Err(Status::INVALID_ARGUMENT),
        };
        insert_text(&mut output, fields[0], interface.name.as_str())?;
        insert(
            &mut output,
            fields[1],
            OutputValue::Unsigned(interface.rx_packets),
        )?;
        insert(
            &mut output,
            fields[2],
            OutputValue::Unsigned(interface.rx_bytes),
        )?;
        insert(
            &mut output,
            fields[3],
            OutputValue::Unsigned(interface.tx_packets),
        )?;
        insert(
            &mut output,
            fields[4],
            OutputValue::Unsigned(interface.tx_bytes),
        )?;
        insert(
            &mut output,
            fields[5],
            OutputValue::Unsigned(interface.drops),
        )?;
        insert(
            &mut output,
            fields[6],
            OutputValue::Unsigned(interface.errors),
        )?;
        used = used.saturating_add(needed);
    }
    insert(
        &mut output,
        "dhcp-discovers",
        OutputValue::Unsigned(view.dhcp.discovers),
    )?;
    insert(
        &mut output,
        "dhcp-offers",
        OutputValue::Unsigned(view.dhcp.offers),
    )?;
    insert(
        &mut output,
        "dhcp-retries",
        OutputValue::Unsigned(view.dhcp.retries),
    )?;
    insert(
        &mut output,
        "dhcp-failures",
        OutputValue::Unsigned(view.dhcp.failures),
    )?;
    insert(
        &mut output,
        "arp-requests",
        OutputValue::Unsigned(view.arp.requests),
    )?;
    insert(
        &mut output,
        "arp-replies",
        OutputValue::Unsigned(view.arp.replies),
    )?;
    insert(
        &mut output,
        "arp-failures",
        OutputValue::Unsigned(view.arp.failures),
    )?;
    insert(
        &mut output,
        "icmp-received",
        OutputValue::Unsigned(view.icmp.received),
    )?;
    insert(
        &mut output,
        "icmp-transmitted",
        OutputValue::Unsigned(view.icmp.transmitted),
    )?;
    insert(
        &mut output,
        "icmp-loss",
        OutputValue::Unsigned(view.icmp.loss),
    )?;
    insert(
        &mut output,
        "udp-received",
        OutputValue::Unsigned(view.udp.received),
    )?;
    insert(
        &mut output,
        "udp-transmitted",
        OutputValue::Unsigned(view.udp.transmitted),
    )?;
    insert(
        &mut output,
        "udp-dropped",
        OutputValue::Unsigned(view.udp.dropped),
    )?;
    insert(
        &mut output,
        "tcp-received",
        OutputValue::Unsigned(view.tcp.received),
    )?;
    insert(
        &mut output,
        "tcp-transmitted",
        OutputValue::Unsigned(view.tcp.transmitted),
    )?;
    insert(
        &mut output,
        "tcp-dropped",
        OutputValue::Unsigned(view.tcp.dropped),
    )?;
    insert(
        &mut output,
        "firewall-allowed",
        OutputValue::Unsigned(view.firewall.allowed),
    )?;
    insert(
        &mut output,
        "firewall-dropped",
        OutputValue::Unsigned(view.firewall.dropped),
    )?;
    insert(
        &mut output,
        "firewall-rejected",
        OutputValue::Unsigned(view.firewall.rejected),
    )?;
    if let Some(next) = omitted.or(view.next_interface) {
        insert(
            &mut output,
            "next-interface",
            OutputValue::Unsigned(next),
        )?;
    }
    Ok(output)
}

pub fn traceroute_output(view: TracerouteView) -> Result<StructuredOutput, Status> {
    let actual_count = view.hops.iter().filter(|hop| hop.is_some()).count();
    if actual_count > MAX_TRACEROUTE_OUTPUT_HOPS
        || actual_count > view.hop_count as usize
        || view.max_hops == 0
        || view.max_hops > TRACEROUTE_MAX_HOPS
        || view.hop_timeout_ms == 0
        || view.hop_timeout_ms > TRACEROUTE_HOP_TIMEOUT_MS
        || view.probe_interval_ms < TRACEROUTE_PROBE_INTERVAL_MS
        || view.total_deadline_ms == 0
        || view.total_deadline_ms > TRACEROUTE_TOTAL_DEADLINE_MS
        || view.hop_count > view.max_hops
    {
        return Err(Status::INVALID_ARGUMENT)
    }
    let mut output = StructuredOutput::new(view.result.status());
    insert_text(&mut output, "operation", "traceroute")?;
    insert_text(&mut output, "destination", view.destination.as_str())?;
    insert_text(&mut output, "result", view.result.as_str())?;
    insert(
        &mut output,
        "result-status",
        OutputValue::Status(view.result.status()),
    )?;
    if let Some(interface) = view.route_interface {
        insert_text(&mut output, "route-interface", interface.as_str())?;
    }
    if let Some(gateway) = view.route_gateway {
        insert_text(&mut output, "route-gateway", gateway.as_str())?;
    }
    insert(
        &mut output,
        "max-hops",
        OutputValue::Unsigned(view.max_hops as u64),
    )?;
    insert(
        &mut output,
        "hop-count",
        OutputValue::Unsigned(view.hop_count as u64),
    )?;
    insert(
        &mut output,
        "hop-timeout-ms",
        OutputValue::Unsigned(view.hop_timeout_ms as u64),
    )?;
    insert(
        &mut output,
        "probe-interval-ms",
        OutputValue::Unsigned(view.probe_interval_ms as u64),
    )?;
    insert(
        &mut output,
        "total-deadline-ms",
        OutputValue::Unsigned(view.total_deadline_ms as u64),
    )?;
    for (index, hop) in view.hops.iter().flatten().enumerate() {
        if hop.ttl == 0 || hop.ttl > view.max_hops {
            return Err(Status::INVALID_ARGUMENT)
        }
        let fields = match index {
            0 => [
                "hop1-ttl",
                "hop1-address",
                "hop1-result",
                "hop1-rtt-ms",
                "hop1-error",
            ],
            1 => [
                "hop2-ttl",
                "hop2-address",
                "hop2-result",
                "hop2-rtt-ms",
                "hop2-error",
            ],
            2 => [
                "hop3-ttl",
                "hop3-address",
                "hop3-result",
                "hop3-rtt-ms",
                "hop3-error",
            ],
            3 => [
                "hop4-ttl",
                "hop4-address",
                "hop4-result",
                "hop4-rtt-ms",
                "hop4-error",
            ],
            _ => return Err(Status::INVALID_ARGUMENT),
        };
        insert(
            &mut output,
            fields[0],
            OutputValue::Unsigned(hop.ttl as u64),
        )?;
        if let Some(address) = hop.address {
            insert_text(&mut output, fields[1], address.as_str())?;
        }
        insert_text(&mut output, fields[2], hop.result.as_str())?;
        if let Some(rtt_ms) = hop.rtt_ms {
            insert(
                &mut output,
                fields[3],
                OutputValue::Unsigned(rtt_ms),
            )?;
        }
        if let Some(error) = hop.error {
            insert_text(&mut output, fields[4], error.as_str())?;
        }
    }
    if let Some(next) = view.next_hop {
        insert(&mut output, "next-hop", OutputValue::Unsigned(next))?;
    }
    Ok(output)
}

pub fn packet_capture_output(
    view: PacketCaptureView,
) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "show-packets")?;
    insert(&mut output, "generation", OutputValue::Unsigned(view.generation))?;
    insert(&mut output, "record-count", OutputValue::Unsigned(view.record_count))?;
    insert(&mut output, "dropped-count", OutputValue::Unsigned(view.dropped_count))?;
    insert(&mut output, "expired-count", OutputValue::Unsigned(view.expired_count))?;
    insert(
        &mut output,
        "capture-expires-at-ms",
        OutputValue::Unsigned(view.capture_expires_at_ms),
    )?;
    for (index, record) in view.records.iter().flatten().enumerate() {
        insert_indexed(
            &mut output,
            "packet",
            index,
            "sequence",
            OutputValue::Unsigned(record.sequence),
        )?;
        insert_indexed(
            &mut output,
            "packet",
            index,
            "timestamp-ms",
            OutputValue::Unsigned(record.timestamp_ms),
        )?;
        insert_indexed_text(
            &mut output,
            "packet",
            index,
            "interface",
            record.interface.as_str(),
        )?;
        insert_indexed_text(
            &mut output,
            "packet",
            index,
            "direction",
            record.direction.as_str(),
        )?;
        insert_indexed_text(
            &mut output,
            "packet",
            index,
            "protocol",
            record.protocol.as_str(),
        )?;
        insert_indexed_text(
            &mut output,
            "packet",
            index,
            "source",
            record.source.as_str(),
        )?;
        insert_indexed_text(
            &mut output,
            "packet",
            index,
            "destination",
            record.destination.as_str(),
        )?;
        insert_indexed(
            &mut output,
            "packet",
            index,
            "length",
            OutputValue::Unsigned(record.length as u64),
        )?;
        insert_indexed(
            &mut output,
            "packet",
            index,
            "original-length",
            OutputValue::Unsigned(record.original_length as u64),
        )?;
        insert_indexed(
            &mut output,
            "packet",
            index,
            "truncated",
            OutputValue::Boolean(record.truncated),
        )?;
        insert_indexed_text(
            &mut output,
            "packet",
            index,
            "payload",
            record.payload.as_str(),
        )?;
        insert_indexed(
            &mut output,
            "packet",
            index,
            "payload-redacted",
            OutputValue::Boolean(record.payload_redacted),
        )?;
    }
    if let Some(next) = view.next_record {
        insert(&mut output, "next-record", OutputValue::Unsigned(next))?;
    }
    Ok(output)
}

fn network_operation_output(
    view: NetworkView,
    operation: &str,
) -> Result<StructuredOutput, Status> {
    let mut output = network_output(view)?;
    insert_text(&mut output, "operation", operation)?;
    Ok(output)
}

pub fn set_interface_output(
    view: NetworkView,
    name: &str,
) -> Result<StructuredOutput, Status> {
    let interface = view
        .interfaces
        .iter()
        .flatten()
        .find(|interface| interface.name.as_str().eq_ignore_ascii_case(name));
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "set-interface")?;
    insert_text(
        &mut output,
        "interface",
        interface.map_or(name, |interface| interface.name.as_str()),
    )?;
    insert(&mut output, "generation", OutputValue::Unsigned(view.generation))?;
    if let Some(interface) = interface {
        emit_interface_details(&mut output, interface)?;
    }
    Ok(output)
}

fn emit_interface_details(
    output: &mut StructuredOutput,
    interface: &NetworkInterfaceView,
) -> Result<(), Status> {
    insert_text(output, "address", interface.address.as_str())?;
    if let Some(prefix_len) = interface.prefix_len {
        insert(output, "prefix-len", OutputValue::Unsigned(prefix_len as u64))?;
    }
    if let Some(mac) = interface.mac {
        insert_text(output, "mac", mac.as_str())?;
    }
    if let Some(gateway) = interface.gateway {
        insert_text(output, "gateway", gateway.as_str())?;
    }
    insert(output, "mtu", OutputValue::Unsigned(interface.mtu as u64))?;
    insert(output, "enabled", OutputValue::Boolean(interface.enabled))?;
    insert(output, "link-up", OutputValue::Boolean(interface.link_up))?;
    if let Some(queue) = interface.rx_queue {
        insert_queue_details(output, "rx-queue", queue)?;
    }
    if let Some(queue) = interface.tx_queue {
        insert_queue_details(output, "tx-queue", queue)?;
    }
    insert_text(output, "mode", interface.mode.as_str())?;
    if let Some(dhcp) = interface.dhcp {
        insert_text(output, "dhcp-state", dhcp.state.as_str())?;
        if let Some(transaction_id) = dhcp.transaction_id {
            insert(output, "dhcp-transaction-id", OutputValue::Unsigned(transaction_id as u64))?;
        }
        if let Some(client_mac) = dhcp.client_mac {
            insert_text(output, "dhcp-client-mac", client_mac.as_str())?;
        }
        if let Some(attempt) = dhcp.attempt {
            insert(output, "dhcp-attempt", OutputValue::Unsigned(attempt as u64))?;
        }
        if let Some(server) = dhcp.server {
            insert_text(output, "dhcp-server", server.as_str())?;
        }
        if let Some(offered_address) = dhcp.offered_address {
            insert_text(output, "dhcp-offered-address", offered_address.as_str())?;
        }
        if let Some(bound_at_ms) = dhcp.bound_at_ms {
            insert(output, "dhcp-bound-ms", OutputValue::Unsigned(bound_at_ms))?;
        }
        if let Some(next_action_ms) = dhcp.next_action_ms {
            insert(output, "dhcp-next-action-ms", OutputValue::Unsigned(next_action_ms))?;
        }
        if let Some(t1_at_ms) = dhcp.t1_at_ms {
            insert(output, "dhcp-t1-ms", OutputValue::Unsigned(t1_at_ms))?;
        }
        if let Some(t2_at_ms) = dhcp.t2_at_ms {
            insert(output, "dhcp-t2-ms", OutputValue::Unsigned(t2_at_ms))?;
        }
        if let Some(expires) = dhcp.expires_at_ms {
            insert(output, "dhcp-expires-ms", OutputValue::Unsigned(expires))?;
        }
        if let Some(failure_reason) = dhcp.failure_reason {
            insert_text(output, "dhcp-failure", failure_reason.as_str())?;
        }
        if let Some(last_packet_at_ms) = dhcp.last_packet_at_ms {
            insert(
                output,
                "dhcp-last-packet-ms",
                OutputValue::Unsigned(last_packet_at_ms),
            )?;
        }
        if let Some(dns0) = dhcp.dns0 {
            insert_text(output, "dns0", dns0.as_str())?;
        }
        if let Some(dns1) = dhcp.dns1 {
            insert_text(output, "dns1", dns1.as_str())?;
        }
    }
    Ok(())
}

pub fn interfaces_output(view: NetworkView) -> Result<StructuredOutput, Status> {
    let mut output = network_output(view)?;
    insert_text(&mut output, "operation", "show-interfaces")?;
    let mut used = output.fields().count();
    let mut omitted = None;
    for (index, interface) in view.interfaces.iter().flatten().enumerate() {
        let needed = interface_field_count(interface);
        // Reserve one slot for next-interface when more rows remain in this page
        // or the source already provided a continuation marker.
        let remaining = view
            .interfaces
            .iter()
            .flatten()
            .skip(index + 1)
            .count()
            .saturating_add(usize::from(view.next_interface.is_some()));
        let reserve = usize::from(remaining > 0);
        if used.saturating_add(needed).saturating_add(reserve) > MAX_OUTPUT_FIELDS {
            let mut identity_fields = 0;
            if used.saturating_add(1).saturating_add(reserve) <= MAX_OUTPUT_FIELDS {
                insert_indexed_text(
                    &mut output,
                    "interface",
                    index,
                    "name",
                    interface.name.as_str(),
                )?;
                identity_fields += 1;
            }
            if used
                .saturating_add(identity_fields)
                .saturating_add(1)
                .saturating_add(reserve)
                <= MAX_OUTPUT_FIELDS
            {
                insert_indexed_text(
                    &mut output,
                    "interface",
                    index,
                    "address",
                    interface.address.as_str(),
                )?;
                identity_fields += 1;
            }
            if identity_fields != 0 {
                omitted = Some(index.saturating_add(1) as u64);
            } else {
                omitted = Some(index as u64);
            }
            break;
        }
        emit_interface(&mut output, index, interface)?;
        used = used.saturating_add(needed);
    }
    if let Some(next) = omitted.or(view.next_interface) {
        insert(&mut output, "next-interface", OutputValue::Unsigned(next))?;
    }
    for (index, event) in view.link_events.iter().flatten().enumerate() {
        if output.fields().count().saturating_add(3) > MAX_OUTPUT_FIELDS {
            break
        }
        insert_indexed_text(
            &mut output,
            "link-event",
            index,
            "interface",
            event.interface.as_str(),
        )?;
        insert_indexed(
            &mut output,
            "link-event",
            index,
            "generation",
            OutputValue::Unsigned(event.generation),
        )?;
        insert_indexed(
            &mut output,
            "link-event",
            index,
            "up",
            OutputValue::Boolean(event.link_up),
        )?;
    }
    Ok(output)
}

pub fn show_network_output(view: NetworkView) -> Result<StructuredOutput, Status> {
    let mut output = interfaces_output(view)?;
    insert_text(&mut output, "operation", "show-network")?;
    let mut used = output.fields().count();
    let mut omitted = None;
    for (index, route) in view.routes.iter().flatten().enumerate() {
        if used.saturating_add(4).saturating_add(1) > MAX_OUTPUT_FIELDS {
            omitted = Some(index as u64);
            break;
        }
        emit_route(&mut output, index, route)?;
        used = used.saturating_add(4);
    }
    if let Some(next) = omitted.or(view.next_route) {
        insert(&mut output, "next-route", OutputValue::Unsigned(next))?;
    }
    Ok(output)
}

pub fn show_interface_output(
    view: NetworkView,
    name: &str,
) -> Result<StructuredOutput, Status> {
    let interface = view
        .interfaces
        .iter()
        .flatten()
        .find(|interface| interface.name.as_str().eq_ignore_ascii_case(name))
        .copied()
        .ok_or(Status::NOT_FOUND)?;
    let mut selected = view;
    selected.interfaces = [None; MAX_NETWORK_OUTPUT_ROWS];
    selected.interfaces[0] = Some(interface);
    selected.interface_count = 1;
    selected.next_interface = None;
    let mut output = interfaces_output(selected)?;
    insert_text(&mut output, "operation", "show-interface")?;
    Ok(output)
}

fn interface_field_count(interface: &NetworkInterfaceView) -> usize {
    let mut count = 6; // name address mtu enabled link-up mode
    if interface.prefix_len.is_some() {
        count += 1;
    }
    if interface.mac.is_some() {
        count += 1;
    }
    if interface.rx_queue.is_some() {
        count += queue_field_count(interface.rx_queue.unwrap());
    }
    if interface.tx_queue.is_some() {
        count += queue_field_count(interface.tx_queue.unwrap());
    }
    if interface.gateway.is_some() {
        count += 1;
    }
    if let Some(dhcp) = interface.dhcp {
        count += 1; // dhcp-state
        if dhcp.transaction_id.is_some() {
            count += 1;
        }
        if dhcp.client_mac.is_some() {
            count += 1;
        }
        if dhcp.attempt.is_some() {
            count += 1;
        }
        if dhcp.server.is_some() {
            count += 1;
        }
        if dhcp.offered_address.is_some() {
            count += 1;
        }
        if dhcp.bound_at_ms.is_some() {
            count += 1;
        }
        if dhcp.next_action_ms.is_some() {
            count += 1;
        }
        if dhcp.t1_at_ms.is_some() {
            count += 1;
        }
        if dhcp.t2_at_ms.is_some() {
            count += 1;
        }
        if dhcp.expires_at_ms.is_some() {
            count += 1;
        }
        if dhcp.failure_reason.is_some() {
            count += 1;
        }
        if dhcp.last_packet_at_ms.is_some() {
            count += 1;
        }
        if dhcp.dns0.is_some() {
            count += 1;
        }
        if dhcp.dns1.is_some() {
            count += 1;
        }
    }
    count
}

fn emit_interface(
    output: &mut StructuredOutput,
    index: usize,
    interface: &NetworkInterfaceView,
) -> Result<(), Status> {
    insert_indexed_text(output, "interface", index, "name", interface.name.as_str())?;
    insert_indexed_text(
        output,
        "interface",
        index,
        "address",
        interface.address.as_str(),
    )?;
    if let Some(prefix_len) = interface.prefix_len {
        insert_indexed(
            output,
            "interface",
            index,
            "prefix-len",
            OutputValue::Unsigned(prefix_len as u64),
        )?;
    }
    if let Some(gateway) = interface.gateway {
        insert_indexed_text(output, "interface", index, "gateway", gateway.as_str())?;
    }
    if let Some(mac) = interface.mac {
        insert_indexed_text(output, "interface", index, "mac", mac.as_str())?;
    }
    insert_indexed(
        output,
        "interface",
        index,
        "mtu",
        OutputValue::Unsigned(interface.mtu as u64),
    )?;
    insert_indexed(
        output,
        "interface",
        index,
        "enabled",
        OutputValue::Boolean(interface.enabled),
    )?;
    insert_indexed(
        output,
        "interface",
        index,
        "link-up",
        OutputValue::Boolean(interface.link_up),
    )?;
    if let Some(queue) = interface.rx_queue {
        insert_indexed_queue(output, index, "rx-queue", queue)?;
    }
    if let Some(queue) = interface.tx_queue {
        insert_indexed_queue(output, index, "tx-queue", queue)?;
    }
    insert_indexed_text(output, "interface", index, "mode", interface.mode.as_str())?;
    if let Some(dhcp) = interface.dhcp {
        insert_indexed_text(output, "interface", index, "dhcp-state", dhcp.state.as_str())?;
        if let Some(transaction_id) = dhcp.transaction_id {
            insert_indexed(
                output,
                "interface",
                index,
                "dhcp-transaction-id",
                OutputValue::Unsigned(transaction_id as u64),
            )?;
        }
        if let Some(client_mac) = dhcp.client_mac {
            insert_indexed_text(output, "interface", index, "dhcp-client-mac", client_mac.as_str())?;
        }
        if let Some(attempt) = dhcp.attempt {
            insert_indexed(
                output,
                "interface",
                index,
                "dhcp-attempt",
                OutputValue::Unsigned(attempt as u64),
            )?;
        }
        if let Some(server) = dhcp.server {
            insert_indexed_text(output, "interface", index, "dhcp-server", server.as_str())?;
        }
        if let Some(offered_address) = dhcp.offered_address {
            insert_indexed_text(
                output,
                "interface",
                index,
                "dhcp-offered-address",
                offered_address.as_str(),
            )?;
        }
        if let Some(bound_at_ms) = dhcp.bound_at_ms {
            insert_indexed(
                output,
                "interface",
                index,
                "dhcp-bound-ms",
                OutputValue::Unsigned(bound_at_ms),
            )?;
        }
        if let Some(next_action_ms) = dhcp.next_action_ms {
            insert_indexed(
                output,
                "interface",
                index,
                "dhcp-next-action-ms",
                OutputValue::Unsigned(next_action_ms),
            )?;
        }
        if let Some(t1_at_ms) = dhcp.t1_at_ms {
            insert_indexed(
                output,
                "interface",
                index,
                "dhcp-t1-ms",
                OutputValue::Unsigned(t1_at_ms),
            )?;
        }
        if let Some(t2_at_ms) = dhcp.t2_at_ms {
            insert_indexed(
                output,
                "interface",
                index,
                "dhcp-t2-ms",
                OutputValue::Unsigned(t2_at_ms),
            )?;
        }
        if let Some(expires) = dhcp.expires_at_ms {
            insert_indexed(
                output,
                "interface",
                index,
                "dhcp-expires-ms",
                OutputValue::Unsigned(expires),
            )?;
        }
        if let Some(failure_reason) = dhcp.failure_reason {
            insert_indexed_text(
                output,
                "interface",
                index,
                "dhcp-failure",
                failure_reason.as_str(),
            )?;
        }
        if let Some(last_packet_at_ms) = dhcp.last_packet_at_ms {
            insert_indexed(
                output,
                "interface",
                index,
                "dhcp-last-packet-ms",
                OutputValue::Unsigned(last_packet_at_ms),
            )?;
        }
        if let Some(dns0) = dhcp.dns0 {
            insert_indexed_text(output, "interface", index, "dns0", dns0.as_str())?;
        }
        if let Some(dns1) = dhcp.dns1 {
            insert_indexed_text(output, "interface", index, "dns1", dns1.as_str())?;
        }
    }
    Ok(())
}

fn insert_queue_details(
    output: &mut StructuredOutput,
    prefix: &str,
    queue: NetworkQueueView,
) -> Result<(), Status> {
    insert(output, queue_name(prefix, "ready"), OutputValue::Boolean(queue.ready))?;
    if let Some(head) = queue.head {
        insert(
            output,
            queue_name(prefix, "head"),
            OutputValue::Unsigned(head as u64),
        )?;
    }
    if let Some(tail) = queue.tail {
        insert(
            output,
            queue_name(prefix, "tail"),
            OutputValue::Unsigned(tail as u64),
        )?;
    }
    insert(
        output,
        queue_name(prefix, "capacity"),
        OutputValue::Unsigned(queue.capacity as u64),
    )
}

fn insert_indexed_queue(
    output: &mut StructuredOutput,
    index: usize,
    prefix: &str,
    queue: NetworkQueueView,
) -> Result<(), Status> {
    insert_indexed(
        output,
        "interface",
        index,
        queue_name(prefix, "ready"),
        OutputValue::Boolean(queue.ready),
    )?;
    if let Some(head) = queue.head {
        insert_indexed(
            output,
            "interface",
            index,
            queue_name(prefix, "head"),
            OutputValue::Unsigned(head as u64),
        )?;
    }
    if let Some(tail) = queue.tail {
        insert_indexed(
            output,
            "interface",
            index,
            queue_name(prefix, "tail"),
            OutputValue::Unsigned(tail as u64),
        )?;
    }
    insert_indexed(
        output,
        "interface",
        index,
        queue_name(prefix, "capacity"),
        OutputValue::Unsigned(queue.capacity as u64),
    )
}

fn queue_field_count(queue: NetworkQueueView) -> usize {
    2 + usize::from(queue.head.is_some()) + usize::from(queue.tail.is_some())
}

fn queue_name(prefix: &str, suffix: &str) -> &'static str {
    match (prefix, suffix) {
        ("rx-queue", "ready") => "rx-queue-ready",
        ("rx-queue", "head") => "rx-queue-head",
        ("rx-queue", "tail") => "rx-queue-tail",
        ("rx-queue", "capacity") => "rx-queue-capacity",
        ("tx-queue", "ready") => "tx-queue-ready",
        ("tx-queue", "head") => "tx-queue-head",
        ("tx-queue", "tail") => "tx-queue-tail",
        ("tx-queue", "capacity") => "tx-queue-capacity",
        _ => "queue-unknown",
    }
}

fn routes_output(view: NetworkView) -> Result<StructuredOutput, Status> {
    let mut output = network_output(view)?;
    insert_text(&mut output, "operation", "show-routes")?;
    for (index, route) in view.routes.iter().flatten().enumerate() {
        emit_route(&mut output, index, route)?;
    }
    if let Some(next) = view.next_route {
        insert(&mut output, "next-route", OutputValue::Unsigned(next))?;
    }
    Ok(output)
}

fn emit_route(
    output: &mut StructuredOutput,
    index: usize,
    route: &NetworkRouteView,
) -> Result<(), Status> {
    insert_indexed_text(output, "route", index, "destination", route.destination.as_str())?;
    insert_indexed_text(output, "route", index, "gateway", route.gateway.as_str())?;
    insert_indexed_text(output, "route", index, "interface", route.interface.as_str())?;
    insert_indexed(
        output,
        "route",
        index,
        "metric",
        OutputValue::Unsigned(route.metric as u64),
    )
}

fn insert_text(output: &mut StructuredOutput, name: &str, value: &str) -> Result<(), Status> {
    output
        .insert(
            name,
            OutputValue::Text(
                synos_system_model::command::OutputText::new(value)
                    .map_err(|_| Status::NO_SPACE)?,
            ),
        )
        .map_err(|_| Status::NO_SPACE)
}

fn insert(output: &mut StructuredOutput, name: &str, value: OutputValue) -> Result<(), Status> {
    output.insert(name, value).map_err(|_| Status::NO_SPACE)
}

fn insert_indexed_text(
    output: &mut StructuredOutput,
    prefix: &str,
    index: usize,
    suffix: &str,
    value: &str,
) -> Result<(), Status> {
    insert_indexed(
        output,
        prefix,
        index,
        suffix,
        OutputValue::Text(
            synos_system_model::command::OutputText::new(value)
                .map_err(|_| Status::NO_SPACE)?,
        ),
    )
}

fn insert_indexed(
    output: &mut StructuredOutput,
    prefix: &str,
    index: usize,
    suffix: &str,
    value: OutputValue,
) -> Result<(), Status> {
    let mut bytes = [0u8; 32];
    let prefix = prefix.as_bytes();
    let suffix = suffix.as_bytes();
    let mut number = index.saturating_add(1);
    let mut digits = [0u8; 10];
    let mut digit_count = 0usize;
    loop {
        digits[digit_count] = b'0' + (number % 10) as u8;
        digit_count += 1;
        number /= 10;
        if number == 0 {
            break;
        }
    }
    let total = prefix
        .len()
        .saturating_add(digit_count)
        .saturating_add(1)
        .saturating_add(suffix.len());
    if total > bytes.len() {
        return Err(Status::NO_SPACE);
    }
    let mut len = 0usize;
    bytes[len..len + prefix.len()].copy_from_slice(prefix);
    len += prefix.len();
    while digit_count != 0 {
        digit_count -= 1;
        bytes[len] = digits[digit_count];
        len += 1;
    }
    bytes[len] = b'-';
    len += 1;
    bytes[len..len + suffix.len()].copy_from_slice(suffix);
    len += suffix.len();
    let name = core::str::from_utf8(&bytes[..len]).map_err(|_| Status::INVALID_ARGUMENT)?;
    output.insert(name, value).map_err(|_| Status::NO_SPACE)
}

fn route(raw: u16) -> RouteId {
    RouteId::from_valid_raw(raw)
}

fn positional(name: &str, kind: ArgumentKind, required: bool) -> Result<ArgumentSpec, Error> {
    ArgumentSpec::new(name, kind, required, true).map_err(|_| Error::InvalidValue)
}

fn qualifier(name: &str, kind: ArgumentKind) -> Result<ArgumentSpec, Error> {
    ArgumentSpec::new(name, kind, false, false).map_err(|_| Error::InvalidValue)
}
