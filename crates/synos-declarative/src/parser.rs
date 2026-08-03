use core::fmt;

use synos_status::{IntoStatus, Status};
use synos_system_model::ContentId;

pub const SYSTEM_SCHEMA_VERSION: u16 = 1;
pub const MAX_SERVICES: usize = 24;
pub const MAX_CAPABILITY_POLICIES: usize = 64;
pub const MAX_NETWORK_INTERFACES: usize = 32;
pub const MAX_NETWORK_ROUTES: usize = 64;
pub const MAX_SERVICE_NAME_BYTES: usize = 48;
pub const MAX_RESOURCE_NAME_BYTES: usize = 64;
pub const MAX_ADDRESS_BYTES: usize = 64;
const MAX_CANONICAL_BYTES: usize = 32_768;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParseError {
    Capacity,
    DuplicateKey,
    DuplicateName,
    InvalidBoolean,
    InvalidInteger,
    InvalidList,
    InvalidString,
    InvalidValue,
    MissingField,
    UnknownKey,
    UnknownSection,
    UnsupportedSchema,
}

impl IntoStatus for ParseError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::UnknownSection | Self::UnknownKey | Self::MissingField => Status::NOT_FOUND,
            Self::UnsupportedSchema
            | Self::DuplicateKey
            | Self::DuplicateName
            | Self::InvalidBoolean
            | Self::InvalidInteger
            | Self::InvalidList
            | Self::InvalidString
            | Self::InvalidValue => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct BoundedText<const CAPACITY: usize> {
    bytes: [u8; CAPACITY],
    length: u8,
}

impl<const CAPACITY: usize> BoundedText<CAPACITY> {
    pub const EMPTY: Self = Self {
        bytes: [0; CAPACITY],
        length: 0,
    };

    pub fn new(value: &str) -> Result<Self, ParseError> {
        if value.is_empty()
            || value.len() > CAPACITY
            || CAPACITY > u8::MAX as usize
            || value.as_bytes().contains(&0)
        {
            return Err(ParseError::InvalidString);
        }
        let mut bytes = [0; CAPACITY];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            length: value.len() as u8,
        })
    }

    pub const fn is_empty(self) -> bool {
        self.length == 0
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.length as usize])
            .expect("BoundedText contains valid UTF-8")
    }
}

impl<const CAPACITY: usize> fmt::Debug for BoundedText<CAPACITY> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("BoundedText")
            .field(&self.as_str())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceKind {
    System,
    Network,
    Storage,
    Compute,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartPolicy {
    Never,
    OnFailure,
    Always,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceSpec {
    pub name: BoundedText<MAX_SERVICE_NAME_BYTES>,
    pub image: u128,
    pub kind: ServiceKind,
    pub enabled: bool,
    pub restart: RestartPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityKind {
    Ipc,
    File,
    Network,
    Memory,
    Device,
    Clock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct CapabilityRights(u16);

impl CapabilityRights {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const EXECUTE: Self = Self(1 << 2);
    pub const MAP: Self = Self(1 << 3);
    pub const BIND: Self = Self(1 << 4);
    pub const CONNECT: Self = Self(1 << 5);
    pub const SEND: Self = Self(1 << 6);
    pub const RECEIVE: Self = Self(1 << 7);

    pub const fn bits(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityPolicy {
    pub service: BoundedText<MAX_SERVICE_NAME_BYTES>,
    pub resource: BoundedText<MAX_RESOURCE_NAME_BYTES>,
    pub kind: CapabilityKind,
    pub rights: u16,
    pub required: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkInterface {
    pub name: BoundedText<MAX_SERVICE_NAME_BYTES>,
    pub address: BoundedText<MAX_ADDRESS_BYTES>,
    pub gateway: Option<BoundedText<MAX_ADDRESS_BYTES>>,
    pub mtu: u32,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkRoute {
    pub destination: BoundedText<MAX_ADDRESS_BYTES>,
    pub gateway: BoundedText<MAX_ADDRESS_BYTES>,
    pub interface: BoundedText<MAX_SERVICE_NAME_BYTES>,
    pub metric: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkSpec {
    pub hostname: Option<BoundedText<MAX_ADDRESS_BYTES>>,
    interfaces: [Option<NetworkInterface>; MAX_NETWORK_INTERFACES],
    routes: [Option<NetworkRoute>; MAX_NETWORK_ROUTES],
}

impl NetworkSpec {
    pub fn interfaces(&self) -> impl Iterator<Item = NetworkInterface> + '_ {
        self.interfaces.iter().flatten().copied()
    }

    pub fn routes(&self) -> impl Iterator<Item = NetworkRoute> + '_ {
        self.routes.iter().flatten().copied()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemSpec {
    schema: u16,
    revision: u64,
    services: [Option<ServiceSpec>; MAX_SERVICES],
    capabilities: [Option<CapabilityPolicy>; MAX_CAPABILITY_POLICIES],
    network: NetworkSpec,
}

impl SystemSpec {
    pub fn parse(source: &str) -> Result<Self, ParseError> {
        Parser::new().parse(source)
    }

    pub const fn schema(&self) -> u16 {
        self.schema
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub fn services(&self) -> impl Iterator<Item = ServiceSpec> + '_ {
        self.services.iter().flatten().copied()
    }

    pub fn capabilities(&self) -> impl Iterator<Item = CapabilityPolicy> + '_ {
        self.capabilities.iter().flatten().copied()
    }

    pub const fn network(&self) -> &NetworkSpec {
        &self.network
    }

    pub fn digest(&self) -> ContentId {
        let mut encoder = Encoder::new();
        encoder.u16(self.schema);
        encoder.u64(self.revision);
        for service in self.services() {
            encoder.u8(1);
            encoder.text(service.name.as_str());
            encoder.u128(service.image);
            encoder.u8(service.kind as u8);
            encoder.u8(service.enabled as u8);
            encoder.u8(service.restart as u8);
        }
        encoder.u8(2);
        for policy in self.capabilities() {
            encoder.text(policy.service.as_str());
            encoder.text(policy.resource.as_str());
            encoder.u8(policy.kind as u8);
            encoder.u16(policy.rights);
            encoder.u8(policy.required as u8);
        }
        encoder.u8(3);
        if let Some(hostname) = self.network.hostname {
            encoder.u8(1);
            encoder.text(hostname.as_str());
        } else {
            encoder.u8(0);
        }
        for interface in self.network.interfaces() {
            encoder.u8(4);
            encoder.text(interface.name.as_str());
            encoder.text(interface.address.as_str());
            encode_optional_text(&mut encoder, interface.gateway);
            encoder.u32(interface.mtu);
            encoder.u8(interface.enabled as u8);
        }
        for route in self.network.routes() {
            encoder.u8(5);
            encoder.text(route.destination.as_str());
            encoder.text(route.gateway.as_str());
            encoder.text(route.interface.as_str());
            encoder.u32(route.metric);
        }
        ContentId::hash(&encoder.bytes[..encoder.length])
    }
}

struct Encoder {
    bytes: [u8; MAX_CANONICAL_BYTES],
    length: usize,
}

impl Encoder {
    const fn new() -> Self {
        Self {
            bytes: [0; MAX_CANONICAL_BYTES],
            length: 0,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        let end = self.length + bytes.len();
        assert!(end <= MAX_CANONICAL_BYTES);
        self.bytes[self.length..end].copy_from_slice(bytes);
        self.length = end;
    }

    fn u8(&mut self, value: u8) {
        self.push(&[value])
    }

    fn u16(&mut self, value: u16) {
        self.push(&value.to_be_bytes())
    }

    fn u32(&mut self, value: u32) {
        self.push(&value.to_be_bytes())
    }

    fn u64(&mut self, value: u64) {
        self.push(&value.to_be_bytes())
    }

    fn u128(&mut self, value: u128) {
        self.push(&value.to_be_bytes())
    }

    fn text(&mut self, value: &str) {
        self.u8(value.len() as u8);
        self.push(value.as_bytes())
    }
}

fn encode_optional_text<const CAPACITY: usize>(
    encoder: &mut Encoder,
    value: Option<BoundedText<CAPACITY>>,
) {
    match value {
        Some(value) => {
            encoder.u8(1);
            encoder.text(value.as_str());
        }
        None => encoder.u8(0),
    }
}

#[derive(Clone, Copy)]
struct ServicePartial {
    name: Option<BoundedText<MAX_SERVICE_NAME_BYTES>>,
    image: Option<u128>,
    kind: Option<ServiceKind>,
    enabled: Option<bool>,
    restart: Option<RestartPolicy>,
}

impl ServicePartial {
    const fn new() -> Self {
        Self {
            name: None,
            image: None,
            kind: None,
            enabled: None,
            restart: None,
        }
    }
}

#[derive(Clone, Copy)]
struct CapabilityPartial {
    service: Option<BoundedText<MAX_SERVICE_NAME_BYTES>>,
    resource: Option<BoundedText<MAX_RESOURCE_NAME_BYTES>>,
    kind: Option<CapabilityKind>,
    rights: Option<u16>,
    required: Option<bool>,
}

impl CapabilityPartial {
    const fn new() -> Self {
        Self {
            service: None,
            resource: None,
            kind: None,
            rights: None,
            required: None,
        }
    }
}

#[derive(Clone, Copy)]
struct InterfacePartial {
    name: Option<BoundedText<MAX_SERVICE_NAME_BYTES>>,
    address: Option<BoundedText<MAX_ADDRESS_BYTES>>,
    gateway: Option<BoundedText<MAX_ADDRESS_BYTES>>,
    mtu: Option<u32>,
    enabled: Option<bool>,
}

impl InterfacePartial {
    const fn new() -> Self {
        Self {
            name: None,
            address: None,
            gateway: None,
            mtu: None,
            enabled: None,
        }
    }
}

#[derive(Clone, Copy)]
struct RoutePartial {
    destination: Option<BoundedText<MAX_ADDRESS_BYTES>>,
    gateway: Option<BoundedText<MAX_ADDRESS_BYTES>>,
    interface: Option<BoundedText<MAX_SERVICE_NAME_BYTES>>,
    metric: Option<u32>,
}

impl RoutePartial {
    const fn new() -> Self {
        Self {
            destination: None,
            gateway: None,
            interface: None,
            metric: None,
        }
    }
}

#[derive(Clone, Copy)]
enum Section {
    Root,
    System,
    Network,
    Service,
    Capability,
    Interface,
    Route,
}

struct Parser {
    section: Section,
    schema: Option<u16>,
    revision: Option<u64>,
    hostname: Option<BoundedText<MAX_ADDRESS_BYTES>>,
    service: Option<ServicePartial>,
    capability: Option<CapabilityPartial>,
    interface: Option<InterfacePartial>,
    route: Option<RoutePartial>,
    services: [Option<ServiceSpec>; MAX_SERVICES],
    service_count: usize,
    capabilities: [Option<CapabilityPolicy>; MAX_CAPABILITY_POLICIES],
    capability_count: usize,
    interfaces: [Option<NetworkInterface>; MAX_NETWORK_INTERFACES],
    interface_count: usize,
    routes: [Option<NetworkRoute>; MAX_NETWORK_ROUTES],
    route_count: usize,
}

impl Parser {
    const fn new() -> Self {
        Self {
            section: Section::Root,
            schema: None,
            revision: None,
            hostname: None,
            service: None,
            capability: None,
            interface: None,
            route: None,
            services: [None; MAX_SERVICES],
            service_count: 0,
            capabilities: [None; MAX_CAPABILITY_POLICIES],
            capability_count: 0,
            interfaces: [None; MAX_NETWORK_INTERFACES],
            interface_count: 0,
            routes: [None; MAX_NETWORK_ROUTES],
            route_count: 0,
        }
    }

    fn parse(mut self, source: &str) -> Result<SystemSpec, ParseError> {
        for raw_line in source.lines() {
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                self.finish_current()?;
                self.section = parse_section(line)?;
                self.start_section();
                continue;
            }
            let (key, value) = split_key_value(line)?;
            self.assign(key, value)?;
        }
        self.finish_current()?;

        let schema = self.schema.ok_or(ParseError::MissingField)?;
        if schema != SYSTEM_SCHEMA_VERSION {
            return Err(ParseError::UnsupportedSchema);
        }
        let revision = self.revision.ok_or(ParseError::MissingField)?;
        if revision == 0 {
            return Err(ParseError::InvalidValue);
        }
        let mut services = [None; MAX_SERVICES];
        services[..self.service_count].copy_from_slice(&self.services[..self.service_count]);
        let mut capabilities = [None; MAX_CAPABILITY_POLICIES];
        capabilities[..self.capability_count]
            .copy_from_slice(&self.capabilities[..self.capability_count]);
        let mut interfaces = [None; MAX_NETWORK_INTERFACES];
        interfaces[..self.interface_count]
            .copy_from_slice(&self.interfaces[..self.interface_count]);
        let mut routes = [None; MAX_NETWORK_ROUTES];
        routes[..self.route_count].copy_from_slice(&self.routes[..self.route_count]);
        if self.capabilities.iter().flatten().any(|policy| {
            !services
                .iter()
                .flatten()
                .any(|service| service.name == policy.service)
        }) || self.routes.iter().flatten().any(|route| {
            !interfaces
                .iter()
                .flatten()
                .any(|interface| interface.name == route.interface)
        }) {
            return Err(ParseError::InvalidValue);
        }
        Ok(SystemSpec {
            schema,
            revision,
            services,
            capabilities,
            network: NetworkSpec {
                hostname: self.hostname,
                interfaces,
                routes,
            },
        })
    }

    fn start_section(&mut self) {
        match self.section {
            Section::Service => self.service = Some(ServicePartial::new()),
            Section::Capability => self.capability = Some(CapabilityPartial::new()),
            Section::Interface => self.interface = Some(InterfacePartial::new()),
            Section::Route => self.route = Some(RoutePartial::new()),
            Section::Root | Section::System | Section::Network => {}
        }
    }

    fn finish_current(&mut self) -> Result<(), ParseError> {
        match self.section {
            Section::Service => {
                let partial = self.service.take().ok_or(ParseError::MissingField)?;
                let service = ServiceSpec {
                    name: partial.name.ok_or(ParseError::MissingField)?,
                    image: partial.image.ok_or(ParseError::MissingField)?,
                    kind: partial.kind.ok_or(ParseError::MissingField)?,
                    enabled: partial.enabled.unwrap_or(true),
                    restart: partial.restart.unwrap_or(RestartPolicy::Never),
                };
                if service.image == 0 || self.services().any(|entry| entry.name == service.name) {
                    return Err(if service.image == 0 {
                        ParseError::InvalidValue
                    } else {
                        ParseError::DuplicateName
                    });
                }
                if self.service_count == MAX_SERVICES {
                    return Err(ParseError::Capacity);
                }
                self.services[self.service_count] = Some(service);
                self.service_count += 1;
            }
            Section::Capability => {
                let partial = self.capability.take().ok_or(ParseError::MissingField)?;
                let policy = CapabilityPolicy {
                    service: partial.service.ok_or(ParseError::MissingField)?,
                    resource: partial.resource.ok_or(ParseError::MissingField)?,
                    kind: partial.kind.ok_or(ParseError::MissingField)?,
                    rights: partial.rights.ok_or(ParseError::MissingField)?,
                    required: partial.required.unwrap_or(true),
                };
                if policy.rights == 0 {
                    return Err(ParseError::InvalidValue);
                }
                if self.capability_count == MAX_CAPABILITY_POLICIES {
                    return Err(ParseError::Capacity);
                }
                self.capabilities[self.capability_count] = Some(policy);
                self.capability_count += 1;
            }
            Section::Interface => {
                let partial = self.interface.take().ok_or(ParseError::MissingField)?;
                let interface = NetworkInterface {
                    name: partial.name.ok_or(ParseError::MissingField)?,
                    address: partial.address.ok_or(ParseError::MissingField)?,
                    gateway: partial.gateway,
                    mtu: partial.mtu.unwrap_or(1500),
                    enabled: partial.enabled.unwrap_or(true),
                };
                if interface.mtu < 576 || interface.mtu > 65_535 {
                    return Err(ParseError::InvalidValue);
                }
                if self
                    .interfaces
                    .iter()
                    .flatten()
                    .any(|entry| entry.name == interface.name)
                {
                    return Err(ParseError::DuplicateName);
                }
                if self.interface_count == MAX_NETWORK_INTERFACES {
                    return Err(ParseError::Capacity);
                }
                self.interfaces[self.interface_count] = Some(interface);
                self.interface_count += 1;
            }
            Section::Route => {
                let partial = self.route.take().ok_or(ParseError::MissingField)?;
                let route = NetworkRoute {
                    destination: partial.destination.ok_or(ParseError::MissingField)?,
                    gateway: partial.gateway.ok_or(ParseError::MissingField)?,
                    interface: partial.interface.ok_or(ParseError::MissingField)?,
                    metric: partial.metric.unwrap_or(100),
                };
                if self.route_count == MAX_NETWORK_ROUTES {
                    return Err(ParseError::Capacity);
                }
                self.routes[self.route_count] = Some(route);
                self.route_count += 1;
            }
            Section::Root | Section::System | Section::Network => {}
        }
        Ok(())
    }

    fn assign(&mut self, key: &str, value: &str) -> Result<(), ParseError> {
        match self.section {
            Section::Root | Section::System => match key {
                "schema" => set_once(&mut self.schema, parse_u16(value)?),
                "revision" => set_once(&mut self.revision, parse_u64(value)?),
                _ => return Err(ParseError::UnknownKey),
            }?,
            Section::Network => match key {
                "hostname" => set_once(&mut self.hostname, parse_text::<MAX_ADDRESS_BYTES>(value)?),
                _ => return Err(ParseError::UnknownKey),
            }?,
            Section::Service => {
                self.assign_service(key, value)?;
            }
            Section::Capability => {
                self.assign_capability(key, value)?;
            }
            Section::Interface => {
                self.assign_interface(key, value)?;
            }
            Section::Route => {
                self.assign_route(key, value)?;
            }
        }
        Ok(())
    }

    fn assign_service(&mut self, key: &str, value: &str) -> Result<(), ParseError> {
        let partial = self.service.as_mut().ok_or(ParseError::MissingField)?;
        match key {
            "name" => set_once(&mut partial.name, parse_text(value)?),
            "image" => set_once(&mut partial.image, parse_u128(value)?),
            "kind" => set_once(&mut partial.kind, parse_service_kind(value)?),
            "enabled" => set_once(&mut partial.enabled, parse_bool(value)?),
            "restart" => set_once(&mut partial.restart, parse_restart(value)?),
            _ => return Err(ParseError::UnknownKey),
        }?;
        Ok(())
    }

    fn assign_capability(&mut self, key: &str, value: &str) -> Result<(), ParseError> {
        let partial = self.capability.as_mut().ok_or(ParseError::MissingField)?;
        match key {
            "service" => set_once(&mut partial.service, parse_text(value)?),
            "resource" => set_once(&mut partial.resource, parse_text(value)?),
            "kind" => set_once(&mut partial.kind, parse_capability_kind(value)?),
            "rights" => set_once(&mut partial.rights, parse_rights(value)?),
            "required" => set_once(&mut partial.required, parse_bool(value)?),
            _ => return Err(ParseError::UnknownKey),
        }?;
        Ok(())
    }

    fn assign_interface(&mut self, key: &str, value: &str) -> Result<(), ParseError> {
        let partial = self.interface.as_mut().ok_or(ParseError::MissingField)?;
        match key {
            "name" => set_once(&mut partial.name, parse_text(value)?),
            "address" => set_once(&mut partial.address, parse_text(value)?),
            "gateway" => set_once(&mut partial.gateway, parse_text(value)?),
            "mtu" => set_once(&mut partial.mtu, parse_u32(value)?),
            "enabled" => set_once(&mut partial.enabled, parse_bool(value)?),
            _ => return Err(ParseError::UnknownKey),
        }?;
        Ok(())
    }

    fn assign_route(&mut self, key: &str, value: &str) -> Result<(), ParseError> {
        let partial = self.route.as_mut().ok_or(ParseError::MissingField)?;
        match key {
            "destination" => set_once(&mut partial.destination, parse_text(value)?),
            "gateway" => set_once(&mut partial.gateway, parse_text(value)?),
            "interface" => set_once(&mut partial.interface, parse_text(value)?),
            "metric" => set_once(&mut partial.metric, parse_u32(value)?),
            _ => return Err(ParseError::UnknownKey),
        }?;
        Ok(())
    }

    fn services(&self) -> impl Iterator<Item = ServiceSpec> + '_ {
        self.services.iter().flatten().copied()
    }
}

fn set_once<T>(slot: &mut Option<T>, value: T) -> Result<(), ParseError> {
    if slot.is_some() {
        Err(ParseError::DuplicateKey)
    } else {
        *slot = Some(value);
        Ok(())
    }
}

fn parse_section(line: &str) -> Result<Section, ParseError> {
    if line.starts_with("[[") && line.ends_with("]]") {
        return match &line[2..line.len() - 2] {
            "services" | "service" => Ok(Section::Service),
            "capabilities" | "capability" => Ok(Section::Capability),
            "network.interfaces" | "network.interface" => Ok(Section::Interface),
            "network.routes" | "network.route" => Ok(Section::Route),
            _ => Err(ParseError::UnknownSection),
        };
    }
    if line.starts_with('[') && line.ends_with(']') {
        return match &line[1..line.len() - 1] {
            "system" => Ok(Section::System),
            "network" => Ok(Section::Network),
            _ => Err(ParseError::UnknownSection),
        };
    }
    Err(ParseError::UnknownSection)
}

fn split_key_value(line: &str) -> Result<(&str, &str), ParseError> {
    let (key, value) = line.split_once('=').ok_or(ParseError::InvalidValue)?;
    let key = key.trim();
    if key.is_empty() {
        return Err(ParseError::InvalidValue);
    }
    Ok((key, value.trim()))
}

fn strip_comment(line: &str) -> &str {
    let mut quote = None;
    for (index, byte) in line.bytes().enumerate() {
        match (quote, byte) {
            (None, b'"') | (None, b'\'') => quote = Some(byte),
            (Some(expected), value) if expected == value => quote = None,
            (None, b'#') => return &line[..index],
            _ => {}
        }
    }
    line
}

fn parse_text<const CAPACITY: usize>(value: &str) -> Result<BoundedText<CAPACITY>, ParseError> {
    let (value, escaped) = if let Some(value) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        (value, true)
    } else if let Some(value) = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        (value, false)
    } else {
        return Err(ParseError::InvalidString);
    };
    if escaped && value.contains('\\') {
        return Err(ParseError::InvalidString);
    }
    BoundedText::new(value)
}

fn parse_bool(value: &str) -> Result<bool, ParseError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(ParseError::InvalidBoolean),
    }
}

fn parse_u16(value: &str) -> Result<u16, ParseError> {
    parse_integer(value)
        .and_then(|value| u16::try_from(value).map_err(|_| ParseError::InvalidInteger))
}

fn parse_u32(value: &str) -> Result<u32, ParseError> {
    parse_integer(value)
        .and_then(|value| u32::try_from(value).map_err(|_| ParseError::InvalidInteger))
}

fn parse_u64(value: &str) -> Result<u64, ParseError> {
    parse_integer(value)
}

fn parse_u128(value: &str) -> Result<u128, ParseError> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    let radix = if value.len()
        != value
            .trim_start_matches(|byte: char| byte.is_ascii_digit())
            .len()
    {
        10
    } else {
        16
    };
    u128::from_str_radix(value, radix).map_err(|_| ParseError::InvalidInteger)
}

fn parse_integer(value: &str) -> Result<u64, ParseError> {
    if let Some(value) = value.strip_prefix("0x") {
        u64::from_str_radix(value, 16).map_err(|_| ParseError::InvalidInteger)
    } else {
        value.parse().map_err(|_| ParseError::InvalidInteger)
    }
}

fn parse_service_kind(value: &str) -> Result<ServiceKind, ParseError> {
    match quoted_value(value) {
        "system" => Ok(ServiceKind::System),
        "network" => Ok(ServiceKind::Network),
        "storage" => Ok(ServiceKind::Storage),
        "compute" => Ok(ServiceKind::Compute),
        _ => Err(ParseError::InvalidValue),
    }
}

fn parse_restart(value: &str) -> Result<RestartPolicy, ParseError> {
    match quoted_value(value) {
        "never" => Ok(RestartPolicy::Never),
        "on-failure" => Ok(RestartPolicy::OnFailure),
        "always" => Ok(RestartPolicy::Always),
        _ => Err(ParseError::InvalidValue),
    }
}

fn parse_capability_kind(value: &str) -> Result<CapabilityKind, ParseError> {
    match quoted_value(value) {
        "ipc" => Ok(CapabilityKind::Ipc),
        "file" => Ok(CapabilityKind::File),
        "network" => Ok(CapabilityKind::Network),
        "memory" => Ok(CapabilityKind::Memory),
        "device" => Ok(CapabilityKind::Device),
        "clock" => Ok(CapabilityKind::Clock),
        _ => Err(ParseError::InvalidValue),
    }
}

fn parse_rights(value: &str) -> Result<u16, ParseError> {
    let mut rights = 0;
    let value = value.trim();
    if value.starts_with('[') {
        if !value.ends_with(']') {
            return Err(ParseError::InvalidList);
        }
        for item in value[1..value.len() - 1].split(',') {
            if item.trim().is_empty() {
                return Err(ParseError::InvalidList);
            }
            rights |= right_bit(quoted_value(item.trim()))?;
        }
    } else {
        for item in quoted_value(value).split('|') {
            rights |= right_bit(item.trim())?;
        }
    }
    if rights == 0 {
        Err(ParseError::InvalidValue)
    } else {
        Ok(rights)
    }
}

fn right_bit(value: &str) -> Result<u16, ParseError> {
    match value {
        "read" => Ok(CapabilityRights::READ.bits()),
        "write" => Ok(CapabilityRights::WRITE.bits()),
        "execute" => Ok(CapabilityRights::EXECUTE.bits()),
        "map" => Ok(CapabilityRights::MAP.bits()),
        "bind" => Ok(CapabilityRights::BIND.bits()),
        "connect" => Ok(CapabilityRights::CONNECT.bits()),
        "send" => Ok(CapabilityRights::SEND.bits()),
        "receive" => Ok(CapabilityRights::RECEIVE.bits()),
        _ => Err(ParseError::InvalidValue),
    }
}

fn quoted_value(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
}
