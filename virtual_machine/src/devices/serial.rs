//! 16550-compatible serial port emulation (port-mapped I/O).

use super::{DeviceError, PortDevice};
use super::{ApicTrigger, LocalApic};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::rc::Rc;
use std::time::{Duration, Instant};

const REG_DATA: u16 = 0x00;
const REG_IER: u16 = 0x01;
const REG_IIR: u16 = 0x02;
const REG_FCR: u16 = 0x02;
const REG_LCR: u16 = 0x03;
const REG_MCR: u16 = 0x04;
const REG_LSR: u16 = 0x05;
const REG_MSR: u16 = 0x06;
const REG_SCR: u16 = 0x07;

const LCR_DLAB: u8 = 0x80;
const LSR_DATA_READY: u8 = 0x01;
const LSR_OVERRUN: u8 = 0x02;
const LSR_THR_EMPTY: u8 = 0x20;
const LSR_TRANSMIT_EMPTY: u8 = 0x40;

const FIFO_SIZE: usize = 16;
const OUTPUT_LIMIT: usize = 1024 * 1024;
const OUTPUT_COMPACTION_THRESHOLD: usize = OUTPUT_LIMIT * 2;
const GUEST_PANIC_MARKER: &[u8] = b"KERNEL PANIC";
const ENROLLMENT_MARKER: &[u8] = b"\x1b]GhostOSEnroll\x07";
const LOGIN_MARKER: &[u8] = b"\x1b]GhostOSLogin\x07";
const AUTHORIZED_PROMPT: &[u8] = b"$ ";
const AUTH_SPINNER_FRAMES: &[u8] = b"|/-\\";
const AUTH_SPINNER_INTERVAL: Duration = Duration::from_millis(80);
const AUTH_HOLD_KEEP: usize = 32;
const AUTHENTICATION_MARKERS: [(&[u8], &[u8]); 2] = [
    (ENROLLMENT_MARKER, b"Administrator username: "),
    (LOGIN_MARKER, b"Username: "),
];

pub(crate) fn write_host_console<W: Write>(
    output: &mut W,
    bytes: &[u8],
    previous_was_cr: &mut bool,
) -> io::Result<()> {
    let mut translated = Vec::with_capacity(bytes.len());
    for &byte in bytes {
        if byte == b'\n' && !*previous_was_cr {
            translated.push(b'\r');
        }
        translated.push(byte);
        *previous_was_cr = byte == b'\r';
    }
    output.write_all(&translated)
}

fn pending_authentication_marker_bytes(output: &[u8]) -> usize {
    AUTHENTICATION_MARKERS
        .iter()
        .flat_map(|(marker, _)| 1..marker.len())
        .filter(|length| {
            AUTHENTICATION_MARKERS.iter().any(|(marker, _)| {
                *length < marker.len() && output.ends_with(&marker[..*length])
            })
        })
        .max()
        .unwrap_or(0)
}

fn authorized_prompt_offset(bytes: &[u8], allow_bare: bool) -> Option<usize> {
    if allow_bare && bytes.starts_with(AUTHORIZED_PROMPT) {
        return Some(0)
    }
    bytes.windows(1 + AUTHORIZED_PROMPT.len()).position(|window| {
        matches!(window[0], b'\n' | b'\r') && window[1..] == *AUTHORIZED_PROMPT
    }).map(|index| index + 1)
}

fn is_only_authorized_prompt(bytes: &[u8]) -> bool {
    let mut index = 0;
    while index < bytes.len() && matches!(bytes[index], b'\r' | b'\n') {
        index += 1;
    }
    bytes.get(index..) == Some(AUTHORIZED_PROMPT)
}

fn compact_held_output(output: &mut Vec<u8>, pending_marker_bytes: usize) {
    let keep = AUTH_HOLD_KEEP + pending_marker_bytes;
    if output.len() > keep {
        let drop = output.len() - keep;
        output.drain(..drop);
    }
}

fn drop_auth_progress_frames(output: &mut Vec<u8>) {
    const PREFIX: &[u8] = b"GhostOS authentication:";
    loop {
        let start = output
            .windows(PREFIX.len())
            .position(|bytes| bytes == PREFIX)
            .map(|index| {
                if index > 0 && output[index - 1] == b'\r' {
                    index - 1
                } else {
                    index
                }
            });
        let Some(start) = start else { return };
        let from = start + PREFIX.len();
        let end = output[from..]
            .iter()
            .position(|&byte| byte == b'\n')
            .map(|offset| from + offset + 1)
            .unwrap_or(output.len());
        output.drain(start..end);
    }
}

/// Emulated 16550 UART. Output is redirected to `std::io::stdout` so a guest
/// kernel can print debug messages.
pub struct Serial16550 {
    base: u16,
    dlab: bool,
    divisor_low: u8,
    divisor_high: u8,
    ier: u8,
    fcr: u8,
    lcr: u8,
    mcr: u8,
    lsr: u8,
    msr: u8,
    scratch: u8,
    tx_buffer: [u8; FIFO_SIZE],
    tx_count: usize,
    host_last_was_cr: bool,
    host_output: Vec<u8>,
    authentication_banner: Vec<u8>,
    auth_waiting: bool,
    prompt_shown: bool,
    spinner_frame: u8,
    spinner_drawn: bool,
    last_spinner: Instant,
    rx_buffer: VecDeque<u8>,
    pending_input: VecDeque<u8>,
    output: Vec<u8>,
    panic_marker_progress: usize,
    panic_detected: bool,
    apic: Option<Rc<RefCell<LocalApic>>>,
    irq_vector: u8,
}

impl Serial16550 {
    pub fn new(base: u16) -> Self {
        Self {
            base,
            dlab: false,
            divisor_low: 0x0C,
            divisor_high: 0x00,
            ier: 0,
            fcr: 0,
            lcr: 0x03,
            mcr: 0,
            lsr: LSR_THR_EMPTY | LSR_TRANSMIT_EMPTY,
            msr: 0,
            scratch: 0,
            tx_buffer: [0; FIFO_SIZE],
            tx_count: 0,
            host_last_was_cr: false,
            host_output: Vec::with_capacity(4096),
            authentication_banner: Vec::new(),
            auth_waiting: false,
            prompt_shown: false,
            spinner_frame: 0,
            spinner_drawn: false,
            last_spinner: Instant::now(),
            rx_buffer: VecDeque::new(),
            pending_input: VecDeque::new(),
            output: Vec::new(),
            panic_marker_progress: 0,
            panic_detected: false,
            apic: None,
            irq_vector: 0x24,
        }
    }

    pub fn base(&self) -> u16 {
        self.base
    }

    /// Connect the UART's receive interrupt to the local APIC.
    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.apic = Some(apic)
    }

    pub fn set_irq_vector(&mut self, vector: u8) {
        self.irq_vector = vector
    }

    /// Put host input into the UART receive FIFO.
    pub fn push_input(&mut self, bytes: &[u8]) {
        self.refill_rx_buffer();
        let was_empty = self.rx_buffer.is_empty();
        for &byte in bytes {
            if self.rx_buffer.len() >= FIFO_SIZE {
                self.lsr |= LSR_OVERRUN;
                break;
            }
            self.rx_buffer.push_back(byte);
        }
        if was_empty && !self.rx_buffer.is_empty() {
            self.signal_receive_irq()
        }
    }

    /// Queue host input without losing bytes when a paste is larger than the
    /// emulated UART FIFO. The guest still sees a 16550-sized FIFO; excess
    /// input waits here until the guest reads it.
    pub(crate) fn push_input_lossless(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return
        }
        let was_empty = self.rx_buffer.is_empty();
        self.pending_input.extend(bytes.iter().copied());
        self.refill_rx_buffer();
        if was_empty && !self.rx_buffer.is_empty() {
            self.signal_receive_irq()
        }
    }

    pub fn input_pending(&self) -> bool {
        !self.rx_buffer.is_empty() || !self.pending_input.is_empty()
    }

    pub fn set_authentication_banner(&mut self, banner: &[u8]) {
        if self.authentication_banner != banner {
            self.authentication_banner.clear();
            self.authentication_banner.extend_from_slice(banner);
        }
    }

    pub fn output(&self) -> &[u8] {
        &self.output
    }

    pub fn guest_panicked(&self) -> bool {
        self.panic_detected
    }

    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }

    /// Flush bytes waiting in the transmit FIFO.
    pub fn flush(&mut self) {
        self.flush_output();
        let writable = if self.host_output.is_empty() {
            0
        } else {
            self.insert_authentication_banner()
        };
        if writable == 0 && !self.auth_waiting {
            return
        }
        let mut stdout = std::io::stdout().lock();
        if writable != 0 {
            if self.spinner_drawn {
                let _ = stdout.write_all(b"\r\x1b[K");
                self.spinner_drawn = false;
            }
            let _ = stdout.write_all(&self.host_output[..writable]);
            self.host_output.drain(..writable);
        }
        if self.auth_waiting {
            self.write_auth_spinner(&mut stdout);
        }
        let _ = stdout.flush();
    }

    fn insert_authentication_banner(&mut self) -> usize {
        let passkey_web = !self.authentication_banner.is_empty();
        let mut waiting_from = if self.auth_waiting { Some(0) } else { None };
        while let Some((marker_start, marker, fallback)) = AUTHENTICATION_MARKERS
            .iter()
            .filter_map(|(marker, fallback)| {
                self.host_output
                    .windows(marker.len())
                    .position(|bytes| bytes == *marker)
                    .map(|start| (start, *marker, *fallback))
            })
            .min_by_key(|(start, _, _)| *start)
        {
            let marker_end = marker_start + marker.len();
            let replacement = if passkey_web {
                if waiting_from.is_none() {
                    waiting_from = Some(marker_start);
                }
                &[] as &[u8]
            } else {
                fallback
            };
            self.host_output.splice(
                marker_start..marker_end,
                replacement.iter().copied(),
            );
        }
        if waiting_from.is_some() {
            self.auth_waiting = true;
        }
        drop_auth_progress_frames(&mut self.host_output);
        let pending_marker_bytes = pending_authentication_marker_bytes(&self.host_output);
        let complete_end = self.host_output.len() - pending_marker_bytes;
        if !self.auth_waiting {
            if self.prompt_shown && is_only_authorized_prompt(&self.host_output[..complete_end]) {
                self.host_output.drain(..complete_end);
                return 0
            }
            if authorized_prompt_offset(&self.host_output[..complete_end], true).is_some() {
                self.prompt_shown = true;
            } else if self.host_output[..complete_end]
                .iter()
                .any(|&byte| !matches!(byte, b'\r' | b'\n'))
            {
                self.prompt_shown = false;
            }
            return complete_end
        }
        if self.panic_detected {
            self.auth_waiting = false;
            return complete_end
        }
        let search_from = waiting_from.unwrap_or(0).min(complete_end);
        let allow_bare = !(self.prompt_shown && search_from == 0);
        if let Some(relative) = authorized_prompt_offset(
            &self.host_output[search_from..complete_end],
            allow_bare,
        )
        {
            let prompt_at = search_from + relative;
            self.host_output.drain(..prompt_at);
            self.auth_waiting = false;
            self.prompt_shown = true;
            return self.host_output.len() - pending_marker_bytes
        }
        let hold_from = waiting_from.unwrap_or(0).min(complete_end);
        if hold_from == 0 {
            compact_held_output(&mut self.host_output, pending_marker_bytes);
        }
        hold_from
    }

    fn write_auth_spinner(&mut self, stdout: &mut impl Write) {
        let now = Instant::now();
        if self.spinner_drawn && now.duration_since(self.last_spinner) < AUTH_SPINNER_INTERVAL {
            return
        }
        let frame = AUTH_SPINNER_FRAMES[self.spinner_frame as usize];
        self.spinner_frame = (self.spinner_frame + 1) % AUTH_SPINNER_FRAMES.len() as u8;
        self.last_spinner = now;
        self.spinner_drawn = true;
        let _ = stdout.write_all(&[b'\r', frame, 0x1b, b'[', b'K']);
    }

    fn signal_receive_irq(&mut self) {
        if self.ier & 0x01 == 0 {
            return
        }
        if let Some(apic) = &self.apic {
            apic.borrow_mut().signal(self.irq_vector, ApicTrigger::Edge);
        }
    }

    fn flush_output(&mut self) {
        if self.tx_count == 0 {
            return
        }

        let tx_count = self.tx_count;
        let _ = write_host_console(
            &mut self.host_output,
            &self.tx_buffer[..tx_count],
            &mut self.host_last_was_cr,
        );
        if self.output.len() > OUTPUT_COMPACTION_THRESHOLD {
            let excess = self.output.len() - OUTPUT_LIMIT;
            self.output.drain(..excess);
        }
        self.tx_count = 0;
    }

    fn observe_panic_marker(&mut self, byte: u8) {
        if self.panic_detected {
            return
        }

        if byte == GUEST_PANIC_MARKER[self.panic_marker_progress] {
            self.panic_marker_progress += 1;
            if self.panic_marker_progress == GUEST_PANIC_MARKER.len() {
                self.panic_detected = true;
            }
        } else {
            self.panic_marker_progress = usize::from(byte == GUEST_PANIC_MARKER[0]);
        }
    }

    fn refill_rx_buffer(&mut self) {
        while self.rx_buffer.len() < FIFO_SIZE {
            let Some(byte) = self.pending_input.pop_front() else {
                break
            };
            self.rx_buffer.push_back(byte)
        }
    }
}

impl PortDevice for Serial16550 {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        if size != 1 {
            return Err(DeviceError::UnsupportedSize);
        }
        let off = (port - self.base) & 0x07;
        let reg = match off {
            REG_DATA => {
                if self.dlab {
                    self.divisor_low
                } else {
                    let byte = self.rx_buffer.pop_front().unwrap_or(0);
                    self.refill_rx_buffer();
                    byte
                }
            }
            REG_IER => {
                if self.dlab {
                    self.divisor_high
                } else {
                    self.ier
                }
            }
            REG_IIR => {
                if !self.rx_buffer.is_empty() && self.ier & 0x01 != 0 {
                    0x04
                } else {
                    0x01
                }
            }
            REG_LCR => self.lcr,
            REG_MCR => self.mcr,
            REG_LSR => {
                if self.rx_buffer.is_empty() {
                    self.lsr & !LSR_DATA_READY
                } else {
                    self.lsr | LSR_DATA_READY
                }
            }
            REG_MSR => self.msr,
            REG_SCR => self.scratch,
            _ => 0xFF,
        };
        Ok(reg as u64)
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 1 {
            return Err(DeviceError::UnsupportedSize);
        }
        let v = value as u8;
        let off = (port - self.base) & 0x07;
        match off {
            REG_DATA => {
                if self.dlab {
                    self.divisor_low = v;
                }
            }
            REG_IER => {
                if self.dlab {
                    self.divisor_high = v;
                } else {
                    self.ier = v;
                    if v & 0x01 != 0 && !self.rx_buffer.is_empty() {
                        self.signal_receive_irq()
                    }
                }
            }
            REG_FCR => {
                self.fcr = v;
                if v & 0x02 != 0 {
                    self.rx_buffer.clear();
                }
                if v & 0x04 != 0 {
                    self.tx_count = 0;
                }
            }
            REG_LCR => {
                self.lcr = v;
                self.dlab = v & LCR_DLAB != 0;
            }
            REG_MCR => self.mcr = v,
            REG_SCR => self.scratch = v,
            _ => {}
        }

        // If a byte is written to the data register while DLAB is clear, treat
        // it as a character to transmit on the host console.
        if off == REG_DATA && !self.dlab {
            if self.tx_count >= FIFO_SIZE {
                self.flush_output();
            }
            self.tx_buffer[self.tx_count] = v;
            self.tx_count += 1;
            self.output.push(v);
            self.observe_panic_marker(v);
            if matches!(v, b'\n' | b'\r') {
                self.flush_output();
            }
        }

        Ok(())
    }

    fn reset(&mut self) {
        *self = Self::new(self.base);
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
        assert_eq!(s.tx_count, 1);

        // Set DLAB.
        s.write(base + REG_LCR, 0x80, 1).unwrap();
        assert!(s.dlab);
        assert!(s.read(base + REG_LCR, 1).unwrap() & 0x80 != 0);

        // Divisor registers map onto 0/1 while DLAB is set.
        s.write(base + REG_DATA, 0x01, 1).unwrap();
        s.write(base + REG_IER, 0x00, 1).unwrap();
        assert_eq!(s.divisor_low, 0x01);

        // Clear DLAB and confirm data register goes back to TX.
        s.write(base + REG_LCR, 0x03, 1).unwrap();
        assert!(!s.dlab);
        s.write(base + REG_DATA, b'B' as u64, 1).unwrap();
        assert_eq!(s.tx_count, 2);
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

        assert_eq!(s.tx_count, 0);
        assert_eq!(s.output(), b"\r");
    }

    #[test]
    fn authentication_banner_holds_setup_until_prompt() {
        let mut s = Serial16550::new(0x3F8);
        s.set_authentication_banner(b"Passkey setup and login: http://localhost:1234/?code=test\r\n");
        s.host_output.extend_from_slice(
            b"Ready\r\n\x1b]GhostOSEnroll\x07Do not type credentials in this terminal.\r\n",
        );

        let writable = s.insert_authentication_banner();

        assert_eq!(&s.host_output[..writable], b"Ready\r\n");
        s.host_output.drain(..writable);
        assert!(s.auth_waiting);
        assert_eq!(
            s.host_output,
            b"Do not type credentials in this terminal.\r\n"
        );

        s.host_output.extend_from_slice(
            b"Administrator account committed.\r\nGhostOS user shell\r\n$ ",
        );
        let writable = s.insert_authentication_banner();
        assert!(!s.auth_waiting);
        assert_eq!(&s.host_output[..writable], b"$ ");
    }

    #[test]
    fn partial_authentication_marker_waits_for_next_flush() {
        let mut s = Serial16550::new(0x3F8);
        s.set_authentication_banner(b"Passkey URL\r\n");
        s.host_output.extend_from_slice(b"Ready\r\n\x1b]Ghost");

        let writable = s.insert_authentication_banner();
        assert_eq!(&s.host_output[..writable], b"Ready\r\n");
        s.host_output.drain(..writable);
        s.host_output
            .extend_from_slice(b"OSEnroll\x07");

        let writable = s.insert_authentication_banner();
        assert_eq!(writable, 0);
        assert!(s.auth_waiting);
        assert!(s.host_output.is_empty());
    }

    #[test]
    fn authentication_marker_becomes_manual_prompt_without_banner() {
        let mut s = Serial16550::new(0x3F8);
        s.host_output.extend_from_slice(ENROLLMENT_MARKER);

        let writable = s.insert_authentication_banner();

        assert_eq!(writable, s.host_output.len());
        assert_eq!(s.host_output, b"Administrator username: ");
        assert!(!s.auth_waiting);
    }

    #[test]
    fn login_marker_holds_output_until_prompt() {
        let mut s = Serial16550::new(0x3F8);
        s.set_authentication_banner(b"Passkey URL\r\n");
        s.host_output.extend_from_slice(LOGIN_MARKER);
        s.host_output.extend_from_slice(b"Username: micky\r\nLogin accepted.\r\n");

        let writable = s.insert_authentication_banner();
        assert_eq!(writable, 0);
        assert!(s.auth_waiting);

        s.host_output.extend_from_slice(b"$ ");
        let writable = s.insert_authentication_banner();
        assert!(!s.auth_waiting);
        assert_eq!(&s.host_output[..writable], b"$ ");
    }

    #[test]
    fn extra_authorized_prompt_is_not_reprinted() {
        let mut s = Serial16550::new(0x3F8);
        s.set_authentication_banner(b"Passkey URL\r\n");
        s.host_output.extend_from_slice(b"$ ");
        let writable = s.insert_authentication_banner();
        assert_eq!(&s.host_output[..writable], b"$ ");
        s.host_output.drain(..writable);

        s.host_output.extend_from_slice(b"\n$ ");
        let writable = s.insert_authentication_banner();
        assert_eq!(writable, 0);
        assert!(!s.auth_waiting);
    }

    #[test]
    fn leftover_prompt_does_not_end_a_later_login_wait() {
        let mut s = Serial16550::new(0x3F8);
        s.set_authentication_banner(b"Passkey URL\r\n");
        s.host_output.extend_from_slice(b"$ ");
        let writable = s.insert_authentication_banner();
        s.host_output.drain(..writable);

        s.host_output.extend_from_slice(LOGIN_MARKER);
        s.host_output.extend_from_slice(b"$ ");
        let writable = s.insert_authentication_banner();
        assert_eq!(writable, 0);
        assert!(s.auth_waiting);

        s.host_output.extend_from_slice(b"\n$ ");
        let writable = s.insert_authentication_banner();
        assert!(!s.auth_waiting);
        assert_eq!(&s.host_output[..writable], b"$ ");
    }

    #[test]
    fn authentication_progress_frames_do_not_reprint_after_prompt() {
        let mut s = Serial16550::new(0x3F8);
        s.set_authentication_banner(b"Passkey URL\r\n");
        s.host_output.extend_from_slice(b"$ \rGhostOS authentication: |");
        s.host_output.extend_from_slice(b"\rGhostOS authentication: /");
        s.host_output.extend_from_slice(b"\rGhostOS authentication: ready\n");

        let writable = s.insert_authentication_banner();

        assert!(!s.auth_waiting);
        assert_eq!(&s.host_output[..writable], b"$ ");
    }

    #[test]
    fn serial_receive_fifo_sets_overrun_and_irq() {
        let base = 0x3F8;
        let apic = Rc::new(RefCell::new(LocalApic::new(0)));
        let mut s = Serial16550::new(base);
        s.attach_apic(apic.clone());
        s.write(base + REG_IER, 1, 1).unwrap();
        s.push_input(&[0xAA; FIFO_SIZE + 1]);
        assert_eq!(s.rx_buffer.len(), FIFO_SIZE);
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
