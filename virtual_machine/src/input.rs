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

/// Encode a host terminal resize for the guest serial console.
pub fn serial_resize_sequence(resize: TerminalResize) -> Vec<u8> {
    format!("\x1b[8;{};{}t", resize.rows, resize.columns).into_bytes()
}

/// Remove the terminal's response to a size query before PS/2 conversion.
/// The response is a control sequence, not a key press.
pub fn strip_terminal_resize_responses(bytes: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if let Some(end) = terminal_resize_response_end(bytes, index) {
            index = end;
        } else {
            result.push(bytes[index]);
            index += 1;
        }
    }
    result
}

fn terminal_resize_response_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start;
    if bytes.get(index) != Some(&0x1b) || bytes.get(index + 1) != Some(&b'[') {
        return None
    }
    index += 2;
    if bytes.get(index) != Some(&b'8') || bytes.get(index + 1) != Some(&b';') {
        return None
    }
    index += 2;
    index = skip_digits(bytes, index)?;
    if bytes.get(index) != Some(&b';') {
        return None
    }
    index = skip_digits(bytes, index + 1)?;
    if bytes.get(index) != Some(&b't') {
        return None
    }
    Some(index + 1)
}

fn skip_digits(bytes: &[u8], mut index: usize) -> Option<usize> {
    let start = index;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        index += 1;
    }
    (index != start).then_some(index)
}

/// Convert one ASCII byte to PS/2 set-1 make/break bytes.
pub fn ascii_to_scancodes(byte: u8) -> Vec<u8> {
    let (byte, control) = if (1..=26).contains(&byte)
        && !matches!(byte, b'\t' | b'\n' | b'\r' | 0x08)
    {
        (b'a' + byte - 1, true)
    } else {
        (byte, false)
    };
    let (code, shift) = match byte {
        b'a'..=b'z' => {
            let codes = [
                0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24,
                0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10, 0x13, 0x1F, 0x14,
                0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C,
            ];
            (codes[(byte - b'a') as usize], false)
        }
        b'A'..=b'Z' => {
            let codes = [
                0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24,
                0x25, 0x26, 0x32, 0x31, 0x18, 0x19, 0x10, 0x13, 0x1F, 0x14,
                0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C,
            ];
            (codes[(byte - b'A') as usize], true)
        }
        b'1' => (0x02, false),
        b'2' => (0x03, false),
        b'3' => (0x04, false),
        b'4' => (0x05, false),
        b'5' => (0x06, false),
        b'6' => (0x07, false),
        b'7' => (0x08, false),
        b'8' => (0x09, false),
        b'9' => (0x0A, false),
        b'0' => (0x0B, false),
        b'!' => (0x02, true),
        b'@' => (0x03, true),
        b'#' => (0x04, true),
        b'$' => (0x05, true),
        b'%' => (0x06, true),
        b'^' => (0x07, true),
        b'&' => (0x08, true),
        b'*' => (0x09, true),
        b'(' => (0x0A, true),
        b')' => (0x0B, true),
        b' ' => (0x39, false),
        b'\n' | b'\r' => (0x1C, false),
        b'\t' => (0x0F, false),
        0x08 | 0x7F => (0x0E, false),
        b'-' => (0x0C, false),
        b'_' => (0x0C, true),
        b'=' => (0x0D, false),
        b'+' => (0x0D, true),
        b'[' => (0x1A, false),
        b'{' => (0x1A, true),
        b']' => (0x1B, false),
        b'}' => (0x1B, true),
        b'\\' => (0x2B, false),
        b'|' => (0x2B, true),
        b';' => (0x27, false),
        b':' => (0x27, true),
        b'\'' => (0x28, false),
        b'"' => (0x28, true),
        b',' => (0x33, false),
        b'<' => (0x33, true),
        b'.' => (0x34, false),
        b'>' => (0x34, true),
        b'/' => (0x35, false),
        b'?' => (0x35, true),
        b'`' => (0x29, false),
        b'~' => (0x29, true),
        _ => return Vec::new(),
    };

    let mut result = Vec::with_capacity(if shift || control { 4 } else { 2 });
    if shift {
        result.push(0x2A)
    }
    if control {
        result.push(0x1D)
    }
    result.push(code);
    result.push(code | 0x80);
    if control {
        result.push(0x9D)
    }
    if shift {
        result.push(0xAA)
    }
    result
}
