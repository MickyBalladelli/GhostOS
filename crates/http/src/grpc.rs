use crate::{DEFAULT_RESPONSE_HEADERS, Method, RequestContext, Response, StatusCode, WebRights};
use ghostos_status::{AuditContext, PublicError, Status};
use ghostos_ipc::{BufferError, BufferLease, BufferOwner};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum GrpcStatus {
    Ok = 0,
    Cancelled = 1,
    Unknown = 2,
    InvalidArgument = 3,
    NotFound = 5,
    PermissionDenied = 7,
    ResourceExhausted = 8,
    Unimplemented = 12,
    Internal = 13,
    Unavailable = 14,
}

impl GrpcStatus {
    pub const fn public_status(self) -> Status {
        match self {
            Self::Ok => Status::NORMAL,
            Self::Cancelled => Status::CANCELLED,
            Self::InvalidArgument => Status::INVALID_ARGUMENT,
            Self::NotFound | Self::Unimplemented => Status::NOT_FOUND,
            Self::PermissionDenied => Status::ACCESS_DENIED,
            Self::ResourceExhausted => Status::NO_SPACE,
            Self::Unavailable => Status::BUSY,
            Self::Unknown | Self::Internal => Status::INTERNAL,
        }
    }

    pub const fn public_error(self, audit: AuditContext) -> PublicError {
        self.public_status()
            .public_error(ghostos_status::operation::GRPC, audit)
    }
}

impl GrpcStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "0",
            Self::Cancelled => "1",
            Self::Unknown => "2",
            Self::InvalidArgument => "3",
            Self::NotFound => "5",
            Self::PermissionDenied => "7",
            Self::ResourceExhausted => "8",
            Self::Unimplemented => "12",
            Self::Internal => "13",
            Self::Unavailable => "14",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GrpcRequest<'a> {
    pub principal: u64,
    pub service_method: &'a str,
    pub message: &'a [u8],
}

pub type GrpcHandler<State> = for<'request, 'response> fn(
    &mut State,
    GrpcRequest<'request>,
    &'response mut [u8],
) -> Result<usize, GrpcStatus>;

struct GrpcRoute<State> {
    path: &'static str,
    required_rights: WebRights,
    handler: GrpcHandler<State>,
}

impl<State> Copy for GrpcRoute<State> {}

impl<State> Clone for GrpcRoute<State> {
    fn clone(&self) -> Self {
        *self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrpcError {
    AccessDenied,
    BufferTooSmall { required: usize },
    Capacity,
    Compressed,
    Duplicate,
    InvalidContentType,
    InvalidFrame,
    InvalidPath,
    MethodNotAllowed,
    NotFound,
    Protocol(ghostos_protocol::ProtocolError),
    Buffer(BufferError),
}

impl GrpcError {
    pub const fn public_error(self, audit: AuditContext) -> PublicError {
        let status = match self {
            Self::AccessDenied => Status::ACCESS_DENIED,
            Self::BufferTooSmall { .. } | Self::Capacity => Status::NO_SPACE,
            Self::Compressed
            | Self::InvalidContentType
            | Self::InvalidFrame
            | Self::InvalidPath
            | Self::MethodNotAllowed => Status::INVALID_ARGUMENT,
            Self::Duplicate => Status::ALREADY_EXISTS,
            Self::NotFound => Status::NOT_FOUND,
            Self::Protocol(_) => Status::INVALID_ARGUMENT,
            Self::Buffer(_) => Status::ACCESS_DENIED,
        };
        status.public_error(ghostos_status::operation::GRPC, audit)
    }
}

pub struct GrpcRouter<State, const SERVICES: usize> {
    state: State,
    routes: [Option<GrpcRoute<State>>; SERVICES],
    count: usize,
}

impl<State, const SERVICES: usize> GrpcRouter<State, SERVICES> {
    pub const fn new(state: State) -> Self {
        Self {
            state,
            routes: [None; SERVICES],
            count: 0,
        }
    }

    pub fn service(
        &mut self,
        path: &'static str,
        required_rights: WebRights,
        handler: GrpcHandler<State>,
    ) -> Result<(), GrpcError> {
        if !valid_grpc_path(path) {
            return Err(GrpcError::InvalidPath);
        }
        if self.routes[..self.count]
            .iter()
            .flatten()
            .any(|route| route.path == path)
        {
            return Err(GrpcError::Duplicate);
        }
        let slot = self.routes.get_mut(self.count).ok_or(GrpcError::Capacity)?;
        *slot = Some(GrpcRoute {
            path,
            required_rights,
            handler,
        });
        self.count += 1;
        Ok(())
    }

    pub fn handle<'response>(
        &mut self,
        context: RequestContext<'_>,
        granted_rights: WebRights,
        message_scratch: &'response mut [u8],
        framed_response: &'response mut [u8],
    ) -> Result<Response<'response, DEFAULT_RESPONSE_HEADERS>, GrpcError> {
        if context.request.method != Method::Post {
            return Err(GrpcError::MethodNotAllowed);
        }
        let content_type = context
            .request
            .header("content-type")
            .ok_or(GrpcError::InvalidContentType)?;
        if !content_type
            .split(';')
            .next()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/grpc"))
        {
            return Err(GrpcError::InvalidContentType);
        }
        let (compressed, message, consumed) = decode_grpc_frame(context.request.body)?;
        if compressed {
            return Err(GrpcError::Compressed);
        }
        if consumed != context.request.body.len() {
            return Err(GrpcError::InvalidFrame);
        }
        let path = context.request.path_without_query();
        let route = self.routes[..self.count]
            .iter()
            .flatten()
            .find(|route| route.path == path)
            .copied()
            .ok_or(GrpcError::NotFound)?;
        if !granted_rights.contains(route.required_rights) {
            return Err(GrpcError::AccessDenied);
        }
        let length = (route.handler)(
            &mut self.state,
            GrpcRequest {
                principal: context.principal,
                service_method: path,
                message,
            },
            message_scratch,
        )
        .map_err(status_error)?;
        let message = message_scratch
            .get(..length)
            .ok_or(GrpcError::BufferTooSmall { required: length })?;
        let framed = encode_grpc_frame(false, message, framed_response)?;
        Response::new(StatusCode::OK, &framed_response[..framed])
            .with_header("content-type", "application/grpc")
            .and_then(|response| response.with_header("grpc-status", GrpcStatus::Ok.as_str()))
            .map_err(|_| GrpcError::Capacity)
    }

    pub const fn state(&self) -> &State {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }

    pub fn into_state(self) -> State {
        self.state
    }
}

impl<State: Default, const SERVICES: usize> Default for GrpcRouter<State, SERVICES> {
    fn default() -> Self {
        Self::new(State::default())
    }
}

pub fn decode_grpc_frame(bytes: &[u8]) -> Result<(bool, &[u8], usize), GrpcError> {
    if bytes.len() < 5 || bytes[0] > 1 {
        return Err(GrpcError::InvalidFrame);
    }
    let length = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
    let consumed = length.checked_add(5).ok_or(GrpcError::InvalidFrame)?;
    let message = bytes.get(5..consumed).ok_or(GrpcError::InvalidFrame)?;
    Ok((bytes[0] == 1, message, consumed))
}

pub fn decode_grpc_frame_guarded<'a>(
    bytes: &'a BufferLease<'a>,
) -> Result<(bool, &'a [u8], usize), GrpcError> {
    if !matches!(bytes.owner(), BufferOwner::RpcTransport | BufferOwner::RpcClient) {
        return Err(GrpcError::Buffer(BufferError::OwnerMismatch));
    }
    decode_grpc_frame(bytes.as_slice().map_err(GrpcError::Buffer)?)
}

pub fn decode_grpc_frame_checked<'a>(
    guard: &mut ghostos_protocol::ProtocolGuard,
    sequence: u64,
    bytes: &'a [u8],
) -> Result<(bool, &'a [u8], usize), GrpcError> {
    guard
        .require_class(ghostos_protocol::TrafficClass::Grpc)
        .map_err(GrpcError::Protocol)?;
    guard
        .validate_message(bytes.len())
        .map_err(GrpcError::Protocol)?;
    let frame = decode_grpc_frame(bytes)?;
    guard
        .accept_sequence(sequence)
        .map_err(GrpcError::Protocol)?;
    Ok(frame)
}

pub fn encode_grpc_frame(
    compressed: bool,
    message: &[u8],
    destination: &mut [u8],
) -> Result<usize, GrpcError> {
    let length = u32::try_from(message.len()).map_err(|_| GrpcError::InvalidFrame)?;
    let required = message
        .len()
        .checked_add(5)
        .ok_or(GrpcError::InvalidFrame)?;
    let destination = destination
        .get_mut(..required)
        .ok_or(GrpcError::BufferTooSmall { required })?;
    destination[0] = compressed as u8;
    destination[1..5].copy_from_slice(&length.to_be_bytes());
    destination[5..].copy_from_slice(message);
    Ok(required)
}

pub fn encode_grpc_frame_guarded(
    compressed: bool,
    message: &[u8],
    destination: &mut BufferLease<'_>,
) -> Result<usize, GrpcError> {
    if destination.owner() != BufferOwner::RpcTransport {
        return Err(GrpcError::Buffer(BufferError::OwnerMismatch));
    }
    let output = destination
        .as_mut_slice()
        .map_err(GrpcError::Buffer)?;
    encode_grpc_frame(compressed, message, output)
}

fn valid_grpc_path(path: &str) -> bool {
    let Some(value) = path.strip_prefix('/') else {
        return false;
    };
    let Some((service, method)) = value.split_once('/') else {
        return false;
    };
    !service.is_empty() && !method.is_empty() && !method.contains('/')
}

const fn status_error(status: GrpcStatus) -> GrpcError {
    match status {
        GrpcStatus::PermissionDenied => GrpcError::AccessDenied,
        GrpcStatus::NotFound | GrpcStatus::Unimplemented => GrpcError::NotFound,
        GrpcStatus::ResourceExhausted => GrpcError::Capacity,
        _ => GrpcError::InvalidFrame,
    }
}
