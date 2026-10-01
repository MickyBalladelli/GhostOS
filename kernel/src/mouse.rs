//! PS/2 mouse packet collection for the boot console and user input service.

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MouseState {
    pub buttons: u8,
    pub delta_x: i16,
    pub delta_y: i16,
    pub sequence: u32,
}

unsafe extern "C" {
    fn ghostos_mouse_ingest(byte: u8);
    fn ghostos_mouse_state_read() -> MouseState;
}

#[allow(unsafe_code)]
pub(crate) fn ingest(byte: u8) {
    unsafe { ghostos_mouse_ingest(byte) }
}

#[allow(unsafe_code)]
pub fn state() -> MouseState {
    unsafe { ghostos_mouse_state_read() }
}
