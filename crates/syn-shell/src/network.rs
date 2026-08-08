use synos_status::Status;
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

pub const MAX_NETWORK_OUTPUT_ROWS: usize = 4;

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
        synopsis: "SHOW INTERFACES",
        description: "Show bounded interface settings, address mode, link state, and DHCP lease details.",
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
    pub server: Option<NetworkText>,
    pub expires_at_ms: Option<u64>,
    pub dns0: Option<NetworkText>,
    pub dns1: Option<NetworkText>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkInterfaceView {
    pub name: NetworkText,
    pub address: NetworkText,
    pub gateway: Option<NetworkText>,
    pub mtu: u32,
    pub enabled: bool,
    pub link_up: bool,
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

/// Source of truth for network settings.
///
/// A system provider should validate the caller's network-administration
/// capability, create a new declarative configuration revision, stage it,
/// health-check it, commit it, and persist it before returning the new view.
pub trait NetworkSource {
    /// Prove the caller has the network-administration capability.
    fn authorize_mutation(&mut self) -> Result<(), Status>;

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
}

pub fn register_network_commands<const CAPACITY: usize>(
    registry: &mut CommandRegistry<CAPACITY>,
) -> Result<(), Error> {
    registry.register(
        CommandSpec::new("SHOW-NETWORK", &[]).map_err(|_| Error::InvalidValue)?,
        route(SHOW_NETWORK_ROUTE),
    )?;
    registry.register(
        CommandSpec::new("SHOW-INTERFACES", &[]).map_err(|_| Error::InvalidValue)?,
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
    )
}

pub struct NetworkExecutor<Source, const CAPACITY: usize = 16> {
    source: Source,
    completions: [Option<Result<StructuredOutput, Status>>; CAPACITY],
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
        SHOW_NETWORK_ROUTE => source.show_network().and_then(network_output),
        SHOW_INTERFACES_ROUTE => source.show_interfaces().and_then(interfaces_output),
        SHOW_ROUTES_ROUTE => source.show_routes().and_then(routes_output),
        SET_HOSTNAME_ROUTE => {
            let hostname = command
                .get_text("HOSTNAME")
                .filter(|value| !value.is_empty())
                .ok_or(Status::INVALID_ARGUMENT)?;
            source.authorize_mutation()?;
            source.set_hostname(hostname).and_then(network_output)
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
            source.set_route(update).and_then(network_output)
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
        let completion = dispatch_network_command(&mut self.source, command);
        self.completions[slot] = Some(completion);
        ExecutionToken::new((slot + 1) as u64).ok_or(Error::InvalidHandle)
    }

    fn poll(&mut self, token: ExecutionToken) -> Option<Result<StructuredOutput, Status>> {
        self.completions
            .get_mut(token.raw().checked_sub(1)? as usize)?
            .take()
    }

    fn cancel(&mut self, token: ExecutionToken) -> Result<(), Error> {
        let completion = self
            .completions
            .get_mut(token.raw().checked_sub(1).ok_or(Error::InvalidHandle)? as usize)
            .ok_or(Error::InvalidHandle)?;
        *completion = None;
        Ok(())
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
    if let Some(gateway) = interface.gateway {
        insert_text(output, "gateway", gateway.as_str())?;
    }
    insert(output, "mtu", OutputValue::Unsigned(interface.mtu as u64))?;
    insert(output, "enabled", OutputValue::Boolean(interface.enabled))?;
    insert(output, "link-up", OutputValue::Boolean(interface.link_up))?;
    insert_text(output, "mode", interface.mode.as_str())?;
    if let Some(dhcp) = interface.dhcp {
        insert_text(output, "dhcp-state", dhcp.state.as_str())?;
        if let Some(server) = dhcp.server {
            insert_text(output, "dhcp-server", server.as_str())?;
        }
        if let Some(expires) = dhcp.expires_at_ms {
            insert(output, "dhcp-expires-ms", OutputValue::Unsigned(expires))?;
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
    Ok(output)
}

fn interface_field_count(interface: &NetworkInterfaceView) -> usize {
    let mut count = 6; // name address mtu enabled link-up mode
    if interface.gateway.is_some() {
        count += 1;
    }
    if let Some(dhcp) = interface.dhcp {
        count += 1; // dhcp-state
        if dhcp.server.is_some() {
            count += 1;
        }
        if dhcp.expires_at_ms.is_some() {
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
    if let Some(gateway) = interface.gateway {
        insert_indexed_text(output, "interface", index, "gateway", gateway.as_str())?;
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
    insert_indexed_text(output, "interface", index, "mode", interface.mode.as_str())?;
    if let Some(dhcp) = interface.dhcp {
        insert_indexed_text(output, "interface", index, "dhcp-state", dhcp.state.as_str())?;
        if let Some(server) = dhcp.server {
            insert_indexed_text(output, "interface", index, "dhcp-server", server.as_str())?;
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
        if let Some(dns0) = dhcp.dns0 {
            insert_indexed_text(output, "interface", index, "dns0", dns0.as_str())?;
        }
        if let Some(dns1) = dhcp.dns1 {
            insert_indexed_text(output, "interface", index, "dns1", dns1.as_str())?;
        }
    }
    Ok(())
}

fn routes_output(view: NetworkView) -> Result<StructuredOutput, Status> {
    let mut output = network_output(view)?;
    insert_text(&mut output, "operation", "show-routes")?;
    for (index, route) in view.routes.iter().flatten().enumerate() {
        insert_indexed_text(&mut output, "route", index, "destination", route.destination.as_str())?;
        insert_indexed_text(&mut output, "route", index, "gateway", route.gateway.as_str())?;
        insert_indexed_text(&mut output, "route", index, "interface", route.interface.as_str())?;
        insert_indexed(
            &mut output,
            "route",
            index,
            "metric",
            OutputValue::Unsigned(route.metric as u64),
        )?;
    }
    if let Some(next) = view.next_route {
        insert(&mut output, "next-route", OutputValue::Unsigned(next))?;
    }
    Ok(output)
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
    RouteId::new(raw).expect("network route is non-zero")
}

fn positional(name: &str, kind: ArgumentKind, required: bool) -> Result<ArgumentSpec, Error> {
    ArgumentSpec::new(name, kind, required, true).map_err(|_| Error::InvalidValue)
}

fn qualifier(name: &str, kind: ArgumentKind) -> Result<ArgumentSpec, Error> {
    ArgumentSpec::new(name, kind, false, false).map_err(|_| Error::InvalidValue)
}
