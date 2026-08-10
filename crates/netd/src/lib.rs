#![no_std]
#![forbid(unsafe_code)]

mod dhcp;
mod capture;
mod memory;
mod firewall;
mod packet;
mod protocol;
mod service;
mod stack;
mod scheduler;
mod transport;

pub use memory::{MappedRegion, MemoryError, SharedMemory};
pub use capture::{CaptureDirection, CaptureKind, CaptureRecord, PacketCapture, MAX_CAPTURE_BYTES,
    MAX_CAPTURE_RECORDS};
pub use dhcp::{
    dhcp_client_firewall_rules, format_ipv4, install_dhcp_client_rules, BACKOFF_MS,
    DHCP_CLIENT_PORT, DHCP_MAGIC_COOKIE, DHCP_SERVER_PORT, MAX_DHCP_DNS_SERVERS, MAX_DHCP_PACKET,
    MAX_DHCP_ROUTES, MAX_DISCOVER_ATTEMPTS, MAX_INTERFACE_NAME, DhcpClient, DhcpClientState,
    CapturingDhcpTransport, DhcpClientView, DhcpError, DhcpLease, DhcpLeaseRuntime, DhcpMessageType,
    DhcpNetworkError, DhcpOffer,
    DhcpServerFixture, DhcpTransport, StaticSnapshot,
};
pub use firewall::{
    CapabilityKey, CapabilityRight, Direction, Firewall, FirewallDecision, FirewallError,
    FirewallPolicy, FirewallRule, FirewallSnapshot, Ipv4Cidr, NetworkCapability, PacketContext,
    PacketSignature, PacketView, PortRange, Protocol, RateLimit, RuleAction, SignedHeader,
    PolicyImage, PolicyStore, POLICY_PATH,
};
pub use packet::{PacketError, PacketQueue, PacketReader, PacketWriter, QueueDevice};
pub use protocol::{
    SOCKET_PROTOCOL_VERSION, SOCKET_REQUEST_SCHEMA, SOCKET_RESPONSE_SCHEMA, SocketOperation,
    SocketRequest, SocketResponse, socket_response,
};
pub use service::{
    ClientChannel, DefaultFirewall, NetworkDaemon, ServiceError, SocketBackend, SocketCapability,
    SocketRights, SocketState,
};
pub use scheduler::{
    NetworkServiceActivity, NetworkServiceScheduler, DEFAULT_SOCKET_INGRESS_BUDGET,
    DEFAULT_SOCKET_REQUEST_BUDGET,
};
pub use stack::{NetworkPoller, PollActivity, SmolTcpStack, TcpBuffers, TcpHandle};
pub use transport::{
    DhcpIngress, EthernetDhcpTransport, DHCP_BROADCAST_IPV4, DHCP_BROADCAST_MAC,
    DHCP_MIN_ETHERNET_FRAME, DHCP_UNSPECIFIED_IPV4,
};
