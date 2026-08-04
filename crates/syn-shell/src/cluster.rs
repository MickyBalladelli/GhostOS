use synos_status::Status;
use synos_system_model::command::{
    ArgumentKind, ArgumentSpec, CommandSpec, OutputText, OutputValue, StructuredOutput,
};

use crate::{
    interpreter::{CommandExecutor, ExecutionToken},
    parser::{CommandCall, CommandRegistry, RouteId, Value},
    Error,
};

// Cluster routes are part of the shell ABI. Do not renumber them after a
// release; adding a command gets a new route at the end of this range.
pub const SHOW_CLUSTER_ROUTE: u16 = 100;
pub const LIST_CLUSTERS_ROUTE: u16 = 101;
pub const CREATE_CLUSTER_ROUTE: u16 = 102;
pub const JOIN_CLUSTER_ROUTE: u16 = 103;
pub const LEAVE_CLUSTER_ROUTE: u16 = 104;
pub const DELETE_CLUSTER_ROUTE: u16 = 105;
pub const MODIFY_CLUSTER_ROUTE: u16 = 106;
pub const RENAME_CLUSTER_ROUTE: u16 = 107;
pub const SET_CLUSTER_ROUTE: u16 = 108;
pub const USE_CLUSTER_ROUTE: u16 = 109;
pub const INVITE_NODE_ROUTE: u16 = 110;
pub const ACCEPT_NODE_ROUTE: u16 = 111;
pub const REJECT_NODE_ROUTE: u16 = 112;
pub const REMOVE_NODE_ROUTE: u16 = 113;
pub const DRAIN_NODE_ROUTE: u16 = 114;
pub const FENCE_NODE_ROUTE: u16 = 115;
pub const REJOIN_NODE_ROUTE: u16 = 116;
pub const HELP_ROUTE: u16 = 117;
pub const INVITE_CLUSTER_ROUTE: u16 = 118;
pub const ACCEPT_CLUSTER_ROUTE: u16 = 119;
pub const REJECT_CLUSTER_ROUTE: u16 = 120;
pub const REMOVE_FEDERATION_ROUTE: u16 = 121;

pub const CLUSTER_COMMAND_COUNT: usize = 21;
pub const CLUSTER_ID_BYTES: usize = 64;
pub const CLUSTER_STATUS_BYTES: usize = 32;
pub const MAX_CLUSTER_VIEW_ROWS: usize = 4;
pub const MAX_CLUSTER_LIST_ROWS: usize = MAX_CLUSTER_VIEW_ROWS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterHealth {
    Healthy,
    Degraded,
    Partitioned,
    Unavailable,
}

impl ClusterHealth {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "HEALTHY",
            Self::Degraded => "DEGRADED",
            Self::Partitioned => "PARTITIONED",
            Self::Unavailable => "UNAVAILABLE",
        }
    }
}

/// Stable summary returned by `SHOW CLUSTER`.
///
/// Identity and role names are text because node and cluster identifiers may
/// be rendered as numeric IDs, UUIDs, or administrator-selected names by the
/// provider. Counts and capacities remain unsigned wire fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterSnapshot {
    pub cluster_id: crate::Text<CLUSTER_ID_BYTES>,
    pub cluster_name: crate::Text<CLUSTER_ID_BYTES>,
    pub status: crate::Text<CLUSTER_STATUS_BYTES>,
    pub leader: crate::Text<CLUSTER_ID_BYTES>,
    pub coordinator: crate::Text<CLUSTER_ID_BYTES>,
    pub generation: u64,
    pub sampled_at_us: u64,
    pub member_count: u64,
    pub healthy_members: u64,
    pub degraded_members: u64,
    pub failed_members: u64,
    pub voting_members: u64,
    pub quorum_required: u64,
    pub quorum_available: u64,
    pub health: ClusterHealth,
    pub cpu_capacity: u64,
    pub cpu_available: u64,
    pub memory_capacity_bytes: u64,
    pub memory_available_bytes: u64,
    pub cxl_capacity_bytes: u64,
    pub vram_capacity_bytes: u64,
    pub storage_capacity_bytes: u64,
    pub control_protocol_version: u64,
    pub data_protocol_version: u64,
    pub minimum_protocol_version: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterListEntry {
    pub cluster_id: crate::Text<CLUSTER_ID_BYTES>,
    pub cluster_name: crate::Text<CLUSTER_ID_BYTES>,
    pub status: crate::Text<CLUSTER_STATUS_BYTES>,
    pub trusted: bool,
    pub joined: bool,
    pub available: bool,
    pub degraded: bool,
    pub federated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterListView {
    pub generation: u64,
    pub total_count: u64,
    pub page: u64,
    pub clusters: [Option<ClusterListEntry>; MAX_CLUSTER_LIST_ROWS],
    pub next_page: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterListQuery {
    pub filter: Option<crate::Text<{ crate::MAX_TOKEN_BYTES }>>,
    pub status: Option<crate::Text<{ crate::MAX_TOKEN_BYTES }>>,
    pub limit: u64,
    pub page: u64,
    pub trusted: Option<bool>,
    pub joined: Option<bool>,
    pub available: Option<bool>,
    pub degraded: Option<bool>,
    pub federated: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterMember {
    pub node: crate::Text<CLUSTER_ID_BYTES>,
    pub role: crate::Text<CLUSTER_STATUS_BYTES>,
    pub status: crate::Text<CLUSTER_STATUS_BYTES>,
    pub voting: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterMembersView {
    pub generation: u64,
    pub member_count: u64,
    pub voting_members: u64,
    pub healthy_members: u64,
    pub degraded_members: u64,
    pub failed_members: u64,
    pub members: [Option<ClusterMember>; MAX_CLUSTER_VIEW_ROWS],
    pub next_member: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterLink {
    pub from: crate::Text<CLUSTER_ID_BYTES>,
    pub to: crate::Text<CLUSTER_ID_BYTES>,
    pub transport: crate::Text<CLUSTER_STATUS_BYTES>,
    pub route: crate::Text<CLUSTER_STATUS_BYTES>,
    pub zone: crate::Text<CLUSTER_STATUS_BYTES>,
    pub status: crate::Text<CLUSTER_STATUS_BYTES>,
    pub latency_us: u64,
    pub bandwidth_mbps: u64,
    pub mtu: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterTopologyView {
    pub generation: u64,
    pub topology: crate::Text<CLUSTER_STATUS_BYTES>,
    pub link_count: u64,
    pub reachable_links: u64,
    pub failed_links: u64,
    pub maximum_latency_us: u64,
    pub minimum_bandwidth_mbps: u64,
    pub links: [Option<ClusterLink>; MAX_CLUSTER_VIEW_ROWS],
    pub next_link: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterHealthView {
    pub generation: u64,
    pub health: ClusterHealth,
    pub status: crate::Text<CLUSTER_STATUS_BYTES>,
    pub heartbeat_period_us: u64,
    pub missed_heartbeat_limit: u64,
    pub last_change_us: u64,
    pub healthy_nodes: u64,
    pub degraded_nodes: u64,
    pub failed_nodes: u64,
    pub quorum: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterResourcesView {
    pub generation: u64,
    pub cpu_capacity: u64,
    pub cpu_available: u64,
    pub memory_capacity_bytes: u64,
    pub memory_available_bytes: u64,
    pub cxl_capacity_bytes: u64,
    pub vram_capacity_bytes: u64,
    pub storage_capacity_bytes: u64,
    pub network_bandwidth_mbps: u64,
    pub active_leases: u64,
    pub running_workloads: u64,
    pub queued_workloads: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterConfigView {
    pub generation: u64,
    pub admission_policy: crate::Text<CLUSTER_STATUS_BYTES>,
    pub quorum_policy: crate::Text<CLUSTER_STATUS_BYTES>,
    pub discovery_policy: crate::Text<CLUSTER_STATUS_BYTES>,
    pub endpoint: crate::Text<CLUSTER_ID_BYTES>,
    pub heartbeat_period_us: u64,
    pub missed_heartbeat_limit: u64,
    pub control_protocol_version: u64,
    pub data_protocol_version: u64,
    pub minimum_protocol_version: u64,
    pub staged: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClusterCommandHelp {
    pub name: &'static str,
    pub synopsis: &'static str,
    pub description: &'static str,
    pub aliases: &'static str,
    pub qualifiers: &'static str,
}

pub const CLUSTER_COMMAND_HELP: [ClusterCommandHelp; CLUSTER_COMMAND_COUNT] = [
    help(
        "SHOW-CLUSTER",
        "SHOW CLUSTER [name] [/view]",
        "Show cluster identity, state, membership, quorum, health, resources, or configuration.",
        "CLUSTER",
        "/MEMBERS /TOPOLOGY /HEALTH /RESOURCES /CONFIG /FEDERATED",
    ),
    help(
        "LIST-CLUSTERS",
        "LIST CLUSTERS [filter] [/view]",
        "List discovered and trusted clusters with bounded pagination.",
        "LS CLUSTERS, LS-CLUSTERS",
        "/STATUS /LIMIT /PAGE /TRUSTED /JOINED /AVAILABLE /DEGRADED /FEDERATED",
    ),
    help(
        "CREATE-CLUSTER",
        "CREATE CLUSTER name [/options]",
        "Create a local cluster identity and bootstrap metadata.",
        "",
        "/ID /DESCRIPTION /ENDPOINT /ADMISSION /QUORUM /ADMIN",
    ),
    help(
        "JOIN-CLUSTER",
        "JOIN CLUSTER [name] [/options]",
        "Request admission to a cluster.",
        "",
        "/INVITATION /TOKEN /ENDPOINT /FINGERPRINT /ATTESTATION /TIMEOUT /APPROVE",
    ),
    help(
        "LEAVE-CLUSTER",
        "LEAVE CLUSTER [name] [/options]",
        "Drain and leave the active or named cluster.",
        "",
        "/DRAIN /FORCE /CONFIRM /RECONCILE",
    ),
    help(
        "DELETE-CLUSTER",
        "DELETE CLUSTER name [/options]",
        "Retire a cluster after safety checks.",
        "REMOVE CLUSTER",
        "/DRAIN /FORCE /CONFIRM /RECONCILE",
    ),
    help(
        "MODIFY-CLUSTER",
        "MODIFY CLUSTER name [/options]",
        "Stage mutable cluster metadata or policy changes.",
        "",
        "/DESCRIPTION /ENDPOINT /ADMISSION /QUORUM /ADMIN /DRY_RUN /CONFIRM",
    ),
    help(
        "RENAME-CLUSTER",
        "RENAME CLUSTER old new",
        "Rename a cluster without changing its immutable identity.",
        "",
        "",
    ),
    help(
        "SET-CLUSTER",
        "SET CLUSTER [name] [/options]",
        "Set active cluster configuration.",
        "",
        "/DESCRIPTION /ENDPOINT /ADMISSION /QUORUM /DRY_RUN /CONFIRM",
    ),
    help(
        "USE-CLUSTER",
        "USE CLUSTER name",
        "Select the active cluster target for later commands.",
        "",
        "",
    ),
    help(
        "INVITE-NODE",
        "INVITE NODE node [/options]",
        "Create a bounded node admission invitation.",
        "",
        "/EXPIRATION /ROLE /ENDPOINT /FINGERPRINT /ATTESTATION",
    ),
    help(
        "ACCEPT-NODE",
        "ACCEPT NODE node [/options]",
        "Accept a pending node admission.",
        "",
        "/INVITATION /CONFIRM",
    ),
    help(
        "REJECT-NODE",
        "REJECT NODE node [/options]",
        "Reject a pending node admission.",
        "",
        "/INVITATION /CONFIRM",
    ),
    help(
        "REMOVE-NODE",
        "REMOVE NODE node [/options]",
        "Remove a node after fencing and reconciliation checks.",
        "",
        "/DRAIN /FORCE /CONFIRM /RECONCILE",
    ),
    help(
        "DRAIN-NODE",
        "DRAIN NODE node [/options]",
        "Stop new work and drain a node.",
        "",
        "/TIMEOUT /FORCE",
    ),
    help(
        "FENCE-NODE",
        "FENCE NODE node /CONFIRM",
        "Fence an unsafe node and advance the cluster epoch.",
        "",
        "/FORCE /CONFIRM",
    ),
    help(
        "REJOIN-NODE",
        "REJOIN NODE node [/options]",
        "Reconcile and re-admit a previously known node.",
        "",
        "/ENDPOINT /INVITATION /TOKEN /ATTESTATION /TIMEOUT",
    ),
    help(
        "INVITE-CLUSTER",
        "INVITE CLUSTER cluster [/options]",
        "Create a scoped federation invitation.",
        "",
        "/EXPIRATION /SCOPE",
    ),
    help(
        "ACCEPT-CLUSTER",
        "ACCEPT CLUSTER cluster [/options]",
        "Accept a federation invitation.",
        "",
        "/INVITATION /CONFIRM",
    ),
    help(
        "REJECT-CLUSTER",
        "REJECT CLUSTER cluster [/options]",
        "Reject a federation invitation.",
        "",
        "/INVITATION /CONFIRM",
    ),
    help(
        "REMOVE-FEDERATION",
        "REMOVE FEDERATION cluster /CONFIRM",
        "Remove a federation relationship.",
        "",
        "/FORCE /CONFIRM",
    ),
];

const fn help(
    name: &'static str,
    synopsis: &'static str,
    description: &'static str,
    aliases: &'static str,
    qualifiers: &'static str,
) -> ClusterCommandHelp {
    ClusterCommandHelp {
        name,
        synopsis,
        description,
        aliases,
        qualifiers,
    }
}

pub fn command_help(name: &str) -> Option<&'static ClusterCommandHelp> {
    let name = if name.eq_ignore_ascii_case("CLUSTER") {
        "SHOW-CLUSTER"
    } else if name.eq_ignore_ascii_case("LS-CLUSTERS") {
        "LIST-CLUSTERS"
    } else if name.eq_ignore_ascii_case("REMOVE-CLUSTER") {
        "DELETE-CLUSTER"
    } else {
        name
    };
    CLUSTER_COMMAND_HELP
        .iter()
        .find(|command| command.name.eq_ignore_ascii_case(name))
}

pub fn register_cluster_commands<const CAPACITY: usize>(
    registry: &mut CommandRegistry<CAPACITY>,
) -> Result<(), Error> {
    register_cluster_commands_impl(registry, true)
}

pub fn register_cluster_commands_without_help<const CAPACITY: usize>(
    registry: &mut CommandRegistry<CAPACITY>,
) -> Result<(), Error> {
    register_cluster_commands_impl(registry, false)
}

fn register_cluster_commands_impl<const CAPACITY: usize>(
    registry: &mut CommandRegistry<CAPACITY>,
    include_help: bool,
) -> Result<(), Error> {
    let name = positional("NAME", ArgumentKind::Text, false)?;
    let required_name = positional("NAME", ArgumentKind::Text, true)?;
    let node = positional("NODE", ArgumentKind::Text, true)?;
    let new_name = positional("NEW_NAME", ArgumentKind::Text, true)?;

    let views = [
        qualifier("MEMBERS", ArgumentKind::Boolean)?,
        qualifier("TOPOLOGY", ArgumentKind::Boolean)?,
        qualifier("HEALTH", ArgumentKind::Boolean)?,
        qualifier("RESOURCES", ArgumentKind::Boolean)?,
        qualifier("CONFIG", ArgumentKind::Boolean)?,
        qualifier("FEDERATED", ArgumentKind::Boolean)?,
    ];
    let show = [
        name, views[0], views[1], views[2], views[3], views[4], views[5],
    ];
    register(registry, "SHOW-CLUSTER", &show, SHOW_CLUSTER_ROUTE)?;

    let list = [
        name,
        qualifier("STATUS", ArgumentKind::Text)?,
        qualifier("LIMIT", ArgumentKind::Integer)?,
        qualifier("PAGE", ArgumentKind::Integer)?,
        qualifier("TRUSTED", ArgumentKind::Boolean)?,
        qualifier("JOINED", ArgumentKind::Boolean)?,
        qualifier("AVAILABLE", ArgumentKind::Boolean)?,
        qualifier("DEGRADED", ArgumentKind::Boolean)?,
        qualifier("FEDERATED", ArgumentKind::Boolean)?,
    ];
    register(registry, "LIST-CLUSTERS", &list, LIST_CLUSTERS_ROUTE)?;

    let create = [
        required_name,
        qualifier("ID", ArgumentKind::Text)?,
        qualifier("DESCRIPTION", ArgumentKind::Text)?,
        qualifier("ENDPOINT", ArgumentKind::Text)?,
        qualifier("ADMISSION", ArgumentKind::Text)?,
        qualifier("QUORUM", ArgumentKind::Text)?,
        qualifier("ADMIN", ArgumentKind::Text)?,
    ];
    register(registry, "CREATE-CLUSTER", &create, CREATE_CLUSTER_ROUTE)?;

    let join = [
        name,
        qualifier("INVITATION", ArgumentKind::Text)?,
        qualifier("TOKEN", ArgumentKind::Text)?,
        qualifier("ENDPOINT", ArgumentKind::Text)?,
        qualifier("FINGERPRINT", ArgumentKind::Text)?,
        qualifier("ATTESTATION", ArgumentKind::Text)?,
        qualifier("TIMEOUT", ArgumentKind::Integer)?,
        qualifier("APPROVE", ArgumentKind::Boolean)?,
    ];
    register(registry, "JOIN-CLUSTER", &join, JOIN_CLUSTER_ROUTE)?;

    let leave = [
        name,
        qualifier("DRAIN", ArgumentKind::Boolean)?,
        qualifier("FORCE", ArgumentKind::Boolean)?,
        qualifier("CONFIRM", ArgumentKind::Boolean)?,
        qualifier("RECONCILE", ArgumentKind::Boolean)?,
    ];
    register(registry, "LEAVE-CLUSTER", &leave, LEAVE_CLUSTER_ROUTE)?;

    let destructive_cluster = [
        required_name,
        qualifier("DRAIN", ArgumentKind::Boolean)?,
        qualifier("FORCE", ArgumentKind::Boolean)?,
        qualifier("CONFIRM", ArgumentKind::Boolean)?,
        qualifier("RECONCILE", ArgumentKind::Boolean)?,
    ];
    register(
        registry,
        "DELETE-CLUSTER",
        &destructive_cluster,
        DELETE_CLUSTER_ROUTE,
    )?;

    let modify = [
        required_name,
        qualifier("DESCRIPTION", ArgumentKind::Text)?,
        qualifier("ENDPOINT", ArgumentKind::Text)?,
        qualifier("ADMISSION", ArgumentKind::Text)?,
        qualifier("QUORUM", ArgumentKind::Text)?,
        qualifier("ADMIN", ArgumentKind::Text)?,
        qualifier("DRY_RUN", ArgumentKind::Boolean)?,
        qualifier("CONFIRM", ArgumentKind::Boolean)?,
    ];
    register(registry, "MODIFY-CLUSTER", &modify, MODIFY_CLUSTER_ROUTE)?;

    register(
        registry,
        "RENAME-CLUSTER",
        &[required_name, new_name],
        RENAME_CLUSTER_ROUTE,
    )?;

    let set = [
        name,
        qualifier("DESCRIPTION", ArgumentKind::Text)?,
        qualifier("ENDPOINT", ArgumentKind::Text)?,
        qualifier("ADMISSION", ArgumentKind::Text)?,
        qualifier("QUORUM", ArgumentKind::Text)?,
        qualifier("DRY_RUN", ArgumentKind::Boolean)?,
        qualifier("CONFIRM", ArgumentKind::Boolean)?,
    ];
    register(registry, "SET-CLUSTER", &set, SET_CLUSTER_ROUTE)?;
    register(registry, "USE-CLUSTER", &[required_name], USE_CLUSTER_ROUTE)?;

    let invite_node = [
        node,
        qualifier("EXPIRATION", ArgumentKind::Integer)?,
        qualifier("ROLE", ArgumentKind::Text)?,
        qualifier("ENDPOINT", ArgumentKind::Text)?,
        qualifier("FINGERPRINT", ArgumentKind::Text)?,
        qualifier("ATTESTATION", ArgumentKind::Text)?,
    ];
    register(registry, "INVITE-NODE", &invite_node, INVITE_NODE_ROUTE)?;
    let admission_node = [
        node,
        qualifier("INVITATION", ArgumentKind::Text)?,
        qualifier("CONFIRM", ArgumentKind::Boolean)?,
    ];
    register(registry, "ACCEPT-NODE", &admission_node, ACCEPT_NODE_ROUTE)?;
    register(registry, "REJECT-NODE", &admission_node, REJECT_NODE_ROUTE)?;

    let remove_node = [
        node,
        qualifier("DRAIN", ArgumentKind::Boolean)?,
        qualifier("FORCE", ArgumentKind::Boolean)?,
        qualifier("CONFIRM", ArgumentKind::Boolean)?,
        qualifier("RECONCILE", ArgumentKind::Boolean)?,
    ];
    register(registry, "REMOVE-NODE", &remove_node, REMOVE_NODE_ROUTE)?;
    register(
        registry,
        "DRAIN-NODE",
        &[
            node,
            qualifier("TIMEOUT", ArgumentKind::Integer)?,
            qualifier("FORCE", ArgumentKind::Boolean)?,
        ],
        DRAIN_NODE_ROUTE,
    )?;
    register(
        registry,
        "FENCE-NODE",
        &[
            node,
            qualifier("FORCE", ArgumentKind::Boolean)?,
            qualifier("CONFIRM", ArgumentKind::Boolean)?,
        ],
        FENCE_NODE_ROUTE,
    )?;
    register(
        registry,
        "REJOIN-NODE",
        &[
            node,
            qualifier("ENDPOINT", ArgumentKind::Text)?,
            qualifier("INVITATION", ArgumentKind::Text)?,
            qualifier("TOKEN", ArgumentKind::Text)?,
            qualifier("ATTESTATION", ArgumentKind::Text)?,
            qualifier("TIMEOUT", ArgumentKind::Integer)?,
        ],
        REJOIN_NODE_ROUTE,
    )?;

    let cluster_admission = [
        required_name,
        qualifier("INVITATION", ArgumentKind::Text)?,
        qualifier("CONFIRM", ArgumentKind::Boolean)?,
    ];
    register(
        registry,
        "INVITE-CLUSTER",
        &[
            required_name,
            qualifier("EXPIRATION", ArgumentKind::Integer)?,
            qualifier("SCOPE", ArgumentKind::Text)?,
        ],
        INVITE_CLUSTER_ROUTE,
    )?;
    register(
        registry,
        "ACCEPT-CLUSTER",
        &cluster_admission,
        ACCEPT_CLUSTER_ROUTE,
    )?;
    register(
        registry,
        "REJECT-CLUSTER",
        &cluster_admission,
        REJECT_CLUSTER_ROUTE,
    )?;
    register(
        registry,
        "REMOVE-FEDERATION",
        &[
            required_name,
            qualifier("FORCE", ArgumentKind::Boolean)?,
            qualifier("CONFIRM", ArgumentKind::Boolean)?,
        ],
        REMOVE_FEDERATION_ROUTE,
    )?;

    if include_help {
        let command = positional("COMMAND", ArgumentKind::Text, false)?;
        register(registry, "HELP", &[command], HELP_ROUTE)?;
    }
    Ok(())
}

fn register<const CAPACITY: usize>(
    registry: &mut CommandRegistry<CAPACITY>,
    name: &str,
    arguments: &[ArgumentSpec],
    route: u16,
) -> Result<(), Error> {
    registry.register(
        CommandSpec::new(name, arguments).map_err(|_| Error::InvalidValue)?,
        RouteId::new(route).expect("cluster route is non-zero"),
    )
}

fn positional(name: &str, kind: ArgumentKind, required: bool) -> Result<ArgumentSpec, Error> {
    ArgumentSpec::new(name, kind, required, true).map_err(|_| Error::InvalidValue)
}

fn qualifier(name: &str, kind: ArgumentKind) -> Result<ArgumentSpec, Error> {
    ArgumentSpec::new(name, kind, false, false).map_err(|_| Error::InvalidValue)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterViewKind {
    Members,
    Topology,
    Health,
    Resources,
    Config,
    Federated,
}

pub trait ClusterSource {
    fn show_cluster(&mut self, _name: Option<&str>) -> Result<ClusterSnapshot, Status> {
        Err(Status::NOT_FOUND)
    }

    fn show_cluster_members(&mut self, _name: Option<&str>) -> Result<ClusterMembersView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn show_cluster_topology(
        &mut self,
        _name: Option<&str>,
    ) -> Result<ClusterTopologyView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn show_cluster_health(&mut self, _name: Option<&str>) -> Result<ClusterHealthView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn show_cluster_resources(
        &mut self,
        _name: Option<&str>,
    ) -> Result<ClusterResourcesView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn show_cluster_config(&mut self, _name: Option<&str>) -> Result<ClusterConfigView, Status> {
        Err(Status::NOT_FOUND)
    }

    fn list_clusters(&mut self, query: ClusterListQuery) -> Result<ClusterListView, Status> {
        Ok(ClusterListView {
            generation: 0,
            total_count: 0,
            page: query.page,
            clusters: [None; MAX_CLUSTER_LIST_ROWS],
            next_page: None,
        })
    }

    /// Execute a mutating cluster command. A provider should perform the
    /// membership, lease, workload, storage, and reconciliation checks here.
    fn execute_cluster_operation(
        &mut self,
        command: CommandCall,
        pipeline_input: Option<&StructuredOutput>,
    ) -> Result<StructuredOutput, Status> {
        acknowledged_output(command, pipeline_input.is_some())
    }

    fn execute_cluster(
        &mut self,
        command: CommandCall,
        pipeline_input: Option<&StructuredOutput>,
    ) -> Result<StructuredOutput, Status> {
        if command.route.raw() == SHOW_CLUSTER_ROUTE {
            let name = command.get_text("NAME");
            return match selected_cluster_view(command) {
                None => show_cluster_output(self.show_cluster(name)?),
                Some(ClusterViewKind::Members) => {
                    cluster_members_output(self.show_cluster_members(name)?)
                }
                Some(ClusterViewKind::Topology) => {
                    cluster_topology_output(self.show_cluster_topology(name)?)
                }
                Some(ClusterViewKind::Health) => {
                    cluster_health_output(self.show_cluster_health(name)?)
                }
                Some(ClusterViewKind::Resources) => {
                    cluster_resources_output(self.show_cluster_resources(name)?)
                }
                Some(ClusterViewKind::Config) => {
                    cluster_config_output(self.show_cluster_config(name)?)
                }
                Some(ClusterViewKind::Federated) => Err(Status::NOT_FOUND),
            };
        }
        if command.route.raw() == LIST_CLUSTERS_ROUTE {
            return cluster_list_output(self.list_clusters(cluster_list_query(command)?)?);
        }
        self.execute_cluster_operation(command, pipeline_input)
    }
}

pub struct ClusterExecutor<Source, const CAPACITY: usize = 16> {
    source: Source,
    completions: [Option<Result<StructuredOutput, Status>>; CAPACITY],
}

impl<Source, const CAPACITY: usize> ClusterExecutor<Source, CAPACITY> {
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

impl<Source: ClusterSource, const CAPACITY: usize> CommandExecutor
    for ClusterExecutor<Source, CAPACITY>
{
    fn submit(
        &mut self,
        command: CommandCall,
        pipeline_input: Option<&StructuredOutput>,
    ) -> Result<ExecutionToken, Error> {
        let slot = self
            .completions
            .iter()
            .position(Option::is_none)
            .ok_or(Error::Capacity)?;
        let completion = if command.route.raw() == HELP_ROUTE {
            help_output(command)
        } else {
            match validate(command) {
                Ok(()) => self.source.execute_cluster(command, pipeline_input),
                Err(status) => Err(status),
            }
        };
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

pub fn validate(command: CommandCall) -> Result<(), Status> {
    if command.route.raw() != HELP_ROUTE
        && !(SHOW_CLUSTER_ROUTE..=REMOVE_FEDERATION_ROUTE).contains(&command.route.raw())
    {
        return Err(Status::NOT_FOUND);
    }
    if command.route.raw() == SHOW_CLUSTER_ROUTE {
        let views = [
            "MEMBERS",
            "TOPOLOGY",
            "HEALTH",
            "RESOURCES",
            "CONFIG",
            "FEDERATED",
        ]
        .iter()
        .filter(|name| {
            command
                .get(name)
                .is_some_and(|value| value == Value::Boolean(true))
        })
        .count();
        if views > 1 {
            return Err(Status::INVALID_ARGUMENT);
        }
    }
    if command.route.raw() == LIST_CLUSTERS_ROUTE {
        let _ = cluster_list_query(command)?;
    }
    if matches!(
        command.route.raw(),
        LEAVE_CLUSTER_ROUTE
            | DELETE_CLUSTER_ROUTE
            | REMOVE_NODE_ROUTE
            | FENCE_NODE_ROUTE
            | REMOVE_FEDERATION_ROUTE
    ) && command.get("CONFIRM") != Some(Value::Boolean(true))
    {
        return Err(Status::INVALID_ARGUMENT);
    }
    if matches!(
        command.route.raw(),
        LEAVE_CLUSTER_ROUTE | REMOVE_NODE_ROUTE | FENCE_NODE_ROUTE
    ) && command.get("FORCE") == Some(Value::Boolean(true))
        && command.get("CONFIRM") != Some(Value::Boolean(true))
    {
        return Err(Status::INVALID_ARGUMENT);
    }
    Ok(())
}

fn cluster_list_query(command: CommandCall) -> Result<ClusterListQuery, Status> {
    let filter = match command.get("NAME") {
        Some(Value::Text(value)) if !value.is_empty() => Some(value),
        Some(Value::Text(_)) => return Err(Status::INVALID_ARGUMENT),
        Some(_) => return Err(Status::INVALID_ARGUMENT),
        None => None,
    };
    let status = match command.get("STATUS") {
        Some(Value::Text(value)) if !value.is_empty() => Some(value),
        Some(Value::Text(_)) => return Err(Status::INVALID_ARGUMENT),
        Some(_) => return Err(Status::INVALID_ARGUMENT),
        None => None,
    };
    let limit = match command.get("LIMIT") {
        Some(Value::Integer(value)) if value > 0 => value as u64,
        Some(Value::Integer(_)) => return Err(Status::INVALID_ARGUMENT),
        Some(_) => return Err(Status::INVALID_ARGUMENT),
        None => MAX_CLUSTER_LIST_ROWS as u64,
    };
    let page = match command.get("PAGE") {
        Some(Value::Integer(value)) if value > 0 => value as u64,
        Some(Value::Integer(_)) => return Err(Status::INVALID_ARGUMENT),
        Some(_) => return Err(Status::INVALID_ARGUMENT),
        None => 1,
    };
    Ok(ClusterListQuery {
        filter,
        status,
        limit,
        page,
        trusted: boolean_filter(command.get("TRUSTED"))?,
        joined: boolean_filter(command.get("JOINED"))?,
        available: boolean_filter(command.get("AVAILABLE"))?,
        degraded: boolean_filter(command.get("DEGRADED"))?,
        federated: boolean_filter(command.get("FEDERATED"))?,
    })
}

fn boolean_filter(value: Option<Value>) -> Result<Option<bool>, Status> {
    match value {
        Some(Value::Boolean(value)) => Ok(Some(value)),
        None => Ok(None),
        Some(_) => Err(Status::INVALID_ARGUMENT),
    }
}

fn selected_cluster_view(command: CommandCall) -> Option<ClusterViewKind> {
    if command.get("MEMBERS") == Some(Value::Boolean(true)) {
        Some(ClusterViewKind::Members)
    } else if command.get("TOPOLOGY") == Some(Value::Boolean(true)) {
        Some(ClusterViewKind::Topology)
    } else if command.get("HEALTH") == Some(Value::Boolean(true)) {
        Some(ClusterViewKind::Health)
    } else if command.get("RESOURCES") == Some(Value::Boolean(true)) {
        Some(ClusterViewKind::Resources)
    } else if command.get("CONFIG") == Some(Value::Boolean(true)) {
        Some(ClusterViewKind::Config)
    } else if command.get("FEDERATED") == Some(Value::Boolean(true)) {
        Some(ClusterViewKind::Federated)
    } else {
        None
    }
}

pub fn cluster_list_output(view: ClusterListView) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "list-clusters")?;
    insert(
        &mut output,
        "total-count",
        OutputValue::Unsigned(view.total_count),
    )?;
    insert(&mut output, "page", OutputValue::Unsigned(view.page))?;
    for (index, cluster) in view.clusters.iter().flatten().enumerate() {
        insert_indexed_text(&mut output, "cluster", index, "id", cluster.cluster_id.as_str())?;
        insert_indexed_text(
            &mut output,
            "cluster",
            index,
            "name",
            cluster.cluster_name.as_str(),
        )?;
        insert_indexed_text(&mut output, "cluster", index, "status", cluster.status.as_str())?;
        insert_indexed(
            &mut output,
            "cluster",
            index,
            "trusted",
            OutputValue::Boolean(cluster.trusted),
        )?;
        insert_indexed(
            &mut output,
            "cluster",
            index,
            "joined",
            OutputValue::Boolean(cluster.joined),
        )?;
        insert_indexed(
            &mut output,
            "cluster",
            index,
            "available",
            OutputValue::Boolean(cluster.available),
        )?;
        insert_indexed(
            &mut output,
            "cluster",
            index,
            "federated",
            OutputValue::Boolean(cluster.federated),
        )?;
    }
    if let Some(next_page) = view.next_page {
        insert(&mut output, "next-page", OutputValue::Unsigned(next_page))?;
    }
    Ok(output)
}

pub fn execute_cluster_surface_command(
    command: CommandCall,
    pipeline_input: Option<&StructuredOutput>,
) -> Result<StructuredOutput, Status> {
    validate(command)?;
    if command.route.raw() == LIST_CLUSTERS_ROUTE {
        let query = cluster_list_query(command)?;
        return cluster_list_output(ClusterListView {
            generation: 0,
            total_count: 0,
            page: query.page,
            clusters: [None; MAX_CLUSTER_LIST_ROWS],
            next_page: None,
        });
    }
    acknowledged_output(command, pipeline_input.is_some())
}

pub fn show_cluster_output(snapshot: ClusterSnapshot) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "show-cluster")?;
    insert_text(&mut output, "cluster-id", snapshot.cluster_id.as_str())?;
    insert_text(&mut output, "cluster-name", snapshot.cluster_name.as_str())?;
    insert_text(&mut output, "status", snapshot.status.as_str())?;
    insert_text(&mut output, "leader", snapshot.leader.as_str())?;
    insert_text(&mut output, "coordinator", snapshot.coordinator.as_str())?;
    insert(
        &mut output,
        "generation",
        OutputValue::Unsigned(snapshot.generation),
    )?;
    insert(
        &mut output,
        "sampled-at-us",
        OutputValue::Unsigned(snapshot.sampled_at_us),
    )?;
    insert(
        &mut output,
        "member-count",
        OutputValue::Unsigned(snapshot.member_count),
    )?;
    insert(
        &mut output,
        "healthy-members",
        OutputValue::Unsigned(snapshot.healthy_members),
    )?;
    insert(
        &mut output,
        "degraded-members",
        OutputValue::Unsigned(snapshot.degraded_members),
    )?;
    insert(
        &mut output,
        "failed-members",
        OutputValue::Unsigned(snapshot.failed_members),
    )?;
    insert(
        &mut output,
        "voting-members",
        OutputValue::Unsigned(snapshot.voting_members),
    )?;
    insert(
        &mut output,
        "quorum-required",
        OutputValue::Unsigned(snapshot.quorum_required),
    )?;
    insert(
        &mut output,
        "quorum-available",
        OutputValue::Unsigned(snapshot.quorum_available),
    )?;
    insert(
        &mut output,
        "quorum",
        OutputValue::Boolean(snapshot.quorum_available >= snapshot.quorum_required),
    )?;
    insert_text(&mut output, "health", snapshot.health.as_str())?;
    insert(
        &mut output,
        "cpu-capacity",
        OutputValue::Unsigned(snapshot.cpu_capacity),
    )?;
    insert(
        &mut output,
        "cpu-available",
        OutputValue::Unsigned(snapshot.cpu_available),
    )?;
    insert(
        &mut output,
        "memory-capacity-bytes",
        OutputValue::Unsigned(snapshot.memory_capacity_bytes),
    )?;
    insert(
        &mut output,
        "memory-available-bytes",
        OutputValue::Unsigned(snapshot.memory_available_bytes),
    )?;
    insert(
        &mut output,
        "cxl-capacity-bytes",
        OutputValue::Unsigned(snapshot.cxl_capacity_bytes),
    )?;
    insert(
        &mut output,
        "vram-capacity-bytes",
        OutputValue::Unsigned(snapshot.vram_capacity_bytes),
    )?;
    insert(
        &mut output,
        "storage-capacity-bytes",
        OutputValue::Unsigned(snapshot.storage_capacity_bytes),
    )?;
    insert(
        &mut output,
        "control-protocol-version",
        OutputValue::Unsigned(snapshot.control_protocol_version),
    )?;
    insert(
        &mut output,
        "data-protocol-version",
        OutputValue::Unsigned(snapshot.data_protocol_version),
    )?;
    insert(
        &mut output,
        "minimum-protocol-version",
        OutputValue::Unsigned(snapshot.minimum_protocol_version),
    )?;
    Ok(output)
}

pub fn cluster_members_output(view: ClusterMembersView) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "show-cluster-members")?;
    insert(
        &mut output,
        "generation",
        OutputValue::Unsigned(view.generation),
    )?;
    insert(
        &mut output,
        "member-count",
        OutputValue::Unsigned(view.member_count),
    )?;
    insert(
        &mut output,
        "voting-members",
        OutputValue::Unsigned(view.voting_members),
    )?;
    insert(
        &mut output,
        "healthy-members",
        OutputValue::Unsigned(view.healthy_members),
    )?;
    insert(
        &mut output,
        "degraded-members",
        OutputValue::Unsigned(view.degraded_members),
    )?;
    insert(
        &mut output,
        "failed-members",
        OutputValue::Unsigned(view.failed_members),
    )?;
    for (index, member) in view.members.iter().flatten().enumerate() {
        insert_indexed_text(&mut output, "member", index, "node", member.node.as_str())?;
        insert_indexed_text(&mut output, "member", index, "role", member.role.as_str())?;
        insert_indexed_text(
            &mut output,
            "member",
            index,
            "status",
            member.status.as_str(),
        )?;
        insert_indexed(
            &mut output,
            "member",
            index,
            "voting",
            OutputValue::Boolean(member.voting),
        )?;
    }
    if let Some(next) = view.next_member {
        insert(&mut output, "next-member", OutputValue::Unsigned(next))?;
    }
    Ok(output)
}

pub fn cluster_topology_output(view: ClusterTopologyView) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "show-cluster-topology")?;
    insert(
        &mut output,
        "generation",
        OutputValue::Unsigned(view.generation),
    )?;
    insert_text(&mut output, "topology", view.topology.as_str())?;
    insert(
        &mut output,
        "link-count",
        OutputValue::Unsigned(view.link_count),
    )?;
    insert(
        &mut output,
        "reachable-links",
        OutputValue::Unsigned(view.reachable_links),
    )?;
    insert(
        &mut output,
        "failed-links",
        OutputValue::Unsigned(view.failed_links),
    )?;
    insert(
        &mut output,
        "maximum-latency-us",
        OutputValue::Unsigned(view.maximum_latency_us),
    )?;
    for (index, link) in view.links.iter().flatten().enumerate() {
        insert_indexed_text(&mut output, "link", index, "from", link.from.as_str())?;
        insert_indexed_text(&mut output, "link", index, "to", link.to.as_str())?;
        insert_indexed_text(
            &mut output,
            "link",
            index,
            "transport",
            link.transport.as_str(),
        )?;
        insert_indexed_text(&mut output, "link", index, "route", link.route.as_str())?;
        insert_indexed_text(&mut output, "link", index, "zone", link.zone.as_str())?;
        insert_indexed_text(&mut output, "link", index, "status", link.status.as_str())?;
        insert_indexed(
            &mut output,
            "link",
            index,
            "latency-us",
            OutputValue::Unsigned(link.latency_us),
        )?;
        insert_indexed(
            &mut output,
            "link",
            index,
            "bandwidth-mbps",
            OutputValue::Unsigned(link.bandwidth_mbps),
        )?;
        insert_indexed(
            &mut output,
            "link",
            index,
            "mtu",
            OutputValue::Unsigned(link.mtu),
        )?;
    }
    if let Some(next) = view.next_link {
        insert(&mut output, "next-link", OutputValue::Unsigned(next))?;
    }
    Ok(output)
}

pub fn cluster_health_output(view: ClusterHealthView) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "show-cluster-health")?;
    insert(
        &mut output,
        "generation",
        OutputValue::Unsigned(view.generation),
    )?;
    insert_text(&mut output, "health", view.health.as_str())?;
    insert_text(&mut output, "status", view.status.as_str())?;
    insert(
        &mut output,
        "heartbeat-period-us",
        OutputValue::Unsigned(view.heartbeat_period_us),
    )?;
    insert(
        &mut output,
        "missed-heartbeat-limit",
        OutputValue::Unsigned(view.missed_heartbeat_limit),
    )?;
    insert(
        &mut output,
        "last-change-us",
        OutputValue::Unsigned(view.last_change_us),
    )?;
    insert(
        &mut output,
        "healthy-nodes",
        OutputValue::Unsigned(view.healthy_nodes),
    )?;
    insert(
        &mut output,
        "degraded-nodes",
        OutputValue::Unsigned(view.degraded_nodes),
    )?;
    insert(
        &mut output,
        "failed-nodes",
        OutputValue::Unsigned(view.failed_nodes),
    )?;
    insert(&mut output, "quorum", OutputValue::Boolean(view.quorum))?;
    Ok(output)
}

pub fn cluster_resources_output(view: ClusterResourcesView) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "show-cluster-resources")?;
    insert(
        &mut output,
        "generation",
        OutputValue::Unsigned(view.generation),
    )?;
    insert(
        &mut output,
        "cpu-capacity",
        OutputValue::Unsigned(view.cpu_capacity),
    )?;
    insert(
        &mut output,
        "cpu-available",
        OutputValue::Unsigned(view.cpu_available),
    )?;
    insert(
        &mut output,
        "memory-capacity-bytes",
        OutputValue::Unsigned(view.memory_capacity_bytes),
    )?;
    insert(
        &mut output,
        "memory-available-bytes",
        OutputValue::Unsigned(view.memory_available_bytes),
    )?;
    insert(
        &mut output,
        "cxl-capacity-bytes",
        OutputValue::Unsigned(view.cxl_capacity_bytes),
    )?;
    insert(
        &mut output,
        "vram-capacity-bytes",
        OutputValue::Unsigned(view.vram_capacity_bytes),
    )?;
    insert(
        &mut output,
        "storage-capacity-bytes",
        OutputValue::Unsigned(view.storage_capacity_bytes),
    )?;
    insert(
        &mut output,
        "network-bandwidth-mbps",
        OutputValue::Unsigned(view.network_bandwidth_mbps),
    )?;
    insert(
        &mut output,
        "active-leases",
        OutputValue::Unsigned(view.active_leases),
    )?;
    insert(
        &mut output,
        "running-workloads",
        OutputValue::Unsigned(view.running_workloads),
    )?;
    insert(
        &mut output,
        "queued-workloads",
        OutputValue::Unsigned(view.queued_workloads),
    )?;
    Ok(output)
}

pub fn cluster_config_output(view: ClusterConfigView) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "show-cluster-config")?;
    insert(
        &mut output,
        "generation",
        OutputValue::Unsigned(view.generation),
    )?;
    insert_text(
        &mut output,
        "admission-policy",
        view.admission_policy.as_str(),
    )?;
    insert_text(&mut output, "quorum-policy", view.quorum_policy.as_str())?;
    insert_text(
        &mut output,
        "discovery-policy",
        view.discovery_policy.as_str(),
    )?;
    insert_text(&mut output, "endpoint", view.endpoint.as_str())?;
    insert(
        &mut output,
        "heartbeat-period-us",
        OutputValue::Unsigned(view.heartbeat_period_us),
    )?;
    insert(
        &mut output,
        "missed-heartbeat-limit",
        OutputValue::Unsigned(view.missed_heartbeat_limit),
    )?;
    insert(
        &mut output,
        "control-protocol-version",
        OutputValue::Unsigned(view.control_protocol_version),
    )?;
    insert(
        &mut output,
        "data-protocol-version",
        OutputValue::Unsigned(view.data_protocol_version),
    )?;
    insert(
        &mut output,
        "minimum-protocol-version",
        OutputValue::Unsigned(view.minimum_protocol_version),
    )?;
    insert(&mut output, "staged", OutputValue::Boolean(view.staged))?;
    Ok(output)
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
        OutputValue::Text(OutputText::new(value).map_err(|_| Status::NO_SPACE)?),
    )
}

fn insert_indexed(
    output: &mut StructuredOutput,
    prefix: &str,
    index: usize,
    suffix: &str,
    value: OutputValue,
) -> Result<(), Status> {
    let mut name = crate::Text::<32>::empty();
    name.push_str(prefix).map_err(|_| Status::NO_SPACE)?;
    let mut digits = [0; 10];
    let mut count = 0;
    let mut number = (index + 1) as u64;
    loop {
        digits[count] = b'0' + (number % 10) as u8;
        count += 1;
        number /= 10;
        if number == 0 {
            break;
        }
    }
    while count != 0 {
        count -= 1;
        name.push_char(digits[count] as char)
            .map_err(|_| Status::NO_SPACE)?;
    }
    name.push_char('-').map_err(|_| Status::NO_SPACE)?;
    name.push_str(suffix).map_err(|_| Status::NO_SPACE)?;
    output
        .insert(name.as_str(), value)
        .map_err(|_| Status::NO_SPACE)
}

pub fn acknowledged_output(
    command: CommandCall,
    pipeline_input: bool,
) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", operation_name(command.route.raw()))?;
    insert_text(&mut output, "command", command.command.as_str())?;
    insert(
        &mut output,
        "route",
        OutputValue::Unsigned(command.route.raw() as u64),
    )?;
    insert(&mut output, "accepted", OutputValue::Boolean(true))?;
    insert(
        &mut output,
        "pipeline-input",
        OutputValue::Boolean(pipeline_input),
    )?;
    for name in ["NAME", "NEW_NAME", "NODE"] {
        if let Some(Value::Text(value)) = command.get(name) {
            insert_text(&mut output, "target", value.as_str())?;
            break;
        }
    }
    for (argument, field) in [
        ("ID", "cluster-id"),
        ("DESCRIPTION", "description"),
        ("ENDPOINT", "endpoint"),
        ("ADMISSION", "admission-policy"),
        ("QUORUM", "quorum-policy"),
        ("ADMIN", "administrator"),
        ("INVITATION", "invitation"),
        ("TOKEN", "token"),
        ("FINGERPRINT", "fingerprint"),
        ("ATTESTATION", "attestation"),
        ("ROLE", "role"),
        ("SCOPE", "scope"),
    ] {
        if let Some(Value::Text(value)) = command.get(argument) {
            insert_text(&mut output, field, value.as_str())?;
        }
    }
    for (argument, field) in [("EXPIRATION", "expiration"), ("TIMEOUT", "timeout")] {
        if let Some(Value::Integer(value)) = command.get(argument) {
            insert(&mut output, field, OutputValue::Integer(value))?;
        }
    }
    for (argument, field) in [
        ("CONFIRM", "confirmed"),
        ("DRAIN", "drain"),
        ("FORCE", "force"),
        ("RECONCILE", "reconcile"),
        ("DRY_RUN", "dry-run"),
        ("APPROVE", "approved"),
    ] {
        if let Some(Value::Boolean(value)) = command.get(argument) {
            insert(&mut output, field, OutputValue::Boolean(value))?;
        }
    }
    Ok(output)
}

fn operation_name(route: u16) -> &'static str {
    match route {
        CREATE_CLUSTER_ROUTE => "create-cluster",
        JOIN_CLUSTER_ROUTE => "join-cluster",
        LEAVE_CLUSTER_ROUTE => "leave-cluster",
        DELETE_CLUSTER_ROUTE => "delete-cluster",
        MODIFY_CLUSTER_ROUTE => "modify-cluster",
        RENAME_CLUSTER_ROUTE => "rename-cluster",
        SET_CLUSTER_ROUTE => "set-cluster",
        USE_CLUSTER_ROUTE => "use-cluster",
        INVITE_NODE_ROUTE => "invite-node",
        ACCEPT_NODE_ROUTE => "accept-node",
        REJECT_NODE_ROUTE => "reject-node",
        REMOVE_NODE_ROUTE => "remove-node",
        DRAIN_NODE_ROUTE => "drain-node",
        FENCE_NODE_ROUTE => "fence-node",
        REJOIN_NODE_ROUTE => "rejoin-node",
        INVITE_CLUSTER_ROUTE => "invite-cluster",
        ACCEPT_CLUSTER_ROUTE => "accept-cluster",
        REJECT_CLUSTER_ROUTE => "reject-cluster",
        REMOVE_FEDERATION_ROUTE => "remove-federation",
        _ => "cluster-command",
    }
}

fn help_output(command: CommandCall) -> Result<StructuredOutput, Status> {
    let mut output = StructuredOutput::new(Status::NORMAL);
    insert_text(&mut output, "operation", "help")?;
    if let Some(target) = command.get_text("COMMAND") {
        let help = command_help(target).ok_or(Status::NOT_FOUND)?;
        insert_text(&mut output, "command", help.name)?;
        insert_text(&mut output, "synopsis", help.synopsis)?;
        insert_text(&mut output, "description", help.description)?;
        insert_text(&mut output, "aliases", help.aliases)?;
        insert_text(&mut output, "qualifiers", help.qualifiers)?;
    } else {
        insert(
            &mut output,
            "command-count",
            OutputValue::Unsigned(CLUSTER_COMMAND_COUNT as u64),
        )?;
        for (index, help) in CLUSTER_COMMAND_HELP.iter().enumerate() {
            let mut field = [0u8; 12];
            let mut field_len = 0;
            for byte in b"command-" {
                field[field_len] = *byte;
                field_len += 1;
            }
            let mut number = (index + 1) as u32;
            let mut digits = [0u8; 3];
            let mut digit_count = 0;
            loop {
                digits[digit_count] = b'0' + (number % 10) as u8;
                digit_count += 1;
                number /= 10;
                if number == 0 {
                    break;
                }
            }
            while digit_count != 0 {
                digit_count -= 1;
                field[field_len] = digits[digit_count];
                field_len += 1;
            }
            let field = core::str::from_utf8(&field[..field_len]).expect("help field invariant");
            insert_text(&mut output, field, help.name)?;
        }
    }
    Ok(output)
}

fn insert_text(output: &mut StructuredOutput, name: &str, value: &str) -> Result<(), Status> {
    insert(
        output,
        name,
        OutputValue::Text(OutputText::new(value).map_err(|_| Status::NO_SPACE)?),
    )
}

fn insert(output: &mut StructuredOutput, name: &str, value: OutputValue) -> Result<(), Status> {
    output.insert(name, value).map_err(|_| Status::NO_SPACE)
}
