#![no_main]

use libfuzzer_sys::fuzz_target;
use syn_script::Script;

fuzz_target!(|data: &[u8]| {
    let source = String::from_utf8_lossy(data);
    let _ = Script::<64>::compile(&source);
});
