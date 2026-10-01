#[repr(C)]
struct CUsbKeyboardStub {
    unused: u8,
}

unsafe extern "C" {
    fn ghostos_usb_keyboard_stub_init(keyboard: *mut CUsbKeyboardStub);
    fn ghostos_usb_keyboard_stub_read_byte(keyboard: *mut CUsbKeyboardStub, byte: *mut u8) -> bool;
}

pub struct UsbKeyboard {
    inner: CUsbKeyboardStub,
}

impl UsbKeyboard {
    pub fn new() -> Option<Self> {
        let mut keyboard = Self { inner: CUsbKeyboardStub { unused: 0 } };
        unsafe { ghostos_usb_keyboard_stub_init(&mut keyboard.inner) };
        None
    }

    pub fn read_byte(&mut self) -> Option<u8> {
        let mut byte = 0;
        if unsafe { ghostos_usb_keyboard_stub_read_byte(&mut self.inner, &mut byte) } {
            Some(byte)
        } else {
            None
        }
    }
}

#[allow(dead_code)]
pub fn read_boot_byte() -> Option<u8> {
    let mut keyboard = CUsbKeyboardStub { unused: 0 };
    unsafe {
        ghostos_usb_keyboard_stub_init(&mut keyboard);
        let mut byte = 0;
        if ghostos_usb_keyboard_stub_read_byte(&mut keyboard, &mut byte) {
            Some(byte)
        } else {
            None
        }
    }
}
