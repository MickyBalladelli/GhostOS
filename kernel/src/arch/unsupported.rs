pub mod paging {
    pub const TABLE_FRAME_COUNT: usize = 6;
    pub unsafe fn install_root(_: &[u64; TABLE_FRAME_COUNT], _: u64) {}
}

pub mod interrupts {
    pub fn set_core_isolated(_cpu: u8, _isolated: bool) {}


    pub unsafe fn init() {}
    pub unsafe fn enable() {}
}

pub fn halt() {
    core::hint::spin_loop()
}
