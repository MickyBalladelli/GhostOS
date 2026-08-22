use ghostos_auth::CapabilityKey;
use ghostos_fabric::NodeId;
use ghostos_status::{IntoStatus, Status};

pub const MAX_ENDPOINTS: usize = 4;
pub const MAX_TOPOLOGY_NODES: usize = 32;
pub const MAX_TOPOLOGY_LINKS: usize = 64;
pub const MAX_DISCOVERY_CACHE: usize = 16;
pub const MAX_ENDPOINT_ADDRESS_BYTES: usize = 64;
pub const MAX_ZONE_BYTES: usize = 24;
pub const ADVERTISEMENT_PAYLOAD_BYTES: usize = 56 + MAX_ENDPOINTS * Endpoint::WIRE_BYTES;
pub const ADVERTISEMENT_WIRE_BYTES: usize = ADVERTISEMENT_PAYLOAD_BYTES + 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Transport {
    Cxl = 1,
    Ethernet = 2,
    Wireless = 3,
    Cellular5g = 4,
    Loopback = 5,
    Tunnel = 6,
}

impl Transport {
    const fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            1 => Self::Cxl,
            2 => Self::Ethernet,
            3 => Self::Wireless,
            4 => Self::Cellular5g,
            5 => Self::Loopback,
            6 => Self::Tunnel,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RouteKind {
    Direct = 1,
    Nat = 2,
    Relay = 3,
    Offline = 4,
}

impl RouteKind {
    const fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            1 => Self::Direct,
            2 => Self::Nat,
            3 => Self::Relay,
            4 => Self::Offline,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Reachability {
    Unknown = 1,
    Reachable = 2,
    Unreachable = 3,
    Offline = 4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterCapabilities(u32);

impl ClusterCapabilities {
    pub const CONTROL_PLANE: Self = Self(1 << 0);
    pub const DATA_PLANE: Self = Self(1 << 1);
    pub const STORAGE: Self = Self(1 << 2);
    pub const MEMORY: Self = Self(1 << 3);
    pub const ACCELERATOR: Self = Self(1 << 4);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }

    pub const fn with(self, capability: Self) -> Self {
        Self(self.0 | capability.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterId([u8; 16]);

impl ClusterId {
    pub fn new(bytes: [u8; 16]) -> Option<Self> {
        if bytes.iter().all(|byte| *byte == 0) {
            None
        } else {
            Some(Self(bytes))
        }
    }

    pub const fn raw(self) -> [u8; 16] {
        self.0
    }

    pub const fn from_valid_raw(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EndpointAddress {
    bytes: [u8; MAX_ENDPOINT_ADDRESS_BYTES],
    len: u8,
}

impl EndpointAddress {
    pub fn new(value: &str) -> Result<Self, TopologyError> {
        if value.is_empty() {
            return Err(TopologyError::InvalidEndpoint);
        }
        if value.len() > MAX_ENDPOINT_ADDRESS_BYTES
            || value
                .bytes()
                .any(|byte| byte == 0 || byte.is_ascii_whitespace())
        {
            return Err(TopologyError::InvalidEndpoint);
        }
        let mut bytes = [0; MAX_ENDPOINT_ADDRESS_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            len: value.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Endpoint {
    pub transport: Transport,
    pub route: RouteKind,
    pub address: EndpointAddress,
    pub port: u16,
    pub priority: u8,
    pub mtu: u16,
}

impl Endpoint {
    pub const WIRE_BYTES: usize = 73;

    pub fn new(
        transport: Transport,
        route: RouteKind,
        address: &str,
        port: u16,
        priority: u8,
        mtu: u16,
    ) -> Result<Self, TopologyError> {
        if priority == 0 || mtu < 576 {
            return Err(TopologyError::InvalidEndpoint);
        }
        if transport != Transport::Loopback && port == 0 {
            return Err(TopologyError::InvalidEndpoint);
        }
        Ok(Self {
            transport,
            route,
            address: EndpointAddress::new(address)?,
            port,
            priority,
            mtu,
        })
    }

    fn encode_into(self, output: &mut [u8]) {
        output.fill(0);
        output[0] = self.transport as u8;
        output[1] = self.route as u8;
        output[2] = self.priority;
        output[4..6].copy_from_slice(&self.port.to_be_bytes());
        output[6..8].copy_from_slice(&self.mtu.to_be_bytes());
        output[8] = self.address.len;
        output[9..9 + self.address.len as usize]
            .copy_from_slice(&self.address.bytes[..self.address.len as usize]);
    }

    fn decode(input: &[u8]) -> Result<Self, TopologyError> {
        let transport = Transport::from_raw(input[0]).ok_or(TopologyError::CorruptAdvertisement)?;
        let route = RouteKind::from_raw(input[1]).ok_or(TopologyError::CorruptAdvertisement)?;
        let address_len = input[8] as usize;
        if address_len == 0 || address_len > MAX_ENDPOINT_ADDRESS_BYTES {
            return Err(TopologyError::CorruptAdvertisement);
        }
        let address = core::str::from_utf8(&input[9..9 + address_len])
            .map_err(|_| TopologyError::CorruptAdvertisement)?;
        Self::new(
            transport,
            route,
            address,
            u16::from_be_bytes([input[4], input[5]]),
            input[2],
            u16::from_be_bytes([input[6], input[7]]),
        )
        .map_err(|_| TopologyError::CorruptAdvertisement)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterAdvertisement {
    pub cluster: ClusterId,
    pub node: NodeId,
    pub sequence: u64,
    pub issued_at_us: u64,
    pub expires_at_us: u64,
    pub control_version: u16,
    pub data_version: u16,
    pub minimum_version: u16,
    pub capabilities: ClusterCapabilities,
    pub endpoints: [Option<Endpoint>; MAX_ENDPOINTS],
    pub signature: [u8; 32],
}

impl ClusterAdvertisement {
    pub fn issue(
        key: CapabilityKey,
        cluster: ClusterId,
        node: NodeId,
        sequence: u64,
        issued_at_us: u64,
        expires_at_us: u64,
        versions: (u16, u16, u16),
        capabilities: ClusterCapabilities,
        endpoints: &[Endpoint],
    ) -> Result<Self, TopologyError> {
        if sequence == 0
            || expires_at_us <= issued_at_us
            || versions.0 == 0
            || versions.1 == 0
            || versions.2 == 0
            || endpoints.is_empty()
            || endpoints.len() > MAX_ENDPOINTS
        {
            return Err(TopologyError::InvalidAdvertisement);
        }
        let mut stored = [None; MAX_ENDPOINTS];
        for (index, endpoint) in endpoints.iter().copied().enumerate() {
            if stored[..index]
                .iter()
                .flatten()
                .any(|existing| existing == &endpoint)
            {
                return Err(TopologyError::DuplicateEndpoint);
            }
            stored[index] = Some(endpoint);
        }
        let mut advertisement = Self {
            cluster,
            node,
            sequence,
            issued_at_us,
            expires_at_us,
            control_version: versions.0,
            data_version: versions.1,
            minimum_version: versions.2,
            capabilities,
            endpoints: stored,
            signature: [0; 32],
        };
        advertisement.signature = key
            .authenticate(&advertisement.payload())
            .map_err(|_| TopologyError::SigningFailed)?;
        Ok(advertisement)
    }

    pub fn verify(&self, key: CapabilityKey, now_us: u64) -> Result<(), TopologyError> {
        self.validate(now_us)?;
        key.verify_authenticator(&self.payload(), &self.signature)
            .map_err(|_| TopologyError::InvalidSignature)
    }

    pub fn encode(self) -> [u8; ADVERTISEMENT_WIRE_BYTES] {
        let mut output = [0; ADVERTISEMENT_WIRE_BYTES];
        output[..ADVERTISEMENT_PAYLOAD_BYTES].copy_from_slice(&self.payload());
        output[ADVERTISEMENT_PAYLOAD_BYTES..].copy_from_slice(&self.signature);
        output
    }

    pub fn decode(input: [u8; ADVERTISEMENT_WIRE_BYTES]) -> Result<Self, TopologyError> {
        let Ok(cluster_bytes) = input[..16].try_into() else {
            return Err(TopologyError::CorruptAdvertisement)
        };
        let cluster = ClusterId::new(cluster_bytes)
            .ok_or(TopologyError::CorruptAdvertisement)?;
        let Ok(node_bytes) = input[16..20].try_into() else {
            return Err(TopologyError::CorruptAdvertisement)
        };
        let node = NodeId::new(u32::from_be_bytes(node_bytes))
            .ok_or(TopologyError::CorruptAdvertisement)?;
        let endpoint_count = input[50] as usize;
        if endpoint_count == 0 || endpoint_count > MAX_ENDPOINTS {
            return Err(TopologyError::CorruptAdvertisement);
        }
        let mut endpoints = [None; MAX_ENDPOINTS];
        for index in 0..endpoint_count {
            let start = 56 + index * Endpoint::WIRE_BYTES;
            endpoints[index] = Some(Endpoint::decode(
                &input[start..start + Endpoint::WIRE_BYTES],
            )?);
        }
        let mut signature = [0; 32];
        signature.copy_from_slice(&input[ADVERTISEMENT_PAYLOAD_BYTES..]);
        let Ok(sequence_bytes) = input[20..28].try_into() else {
            return Err(TopologyError::CorruptAdvertisement)
        };
        let Ok(issued_at_bytes) = input[28..36].try_into() else {
            return Err(TopologyError::CorruptAdvertisement)
        };
        let Ok(expires_at_bytes) = input[36..44].try_into() else {
            return Err(TopologyError::CorruptAdvertisement)
        };
        let Ok(control_version_bytes) = input[44..46].try_into() else {
            return Err(TopologyError::CorruptAdvertisement)
        };
        let Ok(data_version_bytes) = input[46..48].try_into() else {
            return Err(TopologyError::CorruptAdvertisement)
        };
        let Ok(minimum_version_bytes) = input[48..50].try_into() else {
            return Err(TopologyError::CorruptAdvertisement)
        };
        let Ok(capabilities_bytes) = input[52..56].try_into() else {
            return Err(TopologyError::CorruptAdvertisement)
        };
        let advertisement = Self {
            cluster,
            node,
            sequence: u64::from_be_bytes(sequence_bytes),
            issued_at_us: u64::from_be_bytes(issued_at_bytes),
            expires_at_us: u64::from_be_bytes(expires_at_bytes),
            control_version: u16::from_be_bytes(control_version_bytes),
            data_version: u16::from_be_bytes(data_version_bytes),
            minimum_version: u16::from_be_bytes(minimum_version_bytes),
            capabilities: ClusterCapabilities::from_bits(u32::from_be_bytes(capabilities_bytes)),
            endpoints,
            signature,
        };
        advertisement
            .validate(advertisement.issued_at_us)
            .map_err(|_| TopologyError::CorruptAdvertisement)?;
        Ok(advertisement)
    }

    pub fn validate(&self, now_us: u64) -> Result<(), TopologyError> {
        if self.sequence == 0
            || self.expires_at_us <= self.issued_at_us
            || now_us >= self.expires_at_us
            || self.control_version == 0
            || self.data_version == 0
            || self.minimum_version == 0
            || self.endpoints.iter().flatten().next().is_none()
        {
            return Err(TopologyError::InvalidAdvertisement);
        }
        Ok(())
    }

    pub fn preferred_endpoint(&self, transport: Option<Transport>) -> Option<(usize, Endpoint)> {
        self.endpoints
            .iter()
            .enumerate()
            .filter_map(|(index, endpoint)| endpoint.map(|endpoint| (index, endpoint)))
            .filter(|(_, endpoint)| transport.map_or(true, |wanted| endpoint.transport == wanted))
            .min_by_key(|(index, endpoint)| (endpoint.priority, *index))
            .map(|(index, endpoint)| (index, endpoint))
    }

    fn payload(&self) -> [u8; ADVERTISEMENT_PAYLOAD_BYTES] {
        let mut output = [0; ADVERTISEMENT_PAYLOAD_BYTES];
        output[..16].copy_from_slice(&self.cluster.raw());
        output[16..20].copy_from_slice(&self.node.raw().to_be_bytes());
        output[20..28].copy_from_slice(&self.sequence.to_be_bytes());
        output[28..36].copy_from_slice(&self.issued_at_us.to_be_bytes());
        output[36..44].copy_from_slice(&self.expires_at_us.to_be_bytes());
        output[44..46].copy_from_slice(&self.control_version.to_be_bytes());
        output[46..48].copy_from_slice(&self.data_version.to_be_bytes());
        output[48..50].copy_from_slice(&self.minimum_version.to_be_bytes());
        output[50] = self.endpoints.iter().flatten().count() as u8;
        output[52..56].copy_from_slice(&self.capabilities.bits().to_be_bytes());
        for (index, endpoint) in self.endpoints.iter().flatten().enumerate() {
            endpoint.encode_into(
                &mut output[56 + index * Endpoint::WIRE_BYTES..][..Endpoint::WIRE_BYTES],
            );
        }
        output
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Zone {
    bytes: [u8; MAX_ZONE_BYTES],
    len: u8,
}

impl Zone {
    pub fn new(value: &str) -> Result<Self, TopologyError> {
        if value.is_empty()
            || value.len() > MAX_ZONE_BYTES
            || value
                .bytes()
                .any(|byte| byte == 0 || byte.is_ascii_whitespace())
        {
            return Err(TopologyError::InvalidZone);
        }
        let mut bytes = [0; MAX_ZONE_BYTES];
        bytes[..value.len()].copy_from_slice(value.as_bytes());
        Ok(Self {
            bytes,
            len: value.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TopologyNode {
    pub cluster: ClusterId,
    pub node: NodeId,
    pub zone: Zone,
    pub capabilities: ClusterCapabilities,
    pub last_seen_us: u64,
    pub reachability: Reachability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TopologyLink {
    pub from: NodeId,
    pub to: NodeId,
    pub transport: Transport,
    pub route: RouteKind,
    pub reachability: Reachability,
    pub latency_us: u64,
    pub bandwidth_mbps: u64,
    pub mtu: u16,
    pub observed_at_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopologyError {
    Capacity,
    CorruptAdvertisement,
    DuplicateEndpoint,
    DuplicateLink,
    InvalidAdvertisement,
    InvalidEndpoint,
    InvalidMtu,
    InvalidSignature,
    InvalidZone,
    NodeNotFound,
    Replay,
    SigningFailed,
    Backoff { retry_at_us: u64 },
    NoEndpoint,
}

impl IntoStatus for TopologyError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::CorruptAdvertisement | Self::InvalidSignature => Status::CORRUPT,
            Self::NodeNotFound | Self::NoEndpoint => Status::NOT_FOUND,
            Self::Replay => Status::PENDING,
            Self::Backoff { .. } => Status::BUSY,
            Self::DuplicateEndpoint | Self::DuplicateLink => Status::ALREADY_EXISTS,
            Self::InvalidAdvertisement
            | Self::InvalidEndpoint
            | Self::InvalidMtu
            | Self::InvalidZone => Status::INVALID_ARGUMENT,
            Self::SigningFailed => Status::ACCESS_DENIED,
        }
    }
}

pub struct TopologyGraph<
    const NODES: usize = MAX_TOPOLOGY_NODES,
    const LINKS: usize = MAX_TOPOLOGY_LINKS,
> {
    generation: u64,
    nodes: [Option<TopologyNode>; NODES],
    links: [Option<TopologyLink>; LINKS],
}

impl<const NODES: usize, const LINKS: usize> TopologyGraph<NODES, LINKS> {
    pub const fn new() -> Self {
        Self {
            generation: 0,
            nodes: [None; NODES],
            links: [None; LINKS],
        }
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn observe(
        &mut self,
        advertisement: ClusterAdvertisement,
        key: CapabilityKey,
        zone: Zone,
        now_us: u64,
    ) -> Result<(), TopologyError> {
        advertisement.verify(key, now_us)?;
        if let Some(node) = self
            .nodes
            .iter_mut()
            .flatten()
            .find(|node| node.node == advertisement.node)
        {
            if node.cluster != advertisement.cluster
                || node.last_seen_us >= advertisement.issued_at_us
            {
                return Err(TopologyError::Replay);
            }
            node.capabilities = advertisement.capabilities;
            node.last_seen_us = now_us;
            node.reachability = Reachability::Reachable;
            self.generation = self.generation.wrapping_add(1).max(1);
            return Ok(());
        }
        let slot = self
            .nodes
            .iter_mut()
            .find(|node| node.is_none())
            .ok_or(TopologyError::Capacity)?;
        *slot = Some(TopologyNode {
            cluster: advertisement.cluster,
            node: advertisement.node,
            zone,
            capabilities: advertisement.capabilities,
            last_seen_us: now_us,
            reachability: Reachability::Reachable,
        });
        self.generation = self.generation.wrapping_add(1).max(1);
        Ok(())
    }

    pub fn update_link(&mut self, link: TopologyLink) -> Result<(), TopologyError> {
        if link.from == link.to
            || link.mtu < 576
            || link.latency_us == 0
            || link.bandwidth_mbps == 0
        {
            return Err(TopologyError::InvalidMtu);
        }
        if !self.has_node(link.from) || !self.has_node(link.to) {
            return Err(TopologyError::NodeNotFound);
        }
        if let Some(existing) = self.links.iter_mut().flatten().find(|existing| {
            existing.from == link.from
                && existing.to == link.to
                && existing.transport == link.transport
        }) {
            *existing = link;
            self.generation = self.generation.wrapping_add(1).max(1);
            return Ok(());
        }
        let slot = self
            .links
            .iter_mut()
            .find(|entry| entry.is_none())
            .ok_or(TopologyError::Capacity)?;
        *slot = Some(link);
        self.generation = self.generation.wrapping_add(1).max(1);
        Ok(())
    }

    pub fn expire(&mut self, now_us: u64, timeout_us: u64) -> Result<usize, TopologyError> {
        if timeout_us == 0 {
            return Err(TopologyError::InvalidAdvertisement);
        }
        let mut expired = 0;
        for node in self.nodes.iter_mut().flatten() {
            if node.reachability == Reachability::Reachable
                && now_us.saturating_sub(node.last_seen_us) >= timeout_us
            {
                node.reachability = Reachability::Offline;
                expired += 1;
            }
        }
        if expired != 0 {
            self.generation = self.generation.wrapping_add(1).max(1)
        }
        Ok(expired)
    }

    pub fn node(&self, node: NodeId) -> Option<TopologyNode> {
        self.nodes
            .iter()
            .flatten()
            .find(|entry| entry.node == node)
            .copied()
    }
    pub fn nodes(&self) -> impl Iterator<Item = TopologyNode> + '_ {
        self.nodes.iter().flatten().copied()
    }
    pub fn links(&self) -> impl Iterator<Item = TopologyLink> + '_ {
        self.links.iter().flatten().copied()
    }
    pub fn link_count(&self) -> usize {
        self.links().count()
    }

    fn has_node(&self, node: NodeId) -> bool {
        self.node(node).is_some()
    }
}

impl<const NODES: usize, const LINKS: usize> Default for TopologyGraph<NODES, LINKS> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConnectivityStatus {
    Unknown,
    Connecting,
    Reachable,
    Backoff,
    Offline,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectionAttempt {
    pub node: NodeId,
    pub endpoint_index: usize,
    pub endpoint: Endpoint,
    pub attempt: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ConnectivityPeer {
    advertisement: ClusterAdvertisement,
    endpoint_index: usize,
    attempts: u8,
    retry_at_us: u64,
    negotiated_mtu: u16,
    status: ConnectivityStatus,
    cached_at_us: u64,
}

pub struct ConnectivityManager<const CAPACITY: usize = MAX_DISCOVERY_CACHE> {
    peers: [Option<ConnectivityPeer>; CAPACITY],
    base_backoff_us: u64,
    max_backoff_us: u64,
    max_attempts: u8,
}

impl<const CAPACITY: usize> ConnectivityManager<CAPACITY> {
    pub fn new(
        base_backoff_us: u64,
        max_backoff_us: u64,
        max_attempts: u8,
    ) -> Result<Self, TopologyError> {
        if base_backoff_us == 0 || max_backoff_us < base_backoff_us || max_attempts == 0 {
            return Err(TopologyError::InvalidAdvertisement);
        }
        Ok(Self {
            peers: [None; CAPACITY],
            base_backoff_us,
            max_backoff_us,
            max_attempts,
        })
    }

    pub fn observe(
        &mut self,
        advertisement: ClusterAdvertisement,
        key: CapabilityKey,
        now_us: u64,
    ) -> Result<(), TopologyError> {
        advertisement.verify(key, now_us)?;
        let preferred_index = advertisement
            .preferred_endpoint(None)
            .map(|(index, _)| index)
            .ok_or(TopologyError::NoEndpoint)?;
        if let Some(peer) = self
            .peers
            .iter_mut()
            .flatten()
            .find(|peer| peer.advertisement.node == advertisement.node)
        {
            if advertisement.sequence <= peer.advertisement.sequence {
                return Err(TopologyError::Replay);
            }
            peer.advertisement = advertisement;
            peer.endpoint_index = preferred_index;
            peer.attempts = 0;
            peer.retry_at_us = 0;
            peer.status = ConnectivityStatus::Unknown;
            peer.cached_at_us = now_us;
            return Ok(());
        }
        let slot = self
            .peers
            .iter_mut()
            .find(|peer| peer.is_none())
            .ok_or(TopologyError::Capacity)?;
        *slot = Some(ConnectivityPeer {
            advertisement,
            endpoint_index: preferred_index,
            attempts: 0,
            retry_at_us: 0,
            negotiated_mtu: 0,
            status: ConnectivityStatus::Unknown,
            cached_at_us: now_us,
        });
        Ok(())
    }

    pub fn connect(
        &mut self,
        node: NodeId,
        now_us: u64,
    ) -> Result<ConnectionAttempt, TopologyError> {
        let peer = self
            .peers
            .iter_mut()
            .flatten()
            .find(|peer| peer.advertisement.node == node)
            .ok_or(TopologyError::NodeNotFound)?;
        if now_us < peer.retry_at_us {
            return Err(TopologyError::Backoff {
                retry_at_us: peer.retry_at_us,
            });
        }
        let selected = (0..MAX_ENDPOINTS)
            .map(|offset| (peer.endpoint_index + offset) % MAX_ENDPOINTS)
            .find_map(|index| peer.advertisement.endpoints[index].map(|endpoint| (index, endpoint)))
            .ok_or(TopologyError::NoEndpoint)?;
        peer.endpoint_index = selected.0;
        peer.status = ConnectivityStatus::Connecting;
        Ok(ConnectionAttempt {
            node,
            endpoint_index: selected.0,
            endpoint: selected.1,
            attempt: peer.attempts.saturating_add(1),
        })
    }

    pub fn record_result(
        &mut self,
        node: NodeId,
        success: bool,
        now_us: u64,
        remote_mtu: u16,
    ) -> Result<(), TopologyError> {
        if success && remote_mtu < 576 {
            return Err(TopologyError::InvalidMtu);
        }
        let base_backoff_us = self.base_backoff_us;
        let max_backoff_us = self.max_backoff_us;
        let max_attempts = self.max_attempts;
        let peer = self
            .peers
            .iter_mut()
            .flatten()
            .find(|peer| peer.advertisement.node == node)
            .ok_or(TopologyError::NodeNotFound)?;
        let endpoint =
            peer.advertisement.endpoints[peer.endpoint_index].ok_or(TopologyError::NoEndpoint)?;
        if success {
            peer.status = ConnectivityStatus::Reachable;
            peer.attempts = 0;
            peer.retry_at_us = 0;
            peer.negotiated_mtu = endpoint.mtu.min(remote_mtu);
        } else {
            peer.attempts = peer.attempts.saturating_add(1);
            peer.endpoint_index = (peer.endpoint_index + 1) % MAX_ENDPOINTS;
            let shift = u32::from(peer.attempts.saturating_sub(1).min(15));
            let delay = base_backoff_us
                .saturating_mul(1u64 << shift)
                .min(max_backoff_us);
            peer.retry_at_us = now_us.saturating_add(delay);
            peer.status = if peer.attempts >= max_attempts {
                ConnectivityStatus::Offline
            } else {
                ConnectivityStatus::Backoff
            };
        }
        Ok(())
    }

    pub fn cached(
        &self,
        node: NodeId,
        now_us: u64,
        max_age_us: u64,
    ) -> Option<ClusterAdvertisement> {
        self.peers
            .iter()
            .flatten()
            .find(|peer| {
                peer.advertisement.node == node
                    && now_us.saturating_sub(peer.cached_at_us) <= max_age_us
            })
            .map(|peer| peer.advertisement)
    }

    pub fn status(&self, node: NodeId) -> Option<ConnectivityStatus> {
        self.peers
            .iter()
            .flatten()
            .find(|peer| peer.advertisement.node == node)
            .map(|peer| peer.status)
    }
    pub fn negotiated_mtu(&self, node: NodeId) -> Option<u16> {
        self.peers
            .iter()
            .flatten()
            .find(|peer| peer.advertisement.node == node)
            .map(|peer| peer.negotiated_mtu)
    }
}
