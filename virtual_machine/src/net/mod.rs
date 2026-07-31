//! Networking support shared by the emulated NICs.
//!
//! Provides packet buffer management, MAC address handling, the pluggable
//! [`NetBackend`] a NIC transmits into / receives from, and a two-port
//! loopback hub connecting the emulated e1000 and virtio-net devices.

pub mod backend;
pub mod mac;
pub mod packet;

pub use backend::{LoopbackHub, LoopbackPort, NetBackend};
pub use mac::{mac_matches, MacAddress, MAC_ADDRESS_LEN};
pub use packet::{NetError, PacketQueue, ETHERNET_FRAME_MAX, ETHERNET_FRAME_MIN, ETHERNET_HEADER_LEN};

/// Round `value` up to a multiple of `alignment` (power of two).
pub fn align_up(value: u64, alignment: u64) -> u64 {
    (value + alignment - 1) & !(alignment - 1)
}