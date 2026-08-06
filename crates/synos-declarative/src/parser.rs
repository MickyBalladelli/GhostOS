use core::fmt;

use synos_status::{IntoStatus, Status};
use synos_system_model::ContentId;

use crate::{
    AdmissionPolicy, ClusterSpec, DiscoveryPolicy, FederationSpec, MembershipPolicy,
    NodeOverrideSpec, QuorumSpec, ResourceSpec, SecuritySpec, TransportKind, TransportSpec,
    ConfigurationValidationError, MAX_CLUSTER_DESCRIPTION_BYTES, MAX_CLUSTER_NAME_BYTES,
    MAX_ENDPOINT_BYTES, MAX_TRANSPORT_NAME_BYTES,
};

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
    pub const ADMIN: Self = Self(1 << 8);

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
#[repr(u8)]
pub enum AddressMode {
    Static = 0,
    Dhcp = 1,
}

impl AddressMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Static => "static",
            Self::Dhcp => "dhcp",
        }
    }

    pub fn parse(value: &str) -> Result<Self, ParseError> {
        match quoted_value(value) {
            "static" => Ok(Self::Static),
            "dhcp" => Ok(Self::Dhcp),
            _ => Err(ParseError::InvalidValue),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkInterface {
    pub name: BoundedText<MAX_SERVICE_NAME_BYTES>,
    pub address: BoundedText<MAX_ADDRESS_BYTES>,
    pub gateway: Option<BoundedText<MAX_ADDRESS_BYTES>>,
    pub mtu: u32,
    pub enabled: bool,
    pub mode: AddressMode,
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

    pub fn validate(&self) -> Result<(), ParseError> {
        for (index, interface) in self.interfaces().enumerate() {
            let address_ok = match interface.mode {
                AddressMode::Static => !interface.address.is_empty(),
                AddressMode::Dhcp => true,
            };
            if interface.name.is_empty()
                || !address_ok
                || !(576..=65_535).contains(&interface.mtu)
                || self
                    .interfaces()
                    .take(index)
                    .any(|previous| previous.name == interface.name)
            {
                return Err(ParseError::InvalidValue);
            }
        }
        for route in self.routes() {
            if route.destination.is_empty()
                || route.gateway.is_empty()
                || route.interface.is_empty()
                || !self
                    .interfaces()
                    .any(|interface| interface.name == route.interface)
            {
                return Err(ParseError::InvalidValue);
            }
        }
        Ok(())
    }

    pub fn set_hostname(&mut self, hostname: &str) -> Result<(), ParseError> {
        self.hostname = Some(BoundedText::new(hostname)?);
        Ok(())
    }

    pub fn update_interface(
        &mut self,
        name: &str,
        address: Option<&str>,
        gateway: Option<&str>,
        mtu: Option<u32>,
        enabled: Option<bool>,
        mode: Option<AddressMode>,
    ) -> Result<(), ParseError> {
        if address.is_none()
            && gateway.is_none()
            && mtu.is_none()
            && enabled.is_none()
            && mode.is_none()
        {
            return Err(ParseError::InvalidValue);
        }
        let previous = *self;
        let name = BoundedText::new(name)?;
        let interface = self
            .interfaces
            .iter_mut()
            .flatten()
            .find(|interface| interface.name == name)
            .ok_or(ParseError::InvalidValue)?;
        if let Some(mode) = mode {
            interface.mode = mode;
            if mode == AddressMode::Dhcp && address.is_none() && interface.address.is_empty() {
                interface.address = BoundedText::new("0.0.0.0")?;
            }
        }
        if let Some(address) = address {
            interface.address = BoundedText::new(address)?;
            if mode.is_none() {
                interface.mode = AddressMode::Static;
            }
        }
        if let Some(gateway) = gateway {
            interface.gateway = Some(BoundedText::new(gateway)?);
        }
        if let Some(mtu) = mtu {
            interface.mtu = mtu;
        }
        if let Some(enabled) = enabled {
            interface.enabled = enabled;
        }
        let result = self.validate();
        if result.is_err() {
            *self = previous;
        }
        result
    }

    /// Apply a DHCP lease atomically. On validation failure the previous
    /// network snapshot is restored unchanged.
    pub fn apply_dhcp_lease(
        &mut self,
        name: &str,
        address: &str,
        gateway: Option<&str>,
    ) -> Result<(), ParseError> {
        let previous = *self;
        let name = BoundedText::new(name)?;
        let interface = self
            .interfaces
            .iter_mut()
            .flatten()
            .find(|interface| interface.name == name)
            .ok_or(ParseError::InvalidValue)?;
        if interface.mode != AddressMode::Dhcp {
            return Err(ParseError::InvalidValue);
        }
        interface.address = BoundedText::new(address)?;
        interface.gateway = match gateway {
            Some(value) => Some(BoundedText::new(value)?),
            None => None,
        };
        let result = self.validate();
        if result.is_err() {
            *self = previous;
        }
        result
    }

    /// Restore a preserved static configuration after DHCP failure or expiry.
    pub fn restore_static_interface(
        &mut self,
        name: &str,
        address: &str,
        gateway: Option<&str>,
    ) -> Result<(), ParseError> {
        self.update_interface(
            name,
            Some(address),
            gateway,
            None,
            None,
            Some(AddressMode::Static),
        )
    }

    pub fn upsert_route(
        &mut self,
        destination: &str,
        gateway: &str,
        interface: &str,
        metric: Option<u32>,
    ) -> Result<(), ParseError> {
        let previous = *self;
        let destination = BoundedText::new(destination)?;
        let gateway = BoundedText::new(gateway)?;
        let interface = BoundedText::new(interface)?;
        if !self
            .interfaces()
            .any(|entry| entry.name == interface)
        {
            return Err(ParseError::InvalidValue);
        }
        let route = NetworkRoute {
            destination,
            gateway,
            interface,
            metric: metric.unwrap_or(100),
        };
        if let Some(existing) = self
            .routes
            .iter_mut()
            .flatten()
            .find(|existing| existing.destination == route.destination)
        {
            *existing = route;
        } else {
            let count = self.routes.iter().filter(|entry| entry.is_some()).count();
            let slot = self
                .routes
                .get_mut(count)
                .ok_or(ParseError::Capacity)?;
            *slot = Some(route);
        }
        let result = self.validate();
        if result.is_err() {
            *self = previous;
        }
        result
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemSpec {
    schema: u16,
    revision: u64,
    services: [Option<ServiceSpec>; MAX_SERVICES],
    capabilities: [Option<CapabilityPolicy>; MAX_CAPABILITY_POLICIES],
    network: NetworkSpec,
    cluster: ClusterSpec,
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

    pub fn set_network_hostname(&mut self, hostname: &str) -> Result<(), ParseError> {
        let previous = self.network;
        let result = self.network.set_hostname(hostname);
        if result.is_ok() {
            self.revision = self.revision.saturating_add(1);
        } else {
            self.network = previous;
        }
        result
    }

    pub fn update_network_interface(
        &mut self,
        name: &str,
        address: Option<&str>,
        gateway: Option<&str>,
        mtu: Option<u32>,
        enabled: Option<bool>,
        mode: Option<AddressMode>,
    ) -> Result<(), ParseError> {
        let previous = self.network;
        let result = self
            .network
            .update_interface(name, address, gateway, mtu, enabled, mode);
        if result.is_ok() {
            self.revision = self.revision.saturating_add(1);
        } else {
            self.network = previous;
        }
        result
    }

    pub fn apply_network_dhcp_lease(
        &mut self,
        name: &str,
        address: &str,
        gateway: Option<&str>,
    ) -> Result<(), ParseError> {
        let previous = self.network;
        let result = self.network.apply_dhcp_lease(name, address, gateway);
        if result.is_ok() {
            self.revision = self.revision.saturating_add(1);
        } else {
            self.network = previous;
        }
        result
    }

    pub fn restore_network_static_interface(
        &mut self,
        name: &str,
        address: &str,
        gateway: Option<&str>,
    ) -> Result<(), ParseError> {
        let previous = self.network;
        let result = self
            .network
            .restore_static_interface(name, address, gateway);
        if result.is_ok() {
            self.revision = self.revision.saturating_add(1);
        } else {
            self.network = previous;
        }
        result
    }

    pub fn upsert_network_route(
        &mut self,
        destination: &str,
        gateway: &str,
        interface: &str,
        metric: Option<u32>,
    ) -> Result<(), ParseError> {
        let previous = self.network;
        let result = self
            .network
            .upsert_route(destination, gateway, interface, metric);
        if result.is_ok() {
            self.revision = self.revision.saturating_add(1);
        } else {
            self.network = previous;
        }
        result
    }

    pub const fn cluster(&self) -> &ClusterSpec {
        &self.cluster
    }

    pub fn validate(&self) -> Result<(), crate::ConfigurationValidationError> {
        self.cluster.validate()
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
            encoder.u8(interface.mode as u8);
        }
        for route in self.network.routes() {
            encoder.u8(5);
            encoder.text(route.destination.as_str());
            encoder.text(route.gateway.as_str());
            encoder.text(route.interface.as_str());
            encoder.u32(route.metric);
        }
        encoder.u8(6);
        encoder.push(self.cluster.digest().as_bytes());
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
    mode: Option<AddressMode>,
}

impl InterfacePartial {
    const fn new() -> Self {
        Self {
            name: None,
            address: None,
            gateway: None,
            mtu: None,
            enabled: None,
            mode: None,
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

#[derive(Clone, Copy)]
struct ClusterPartial {
    id: Option<u128>,
    name: Option<BoundedText<MAX_CLUSTER_NAME_BYTES>>,
    description: Option<BoundedText<MAX_CLUSTER_DESCRIPTION_BYTES>>,
    discovery: Option<DiscoveryPolicy>,
    membership: Option<MembershipPolicy>,
    admission: Option<AdmissionPolicy>,
    heartbeat_period_us: Option<u64>,
    missed_heartbeat_limit: Option<u16>,
}

impl ClusterPartial {
    const fn new() -> Self {
        Self {
            id: None,
            name: None,
            description: None,
            discovery: None,
            membership: None,
            admission: None,
            heartbeat_period_us: None,
            missed_heartbeat_limit: None,
        }
    }
}

#[derive(Clone, Copy)]
struct QuorumPartial {
    voting_members: Option<u16>,
    required_votes: Option<u16>,
    read_only_without_quorum: Option<bool>,
}

impl QuorumPartial {
    const fn new() -> Self {
        Self {
            voting_members: None,
            required_votes: None,
            read_only_without_quorum: None,
        }
    }
}

#[derive(Clone, Copy)]
struct SecurityPartial {
    require_signed_commits: Option<bool>,
    require_mutual_identity: Option<bool>,
    require_attestation: Option<bool>,
    encrypt_control_plane: Option<bool>,
    encrypt_data_plane: Option<bool>,
    trust_root: Option<u128>,
}

impl SecurityPartial {
    const fn new() -> Self {
        Self {
            require_signed_commits: None,
            require_mutual_identity: None,
            require_attestation: None,
            encrypt_control_plane: None,
            encrypt_data_plane: None,
            trust_root: None,
        }
    }
}

#[derive(Clone, Copy)]
struct ResourcePartial {
    cpu_limit: Option<u64>,
    memory_limit_bytes: Option<u64>,
    cxl_limit_bytes: Option<u64>,
    storage_limit_bytes: Option<u64>,
    network_limit_mbps: Option<u64>,
}

impl ResourcePartial {
    const fn new() -> Self {
        Self {
            cpu_limit: None,
            memory_limit_bytes: None,
            cxl_limit_bytes: None,
            storage_limit_bytes: None,
            network_limit_mbps: None,
        }
    }
}

#[derive(Clone, Copy)]
struct FederationPartial {
    enabled: Option<bool>,
    allow_remote_workloads: Option<bool>,
    require_attestation: Option<bool>,
    lease_ttl_us: Option<u64>,
    max_leases: Option<u32>,
}

impl FederationPartial {
    const fn new() -> Self {
        Self {
            enabled: None,
            allow_remote_workloads: None,
            require_attestation: None,
            lease_ttl_us: None,
            max_leases: None,
        }
    }
}

#[derive(Clone, Copy)]
struct ClusterTransportPartial {
    name: Option<BoundedText<MAX_TRANSPORT_NAME_BYTES>>,
    kind: Option<TransportKind>,
    endpoint: Option<BoundedText<MAX_ENDPOINT_BYTES>>,
    enabled: Option<bool>,
    priority: Option<u16>,
    mtu: Option<u32>,
}

impl ClusterTransportPartial {
    const fn new() -> Self {
        Self {
            name: None,
            kind: None,
            endpoint: None,
            enabled: None,
            priority: None,
            mtu: None,
        }
    }
}

#[derive(Clone, Copy)]
struct NodeOverridePartial {
    node: Option<u32>,
    discovery: Option<DiscoveryPolicy>,
    admission: Option<AdmissionPolicy>,
    heartbeat_period_us: Option<u64>,
    missed_heartbeat_limit: Option<u16>,
    transport: Option<TransportKind>,
}

impl NodeOverridePartial {
    const fn new() -> Self {
        Self {
            node: None,
            discovery: None,
            admission: None,
            heartbeat_period_us: None,
            missed_heartbeat_limit: None,
            transport: None,
        }
    }
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
    Cluster,
    ClusterQuorum,
    ClusterSecurity,
    ClusterResources,
    ClusterFederation,
    ClusterTransport,
    ClusterOverride,
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
    cluster: Option<ClusterSpec>,
    cluster_partial: Option<ClusterPartial>,
    quorum_partial: Option<QuorumPartial>,
    security_partial: Option<SecurityPartial>,
    resource_partial: Option<ResourcePartial>,
    federation_partial: Option<FederationPartial>,
    transport_partial: Option<ClusterTransportPartial>,
    override_partial: Option<NodeOverridePartial>,
    explicit_transport: bool,
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
            cluster: None,
            cluster_partial: None,
            quorum_partial: None,
            security_partial: None,
            resource_partial: None,
            federation_partial: None,
            transport_partial: None,
            override_partial: None,
            explicit_transport: false,
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
        let network = NetworkSpec {
            hostname: self.hostname,
            interfaces,
            routes,
        };
        network.validate()?;
        Ok(SystemSpec {
            schema,
            revision,
            services,
            capabilities,
            network,
            cluster: {
                let cluster = self.cluster.unwrap_or_else(ClusterSpec::safe_defaults);
                cluster.validate().map_err(map_validation_error)?;
                cluster
            },
        })
    }

    fn start_section(&mut self) {
        match self.section {
            Section::Service => self.service = Some(ServicePartial::new()),
            Section::Capability => self.capability = Some(CapabilityPartial::new()),
            Section::Interface => self.interface = Some(InterfacePartial::new()),
            Section::Route => self.route = Some(RoutePartial::new()),
            Section::Cluster => self.cluster_partial = Some(ClusterPartial::new()),
            Section::ClusterQuorum => self.quorum_partial = Some(QuorumPartial::new()),
            Section::ClusterSecurity => self.security_partial = Some(SecurityPartial::new()),
            Section::ClusterResources => self.resource_partial = Some(ResourcePartial::new()),
            Section::ClusterFederation => self.federation_partial = Some(FederationPartial::new()),
            Section::ClusterTransport => self.transport_partial = Some(ClusterTransportPartial::new()),
            Section::ClusterOverride => self.override_partial = Some(NodeOverridePartial::new()),
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
                let mode = partial.mode.unwrap_or(AddressMode::Static);
                let address = match (mode, partial.address) {
                    (_, Some(address)) => address,
                    (AddressMode::Dhcp, None) => BoundedText::new("0.0.0.0")?,
                    (AddressMode::Static, None) => return Err(ParseError::MissingField),
                };
                let interface = NetworkInterface {
                    name: partial.name.ok_or(ParseError::MissingField)?,
                    address,
                    gateway: partial.gateway,
                    mtu: partial.mtu.unwrap_or(1500),
                    enabled: partial.enabled.unwrap_or(true),
                    mode,
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
            Section::Cluster => {
                let partial = self.cluster_partial.take().ok_or(ParseError::MissingField)?;
                let mut cluster = self.cluster.take().unwrap_or_else(ClusterSpec::safe_defaults);
                if let Some(value) = partial.id { cluster.identity.id = value }
                if let Some(value) = partial.name { cluster.identity.name = value }
                if let Some(value) = partial.description { cluster.identity.description = value }
                if let Some(value) = partial.discovery { cluster.discovery = value }
                if let Some(value) = partial.membership { cluster.membership = value }
                if let Some(value) = partial.admission { cluster.admission = value }
                if let Some(value) = partial.heartbeat_period_us { cluster.heartbeat_period_us = value }
                if let Some(value) = partial.missed_heartbeat_limit { cluster.missed_heartbeat_limit = value }
                self.cluster = Some(cluster);
            }
            Section::ClusterQuorum => {
                let partial = self.quorum_partial.take().ok_or(ParseError::MissingField)?;
                let mut cluster = self.cluster.take().unwrap_or_else(ClusterSpec::safe_defaults);
                cluster.quorum = QuorumSpec {
                    voting_members: partial.voting_members.unwrap_or(cluster.quorum.voting_members),
                    required_votes: partial.required_votes.unwrap_or(cluster.quorum.required_votes),
                    read_only_without_quorum: partial.read_only_without_quorum.unwrap_or(cluster.quorum.read_only_without_quorum),
                };
                self.cluster = Some(cluster);
            }
            Section::ClusterSecurity => {
                let partial = self.security_partial.take().ok_or(ParseError::MissingField)?;
                let mut cluster = self.cluster.take().unwrap_or_else(ClusterSpec::safe_defaults);
                cluster.security = SecuritySpec {
                    require_signed_commits: partial.require_signed_commits.unwrap_or(cluster.security.require_signed_commits),
                    require_mutual_identity: partial.require_mutual_identity.unwrap_or(cluster.security.require_mutual_identity),
                    require_attestation: partial.require_attestation.unwrap_or(cluster.security.require_attestation),
                    encrypt_control_plane: partial.encrypt_control_plane.unwrap_or(cluster.security.encrypt_control_plane),
                    encrypt_data_plane: partial.encrypt_data_plane.unwrap_or(cluster.security.encrypt_data_plane),
                    trust_root: partial.trust_root.unwrap_or(cluster.security.trust_root),
                };
                self.cluster = Some(cluster);
            }
            Section::ClusterResources => {
                let partial = self.resource_partial.take().ok_or(ParseError::MissingField)?;
                let mut cluster = self.cluster.take().unwrap_or_else(ClusterSpec::safe_defaults);
                cluster.resources = ResourceSpec {
                    cpu_limit: partial.cpu_limit.unwrap_or(cluster.resources.cpu_limit),
                    memory_limit_bytes: partial.memory_limit_bytes.unwrap_or(cluster.resources.memory_limit_bytes),
                    cxl_limit_bytes: partial.cxl_limit_bytes.unwrap_or(cluster.resources.cxl_limit_bytes),
                    storage_limit_bytes: partial.storage_limit_bytes.unwrap_or(cluster.resources.storage_limit_bytes),
                    network_limit_mbps: partial.network_limit_mbps.unwrap_or(cluster.resources.network_limit_mbps),
                };
                self.cluster = Some(cluster);
            }
            Section::ClusterFederation => {
                let partial = self.federation_partial.take().ok_or(ParseError::MissingField)?;
                let mut cluster = self.cluster.take().unwrap_or_else(ClusterSpec::safe_defaults);
                cluster.federation = FederationSpec {
                    enabled: partial.enabled.unwrap_or(cluster.federation.enabled),
                    allow_remote_workloads: partial.allow_remote_workloads.unwrap_or(cluster.federation.allow_remote_workloads),
                    require_attestation: partial.require_attestation.unwrap_or(cluster.federation.require_attestation),
                    lease_ttl_us: partial.lease_ttl_us.unwrap_or(cluster.federation.lease_ttl_us),
                    max_leases: partial.max_leases.unwrap_or(cluster.federation.max_leases),
                };
                self.cluster = Some(cluster);
            }
            Section::ClusterTransport => {
                let partial = self.transport_partial.take().ok_or(ParseError::MissingField)?;
                let mut cluster = self.cluster.take().unwrap_or_else(ClusterSpec::safe_defaults);
                if !self.explicit_transport {
                    cluster.clear_transports();
                    self.explicit_transport = true;
                }
                cluster.push_transport(TransportSpec {
                    name: partial.name.ok_or(ParseError::MissingField)?,
                    kind: partial.kind.ok_or(ParseError::MissingField)?,
                    endpoint: partial.endpoint.unwrap_or(BoundedText::EMPTY),
                    enabled: partial.enabled.unwrap_or(true),
                    priority: partial.priority.unwrap_or(100),
                    mtu: partial.mtu.unwrap_or(1500),
                }).map_err(map_validation_error)?;
                self.cluster = Some(cluster);
            }
            Section::ClusterOverride => {
                let partial = self.override_partial.take().ok_or(ParseError::MissingField)?;
                let mut cluster = self.cluster.take().unwrap_or_else(ClusterSpec::safe_defaults);
                cluster.push_override(NodeOverrideSpec {
                    node: partial.node.ok_or(ParseError::MissingField)?,
                    discovery: partial.discovery,
                    admission: partial.admission,
                    heartbeat_period_us: partial.heartbeat_period_us,
                    missed_heartbeat_limit: partial.missed_heartbeat_limit,
                    transport: partial.transport,
                }).map_err(map_validation_error)?;
                self.cluster = Some(cluster);
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
            Section::Cluster => self.assign_cluster(key, value)?,
            Section::ClusterQuorum => self.assign_quorum(key, value)?,
            Section::ClusterSecurity => self.assign_security(key, value)?,
            Section::ClusterResources => self.assign_resources(key, value)?,
            Section::ClusterFederation => self.assign_federation(key, value)?,
            Section::ClusterTransport => self.assign_transport(key, value)?,
            Section::ClusterOverride => self.assign_override(key, value)?,
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
            "mode" => set_once(&mut partial.mode, AddressMode::parse(value)?),
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

    fn assign_cluster(&mut self, key: &str, value: &str) -> Result<(), ParseError> {
        let partial = self.cluster_partial.as_mut().ok_or(ParseError::MissingField)?;
        match key {
            "id" => set_once(&mut partial.id, parse_u128(value)?),
            "name" => set_once(&mut partial.name, parse_text(value)?),
            "description" => set_once(&mut partial.description, parse_text(value)?),
            "discovery" => set_once(&mut partial.discovery, parse_discovery(value)?),
            "membership" => set_once(&mut partial.membership, parse_membership(value)?),
            "admission" => set_once(&mut partial.admission, parse_admission(value)?),
            "heartbeat_period_us" | "heartbeat-period-us" => set_once(&mut partial.heartbeat_period_us, parse_u64(value)?),
            "missed_heartbeat_limit" | "missed-heartbeat-limit" => set_once(&mut partial.missed_heartbeat_limit, parse_u16(value)?),
            _ => Err(ParseError::UnknownKey),
        }
    }

    fn assign_quorum(&mut self, key: &str, value: &str) -> Result<(), ParseError> {
        let partial = self.quorum_partial.as_mut().ok_or(ParseError::MissingField)?;
        match key {
            "voting_members" | "voting-members" => set_once(&mut partial.voting_members, parse_u16(value)?),
            "required_votes" | "required-votes" => set_once(&mut partial.required_votes, parse_u16(value)?),
            "read_only_without_quorum" | "read-only-without-quorum" => set_once(&mut partial.read_only_without_quorum, parse_bool(value)?),
            _ => Err(ParseError::UnknownKey),
        }
    }

    fn assign_security(&mut self, key: &str, value: &str) -> Result<(), ParseError> {
        let partial = self.security_partial.as_mut().ok_or(ParseError::MissingField)?;
        match key {
            "require_signed_commits" | "require-signed-commits" => set_once(&mut partial.require_signed_commits, parse_bool(value)?),
            "require_mutual_identity" | "require-mutual-identity" => set_once(&mut partial.require_mutual_identity, parse_bool(value)?),
            "require_attestation" | "require-attestation" => set_once(&mut partial.require_attestation, parse_bool(value)?),
            "encrypt_control_plane" | "encrypt-control-plane" => set_once(&mut partial.encrypt_control_plane, parse_bool(value)?),
            "encrypt_data_plane" | "encrypt-data-plane" => set_once(&mut partial.encrypt_data_plane, parse_bool(value)?),
            "trust_root" | "trust-root" => set_once(&mut partial.trust_root, parse_u128(value)?),
            _ => Err(ParseError::UnknownKey),
        }
    }

    fn assign_resources(&mut self, key: &str, value: &str) -> Result<(), ParseError> {
        let partial = self.resource_partial.as_mut().ok_or(ParseError::MissingField)?;
        match key {
            "cpu_limit" | "cpu-limit" => set_once(&mut partial.cpu_limit, parse_u64(value)?),
            "memory_limit_bytes" | "memory-limit-bytes" => set_once(&mut partial.memory_limit_bytes, parse_u64(value)?),
            "cxl_limit_bytes" | "cxl-limit-bytes" => set_once(&mut partial.cxl_limit_bytes, parse_u64(value)?),
            "storage_limit_bytes" | "storage-limit-bytes" => set_once(&mut partial.storage_limit_bytes, parse_u64(value)?),
            "network_limit_mbps" | "network-limit-mbps" => set_once(&mut partial.network_limit_mbps, parse_u64(value)?),
            _ => Err(ParseError::UnknownKey),
        }
    }

    fn assign_federation(&mut self, key: &str, value: &str) -> Result<(), ParseError> {
        let partial = self.federation_partial.as_mut().ok_or(ParseError::MissingField)?;
        match key {
            "enabled" => set_once(&mut partial.enabled, parse_bool(value)?),
            "allow_remote_workloads" | "allow-remote-workloads" => set_once(&mut partial.allow_remote_workloads, parse_bool(value)?),
            "require_attestation" | "require-attestation" => set_once(&mut partial.require_attestation, parse_bool(value)?),
            "lease_ttl_us" | "lease-ttl-us" => set_once(&mut partial.lease_ttl_us, parse_u64(value)?),
            "max_leases" | "max-leases" => set_once(&mut partial.max_leases, parse_u32(value)?),
            _ => Err(ParseError::UnknownKey),
        }
    }

    fn assign_transport(&mut self, key: &str, value: &str) -> Result<(), ParseError> {
        let partial = self.transport_partial.as_mut().ok_or(ParseError::MissingField)?;
        match key {
            "name" => set_once(&mut partial.name, parse_text(value)?),
            "kind" => set_once(&mut partial.kind, parse_transport_kind(value)?),
            "endpoint" => set_once(&mut partial.endpoint, parse_text(value)?),
            "enabled" => set_once(&mut partial.enabled, parse_bool(value)?),
            "priority" => set_once(&mut partial.priority, parse_u16(value)?),
            "mtu" => set_once(&mut partial.mtu, parse_u32(value)?),
            _ => Err(ParseError::UnknownKey),
        }
    }

    fn assign_override(&mut self, key: &str, value: &str) -> Result<(), ParseError> {
        let partial = self.override_partial.as_mut().ok_or(ParseError::MissingField)?;
        match key {
            "node" => set_once(&mut partial.node, parse_u32(value)?),
            "discovery" => set_once(&mut partial.discovery, parse_discovery(value)?),
            "admission" => set_once(&mut partial.admission, parse_admission(value)?),
            "heartbeat_period_us" | "heartbeat-period-us" => set_once(&mut partial.heartbeat_period_us, parse_u64(value)?),
            "missed_heartbeat_limit" | "missed-heartbeat-limit" => set_once(&mut partial.missed_heartbeat_limit, parse_u16(value)?),
            "transport" => set_once(&mut partial.transport, parse_transport_kind(value)?),
            _ => Err(ParseError::UnknownKey),
        }
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
            "cluster.transports" | "cluster.transport" => Ok(Section::ClusterTransport),
            "cluster.node_overrides" | "cluster.node-override" | "cluster.node_override" => {
                Ok(Section::ClusterOverride)
            }
            _ => Err(ParseError::UnknownSection),
        };
    }
    if line.starts_with('[') && line.ends_with(']') {
        return match &line[1..line.len() - 1] {
            "system" => Ok(Section::System),
            "network" => Ok(Section::Network),
            "cluster" => Ok(Section::Cluster),
            "cluster.quorum" => Ok(Section::ClusterQuorum),
            "cluster.security" => Ok(Section::ClusterSecurity),
            "cluster.resources" => Ok(Section::ClusterResources),
            "cluster.federation" => Ok(Section::ClusterFederation),
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
    if let Some(value) = value.strip_prefix("0x") {
        u128::from_str_radix(value, 16).map_err(|_| ParseError::InvalidInteger)
    } else {
        value.parse().map_err(|_| ParseError::InvalidInteger)
    }
}

fn parse_integer(value: &str) -> Result<u64, ParseError> {
    if let Some(value) = value.strip_prefix("0x") {
        u64::from_str_radix(value, 16).map_err(|_| ParseError::InvalidInteger)
    } else {
        value.parse().map_err(|_| ParseError::InvalidInteger)
    }
}

fn parse_discovery(value: &str) -> Result<DiscoveryPolicy, ParseError> {
    match quoted_value(value) {
        "disabled" => Ok(DiscoveryPolicy::Disabled),
        "static" => Ok(DiscoveryPolicy::Static),
        "mesh" => Ok(DiscoveryPolicy::Mesh),
        "mdns" => Ok(DiscoveryPolicy::Mdns),
        "hybrid" => Ok(DiscoveryPolicy::Hybrid),
        _ => Err(ParseError::InvalidValue),
    }
}

fn parse_membership(value: &str) -> Result<MembershipPolicy, ParseError> {
    match quoted_value(value) {
        "static" => Ok(MembershipPolicy::Static),
        "automatic" => Ok(MembershipPolicy::Automatic),
        _ => Err(ParseError::InvalidValue),
    }
}

fn parse_admission(value: &str) -> Result<AdmissionPolicy, ParseError> {
    match quoted_value(value) {
        "open" => Ok(AdmissionPolicy::Open),
        "invitation" => Ok(AdmissionPolicy::Invitation),
        "attested" => Ok(AdmissionPolicy::Attested),
        _ => Err(ParseError::InvalidValue),
    }
}

fn parse_transport_kind(value: &str) -> Result<TransportKind, ParseError> {
    match quoted_value(value) {
        "loopback" => Ok(TransportKind::Loopback),
        "ethernet" => Ok(TransportKind::Ethernet),
        "cxl" => Ok(TransportKind::Cxl),
        "wireless" => Ok(TransportKind::Wireless),
        "tunnel" => Ok(TransportKind::Tunnel),
        _ => Err(ParseError::InvalidValue),
    }
}

fn map_validation_error(error: ConfigurationValidationError) -> ParseError {
    match error {
        ConfigurationValidationError::Capacity => ParseError::Capacity,
        ConfigurationValidationError::DuplicateNodeOverride
        | ConfigurationValidationError::DuplicateTransport => ParseError::DuplicateName,
        ConfigurationValidationError::InvalidFederation
        | ConfigurationValidationError::InvalidHeartbeat
        | ConfigurationValidationError::InvalidIdentity
        | ConfigurationValidationError::InvalidNodeOverride { .. }
        | ConfigurationValidationError::InvalidQuorum
        | ConfigurationValidationError::InvalidTransport { .. }
        | ConfigurationValidationError::MissingTrustRoot => ParseError::InvalidValue,
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
        "admin" => Ok(CapabilityRights::ADMIN.bits()),
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
