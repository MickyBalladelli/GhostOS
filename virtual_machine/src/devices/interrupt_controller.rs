use std::collections::BTreeMap;

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

    pub fn get_idt_entry(&self, vector: u8) -> u64 {
        if vector as usize >= 256 {
            return 0;
        }
        
        let entry_size: u64 = 16;
        let offset = self.idt_base + ((vector as usize) as u64 * entry_size);
        offset
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