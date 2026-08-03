#![no_std]
#![forbid(unsafe_code)]

mod memory;
mod firewall;
mod packet;
mod protocol;
mod service;
mod stack;

pub use memory::{MappedRegion, MemoryError, SharedMemory};
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
pub use stack::{PollActivity, SmolTcpStack, TcpBuffers, TcpHandle};
