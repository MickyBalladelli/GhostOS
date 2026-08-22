#![no_main]

use libfuzzer_sys::fuzz_target;
use ghostos_ghostfs::SynFs;

const MAX_BLOCKS: usize = 64;

fuzz_target!(|data: &[u8]| {
    let mut image = [0; SynFs::<MAX_BLOCKS>::volume_bytes()];
    let length = data.len().min(image.len());
    image[..length].copy_from_slice(&data[..length]);

    if let Ok(filesystem) = SynFs::<MAX_BLOCKS>::load(&image) {
        let _ = filesystem.check_consistency();
    }
    if let Ok(filesystem) = SynFs::<MAX_BLOCKS>::recover(&image) {
        let _ = filesystem.check_consistency();
    }
});
