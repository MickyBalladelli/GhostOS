use std::collections::BTreeMap;

/// A decoded IDT gate descriptor (16 bytes in guest memory).
#[derive(Clone, Copy, Debug)]
pub struct IdtGate {
    /// 16-bit selector.
    pub selector: u16,
    /// 16-bit attributes / type field (P, DPL, TYPE).
    pub type_attr: u16,
    /// 64-bit handler offset.
    pub offset: u64,
    /// IST index (0 means "not used").
    pub ist: u8,
}

impl IdtGate {
    /// Decode a 16-byte IDT descriptor.
    pub fn decode(raw: &[u8; 16]) -> Self {
        let offset_lo = u16::from_le_bytes([raw[0], raw[1]]) as u64;
        let selector = u16::from_le_bytes([raw[2], raw[3]]);
        let ist = raw[4] & 0x07;
        let type_attr = u16::from_le_bytes([raw[5], raw[6]]);
        let offset_mid = u16::from_le_bytes([raw[7], raw[8]]) as u64;
        let offset_hi = u32::from_le_bytes([raw[9], raw[10], raw[11], raw[12]]) as u64;
        let offset = offset_lo | (offset_mid << 16) | (offset_hi << 32);
        Self {
            selector,
            type_attr,
            offset,
            ist,
        }
    }

    pub fn present(&self) -> bool {
        self.type_attr & (1 << 15) != 0
    }

    pub fn dpl(&self) -> u8 {
        ((self.type_attr >> 13) & 0x03) as u8
    }
}

pub struct InterruptController {
    idt_base: u64,
    idt_limit: u16,
    irq_routing: BTreeMap<u8, u64>,
    pic_mapped: bool,
}

impl InterruptController {
    pub fn new() -> Self {
        Self {
            idt_base: 0,
            idt_limit: 0,
            irq_routing: BTreeMap::new(),
            pic_mapped: false,
        }
    }

    pub fn init(&mut self) {
        self.idt_base = 0x1000;
        self.idt_limit = 0;
        self.pic_mapped = false;
    }

    pub fn set_idt(&mut self, base: u64, limit: u16) {
        self.idt_base = base;
        self.idt_limit = limit;
    }

    /// Physical address of the IDT entry for `vector`, or `None` when the IDT
    /// has not been installed yet.
    pub fn idt_entry_address(&self, vector: u8) -> Option<u64> {
        if self.idt_base == 0 || (vector as u32 * 16 + 16) > (self.idt_limit as u32 + 1) {
            return None;
        }
        Some(self.idt_base + (vector as u64) * 16)
    }

    pub fn get_idt_entry(&self, vector: u8) -> u64 {
        self.idt_entry_address(vector).unwrap_or(0)
    }

    pub fn map_irq(&mut self, irq: u8, vector: u8) {
        self.irq_routing.insert(irq, vector as u64);
    }

    pub fn handle_irq(&mut self, irq: u8) -> Option<u8> {
        self.irq_routing.get(&irq).copied().map(|v| v as u8)
    }

    pub fn remap_pic(&mut self) {
        self.pic_mapped = true;
    }

    pub fn reset(&mut self) {
        self.irq_routing.clear();
        self.pic_mapped = false;
    }

    pub fn is_mapped(&self) -> bool {
        self.pic_mapped
    }
}

impl Default for InterruptController {
    fn default() -> Self {
        Self::new()
    }
}