//! PS/2 controller emulation for keyboard and mouse input.
//! C owns the controller and queues; Rust adapts shared APIC ownership.

use super::{ApicTrigger, DeviceError, LocalApic, PortDevice};
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;

pub const PS2_DATA_PORT: u16 = 0x60;
pub const PS2_STATUS_PORT: u16 = 0x64;
pub const PS2_PORT_COUNT: u16 = 5;

#[cfg(test)]
const COMMAND_ENABLE_MOUSE: u8 = 0xA8;
#[cfg(test)]
const COMMAND_DISABLE_KEYBOARD: u8 = 0xAD;
#[cfg(test)]
const COMMAND_ENABLE_KEYBOARD: u8 = 0xAE;
#[cfg(test)]
const COMMAND_WRITE_MOUSE: u8 = 0xD4;

#[repr(C)]
struct CController {
    _private: [u8; 0],
}

type IrqCallback = unsafe extern "C" fn(*mut c_void, u8);

unsafe extern "C" {
    fn ghostos_vm_ps2_new() -> *mut CController;
    fn ghostos_vm_ps2_free(ps2: *mut CController);
    fn ghostos_vm_ps2_reset(ps2: *mut CController);
    fn ghostos_vm_ps2_set_keyboard_vector(ps2: *mut CController, vector: u8);
    fn ghostos_vm_ps2_set_mouse_vector(ps2: *mut CController, vector: u8);
    fn ghostos_vm_ps2_input_pending(ps2: *const CController) -> bool;
    fn ghostos_vm_ps2_keyboard(ps2: *mut CController, bytes: *const u8, length: usize,
        irq: IrqCallback, context: *mut c_void);
    fn ghostos_vm_ps2_keyboard_lossless(ps2: *mut CController, bytes: *const u8,
        length: usize, irq: IrqCallback, context: *mut c_void) -> bool;
    fn ghostos_vm_ps2_mouse_packet(ps2: *mut CController, packet: *const u8,
        irq: IrqCallback, context: *mut c_void);
    fn ghostos_vm_ps2_mouse_motion(ps2: *mut CController, dx: i16, dy: i16, buttons: u8,
        irq: IrqCallback, context: *mut c_void);
    fn ghostos_vm_ps2_read(ps2: *mut CController, port: u16, size: u8, value: *mut u64,
        irq: IrqCallback, context: *mut c_void) -> u8;
    fn ghostos_vm_ps2_write(ps2: *mut CController, port: u16, value: u64, size: u8,
        irq: IrqCallback, context: *mut c_void) -> u8;
}

#[derive(Default)]
struct InterruptRoute {
    apic: Option<Rc<RefCell<LocalApic>>>,
    pending_irq: Option<u8>,
}

impl InterruptRoute {
    fn signal(&mut self, vector: u8) {
        let Some(apic) = &self.apic else { return };
        if let Ok(mut apic) = apic.try_borrow_mut() {
            apic.signal(vector, ApicTrigger::Edge)
        } else {
            self.pending_irq = Some(vector)
        }
    }

    fn context(&mut self) -> *mut c_void {
        (self as *mut Self).cast()
    }
}

// C calls this synchronously with the exclusively borrowed route field and
// never keeps the pointer. It does not access the separate C controller.
unsafe extern "C" fn signal_irq(context: *mut c_void, vector: u8) {
    unsafe { &mut *context.cast::<InterruptRoute>() }.signal(vector)
}

/// Minimal dual-channel i8042 controller with PS/2 set-2 command handling.
pub struct Ps2Controller {
    state: *mut CController,
    irq: InterruptRoute,
}

impl Ps2Controller {
    pub fn new() -> Self {
        let state = unsafe { ghostos_vm_ps2_new() };
        assert!(!state.is_null(), "could not allocate PS/2 controller");
        Self { state, irq: InterruptRoute::default() }
    }

    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.irq.apic = Some(apic)
    }

    pub fn set_keyboard_irq_vector(&mut self, vector: u8) {
        unsafe { ghostos_vm_ps2_set_keyboard_vector(self.state, vector) }
    }

    pub fn set_mouse_irq_vector(&mut self, vector: u8) {
        unsafe { ghostos_vm_ps2_set_mouse_vector(self.state, vector) }
    }

    pub fn push_keyboard_scancode(&mut self, scancode: u8) {
        self.push_keyboard_bytes(&[scancode])
    }

    pub fn push_keyboard_bytes(&mut self, bytes: &[u8]) {
        unsafe {
            ghostos_vm_ps2_keyboard(self.state, bytes.as_ptr(), bytes.len(),
                signal_irq, self.irq.context())
        }
    }

    /// Keep long pastes queued until the guest consumes them.
    pub(crate) fn push_keyboard_scancodes_lossless(&mut self, scancodes: &[u8]) {
        let queued = unsafe {
            ghostos_vm_ps2_keyboard_lossless(self.state, scancodes.as_ptr(),
                scancodes.len(), signal_irq, self.irq.context())
        };
        assert!(queued, "could not allocate PS/2 pending keyboard queue")
    }

    pub fn push_mouse_packet(&mut self, packet: [u8; 3]) {
        unsafe {
            ghostos_vm_ps2_mouse_packet(self.state, packet.as_ptr(),
                signal_irq, self.irq.context())
        }
    }

    pub fn push_mouse_motion(&mut self, dx: i16, dy: i16, buttons: u8) {
        unsafe {
            ghostos_vm_ps2_mouse_motion(self.state, dx, dy, buttons,
                signal_irq, self.irq.context())
        }
    }

    pub fn input_pending(&self) -> bool {
        unsafe { ghostos_vm_ps2_input_pending(self.state) }
    }

    pub fn poll_interrupt(&mut self) {
        let Some(vector) = self.irq.pending_irq.take() else { return };
        self.irq.signal(vector)
    }
}

impl Default for Ps2Controller {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Ps2Controller {
    fn drop(&mut self) {
        unsafe { ghostos_vm_ps2_free(self.state) }
    }
}

fn port_result(code: u8) -> Result<(), DeviceError> {
    match code {
        0 => Ok(()),
        1 => Err(DeviceError::UnsupportedSize),
        _ => Err(DeviceError::InvalidAddress),
    }
}

impl PortDevice for Ps2Controller {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        let mut value = 0;
        let code = unsafe {
            ghostos_vm_ps2_read(self.state, port, size, &mut value,
                signal_irq, self.irq.context())
        };
        port_result(code)?;
        Ok(value)
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        let code = unsafe {
            ghostos_vm_ps2_write(self.state, port, value, size, signal_irq, self.irq.context())
        };
        port_result(code)
    }

    fn reset(&mut self) {
        unsafe { ghostos_vm_ps2_reset(self.state) };
        self.irq = InterruptRoute::default();
    }
}

impl PortDevice for Rc<RefCell<Ps2Controller>> {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        self.borrow_mut().read(port, size)
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        self.borrow_mut().write(port, value, size)
    }

    fn reset(&mut self) {
        self.borrow_mut().reset()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_bytes_are_visible_at_data_port() {
        let mut ps2 = Ps2Controller::new();
        ps2.push_keyboard_scancode(0x1E);
        assert_eq!(ps2.read(PS2_STATUS_PORT, 1).unwrap() & 1, 1);
        assert_eq!(ps2.read(PS2_DATA_PORT, 1).unwrap(), 0x1E);
        assert_eq!(ps2.read(PS2_STATUS_PORT, 1).unwrap() & 1, 0);
    }

    #[test]
    fn mouse_streaming_can_be_enabled() {
        let mut ps2 = Ps2Controller::new();
        ps2.write(PS2_STATUS_PORT, COMMAND_ENABLE_MOUSE as u64, 1)
            .unwrap();
        ps2.write(PS2_STATUS_PORT, COMMAND_WRITE_MOUSE as u64, 1)
            .unwrap();
        ps2.write(PS2_DATA_PORT, 0xF4, 1).unwrap();
        while ps2.input_pending() {
            let _ = ps2.read(PS2_DATA_PORT, 1);
        }
        ps2.push_mouse_packet([0x08, 1, 0]);
        assert_eq!(ps2.read(PS2_DATA_PORT, 1).unwrap(), 0x08);
    }

    #[test]
    fn controller_commands_and_overflow_are_bounded() {
        let mut ps2 = Ps2Controller::new();
        ps2.write(PS2_STATUS_PORT, COMMAND_DISABLE_KEYBOARD as u64, 1).unwrap();
        ps2.push_keyboard_bytes(&[0x1C, 0xF0, 0x1C]);
        assert!(!ps2.input_pending());

        ps2.write(PS2_STATUS_PORT, COMMAND_ENABLE_KEYBOARD as u64, 1).unwrap();
        for _ in 0..256 {
            ps2.push_keyboard_scancode(0x1E);
        }
        let mut count = 0;
        while ps2.input_pending() {
            let _ = ps2.read(PS2_DATA_PORT, 1).unwrap();
            count += 1;
        }
        assert!(count <= 64);
        assert_eq!(ps2.read(PS2_STATUS_PORT, 1).unwrap() & 1, 0);
    }
}
