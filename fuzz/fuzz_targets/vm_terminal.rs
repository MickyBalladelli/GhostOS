#![no_main]

use libfuzzer_sys::fuzz_target;
use synos_vm::{ascii_to_scancodes, translate_input_bytes, ReplaySession};

fuzz_target!(|data: &[u8]| {
    let data = &data[..data.len().min(64 * 1024)];
    let translated = translate_input_bytes(data);
    assert_eq!(translated, translate_input_bytes(data));
    for byte in data.iter().copied().take(4096) {
        let _ = ascii_to_scancodes(byte);
    }

    let mut session = ReplaySession::with_max_events(1);
    session.begin_recording();
    session.host_input(1, None, None, &translated).unwrap();
    let trace = session.trace();
    session.begin_replay(trace).unwrap();
    let replayed = session.next_host_input().unwrap().unwrap();
    assert_eq!(replayed.bytes, translated);
});
