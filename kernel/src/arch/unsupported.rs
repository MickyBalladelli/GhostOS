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

pub(crate) fn capture_registers(fault_address: u64) -> crate::crash::RegisterState {
    crate::crash::RegisterState::empty().with_fault_address(fault_address)
}
