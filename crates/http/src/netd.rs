use ghostos_ipc::{Ring, RingError, SharedBuffer};
use ghostos_netd::{
    SOCKET_RESPONSE_SCHEMA, SocketCapability, SocketOperation, SocketRequest, SocketResponse,
    SocketRights,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetdError {
    Busy,
    InvalidCompletion,
    NoOutstandingRequest,
    RequestPending,
    RequestRingFull,
    UnexpectedCompletion,
}

/// Single-flight async client for a capability-mapped `ghostos-netd` channel.
pub struct NetdClient<'a, const RING_CAPACITY: usize> {
    requests: &'a Ring<RING_CAPACITY>,
    completions: &'a Ring<RING_CAPACITY>,
    next_correlation: u128,
    outstanding: Option<u128>,
}

impl<'a, const RING_CAPACITY: usize> NetdClient<'a, RING_CAPACITY> {
    pub const fn new(
        requests: &'a Ring<RING_CAPACITY>,
        completions: &'a Ring<RING_CAPACITY>,
        correlation_seed: u128,
    ) -> Self {
        Self {
            requests,
            completions,
            next_correlation: correlation_seed,
            outstanding: None,
        }
    }

    pub const fn has_outstanding_request(&self) -> bool {
        self.outstanding.is_some()
    }

    pub fn open_tcp(&mut self, rights: SocketRights) -> Result<u128, NetdError> {
        let correlation = self.correlation();
        self.submit(SocketRequest::open(correlation, rights))
    }

    pub fn listen(&mut self, capability: SocketCapability, port: u16) -> Result<u128, NetdError> {
        self.operation(SocketOperation::Listen, capability, port as u64, 0, None)
    }

    pub fn receive(
        &mut self,
        capability: SocketCapability,
        buffer: SharedBuffer,
    ) -> Result<u128, NetdError> {
        self.operation(SocketOperation::Receive, capability, 0, 0, Some(buffer))
    }

    pub fn send(
        &mut self,
        capability: SocketCapability,
        buffer: SharedBuffer,
    ) -> Result<u128, NetdError> {
        self.operation(SocketOperation::Send, capability, 0, 0, Some(buffer))
    }

    pub fn state(&mut self, capability: SocketCapability) -> Result<u128, NetdError> {
        self.operation(SocketOperation::State, capability, 0, 0, None)
    }

    pub fn close(&mut self, capability: SocketCapability) -> Result<u128, NetdError> {
        self.operation(SocketOperation::Close, capability, 0, 0, None)
    }

    pub fn poll_completion(&mut self) -> Result<Option<SocketResponse>, NetdError> {
        let expected = self.outstanding.ok_or(NetdError::NoOutstandingRequest)?;
        let envelope = match self.completions.try_receive() {
            Ok(envelope) => envelope,
            Err(RingError::Empty) => return Ok(None),
            Err(RingError::Full) => unreachable!(),
        };
        if envelope.label != SOCKET_RESPONSE_SCHEMA {
            return Err(NetdError::InvalidCompletion);
        }
        let response =
            SocketResponse::decode(envelope).map_err(|_| NetdError::InvalidCompletion)?;
        if response.correlation != expected {
            return Err(NetdError::UnexpectedCompletion);
        }
        self.outstanding = None;
        Ok(Some(response))
    }

    fn operation(
        &mut self,
        operation: SocketOperation,
        capability: SocketCapability,
        argument0: u64,
        argument1: u64,
        buffer: Option<SharedBuffer>,
    ) -> Result<u128, NetdError> {
        let correlation = self.correlation();
        self.submit(SocketRequest::encode(
            correlation,
            operation,
            Some(capability),
            argument0,
            argument1,
            buffer,
        ))
    }

    fn correlation(&mut self) -> u128 {
        let correlation = self.next_correlation;
        self.next_correlation = self.next_correlation.wrapping_add(1);
        correlation
    }

    fn submit(&mut self, request: ghostos_ipc::Envelope) -> Result<u128, NetdError> {
        if self.outstanding.is_some() {
            return Err(NetdError::RequestPending);
        }
        let correlation = request.correlation;
        self.requests
            .try_send(request)
            .map_err(|_| NetdError::RequestRingFull)?;
        self.outstanding = Some(correlation);
        Ok(correlation)
    }
}
