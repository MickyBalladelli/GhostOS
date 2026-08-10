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

pub const MAX_NETWORK_OUTPUT_ROWS: usize = 4;
pub const MAX_NETWORK_LINK_EVENTS: usize = 4;
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

    fn show_network(&mut self) -> Result<NetworkView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn show_interfaces(&mut self) -> Result<NetworkView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn show_routes(&mut self) -> Result<NetworkView, Status> {
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
            omitted = Some(index as u64);
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
