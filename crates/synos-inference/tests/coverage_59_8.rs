use synos_inference::{Error, protocol::{CompletionKind, CompletionResponse, GrpcCodec, ModelName, OpenAiCodec, OpenAiRequest}};

#[test]
fn openai_protocol_validates_requests_and_escapes_responses() {
    let request = OpenAiCodec::decode(
        "/v1/completions",
        br#"{"model":"tiny","prompt":"hello","max_tokens":4,"stream":true}"#,
    )
    .unwrap();
    match request {
        OpenAiRequest::Complete(request) => {
            assert_eq!(request.model, "tiny");
            assert_eq!(request.prompt, "hello");
            assert_eq!(request.max_tokens, 4);
            assert!(request.stream);
            assert_eq!(request.kind, CompletionKind::Text);
        }
        OpenAiRequest::ListModels => panic!("wrong request kind"),
    }
    assert!(matches!(OpenAiCodec::decode("/v1/unknown", b""), Err(Error::UnsupportedEndpoint)));
    assert!(matches!(OpenAiCodec::decode("/v1/completions", br#"{"model":"tiny","prompt":"x","max_tokens":0}"#), Err(Error::InvalidRequest)));

    let mut output = [0; 256];
    let length = OpenAiCodec::encode_completion(
        CompletionResponse {
            id: 3,
            model: "tiny",
            text: "a\"b",
            kind: CompletionKind::Chat,
            created_at: 9,
            prompt_tokens: 2,
            completion_tokens: 1,
            finished: true,
        },
        &mut output,
    )
    .unwrap();
    let encoded = core::str::from_utf8(&output[..length]).unwrap();
    assert!(encoded.contains("chatcmpl-3"));
    assert!(encoded.contains("a\\\"b"));
}

#[test]
fn grpc_codec_round_trips_and_rejects_bad_frames() {
    let response = CompletionResponse {
        id: 8,
        model: "tiny",
        text: "ok",
        kind: CompletionKind::Text,
        created_at: 0,
        prompt_tokens: 2,
        completion_tokens: 1,
        finished: true,
    };
    let mut encoded = [0; 128];
    let length = GrpcCodec::encode(response, &mut encoded).unwrap();
    assert_eq!(encoded[0], 0);
    encoded[0] = 1;
    assert_eq!(GrpcCodec::decode(&encoded[..length]), Err(Error::UnsupportedProtocol));
    encoded[0] = 0;
    encoded[4] = encoded[4].wrapping_add(1u8);
    assert_eq!(GrpcCodec::decode(&encoded[..length]), Err(Error::InvalidRequest));
    assert_eq!(ModelName::new("").unwrap_err(), Error::InvalidRequest);
    assert_eq!(ModelName::new("tiny").unwrap().as_str(), "tiny");
}
