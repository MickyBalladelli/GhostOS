use smoltcp::iface::{Interface, PollIngressSingleResult, SocketHandle, SocketSet, SocketStorage};
use smoltcp::phy::Device;
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{IpAddress, IpEndpoint, Ipv4Address};

use crate::{ServiceError, SocketBackend, SocketState};

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

/// Heap-free `smoltcp` TCP/IP stack owned by the Ring 3 network daemon.
pub struct SmolTcpStack<'a, D, const SOCKETS: usize> {
    interface: Interface,
    device: D,
    sockets: SocketSet<'a>,
    handles: [Option<SocketHandle>; SOCKETS],
    leased: [bool; SOCKETS],
    next_ephemeral: u16,
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
