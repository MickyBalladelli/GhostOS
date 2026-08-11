#![no_std]
#![forbid(unsafe_code)]

//! Cross-platform SynOS client protocol.

//!
//! The crate contains no sockets, executor, allocator, or platform APIs. A
//! macOS, iOS, Android, or WebAssembly host supplies an [`RpcTransport`] and
//! receives the same versioned binary RPC contract on every platform.

pub use synos_protocol::{
    ProtocolError as TransportProtocolError, ProtocolGuard, ProtocolLimits, TrafficClass,
    VersionRange,
};
pub use synos_abi::{ABI_REVISION, ABI_SCHEMA_VERSION};
pub use synos_system_model::performance::{
    PerformanceBudget, PerformanceDiagnostics, TailLatencyWindow,
    PERFORMANCE_DIAGNOSTICS_VERSION,
};

mod client;
mod cluster;
mod gateway;
mod model;
mod wire;

pub use client::{Client, ClientError, LoanedRpcError, RemoteError, RpcTransport};
pub use cluster::{
    AuditEventList, BoundedText, ChangeBatch, ChangeEvent, ChangeLog, ChangeLogError,
    ClusterAuditEvent, ClusterCreateRequest,
    ClusterHealth, ClusterHealthSnapshot, ClusterId, ClusterInvitation, ClusterJoinRequest,
    ClusterLeaveRequest, ClusterLifecycle, ClusterMember, ClusterName, ClusterRemoveRequest,
    ClusterResources, ClusterSummary, InvitationList, InvitationState, JoinPlan, LeavePlan,
    LifecycleReceipt, MemberList, MemberRole, MemberState, Subscription, SubscriptionKind,
    MAX_CLUSTER_AUDIT_EVENTS, MAX_CLUSTER_CHANGES, MAX_CLUSTER_INVITATIONS,
    MAX_CLUSTER_MEMBERS, MAX_CLUSTER_NAME_BYTES,
};
pub use gateway::{FrontendGateway, GatewayService, NoopPerformanceClock, PerformanceClock};
pub use model::{
    CapabilityDelegation, ClusterNode, ClusterState, JobReceipt, JobSpec, MAX_CLUSTER_NODES,
    MAX_JOB_COMMAND_BYTES, MAX_TOPOLOGY_LINKS, NodeHealth, TopologyLink, TopologyReachability,
    TopologyRoute, TopologyState, TopologyTransport,
};
pub use wire::{
    decode_frame_checked, FRAME_HEADER_BYTES, FrameHeader, MAX_FRAME_BYTES, Method,
    PERFORMANCE_DIAGNOSTICS_BYTES,
    PROTOCOL_VERSION, ProtocolError, RpcStatus,
};

pub use synos_auth::{CryptographicCapability, TransportRights};
pub use synos_fabric::NodeId;
pub use synos_kernel::Rights;
