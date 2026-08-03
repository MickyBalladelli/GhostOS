#![no_main]

use libfuzzer_sys::fuzz_target;
use synos_http::{decode_grpc_frame, parse_request};

fuzz_target!(|data: &[u8]| {
    let _ = parse_request::<32>(data);
    let _ = decode_grpc_frame(data);
});
