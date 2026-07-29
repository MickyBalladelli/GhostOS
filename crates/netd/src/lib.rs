#![no_std]
#![forbid(unsafe_code)]

mod memory;
mod packet;
mod protocol;
mod service;
mod stack;

pub use memory::{MappedRegion, MemoryError, SharedMemory};
pub use packet::{PacketError, PacketQueue, PacketReader, PacketWriter, QueueDevice};
pub use protocol::{
    SOCKET_PROTOCOL_VERSION, SOCKET_REQUEST_SCHEMA, SOCKET_RESPONSE_SCHEMA, SocketOperation,
    SocketRequest, socket_response,
};
pub use service::{
    ClientChannel, NetworkDaemon, ServiceError, SocketBackend, SocketCapability, SocketRights,
    SocketState,
};
pub use stack::{PollActivity, SmolTcpStack, TcpBuffers, TcpHandle};
