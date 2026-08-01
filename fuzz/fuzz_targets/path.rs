#![no_main]

use libfuzzer_sys::fuzz_target;
use synos_fsd::NamespacePath;
use synos_synfs::{FileName, VersionedPath};

fuzz_target!(|data: &[u8]| {
    let value = String::from_utf8_lossy(data);
    let _ = FileName::new(&value);
    let _ = VersionedPath::parse(&value);
    let _ = NamespacePath::new(&value);
});
