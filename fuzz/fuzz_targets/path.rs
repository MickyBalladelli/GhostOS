#![no_main]

use libfuzzer_sys::fuzz_target;
use ghostos_fsd::NamespacePath;
use ghostos_ghostfs::{FileName, VersionedPath};

fuzz_target!(|data: &[u8]| {
    let value = String::from_utf8_lossy(data);
    let _ = FileName::new(&value);
    let _ = VersionedPath::parse(&value);
    let _ = NamespacePath::new(&value);
});
