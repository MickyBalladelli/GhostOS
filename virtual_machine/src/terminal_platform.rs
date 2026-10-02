//! Native terminal settings and signal cleanup are owned by C.

use std::io;
use std::ptr::NonNull;

type CTerminalMode = std::ffi::c_void;

unsafe extern "C" {
    fn ghostos_vm_terminal_mode_enter(mode: *mut *mut CTerminalMode, error: *mut i32) -> u32;
    fn ghostos_vm_terminal_mode_restore(mode: *mut CTerminalMode, error: *mut i32) -> u32;
    fn ghostos_vm_terminal_mode_free(mode: *mut CTerminalMode);
    fn ghostos_vm_terminal_size(rows: *mut u16, columns: *mut u16) -> bool;
}

fn platform_result(code: u32, error: i32) -> io::Result<()> {
    match code {
        0 => Ok(()),
        2 => Err(io::Error::new(io::ErrorKind::AlreadyExists, "another raw terminal session is active")),
        3 => Err(io::Error::other("terminal mode allocation failed")),
        _ => Err(io::Error::from_raw_os_error(error)),
    }
}

pub(super) struct TerminalMode {
    mode: NonNull<CTerminalMode>,
}

impl TerminalMode {
    pub(super) fn enter() -> io::Result<Self> {
        let mut mode = std::ptr::null_mut();
        let mut error = 0;
        let code = unsafe { ghostos_vm_terminal_mode_enter(&mut mode, &mut error) };
        platform_result(code, error)?;
        Ok(Self { mode: NonNull::new(mode).expect("C terminal mode allocation") })
    }

    pub(super) fn restore(&mut self) -> io::Result<()> {
        let mut error = 0;
        let code = unsafe { ghostos_vm_terminal_mode_restore(self.mode.as_ptr(), &mut error) };
        platform_result(code, error)
    }
}

impl Drop for TerminalMode {
    fn drop(&mut self) {
        unsafe { ghostos_vm_terminal_mode_free(self.mode.as_ptr()) }
    }
}

pub(super) fn size() -> Option<(u16, u16)> {
    let mut rows = 0;
    let mut columns = 0;
    unsafe { ghostos_vm_terminal_size(&mut rows, &mut columns) }.then_some((rows, columns))
}
