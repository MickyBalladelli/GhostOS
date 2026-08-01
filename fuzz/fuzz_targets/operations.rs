#![no_main]

use libfuzzer_sys::fuzz_target;
use synos_synfs::SynFs;

const MAX_BLOCKS: usize = 64;

fuzz_target!(|data: &[u8]| {
    let mut filesystem = SynFs::<MAX_BLOCKS>::new();
    for (index, chunk) in data.chunks(8).take(128).enumerate() {
        let path = format!("/f{index}");
        if chunk.first().is_some_and(|byte| byte & 1 != 0) {
            let _ = filesystem.write(&path, chunk);
        } else {
            let _ = filesystem.create_directory(&format!("/d{index}"), false);
        }
        if chunk.get(1).is_some_and(|byte| byte & 1 != 0) {
            let _ = filesystem.lookup(&path);
        }
    }

    let _ = filesystem.check_consistency();
    let mut image = [0; SynFs::<MAX_BLOCKS>::volume_bytes()];
    if filesystem.flush(&mut image).is_ok() {
        let _ = SynFs::<MAX_BLOCKS>::recover(&image);
    }
});
