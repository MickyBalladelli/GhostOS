#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseState {
    pub buttons: u8,
    pub delta_x: i16,
    pub delta_y: i16,
    pub sequence: u32,
}

pub const fn state() -> MouseState {
    MouseState {
        buttons: 0,
        delta_x: 0,
        delta_y: 0,
        sequence: 0,
    }
}
