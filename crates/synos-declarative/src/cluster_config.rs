use synos_fabric::NodeId;
use synos_status::{IntoStatus, Status};
use synos_synfs::{CheckpointId, Error as SynFsError, SynFs};
use synos_system_model::ContentId;

use crate::{BoundedText, ConfigurationRuntime, SignedConfiguration, SystemSpec};

pub const MAX_CLUSTER_NAME_BYTES: usize = 64;
pub const MAX_CLUSTER_DESCRIPTION_BYTES: usize = 128;
pub const MAX_TRANSPORTS: usize = 8;
pub const MAX_TRANSPORT_NAME_BYTES: usize = 32;
pub const MAX_ENDPOINT_BYTES: usize = 96;
pub const MAX_NODE_OVERRIDES: usize = 32;
pub const MAX_CONFIGURATION_CHANGES: usize = 32;
pub const MAX_AFFECTED_NODES: usize = 32;
pub const MAX_AFFECTED_SERVICES: usize = 24;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryPolicy {
    Disabled,
    Static,
    Mesh,
    Mdns,
    Hybrid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MembershipPolicy {
    Static,
    Automatic,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionPolicy {
    Open,
    Invitation,
    Attested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportKind {
    Loopback,
    Ethernet,
    Cxl,
    Wireless,
    Tunnel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterIdentitySpec {
    pub id: u128,
    pub name: BoundedText<MAX_CLUSTER_NAME_BYTES>,
    pub description: BoundedText<MAX_CLUSTER_DESCRIPTION_BYTES>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuorumSpec {
    pub voting_members: u16,
    pub required_votes: u16,
    pub read_only_without_quorum: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransportSpec {
    pub name: BoundedText<MAX_TRANSPORT_NAME_BYTES>,
    pub kind: TransportKind,
    pub endpoint: BoundedText<MAX_ENDPOINT_BYTES>,
    pub enabled: bool,
    pub priority: u16,
    pub mtu: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SecuritySpec {
    pub require_signed_commits: bool,
    pub require_mutual_identity: bool,
    pub require_attestation: bool,
    pub encrypt_control_plane: bool,
    pub encrypt_data_plane: bool,
    pub trust_root: u128,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceSpec {
    pub cpu_limit: u64,
    pub memory_limit_bytes: u64,
    pub cxl_limit_bytes: u64,
    pub storage_limit_bytes: u64,
    pub network_limit_mbps: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FederationSpec {
    pub enabled: bool,
    pub allow_remote_workloads: bool,
    pub require_attestation: bool,
    pub lease_ttl_us: u64,
    pub max_leases: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeOverrideSpec {
    pub node: u32,
    pub discovery: Option<DiscoveryPolicy>,
    pub admission: Option<AdmissionPolicy>,
    pub heartbeat_period_us: Option<u64>,
    pub missed_heartbeat_limit: Option<u16>,
    pub transport: Option<TransportKind>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterSpec {
    pub identity: ClusterIdentitySpec,
    pub discovery: DiscoveryPolicy,
    pub membership: MembershipPolicy,
    pub admission: AdmissionPolicy,
    pub quorum: QuorumSpec,
    pub heartbeat_period_us: u64,
    pub missed_heartbeat_limit: u16,
    pub security: SecuritySpec,
    pub resources: ResourceSpec,
    pub federation: FederationSpec,
    transports: [Option<TransportSpec>; MAX_TRANSPORTS],
    transport_count: usize,
    overrides: [Option<NodeOverrideSpec>; MAX_NODE_OVERRIDES],
    override_count: usize,
}

impl ClusterSpec {
    pub fn safe_defaults() -> Self {
        let mut spec = Self {
            identity: ClusterIdentitySpec {
                id: 1,
                name: BoundedText::new("synos").expect("valid default cluster name"),
                description: BoundedText::EMPTY,
            },
            discovery: DiscoveryPolicy::Hybrid,
            membership: MembershipPolicy::Static,
            admission: AdmissionPolicy::Invitation,
            quorum: QuorumSpec {
                voting_members: 1,
                required_votes: 1,
                read_only_without_quorum: true,
            },
            heartbeat_period_us: 1_000_000,
            missed_heartbeat_limit: 5,
            security: SecuritySpec {
                require_signed_commits: true,
                require_mutual_identity: true,
                require_attestation: false,
                encrypt_control_plane: true,
                encrypt_data_plane: true,
                trust_root: 1,
            },
            resources: ResourceSpec {
                cpu_limit: u64::MAX,
                memory_limit_bytes: u64::MAX,
                cxl_limit_bytes: u64::MAX,
                storage_limit_bytes: u64::MAX,
                network_limit_mbps: u64::MAX,
            },
            federation: FederationSpec {
                enabled: false,
                allow_remote_workloads: false,
                require_attestation: true,
                lease_ttl_us: 3_600_000_000,
                max_leases: 0,
            },
            transports: [None; MAX_TRANSPORTS],
            transport_count: 0,
            overrides: [None; MAX_NODE_OVERRIDES],
            override_count: 0,
        };
        spec.push_transport(TransportSpec {
            name: BoundedText::new("loopback").expect("valid default transport name"),
            kind: TransportKind::Loopback,
            endpoint: BoundedText::EMPTY,
            enabled: true,
            priority: 0,
            mtu: 65_535,
        })
        .expect("default transport fits");
        spec
    }

    pub fn transports(&self) -> impl Iterator<Item = TransportSpec> + '_ {
        self.transports.iter().flatten().copied()
    }

    pub fn overrides(&self) -> impl Iterator<Item = NodeOverrideSpec> + '_ {
        self.overrides.iter().flatten().copied()
    }

    pub fn override_for(&self, node: NodeId) -> Option<NodeOverrideSpec> {
        self.overrides()
            .find(|override_spec| override_spec.node == node.raw())
    }

    pub fn effective_for(&self, node: NodeId) -> EffectiveClusterSpec {
        let mut effective = EffectiveClusterSpec {
            discovery: self.discovery,
            admission: self.admission,
            heartbeat_period_us: self.heartbeat_period_us,
            missed_heartbeat_limit: self.missed_heartbeat_limit,
            transport: None,
        };
        if let Some(override_spec) = self.override_for(node) {
            if let Some(value) = override_spec.discovery {
                effective.discovery = value
            }
            if let Some(value) = override_spec.admission {
                effective.admission = value
            }
            if let Some(value) = override_spec.heartbeat_period_us {
                effective.heartbeat_period_us = value
            }
            if let Some(value) = override_spec.missed_heartbeat_limit {
                effective.missed_heartbeat_limit = value
            }
            effective.transport = override_spec.transport;
        }
        effective
    }

    pub fn validate(&self) -> Result<(), ConfigurationValidationError> {
        if self.identity.id == 0 || self.identity.name.is_empty() {
            return Err(ConfigurationValidationError::InvalidIdentity);
        }
        if self.quorum.voting_members == 0
            || self.quorum.required_votes == 0
            || self.quorum.required_votes > self.quorum.voting_members
        {
            return Err(ConfigurationValidationError::InvalidQuorum);
        }
        if self.heartbeat_period_us == 0 || self.missed_heartbeat_limit == 0 {
            return Err(ConfigurationValidationError::InvalidHeartbeat);
        }
        if self.security.require_attestation && self.security.trust_root == 0 {
            return Err(ConfigurationValidationError::MissingTrustRoot);
        }
        if self.federation.enabled
            && (self.federation.lease_ttl_us == 0 || self.federation.max_leases == 0)
        {
            return Err(ConfigurationValidationError::InvalidFederation);
        }
        for (index, transport) in self.transports().enumerate() {
            if transport.name.is_empty()
                || transport.mtu < 576
                || transport.mtu > 65_535
                || (transport.enabled
                    && transport.kind != TransportKind::Loopback
                    && transport.endpoint.is_empty())
            {
                return Err(ConfigurationValidationError::InvalidTransport { index });
            }
            if self
                .transports()
                .take(index)
                .any(|previous| previous.name == transport.name)
            {
                return Err(ConfigurationValidationError::DuplicateTransport);
            }
        }
        for (index, override_spec) in self.overrides().enumerate() {
            if override_spec.node == 0
                || override_spec.heartbeat_period_us == Some(0)
                || override_spec.missed_heartbeat_limit == Some(0)
            {
                return Err(ConfigurationValidationError::InvalidNodeOverride { index });
            }
            if self
                .overrides()
                .take(index)
                .any(|previous| previous.node == override_spec.node)
            {
                return Err(ConfigurationValidationError::DuplicateNodeOverride);
            }
        }
        Ok(())
    }

    pub(crate) fn push_transport(
        &mut self,
        transport: TransportSpec,
    ) -> Result<(), ConfigurationValidationError> {
        if self.transport_count == MAX_TRANSPORTS {
            return Err(ConfigurationValidationError::Capacity);
        }
        self.transports[self.transport_count] = Some(transport);
        self.transport_count += 1;
        Ok(())
    }

    pub(crate) fn clear_transports(&mut self) {
        self.transports = [None; MAX_TRANSPORTS];
        self.transport_count = 0;
    }

    pub(crate) fn push_override(
        &mut self,
        override_spec: NodeOverrideSpec,
    ) -> Result<(), ConfigurationValidationError> {
        if self.override_count == MAX_NODE_OVERRIDES {
            return Err(ConfigurationValidationError::Capacity);
        }
        self.overrides[self.override_count] = Some(override_spec);
        self.override_count += 1;
        Ok(())
    }

    pub fn digest(&self) -> ContentId {
        let mut encoder = ClusterEncoder::new();
        encoder.u128(self.identity.id);
        encoder.text(self.identity.name.as_str());
        encoder.text(self.identity.description.as_str());
        encoder.u8(self.discovery as u8);
        encoder.u8(self.membership as u8);
        encoder.u8(self.admission as u8);
        encoder.u16(self.quorum.voting_members);
        encoder.u16(self.quorum.required_votes);
        encoder.u8(self.quorum.read_only_without_quorum as u8);
        encoder.u64(self.heartbeat_period_us);
        encoder.u16(self.missed_heartbeat_limit);
        encoder.u8(self.security.require_signed_commits as u8);
        encoder.u8(self.security.require_mutual_identity as u8);
        encoder.u8(self.security.require_attestation as u8);
        encoder.u8(self.security.encrypt_control_plane as u8);
        encoder.u8(self.security.encrypt_data_plane as u8);
        encoder.u128(self.security.trust_root);
        encoder.u64(self.resources.cpu_limit);
        encoder.u64(self.resources.memory_limit_bytes);
        encoder.u64(self.resources.cxl_limit_bytes);
        encoder.u64(self.resources.storage_limit_bytes);
        encoder.u64(self.resources.network_limit_mbps);
        encoder.u8(self.federation.enabled as u8);
        encoder.u8(self.federation.allow_remote_workloads as u8);
        encoder.u8(self.federation.require_attestation as u8);
        encoder.u64(self.federation.lease_ttl_us);
        encoder.u32(self.federation.max_leases);
        for transport in self.transports() {
            encoder.u8(1);
            encoder.text(transport.name.as_str());
            encoder.u8(transport.kind as u8);
            encoder.text(transport.endpoint.as_str());
            encoder.u8(transport.enabled as u8);
            encoder.u16(transport.priority);
            encoder.u32(transport.mtu);
        }
        for override_spec in self.overrides() {
            encoder.u8(2);
            encoder.u32(override_spec.node);
            encoder.u8(override_spec.discovery.map_or(0, |value| value as u8 + 1));
            encoder.u8(override_spec.admission.map_or(0, |value| value as u8 + 1));
            encode_optional_u64(&mut encoder, override_spec.heartbeat_period_us);
            encode_optional_u16(&mut encoder, override_spec.missed_heartbeat_limit);
            encoder.u8(override_spec.transport.map_or(0, |value| value as u8 + 1));
        }
        ContentId::hash(&encoder.bytes[..encoder.length])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EffectiveClusterSpec {
    pub discovery: DiscoveryPolicy,
    pub admission: AdmissionPolicy,
    pub heartbeat_period_us: u64,
    pub missed_heartbeat_limit: u16,
    pub transport: Option<TransportKind>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationValidationError {
    Capacity,
    DuplicateNodeOverride,
    DuplicateTransport,
    InvalidFederation,
    InvalidHeartbeat,
    InvalidIdentity,
    InvalidNodeOverride { index: usize },
    InvalidQuorum,
    InvalidTransport { index: usize },
    MissingTrustRoot,
}

impl IntoStatus for ConfigurationValidationError {
    fn status(self) -> Status {
        match self {
            Self::Capacity => Status::NO_SPACE,
            Self::DuplicateNodeOverride
            | Self::DuplicateTransport
            | Self::InvalidFederation
            | Self::InvalidHeartbeat
            | Self::InvalidIdentity
            | Self::InvalidNodeOverride { .. }
            | Self::InvalidQuorum
            | Self::InvalidTransport { .. }
            | Self::MissingTrustRoot => Status::INVALID_ARGUMENT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigurationArea {
    ClusterIdentity,
    Discovery,
    Membership,
    Quorum,
    Transport,
    Security,
    Resources,
    Federation,
    NodeOverrides,
    Services,
    Network,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigurationChange {
    pub area: ConfigurationArea,
    pub before: ContentId,
    pub after: ContentId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffError {
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigurationDiff {
    pub from_revision: Option<u64>,
    pub to_revision: u64,
    changes: [Option<ConfigurationChange>; MAX_CONFIGURATION_CHANGES],
    change_count: usize,
    affected_nodes: [Option<u32>; MAX_AFFECTED_NODES],
    node_count: usize,
    affected_services:
        [Option<BoundedText<{ crate::MAX_SERVICE_NAME_BYTES }>>; MAX_AFFECTED_SERVICES],
    service_count: usize,
    pub cluster_wide: bool,
}

impl ConfigurationDiff {
    pub fn between(previous: Option<&SystemSpec>, next: &SystemSpec) -> Result<Self, DiffError> {
        let mut diff = Self {
            from_revision: previous.map(SystemSpec::revision),
            to_revision: next.revision(),
            changes: [None; MAX_CONFIGURATION_CHANGES],
            change_count: 0,
            affected_nodes: [None; MAX_AFFECTED_NODES],
            node_count: 0,
            affected_services: [None; MAX_AFFECTED_SERVICES],
            service_count: 0,
            cluster_wide: false,
        };
        let old_cluster_digest = previous.map_or(ContentId::from_bytes([0; 32]), |value| {
            value.cluster().digest()
        });
        let new_cluster_digest = next.cluster().digest();
        let old_cluster = previous.map(SystemSpec::cluster);
        let new_cluster = next.cluster();
        if old_cluster.map_or(true, |value| value.identity != new_cluster.identity) {
            diff.add_change(
                ConfigurationArea::ClusterIdentity,
                old_cluster_digest,
                new_cluster_digest,
            )?;
        }
        if old_cluster.map_or(true, |value| value.discovery != new_cluster.discovery) {
            diff.add_change(
                ConfigurationArea::Discovery,
                old_cluster_digest,
                new_cluster_digest,
            )?;
        }
        if old_cluster.map_or(true, |value| {
            value.membership != new_cluster.membership || value.admission != new_cluster.admission
        }) {
            diff.add_change(
                ConfigurationArea::Membership,
                old_cluster_digest,
                new_cluster_digest,
            )?;
        }
        if old_cluster.map_or(true, |value| {
            value.quorum != new_cluster.quorum
                || value.heartbeat_period_us != new_cluster.heartbeat_period_us
                || value.missed_heartbeat_limit != new_cluster.missed_heartbeat_limit
        }) {
            diff.add_change(
                ConfigurationArea::Quorum,
                old_cluster_digest,
                new_cluster_digest,
            )?;
        }
        if old_cluster.map_or(true, |value| {
            !value.transports().eq(new_cluster.transports())
        }) {
            diff.add_change(
                ConfigurationArea::Transport,
                old_cluster_digest,
                new_cluster_digest,
            )?;
        }
        if old_cluster.map_or(true, |value| value.security != new_cluster.security) {
            diff.add_change(
                ConfigurationArea::Security,
                old_cluster_digest,
                new_cluster_digest,
            )?;
        }
        if old_cluster.map_or(true, |value| value.resources != new_cluster.resources) {
            diff.add_change(
                ConfigurationArea::Resources,
                old_cluster_digest,
                new_cluster_digest,
            )?;
        }
        if old_cluster.map_or(true, |value| value.federation != new_cluster.federation) {
            diff.add_change(
                ConfigurationArea::Federation,
                old_cluster_digest,
                new_cluster_digest,
            )?;
        }
        if old_cluster.map_or(true, |value| !value.overrides().eq(new_cluster.overrides())) {
            diff.add_change(
                ConfigurationArea::NodeOverrides,
                old_cluster_digest,
                new_cluster_digest,
            )?;
            diff.cluster_wide = true;
        }
        if old_cluster_digest != new_cluster_digest {
            diff.cluster_wide = true;
            if let Some(value) = old_cluster {
                for node in value.overrides() {
                    diff.add_node(node.node)?;
                }
            }
            for node in new_cluster.overrides() {
                diff.add_node(node.node)?;
            }
        }
        if previous.map_or(true, |value| !value.services().eq(next.services())) {
            diff.add_change(
                ConfigurationArea::Services,
                ContentId::hash(b"services-before"),
                ContentId::hash(b"services-after"),
            )?;
            if let Some(value) = previous {
                for service in value.services() {
                    diff.add_service(service.name)?;
                }
            }
            for service in next.services() {
                diff.add_service(service.name)?;
            }
        }
        if previous.map_or(true, |value| value.network() != next.network()) {
            diff.add_change(
                ConfigurationArea::Network,
                ContentId::hash(b"network-before"),
                ContentId::hash(b"network-after"),
            )?;
            diff.cluster_wide = true;
        }
        Ok(diff)
    }

    pub fn changes(&self) -> impl Iterator<Item = ConfigurationChange> + '_ {
        self.changes.iter().flatten().copied()
    }

    pub fn affected_nodes(&self) -> impl Iterator<Item = u32> + '_ {
        self.affected_nodes.iter().flatten().copied()
    }

    pub fn affected_services(
        &self,
    ) -> impl Iterator<Item = BoundedText<{ crate::MAX_SERVICE_NAME_BYTES }>> + '_ {
        self.affected_services.iter().flatten().copied()
    }

    fn add_change(
        &mut self,
        area: ConfigurationArea,
        before: ContentId,
        after: ContentId,
    ) -> Result<(), DiffError> {
        if self.change_count == MAX_CONFIGURATION_CHANGES {
            return Err(DiffError::Capacity);
        }
        self.changes[self.change_count] = Some(ConfigurationChange {
            area,
            before,
            after,
        });
        self.change_count += 1;
        Ok(())
    }

    fn add_node(&mut self, node: u32) -> Result<(), DiffError> {
        if self
            .affected_nodes
            .iter()
            .flatten()
            .any(|existing| *existing == node)
        {
            return Ok(());
        }
        if self.node_count == MAX_AFFECTED_NODES {
            return Err(DiffError::Capacity);
        }
        self.affected_nodes[self.node_count] = Some(node);
        self.node_count += 1;
        Ok(())
    }

    fn add_service(
        &mut self,
        service: BoundedText<{ crate::MAX_SERVICE_NAME_BYTES }>,
    ) -> Result<(), DiffError> {
        if self
            .affected_services
            .iter()
            .flatten()
            .any(|existing| *existing == service)
        {
            return Ok(());
        }
        if self.service_count == MAX_AFFECTED_SERVICES {
            return Err(DiffError::Capacity);
        }
        self.affected_services[self.service_count] = Some(service);
        self.service_count += 1;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QuorumReceipt {
    pub required_votes: u16,
    pub acknowledged_votes: u16,
}

pub trait ConfigurationQuorum {
    type Error;

    fn acknowledge(
        &mut self,
        update: &SignedConfiguration,
        diff: &ConfigurationDiff,
    ) -> Result<QuorumReceipt, Self::Error>;

    fn rollback(&mut self, _revision: u64) {}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterActivationReceipt {
    pub revision: u64,
    pub previous_revision: Option<u64>,
    pub generation: u64,
    pub quorum: QuorumReceipt,
    pub diff: ConfigurationDiff,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterReconfigureError<Q, R> {
    AlreadyActive,
    HistoryFull,
    MaintenanceWindowRequired,
    Signature(crate::SignatureError),
    Validation(ConfigurationValidationError),
    Diff(DiffError),
    StaleRevision,
    Storage(SynFsError),
    HealthCheck(Status),
    Quorum(Q),
    QuorumUnavailable,
    Runtime(R),
    NoRollbackTarget,
}

#[derive(Clone, Copy)]
struct ClusterHistoryEntry {
    checkpoint: CheckpointId,
    previous: Option<SystemSpec>,
    receipt: ClusterActivationReceipt,
}

pub struct ClusterConfigurationManager<
    const HISTORY: usize = { crate::DEFAULT_RECONFIGURE_HISTORY },
> {
    active: Option<SystemSpec>,
    staged: Option<SignedConfiguration>,
    entries: [Option<ClusterHistoryEntry>; HISTORY],
    maintenance_window: Option<(u64, u64)>,
}

impl<const HISTORY: usize> ClusterConfigurationManager<HISTORY> {
    pub const fn new() -> Self {
        Self {
            active: None,
            staged: None,
            entries: [None; HISTORY],
            maintenance_window: None,
        }
    }

    pub const fn active(&self) -> Option<&SystemSpec> {
        self.active.as_ref()
    }

    pub const fn staged(&self) -> Option<&SignedConfiguration> {
        self.staged.as_ref()
    }

    pub const fn history_len(&self) -> usize {
        let mut index = 0;
        while index < HISTORY {
            if self.entries[index].is_none() {
                return index;
            }
            index += 1;
        }
        index
    }

    pub fn set_maintenance_window(
        &mut self,
        starts_at_us: u64,
        ends_at_us: u64,
    ) -> Result<(), ClusterReconfigureError<(), ()>> {
        if starts_at_us >= ends_at_us {
            return Err(ClusterReconfigureError::MaintenanceWindowRequired);
        }
        self.maintenance_window = Some((starts_at_us, ends_at_us));
        Ok(())
    }

    pub fn stage<const KEYS: usize>(
        &mut self,
        enforcer: &crate::TpmConfigurationEnforcer<KEYS>,
        update: SignedConfiguration,
        nodes: &[NodeId],
    ) -> Result<ConfigurationDiff, ClusterReconfigureError<(), ()>> {
        update
            .configuration()
            .cluster()
            .validate()
            .map_err(ClusterReconfigureError::Validation)?;
        enforcer
            .verify_cluster(&update, nodes)
            .map_err(ClusterReconfigureError::Signature)?;
        if let Some(active) = self.active {
            if update.revision() <= active.revision() {
                return Err(if update.revision() == active.revision() {
                    ClusterReconfigureError::AlreadyActive
                } else {
                    ClusterReconfigureError::StaleRevision
                });
            }
            if update.expected_previous_revision() != Some(active.revision()) {
                return Err(ClusterReconfigureError::StaleRevision);
            }
        } else if update.expected_previous_revision().is_some() {
            return Err(ClusterReconfigureError::StaleRevision);
        }
        let diff = ConfigurationDiff::between(self.active.as_ref(), update.configuration())
            .map_err(ClusterReconfigureError::Diff)?;
        self.staged = Some(update);
        Ok(diff)
    }

    pub fn activate_staged<const BLOCKS: usize, Q, R>(
        &mut self,
        filesystem: &mut SynFs<BLOCKS>,
        quorum: &mut Q,
        runtime: &mut R,
        now_us: u64,
    ) -> Result<ClusterActivationReceipt, ClusterReconfigureError<Q::Error, R::Error>>
    where
        Q: ConfigurationQuorum,
        R: ConfigurationRuntime,
    {
        let update = self.staged.ok_or(ClusterReconfigureError::StaleRevision)?;
        if self
            .maintenance_window
            .is_some_and(|(start, end)| now_us < start || now_us > end)
        {
            return Err(ClusterReconfigureError::MaintenanceWindowRequired);
        }
        if self.history_len() == HISTORY || HISTORY == 0 {
            return Err(ClusterReconfigureError::HistoryFull);
        }
        let previous = self.active;
        let diff = ConfigurationDiff::between(previous.as_ref(), update.configuration())
            .map_err(ClusterReconfigureError::Diff)?;
        let checkpoint = filesystem
            .create_checkpoint()
            .map_err(ClusterReconfigureError::Storage)?;
        let rollback = |filesystem: &mut SynFs<BLOCKS>, runtime: &mut R| {
            runtime.rollback();
            runtime.clear_stage();
            let _ = filesystem.rollback_to_checkpoint(checkpoint.id);
            let _ = filesystem.release_checkpoint(checkpoint.id);
        };
        runtime.stage(update.configuration()).map_err(|error| {
            rollback(filesystem, runtime);
            ClusterReconfigureError::Runtime(error)
        })?;
        runtime
            .health_check(update.configuration())
            .map_err(|status| {
                rollback(filesystem, runtime);
                ClusterReconfigureError::HealthCheck(status)
            })?;
        let quorum_receipt = quorum.acknowledge(&update, &diff).map_err(|error| {
            rollback(filesystem, runtime);
            ClusterReconfigureError::Quorum(error)
        })?;
        if quorum_receipt.required_votes < update.configuration().cluster().quorum.required_votes
            || quorum_receipt.acknowledged_votes < quorum_receipt.required_votes
        {
            quorum.rollback(update.revision());
            rollback(filesystem, runtime);
            return Err(ClusterReconfigureError::QuorumUnavailable);
        }
        runtime.commit().map_err(|error| {
            quorum.rollback(update.revision());
            rollback(filesystem, runtime);
            ClusterReconfigureError::Runtime(error)
        })?;
        let receipt = ClusterActivationReceipt {
            revision: update.revision(),
            previous_revision: previous.map(|value| value.revision()),
            generation: filesystem.generation(),
            quorum: quorum_receipt,
            diff,
        };
        self.entries[self.history_len()] = Some(ClusterHistoryEntry {
            checkpoint: checkpoint.id,
            previous,
            receipt,
        });
        self.active = Some(*update.configuration());
        self.staged = None;
        Ok(receipt)
    }

    pub fn rollback_last<const BLOCKS: usize, R: ConfigurationRuntime>(
        &mut self,
        filesystem: &mut SynFs<BLOCKS>,
        runtime: &mut R,
    ) -> Result<ClusterActivationReceipt, ClusterReconfigureError<(), R::Error>> {
        let index = self
            .history_len()
            .checked_sub(1)
            .ok_or(ClusterReconfigureError::NoRollbackTarget)?;
        let entry = self.entries[index].ok_or(ClusterReconfigureError::NoRollbackTarget)?;
        filesystem
            .rollback_to_checkpoint(entry.checkpoint)
            .map_err(ClusterReconfigureError::Storage)?;
        if let Some(previous) = entry.previous {
            runtime
                .stage(&previous)
                .map_err(ClusterReconfigureError::Runtime)?;
        } else {
            runtime.clear_stage();
        }
        runtime.commit().map_err(ClusterReconfigureError::Runtime)?;
        filesystem
            .release_checkpoint(entry.checkpoint)
            .map_err(ClusterReconfigureError::Storage)?;
        self.entries[index] = None;
        self.active = entry.previous;
        Ok(ClusterActivationReceipt {
            revision: entry.previous.map_or(0, |value| value.revision()),
            previous_revision: Some(entry.receipt.revision),
            generation: filesystem.generation(),
            quorum: entry.receipt.quorum,
            diff: entry.receipt.diff,
        })
    }
}

impl<const HISTORY: usize> Default for ClusterConfigurationManager<HISTORY> {
    fn default() -> Self {
        Self::new()
    }
}

struct ClusterEncoder {
    bytes: [u8; 16_384],
    length: usize,
}

impl ClusterEncoder {
    const fn new() -> Self {
        Self {
            bytes: [0; 16_384],
            length: 0,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        let end = self.length + bytes.len();
        assert!(end <= self.bytes.len());
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
        self.u16(value.len() as u16);
        self.push(value.as_bytes())
    }
}

fn encode_optional_u16(encoder: &mut ClusterEncoder, value: Option<u16>) {
    if let Some(value) = value {
        encoder.u8(1);
        encoder.u16(value);
    } else {
        encoder.u8(0)
    }
}

fn encode_optional_u64(encoder: &mut ClusterEncoder, value: Option<u64>) {
    if let Some(value) = value {
        encoder.u8(1);
        encoder.u64(value);
    } else {
        encoder.u8(0)
    }
}
