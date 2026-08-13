pub struct UsbKeyboard;

impl UsbKeyboard {
    pub const fn new() -> Option<Self> {
        None
    }

    pub const fn read_byte(&mut self) -> Option<u8> {
        None
    }
}

#[allow(dead_code)]
pub const fn read_boot_byte() -> Option<u8> {
    None
}
