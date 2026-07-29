pub struct Keyboard;

impl Keyboard {
    pub const fn new() -> Self {
        Self
    }

    pub const fn read_byte(&mut self) -> Option<u8> {
        None
    }
}
