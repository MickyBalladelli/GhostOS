use ghostos_embedded_script::{
    EmbeddedScriptEngine, Error, ScriptCapability, ScriptLimits,
};

#[test]
fn embedded_scripts_are_capability_gated_and_bounded() {
    let engine = EmbeddedScriptEngine::default();
    let capability = ScriptCapability::single("filesystem", 1).unwrap();
    let outcome = engine
        .execute(
            r#"if ghostos.has("filesystem", 1) { ghostos.request("filesystem", 1, "read") } else { 0 }"#,
            &[capability],
        )
        .expect("execute authorized script");
    assert_eq!(outcome.requests.len(), 1);
    assert_eq!(outcome.requests[0].operation, 1);
    assert_eq!(outcome.requests[0].payload, "read");

    let denied = engine.execute(r#"ghostos.request("filesystem", 1, "read")"#, &[]);
    assert!(matches!(denied, Err(error) if error.kind() == ghostos_embedded_script::ErrorKind::Evaluate));

    let mut limits = ScriptLimits::DEFAULT;
    limits.max_operations = 0;
    assert!(matches!(EmbeddedScriptEngine::new(limits), Err(Error::InvalidLimits)));
    assert!(matches!(
        ScriptCapability::single("filesystem", 64),
        Err(Error::InvalidOperation)
    ));
    assert!(matches!(
        ScriptCapability::new("filesystem", 0),
        Err(Error::InvalidCapability)
    ));
}

#[test]
fn embedded_script_request_and_source_limits_fail_cleanly() {
    let mut limits = ScriptLimits::DEFAULT;
    limits.max_requests = 1;
    limits.max_source_bytes = 4;
    let engine = EmbeddedScriptEngine::new(limits).unwrap();
    let source_error = match engine.execute("12345", &[]) {
        Ok(_) => panic!("source limit should reject input"),
        Err(error) => error,
    };
    assert_eq!(source_error.kind(), ghostos_embedded_script::ErrorKind::SourceTooLarge);
    let capability = ScriptCapability::single("fs", 0).unwrap();
    let mut request_limits = ScriptLimits::DEFAULT;
    request_limits.max_requests = 1;
    let request_engine = EmbeddedScriptEngine::new(request_limits).unwrap();
    let error = match request_engine.execute(
            r#"ghostos.request("fs", 0, "a"); ghostos.request("fs", 0, "b")"#,
            &[capability],
        ) {
        Ok(_) => panic!("request limit should reject second request"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), ghostos_embedded_script::ErrorKind::Evaluate);
}
