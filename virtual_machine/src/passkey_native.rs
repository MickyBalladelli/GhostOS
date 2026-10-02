//! Owned buffers and native I/O adapters for the C passkey bridge.
use super::{Mode, InputFlow, PasskeyBridge, Request};
use std::io;

#[repr(C)]
#[derive(Default)]
struct CRequest {
    method_start: usize, method_length: usize, target_start: usize, target_length: usize,
    token_start: usize, token_length: usize, body_start: usize, body_length: usize,
    has_token: bool,
}
#[repr(C)]
struct State { mode: u32, committed: bool, login_in_progress: bool }
#[repr(C)]
#[derive(Default)]
struct Observation { challenge_action: u32, error_action: u32, challenge_start: usize, clear_flow: bool }

pub(super) fn asset(kind: u32) -> &'static str {
    let mut length = 0;
    let bytes = unsafe { ghostos_passkey_asset(kind, &mut length) };
    assert!(!bytes.is_null(), "valid C passkey asset");
    // The pointer is immutable static storage, with the length supplied by C.
    std::str::from_utf8(unsafe { std::slice::from_raw_parts(bytes, length) }).expect("C passkey asset is UTF-8")
}

pub(super) fn request_fits(current: usize, incoming: usize) -> bool {
    unsafe { ghostos_passkey_request_fits(current, incoming) }
}

pub(super) fn parse_request(bytes: &[u8]) -> io::Result<Option<Request>> {
    let mut request = CRequest::default();
    let code = unsafe { ghostos_passkey_request_parse(bytes.as_ptr(), bytes.len(), cfg!(debug_assertions), &mut request) };
    match code {
        0 => return Ok(None),
        2 => return Err(io::ErrorKind::InvalidData.into()),
        3 => panic!("attempt to add with overflow"),
        1 | 4 => {},
        _ => unreachable!("C passkey HTTP result"),
    }
    let text = |start: usize, length: usize| std::str::from_utf8(&bytes[start..start + length])
        .expect("C validated HTTP header UTF-8").to_string();
    Ok(Some(Request {
        method: text(request.method_start, request.method_length),
        target: text(request.target_start, request.target_length),
        token: request.has_token.then(|| text(request.token_start, request.token_length)),
        // Retain the existing release-mode wrapped range panic on malformed
        // overflowing Content-Length rather than silently accepting that input.
        body: bytes[request.body_start..request.body_start.wrapping_add(request.body_length)].to_vec(),
    }))
}

pub(super) fn query<'a>(target: &'a str, wanted: &str) -> Option<&'a str> {
    let mut start = 0;
    let mut length = 0;
    let found = unsafe { ghostos_passkey_query(target.as_ptr(), target.len(), wanted.as_ptr(), wanted.len(), &mut start, &mut length) };
    found.then(|| &target[start..start + length])
}

pub(super) fn percent_decode(input: &str) -> Option<String> {
    let mut output = vec![0; input.len()];
    let mut written = 0;
    if !unsafe { ghostos_passkey_percent_decode(input.as_ptr(), input.len(), output.as_mut_ptr(), &mut written) } { return None; }
    output.truncate(written);
    Some(String::from_utf8(output).expect("C validated percent-decoded UTF-8"))
}

pub(super) fn valid_username(input: &str) -> bool {
    unsafe { ghostos_passkey_valid_username(input.as_ptr(), input.len()) }
}
pub(super) fn valid_hex(input: &str, maximum: usize) -> bool {
    unsafe { ghostos_passkey_valid_hex(input.as_ptr(), input.len(), maximum) }
}
pub(super) fn decode_hex(input: &str) -> Option<Vec<u8>> {
    let mut output = vec![0; input.len() / 2];
    unsafe { ghostos_passkey_decode_hex(input.as_ptr(), input.len(), output.as_mut_ptr()) }.then_some(output)
}
pub(super) fn frame(input: &[u8], mode: u32, maximum: usize) -> Option<Vec<u8>> {
    let required = unsafe { ghostos_passkey_frame(input.as_ptr(), input.len(), mode, maximum, std::ptr::null_mut(), 0) };
    if required == 0 { return None; }
    let mut output = vec![0; required];
    let length = unsafe { ghostos_passkey_frame(input.as_ptr(), input.len(), mode, maximum, output.as_mut_ptr(), output.len()) };
    if length == 0 { None } else { output.truncate(length); Some(output) }
}
pub(super) fn enrollment_pending(text: &str) -> bool {
    unsafe { ghostos_passkey_enrollment_pending(text.as_ptr(), text.len()) }
}

fn mode_code(mode: Mode) -> u32 {
    match mode { Mode::Waiting => 0, Mode::Enroll => 1, Mode::Login => 2, Mode::Challenge => 3, Mode::Success => 4 }
}
pub(super) fn next_input(mode: Mode, flow: &InputFlow, text: &str) -> u32 {
    let flow = match flow { InputFlow::None => 0, InputFlow::EnrollKind { .. } => 1,
        InputFlow::EnrollMaterial { .. } => 2, InputFlow::EnrollConfirm => 3, InputFlow::LoginKind => 4 };
    unsafe { ghostos_passkey_next_input(mode_code(mode), flow, text.as_ptr(), text.len()) }
}

pub(super) fn observe(bridge: &mut PasskeyBridge, new_text: &str) {
    let mut state = State { mode: mode_code(bridge.mode),
        committed: bridge.administrator_committed, login_in_progress: bridge.login_in_progress };
    let mut observation = Observation::default();
    unsafe { ghostos_passkey_observe(&mut state, bridge.guest_text.as_ptr(), bridge.guest_text.len(),
        new_text.as_ptr(), new_text.len(), &mut observation); }
    bridge.mode = match state.mode { 0 => Mode::Waiting, 1 => Mode::Enroll, 2 => Mode::Login,
        3 => Mode::Challenge, 4 => Mode::Success, _ => unreachable!("C passkey mode") };
    bridge.administrator_committed = state.committed;
    bridge.login_in_progress = state.login_in_progress;
    match observation.challenge_action {
        0 => {}, 1 => bridge.challenge = None,
        2 => bridge.challenge = Some(bridge.guest_text[observation.challenge_start..observation.challenge_start + 64].to_string()),
        _ => unreachable!("C passkey challenge action"),
    }
    match observation.error_action {
        0 => {}, 1 => bridge.error = None,
        2 => bridge.error = Some("GhostOS rejected that passkey. Try again.".to_string()),
        3 => bridge.error = Some("GhostOS could not save that passkey. Create it again.".to_string()),
        _ => unreachable!("C passkey error action"),
    }
    if observation.clear_flow { bridge.input_flow = InputFlow::None; }
}

unsafe extern "C" {
    fn ghostos_passkey_asset(kind: u32, length: *mut usize) -> *const u8;
    fn ghostos_passkey_request_fits(current: usize, incoming: usize) -> bool;
    fn ghostos_passkey_request_parse(bytes: *const u8, length: usize, checked: bool, request: *mut CRequest) -> u32;
    fn ghostos_passkey_query(target: *const u8, length: usize, wanted: *const u8, wanted_length: usize, start: *mut usize, size: *mut usize) -> bool;
    fn ghostos_passkey_percent_decode(input: *const u8, length: usize, output: *mut u8, written: *mut usize) -> bool;
    fn ghostos_passkey_valid_username(input: *const u8, length: usize) -> bool;
    fn ghostos_passkey_valid_hex(input: *const u8, length: usize, maximum: usize) -> bool;
    fn ghostos_passkey_decode_hex(input: *const u8, length: usize, output: *mut u8) -> bool;
    fn ghostos_passkey_frame(input: *const u8, length: usize, mode: u32, maximum: usize, output: *mut u8, capacity: usize) -> usize;
    fn ghostos_passkey_next_input(mode: u32, flow: u32, text: *const u8, length: usize) -> u32;
    fn ghostos_passkey_enrollment_pending(text: *const u8, length: usize) -> bool;
    fn ghostos_passkey_observe(state: *mut State, text: *const u8, length: usize,
        new_text: *const u8, new_length: usize, observation: *mut Observation);
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(super::MAX_REQUEST_BYTES == 16384);
    assert!(std::mem::size_of::<CRequest>() == 72);
    assert!(std::mem::size_of::<State>() == 8);
    assert!(std::mem::size_of::<Observation>() == 24);
};
