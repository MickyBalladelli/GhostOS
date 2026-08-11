#![no_std]
#![forbid(unsafe_code)]

pub use synos_numa::{NumaCounters, NumaDecision, NumaReport, NumaTopology, NumaTopologyError, PlacementKind, PlacementLocality};

mod dhcp;
mod capture;
mod memory;
mod firewall;
mod packet;
mod protocol;
mod service;
mod stack;
mod scheduler;
mod stats;
mod transport;

pub use memory::{MappedRegion, MemoryError, SharedMemory};
pub use capture::{
    CaptureDirection, CaptureFlowAggregate, CaptureKind, CaptureRecord, DhcpLifecycleEvent,
    PacketCapture, MAX_CAPTURE_BYTES, MAX_CAPTURE_FLOWS, MAX_CAPTURE_RECORDS,
};
pub use dhcp::{
    dhcp_client_firewall_rules, format_ipv4, install_dhcp_client_rules, BACKOFF_MS,
    DHCP_CLIENT_PORT, DHCP_MAGIC_COOKIE, DHCP_SERVER_PORT, MAX_DHCP_DNS_SERVERS, MAX_DHCP_PACKET,
    MAX_DHCP_ROUTES, MAX_DISCOVER_ATTEMPTS, MAX_INTERFACE_NAME, MAX_RETRY_DELAY_MS,
    DHCP_LEASE_RECORD_BYTES, DHCP_LEASE_RECORD_VERSION, RETRY_JITTER_PERCENT, DhcpClient,
    DhcpClientState, DhcpLeaseRecord,
    CapturingDhcpTransport, DhcpClientView, DhcpError, DhcpInterfaceState, DhcpLease,
    DhcpLeaseApplication, DhcpLeaseRuntime, DhcpMessageType, DhcpNetworkError, DhcpOffer,
    DhcpRoute,
    DhcpServerFixture, DhcpTransport, StaticSnapshot,
};
pub use firewall::{
    core_network_firewall_rules, install_core_network_rules, CapabilityKey, CapabilityRight,
    Direction, Firewall, FirewallDecision, FirewallError, FirewallPolicy, FirewallRule,
    FirewallSnapshot, Ipv4Cidr, NetworkCapability, PacketContext, PacketSignature, PacketView,
    PortRange, Protocol, RateLimit, RuleAction, SignedHeader, PolicyImage, PolicyStore,
    MAX_CORE_NETWORK_RULES, POLICY_PATH,
};
pub use packet::{PacketError, PacketQueue, PacketReader, PacketWriter, QueueDevice, QueueMetrics};
pub use protocol::{
    SOCKET_PROTOCOL_VERSION, SOCKET_REQUEST_SCHEMA, SOCKET_RESPONSE_SCHEMA, SocketOperation,
    SocketRequest, SocketResponse, socket_response,
};
pub use service::{
    ClientChannel, DefaultFirewall, NetworkDaemon, NetworkRatePolicy, NetworkShapeDecision,
    NetworkShaper, ServiceError, SocketBackend, SocketCapability, SocketRights, SocketState,
    DEFAULT_NETWORK_TENANT_CAPACITY,
};
pub use scheduler::{
    NetworkServiceActivity, NetworkServiceScheduler, DEFAULT_SOCKET_INGRESS_BUDGET,
    DEFAULT_SOCKET_REQUEST_BUDGET,
};
pub use stats::NetworkStats;
pub use stack::{
    InterfaceConfig, InterfaceConfigError, InterfaceRoute, NeighborEntry, NeighborState,
    NeighborTable, NeighborTableError, IcmpEchoObservation, NetworkPoller, PollActivity,
    SmolTcpStack, TcpBuffers, TcpHandle, MAX_ICMP_ECHO_PAYLOAD, MAX_INTERFACE_ROUTES,
    MAX_NEIGHBOR_ATTEMPTS, MAX_NEIGHBOR_ENTRIES, NEIGHBOR_REACHABLE_MS,
    NEIGHBOR_RESOLUTION_TIMEOUT_MS, NetworkFrameKind, inspect_frame,
};
pub use transport::{
    DhcpIngress, EthernetDhcpTransport, DHCP_BROADCAST_IPV4, DHCP_BROADCAST_MAC,
    DHCP_MIN_ETHERNET_FRAME, DHCP_UNSPECIFIED_IPV4,
};
