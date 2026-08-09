#![no_main]

use libfuzzer_sys::fuzz_target;
use synos_vm::{SnapshotAuthKey, VmSnapshot};

const KEY: SnapshotAuthKey = SnapshotAuthKey::new([0x53; 32]);

fuzz_target!(|data: &[u8]| {
    let data = &data[..data.len().min(1024 * 1024)];
    let _ = VmSnapshot::from_bytes(data);
    let _ = VmSnapshot::from_authenticated_bytes(data, KEY);

    if let Ok(snapshot) = VmSnapshot::from_bytes(data) {
        let encoded = snapshot.to_bytes();
        let decoded = VmSnapshot::from_bytes(&encoded).expect("re-encoded snapshot must decode");
        assert_eq!(decoded, snapshot);
    }
});
