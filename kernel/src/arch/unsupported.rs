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

    pub fn current_cpu() -> crate::task::CpuId {
        crate::task::CpuId::new(0).expect("CPU 0 is valid")
    }

    pub fn send_ipi(_target: crate::task::CpuId, _vector: u8) -> bool {
        false
    }

    #[allow(dead_code)]
    pub fn end_of_interrupt() {}

    pub fn disable() {}

    pub fn enable() {}


    pub unsafe fn init() {}
}

pub fn halt() {
    core::hint::spin_loop()
}

pub(crate) fn idle(_state: synos_power::CpuIdleState) {
    core::hint::spin_loop()
}
