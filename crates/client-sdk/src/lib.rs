#![no_std]
#![forbid(unsafe_code)]

//! Cross-platform SynOS client protocol.
//!
//! The crate contains no sockets, executor, allocator, or platform APIs. A
//! macOS, iOS, Android, or WebAssembly host supplies an [`RpcTransport`] and
//! receives the same versioned binary RPC contract on every platform.

mod client;
mod gateway;
mod model;
mod wire;

pub use client::{Client, ClientError, RpcTransport};
pub use gateway::{FrontendGateway, GatewayService};
pub use model::{
    CapabilityDelegation, ClusterNode, ClusterState, JobReceipt, JobSpec, MAX_CLUSTER_NODES,
    MAX_JOB_COMMAND_BYTES, NodeHealth,
};
pub use wire::{
    FRAME_HEADER_BYTES, FrameHeader, MAX_FRAME_BYTES, Method, PROTOCOL_VERSION, ProtocolError,
    RpcStatus,
};

pub use synos_auth::{CryptographicCapability, TransportRights};
pub use synos_fabric::NodeId;
pub use synos_kernel::Rights;
