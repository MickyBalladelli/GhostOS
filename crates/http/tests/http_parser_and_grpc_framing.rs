use ghostos_http::{
    GrpcError, Method, ParseError, StatusCode, decode_grpc_frame, encode_error_body,
    encode_grpc_frame, encode_response, parse_request,
};
use ghostos_status::{AuditContext, PublicError, RetryHint, Status};

#[test]
fn http_parser_handles_query_body_and_rejects_ambiguous_lengths() {
    let request = b"POST /run?x=1 HTTP/1.1\r\nHost: ghostos\r\nContent-Length: 3\r\n\r\nabcEXTRA";
    let parsed = parse_request::<4>(request).unwrap();
    assert_eq!(parsed.request.method, Method::Post);
    assert_eq!(parsed.request.path_without_query(), "/run");
    assert_eq!(parsed.request.query(), Some("x=1"));
    assert_eq!(parsed.request.body, b"abc");
    assert_eq!(parsed.consumed, request.len() - 5);
    assert_eq!(
        parse_request::<4>(b"GET / HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\nx"),
        Err(ParseError::InvalidContentLength)
    );
    assert_eq!(parse_request::<4>(b"GET / HTTP/2\r\n\r\n"), Err(ParseError::UnsupportedVersion));
}

#[test]
fn grpc_framing_rejects_compression_and_truncation() {
    let mut frame = [0; 16];
    let length = encode_grpc_frame(false, b"ping", &mut frame).unwrap();
    let (compressed, message, consumed) = decode_grpc_frame(&frame[..length]).unwrap();
    assert!(!compressed);
    assert_eq!(message, b"ping");
    assert_eq!(consumed, length);
    frame[0] = 1;
    assert_eq!(decode_grpc_frame(&frame[..length]).unwrap().0, true);
    frame[1] = 0xff;
    assert_eq!(decode_grpc_frame(&frame[..length]), Err(GrpcError::InvalidFrame));
    assert_eq!(decode_grpc_frame(&[0, 0, 0]), Err(GrpcError::InvalidFrame));
}

#[test]
fn http_response_encoder_preserves_status_and_capacity_limits() {
    let response = ghostos_http::Response::<2>::new(StatusCode::CREATED, b"ok")
        .with_header("content-type", "text/plain")
        .unwrap();
    let mut output = [0; 128];
    let written = encode_response(response, &mut output).unwrap();
    assert!(core::str::from_utf8(&output[..written]).unwrap().starts_with("HTTP/1.1 201 Created"));
    assert!(matches!(encode_response(response, &mut [0; 4]), Err(ghostos_http::EncodeError::BufferTooSmall { .. })));
}

#[test]
fn public_http_error_contains_contract_fields_without_request_data() {
    let error = PublicError::new(
        Status::ACCESS_DENIED,
        ghostos_status::operation::HTTP_ROUTE,
        RetryHint::Never,
        AuditContext::new(0xfeed, 2),
    );
    let mut body = [0; 512];
    let length = encode_error_body(error, &mut body).unwrap();
    let body = core::str::from_utf8(&body[..length]).unwrap();

    assert!(body.contains("\"code\":"));
    assert!(body.contains("\"operation\":2"));
    assert!(body.contains("\"retry\":\"never\""));
    assert!(body.contains("0000000000000000000000000000feed"));
    assert!(body.contains("\"node\":2"));
    assert!(!body.contains("capability-secret"));
}

#[test]
fn grpc_errors_expose_safe_retry_and_audit_metadata() {
    let error = GrpcError::AccessDenied.public_error(ghostos_status::AuditContext::new(7, 4));
    assert_eq!(error.code, Status::ACCESS_DENIED);
    assert_eq!(error.operation, ghostos_status::operation::GRPC);
    assert_eq!(error.audit.correlation, 7);
    assert_eq!(error.audit.node, 4);
    assert_eq!(error.retry, RetryHint::Never);
}
