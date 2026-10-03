use core::ffi::c_void;

#[repr(C)]
struct KeyboardState {
    left_shift: bool,
    right_shift: bool,
    control: bool,
    caps_lock: bool,
    extended: bool,
    pending: [u8; 4],
    pending_start: u8,
    pending_count: u8,
}

#[repr(C)]
struct KeyboardIo {
    context: *mut c_void,
    read_port: Option<unsafe extern "C" fn(*mut c_void, u16) -> u8>,
    write_port: Option<unsafe extern "C" fn(*mut c_void, u16, u8)>,
    spin: Option<unsafe extern "C" fn(*mut c_void)>,
    mouse_byte: Option<unsafe extern "C" fn(*mut c_void, u8)>,
}

pub struct Keyboard {
    state: KeyboardState,
}

pub fn read_boot_byte() -> Option<u8> {
    let io = native_io();
    let mut byte = 0;
    // C owns and serializes the boot keyboard singleton. The callback table is
    // borrowed for this call only; auxiliary bytes reach the existing mouse API.
    unsafe { ghostos_keyboard_read_boot_byte(&io, &mut byte) }.then_some(byte)
}

impl Keyboard {
    pub fn new() -> Self {
        let mut keyboard = Self { state: KeyboardState {
            left_shift: false, right_shift: false, control: false,
            caps_lock: false, extended: false, pending: [0; 4],
            pending_start: 0, pending_count: 0,
        } };
        // C initializes checked caller-owned state and the PS/2 controller.
        unsafe { ghostos_keyboard_create(&mut keyboard.state, &native_io()) }
        keyboard
    }

    pub fn read_byte(&mut self) -> Option<u8> {
        let mut byte = 0;
        unsafe { ghostos_keyboard_read_byte(&mut self.state, &native_io(), &mut byte) }.then_some(byte)
    }
}

extern "C" fn mouse_byte(_context: *mut c_void, byte: u8) {
    crate::mouse::ingest(byte)
}

fn native_io() -> KeyboardIo {
    let mut io = KeyboardIo { context: core::ptr::null_mut(), read_port: None,
        write_port: None, spin: None, mouse_byte: None };
    // This module is compiled only for x86-64 kernel/UEFI targets. C supplies
    // native port access and pause instructions; it retains no table pointer.
    let initialized = unsafe { ghostos_keyboard_x86_io(&mut io, core::ptr::null_mut(), Some(mouse_byte)) };
    assert!(initialized, "x86 keyboard I/O unavailable");
    io
}

const _: () = {
    assert!(core::mem::size_of::<KeyboardState>() == 11);
    assert!(core::mem::align_of::<KeyboardState>() == 1);
    assert!(core::mem::offset_of!(KeyboardState, right_shift) == 1);
    assert!(core::mem::offset_of!(KeyboardState, control) == 2);
    assert!(core::mem::offset_of!(KeyboardState, caps_lock) == 3);
    assert!(core::mem::offset_of!(KeyboardState, extended) == 4);
    assert!(core::mem::offset_of!(KeyboardState, pending) == 5);
    assert!(core::mem::offset_of!(KeyboardState, pending_start) == 9);
    assert!(core::mem::offset_of!(KeyboardState, pending_count) == 10);
    assert!(core::mem::size_of::<KeyboardIo>() == 40);
    assert!(core::mem::offset_of!(KeyboardIo, read_port) == 8);
    assert!(core::mem::offset_of!(KeyboardIo, write_port) == 16);
    assert!(core::mem::offset_of!(KeyboardIo, spin) == 24);
    assert!(core::mem::offset_of!(KeyboardIo, mouse_byte) == 32);
};

unsafe extern "C" {
    fn ghostos_keyboard_create(keyboard: *mut KeyboardState, io: *const KeyboardIo);
    fn ghostos_keyboard_read_byte(keyboard: *mut KeyboardState, io: *const KeyboardIo, byte: *mut u8) -> bool;
    fn ghostos_keyboard_read_boot_byte(io: *const KeyboardIo, byte: *mut u8) -> bool;
    fn ghostos_keyboard_x86_io(io: *mut KeyboardIo, context: *mut c_void,
        mouse_byte: Option<unsafe extern "C" fn(*mut c_void, u8)>) -> bool;
}
