//! Guest input routing and emulation helpers.
//!
//! Host terminal policy stays in [`crate::terminal`]. This module decides how
//! already-normalized host bytes reach guest serial or PS/2 devices.

use crate::terminal::TerminalResize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestInputMode {
    Serial,
    Ps2,
}

unsafe extern "C" {
    fn ghostos_vm_input_resize(rows: u16, columns: u16, output: *mut u8) -> usize;
    fn ghostos_vm_input_strip_resize(input: *const u8, length: usize, output: *mut u8) -> usize;
    fn ghostos_vm_input_scancodes(byte: u8, output: *mut u8) -> usize;
}

/// Encode a host terminal resize for the guest serial console.
pub fn serial_resize_sequence(resize: TerminalResize) -> Vec<u8> {
    let mut output = [0; 16];
    let length = unsafe {
        ghostos_vm_input_resize(resize.rows, resize.columns, output.as_mut_ptr())
    };
    output[..length].to_vec()
}

/// Remove the terminal's response to a size query before PS/2 conversion.
/// The response is a control sequence, not a key press.
pub fn strip_terminal_resize_responses(bytes: &[u8]) -> Vec<u8> {
    let mut output = vec![0; bytes.len()];
    let length = unsafe {
        ghostos_vm_input_strip_resize(bytes.as_ptr(), bytes.len(), output.as_mut_ptr())
    };
    output.truncate(length);
    output
}

/// Convert one ASCII byte to PS/2 set-1 make/break bytes.
pub fn ascii_to_scancodes(byte: u8) -> Vec<u8> {
    let mut output = [0; 4];
    let length = unsafe { ghostos_vm_input_scancodes(byte, output.as_mut_ptr()) };
    output[..length].to_vec()
}
