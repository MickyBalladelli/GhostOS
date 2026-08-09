use synos_ipc::SharedBuffer;
use synos_netd::{SocketCapability, SocketRights};
use synos_status::{AuditContext, PublicError, RetryHint, Status, operation};

use crate::{
    DEFAULT_REQUEST_HEADERS, EncodeError, NetdClient, NetdError,
    ParseError, Router, WebRights, encode_error_http_response, encode_response, parse_error,
    parse_request, route_error,
};

pub const SERVER_SOCKET_RIGHTS: SocketRights = SocketRights::LISTEN
    .union(SocketRights::SEND)
    .union(SocketRights::RECEIVE)
    .union(SocketRights::CLOSE);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerState {
    Opening,
    Binding,
    Receiving,
    Sending,
    Closing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerEvent {
    Pending,
    Progress,
    RequestReceived { bytes: usize },
    ResponseSent { bytes: usize },
    ConnectionClosed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerError {
    BufferDescriptor,
    Encode(EncodeError),
    Netd(NetdError),
    NetdStatus(Status),
    Protocol,
}

impl From<NetdError> for ServerError {
    fn from(error: NetdError) -> Self {
        Self::Netd(error)
    }
}

impl From<EncodeError> for ServerError {
    fn from(error: EncodeError) -> Self {
        Self::Encode(error)
    }
}

/// Bounded HTTP/1 server loop over a single `synos-netd` TCP capability.
///
/// The caller maps `receive_descriptor` to `request_bytes` and
/// `send_descriptor` to `response_bytes`. It also drives `NetworkDaemon`
/// independently. Every call performs at most one completion or submission.
pub struct HttpServer<'a, State, const ROUTES: usize, const RING_CAPACITY: usize> {
    router: Router<State, ROUTES>,
    netd: NetdClient<'a, RING_CAPACITY>,
    principal: u64,
    web_rights: WebRights,
    port: u16,
    state: ServerState,
    socket: Option<SocketCapability>,
    received: usize,
    response_length: usize,
    sent: usize,
}

impl<'a, State, const ROUTES: usize, const RING_CAPACITY: usize>
    HttpServer<'a, State, ROUTES, RING_CAPACITY>
{
    pub fn new(
        router: Router<State, ROUTES>,
        netd: NetdClient<'a, RING_CAPACITY>,
        principal: u64,
        web_rights: WebRights,
        port: u16,
    ) -> Result<Self, ServerError> {
        if port == 0 {
            return Err(ServerError::Protocol);
        }
        Ok(Self {
            router,
            netd,
            principal,
            web_rights,
            port,
            state: ServerState::Opening,
            socket: None,
            received: 0,
            response_length: 0,
            sent: 0,
        })
    }

    pub const fn state(&self) -> ServerState {
        self.state
    }

    pub const fn router(&self) -> &Router<State, ROUTES> {
        &self.router
    }

    pub fn router_mut(&mut self) -> &mut Router<State, ROUTES> {
        &mut self.router
    }

    #[allow(clippy::too_many_arguments)]
    pub fn poll(
        &mut self,
        receive_descriptor: SharedBuffer,
        send_descriptor: SharedBuffer,
        request_bytes: &mut [u8],
        response_bytes: &mut [u8],
        handler_body: &mut [u8],
    ) -> Result<ServerEvent, ServerError> {
        validate_buffers(
            receive_descriptor,
            send_descriptor,
            request_bytes,
            response_bytes,
        )?;
        if !self.netd.has_outstanding_request() {
            return self.submit_next(receive_descriptor, send_descriptor);
        }
        let Some(completion) = self.netd.poll_completion()? else {
            return Ok(ServerEvent::Pending);
        };
        if completion.status == Status::BUSY
            && matches!(self.state, ServerState::Receiving | ServerState::Sending)
        {
            return self.submit_next(receive_descriptor, send_descriptor);
        }
        if !completion.status.is_success() {
            return Err(ServerError::NetdStatus(completion.status));
        }
        match self.state {
            ServerState::Opening => {
                self.socket = completion.capability;
                if self.socket.is_none() {
                    return Err(ServerError::Protocol);
                }
                self.state = ServerState::Binding;
                self.submit_next(receive_descriptor, send_descriptor)
            }
            ServerState::Binding => {
                self.state = ServerState::Receiving;
                self.received = 0;
                self.submit_next(receive_descriptor, send_descriptor)
            }
            ServerState::Receiving => {
                let count = usize::try_from(completion.value).map_err(|_| ServerError::Protocol)?;
                let remaining = request_bytes.len().saturating_sub(self.received);
                if count > remaining {
                    return Err(ServerError::Protocol);
                }
                self.received += count;
                match parse_request::<DEFAULT_REQUEST_HEADERS>(&request_bytes[..self.received]) {
                    Ok(parsed) => {
                        let audit = AuditContext::new(completion.correlation, 1);
                        match self.router.handle(
                            self.principal,
                            self.web_rights,
                            parsed.request,
                            handler_body,
                        ) {
                            Ok(response) => {
                                self.response_length = encode_response(response, response_bytes)?;
                            }
                            Err(error) => {
                                self.response_length = encode_error_http_response(
                                    route_error(error, audit),
                                    response_bytes,
                                )?;
                            }
                        }
                    }
                    Err(ParseError::Incomplete) if self.received < request_bytes.len() => {
                        return self.submit_next(receive_descriptor, send_descriptor);
                    }
                    Err(ParseError::Incomplete) => {
                        let error = PublicError::new(
                            Status::REQUEST_TOO_LARGE,
                            operation::HTTP_PARSE,
                            RetryHint::Never,
                            AuditContext::new(completion.correlation, 1),
                        );
                        self.response_length = encode_error_http_response(error, response_bytes)?;
                    }
                    Err(error) => {
                        self.response_length = encode_error_http_response(
                            parse_error(error, AuditContext::new(completion.correlation, 1)),
                            response_bytes,
                        )?;
                    }
                }
                self.sent = 0;
                self.state = ServerState::Sending;
                self.submit_next(receive_descriptor, send_descriptor)?;
                Ok(ServerEvent::RequestReceived {
                    bytes: self.received,
                })
            }
            ServerState::Sending => {
                let count = usize::try_from(completion.value).map_err(|_| ServerError::Protocol)?;
                let remaining = self.response_length.saturating_sub(self.sent);
                if count == 0 || count > remaining {
                    return Err(ServerError::Protocol);
                }
                self.sent += count;
                if self.sent < self.response_length {
                    return self.submit_next(receive_descriptor, send_descriptor);
                }
                let bytes = self.sent;
                self.state = ServerState::Closing;
                self.submit_next(receive_descriptor, send_descriptor)?;
                Ok(ServerEvent::ResponseSent { bytes })
            }
            ServerState::Closing => {
                self.socket = None;
                self.received = 0;
                self.response_length = 0;
                self.sent = 0;
                self.state = ServerState::Opening;
                Ok(ServerEvent::ConnectionClosed)
            }
        }
    }

    fn submit_next(
        &mut self,
        receive_descriptor: SharedBuffer,
        send_descriptor: SharedBuffer,
    ) -> Result<ServerEvent, ServerError> {
        match self.state {
            ServerState::Opening => {
                self.netd.open_tcp(SERVER_SOCKET_RIGHTS)?;
            }
            ServerState::Binding => {
                self.netd.listen(self.socket()?, self.port)?;
            }
            ServerState::Receiving => {
                let descriptor = sub_buffer(
                    receive_descriptor,
                    self.received,
                    receive_descriptor.length as usize - self.received,
                    true,
                )?;
                self.netd.receive(self.socket()?, descriptor)?;
            }
            ServerState::Sending => {
                let descriptor = sub_buffer(
                    send_descriptor,
                    self.sent,
                    self.response_length - self.sent,
                    false,
                )?;
                self.netd.send(self.socket()?, descriptor)?;
            }
            ServerState::Closing => {
                self.netd.close(self.socket()?)?;
            }
        }
        Ok(ServerEvent::Progress)
    }

    fn socket(&self) -> Result<SocketCapability, ServerError> {
        self.socket.ok_or(ServerError::Protocol)
    }
}

fn validate_buffers(
    receive: SharedBuffer,
    send: SharedBuffer,
    request_bytes: &[u8],
    response_bytes: &[u8],
) -> Result<(), ServerError> {
    if !receive.writable
        || send.writable
        || receive.length as usize != request_bytes.len()
        || send.length as usize != response_bytes.len()
        || request_bytes.is_empty()
        || response_bytes.is_empty()
    {
        Err(ServerError::BufferDescriptor)
    } else {
        Ok(())
    }
}

fn sub_buffer(
    base: SharedBuffer,
    relative_offset: usize,
    length: usize,
    writable: bool,
) -> Result<SharedBuffer, ServerError> {
    if length == 0 {
        return Err(ServerError::BufferDescriptor);
    }
    let relative_offset =
        u32::try_from(relative_offset).map_err(|_| ServerError::BufferDescriptor)?;
    let length = u32::try_from(length).map_err(|_| ServerError::BufferDescriptor)?;
    let offset = base
        .offset
        .checked_add(relative_offset)
        .ok_or(ServerError::BufferDescriptor)?;
    Ok(SharedBuffer {
        region: base.region,
        offset,
        length,
        writable,
    })
}
