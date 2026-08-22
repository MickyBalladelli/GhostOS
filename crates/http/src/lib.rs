#![no_std]
#![forbid(unsafe_code)]

//! Capability-constrained HTTP and gRPC building blocks for Ring 3 services.
//!
//! The API keeps parsing, routing, and transport separate. `HttpServer` talks
//! to networking only through `ghostos-netd` IPC requests carrying an
//! owner-bound `SocketCapability`.

pub use ghostos_protocol::{ProtocolError, ProtocolGuard, ProtocolLimits, TrafficClass, VersionRange};
pub use ghostos_service_scale::{
    EffectReceipt, HandoffReceipt, HandoffToken, HttpScale, InstanceId, InstanceState, JoinReceipt,
    RequestId, RouteDecision, ServiceKind, SessionId, SessionState, ScaleError, ScaleSnapshot,
};

mod grpc;
mod http;
mod netd;
mod rpc;
mod router;
mod server;
mod error;

pub use grpc::{
    GrpcError, GrpcHandler, GrpcRequest, GrpcRouter, GrpcStatus, decode_grpc_frame,
    decode_grpc_frame_checked, decode_grpc_frame_guarded, encode_grpc_frame,
    encode_grpc_frame_guarded,
};
pub use error::{
    encode_error_body, encode_error_http_response, error_response, parse_error, route_error,
    status_code,
};
pub use http::{
    DEFAULT_REQUEST_HEADERS, DEFAULT_RESPONSE_HEADERS, EncodeError, Header, Method, ParseError,
    ParsedRequest, Request, Response, StatusCode, Version, encode_response, parse_request,
    parse_request_checked,
};
pub use netd::{NetdClient, NetdError};
pub use rpc::{
    RpcHttpError, GHOSTOS_RPC_CONTENT_TYPE, decode_rpc_request, is_rpc_content_type,
    rpc_error_response, rpc_response, rpc_response_loaned,
};
pub use router::{Handler, RequestContext, Route, RouteError, Router, WebRights};
pub use server::{HttpServer, SERVER_SOCKET_RIGHTS, ServerError, ServerEvent, ServerState};

#[cfg(test)]
mod tests;
