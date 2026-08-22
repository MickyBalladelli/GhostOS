#![no_main]

use libfuzzer_sys::fuzz_target;
use std::fs;
use ghostos_vm::DiskImage;

fuzz_target!(|data: &[u8]| {
    let path = std::env::temp_dir().join(format!(
        "ghostos-vm-fuzz-image-{}",
        std::process::id()
    ));
    let mut image = vec![0u8; data.len().max(512).min(1024 * 1024).div_ceil(512) * 512];
    let length = data.len().min(image.len());
    image[..length].copy_from_slice(&data[..length]);
    if fs::write(&path, image).is_ok() {
        let _ = DiskImage::open_with_access(&path, false);
        let _ = fs::remove_file(path);
    }
});
