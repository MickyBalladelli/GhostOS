//! PS/2 controller emulation for keyboard and mouse input.

use super::{ApicTrigger, DeviceError, LocalApic, PortDevice};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub const PS2_DATA_PORT: u16 = 0x60;
pub const PS2_STATUS_PORT: u16 = 0x64;
pub const PS2_PORT_COUNT: u16 = 5;

const STATUS_OUTPUT_FULL: u8 = 1 << 0;
const STATUS_AUX_DATA: u8 = 1 << 5;
const COMMAND_READ_BYTE: u8 = 0x20;
const COMMAND_WRITE_BYTE: u8 = 0x60;
const COMMAND_DISABLE_MOUSE: u8 = 0xA7;
const COMMAND_ENABLE_MOUSE: u8 = 0xA8;
const COMMAND_DISABLE_KEYBOARD: u8 = 0xAD;
const COMMAND_ENABLE_KEYBOARD: u8 = 0xAE;
const COMMAND_SELF_TEST: u8 = 0xAA;
const COMMAND_INTERFACE_TEST: u8 = 0xAB;
const COMMAND_WRITE_MOUSE: u8 = 0xD4;

const KEYBOARD_IRQ_VECTOR: u8 = 0x21;
const MOUSE_IRQ_VECTOR: u8 = 0x2C;

#[derive(Clone, Copy)]
struct OutputByte {
    value: u8,
    auxiliary: bool,
}

/// Minimal dual-channel i8042 controller with PS/2 set-2 command handling.
/// Host input is injected through the public queue methods and appears at the
/// normal guest ports 0x60 and 0x64.
pub struct Ps2Controller {
    command_byte: u8,
    output: VecDeque<OutputByte>,
    expecting_command_byte: bool,
    expecting_mouse_command: bool,
    keyboard_enabled: bool,
    mouse_enabled: bool,
    mouse_streaming: bool,
    apic: Option<Rc<RefCell<LocalApic>>>,
    keyboard_irq_vector: u8,
    mouse_irq_vector: u8,
}

impl Ps2Controller {
    pub fn new() -> Self {
        Self {
            command_byte: 0x45,
            output: VecDeque::new(),
            expecting_command_byte: false,
            expecting_mouse_command: false,
            keyboard_enabled: true,
            mouse_enabled: false,
            mouse_streaming: false,
            apic: None,
            keyboard_irq_vector: KEYBOARD_IRQ_VECTOR,
            mouse_irq_vector: MOUSE_IRQ_VECTOR,
        }
    }

    pub fn attach_apic(&mut self, apic: Rc<RefCell<LocalApic>>) {
        self.apic = Some(apic)
    }

    pub fn set_keyboard_irq_vector(&mut self, vector: u8) {
        self.keyboard_irq_vector = vector
    }

    pub fn set_mouse_irq_vector(&mut self, vector: u8) {
        self.mouse_irq_vector = vector
    }

    /// Inject one keyboard scan-code byte into the guest output buffer.
    pub fn push_keyboard_scancode(&mut self, scancode: u8) {
        self.push_output(OutputByte {
            value: scancode,
            auxiliary: false,
        })
    }

    pub fn push_keyboard_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.push_keyboard_scancode(byte)
        }
    }

    /// Inject a raw three-byte PS/2 mouse packet.
    pub fn push_mouse_packet(&mut self, packet: [u8; 3]) {
        if !self.mouse_enabled || !self.mouse_streaming {
            return
        }
        for value in packet {
            self.push_output(OutputByte {
                value,
                auxiliary: true,
            })
        }
    }

    /// Convert relative mouse movement into a standard PS/2 packet.
    pub fn push_mouse_motion(&mut self, dx: i16, dy: i16, buttons: u8) {
        let dx = dx.clamp(-255, 255) as i8;
        let dy = dy.clamp(-255, 255) as i8;
        let mut first = 0x08 | (buttons & 0x07);
        if dx < 0 {
            first |= 0x10;
        }
        if dy < 0 {
            first |= 0x20;
        }
        self.push_mouse_packet([first, dx as u8, dy as u8])
    }

    pub fn input_pending(&self) -> bool {
        !self.output.is_empty()
    }

    fn keyboard_interrupt_enabled(&self) -> bool {
        self.keyboard_enabled && self.command_byte & 0x01 != 0
    }

    fn mouse_interrupt_enabled(&self) -> bool {
        self.mouse_enabled && self.command_byte & 0x02 != 0
    }

    fn push_output(&mut self, byte: OutputByte) {
        self.output.push_back(byte);
        let irq_enabled = if byte.auxiliary {
            self.mouse_interrupt_enabled()
        } else {
            self.keyboard_interrupt_enabled()
        };
        if irq_enabled {
            let vector = if byte.auxiliary {
                self.mouse_irq_vector
            } else {
                self.keyboard_irq_vector
            };
            if let Some(apic) = &self.apic {
                apic.borrow_mut().signal(vector, ApicTrigger::Edge);
            }
        }
    }

    fn handle_keyboard_command(&mut self, command: u8) {
        match command {
            0xFF => {
                self.push_output(OutputByte {
                    value: 0xFA,
                    auxiliary: false,
                });
                self.push_output(OutputByte {
                    value: 0xAA,
                    auxiliary: false,
                });
            }
            0xF2 => {
                self.push_output(OutputByte {
                    value: 0xFA,
                    auxiliary: false,
                });
                self.push_output(OutputByte {
                    value: 0xAB,
                    auxiliary: false,
                });
                self.push_output(OutputByte {
                    value: 0x83,
                    auxiliary: false,
                });
            }
            0xF4 => {
                self.push_output(OutputByte {
                    value: 0xFA,
                    auxiliary: false,
                });
                self.keyboard_enabled = true;
            }
            0xF5 => {
                self.push_output(OutputByte {
                    value: 0xFA,
                    auxiliary: false,
                });
                self.keyboard_enabled = false;
            }
            _ => self.push_output(OutputByte {
                value: 0xFA,
                auxiliary: false,
            }),
        }
    }

    fn handle_mouse_command(&mut self, command: u8) {
        match command {
            0xFF => {
                self.push_output(OutputByte {
                    value: 0xFA,
                    auxiliary: true,
                });
                self.push_output(OutputByte {
                    value: 0xAA,
                    auxiliary: true,
                });
                self.push_output(OutputByte {
                    value: 0x00,
                    auxiliary: true,
                });
            }
            0xF2 => {
                self.push_output(OutputByte {
                    value: 0xFA,
                    auxiliary: true,
                });
                self.push_output(OutputByte {
                    value: 0x00,
                    auxiliary: true,
                });
            }
            0xF4 => {
                self.mouse_streaming = true;
                self.push_output(OutputByte {
                    value: 0xFA,
                    auxiliary: true,
                });
            }
            0xF5 => {
                self.mouse_streaming = false;
                self.push_output(OutputByte {
                    value: 0xFA,
                    auxiliary: true,
                });
            }
            _ => self.push_output(OutputByte {
                value: 0xFA,
                auxiliary: true,
            }),
        }
    }
}

impl Default for Ps2Controller {
    fn default() -> Self {
        Self::new()
    }
}

impl PortDevice for Ps2Controller {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        if size != 1 {
            return Err(DeviceError::UnsupportedSize);
        }
        match port {
            PS2_DATA_PORT => Ok(self.output.pop_front().map(|byte| byte.value).unwrap_or(0) as u64),
            PS2_STATUS_PORT => {
                let mut status = 0;
                if let Some(byte) = self.output.front() {
                    status |= STATUS_OUTPUT_FULL;
                    if byte.auxiliary {
                        status |= STATUS_AUX_DATA;
                    }
                }
                Ok(status as u64)
            }
            _ => Err(DeviceError::InvalidAddress),
        }
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 1 {
            return Err(DeviceError::UnsupportedSize);
        }
        let value = value as u8;
        match port {
            PS2_STATUS_PORT => match value {
                COMMAND_READ_BYTE => self.push_output(OutputByte {
                    value: self.command_byte,
                    auxiliary: false,
                }),
                COMMAND_WRITE_BYTE => self.expecting_command_byte = true,
                COMMAND_DISABLE_MOUSE => {
                    self.mouse_enabled = false;
                    self.command_byte |= 0x20;
                }
                COMMAND_ENABLE_MOUSE => {
                    self.mouse_enabled = true;
                    self.command_byte &= !0x20;
                }
                COMMAND_DISABLE_KEYBOARD => {
                    self.keyboard_enabled = false;
                    self.command_byte |= 0x10;
                }
                COMMAND_ENABLE_KEYBOARD => {
                    self.keyboard_enabled = true;
                    self.command_byte &= !0x10;
                }
                COMMAND_SELF_TEST => self.push_output(OutputByte {
                    value: 0x55,
                    auxiliary: false,
                }),
                COMMAND_INTERFACE_TEST => self.push_output(OutputByte {
                    value: 0x00,
                    auxiliary: false,
                }),
                COMMAND_WRITE_MOUSE => self.expecting_mouse_command = true,
                _ => {}
            },
            PS2_DATA_PORT => {
                if self.expecting_command_byte {
                    self.command_byte = value;
                    self.keyboard_enabled = value & 0x10 == 0;
                    self.mouse_enabled = value & 0x20 == 0;
                    self.expecting_command_byte = false;
                } else if self.expecting_mouse_command {
                    self.expecting_mouse_command = false;
                    self.handle_mouse_command(value);
                } else {
                    self.handle_keyboard_command(value);
                }
            }
            _ => return Err(DeviceError::InvalidAddress),
        }
        Ok(())
    }

    fn reset(&mut self) {
        *self = Self::new();
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
}
