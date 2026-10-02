//! Packet backends: the pluggable [`NetBackend`] trait, isolated loopback,
//! and deterministic shared Ethernet segments.

use crate::net::mac::MacAddress;
use crate::net::packet::{NetError, ETHERNET_FRAME_MAX, ETHERNET_HEADER_LEN};
use crate::net::dhcp::DeterministicVmNetwork;
use std::cell::RefCell;
use std::ffi::c_void;
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::rc::Rc;

#[repr(C)]
struct CHostIo {
    link_up: unsafe extern "C" fn(*mut c_void) -> bool,
    send: unsafe extern "C" fn(*mut c_void, *const u8, usize) -> i32,
    receive: unsafe extern "C" fn(*mut c_void, *mut u8, usize, *mut usize) -> i32,
    context: *mut c_void,
}
unsafe extern "C" {
    fn ghostos_vm_host_net_new(mac: *const MacAddress, udp: bool) -> *mut c_void;
    fn ghostos_vm_host_net_free(state: *mut c_void);
    fn ghostos_vm_host_net_admin(state: *const c_void) -> bool;
    fn ghostos_vm_host_net_set_admin(state: *mut c_void, up: bool);
    fn ghostos_vm_host_net_set_promiscuous(state: *mut c_void, enabled: bool);
    fn ghostos_vm_host_net_transmitted(state: *const c_void) -> usize;
    fn ghostos_vm_host_net_transmit(state: *mut c_void, io: *const CHostIo, bytes: *const u8, length: usize) -> i32;
    fn ghostos_vm_host_net_receive(state: *mut c_void, io: *const CHostIo, bytes: *mut u8, length: *mut usize) -> i32;
}

/// Host-facing network selection for a VM.
///
/// Loopback is intentionally not a VM configuration option. It remains
/// available through [`LoopbackHub`] for isolated backend tests only.
#[derive(Clone)]
pub enum NetworkBackendConfig {
    /// Bounded, reproducible Ethernet segment for deterministic VM tests.
    Deterministic,
    /// Shared deterministic VM fixture. The fixture owns the DHCP server and
    /// allocates distinct MAC pairs as VMs attach.
    DeterministicShared {
        network: Rc<RefCell<DeterministicVmNetwork>>,
    },
    /// User-mode networking transport. A host-side gateway receives the
    /// Ethernet frames over UDP and performs the NAT/user-mode service.
    UserNat { bind: SocketAddr, peer: SocketAddr },
    /// Attach the guest directly to a host Ethernet interface.
    Bridged { interface: String },
}

impl Default for NetworkBackendConfig {
    fn default() -> Self {
        Self::Deterministic
    }
}

pub trait NetBackend {
    fn transmit(&mut self, packet: &[u8]) -> Result<(), NetError>;
    fn receive(&mut self) -> Result<Option<Vec<u8>>, NetError>;
    fn link_up(&self) -> bool;
    fn admin_up(&self) -> bool {
        true
    }
    fn set_admin_up(&mut self, _up: bool) {}
    fn queue_state(&self) -> NetQueueState {
        NetQueueState::EMPTY
    }
    fn set_promiscuous(&mut self, enabled: bool) {
        let _ = enabled;
    }
}

/// Host transport used by user-mode/NAT and bridged VM networking.
pub struct HostNetworkBackend {
    transport: HostTransport,
    state: *mut c_void,
}

enum HostTransport {
    Udp(UdpSocket),
    #[cfg(target_os = "linux")]
    Raw {
        fd: libc::c_int,
        interface: String,
    },
}

impl HostNetworkBackend {
    /// Open a UDP Ethernet frame transport for a user-mode/NAT gateway.
    pub fn user_nat(bind: SocketAddr, peer: SocketAddr, mac: MacAddress) -> io::Result<Self> {
        let socket = UdpSocket::bind(bind)?;
        socket.connect(peer)?;
        socket.set_nonblocking(true)?;
        Ok(Self::from_transport(HostTransport::Udp(socket), mac))
    }

    /// Open a host Ethernet interface for bridged networking.
    #[cfg(target_os = "linux")]
    pub fn bridged(interface: &str, mac: MacAddress) -> io::Result<Self> {
        use std::ffi::CString;
        use std::mem;
        let name = CString::new(interface.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid interface name"))?;
        let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
        if index == 0 {
            return Err(io::Error::last_os_error());
        }
        let protocol = (libc::ETH_P_ALL as u16).to_be() as i32;
        let fd = unsafe {
            libc::socket(
                libc::AF_PACKET,
                libc::SOCK_RAW | libc::SOCK_NONBLOCK,
                protocol,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut address: libc::sockaddr_ll = unsafe { mem::zeroed() };
        address.sll_family = libc::AF_PACKET as u16;
        address.sll_protocol = (libc::ETH_P_ALL as u16).to_be();
        address.sll_ifindex = index as i32;
        let result = unsafe {
            libc::bind(
                fd,
                (&address as *const libc::sockaddr_ll).cast(),
                mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            )
        };
        if result < 0 {
            let error = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(error);
        }

        Ok(Self::from_transport(HostTransport::Raw { fd, interface: interface.to_string() }, mac))
    }

    #[cfg(not(target_os = "linux"))]
    pub fn bridged(_interface: &str, _mac: MacAddress) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "bridged Ethernet backend requires Linux AF_PACKET support",
        ))
    }

    fn from_transport(transport: HostTransport, mac: MacAddress) -> Self {
        let state = unsafe { ghostos_vm_host_net_new(&mac, matches!(&transport, HostTransport::Udp(_))) };
        assert!(!state.is_null(), "host network native allocation failed");
        Self { transport, state }
    }
}

impl HostTransport {
    fn link_up(&self) -> bool {
        match self {
            Self::Udp(_) => true,
            #[cfg(target_os = "linux")]
            Self::Raw { interface, .. } => {
                std::fs::read_to_string(format!("/sys/class/net/{interface}/carrier"))
                    .map(|state| state.trim() == "1")
                    .unwrap_or_else(|_| {
                        std::fs::read_to_string(format!("/sys/class/net/{interface}/operstate"))
                            .map(|state| state.trim() != "down").unwrap_or(false)
                    })
            }
        }
    }
    fn io(&mut self) -> CHostIo {
        CHostIo { link_up: host_link, send: host_send, receive: host_receive,
            context: (self as *mut HostTransport).cast() }
    }
}

unsafe extern "C" fn host_link(raw: *mut c_void) -> bool {
    unsafe { &*raw.cast::<HostTransport>() }.link_up()
}
unsafe extern "C" fn host_send(raw: *mut c_void, bytes: *const u8, length: usize) -> i32 {
    let transport = unsafe { &*raw.cast::<HostTransport>() };
    let frame = unsafe { std::slice::from_raw_parts(bytes, length) };
    let result = match transport {
        HostTransport::Udp(socket) => socket.send(frame).map(|_| ()),
        #[cfg(target_os = "linux")]
        HostTransport::Raw { fd, .. } => {
            let sent = unsafe { libc::send(*fd, frame.as_ptr().cast(), frame.len(), libc::MSG_DONTWAIT) };
            if sent < 0 { Err(io::Error::last_os_error()) }
            else if sent as usize != frame.len() { return NetError::BackendUnavailable as i32; }
            else { Ok(()) }
        }
    };
    result.map_or_else(|error| if error.kind() == io::ErrorKind::WouldBlock {
        NetError::QueueFull as i32
    } else { NetError::BackendUnavailable as i32 }, |_| -1)
}
unsafe extern "C" fn host_receive(raw: *mut c_void, bytes: *mut u8, capacity: usize, length: *mut usize) -> i32 {
    let transport = unsafe { &*raw.cast::<HostTransport>() };
    let frame = unsafe { std::slice::from_raw_parts_mut(bytes, capacity) };
    let result = match transport {
        HostTransport::Udp(socket) => socket.recv(frame),
        #[cfg(target_os = "linux")]
        HostTransport::Raw { fd, .. } => {
            let received = unsafe { libc::recv(*fd, frame.as_mut_ptr().cast(), frame.len(), libc::MSG_DONTWAIT) };
            if received < 0 { Err(io::Error::last_os_error()) } else { Ok(received as usize) }
        }
    };
    match result {
        Ok(received) => { unsafe { *length = received }; 1 }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => 0,
        Err(_) => -1,
    }
}

// Only owning mutable operations change the C state; transport handles retain
// the same Send/Sync semantics as the previous Rust backend.
unsafe impl Send for HostNetworkBackend {}
unsafe impl Sync for HostNetworkBackend {}

impl Drop for HostNetworkBackend {
    fn drop(&mut self) {
        unsafe { ghostos_vm_host_net_free(self.state) };
    }
}

impl Drop for HostTransport {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        if let Self::Raw { fd, .. } = self {
            unsafe { libc::close(*fd) };
        }
    }
}

impl NetBackend for HostNetworkBackend {
    fn transmit(&mut self, packet: &[u8]) -> Result<(), NetError> {
        native_transmitted(unsafe { ghostos_vm_host_net_transmit(self.state, &self.transport.io(), packet.as_ptr(), packet.len()) })
    }
    fn receive(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        let mut bytes = [0; ETHERNET_FRAME_MAX]; let mut length = 0;
        let result = unsafe { ghostos_vm_host_net_receive(self.state, &self.transport.io(), bytes.as_mut_ptr(), &mut length) };
        native_received(result, &bytes, length)
    }
    fn link_up(&self) -> bool { self.transport.link_up() }
    fn admin_up(&self) -> bool { unsafe { ghostos_vm_host_net_admin(self.state) } }
    fn set_admin_up(&mut self, up: bool) { unsafe { ghostos_vm_host_net_set_admin(self.state, up) }; }
    fn queue_state(&self) -> NetQueueState {
        NetQueueState { rx_packets: 0, tx_packets: unsafe { ghostos_vm_host_net_transmitted(self.state) } }
    }
    fn set_promiscuous(&mut self, enabled: bool) { unsafe { ghostos_vm_host_net_set_promiscuous(self.state, enabled) }; }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NetQueueState {
    pub rx_packets: usize,
    pub tx_packets: usize,
}

impl NetQueueState {
    pub const EMPTY: Self = Self {
        rx_packets: 0,
        tx_packets: 0,
    };
}

unsafe extern "C" {
    fn ghostos_vm_segment_new(ports: usize, loopback: bool) -> *mut c_void;
    fn ghostos_vm_segment_free(segment: *mut c_void);
    fn ghostos_vm_segment_set_link(segment: *mut c_void, up: bool);
    fn ghostos_vm_segment_link(segment: *const c_void) -> bool;
    fn ghostos_vm_segment_set_limit(segment: *mut c_void, limit: usize);
    fn ghostos_vm_segment_drop_next(segment: *mut c_void, count: usize);
    fn ghostos_vm_segment_clear(segment: *mut c_void);
    fn ghostos_vm_segment_disconnect(segment: *mut c_void, mac: *const MacAddress) -> bool;
    fn ghostos_vm_segment_connect(segment: *mut c_void, mac: *const MacAddress, port: *mut usize) -> i32;
    fn ghostos_vm_segment_admin(segment: *const c_void, port: usize) -> bool;
    fn ghostos_vm_segment_set_admin(segment: *mut c_void, port: usize, up: bool);
    fn ghostos_vm_segment_set_promiscuous(segment: *mut c_void, port: usize, enabled: bool);
    fn ghostos_vm_segment_queued(segment: *const c_void, port: usize) -> usize;
    fn ghostos_vm_segment_transmitted(segment: *const c_void) -> usize;
    fn ghostos_vm_segment_transmit(segment: *mut c_void, port: usize, packet: *const u8, length: usize) -> i32;
    fn ghostos_vm_segment_receive(segment: *mut c_void, port: usize, output: *mut u8, length: *mut usize) -> i32;
    fn ghostos_vm_loopback_transmit(segment: *mut c_void, port: usize, admin: bool, packet: *const u8, length: usize) -> i32;
    fn ghostos_vm_loopback_receive(segment: *mut c_void, port: usize, mac: *const MacAddress,
        admin: bool, promiscuous: bool, output: *mut u8, length: *mut usize) -> i32;
}

fn native_error(code: i32) -> NetError {
    match code {
        0 => NetError::PacketTooLarge,
        1 => NetError::Truncated,
        2 => NetError::QueueFull,
        3 => NetError::LinkDown,
        4 => NetError::AdminDown,
        5 => NetError::BackendUnavailable,
        _ => panic!("network segment native allocation failed"),
    }
}
fn native_transmitted(result: i32) -> Result<(), NetError> {
    if result == -1 { Ok(()) } else { Err(native_error(result)) }
}
fn native_received(result: i32, bytes: &[u8], length: usize) -> Result<Option<Vec<u8>>, NetError> {
    match result {
        0 => Ok(None),
        1 => Ok(Some(bytes[..length].to_vec())),
        error => Err(native_error(-error - 1)),
    }
}

/// Two-port hub; queues, delivery, carrier, and counters are owned by C.
pub struct LoopbackHub { state: *mut c_void }

impl LoopbackHub {
    pub fn new() -> Self {
        let state = unsafe { ghostos_vm_segment_new(2, true) };
        assert!(!state.is_null(), "loopback native allocation failed");
        Self { state }
    }
    pub fn set_link_up(&mut self, up: bool) { unsafe { ghostos_vm_segment_set_link(self.state, up) }; }
    pub fn link_up(&self) -> bool { unsafe { ghostos_vm_segment_link(self.state) } }
    pub fn queued_packets(&self, port: usize) -> usize { unsafe { ghostos_vm_segment_queued(self.state, port) } }
    pub fn clear(&mut self) { unsafe { ghostos_vm_segment_clear(self.state) }; }
}
// As with the previous owned queues, moving a hub is safe. Shared ports still
// use Rc<RefCell<_>> and cannot cross threads.
unsafe impl Send for LoopbackHub {}
unsafe impl Sync for LoopbackHub {}
impl Drop for LoopbackHub {
    fn drop(&mut self) { unsafe { ghostos_vm_segment_free(self.state) }; }
}
impl Default for LoopbackHub { fn default() -> Self { Self::new() } }

pub struct LoopbackPort {
    hub: Rc<RefCell<LoopbackHub>>,
    index: usize,
    mac: MacAddress,
    promiscuous: bool,
    admin_up: bool,
}
impl LoopbackPort {
    pub fn new(hub: Rc<RefCell<LoopbackHub>>, index: usize, mac: MacAddress) -> Self {
        assert!(index < 2, "loopback hub has exactly two ports");
        Self { hub, index, mac, promiscuous: false, admin_up: true }
    }
    pub fn mac(&self) -> MacAddress { self.mac }
}
impl NetBackend for LoopbackPort {
    fn transmit(&mut self, packet: &[u8]) -> Result<(), NetError> {
        if !self.admin_up { return Err(NetError::AdminDown); }
        if !self.hub.borrow().link_up() { return Err(NetError::LinkDown); }
        if packet.len() < ETHERNET_HEADER_LEN { return Err(NetError::Truncated); }
        native_transmitted(unsafe { ghostos_vm_loopback_transmit(self.hub.borrow_mut().state,
            self.index, self.admin_up, packet.as_ptr(), packet.len()) })
    }
    fn receive(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        if !self.admin_up { return Err(NetError::AdminDown); }
        if !self.hub.borrow().link_up() { return Err(NetError::LinkDown); }
        let mut bytes = [0; ETHERNET_FRAME_MAX]; let mut length = 0;
        let result = unsafe { ghostos_vm_loopback_receive(self.hub.borrow_mut().state,
            self.index, &self.mac, self.admin_up, self.promiscuous, bytes.as_mut_ptr(), &mut length) };
        native_received(result, &bytes, length)
    }
    fn link_up(&self) -> bool { self.hub.borrow().link_up() }
    fn admin_up(&self) -> bool { self.admin_up }
    fn set_admin_up(&mut self, up: bool) { self.admin_up = up; }
    fn queue_state(&self) -> NetQueueState {
        let hub = self.hub.borrow();
        NetQueueState { rx_packets: hub.queued_packets(self.index),
            tx_packets: unsafe { ghostos_vm_segment_transmitted(hub.state) } }
    }
    fn set_promiscuous(&mut self, enabled: bool) { self.promiscuous = enabled; }
}

/// Bounded shared segment; C retains port identities and all packet state.
pub struct DeterministicSegment { state: *mut c_void }
impl DeterministicSegment {
    pub fn new(max_ports: usize) -> Rc<RefCell<Self>> {
        let state = unsafe { ghostos_vm_segment_new(max_ports, false) };
        assert!(!state.is_null(), "segment native allocation failed");
        Rc::new(RefCell::new(Self { state }))
    }
    pub fn set_link_up(&mut self, up: bool) { unsafe { ghostos_vm_segment_set_link(self.state, up) }; }
    pub fn link_up(&self) -> bool { unsafe { ghostos_vm_segment_link(self.state) } }
    pub fn set_max_queue(&mut self, max_queue: usize) { unsafe { ghostos_vm_segment_set_limit(self.state, max_queue) }; }
    pub fn drop_next(&mut self, count: usize) { unsafe { ghostos_vm_segment_drop_next(self.state, count) }; }
    pub fn disconnect(&mut self, mac: MacAddress) -> bool { unsafe { ghostos_vm_segment_disconnect(self.state, &mac) } }
    pub fn connect(segment: Rc<RefCell<Self>>, mac: MacAddress) -> Result<DeterministicPort, NetError> {
        let mut index = 0;
        let result = unsafe { ghostos_vm_segment_connect(segment.borrow_mut().state, &mac, &mut index) };
        native_transmitted(result)?;
        Ok(DeterministicPort { segment, index, mac })
    }
    pub fn queued_packets(&self, port: usize) -> usize { unsafe { ghostos_vm_segment_queued(self.state, port) } }
}
unsafe impl Send for DeterministicSegment {}
unsafe impl Sync for DeterministicSegment {}
impl Drop for DeterministicSegment {
    fn drop(&mut self) { unsafe { ghostos_vm_segment_free(self.state) }; }
}

pub struct DeterministicPort {
    segment: Rc<RefCell<DeterministicSegment>>,
    index: usize,
    mac: MacAddress,
}
impl DeterministicPort { pub fn mac(&self) -> MacAddress { self.mac } }
impl NetBackend for DeterministicPort {
    fn transmit(&mut self, packet: &[u8]) -> Result<(), NetError> {
        native_transmitted(unsafe { ghostos_vm_segment_transmit(self.segment.borrow_mut().state,
            self.index, packet.as_ptr(), packet.len()) })
    }
    fn receive(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        let mut bytes = [0; ETHERNET_FRAME_MAX]; let mut length = 0;
        let result = unsafe { ghostos_vm_segment_receive(self.segment.borrow_mut().state,
            self.index, bytes.as_mut_ptr(), &mut length) };
        native_received(result, &bytes, length)
    }
    fn link_up(&self) -> bool { self.segment.borrow().link_up() }
    fn admin_up(&self) -> bool { unsafe { ghostos_vm_segment_admin(self.segment.borrow().state, self.index) } }
    fn set_admin_up(&mut self, up: bool) { unsafe { ghostos_vm_segment_set_admin(self.segment.borrow_mut().state, self.index, up) }; }
    fn queue_state(&self) -> NetQueueState {
        let segment = self.segment.borrow();
        NetQueueState { rx_packets: segment.queued_packets(self.index),
            tx_packets: unsafe { ghostos_vm_segment_transmitted(segment.state) } }
    }
    fn set_promiscuous(&mut self, enabled: bool) { unsafe { ghostos_vm_segment_set_promiscuous(self.segment.borrow_mut().state, self.index, enabled) }; }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(destination: MacAddress, source: MacAddress) -> Vec<u8> {
        let mut frame = vec![0; ETHERNET_HEADER_LEN];
        frame[..6].copy_from_slice(&destination.to_bytes());
        frame[6..12].copy_from_slice(&source.to_bytes());
        frame
    }

    #[test]
    fn shared_segment_delivers_unicast_and_broadcast() {
        let segment = DeterministicSegment::new(3);
        let first_mac = MacAddress::ghostos_default(0x60);
        let second_mac = MacAddress::ghostos_default(0x61);
        let third_mac = MacAddress::ghostos_default(0x62);
        let mut first = DeterministicSegment::connect(segment.clone(), first_mac).unwrap();
        let mut second = DeterministicSegment::connect(segment.clone(), second_mac).unwrap();
        let mut third = DeterministicSegment::connect(segment, third_mac).unwrap();

        first.transmit(&frame(second_mac, first_mac)).unwrap();
        assert!(second.receive().unwrap().is_some());
        assert!(third.receive().unwrap().is_none());

        first.transmit(&frame(MacAddress::BROADCAST, first_mac)).unwrap();
        assert!(second.receive().unwrap().is_some());
        assert!(third.receive().unwrap().is_some());
    }

    #[test]
    fn administrative_state_is_distinct_from_carrier_state() {
        let segment = DeterministicSegment::new(1);
        let mac = MacAddress::ghostos_default(0x63);
        let mut port = DeterministicSegment::connect(segment.clone(), mac).unwrap();
        assert!(port.link_up());
        port.set_admin_up(false);
        assert!(port.link_up());
        assert!(!port.admin_up());
        assert_eq!(port.transmit(&frame(MacAddress::BROADCAST, mac)), Err(NetError::AdminDown));
        segment.borrow_mut().set_link_up(false);
        assert!(!port.link_up());
    }
}
