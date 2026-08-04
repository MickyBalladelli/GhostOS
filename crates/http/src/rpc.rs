use crate::{DEFAULT_RESPONSE_HEADERS, Method, Request, Response, StatusCode};

pub const SYNOS_RPC_CONTENT_TYPE: &str = "application/vnd.synos.rpc";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RpcHttpError {
    MethodNotAllowed,
    InvalidContentType,
    EmptyBody,
    BufferTooSmall { required: usize },
    Encode(crate::EncodeError),
}

pub fn is_rpc_content_type(value: &str) -> bool {
    value
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case(SYNOS_RPC_CONTENT_TYPE))
}

pub fn decode_rpc_request<'a>(
    request: Request<'a>,
) -> Result<&'a [u8], RpcHttpError> {
    if request.method != Method::Post {
        return Err(RpcHttpError::MethodNotAllowed);
    }
    let content_type = request
        .header("content-type")
        .ok_or(RpcHttpError::InvalidContentType)?;
    if !is_rpc_content_type(content_type) {
        return Err(RpcHttpError::InvalidContentType);
    }
    if request.body.is_empty() {
        return Err(RpcHttpError::EmptyBody);
    }
    Ok(request.body)
}

pub fn rpc_response<'a>(
    frame: &'a [u8],
    destination: &'a mut [u8],
) -> Result<Response<'a, DEFAULT_RESPONSE_HEADERS>, RpcHttpError> {
    if frame.is_empty() {
        return Err(RpcHttpError::EmptyBody);
    }
    if destination.len() < frame.len() {
        return Err(RpcHttpError::BufferTooSmall { required: frame.len() });
    }
    destination[..frame.len()].copy_from_slice(frame);
    Response::new(StatusCode::OK, &destination[..frame.len()])
        .with_header("content-type", SYNOS_RPC_CONTENT_TYPE)
        .map_err(RpcHttpError::Encode)
}
