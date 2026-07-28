const DATA_PORT: u16 = 0x60;
const STATUS_PORT: u16 = 0x64;
const ENABLE_FIRST_PORT: u8 = 0xae;
const ENABLE_SCANNING: u8 = 0xf4;
const OUTPUT_FULL: u8 = 1;
const INPUT_FULL: u8 = 1 << 1;
const AUXILIARY_DATA: u8 = 1 << 5;

pub struct Keyboard {
    left_shift: bool,
    right_shift: bool,
    control: bool,
    caps_lock: bool,
    extended: bool,
}

impl Keyboard {
    pub fn new() -> Self {
        let keyboard = Self {
            left_shift: false,
            right_shift: false,
            control: false,
            caps_lock: false,
            extended: false,
        };
        unsafe {
            initialize_controller()
        }
        keyboard
    }

    pub fn read_byte(&mut self) -> Option<u8> {
        let status = unsafe { inb(STATUS_PORT) };
        if status & OUTPUT_FULL == 0 {
            return None
        }

        let scan_code = unsafe { inb(DATA_PORT) };
        if status & AUXILIARY_DATA != 0 {
            return None
        }
        if scan_code == 0xe0 {
            self.extended = true;
            return None
        }
        if matches!(scan_code, 0xfa | 0xfe) {
            return None
        }

        let released = scan_code & 0x80 != 0;
        let code = scan_code & 0x7f;
        if self.extended {
            self.extended = false;
            return self.decode_extended(code, released)
        }

        match code {
            0x1d => {
                self.control = !released;
                return None
            }
            0x2a => {
                self.left_shift = !released;
                return None
            }
            0x36 => {
                self.right_shift = !released;
                return None
            }
            0x3a if !released => {
                self.caps_lock = !self.caps_lock;
                return None
            }
            _ => {}
        }
        if released {
            return None
        }

        match code {
            0x01 => Some(3),
            0x0e => Some(8),
            0x0f => Some(b'\t'),
            0x1c => Some(b'\r'),
            0x39 => Some(b' '),
            _ => self.decode_character(code),
        }
    }

    fn decode_extended(&mut self, code: u8, released: bool) -> Option<u8> {
        if code == 0x1d {
            self.control = !released;
            return None
        }
        if released {
            return None
        }
        match code {
            0x1c => Some(b'\r'),
            0x53 => Some(127),
            _ => None,
        }
    }

    fn decode_character(&self, code: u8) -> Option<u8> {
        if let Some(letter) = letter(code) {
            if self.control {
                return Some(letter & 0x1f)
            }
            let uppercase = self.shifted() ^ self.caps_lock;
            return Some(if uppercase {
                letter.to_ascii_uppercase()
            } else {
                letter
            })
        }

        let (plain, shifted) = symbol(code)?;
        Some(if self.shifted() { shifted } else { plain })
    }

    const fn shifted(&self) -> bool {
        self.left_shift || self.right_shift
    }
}

fn letter(code: u8) -> Option<u8> {
    match code {
        0x10 => Some(b'q'),
        0x11 => Some(b'w'),
        0x12 => Some(b'e'),
        0x13 => Some(b'r'),
        0x14 => Some(b't'),
        0x15 => Some(b'y'),
        0x16 => Some(b'u'),
        0x17 => Some(b'i'),
        0x18 => Some(b'o'),
        0x19 => Some(b'p'),
        0x1e => Some(b'a'),
        0x1f => Some(b's'),
        0x20 => Some(b'd'),
        0x21 => Some(b'f'),
        0x22 => Some(b'g'),
        0x23 => Some(b'h'),
        0x24 => Some(b'j'),
        0x25 => Some(b'k'),
        0x26 => Some(b'l'),
        0x2c => Some(b'z'),
        0x2d => Some(b'x'),
        0x2e => Some(b'c'),
        0x2f => Some(b'v'),
        0x30 => Some(b'b'),
        0x31 => Some(b'n'),
        0x32 => Some(b'm'),
        _ => None,
    }
}

fn symbol(code: u8) -> Option<(u8, u8)> {
    match code {
        0x02 => Some((b'1', b'!')),
        0x03 => Some((b'2', b'@')),
        0x04 => Some((b'3', b'#')),
        0x05 => Some((b'4', b'$')),
        0x06 => Some((b'5', b'%')),
        0x07 => Some((b'6', b'^')),
        0x08 => Some((b'7', b'&')),
        0x09 => Some((b'8', b'*')),
        0x0a => Some((b'9', b'(')),
        0x0b => Some((b'0', b')')),
        0x0c => Some((b'-', b'_')),
        0x0d => Some((b'=', b'+')),
        0x1a => Some((b'[', b'{')),
        0x1b => Some((b']', b'}')),
        0x27 => Some((b';', b':')),
        0x28 => Some((b'\'', b'"')),
        0x29 => Some((b'`', b'~')),
        0x2b => Some((b'\\', b'|')),
        0x33 => Some((b',', b'<')),
        0x34 => Some((b'.', b'>')),
        0x35 => Some((b'/', b'?')),
        _ => None,
    }
}

unsafe fn initialize_controller() {
    unsafe {
        for _ in 0..32 {
            if inb(STATUS_PORT) & OUTPUT_FULL == 0 {
                break
            }
            let _ = inb(DATA_PORT);
        }

        if wait_for_input_buffer() {
            outb(STATUS_PORT, ENABLE_FIRST_PORT)
        }
        if wait_for_input_buffer() {
            outb(DATA_PORT, ENABLE_SCANNING)
        }
    }
}

unsafe fn wait_for_input_buffer() -> bool {
    for _ in 0..100_000 {
        if unsafe { inb(STATUS_PORT) } & INPUT_FULL == 0 {
            return true
        }
        core::hint::spin_loop()
    }
    false
}

unsafe fn outb(port: u16, value: u8) {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") port,
            in("al") value,
            options(nomem, nostack)
        )
    }
}

unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    unsafe {
        core::arch::asm!(
            "in al, dx",
            out("al") value,
            in("dx") port,
            options(nomem, nostack)
        )
    }
    value
}
