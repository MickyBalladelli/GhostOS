#![no_main]

use libfuzzer_sys::fuzz_target;
use synos_app::AppManifest;

fuzz_target!(|data: &[u8]| {
    let source = String::from_utf8_lossy(&data[..data.len().min(64 * 1024)]);
    if let Ok(manifest) = AppManifest::parse(&source) {
        let _ = manifest.validate_profile();
        let _ = manifest.capabilities().count();
        let _ = manifest.dependencies().count();
    }
});
