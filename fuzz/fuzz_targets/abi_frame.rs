#![no_main]

use libfuzzer_sys::fuzz_target;
use ghostos_abi::{RpcFrameHeader, RPC_FRAME_HEADER_BYTES, RPC_MAX_FRAME_BYTES};

fuzz_target!(|data: &[u8]| {
    let bounded = &data[..data.len().min(RPC_MAX_FRAME_BYTES)];
    let _ = RpcFrameHeader::decode(bounded);

    if let Ok(header) = RpcFrameHeader::decode(bounded) {
        let mut encoded = [0; RPC_FRAME_HEADER_BYTES];
        if header.encode(&mut encoded).is_ok() {
            let _ = RpcFrameHeader::decode(&encoded);
        }
    }
});
