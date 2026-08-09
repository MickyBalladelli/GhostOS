use synos_fabric::NodeId;
use synos_status::{IntoStatus, Status};

pub const MAX_ANNOUNCEMENT_BYTES: usize = 48;
pub const MAX_MESH_INTERFACES: u8 = 0b111;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MeshInterface {
    Wireless = 1,
    Cellular5g = 2,
    LocalNetwork = 3,
}

impl MeshInterface {
    pub const fn bit(self) -> u8 {
        1 << (self as u8 - 1)
    }

    pub const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Wireless),
            2 => Some(Self::Cellular5g),
            3 => Some(Self::LocalNetwork),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct InterfaceSet(u8);

impl InterfaceSet {
    pub const NONE: Self = Self(0);
    pub const ALL: Self = Self(MAX_MESH_INTERFACES);

    pub const fn new(bits: u8) -> Option<Self> {
        if bits & !MAX_MESH_INTERFACES == 0 { Some(Self(bits)) } else { None }
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, interface: MeshInterface) -> bool {
        self.0 & interface.bit() != 0
    }

    pub const fn with(mut self, interface: MeshInterface) -> Self {
        self.0 |= interface.bit();
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum NodeRole {
    Edge = 1,
    Cluster = 2,
}

impl NodeRole {
    const fn from_raw(raw: u8) -> Option<Self> {
        match raw {
            1 => Some(Self::Edge),
            2 => Some(Self::Cluster),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeAdvertisement {
    pub node: NodeId,
    pub incarnation: u64,
    pub role: NodeRole,
    pub interfaces: InterfaceSet,
    pub cpu_capacity_millis: u32,
    pub memory_bytes: u64,
    pub bandwidth_mbps: u32,
    pub latency_us: u32,
    pub filesystem_generation: u64,
}

impl NodeAdvertisement {
    pub const WIRE_BYTES: usize = MAX_ANNOUNCEMENT_BYTES;

    pub const fn new(
        node: NodeId,
        incarnation: u64,
        role: NodeRole,
        interfaces: InterfaceSet,
        cpu_capacity_millis: u32,
        memory_bytes: u64,
        bandwidth_mbps: u32,
        latency_us: u32,
        filesystem_generation: u64,
    ) -> Option<Self> {
        if incarnation == 0 || interfaces.bits() == 0 || cpu_capacity_millis == 0 {
            return None;
        }
        Some(Self {
            node,
            incarnation,
            role,
            interfaces,
            cpu_capacity_millis,
            memory_bytes,
            bandwidth_mbps,
            latency_us,
            filesystem_generation,
        })
    }

    pub fn encode(self) -> [u8; Self::WIRE_BYTES] {
        let mut bytes = [0; Self::WIRE_BYTES];
        bytes[0..4].copy_from_slice(&self.node.raw().to_be_bytes());
        bytes[4..12].copy_from_slice(&self.incarnation.to_be_bytes());
        bytes[12] = self.role as u8;
        bytes[13] = self.interfaces.bits();
        bytes[16..20].copy_from_slice(&self.cpu_capacity_millis.to_be_bytes());
        bytes[20..28].copy_from_slice(&self.memory_bytes.to_be_bytes());
        bytes[28..32].copy_from_slice(&self.bandwidth_mbps.to_be_bytes());
        bytes[32..36].copy_from_slice(&self.latency_us.to_be_bytes());
        bytes[40..48].copy_from_slice(&self.filesystem_generation.to_be_bytes());
        bytes
    }

    pub fn decode(bytes: [u8; Self::WIRE_BYTES]) -> Result<Self, DiscoveryError> {
        let node = NodeId::new(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            .ok_or(DiscoveryError::CorruptPacket)?;
        let incarnation = u64::from_be_bytes(bytes[4..12].try_into().map_err(|_| DiscoveryError::CorruptPacket)?);
        let role = NodeRole::from_raw(bytes[12]).ok_or(DiscoveryError::CorruptPacket)?;
        let interfaces = InterfaceSet::new(bytes[13]).ok_or(DiscoveryError::CorruptPacket)?;
        let advertisement = Self::new(
            node,
            incarnation,
            role,
            interfaces,
            u32::from_be_bytes(bytes[16..20].try_into().map_err(|_| DiscoveryError::CorruptPacket)?),
            u64::from_be_bytes(bytes[20..28].try_into().map_err(|_| DiscoveryError::CorruptPacket)?),
            u32::from_be_bytes(bytes[28..32].try_into().map_err(|_| DiscoveryError::CorruptPacket)?),
            u32::from_be_bytes(bytes[32..36].try_into().map_err(|_| DiscoveryError::CorruptPacket)?),
            u64::from_be_bytes(bytes[40..48].try_into().map_err(|_| DiscoveryError::CorruptPacket)?),
        )
        .ok_or(DiscoveryError::InvalidAdvertisement)?;
        Ok(advertisement)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GossipAnnouncement {
    pub relay: NodeId,
    pub sequence: u32,
    pub interface: MeshInterface,
    pub advertisement: NodeAdvertisement,
}

impl GossipAnnouncement {
    pub const WIRE_BYTES: usize = 64;

    pub fn encode(self) -> [u8; Self::WIRE_BYTES] {
        let mut bytes = [0; Self::WIRE_BYTES];
        bytes[0..4].copy_from_slice(&self.relay.raw().to_be_bytes());
        bytes[4..8].copy_from_slice(&self.sequence.to_be_bytes());
        bytes[8] = self.interface as u8;
        bytes[16..].copy_from_slice(&self.advertisement.encode());
        bytes
    }

    pub fn decode(bytes: [u8; Self::WIRE_BYTES]) -> Result<Self, DiscoveryError> {
        let relay = NodeId::new(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
            .ok_or(DiscoveryError::CorruptPacket)?;
        let interface = MeshInterface::from_raw(bytes[8]).ok_or(DiscoveryError::CorruptPacket)?;
        Ok(Self {
            relay,
            sequence: u32::from_be_bytes(bytes[4..8].try_into().map_err(|_| DiscoveryError::CorruptPacket)?),
            interface,
            advertisement: NodeAdvertisement::decode(
                bytes[16..]
                    .try_into()
                    .map_err(|_| DiscoveryError::CorruptPacket)?,
            )?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryError {
    Capacity,
    CorruptPacket,
    InvalidAdvertisement,
    InvalidTimeout,
    SelfAnnouncement,
    StaleAnnouncement,
    UnknownNode,
}

impl IntoStatus for DiscoveryError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => crate::mesh_status(1, synos_status::Severity::Error),
            Self::InvalidTimeout | Self::InvalidAdvertisement | Self::SelfAnnouncement => {
                Status::INVALID_ARGUMENT
            }
            Self::CorruptPacket => Status::CORRUPT,
            Self::StaleAnnouncement => Status::PENDING,
            Self::UnknownNode => Status::NOT_FOUND,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerStatus {
    Alive,
    Expired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerState {
    pub advertisement: NodeAdvertisement,
    pub last_sequence: u32,
    pub last_seen_us: u64,
    pub status: PeerStatus,
}

pub const DEFAULT_DISCOVERY_CAPACITY: usize = 32;

pub struct GossipDiscovery<const CAPACITY: usize = DEFAULT_DISCOVERY_CAPACITY> {
    local: NodeAdvertisement,
    next_sequence: u32,
    peers: [Option<PeerState>; CAPACITY],
}

impl<const CAPACITY: usize> GossipDiscovery<CAPACITY> {
    pub const fn new(local: NodeAdvertisement) -> Self {
        Self {
            local,
            next_sequence: 1,
            peers: [None; CAPACITY],
        }
    }

    pub const fn local(&self) -> NodeAdvertisement {
        self.local
    }

    pub fn update_local(&mut self, local: NodeAdvertisement) -> Result<(), DiscoveryError> {
        if local.node != self.local.node || local.incarnation < self.local.incarnation {
            return Err(DiscoveryError::StaleAnnouncement);
        }
        self.local = local;
        Ok(())
    }

    pub fn due(&mut self, interface: MeshInterface) -> GossipAnnouncement {
        let announcement = GossipAnnouncement {
            relay: self.local.node,
            sequence: self.next_sequence,
            interface,
            advertisement: self.local,
        };
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        announcement
    }

    pub fn observe(
        &mut self,
        announcement: GossipAnnouncement,
        received_at_us: u64,
    ) -> Result<bool, DiscoveryError> {
        if announcement.advertisement.node == self.local.node {
            return Err(DiscoveryError::SelfAnnouncement);
        }
        if !announcement.advertisement.interfaces.contains(announcement.interface) {
            return Err(DiscoveryError::InvalidAdvertisement);
        }
        if let Some(peer) = self
            .peers
            .iter_mut()
            .flatten()
            .find(|peer| peer.advertisement.node == announcement.advertisement.node)
        {
            if announcement.advertisement.incarnation < peer.advertisement.incarnation
                || (announcement.advertisement.incarnation == peer.advertisement.incarnation
                    && announcement.sequence <= peer.last_sequence)
            {
                return Err(DiscoveryError::StaleAnnouncement);
            }
            peer.advertisement = announcement.advertisement;
            peer.last_sequence = announcement.sequence;
            peer.last_seen_us = received_at_us;
            peer.status = PeerStatus::Alive;
            return Ok(true);
        }
        let slot = self
            .peers
            .iter_mut()
            .find(|peer| peer.is_none())
            .ok_or(DiscoveryError::Capacity)?;
        *slot = Some(PeerState {
            advertisement: announcement.advertisement,
            last_sequence: announcement.sequence,
            last_seen_us: received_at_us,
            status: PeerStatus::Alive,
        });
        Ok(true)
    }

    pub fn expire(&mut self, now_us: u64, timeout_us: u64) -> Result<Option<NodeId>, DiscoveryError> {
        if timeout_us == 0 {
            return Err(DiscoveryError::InvalidTimeout);
        }
        let peer = self.peers.iter_mut().flatten().find(|peer| {
            peer.status == PeerStatus::Alive
                && now_us.saturating_sub(peer.last_seen_us) >= timeout_us
        });
        let Some(peer) = peer else { return Ok(None) };
        peer.status = PeerStatus::Expired;
        Ok(Some(peer.advertisement.node))
    }

    pub fn peer(&self, node: NodeId) -> Option<PeerState> {
        self.peers
            .iter()
            .flatten()
            .find(|peer| peer.advertisement.node == node)
            .copied()
    }

    pub fn len(&self) -> usize {
        self.peers.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn peers(&self) -> impl Iterator<Item = PeerState> + '_ {
        self.peers.iter().flatten().copied()
    }
}
