use ghostos_wasm_script::{
    CapabilityGrant, HOST_ACCESS_DENIED, HostCall, WasmHost, WasmLimits, WasmRuntime,
};

const RETURN_FIRST_ARGUMENT: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x07, 0x01, 0x60, 0x02, 0x7e, 0x7e, 0x01, 0x7e,
    0x03, 0x02, 0x01, 0x00,
    0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6e, 0x00, 0x00,
    0x0a, 0x06, 0x01, 0x04, 0x00, 0x20, 0x00, 0x0b,
];

const INVOKE_HOST: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x11, 0x02,
    0x60, 0x06, 0x7e, 0x7f, 0x7e, 0x7e, 0x7e, 0x7e, 0x01, 0x7e,
    0x60, 0x02, 0x7e, 0x7e, 0x01, 0x7e,
    0x02, 0x10, 0x01, 0x05, 0x73, 0x79, 0x6e, 0x6f, 0x73,
    0x06, 0x69, 0x6e, 0x76, 0x6f, 0x6b, 0x65, 0x00, 0x00,
    0x03, 0x02, 0x01, 0x01,
    0x07, 0x07, 0x01, 0x03, 0x72, 0x75, 0x6e, 0x00, 0x01,
    0x0a, 0x12, 0x01, 0x10, 0x00,
    0x42, 0x01, 0x41, 0x00, 0x42, 0x07,
    0x42, 0x00, 0x42, 0x00, 0x42, 0x00,
    0x10, 0x00, 0x0b,
];

#[derive(Default)]
struct Host {
    calls: Vec<HostCall>,
}

impl WasmHost for Host {
    fn invoke(&mut self, call: HostCall) -> i64 {
        self.calls.push(call);
        77
    }
}

#[test]
fn wasm_runtime_runs_deterministically_and_limits_host_access() {
    let runtime = WasmRuntime::default();
    let first = runtime
        .execute(RETURN_FIRST_ARGUMENT, "run", [42, 0], &[], ())
        .expect("run deterministic wasm");
    let second = runtime
        .execute(RETURN_FIRST_ARGUMENT, "run", [42, 0], &[], ())
        .expect("run wasm again");
    assert_eq!(first.value, 42);
    assert_eq!(first.value, second.value);
    assert!(first.fuel_consumed > 0);

    let grant = CapabilityGrant::single(1, 0).unwrap();
    let allowed = runtime
        .execute(INVOKE_HOST, "run", [0, 0], &[grant], Host::default())
        .expect("invoke authorized host operation");
    assert_eq!(allowed.value, 77);
    assert_eq!(allowed.host.calls.len(), 1);

    let denied = runtime
        .execute(INVOKE_HOST, "run", [0, 0], &[], Host::default())
        .expect("denied host call returns sentinel");
    assert_eq!(denied.value, HOST_ACCESS_DENIED);
    assert_eq!(denied.denied_calls, 1);
}

#[test]
fn wasm_runtime_rejects_invalid_entry_modules_and_limits() {
    let runtime = WasmRuntime::default();
    assert!(matches!(runtime.execute(RETURN_FIRST_ARGUMENT, "", [0, 0], &[], ()), Err(error) if error.kind() == ghostos_wasm_script::ErrorKind::InvalidEntry));
    assert!(runtime.execute(&[0, 1, 2], "run", [0, 0], &[], ()).is_err());
    let mut limits = WasmLimits::DEFAULT;
    limits.fuel = 0;
    assert!(WasmRuntime::new(limits).is_err());
}
