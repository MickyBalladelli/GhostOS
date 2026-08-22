use super::{decode_grpc_frame, encode_grpc_frame, parse_request, Method, ParseError, Version};

#[test]
fn parser_accepts_generated_content_lengths_only_when_complete() {
    use ghostos_test_support::property::{run_assert, Config};

    run_assert("http.content-length", Config::new(0x59_3, 128), |_, _, _| {
        let bytes = b"POST /data HTTP/1.1\r\ncontent-length: 8\r\n\r\n12345678";
        let Ok(parsed) = parse_request::<8>(bytes) else { return false };
        if parsed.request.method != Method::Post
            || parsed.request.version != Version::Http11
            || parsed.request.body != b"12345678"
        {
            return false;
        }
        let truncated = &bytes[..bytes.len() - 1];
        parse_request::<8>(truncated) == Err(ParseError::Incomplete)
    })
    .expect("generated HTTP lengths preserve framing");
}

#[test]
fn grpc_frame_round_trips_generated_messages() {
    use ghostos_test_support::property::{run_assert, Config};

    run_assert("http.grpc-frame", Config::new(0x59_3, 128), |_, _, entropy| {
        let length = (entropy.next_u64() as usize) % 129;
        let mut message = [0; 128];
        entropy.fill_bytes(&mut message[..length]);
        let mut frame = [0; 133];
        let Ok(encoded) = encode_grpc_frame(false, &message[..length], &mut frame) else {
            return false;
        };
        let Ok((compressed, decoded, consumed)) = decode_grpc_frame(&frame[..encoded]) else {
            return false;
        };
        !compressed && consumed == encoded && decoded == &message[..length]
    })
    .expect("generated gRPC frames round-trip");
}
