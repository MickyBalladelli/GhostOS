#![no_main]

use libfuzzer_sys::fuzz_target;
use ghostos_abi::Request;
use ghostos_auth::{CapabilityKey, CryptographicCapability, TransportRights};
use ghostos_kernel::Rights;

fuzz_target!(|data: &[u8]| {
    if data.len() >= 64 {
        let request = Request {
            operation: u16::from_le_bytes([data[0], data[1]]),
            abi_version: u16::from_le_bytes([data[2], data[3]]),
            flags: u16::from_le_bytes([data[4], data[5]]),
            reserved: u16::from_le_bytes([data[6], data[7]]),
            capability: read_u64(data, 8),
            arguments: core::array::from_fn(|index| read_u64(data, 16 + index * 8)),
        };
        let _ = ghostos_kernel::syscall::validate_request_shape(&request);
    }

    if data.len() >= CryptographicCapability::WIRE_BYTES {
        let mut wire = [0; CryptographicCapability::WIRE_BYTES];
        wire.copy_from_slice(&data[..CryptographicCapability::WIRE_BYTES]);
        if let Ok(token) = CryptographicCapability::decode(wire) {
            let now_us = token.not_before_us;
            let _ = token.verify(
                CapabilityKey::new([0x5a; 32]),
                token.subject,
                token.effective_rights(),
                token.effective_transports(),
                now_us,
                token.revocation_epoch,
            );
            let _ = token.verify(
                CapabilityKey::new([0x5a; 32]),
                token.subject,
                Rights::ALL,
                TransportRights::LAYER2,
                read_u64(data, 0),
                read_u64(data, 8),
            );
        }
    }
});

fn read_u64(data: &[u8], offset: usize) -> u64 {
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&data[offset..offset + 8]);
    u64::from_le_bytes(bytes)
}
