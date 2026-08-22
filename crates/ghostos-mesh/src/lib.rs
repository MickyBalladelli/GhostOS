#![no_std]
#![forbid(unsafe_code)]

//! Heap-free coordination for edge nodes joining a GhostOS fabric.
//!
//! The mesh deliberately does not own a socket or a scheduler thread. A
//! platform daemon supplies packets and timers, while this crate provides the
//! bounded protocol state, CoW reconciliation, and deterministic placement
//! decisions.

pub use ghostos_protocol::{ProtocolError, ProtocolGuard, ProtocolLimits, TrafficClass, VersionRange};

pub fn validate_mesh_message(
    guard: &mut ProtocolGuard,
    sequence: u64,
    bytes: usize,
) -> Result<(), ProtocolError> {
    guard.require_class(TrafficClass::Mesh)?;
    guard.validate_message(bytes)?;
    guard.accept_sequence(sequence)
}

mod delta;
mod discovery;
mod offload;
mod topology;

pub use delta::{
    CowDelta, CowOperation, DeltaApplyReceipt, DeltaError, DeltaMode, FileFingerprint,
    apply_delta, build_delta,
};
pub use discovery::{
    DiscoveryError, GossipAnnouncement, GossipDiscovery, InterfaceSet, MeshInterface,
    NodeAdvertisement, NodeRole, PeerState, PeerStatus,
};
pub use offload::{
    ComputeTarget, OffloadError, OffloadPlan, OffloadPlanner, OffloadSession, WorkloadChunk,
    WorkloadClass, WorkloadSpec, WorkloadTransport,
};
pub use topology::{
    ClusterAdvertisement, ClusterCapabilities, ClusterId, ConnectivityManager,
    ConnectivityStatus, ConnectionAttempt, Endpoint, EndpointAddress, Reachability, RouteKind,
    TopologyError, TopologyGraph, TopologyLink, TopologyNode, Transport, Zone,
    ADVERTISEMENT_PAYLOAD_BYTES, ADVERTISEMENT_WIRE_BYTES, MAX_DISCOVERY_CACHE,
    MAX_ENDPOINTS, MAX_TOPOLOGY_LINKS, MAX_TOPOLOGY_NODES,
};

use ghostos_status::{IntoStatus, Severity, Status, facility};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Discovery(DiscoveryError),
    Delta(DeltaError),
    Offload(OffloadError),
}

impl From<DiscoveryError> for Error {
    fn from(error: DiscoveryError) -> Self {
        Self::Discovery(error)
    }
}

impl From<DeltaError> for Error {
    fn from(error: DeltaError) -> Self {
        Self::Delta(error)
    }
}

impl From<OffloadError> for Error {
    fn from(error: OffloadError) -> Self {
        Self::Offload(error)
    }
}

impl IntoStatus for Error {
    fn status(self) -> Status {
        match self {
            Self::Discovery(error) => error.status(),
            Self::Delta(error) => error.status(),
            Self::Offload(error) => error.status(),
        }
    }
}

fn mesh_status(code: u16, severity: Severity) -> Status {
    Status::new(severity, facility::NETWORK, code, 0).unwrap_or(Status::INVALID_ARGUMENT)
}
