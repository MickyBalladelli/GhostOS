//! Networking support shared by the emulated NICs.
//!
//! Provides packet buffer management, MAC address handling, the pluggable
//! [`NetBackend`] a NIC transmits into / receives from, and deterministic
//! loopback/shared-segment fixtures.

pub mod backend;
pub mod dhcp;
pub mod mac;
pub mod packet;

unsafe extern "C" {
    fn ghostos_vm_net_align_up(value: u64, alignment: u64) -> u64;
}

pub use backend::{
    DeterministicPort, DeterministicSegment, HostNetworkBackend, LoopbackHub, LoopbackPort,
    NetBackend, NetQueueState, NetworkBackendConfig,
};
pub use dhcp::{
    DhcpConfigError, DhcpLeaseInfo, DhcpReservation, DhcpServerConfig,
    DeterministicVmNetwork,
    DeterministicDhcpServer, DHCP_CLIENT_PORT as VM_DHCP_CLIENT_PORT,
    DHCP_SERVER_MAC, DHCP_SERVER_PORT as VM_DHCP_SERVER_PORT,
};
pub use mac::{mac_matches, MacAddress, MAC_ADDRESS_LEN};
pub use packet::{NetError, PacketQueue, ETHERNET_FRAME_MAX, ETHERNET_FRAME_MIN, ETHERNET_HEADER_LEN};

/// Round `value` up to a multiple of `alignment` (power of two).
pub fn align_up(value: u64, alignment: u64) -> u64 {
    unsafe { ghostos_vm_net_align_up(value, alignment) }
}
