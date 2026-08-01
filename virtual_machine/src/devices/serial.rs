//! 16550-compatible serial port emulation (port-mapped I/O).

use super::{DeviceError, PortDevice};
use super::{ApicTrigger, LocalApic};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Write;
use std::rc::Rc;

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
const LSR_THR_EMPTY: u8 = 0x20;
const LSR_TRANSMIT_EMPTY: u8 = 0x40;

const FIFO_SIZE: usize = 16;

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
    rx_buffer: VecDeque<u8>,
    output: Vec<u8>,
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
            rx_buffer: VecDeque::new(),
            output: Vec::new(),
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
        let was_empty = self.rx_buffer.is_empty();
        self.rx_buffer.extend(bytes.iter().copied());
        if was_empty && !self.rx_buffer.is_empty() {
            self.signal_receive_irq()
        }
    }

    pub fn input_pending(&self) -> bool {
        !self.rx_buffer.is_empty()
    }

    pub fn output(&self) -> &[u8] {
        &self.output
    }

    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
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
        let mut stdout = std::io::stdout().lock();
        for &b in &self.tx_buffer[..self.tx_count] {
            let _ = stdout.write_all(&[b]);
        }
        let _ = stdout.flush();
        self.output.extend_from_slice(&self.tx_buffer[..self.tx_count]);
        if self.output.len() > 1024 * 1024 {
            let excess = self.output.len() - 1024 * 1024;
            self.output.drain(..excess);
        }
        self.tx_count = 0;
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
                    self.rx_buffer.pop_front().unwrap_or(0)
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
            if v == b'\n' {
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
        assert_eq!(s.read(base + REG_LSR, 1).unwrap() & LSR_DATA_READY, 1);
        assert_eq!(s.read(base + REG_DATA, 1).unwrap(), b'h' as u64);
        assert_eq!(s.read(base + REG_DATA, 1).unwrap(), b'i' as u64);
        assert_eq!(s.read(base + REG_LSR, 1).unwrap() & LSR_DATA_READY, 0);
    }
}
