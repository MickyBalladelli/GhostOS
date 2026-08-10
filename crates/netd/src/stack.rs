use smoltcp::iface::{
    Interface, PollIngressSingleResult, Route, SocketHandle, SocketSet, SocketStorage,
};
use smoltcp::phy::Device;
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{IpAddress, IpCidr, IpEndpoint, Ipv4Address, Ipv4Cidr};

use crate::{
    DhcpError, DhcpLease, DhcpLeaseRuntime, ServiceError, SocketBackend, SocketState,
    StaticSnapshot,
};
use synos_time_sync::MonotonicClock;

/// smoltcp 0.13 keeps four routes per interface by default.
pub const MAX_INTERFACE_ROUTES: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InterfaceConfigError {
    InvalidAddress,
    InvalidSubnetMask,
    InvalidRoute,
    TooManyRoutes,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterfaceRoute {
    pub destination: [u8; 4],
    pub prefix_len: u8,
    pub gateway: [u8; 4],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InterfaceConfig {
    pub address: [u8; 4],
    pub subnet_mask: [u8; 4],
    pub gateway: Option<[u8; 4]>,
    pub routes: [Option<InterfaceRoute>; MAX_INTERFACE_ROUTES],
}

impl InterfaceConfig {
    pub const fn new(address: [u8; 4], subnet_mask: [u8; 4]) -> Self {
        Self {
            address,
            subnet_mask,
            gateway: None,
            routes: [None; MAX_INTERFACE_ROUTES],
        }
    }

    fn prefix_len(&self) -> Result<u8, InterfaceConfigError> {
        let mask = u32::from_be_bytes(self.subnet_mask);
        let prefix_len = mask.leading_ones() as u8;
        let expected = if prefix_len == 0 {
            0
        } else {
            u32::MAX << (32 - prefix_len)
        };
        if mask == expected {
            Ok(prefix_len)
        } else {
            Err(InterfaceConfigError::InvalidSubnetMask)
        }
    }

    fn validate(&self) -> Result<u8, InterfaceConfigError> {
        if !valid_unicast_ipv4(self.address) {
            return Err(InterfaceConfigError::InvalidAddress)
        }
        let prefix_len = self.prefix_len()?;
        if self
            .gateway
            .is_some_and(|gateway| !valid_unicast_ipv4(gateway))
        {
            return Err(InterfaceConfigError::InvalidRoute)
        }
        for route in self.routes.iter().flatten() {
            if route.prefix_len > 32
                || !valid_unicast_ipv4(route.gateway)
                || !canonical_route_destination(route.destination, route.prefix_len)
            {
                return Err(InterfaceConfigError::InvalidRoute)
            }
        }
        let route_count = self.routes.iter().flatten().count();
        let has_default = self
            .routes
            .iter()
            .flatten()
            .any(|route| route.prefix_len == 0);
        if self.gateway.is_some() && !has_default {
            if route_count == MAX_INTERFACE_ROUTES {
                return Err(InterfaceConfigError::TooManyRoutes)
            }
        }
        Ok(prefix_len)
    }

    pub fn from_dhcp_lease(lease: &DhcpLease) -> Result<Self, InterfaceConfigError> {
        let mut config = Self::new(lease.address, lease.subnet_mask);
        config.gateway = lease.gateway;
        for route in lease.routes.iter().take(lease.route_count as usize) {
            let slot = config
                .routes
                .iter_mut()
                .find(|entry| entry.is_none())
                .ok_or(InterfaceConfigError::TooManyRoutes)?;
            *slot = Some(InterfaceRoute {
                destination: route.destination,
                prefix_len: route.prefix_len,
                gateway: route.gateway,
            });
        }
        if lease.route_count as usize > lease.routes.len() {
            return Err(InterfaceConfigError::TooManyRoutes)
        }
        Ok(config)
    }

    pub fn from_static_snapshot(
        snapshot: &StaticSnapshot,
    ) -> Result<Self, InterfaceConfigError> {
        let subnet_mask = snapshot
            .subnet_mask
            .ok_or(InterfaceConfigError::InvalidSubnetMask)?;
        let mut config = Self::new(snapshot.address, subnet_mask);
        config.gateway = snapshot.gateway;
        Ok(config)
    }
}

fn valid_unicast_ipv4(address: [u8; 4]) -> bool {
    address != [0; 4]
        && address != [255; 4]
        && (address[0] & 0xf0) != 0xe0
}

fn canonical_route_destination(destination: [u8; 4], prefix_len: u8) -> bool {
    if prefix_len > 32 {
        return false
    }
    let bits = u32::from_be_bytes(destination);
    let mask = if prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_len)
    };
    bits & mask == bits
}

pub struct TcpBuffers<const BUFFER_SIZE: usize> {
    receive: [u8; BUFFER_SIZE],
    transmit: [u8; BUFFER_SIZE],
}

impl<const BUFFER_SIZE: usize> TcpBuffers<BUFFER_SIZE> {
    pub const fn new() -> Self {
        Self {
            receive: [0; BUFFER_SIZE],
            transmit: [0; BUFFER_SIZE],
        }
    }
}

impl<const BUFFER_SIZE: usize> Default for TcpBuffers<BUFFER_SIZE> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct TcpHandle(usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PollActivity {
    pub ingress_packets: usize,
    pub socket_state_changed: bool,
}

pub trait NetworkPoller {
    fn poll_network(&mut self, now_millis: i64, ingress_budget: usize) -> PollActivity;
}

/// Heap-free `smoltcp` TCP/IP stack owned by the Ring 3 network daemon.
pub struct SmolTcpStack<'a, D, const SOCKETS: usize> {
    interface: Interface,
    device: D,
    sockets: SocketSet<'a>,
    handles: [Option<SocketHandle>; SOCKETS],
    leased: [bool; SOCKETS],
    next_ephemeral: u16,
    static_config: Option<InterfaceConfig>,
}

impl<'a, D: Device, const SOCKETS: usize> SmolTcpStack<'a, D, SOCKETS> {
    pub fn new<const BUFFER_SIZE: usize>(
        interface: Interface,
        device: D,
        storage: &'a mut [SocketStorage<'a>; SOCKETS],
        buffers: &'a mut [TcpBuffers<BUFFER_SIZE>; SOCKETS],
    ) -> Self {
        assert!(BUFFER_SIZE > 0);
        let mut sockets = SocketSet::new(&mut storage[..]);
        let mut handles = [None; SOCKETS];
        for (index, buffer) in buffers.iter_mut().enumerate() {
            let receive = tcp::SocketBuffer::new(&mut buffer.receive[..]);
            let transmit = tcp::SocketBuffer::new(&mut buffer.transmit[..]);
            handles[index] = Some(sockets.add(tcp::Socket::new(receive, transmit)))
        }
        Self {
            interface,
            device,
            sockets,
            handles,
            leased: [false; SOCKETS],
            next_ephemeral: 49_152,
            static_config: None,
        }
    }

    pub fn device(&self) -> &D {
        &self.device
    }

    pub fn device_mut(&mut self) -> &mut D {
        &mut self.device
    }

    pub fn interface(&self) -> &Interface {
        &self.interface
    }

    pub fn interface_mut(&mut self) -> &mut Interface {
        &mut self.interface
    }

    /// Publish one complete IPv4 configuration to smoltcp.
    ///
    /// Validation happens before either smoltcp table is changed, so a bad
    /// update leaves the previous address and routes active.
    pub fn apply_interface_config(
        &mut self,
        config: InterfaceConfig,
    ) -> Result<(), InterfaceConfigError> {
        self.apply_config(config)?;
        self.static_config = Some(config);
        Ok(())
    }

    fn apply_config(&mut self, config: InterfaceConfig) -> Result<(), InterfaceConfigError> {
        let prefix_len = config.validate()?;
        let address = Ipv4Address::from_octets(config.address);
        let cidr = Ipv4Cidr::new(address, prefix_len);

        self.interface.update_ip_addrs(|addrs| {
            addrs.clear();
            addrs
                .push(IpCidr::Ipv4(cidr))
                .expect("validated IPv4 address fits smoltcp address table")
        });

        self.interface.routes_mut().update(|routes| {
            routes.clear();
            for route in config.routes.iter().flatten() {
                routes
                    .push(Route {
                        cidr: Ipv4Cidr::new(
                            Ipv4Address::from_octets(route.destination),
                            route.prefix_len,
                        )
                        .into(),
                        via_router: Ipv4Address::from_octets(route.gateway).into(),
                        preferred_until: None,
                        expires_at: None,
                    })
                    .expect("validated routes fit smoltcp route table")
            }
            if config.gateway.is_some()
                && !config
                    .routes
                    .iter()
                    .flatten()
                    .any(|route| route.prefix_len == 0)
            {
                routes
                    .push(Route::new_ipv4_gateway(
                        Ipv4Address::from_octets(config.gateway.unwrap()),
                    ))
                    .expect("validated gateway fits smoltcp route table")
            }
        });
        Ok(())
    }

    pub fn apply_dhcp_lease(&mut self, lease: &DhcpLease) -> Result<(), InterfaceConfigError> {
        self.apply_config(InterfaceConfig::from_dhcp_lease(lease)?)
    }

    pub fn restore_static(
        &mut self,
        snapshot: &StaticSnapshot,
    ) -> Result<(), InterfaceConfigError> {
        let config = match self.static_config {
            Some(config) => config,
            None => InterfaceConfig::from_static_snapshot(snapshot)?,
        };
        self.apply_config(config)
    }

    /// Performs bounded ingress work, then one bounded egress pass.
    pub fn poll(&mut self, now_millis: i64, ingress_budget: usize) -> PollActivity {
        let timestamp = Instant::from_millis(now_millis);
        let mut activity = PollActivity {
            ingress_packets: 0,
            socket_state_changed: false,
        };
        self.interface.poll_maintenance(timestamp);
        for _ in 0..ingress_budget {
            match self.interface.poll_ingress_single(
                timestamp,
                &mut self.device,
                &mut self.sockets,
            ) {
                PollIngressSingleResult::None => break,
                PollIngressSingleResult::PacketProcessed => activity.ingress_packets += 1,
                PollIngressSingleResult::SocketStateChanged => {
                    activity.ingress_packets += 1;
                    activity.socket_state_changed = true
                }
            }
        }
        let egress = self
            .interface
            .poll_egress(timestamp, &mut self.device, &mut self.sockets);
        activity.socket_state_changed |=
            matches!(egress, smoltcp::iface::PollResult::SocketStateChanged);
        activity
    }

    pub fn poll_with_clock<C: MonotonicClock>(
        &mut self,
        clock: &C,
        ingress_budget: usize,
    ) -> PollActivity {
        self.poll((clock.now_us() / 1_000) as i64, ingress_budget)
    }

    fn socket_handle(&self, handle: TcpHandle) -> Result<SocketHandle, ServiceError> {
        if !self.leased.get(handle.0).copied().unwrap_or(false) {
            return Err(ServiceError::InvalidCapability)
        }
        self.handles
            .get(handle.0)
            .copied()
            .flatten()
            .ok_or(ServiceError::InvalidCapability)
    }
}

impl<D: Device, const SOCKETS: usize> NetworkPoller for SmolTcpStack<'_, D, SOCKETS> {
    fn poll_network(&mut self, now_millis: i64, ingress_budget: usize) -> PollActivity {
        self.poll(now_millis, ingress_budget)
    }
}

impl<D: Device, const SOCKETS: usize> DhcpLeaseRuntime for SmolTcpStack<'_, D, SOCKETS> {
    fn apply_lease(&mut self, _interface: &str, lease: &DhcpLease) -> Result<(), DhcpError> {
        self.apply_dhcp_lease(lease)
            .map_err(|_| DhcpError::Runtime)
    }

    fn restore_static(
        &mut self,
        _interface: &str,
        snapshot: &StaticSnapshot,
    ) -> Result<(), DhcpError> {
        self.restore_static(snapshot)
            .map_err(|_| DhcpError::Runtime)
    }
}

impl<D: Device, const SOCKETS: usize> SocketBackend for SmolTcpStack<'_, D, SOCKETS> {
    type Handle = TcpHandle;

    fn open_tcp(&mut self) -> Result<Self::Handle, ServiceError> {
        let index = self
            .leased
            .iter()
            .position(|leased| !*leased)
            .ok_or(ServiceError::NoSocketSpace)?;
        self.leased[index] = true;
        let handle = self.handles[index].ok_or(ServiceError::Backend)?;
        self.sockets.get_mut::<tcp::Socket>(handle).abort();
        Ok(TcpHandle(index))
    }

    fn listen(&mut self, handle: Self::Handle, port: u16) -> Result<(), ServiceError> {
        let handle = self.socket_handle(handle)?;
        self.sockets
            .get_mut::<tcp::Socket>(handle)
            .listen(port)
            .map_err(|_| ServiceError::Backend)
    }

    fn connect_ipv4(
        &mut self,
        handle: Self::Handle,
        address: [u8; 4],
        port: u16,
    ) -> Result<(), ServiceError> {
        let handle = self.socket_handle(handle)?;
        let local_port = self.next_ephemeral;
        self.next_ephemeral = if self.next_ephemeral == 65_535 {
            49_152
        } else {
            self.next_ephemeral + 1
        };
        let remote = IpEndpoint::new(
            IpAddress::Ipv4(Ipv4Address::from_octets(address)),
            port,
        );
        self.sockets
            .get_mut::<tcp::Socket>(handle)
            .connect(self.interface.context(), remote, local_port)
            .map_err(|_| ServiceError::Backend)
    }

    fn send(&mut self, handle: Self::Handle, bytes: &[u8]) -> Result<usize, ServiceError> {
        let handle = self.socket_handle(handle)?;
        let socket = self.sockets.get_mut::<tcp::Socket>(handle);
        if !socket.can_send() {
            return Err(ServiceError::WouldBlock)
        }
        let written = socket
            .send_slice(bytes)
            .map_err(|_| ServiceError::Backend)?;
        if written == 0 {
            Err(ServiceError::WouldBlock)
        } else {
            Ok(written)
        }
    }

    fn receive(
        &mut self,
        handle: Self::Handle,
        bytes: &mut [u8],
    ) -> Result<usize, ServiceError> {
        let handle = self.socket_handle(handle)?;
        let socket = self.sockets.get_mut::<tcp::Socket>(handle);
        if !socket.can_recv() {
            return Err(ServiceError::WouldBlock)
        }
        socket
            .recv_slice(bytes)
            .map_err(|_| ServiceError::Backend)
    }

    fn close(&mut self, handle: Self::Handle) {
        if let Ok(socket_handle) = self.socket_handle(handle) {
            self.sockets.get_mut::<tcp::Socket>(socket_handle).abort()
        }
        if let Some(leased) = self.leased.get_mut(handle.0) {
            *leased = false
        }
    }

    fn state(&self, handle: Self::Handle) -> SocketState {
        let Ok(handle) = self.socket_handle(handle) else {
            return SocketState::Closed
        };
        match self.sockets.get::<tcp::Socket>(handle).state() {
            tcp::State::Closed => SocketState::Closed,
            tcp::State::Listen => SocketState::Listening,
            tcp::State::SynSent | tcp::State::SynReceived => SocketState::Connecting,
            tcp::State::Established | tcp::State::CloseWait => SocketState::Established,
            tcp::State::FinWait1
            | tcp::State::FinWait2
            | tcp::State::Closing
            | tcp::State::LastAck
            | tcp::State::TimeWait => SocketState::Closing,
        }
    }
}
