pub mod paging {
    pub const TABLE_FRAME_COUNT: usize = 6;
    #[allow(dead_code)]
    pub unsafe fn install_root(_: &[u64; TABLE_FRAME_COUNT], _: u64) {}
    #[allow(dead_code)]
    pub unsafe fn invalidate_page(_: u64) {}
    #[allow(dead_code)]
    pub unsafe fn invalidate_all() {}
    pub unsafe fn invalidate_range(_: u64, _: u64) {}

}

pub(crate) fn enter_user(_: &crate::Context, _: crate::PageTableRoot) -> ! {
    crate::halt()
}

pub mod interrupts {
    pub fn set_core_isolated(_cpu: u8, _isolated: bool) {}

    pub fn disable() {}


    pub unsafe fn init() {}
}

pub fn halt() {
    core::hint::spin_loop()
}
