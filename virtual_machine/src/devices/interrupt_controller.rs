use super::{DeviceError, LocalApic, PortDevice};
use std::cell::RefCell;
use std::rc::Rc;

#[repr(C)]
struct CInterruptController {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn ghostos_vm_interrupt_controller_new() -> *mut CInterruptController;
    fn ghostos_vm_interrupt_controller_free(controller: *mut CInterruptController);
    fn ghostos_vm_idt_gate_decode(raw: *const u8, out: *mut IdtGate) -> bool;
    fn ghostos_vm_idt_gate_present(gate: *const IdtGate) -> bool;
    fn ghostos_vm_idt_gate_dpl(gate: *const IdtGate) -> u8;
    fn ghostos_vm_interrupt_controller_init(controller: *mut CInterruptController);
    fn ghostos_vm_interrupt_controller_set_idt(controller: *mut CInterruptController, base: u64, limit: u16);
    fn ghostos_vm_interrupt_controller_idt_entry(
        controller: *const CInterruptController,
        vector: u8,
        address: *mut u64,
    ) -> bool;
    fn ghostos_vm_interrupt_controller_map_irq(controller: *mut CInterruptController, irq: u8, vector: u64);
    fn ghostos_vm_interrupt_controller_handle_irq(controller: *const CInterruptController, irq: u8, vector: *mut u8) -> bool;
    fn ghostos_vm_interrupt_controller_remap_pic(controller: *mut CInterruptController);
    fn ghostos_vm_interrupt_controller_reset(controller: *mut CInterruptController);
    fn ghostos_vm_interrupt_controller_is_mapped(controller: *const CInterruptController) -> bool;
    fn ghostos_vm_interrupt_controller_idt_limit(controller: *const CInterruptController) -> u16;
    fn ghostos_vm_interrupt_controller_idt_base(controller: *const CInterruptController) -> u64;
    fn ghostos_vm_interrupt_controller_route_at(
        controller: *const CInterruptController,
        index: u16,
        irq: *mut u8,
        vector: *mut u64,
    ) -> bool;
}

/// A decoded IDT gate descriptor (16 bytes in guest memory).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct IdtGate {
    /// 16-bit selector.
    pub selector: u16,
    /// Gate type and attributes (P, DPL, TYPE).
    pub type_attr: u8,
    /// 64-bit handler offset.
    pub offset: u64,
    /// IST index (0 means "not used").
    pub ist: u8,
}

impl IdtGate {
    /// Decode a 16-byte IDT descriptor.
    pub fn decode(raw: &[u8; 16]) -> Self {
        let mut gate = Self { selector: 0, type_attr: 0, offset: 0, ist: 0 };
        unsafe { ghostos_vm_idt_gate_decode(raw.as_ptr(), &mut gate) };
        gate
    }

    pub fn present(&self) -> bool {
        unsafe { ghostos_vm_idt_gate_present(self) }
    }

    pub fn dpl(&self) -> u8 {
        unsafe { ghostos_vm_idt_gate_dpl(self) }
    }
}

pub struct InterruptController {
    state: *mut CInterruptController,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InterruptControllerState {
    pub idt_base: u64,
    pub idt_limit: u16,
    pub irq_routing: Vec<(u8, u64)>,
    pub pic_mapped: bool,
}

impl InterruptController {
    pub fn new() -> Self {
        Self {
            state: unsafe { ghostos_vm_interrupt_controller_new() },
        }
    }

    pub fn init(&mut self) {
        unsafe { ghostos_vm_interrupt_controller_init(self.state) }
    }

    pub(crate) fn snapshot_state(&self) -> InterruptControllerState {
        InterruptControllerState {
            idt_base: unsafe { ghostos_vm_interrupt_controller_idt_base(self.state) },
            idt_limit: unsafe { ghostos_vm_interrupt_controller_idt_limit(self.state) },
            irq_routing: (0..256)
                .filter_map(|index| {
                    let mut irq = 0;
                    let mut vector = 0;
                    unsafe {
                        ghostos_vm_interrupt_controller_route_at(
                            self.state, index, &mut irq, &mut vector,
                        )
                    }
                    .then_some((irq, vector))
                })
                .collect(),
            pic_mapped: unsafe { ghostos_vm_interrupt_controller_is_mapped(self.state) },
        }
    }

    pub(crate) fn restore_state(&mut self, state: &InterruptControllerState) {
        unsafe {
            ghostos_vm_interrupt_controller_set_idt(self.state, state.idt_base, state.idt_limit);
            ghostos_vm_interrupt_controller_reset(self.state);
            for &(irq, vector) in &state.irq_routing {
                ghostos_vm_interrupt_controller_map_irq(self.state, irq, vector);
            }
            if state.pic_mapped {
                ghostos_vm_interrupt_controller_remap_pic(self.state);
            }
        }
    }

    pub fn set_idt(&mut self, base: u64, limit: u16) {
        unsafe { ghostos_vm_interrupt_controller_set_idt(self.state, base, limit) }
    }

    /// Physical address of the IDT entry for `vector`, or `None` when the IDT
    /// has not been installed yet.
    pub fn idt_entry_address(&self, vector: u8) -> Option<u64> {
        let mut address = 0;
        unsafe {
            ghostos_vm_interrupt_controller_idt_entry(self.state, vector, &mut address)
                .then_some(address)
        }
    }

    pub fn get_idt_entry(&self, vector: u8) -> u64 {
        self.idt_entry_address(vector).unwrap_or(0)
    }

    pub fn map_irq(&mut self, irq: u8, vector: u8) {
        unsafe { ghostos_vm_interrupt_controller_map_irq(self.state, irq, u64::from(vector)) }
    }

    pub fn handle_irq(&mut self, irq: u8) -> Option<u8> {
        let mut vector = 0;
        unsafe {
            ghostos_vm_interrupt_controller_handle_irq(self.state, irq, &mut vector)
                .then_some(vector)
        }
    }

    pub fn remap_pic(&mut self) {
        unsafe { ghostos_vm_interrupt_controller_remap_pic(self.state) }
    }

    pub fn reset(&mut self) {
        unsafe { ghostos_vm_interrupt_controller_reset(self.state) }
    }

    pub fn is_mapped(&self) -> bool {
        unsafe { ghostos_vm_interrupt_controller_is_mapped(self.state) }
    }
}

impl Drop for InterruptController {
    fn drop(&mut self) {
        unsafe { ghostos_vm_interrupt_controller_free(self.state) }
    }
}

impl Default for InterruptController {
    fn default() -> Self {
        Self::new()
    }
}

/// Minimal legacy PIC port model.
///
/// GhostOS acknowledges ISA interrupts through the 8259A command ports while
/// this VM delivers those interrupts through the local APIC. Forwarding EOI
/// commands keeps both interrupt models in sync.
pub struct LegacyPic {
    apic: Rc<RefCell<LocalApic>>,
}

impl LegacyPic {
    pub fn new(apic: Rc<RefCell<LocalApic>>) -> Self {
        Self { apic }
    }
}

impl PortDevice for LegacyPic {
    fn read(&mut self, _port: u16, size: u8) -> Result<u64, DeviceError> {
        if size == 1 {
            Ok(0)
        } else {
            Err(DeviceError::UnsupportedSize)
        }
    }

    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 1 {
            return Err(DeviceError::UnsupportedSize);
        }
        if (port == 0x20 || port == 0xa0) && value as u8 == 0x20 {
            self.apic.borrow_mut().eoi();
        }
        Ok(())
    }

    fn reset(&mut self) {}
}
