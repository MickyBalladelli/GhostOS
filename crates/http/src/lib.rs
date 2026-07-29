#![no_std]
#![forbid(unsafe_code)]

//! Capability-constrained HTTP and gRPC building blocks for Ring 3 services.
//!
//! The API keeps parsing, routing, and transport separate. `HttpServer` talks
//! to networking only through `synos-netd` IPC requests carrying an
//! owner-bound `SocketCapability`.

mod grpc;
mod http;
mod netd;
mod router;
mod server;

pub use grpc::{
    GrpcError, GrpcHandler, GrpcRequest, GrpcRouter, GrpcStatus, decode_grpc_frame,
    encode_grpc_frame,
};
pub use http::{
    DEFAULT_REQUEST_HEADERS, DEFAULT_RESPONSE_HEADERS, EncodeError, Header, Method, ParseError,
    ParsedRequest, Request, Response, StatusCode, Version, encode_response, parse_request,
};
pub use netd::{NetdClient, NetdError};
pub use router::{Handler, RequestContext, Route, RouteError, Router, WebRights};
pub use server::{HttpServer, SERVER_SOCKET_RIGHTS, ServerError, ServerEvent, ServerState};
