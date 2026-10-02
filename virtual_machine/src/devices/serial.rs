//! 16550-compatible serial port emulation (port-mapped I/O).
//! C owns UART, queue, capture, and authentication state. Rust connects host
//! console I/O and the shared local APIC.

use super::{DeviceError, PortDevice};
use super::{ApicTrigger, LocalApic};
use std::cell::RefCell;
use std::ffi::c_void;
use std::io::{self, Write};
use std::rc::Rc;
use std::time::Instant;

#[cfg(test)]
const REG_DATA: u16 = 0;
#[cfg(test)]
const REG_IER: u16 = 1;
#[cfg(test)]
const REG_LCR: u16 = 3;
#[cfg(test)]
const REG_LSR: u16 = 5;
#[cfg(test)]
const LSR_DATA_READY: u8 = 1;
#[cfg(test)]
const LSR_OVERRUN: u8 = 2;
#[cfg(test)]
const FIFO_SIZE: usize = 16;
#[cfg(test)]
const ENROLLMENT_MARKER: &[u8] = b"\x1b]GhostOSEnroll\x07";
#[cfg(test)]
const LOGIN_MARKER: &[u8] = b"\x1b]GhostOSLogin\x07";

#[repr(C)]
struct CSerial {
    _private: [u8; 0],
}

type ConsoleWrite = unsafe extern "C" fn(*mut c_void, *const u8, usize);
type ConsoleFlush = unsafe extern "C" fn(*mut c_void);
type ConsoleTime = unsafe extern "C" fn(*mut c_void) -> u64;

unsafe extern "C" {
    fn ghostos_vm_serial_translate_newlines(input: *const u8, input_length: usize,
        previous_was_cr: bool, output: *mut u8, output_capacity: usize,
        output_length: *mut usize, output_previous_was_cr: *mut bool) -> bool;
    fn ghostos_vm_serial_new(base: u16, now_ns: u64) -> *mut CSerial;
    fn ghostos_vm_serial_free(serial: *mut CSerial);
    fn ghostos_vm_serial_reset(serial: *mut CSerial, now_ns: u64);
    fn ghostos_vm_serial_base(serial: *const CSerial) -> u16;
    fn ghostos_vm_serial_push_input(serial: *mut CSerial, bytes: *const u8, length: usize) -> bool;
    fn ghostos_vm_serial_push_input_lossless(serial: *mut CSerial, bytes: *const u8,
        length: usize, interrupt: *mut bool) -> bool;
    fn ghostos_vm_serial_input_pending(serial: *const CSerial) -> bool;
    fn ghostos_vm_serial_set_banner(serial: *mut CSerial, bytes: *const u8, length: usize) -> bool;
    fn ghostos_vm_serial_guest_panicked(serial: *const CSerial) -> bool;
    fn ghostos_vm_serial_output(serial: *const CSerial, length: *mut usize) -> *const u8;
    fn ghostos_vm_serial_clear_output(serial: *mut CSerial);
    fn ghostos_vm_serial_read(serial: *mut CSerial, port: u16, size: u8, value: *mut u64) -> u8;
    fn ghostos_vm_serial_write(serial: *mut CSerial, port: u16, value: u64, size: u8,
        interrupt: *mut bool) -> u8;
    fn ghostos_vm_serial_flush(serial: *mut CSerial, write: ConsoleWrite,
        flush: ConsoleFlush, now: ConsoleTime, context: *mut c_void) -> bool;
    #[cfg(test)]
    fn ghostos_vm_serial_host_output(serial: *const CSerial, length: *mut usize) -> *const u8;
    #[cfg(test)]
    fn ghostos_vm_serial_append_host_output(serial: *mut CSerial, bytes: *const u8, length: usize) -> bool;
    #[cfg(test)]
    fn ghostos_vm_serial_consume_host_output(serial: *mut CSerial, length: usize);
    #[cfg(test)]
    fn ghostos_vm_serial_prepare_host_output(serial: *mut CSerial, writable: *mut usize) -> bool;
    #[cfg(test)]
    fn ghostos_vm_serial_auth_waiting(serial: *const CSerial) -> bool;
    #[cfg(test)]
    fn ghostos_vm_serial_tx_count(serial: *const CSerial) -> usize;
    #[cfg(test)]
    fn ghostos_vm_serial_rx_count(serial: *const CSerial) -> usize;
}

pub(crate) fn write_host_console<W: Write>(
    output: &mut W,
    bytes: &[u8],
    previous_was_cr: &mut bool,
) -> io::Result<()> {
    let mut translated_length = 0;
    let mut output_previous_was_cr = *previous_was_cr;
    let sized = unsafe {
        ghostos_vm_serial_translate_newlines(
            bytes.as_ptr(), bytes.len(), *previous_was_cr, std::ptr::null_mut(), 0,
            &mut translated_length, &mut output_previous_was_cr,
        )
    };
    if !sized {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "serial newline conversion failed"))
    }
    let mut translated = vec![0; translated_length];
    if !unsafe {
        ghostos_vm_serial_translate_newlines(
            bytes.as_ptr(), bytes.len(), *previous_was_cr, translated.as_mut_ptr(),
            translated.len(), &mut translated_length, &mut output_previous_was_cr,
        )
    } {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "serial newline conversion failed"))
    }
    *previous_was_cr = output_previous_was_cr;
    output.write_all(&translated)
}

struct HostConsole<'a> {
    stdout: &'a std::io::Stdout,
    output: Option<std::io::StdoutLock<'a>>,
    start: Instant,
}

unsafe extern "C" fn console_write(context: *mut c_void, bytes: *const u8, length: usize) {
    // C calls synchronously with valid live buffers and exclusive context.
    let console = unsafe { &mut *context.cast::<HostConsole<'_>>() };
    let bytes = unsafe { std::slice::from_raw_parts(bytes, length) };
    let stdout = console.stdout;
    let output = console.output.get_or_insert_with(|| stdout.lock());
    let _ = output.write_all(bytes);
}

unsafe extern "C" fn console_flush(context: *mut c_void) {
    let console = unsafe { &mut *context.cast::<HostConsole<'_>>() };
    let stdout = console.stdout;
    let output = console.output.get_or_insert_with(|| stdout.lock());
    let _ = output.flush();
}

unsafe extern "C" fn console_time(context: *mut c_void) -> u64 {
    let console = unsafe { &mut *context.cast::<HostConsole<'_>>() };
    let stdout = console.stdout;
    let _ = console.output.get_or_insert_with(|| stdout.lock());
    console.start.elapsed().as_nanos().min(u64::MAX as u128) as u64
}

/// C-owned 16550 UART with host console and shared APIC adapters.
pub struct Serial16550 {
    state: *mut CSerial,
    start: Instant,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
}

impl Serial16550 {
    pub fn new(base: u16) -> Self {
        let state = unsafe { ghostos_vm_serial_new(base, 0) };
        assert!(!state.is_null(), "could not allocate serial controller");
        Self { state, start: Instant::now(), apic: None, irq_vector: 0x24 }
    }

    pub fn base(&self) -> u16 {
        unsafe { ghostos_vm_serial_base(self.state) }
    }

    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.apic = Some(apic)
    }

    pub fn set_irq_vector(&mut self, vector: u8) {
        self.irq_vector = vector
    }

    fn signal_receive_irq(&self) {
        if let Some(apic) = &self.apic {
            apic.borrow_mut().signal(self.irq_vector, ApicTrigger::Edge)
        }
    }

    pub fn push_input(&mut self, bytes: &[u8]) {
        if unsafe { ghostos_vm_serial_push_input(self.state, bytes.as_ptr(), bytes.len()) } {
            self.signal_receive_irq()
        }
    }

    /// Retain excess paste bytes until the guest consumes the receive FIFO.
    pub(crate) fn push_input_lossless(&mut self, bytes: &[u8]) {
        let mut interrupt = false;
        let queued = unsafe {
            ghostos_vm_serial_push_input_lossless(self.state, bytes.as_ptr(), bytes.len(), &mut interrupt)
        };
        assert!(queued, "could not allocate pending serial input");
        if interrupt { self.signal_receive_irq() }
    }

    pub fn input_pending(&self) -> bool {
        unsafe { ghostos_vm_serial_input_pending(self.state) }
    }

    pub fn set_authentication_banner(&mut self, banner: &[u8]) {
        assert!(unsafe { ghostos_vm_serial_set_banner(self.state, banner.as_ptr(), banner.len()) },
            "could not allocate serial authentication banner")
    }

    pub fn output(&self) -> &[u8] {
        let mut length = 0;
        let bytes = unsafe { ghostos_vm_serial_output(self.state, &mut length) };
        if length == 0 { &[] } else { unsafe { std::slice::from_raw_parts(bytes, length) } }
    }

    pub fn guest_panicked(&self) -> bool {
        unsafe { ghostos_vm_serial_guest_panicked(self.state) }
    }

    pub fn take_output(&mut self) -> Vec<u8> {
        let output = self.output().to_vec();
        unsafe { ghostos_vm_serial_clear_output(self.state) };
        output
    }

    pub fn flush(&mut self) {
        let stdout = std::io::stdout();
        let mut console = HostConsole { stdout: &stdout, output: None, start: self.start };
        let flushed = unsafe {
            ghostos_vm_serial_flush(self.state, console_write, console_flush, console_time,
                (&mut console as *mut HostConsole<'_>).cast())
        };
        assert!(flushed, "could not allocate serial host output")
    }

    #[cfg(test)]
    fn tx_count(&self) -> usize { unsafe { ghostos_vm_serial_tx_count(self.state) } }
    #[cfg(test)]
    fn rx_count(&self) -> usize { unsafe { ghostos_vm_serial_rx_count(self.state) } }
    #[cfg(test)]
    fn auth_waiting(&self) -> bool { unsafe { ghostos_vm_serial_auth_waiting(self.state) } }
    #[cfg(test)]
    fn append_host_output(&mut self, bytes: &[u8]) {
        assert!(unsafe { ghostos_vm_serial_append_host_output(self.state, bytes.as_ptr(), bytes.len()) })
    }
    #[cfg(test)]
    fn consume_host_output(&mut self, length: usize) {
        unsafe { ghostos_vm_serial_consume_host_output(self.state, length) }
    }
    #[cfg(test)]
    fn host_output(&self) -> &[u8] {
        let mut length = 0;
        let bytes = unsafe { ghostos_vm_serial_host_output(self.state, &mut length) };
        if length == 0 { &[] } else { unsafe { std::slice::from_raw_parts(bytes, length) } }
    }
    #[cfg(test)]
    fn insert_authentication_banner(&mut self) -> usize {
        let mut writable = 0;
        assert!(unsafe { ghostos_vm_serial_prepare_host_output(self.state, &mut writable) });
        writable
    }
}

impl Drop for Serial16550 {
    fn drop(&mut self) {
        unsafe { ghostos_vm_serial_free(self.state) }
    }
}

fn port_result(code: u8) -> Result<(), DeviceError> {
    match code {
        0 => Ok(()),
        1 => Err(DeviceError::UnsupportedSize),
        _ => panic!("could not allocate serial output"),
    }
}

impl PortDevice for Serial16550 {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        let mut value = 0;
        port_result(unsafe { ghostos_vm_serial_read(self.state, port, size, &mut value) })?;
        Ok(value)
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        let mut interrupt = false;
        let code = unsafe { ghostos_vm_serial_write(self.state, port, value, size, &mut interrupt) };
        port_result(code)?;
        if interrupt { self.signal_receive_irq() }
        Ok(())
    }

    fn reset(&mut self) {
        unsafe { ghostos_vm_serial_reset(self.state, 0) };
        self.start = Instant::now();
        self.apic = None;
        self.irq_vector = 0x24;
    }
}

impl PortDevice for Rc<RefCell<Serial16550>> {
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
    fn serial_line_control_toggles_dlab() {
        let base = 0x3F8;
        let mut s = Serial16550::new(base);
        // Default without DLAB: data register is TX path.
        assert!(s.write(base + REG_DATA, b'A' as u64, 1).is_ok());
        assert_eq!(s.tx_count(), 1);

        // Set DLAB.
        s.write(base + REG_LCR, 0x80, 1).unwrap();
        assert!(s.read(base + REG_LCR, 1).unwrap() & 0x80 != 0);

        // Divisor registers map onto 0/1 while DLAB is set.
        s.write(base + REG_DATA, 0x01, 1).unwrap();
        s.write(base + REG_IER, 0x00, 1).unwrap();
        assert_eq!(s.read(base + REG_DATA, 1).unwrap(), 0x01);

        // Clear DLAB and confirm data register goes back to TX.
        s.write(base + REG_LCR, 0x03, 1).unwrap();
        assert_eq!(s.read(base + REG_LCR, 1).unwrap() & 0x80, 0);
        s.write(base + REG_DATA, b'B' as u64, 1).unwrap();
        assert_eq!(s.tx_count(), 2);
    }

    #[test]
    fn serial_rejects_non_byte_accesses() {
        let mut s = Serial16550::new(0x2F8);
        assert!(s.read(0x2F8, 2).is_err());
        assert!(s.write(0x2F8, 0x1234, 2).is_err());
    }

    #[test]
    fn serial_receives_input_and_reports_data_ready() {
        let base = 0x3F8;
        let mut s = Serial16550::new(base);
        s.push_input(b"hi");
        assert_eq!(s.read(base + REG_LSR, 1).unwrap() & u64::from(LSR_DATA_READY), 1);
        assert_eq!(s.read(base + REG_DATA, 1).unwrap(), b'h' as u64);
        assert_eq!(s.read(base + REG_DATA, 1).unwrap(), b'i' as u64);
        assert_eq!(s.read(base + REG_LSR, 1).unwrap() & u64::from(LSR_DATA_READY), 0);
    }

    #[test]
    fn serial_flushes_output_on_carriage_return() {
        let base = 0x3F8;
        let mut s = Serial16550::new(base);
        s.write(base + REG_DATA, b'\r' as u64, 1).unwrap();

        assert_eq!(s.tx_count(), 0);
        assert_eq!(s.output(), b"\r");
    }

    #[test]
    fn authentication_banner_holds_setup_until_prompt() {
        let mut s = Serial16550::new(0x3F8);
        s.set_authentication_banner(b"Passkey setup and login: http://localhost:1234/?code=test\r\n");
        s.append_host_output(
            b"Ready\r\n\x1b]GhostOSEnroll\x07Do not type credentials in this terminal.\r\n",
        );

        let writable = s.insert_authentication_banner();

        assert_eq!(&s.host_output()[..writable], b"Ready\r\n");
        s.consume_host_output(writable);
        assert!(s.auth_waiting());
        assert_eq!(
            s.host_output(),
            b"Do not type credentials in this terminal.\r\n"
        );

        s.append_host_output(
            b"Administrator account committed.\r\nGhostOS user shell\r\n$ ",
        );
        let writable = s.insert_authentication_banner();
        assert!(!s.auth_waiting());
        assert_eq!(&s.host_output()[..writable], b"$ ");
    }

    #[test]
    fn partial_authentication_marker_waits_for_next_flush() {
        let mut s = Serial16550::new(0x3F8);
        s.set_authentication_banner(b"Passkey URL\r\n");
        s.append_host_output(b"Ready\r\n\x1b]Ghost");

        let writable = s.insert_authentication_banner();
        assert_eq!(&s.host_output()[..writable], b"Ready\r\n");
        s.consume_host_output(writable);
        s.append_host_output(b"OSEnroll\x07");

        let writable = s.insert_authentication_banner();
        assert_eq!(writable, 0);
        assert!(s.auth_waiting());
        assert!(s.host_output().is_empty());
    }

    #[test]
    fn authentication_marker_becomes_manual_prompt_without_banner() {
        let mut s = Serial16550::new(0x3F8);
        s.append_host_output(ENROLLMENT_MARKER);

        let writable = s.insert_authentication_banner();

        assert_eq!(writable, s.host_output().len());
        assert_eq!(s.host_output(), b"Administrator username: ");
        assert!(!s.auth_waiting());
    }

    #[test]
    fn login_marker_holds_output_until_prompt() {
        let mut s = Serial16550::new(0x3F8);
        s.set_authentication_banner(b"Passkey URL\r\n");
        s.append_host_output(LOGIN_MARKER);
        s.append_host_output(b"Username: micky\r\nLogin accepted.\r\n");

        let writable = s.insert_authentication_banner();
        assert_eq!(writable, 0);
        assert!(s.auth_waiting());

        s.append_host_output(b"$ ");
        let writable = s.insert_authentication_banner();
        assert!(!s.auth_waiting());
        assert_eq!(&s.host_output()[..writable], b"$ ");
    }

    #[test]
    fn extra_authorized_prompt_is_not_reprinted() {
        let mut s = Serial16550::new(0x3F8);
        s.set_authentication_banner(b"Passkey URL\r\n");
        s.append_host_output(b"$ ");
        let writable = s.insert_authentication_banner();
        assert_eq!(&s.host_output()[..writable], b"$ ");
        s.consume_host_output(writable);

        s.append_host_output(b"\n$ ");
        let writable = s.insert_authentication_banner();
        assert_eq!(writable, 0);
        assert!(!s.auth_waiting());
    }

    #[test]
    fn leftover_prompt_does_not_end_a_later_login_wait() {
        let mut s = Serial16550::new(0x3F8);
        s.set_authentication_banner(b"Passkey URL\r\n");
        s.append_host_output(b"$ ");
        let writable = s.insert_authentication_banner();
        s.consume_host_output(writable);

        s.append_host_output(LOGIN_MARKER);
        s.append_host_output(b"$ ");
        let writable = s.insert_authentication_banner();
        assert_eq!(writable, 0);
        assert!(s.auth_waiting());

        s.append_host_output(b"\n$ ");
        let writable = s.insert_authentication_banner();
        assert!(!s.auth_waiting());
        assert_eq!(&s.host_output()[..writable], b"$ ");
    }

    #[test]
    fn authentication_progress_frames_do_not_reprint_after_prompt() {
        let mut s = Serial16550::new(0x3F8);
        s.set_authentication_banner(b"Passkey URL\r\n");
        s.append_host_output(b"$ \rGhostOS authentication: |");
        s.append_host_output(b"\rGhostOS authentication: /");
        s.append_host_output(b"\rGhostOS authentication: ready\n");

        let writable = s.insert_authentication_banner();

        assert!(!s.auth_waiting());
        assert_eq!(&s.host_output()[..writable], b"$ ");
    }

    #[test]
    fn serial_receive_fifo_sets_overrun_and_irq() {
        let base = 0x3F8;
        let apic = Rc::new(RefCell::new(LocalApic::new(0)));
        let mut s = Serial16550::new(base);
        s.attach_apic(apic.clone());
        s.write(base + REG_IER, 1, 1).unwrap();
        s.push_input(&[0xAA; FIFO_SIZE + 1]);
        assert_eq!(s.rx_count(), FIFO_SIZE);
        assert_ne!(s.read(base + REG_LSR, 1).unwrap() as u8 & LSR_OVERRUN, 0);
        assert_eq!(apic.borrow_mut().pending_vector(), Some(0x24));
        s.reset();
        assert!(!s.input_pending());
    }

    #[test]
    fn lossless_host_input_drains_past_fifo_capacity() {
        let base = 0x3F8;
        let mut s = Serial16550::new(base);
        let input = b"0123456789abcdefghijklmnop\r";
        s.push_input_lossless(input);

        let mut received = Vec::new();
        for _ in input {
            received.push(s.read(base, 1).unwrap() as u8);
        }

        assert_eq!(received, input);
        assert!(!s.input_pending());
    }

    #[test]
    fn host_console_translates_lf_without_breaking_crlf() {
        let mut output = Vec::new();
        let mut previous_was_cr = false;
        write_host_console(&mut output, b"one\ntwo\r\n", &mut previous_was_cr).unwrap();

        assert_eq!(output, b"one\r\ntwo\r\n");
    }
}
