#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseState {
    pub buttons: u8,
    pub delta_x: i16,
    pub delta_y: i16,
    pub sequence: u32,
}

const _: [(); 12] = [(); core::mem::size_of::<MouseState>()];
const _: [(); 4] = [(); core::mem::align_of::<MouseState>()];

unsafe extern "C" {
    fn ghostos_mouse_stub_state_read() -> MouseState;
}

#[allow(unsafe_code)]
pub fn state() -> MouseState {
    unsafe { ghostos_mouse_stub_state_read() }
}
