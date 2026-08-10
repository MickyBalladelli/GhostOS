use crate::capture::{CaptureDirection, CaptureKind, PacketCapture};

pub const POLICY_PATH: &str = "SYS$SYSTEM:FIREWALL.POLICY;1";
pub const MAX_FIREWALL_RULES: usize = 32;
pub const MAX_CONNECTIONS: usize = 64;
pub const MAX_RATE_BUCKETS: usize = 32;
pub const MAX_POLICY_IMAGE_BYTES: usize = 24 + MAX_FIREWALL_RULES * 40;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Ingress,
    Egress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    Tcp,
    Udp,
    Other(u8),
}

impl Protocol {
    const fn from_raw(raw: u8) -> Self {
        match raw {
            6 => Self::Tcp,
            17 => Self::Udp,
            value => Self::Other(value),
        }
    }

    const fn raw(self) -> u8 {
        match self {
            Self::Tcp => 6,
            Self::Udp => 17,
            Self::Other(value) => value,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FirewallError {
    InvalidFrame,
    InvalidPolicy,
    PolicyTooLarge,
    ExpiredCapability,
    InvalidCapability,
    SignatureRequired,
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ipv4Cidr {
    pub address: [u8; 4],
    pub prefix: u8,
}

impl Ipv4Cidr {
    pub const ANY: Self = Self {
        address: [0; 4],
        prefix: 0,
    };

    pub const fn new(address: [u8; 4], prefix: u8) -> Option<Self> {
        if prefix > 32 {
            None
        } else {
            Some(Self { address, prefix })
        }
    }

    pub const fn contains(self, address: [u8; 4]) -> bool {
        let full_bytes = (self.prefix / 8) as usize;
        let remaining = self.prefix % 8;
        let mut index = 0;
        while index < full_bytes {
            if self.address[index] != address[index] {
                return false;
            }
            index += 1;
        }
        if remaining == 0 {
            true
        } else {
            let mask = 0xff << (8 - remaining);
            self.address[full_bytes] & mask == address[full_bytes] & mask
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortRange {
    pub first: u16,
    pub last: u16,
}

impl PortRange {
    pub const ANY: Self = Self { first: 0, last: u16::MAX };

    pub const fn new(first: u16, last: u16) -> Option<Self> {
        if first > last { None } else { Some(Self { first, last }) }
    }

    pub const fn contains(self, port: u16) -> bool {
        port >= self.first && port <= self.last
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum RuleAction {
    Allow,
    Drop,
    Reject,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RateLimit {
    pub packets: u32,
    pub window_ms: u64,
}

impl RateLimit {
    pub const fn new(packets: u32, window_ms: u64) -> Option<Self> {
        if packets == 0 || window_ms == 0 { None } else { Some(Self { packets, window_ms }) }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FirewallRule {
    pub direction: Option<Direction>,
    pub protocol: Option<Protocol>,
    pub source: Option<Ipv4Cidr>,
    pub destination: Option<Ipv4Cidr>,
    pub source_ports: Option<PortRange>,
    pub destination_ports: Option<PortRange>,
    pub action: RuleAction,
    pub stateful: bool,
    pub capability: Option<CapabilityRight>,
    pub rate_limit: Option<RateLimit>,
}

impl FirewallRule {
    pub const fn allow() -> Self {
        Self {
            direction: None,
            protocol: None,
            source: None,
            destination: None,
            source_ports: None,
            destination_ports: None,
            action: RuleAction::Allow,
            stateful: false,
            capability: None,
            rate_limit: None,
        }
    }

    fn matches(self, packet: PacketView<'_>, direction: Direction) -> bool {
        self.direction.is_none_or(|value| value == direction)
            && self.protocol.is_none_or(|value| value == packet.protocol)
            && self.source.is_none_or(|value| value.contains(packet.source))
            && self.destination.is_none_or(|value| value.contains(packet.destination))
            && self.source_ports.is_none_or(|value| value.contains(packet.source_port))
            && self.destination_ports.is_none_or(|value| value.contains(packet.destination_port))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FirewallPolicy<const RULES: usize = MAX_FIREWALL_RULES> {
    rules: [Option<FirewallRule>; RULES],
    rule_count: usize,
    pub default_action: RuleAction,
    pub host_intranet: Option<Ipv4Cidr>,
    pub require_cluster_signatures: bool,
    pub version: u64,
}

impl<const RULES: usize> FirewallPolicy<RULES> {
    pub const fn new() -> Self {
        Self {
            rules: [None; RULES],
            rule_count: 0,
            default_action: RuleAction::Drop,
            host_intranet: None,
            require_cluster_signatures: false,
            version: 1,
        }
    }

    pub fn rules(&self) -> &[Option<FirewallRule>] {
        &self.rules[..self.rule_count]
    }

    pub const fn len(&self) -> usize {
        self.rule_count
    }

    pub fn add_rule(&mut self, rule: FirewallRule) -> Result<(), FirewallError> {
        let slot = self.rules.get_mut(self.rule_count).ok_or(FirewallError::Capacity)?;
        *slot = Some(rule);
        self.rule_count += 1;
        self.version = self.version.saturating_add(1);
        Ok(())
    }

    pub fn replace_rule(&mut self, index: usize, rule: FirewallRule) -> Result<(), FirewallError> {
        let slot = self.rules.get_mut(index).ok_or(FirewallError::InvalidPolicy)?;
        if slot.is_none() { return Err(FirewallError::InvalidPolicy) }
        *slot = Some(rule);
        self.version = self.version.saturating_add(1);
        Ok(())
    }

    pub fn encode(&self, output: &mut [u8]) -> Result<usize, FirewallError> {
        let required = 24usize.saturating_add(self.rule_count.saturating_mul(40));
        if output.len() < required { return Err(FirewallError::PolicyTooLarge) }
        output[..required].fill(0);
        output[0..4].copy_from_slice(b"SYFW");
        output[4] = 1;
        output[5] = self.default_action as u8;
        output[6] = self.require_cluster_signatures as u8;
        output[7] = self.rule_count as u8;
        output[8..16].copy_from_slice(&self.version.to_le_bytes());
        if let Some(cidr) = self.host_intranet {
            output[16] = 1;
            output[17..21].copy_from_slice(&cidr.address);
            output[21] = cidr.prefix;
        }
        for (index, rule) in self.rules[..self.rule_count].iter().flatten().enumerate() {
            encode_rule(rule, &mut output[24 + index * 40..64 + index * 40]);
        }
        Ok(required)
    }

    pub fn decode(input: &[u8]) -> Result<Self, FirewallError> {
        if input.len() < 24 || &input[..4] != b"SYFW" || input[4] != 1 {
            return Err(FirewallError::InvalidPolicy)
        }
        let count = input[7] as usize;
        if count > RULES || input.len() < 24 + count * 40 {
            return Err(FirewallError::InvalidPolicy)
        }
        let default_action = action(input[5]).ok_or(FirewallError::InvalidPolicy)?;
        let host_intranet = if input[16] == 0 {
            None
        } else {
            Some(Ipv4Cidr::new([input[17], input[18], input[19], input[20]], input[21])
                .ok_or(FirewallError::InvalidPolicy)?)
        };
        let mut policy = Self::new();
        policy.default_action = default_action;
        policy.require_cluster_signatures = input[6] != 0;
        let Ok(version) = input[8..16].try_into() else {
            return Err(FirewallError::InvalidPolicy)
        };
        policy.version = u64::from_le_bytes(version);
        policy.host_intranet = host_intranet;
        for index in 0..count {
            policy.add_rule(decode_rule(&input[24 + index * 40..64 + index * 40])?)?;
        }
        let Ok(version) = input[8..16].try_into() else {
            return Err(FirewallError::InvalidPolicy)
        };
        policy.version = u64::from_le_bytes(version);
        Ok(policy)
    }
}

/// Append-only policy image ledger. Each entry represents one immutable
/// SynFS version; publishing never overwrites an older image.
pub struct PolicyStore<const VERSIONS: usize = 8, const IMAGE_BYTES: usize = MAX_POLICY_IMAGE_BYTES> {
    images: [Option<PolicyImage<IMAGE_BYTES>>; VERSIONS],
    count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyImage<const IMAGE_BYTES: usize> {
    pub version: u64,
    bytes: [u8; IMAGE_BYTES],
    length: usize,
}

impl<const VERSIONS: usize, const IMAGE_BYTES: usize> PolicyStore<VERSIONS, IMAGE_BYTES> {
    pub const fn new() -> Self {
        Self { images: [None; VERSIONS], count: 0 }
    }

    pub fn publish<const RULES: usize>(
        &mut self,
        policy: &FirewallPolicy<RULES>,
    ) -> Result<u64, FirewallError> {
        if self.count == VERSIONS || self.current().is_some_and(|version| policy.version <= version) {
            return Err(FirewallError::Capacity)
        }
        let mut bytes = [0; IMAGE_BYTES];
        let length = policy.encode(&mut bytes)?;
        self.images[self.count] = Some(PolicyImage { version: policy.version, bytes, length });
        self.count += 1;
        Ok(policy.version)
    }

    pub const fn current(&self) -> Option<u64> {
        if self.count == 0 {
            None
        } else {
            match self.images[self.count - 1] {
                Some(image) => Some(image.version),
                None => None,
            }
        }
    }

    pub fn image(&self, version: u64) -> Option<&[u8]> {
        self.images[..self.count]
            .iter()
            .flatten()
            .find(|image| image.version == version)
            .map(|image| &image.bytes[..image.length])
    }
}

impl<const VERSIONS: usize, const IMAGE_BYTES: usize> Default
    for PolicyStore<VERSIONS, IMAGE_BYTES>
{
    fn default() -> Self { Self::new() }
}

impl<const RULES: usize> Default for FirewallPolicy<RULES> {
    fn default() -> Self { Self::new() }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FirewallSnapshot {
    pub policy_version: u64,
    pub rule_count: usize,
    pub active_connections: usize,
    pub dropped_packets: u64,
    pub allowed_packets: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacketView<'a> {
    pub frame: &'a [u8],
    pub source: [u8; 4],
    pub destination: [u8; 4],
    pub source_port: u16,
    pub destination_port: u16,
    pub protocol: Protocol,
    pub tcp_flags: u8,
    pub payload: &'a [u8],
}

impl<'a> PacketView<'a> {
    pub fn parse(frame: &'a [u8]) -> Result<Self, FirewallError> {
        if frame.len() < 14 || u16::from_be_bytes([frame[12], frame[13]]) != 0x0800 {
            return Err(FirewallError::InvalidFrame)
        }
        let ip = &frame[14..];
        if ip.len() < 20 || ip[0] >> 4 != 4 {
            return Err(FirewallError::InvalidFrame)
        }
        let header_length = (ip[0] as usize & 0x0f) * 4;
        let total_length = u16::from_be_bytes([ip[2], ip[3]]) as usize;
        if header_length < 20 || total_length < header_length || total_length > ip.len() {
            return Err(FirewallError::InvalidFrame)
        }
        let transport = &ip[header_length..total_length];
        let protocol = Protocol::from_raw(ip[9]);
        let transport_header = match protocol {
            Protocol::Tcp => 20,
            Protocol::Udp => 8,
            Protocol::Other(_) => 0,
        };
        if transport.len() < transport_header {
            return Err(FirewallError::InvalidFrame)
        }
        let source_port = if transport_header == 0 { 0 } else {
            u16::from_be_bytes([transport[0], transport[1]])
        };
        let destination_port = if transport_header == 0 { 0 } else {
            u16::from_be_bytes([transport[2], transport[3]])
        };
        let tcp_flags = if matches!(protocol, Protocol::Tcp) { transport[13] } else { 0 };
        let payload_start = if matches!(protocol, Protocol::Tcp) {
            (transport[12] as usize >> 4) * 4
        } else {
            transport_header
        };
        if payload_start < transport_header || payload_start > transport.len() {
            return Err(FirewallError::InvalidFrame)
        }
        Ok(Self {
            frame,
            source: [ip[12], ip[13], ip[14], ip[15]],
            destination: [ip[16], ip[17], ip[18], ip[19]],
            source_port,
            destination_port,
            protocol,
            tcp_flags,
            payload: &transport[payload_start..],
        })
    }

    fn key(self) -> ConnectionKey {
        ConnectionKey {
            source: self.source,
            destination: self.destination,
            source_port: self.source_port,
            destination_port: self.destination_port,
            protocol: self.protocol,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FirewallDecision {
    Allow,
    Drop,
    Reject,
    RateLimited,
    Invalid,
    AccessDenied,
    SignatureRequired,
}

#[derive(Clone, Copy)]
pub struct PacketContext<'a> {
    pub principal: u64,
    pub direction: Direction,
    pub now_ms: u64,
    pub leased_workload: bool,
    pub capability: Option<NetworkCapability>,
    pub signature: Option<PacketSignature>,
    pub signer: Option<&'a CapabilityKey>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityKey([u8; 32]);

impl CapabilityKey {
    pub const fn new(bytes: [u8; 32]) -> Self { Self(bytes) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum CapabilityRight {
    Listen = 1,
    Connect = 2,
    Raw = 4,
    Ingress = 8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetworkCapability {
    pub principal: u64,
    pub rights: u8,
    pub local_ports: PortRange,
    pub destination: Ipv4Cidr,
    pub expires_at_ms: u64,
    pub nonce: u64,
    tag: PacketSignature,
}

impl NetworkCapability {
    pub fn issue(
        key: &CapabilityKey,
        principal: u64,
        rights: u8,
        local_ports: PortRange,
        destination: Ipv4Cidr,
        expires_at_ms: u64,
        nonce: u64,
    ) -> Result<Self, FirewallError> {
        if principal == 0 || rights == 0 || expires_at_ms == 0 || nonce == 0 {
            return Err(FirewallError::InvalidCapability)
        }
        let mut token = Self {
            principal,
            rights,
            local_ports,
            destination,
            expires_at_ms,
            nonce,
            tag: PacketSignature([0; 32]),
        };
        token.tag = mac(key, &token.bytes());
        Ok(token)
    }

    pub fn verify(
        &self,
        key: &CapabilityKey,
        principal: u64,
        right: CapabilityRight,
        destination: [u8; 4],
        port: u16,
        now_ms: u64,
    ) -> Result<(), FirewallError> {
        if self.principal != principal
            || self.expires_at_ms <= now_ms
            || self.rights & right as u8 == 0
            || !self.destination.contains(destination)
            || !self.local_ports.contains(port)
            || self.tag != mac(key, &self.bytes())
        {
            return Err(if self.expires_at_ms <= now_ms {
                FirewallError::ExpiredCapability
            } else {
                FirewallError::InvalidCapability
            })
        }
        Ok(())
    }

    fn bytes(&self) -> [u8; 40] {
        let mut bytes = [0; 40];
        bytes[0..8].copy_from_slice(&self.principal.to_le_bytes());
        bytes[8] = self.rights;
        bytes[9..11].copy_from_slice(&self.local_ports.first.to_le_bytes());
        bytes[11..13].copy_from_slice(&self.local_ports.last.to_le_bytes());
        bytes[13..17].copy_from_slice(&self.destination.address);
        bytes[17] = self.destination.prefix;
        bytes[24..32].copy_from_slice(&self.expires_at_ms.to_le_bytes());
        bytes[32..40].copy_from_slice(&self.nonce.to_le_bytes());
        bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PacketSignature([u8; 32]);

impl PacketSignature {
    pub const fn new(bytes: [u8; 32]) -> Self { Self(bytes) }
    pub const fn bytes(self) -> [u8; 32] { self.0 }
}

pub type SignedHeader = PacketSignature;

#[derive(Clone, Copy)]
struct ConnectionKey {
    source: [u8; 4],
    destination: [u8; 4],
    source_port: u16,
    destination_port: u16,
    protocol: Protocol,
}

#[derive(Clone, Copy)]
struct ConnectionState {
    key: ConnectionKey,
    last_seen_ms: u64,
    established: bool,
}

#[derive(Clone, Copy)]
struct RateBucket {
    source: [u8; 4],
    rule: usize,
    window_start_ms: u64,
    packets: u32,
}

pub struct Firewall<
    const RULES: usize = MAX_FIREWALL_RULES,
    const CONNECTIONS: usize = MAX_CONNECTIONS,
    const BUCKETS: usize = MAX_RATE_BUCKETS,
> {
    policy: FirewallPolicy<RULES>,
    capability_key: Option<CapabilityKey>,
    connections: [Option<ConnectionState>; CONNECTIONS],
    buckets: [Option<RateBucket>; BUCKETS],
    dropped_packets: u64,
    allowed_packets: u64,
}

impl<const RULES: usize, const CONNECTIONS: usize, const BUCKETS: usize>
    Firewall<RULES, CONNECTIONS, BUCKETS>
{
    pub const fn new(policy: FirewallPolicy<RULES>) -> Self {
        Self {
            policy,
            capability_key: None,
            connections: [None; CONNECTIONS],
            buckets: [None; BUCKETS],
            dropped_packets: 0,
            allowed_packets: 0,
        }
    }

    pub const fn policy(&self) -> &FirewallPolicy<RULES> { &self.policy }
    pub const fn policy_mut(&mut self) -> &mut FirewallPolicy<RULES> { &mut self.policy }

    pub fn set_capability_key(&mut self, key: CapabilityKey) { self.capability_key = Some(key) }

    pub fn inspect(&mut self, frame: &[u8], context: PacketContext<'_>) -> FirewallDecision {
        let packet = match PacketView::parse(frame) {
            Ok(packet) => packet,
            Err(_) => return self.record(FirewallDecision::Invalid),
        };
        if context.leased_workload && self.policy.host_intranet.is_some_and(|network| {
            network.contains(packet.destination) || network.contains(packet.source)
        }) {
            return self.record(FirewallDecision::AccessDenied)
        }
        if self.policy.require_cluster_signatures && context.leased_workload {
            let Some(signature) = context.signature else {
                return self.record(FirewallDecision::AccessDenied)
            };
            let Some(key) = context.signer else {
                return self.record(FirewallDecision::SignatureRequired)
            };
            if signature != sign_header(key, packet) {
                return self.record(FirewallDecision::AccessDenied)
            }
        }
        let existing = self.find_connection(packet.key(), context.now_ms);
        let mut decision = self.policy.default_action;
        let mut stateful = false;
        for (index, rule) in self.policy.rules().iter().flatten().copied().enumerate() {
            if !rule.matches(packet, context.direction) { continue }
            if let Some(right) = rule.capability {
                let valid = self.capability_key.zip(context.capability).is_some_and(|(key, token)| {
                    token.verify(&key, context.principal, right, packet.destination, packet.destination_port, context.now_ms).is_ok()
                });
                if !valid { return self.record(FirewallDecision::AccessDenied) }
            }
            if let Some(limit) = rule.rate_limit {
                if !self.take_rate(index, packet.source, limit, context.now_ms) {
                    return self.record(FirewallDecision::RateLimited)
                }
            }
            decision = rule.action;
            stateful = rule.stateful;
            break
        }
        if existing.is_some_and(|state| state.established) && decision == RuleAction::Drop {
            decision = RuleAction::Allow;
            stateful = true;
        }
        if decision == RuleAction::Allow && stateful {
            self.remember(packet, context.now_ms)
        }
        self.record(match decision {
            RuleAction::Allow => FirewallDecision::Allow,
            RuleAction::Drop => FirewallDecision::Drop,
            RuleAction::Reject => FirewallDecision::Reject,
        })
    }

    /// Inspect a frame and retain both the bounded packet bytes and the
    /// resulting policy decision in deterministic evidence order.
    pub fn inspect_with_capture<const CAPACITY: usize>(
        &mut self,
        frame: &[u8],
        context: PacketContext<'_>,
        capture: &mut PacketCapture<CAPACITY>,
    ) -> FirewallDecision {
        let decision = self.inspect(frame, context);
        let packet = PacketView::parse(frame);
        let (source_ip, destination_ip, source_port, destination_port) = packet
            .map(|packet| {
                (
                    packet.source,
                    packet.destination,
                    packet.source_port,
                    packet.destination_port,
                )
            })
            .unwrap_or(([0; 4], [0; 4], 0, 0));
        let source_mac = frame.get(6..12).and_then(|bytes| bytes.try_into().ok()).unwrap_or([0; 6]);
        let destination_mac = frame.get(..6).and_then(|bytes| bytes.try_into().ok()).unwrap_or([0; 6]);
        capture.record_packet(
            context.now_ms,
            match context.direction {
                Direction::Ingress => CaptureDirection::Ingress,
                Direction::Egress => CaptureDirection::Egress,
            },
            source_mac,
            destination_mac,
            source_ip,
            destination_ip,
            source_port,
            destination_port,
            frame,
        );
        capture.record_event(
            context.now_ms,
            CaptureKind::FirewallDecision,
            firewall_decision_code(decision),
        );
        decision
    }

    pub fn authorize_endpoint(
        &self,
        principal: u64,
        right: CapabilityRight,
        destination: [u8; 4],
        port: u16,
        now_ms: u64,
        capability: Option<NetworkCapability>,
    ) -> Result<(), FirewallError> {
        let requires = self.policy.rules().iter().flatten().any(|rule| {
            rule.capability == Some(right) && endpoint_matches(*rule, right, destination, port)
        });
        if !requires { return Ok(()) }
        let key = self.capability_key.ok_or(FirewallError::InvalidCapability)?;
        capability.ok_or(FirewallError::InvalidCapability)?.verify(
            &key, principal, right, destination, port, now_ms,
        )
    }

    /// Existing socket handles are unforgeable, owner-bound network grants.
    /// This path lets `synos-netd` carry that proof without copying a large
    /// cryptographic token through the socket wire envelope.
    pub fn authorize_socket_endpoint(
        &self,
        right: CapabilityRight,
        destination: [u8; 4],
        port: u16,
        socket_granted: bool,
    ) -> Result<(), FirewallError> {
        let requires = self.policy.rules().iter().flatten().any(|rule| {
            rule.capability == Some(right) && endpoint_matches(*rule, right, destination, port)
        });
        if requires && !socket_granted {
            Err(FirewallError::InvalidCapability)
        } else {
            Ok(())
        }
    }

    pub fn snapshot(&self) -> FirewallSnapshot {
        FirewallSnapshot {
            policy_version: self.policy.version,
            rule_count: self.policy.len(),
            active_connections: self.connections.iter().flatten().count(),
            dropped_packets: self.dropped_packets,
            allowed_packets: self.allowed_packets,
        }
    }

    fn record(&mut self, decision: FirewallDecision) -> FirewallDecision {
        match decision {
            FirewallDecision::Allow => self.allowed_packets = self.allowed_packets.saturating_add(1),
            _ => self.dropped_packets = self.dropped_packets.saturating_add(1),
        }
        decision
    }

    fn find_connection(&mut self, key: ConnectionKey, now_ms: u64) -> Option<ConnectionState> {
        let mut found = None;
        for entry in &mut self.connections {
            if entry.is_some_and(|state| now_ms.saturating_sub(state.last_seen_ms) > 120_000) {
                *entry = None;
            } else if entry.is_some_and(|state| same_key(state.key, key) || same_key(state.key, reverse(key))) {
                found = *entry;
            }
        }
        found
    }

    fn remember(&mut self, packet: PacketView<'_>, now_ms: u64) {
        let key = packet.key();
        if let Some(entry) = self.connections.iter_mut().flatten().find(|state| {
            same_key(state.key, key) || same_key(state.key, reverse(key))
        }) {
            entry.last_seen_ms = now_ms;
            entry.established |= packet.tcp_flags & 0x02 == 0;
            return
        }
        let slot_index = self.connections.iter().position(Option::is_none).unwrap_or(0);
        if let Some(entry) = self.connections.get_mut(slot_index) {
            *entry = Some(ConnectionState {
                key,
                last_seen_ms: now_ms,
                established: packet.tcp_flags & 0x02 == 0,
            })
        }
    }

    fn take_rate(&mut self, rule: usize, source: [u8; 4], limit: RateLimit, now_ms: u64) -> bool {
        let bucket_index = self.buckets.iter().position(|entry| {
            entry.is_some_and(|bucket| bucket.rule == rule && bucket.source == source)
        }).or_else(|| self.buckets.iter().position(Option::is_none));
        let Some(bucket_index) = bucket_index else { return false };
        let Some(entry) = self.buckets.get_mut(bucket_index) else { return false };
        let current = entry.get_or_insert(RateBucket {
            source,
            rule,
            window_start_ms: now_ms,
            packets: 0,
        });
        if now_ms.saturating_sub(current.window_start_ms) >= limit.window_ms {
            current.window_start_ms = now_ms;
            current.packets = 0;
        }
        if current.packets >= limit.packets { false } else {
            current.packets += 1;
            true
        }
    }
}

fn firewall_decision_code(decision: FirewallDecision) -> u16 {
    match decision {
        FirewallDecision::Allow => 0,
        FirewallDecision::Drop => 1,
        FirewallDecision::Reject => 2,
        FirewallDecision::RateLimited => 3,
        FirewallDecision::Invalid => 4,
        FirewallDecision::AccessDenied => 5,
        FirewallDecision::SignatureRequired => 6,
    }
}

impl<const RULES: usize, const CONNECTIONS: usize, const BUCKETS: usize> Default
    for Firewall<RULES, CONNECTIONS, BUCKETS>
{
    fn default() -> Self { Self::new(FirewallPolicy::new()) }
}

fn same_key(left: ConnectionKey, right: ConnectionKey) -> bool {
    left.source == right.source
        && left.destination == right.destination
        && left.source_port == right.source_port
        && left.destination_port == right.destination_port
        && left.protocol == right.protocol
}

fn endpoint_matches(
    rule: FirewallRule,
    right: CapabilityRight,
    destination: [u8; 4],
    port: u16,
) -> bool {
    let direction = match right {
        CapabilityRight::Listen | CapabilityRight::Ingress => Direction::Ingress,
        CapabilityRight::Connect | CapabilityRight::Raw => Direction::Egress,
    };
    rule.direction.is_none_or(|value| value == direction)
        && rule.destination.is_none_or(|value| value.contains(destination))
        && rule.destination_ports.is_none_or(|value| value.contains(port))
}

fn reverse(key: ConnectionKey) -> ConnectionKey {
    ConnectionKey {
        source: key.destination,
        destination: key.source,
        source_port: key.destination_port,
        destination_port: key.source_port,
        protocol: key.protocol,
    }
}

fn encode_rule(rule: &FirewallRule, output: &mut [u8]) {
    output[0] = rule.direction.map_or(0, |value| match value { Direction::Ingress => 1, Direction::Egress => 2 });
    output[1] = rule.protocol.map_or(0, |value| value.raw());
    output[2] = rule.action as u8;
    output[3] = rule.stateful as u8;
    output[4] = rule.capability.map_or(0, |value| value as u8);
    if let Some(cidr) = rule.source { output[5] = 1; output[6..10].copy_from_slice(&cidr.address); output[10] = cidr.prefix }
    if let Some(cidr) = rule.destination { output[11] = 1; output[12..16].copy_from_slice(&cidr.address); output[16] = cidr.prefix }
    if let Some(range) = rule.source_ports { output[17] = 1; output[18..20].copy_from_slice(&range.first.to_le_bytes()); output[20..22].copy_from_slice(&range.last.to_le_bytes()) }
    if let Some(range) = rule.destination_ports { output[22] = 1; output[23..25].copy_from_slice(&range.first.to_le_bytes()); output[25..27].copy_from_slice(&range.last.to_le_bytes()) }
    if let Some(limit) = rule.rate_limit { output[27] = 1; output[28..32].copy_from_slice(&limit.packets.to_le_bytes()); output[32..40].copy_from_slice(&limit.window_ms.to_le_bytes()) }
}

fn decode_rule(input: &[u8]) -> Result<FirewallRule, FirewallError> {
    let direction = match input[0] { 0 => None, 1 => Some(Direction::Ingress), 2 => Some(Direction::Egress), _ => return Err(FirewallError::InvalidPolicy) };
    let protocol = if input[1] == 0 { None } else { Some(Protocol::from_raw(input[1])) };
    let source = if input[5] == 0 { None } else { Some(Ipv4Cidr::new([input[6], input[7], input[8], input[9]], input[10]).ok_or(FirewallError::InvalidPolicy)?) };
    let destination = if input[11] == 0 { None } else { Some(Ipv4Cidr::new([input[12], input[13], input[14], input[15]], input[16]).ok_or(FirewallError::InvalidPolicy)?) };
    let source_ports = if input[17] == 0 { None } else { Some(PortRange::new(u16::from_le_bytes([input[18], input[19]]), u16::from_le_bytes([input[20], input[21]])).ok_or(FirewallError::InvalidPolicy)?) };
    let destination_ports = if input[22] == 0 { None } else { Some(PortRange::new(u16::from_le_bytes([input[23], input[24]]), u16::from_le_bytes([input[25], input[26]])).ok_or(FirewallError::InvalidPolicy)?) };
    let rate_limit = if input[27] == 0 {
        None
    } else {
        let Ok(window_ms) = input[32..40].try_into() else {
            return Err(FirewallError::InvalidPolicy)
        };
        RateLimit::new(
            u32::from_le_bytes([input[28], input[29], input[30], input[31]]),
            u64::from_le_bytes(window_ms),
        )
    };
    Ok(FirewallRule { direction, protocol, source, destination, source_ports, destination_ports, action: action(input[2]).ok_or(FirewallError::InvalidPolicy)?, stateful: input[3] != 0, capability: match input[4] { 0 => None, 1 => Some(CapabilityRight::Listen), 2 => Some(CapabilityRight::Connect), 4 => Some(CapabilityRight::Raw), 8 => Some(CapabilityRight::Ingress), _ => return Err(FirewallError::InvalidPolicy) }, rate_limit })
}

fn action(value: u8) -> Option<RuleAction> {
    match value { 0 => Some(RuleAction::Allow), 1 => Some(RuleAction::Drop), 2 => Some(RuleAction::Reject), _ => None }
}

fn sign_header(key: &CapabilityKey, packet: PacketView<'_>) -> PacketSignature {
    let header_length = packet.frame.len().min(packet.payload.as_ptr() as usize - packet.frame.as_ptr() as usize);
    mac(key, &packet.frame[..header_length])
}

fn mac(key: &CapabilityKey, message: &[u8]) -> PacketSignature {
    let mut inner = [0x36; 64];
    let mut outer = [0x5c; 64];
    for index in 0..32 { inner[index] ^= key.0[index]; outer[index] ^= key.0[index] }
    let mut input = [0; 256];
    let inner_len = 64 + message.len().min(192);
    input[..64].copy_from_slice(&inner);
    input[64..inner_len].copy_from_slice(&message[..inner_len - 64]);
    let inner_hash = sha256(&input[..inner_len]);
    input[..64].copy_from_slice(&outer);
    input[64..96].copy_from_slice(&inner_hash);
    PacketSignature(sha256(&input[..96]))
}

fn sha256(message: &[u8]) -> [u8; 32] {
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
        0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
    ];
    let bit_length = (message.len() as u64) * 8;
    let blocks = message.len().saturating_add(9).div_ceil(64);
    for block in 0..blocks {
        let mut schedule = [0u32; 64];
        for (index, word) in schedule[..16].iter_mut().enumerate() {
            let offset = block * 64 + index * 4;
            let value = if offset + 4 <= message.len() {
                let Ok(word) = message[offset..offset + 4].try_into() else {
                    return [0; 32]
                };
                u32::from_be_bytes(word)
            } else {
                let mut bytes = [0; 4];
                for byte in 0..4 {
                    let position = offset + byte;
                    bytes[byte] = if position < message.len() { message[position] } else if position == message.len() { 0x80 } else if position >= blocks * 64 - 8 { bit_length.to_be_bytes()[position - (blocks * 64 - 8)] } else { 0 };
                }
                u32::from_be_bytes(bytes)
            };
            *word = value;
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7) ^ schedule[index - 15].rotate_right(18) ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17) ^ schedule[index - 2].rotate_right(19) ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16].wrapping_add(s0).wrapping_add(schedule[index - 7]).wrapping_add(s1);
        }
        let mut working = state;
        for index in 0..64 {
            let constants = [
                0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
                0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
                0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
                0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
                0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
                0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
                0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
                0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
            ];
            let s1 = working[4].rotate_right(6) ^ working[4].rotate_right(11) ^ working[4].rotate_right(25);
            let choice = (working[4] & working[5]) ^ (!working[4] & working[6]);
            let temp1 = working[7].wrapping_add(s1).wrapping_add(choice).wrapping_add(constants[index]).wrapping_add(schedule[index]);
            let s0 = working[0].rotate_right(2) ^ working[0].rotate_right(13) ^ working[0].rotate_right(22);
            let majority = (working[0] & working[1]) ^ (working[0] & working[2]) ^ (working[1] & working[2]);
            let temp2 = s0.wrapping_add(majority);
            working = [temp1.wrapping_add(temp2), working[0], working[1], working[2], working[3].wrapping_add(temp1), working[4], working[5], working[6]];
        }
        for index in 0..8 { state[index] = state[index].wrapping_add(working[index]) }
    }
    let mut output = [0; 32];
    for (index, word) in state.iter().enumerate() { output[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes()) }
    output
}

#[cfg(test)]
mod tests {
    use super::{FirewallError, FirewallPolicy, MAX_FIREWALL_RULES};

    #[test]
    fn truncated_policy_is_rejected() {
        assert_eq!(
            FirewallPolicy::<MAX_FIREWALL_RULES>::decode(&[0; 23]),
            Err(FirewallError::InvalidPolicy)
        );
    }
}
