pub struct UsbKeyboard;

impl UsbKeyboard {
    pub const fn new() -> Option<Self> {
        None
    }

    pub const fn read_byte(&mut self) -> Option<u8> {
        None
    }
}
