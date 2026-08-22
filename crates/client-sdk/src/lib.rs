#![no_std]
#![forbid(unsafe_code)]

//! Cross-platform GhostOS client protocol.

//!
//! The crate contains no sockets, executor, allocator, or platform APIs. A
//! macOS, iOS, Android, or WebAssembly host supplies an [`RpcTransport`] and
//! receives the same versioned binary RPC contract on every platform.

pub use ghostos_protocol::{
    ProtocolError as TransportProtocolError, ProtocolGuard, ProtocolLimits, TrafficClass,
    VersionRange,
};
pub use ghostos_api_compat::{ApiVersion, Compatibility, CompatibilityError};
pub use ghostos_abi::{ABI_REVISION, ABI_SCHEMA_VERSION};
pub use ghostos_system_model::performance::{
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

pub use ghostos_auth::{CryptographicCapability, TransportRights};
pub use ghostos_fabric::NodeId;
pub use ghostos_kernel::Rights;

/// Stable source-level contract shared by the Rust and Swift user-space SDKs.
pub const SDK_API: ghostos_api_compat::ApiContract = ghostos_api_compat::SDK_API;
pub const SDK_API_VERSION: ApiVersion = SDK_API.current;
pub const SDK_MINIMUM_API_VERSION: ApiVersion = SDK_API.supported.minimum;
pub const SDK_MAXIMUM_API_VERSION: ApiVersion = SDK_API.supported.maximum;
