#![no_std]
#![forbid(unsafe_code)]

//! Generated GhostOS ABI. Edit abi/ghostos-abi.toml, then run
//! python3 tools/generate_abi.py.

include!("generated.rs");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incompatible_frames_have_stable_failure_contract() {
        let mut frame = [0; RPC_FRAME_HEADER_BYTES];
        RpcFrameHeader {
            method: RpcMethod::ClusterState,
            flags: 0,
            request_id: 7,
            payload_bytes: 0,
            status: RpcStatus::Ok,
        }
        .encode(&mut frame)
        .expect("valid frame");
        frame[4] = RPC_PROTOCOL_VERSION.saturating_add(1);

        assert_eq!(
            RpcFrameHeader::decode(&frame),
            Err(FrameError::UnsupportedVersion)
        );
        assert_eq!(RpcStatus::ProtocolMismatch as u16, 8);
    }

    #[test]
    fn syscall_request_carries_generated_schema_version() {
        let request = Request::new(Operation::ClockNow);
        assert_eq!(request.abi_version, ABI_SCHEMA_VERSION);
        assert_eq!(core::mem::size_of::<Request>(), 64);
    }
}
