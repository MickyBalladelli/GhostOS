use core::mem::MaybeUninit;
use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU8, Ordering};
use synos_legacy_pc_drivers::pci::{Bar, ConfigAccess, PortConfig, enumerate};

const TRB_COUNT: usize = 256;
const EVENT_COUNT: usize = 256;
const CONTEXT_BYTES: usize = 4096;
const BUFFER_BYTES: usize = 4096;
const TYPE_NORMAL: u32 = 1;
const TYPE_SETUP_STAGE: u32 = 2;
const TYPE_DATA_STAGE: u32 = 3;
const TYPE_STATUS_STAGE: u32 = 4;
const TYPE_ENABLE_SLOT: u32 = 9;
const TYPE_DISABLE_SLOT: u32 = 10;
const TYPE_ADDRESS_DEVICE: u32 = 11;
const TYPE_CONFIGURE_ENDPOINT: u32 = 12;
const TYPE_EVALUATE_CONTEXT: u32 = 13;
const TYPE_TRANSFER_EVENT: u32 = 32;
const TYPE_COMMAND_COMPLETION: u32 = 33;
const COMPLETION_SUCCESS: u8 = 1;
const COMPLETION_SHORT_PACKET: u8 = 13;

#[repr(C, align(16))]
#[derive(Clone, Copy)]
struct Trb {
    parameter_low: u32,
    parameter_high: u32,
    status: u32,
    control: u32,
}

impl Trb {
    const EMPTY: Self = Self {
        parameter_low: 0,
        parameter_high: 0,
        status: 0,
        control: 0,
    };

    fn new(parameter: u64, status: u32, control: u32) -> Self {
        Self {
            parameter_low: parameter as u32,
            parameter_high: (parameter >> 32) as u32,
            status,
            control,
        }
    }

    const fn trb_type(self) -> u32 {
        (self.control >> 10) & 0x3f
    }

    const fn completion_code(self) -> u8 {
        (self.status >> 24) as u8
    }
}

#[repr(C, align(4096))]
struct TrbPage([Trb; TRB_COUNT]);

#[repr(C, align(4096))]
struct BytePage([u8; BUFFER_BYTES]);

#[repr(C, align(4096))]
struct U64Page([u64; 512]);

static mut COMMAND_RING: TrbPage = TrbPage([Trb::EMPTY; TRB_COUNT]);
static mut EVENT_RING: TrbPage = TrbPage([Trb::EMPTY; EVENT_COUNT]);
static mut EP0_RING: TrbPage = TrbPage([Trb::EMPTY; TRB_COUNT]);
static mut INTERRUPT_RING: TrbPage = TrbPage([Trb::EMPTY; TRB_COUNT]);
static mut DCBAA: U64Page = U64Page([0; 512]);
static mut ERST: U64Page = U64Page([0; 512]);
static mut DEVICE_CONTEXT: BytePage = BytePage([0; CONTEXT_BYTES]);
static mut INPUT_CONTEXT: BytePage = BytePage([0; CONTEXT_BYTES]);
static mut CONTROL_BUFFER: BytePage = BytePage([0; BUFFER_BYTES]);
static mut REPORT_BUFFER: BytePage = BytePage([0; BUFFER_BYTES]);

#[derive(Clone, Copy)]
struct Event {
    trb: Trb,
}

struct ProducerRing {
    address: *mut Trb,
    index: usize,
    cycle: u32,
}

impl ProducerRing {
    unsafe fn new(address: *mut Trb) -> Self {
        Self {
            address,
            index: 0,
            cycle: 1,
        }
    }

    unsafe fn push(&mut self, mut trb: Trb) {
        trb.control = (trb.control & !1) | self.cycle;
        unsafe { write_volatile(self.address.add(self.index), trb) }
        self.index += 1;
        if self.index == TRB_COUNT - 1 {
            let address = self.address as u64;
            let link = Trb::new(address, 0, (6 << 10) | (1 << 1) | self.cycle);
            unsafe { write_volatile(self.address.add(TRB_COUNT - 1), link) }
            self.index = 0;
            self.cycle ^= 1
        }
    }

    fn dequeue(&self) -> u64 {
        unsafe { self.address.add(self.index) as u64 | self.cycle as u64 }
    }
}

struct Xhci {
    operational: *mut u8,
    runtime: *mut u8,
    doorbells: *mut u8,
    context_size: usize,
    max_ports: u8,
    event_index: usize,
    event_cycle: u32,
    command: ProducerRing,
    ep0: ProducerRing,
    interrupt: ProducerRing,
    slot_id: u8,
    endpoint_id: u8,
    interrupt_packet_size: u16,
}

pub struct UsbKeyboard {
    controller: Xhci,
    previous: [u8; 6],
    caps_lock: bool,
    pending: [u8; 6],
    pending_start: usize,
    pending_count: usize,
}

static mut BOOT_KEYBOARD: MaybeUninit<UsbKeyboard> = MaybeUninit::uninit();
static BOOT_KEYBOARD_STATE: AtomicU8 = AtomicU8::new(0);

pub fn read_boot_byte() -> Option<u8> {
    if BOOT_KEYBOARD_STATE.load(Ordering::Acquire) == 0 {
        let Some(keyboard) = UsbKeyboard::new() else {
            BOOT_KEYBOARD_STATE.store(2, Ordering::Release);
            return None
        };
        unsafe {
            (&raw mut BOOT_KEYBOARD).write(MaybeUninit::new(keyboard));
        }
        BOOT_KEYBOARD_STATE.store(1, Ordering::Release);
    }
    if BOOT_KEYBOARD_STATE.load(Ordering::Acquire) != 1 {
        return None
    }
    unsafe {
        let keyboard = core::ptr::addr_of_mut!(BOOT_KEYBOARD);
        (*keyboard).assume_init_mut().read_byte()
    }
}

impl UsbKeyboard {
    pub fn new() -> Option<Self> {
        let mut controller = unsafe { Xhci::discover()? };
        unsafe {
            controller.initialize()?;
            controller.attach_keyboard()?;
        }
        Some(Self {
            controller,
            previous: [0; 6],
            caps_lock: false,
            pending: [0; 6],
            pending_start: 0,
            pending_count: 0,
        })
    }

    pub fn read_byte(&mut self) -> Option<u8> {
        if self.pending_count != 0 {
            let byte = self.pending[self.pending_start];
            self.pending_start = (self.pending_start + 1) % self.pending.len();
            self.pending_count -= 1;
            return Some(byte);
        }

        let report = unsafe { self.controller.poll_report()? };
        let modifiers = report[0];
        for usage in report[2..8].iter().copied().filter(|usage| *usage != 0) {
            if self.previous.contains(&usage) {
                continue;
            }
            if usage == 0x39 {
                self.caps_lock = !self.caps_lock;
                continue;
            }
            if let Some(sequence) = navigation_sequence(usage, modifiers & 0x22 != 0) {
                self.queue_sequence(sequence);
                continue;
            }
            if let Some(byte) = hid_usage(usage, modifiers, self.caps_lock) {
                if self.pending_count < self.pending.len() {
                    let index = (self.pending_start + self.pending_count) % self.pending.len();
                    self.pending[index] = byte;
                    self.pending_count += 1
                }
            }
        }
        self.previous.copy_from_slice(&report[2..8]);

        if self.pending_count == 0 {
            None
        } else {
            self.read_byte()
        }
    }

    fn queue_sequence(&mut self, sequence: &[u8]) {
        for byte in sequence.iter().copied() {
            if self.pending_count == self.pending.len() {
                return;
            }
            let index = (self.pending_start + self.pending_count) % self.pending.len();
            self.pending[index] = byte;
            self.pending_count += 1
        }
    }
}

impl Xhci {
    unsafe fn discover() -> Option<Self> {
        let mut config = PortConfig;
        let mut found = None;
        enumerate(&mut config, |device| {
            if found.is_none()
                && device.class == 0x0c
                && device.subclass == 0x03
                && device.programming_interface == 0x30
            {
                found = Some(device)
            }
        });
        let device = found?;
        let mmio = match device.bars[0] {
            Bar::Memory32 { address, .. } => address as u64,
            Bar::Memory64 { address, .. } => address,
            _ => return None,
        };
        if mmio == 0 {
            return None;
        }

        let command = unsafe { config.read_u32(device.address, 0x04) };
        unsafe { config.write_u32(device.address, 0x04, command | 0x6) }

        let capability = mmio as *mut u8;
        let capability_length = unsafe { read8(capability, 0) } as usize;
        let parameters1 = unsafe { read32(capability, 0x04) };
        let parameters2 = unsafe { read32(capability, 0x08) };
        let scratchpads = (((parameters2 >> 21) & 0x1f) << 5) | ((parameters2 >> 27) & 0x1f);
        if scratchpads != 0 {
            return None;
        }
        let hcc = unsafe { read32(capability, 0x10) };
        unsafe { legacy_handoff(capability, hcc) }

        let doorbell_offset = unsafe { read32(capability, 0x14) } & !3;
        let runtime_offset = unsafe { read32(capability, 0x18) } & !0x1f;
        Some(Self {
            operational: unsafe { capability.add(capability_length) },
            runtime: unsafe { capability.add(runtime_offset as usize) },
            doorbells: unsafe { capability.add(doorbell_offset as usize) },
            context_size: if hcc & (1 << 2) != 0 { 64 } else { 32 },
            max_ports: (parameters1 >> 24) as u8,
            event_index: 0,
            event_cycle: 1,
            command: unsafe { ProducerRing::new((&raw mut COMMAND_RING.0).cast::<Trb>()) },
            ep0: unsafe { ProducerRing::new((&raw mut EP0_RING.0).cast::<Trb>()) },
            interrupt: unsafe { ProducerRing::new((&raw mut INTERRUPT_RING.0).cast::<Trb>()) },
            slot_id: 0,
            endpoint_id: 0,
            interrupt_packet_size: 0,
        })
    }

    unsafe fn initialize(&mut self) -> Option<()> {
        unsafe {
            clear_dma();

            let command = read32(self.operational, 0x00) & !1;
            write32(self.operational, 0x00, command);
            if !wait_for(self.operational, 0x04, 1, 1) {
                return None;
            }

            write32(self.operational, 0x00, command | (1 << 1));
            if !wait_for(self.operational, 0x00, 1 << 1, 0)
                || !wait_for(self.operational, 0x04, 1 << 11, 0)
            {
                return None;
            }
            if read32(self.operational, 0x08) & 1 == 0 {
                return None;
            }

            let command_address = (&raw const COMMAND_RING.0) as u64;
            write64(self.operational, 0x18, command_address | 1);
            write64(self.operational, 0x30, (&raw const DCBAA.0) as u64);
            write32(
                self.operational,
                0x38,
                read32(self.operational, 0x38) & !0xff | 1,
            );

            ERST.0[0] = (&raw const EVENT_RING.0) as u64;
            ERST.0[1] = EVENT_COUNT as u64;
            let interrupter = self.runtime.add(0x20);
            write32(interrupter, 0x08, 1);
            write64(interrupter, 0x10, (&raw const ERST.0) as u64);
            write64(interrupter, 0x18, (&raw const EVENT_RING.0) as u64);

            write32(self.operational, 0x04, u32::MAX);
            write32(self.operational, 0x00, command | 1);
            if !wait_for(self.operational, 0x04, 1, 0) {
                return None;
            }
        }
        Some(())
    }

    unsafe fn attach_keyboard(&mut self) -> Option<()> {
        for port in 0..self.max_ports {
            let register = 0x400 + port as usize * 0x10;
            let mut port_status = unsafe { read32(self.operational, register) };
            if port_status & 1 == 0 {
                continue;
            }
            if port_status & (1 << 9) == 0 {
                unsafe { write32(self.operational, register, port_status | (1 << 9)) }
            }
            unsafe { write32(self.operational, register, port_status | (1 << 4)) }
            let mut reset_done = false;
            for _ in 0..2_000_000 {
                port_status = unsafe { read32(self.operational, register) };
                if port_status & (1 << 4) == 0 && port_status & (1 << 1) != 0 {
                    reset_done = true;
                    break;
                }
                core::hint::spin_loop()
            }
            if !reset_done {
                continue;
            }

            let speed = ((port_status >> 10) & 0xf) as u8;
            if unsafe { self.configure_device(port + 1, speed) }.is_some() {
                return Some(());
            }
            unsafe { self.release_slot() }
        }
        None
    }

    unsafe fn configure_device(&mut self, root_port: u8, speed: u8) -> Option<()> {
        let event = unsafe { self.command(Trb::new(0, 0, TYPE_ENABLE_SLOT << 10))? };
        self.slot_id = (event.trb.control >> 24) as u8;
        if self.slot_id == 0 {
            return None;
        }

        unsafe {
            DCBAA.0[self.slot_id as usize] = (&raw const DEVICE_CONTEXT.0) as u64;
            core::ptr::write_bytes((&raw mut INPUT_CONTEXT.0).cast::<u8>(), 0, CONTEXT_BYTES);
        }
        let max_packet = match speed {
            3 => 64,
            4..=15 => 512,
            _ => 8,
        };
        unsafe {
            self.prepare_address_context(root_port, speed, max_packet);
            self.command(Trb::new(
                (&raw const INPUT_CONTEXT.0) as u64,
                0,
                (TYPE_ADDRESS_DEVICE << 10) | (self.slot_id as u32) << 24,
            ))?;
        }

        let descriptor_length = unsafe { self.control_in(0x80, 6, 0x0100, 0, 18)? };
        if descriptor_length < 8 {
            return None;
        }
        let actual_max_packet = unsafe { CONTROL_BUFFER.0[7] } as u16;
        if actual_max_packet != 0 && actual_max_packet != max_packet {
            unsafe {
                self.prepare_ep0_update(actual_max_packet);
                self.command(Trb::new(
                    (&raw const INPUT_CONTEXT.0) as u64,
                    0,
                    (TYPE_EVALUATE_CONTEXT << 10) | (self.slot_id as u32) << 24,
                ))?;
            }
        }

        let header_length = unsafe { self.control_in(0x80, 6, 0x0200, 0, 9)? };
        if header_length < 9 {
            return None;
        }
        let total_length =
            unsafe { u16::from_le_bytes([CONTROL_BUFFER.0[2], CONTROL_BUFFER.0[3]]) } as usize;
        let configuration_value = unsafe { CONTROL_BUFFER.0[5] };
        let fetched =
            unsafe { self.control_in(0x80, 6, 0x0200, 0, total_length.min(BUFFER_BYTES) as u16)? };
        let (interface, endpoint, packet_size, interval) =
            unsafe { find_keyboard_descriptor(fetched)? };

        unsafe {
            self.control_out(0x00, 9, configuration_value as u16, 0)?;
            self.control_out(0x21, 11, 0, interface as u16)?;
            self.configure_interrupt_endpoint(endpoint, packet_size, interval)?;
            self.queue_report()
        }
        Some(())
    }

    unsafe fn prepare_address_context(&self, root_port: u8, speed: u8, max_packet: u16) {
        unsafe {
            clear_input_context();
            context_write(0, 1, (1 << 0) | (1 << 1));
            let slot = self.context_size;
            context_write(slot, 0, (speed as u32) << 20 | 1 << 27);
            context_write(slot, 1, (root_port as u32) << 16);
            let endpoint = self.context_size * 2;
            context_write(endpoint, 1, 3 << 1 | 4 << 3 | (max_packet as u32) << 16);
            let ring = self.ep0.dequeue();
            context_write(endpoint, 2, ring as u32);
            context_write(endpoint, 3, (ring >> 32) as u32);
            context_write(endpoint, 4, 8)
        }
    }

    unsafe fn prepare_ep0_update(&self, max_packet: u16) {
        unsafe {
            clear_input_context();
            context_write(0, 1, 1 << 1);
            let endpoint = self.context_size * 2;
            context_write(endpoint, 1, 3 << 1 | 4 << 3 | (max_packet as u32) << 16);
            let ring = self.ep0.dequeue();
            context_write(endpoint, 2, ring as u32);
            context_write(endpoint, 3, (ring >> 32) as u32);
            context_write(endpoint, 4, 8)
        }
    }

    unsafe fn configure_interrupt_endpoint(
        &mut self,
        endpoint: u8,
        packet_size: u16,
        interval: u8,
    ) -> Option<()> {
        let number = endpoint & 0x0f;
        if number == 0 || endpoint & 0x80 == 0 {
            return None;
        }
        self.endpoint_id = number.saturating_mul(2).saturating_add(1);
        self.interrupt_packet_size = packet_size;
        unsafe {
            clear_input_context();
            context_write(0, 1, (1 << 0) | (1 << self.endpoint_id));
            let slot = self.context_size;
            core::ptr::copy_nonoverlapping(
                (&raw const DEVICE_CONTEXT.0).cast::<u8>(),
                (&raw mut INPUT_CONTEXT.0).cast::<u8>().add(slot),
                self.context_size,
            );
            let current_slot = context_read(slot, 0);
            context_write(
                slot,
                0,
                (current_slot & !(0x1f << 27)) | (self.endpoint_id as u32) << 27,
            );
            let context = self.context_size * (self.endpoint_id as usize + 1);
            context_write(
                context,
                0,
                (interval.saturating_sub(1).min(15) as u32) << 16,
            );
            context_write(context, 1, 3 << 1 | 7 << 3 | (packet_size as u32) << 16);
            let ring = (&raw const INTERRUPT_RING.0) as u64 | 1;
            context_write(context, 2, ring as u32);
            context_write(context, 3, (ring >> 32) as u32);
            context_write(context, 4, packet_size as u32 | (packet_size as u32) << 16);
            self.command(Trb::new(
                (&raw const INPUT_CONTEXT.0) as u64,
                0,
                (TYPE_CONFIGURE_ENDPOINT << 10) | (self.slot_id as u32) << 24,
            ))?;
        }
        Some(())
    }

    unsafe fn release_slot(&mut self) {
        if self.slot_id == 0 {
            return;
        }
        let slot = self.slot_id;
        let _ = unsafe {
            self.command(Trb::new(
                0,
                0,
                (TYPE_DISABLE_SLOT << 10) | (slot as u32) << 24,
            ))
        };
        unsafe { DCBAA.0[slot as usize] = 0 }
        self.slot_id = 0
    }

    unsafe fn control_in(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        length: u16,
    ) -> Option<usize> {
        unsafe {
            core::ptr::write_bytes((&raw mut CONTROL_BUFFER.0).cast::<u8>(), 0, BUFFER_BYTES);
            let setup = setup_packet(request_type, request, value, index, length);
            self.ep0.push(Trb::new(
                setup,
                8,
                (TYPE_SETUP_STAGE << 10) | (1 << 6) | (3 << 16),
            ));
            self.ep0.push(Trb::new(
                (&raw const CONTROL_BUFFER.0) as u64,
                length as u32,
                (TYPE_DATA_STAGE << 10) | (1 << 16),
            ));
            self.ep0
                .push(Trb::new(0, 0, (TYPE_STATUS_STAGE << 10) | (1 << 5)));
            self.ring_doorbell(self.endpoint_id_for_control());
            let event = self.wait_event(TYPE_TRANSFER_EVENT)?;
            if !completion_ok(event.trb.completion_code()) {
                return None;
            }
            Some(length as usize - (event.trb.status & 0x00ff_ffff) as usize)
        }
    }

    unsafe fn control_out(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
    ) -> Option<()> {
        unsafe {
            let setup = setup_packet(request_type, request, value, index, 0);
            self.ep0
                .push(Trb::new(setup, 8, (TYPE_SETUP_STAGE << 10) | (1 << 6)));
            self.ep0.push(Trb::new(
                0,
                0,
                (TYPE_STATUS_STAGE << 10) | (1 << 16) | (1 << 5),
            ));
            self.ring_doorbell(self.endpoint_id_for_control());
            let event = self.wait_event(TYPE_TRANSFER_EVENT)?;
            completion_ok(event.trb.completion_code()).then_some(())
        }
    }

    const fn endpoint_id_for_control(&self) -> u8 {
        1
    }

    unsafe fn command(&mut self, trb: Trb) -> Option<Event> {
        unsafe {
            self.command.push(trb);
            write32(self.doorbells, 0, 0);
            let event = self.wait_event(TYPE_COMMAND_COMPLETION)?;
            (event.trb.completion_code() == COMPLETION_SUCCESS).then_some(event)
        }
    }

    unsafe fn queue_report(&mut self) {
        unsafe {
            let length = self.interrupt_packet_size.max(8) as u32;
            self.interrupt.push(Trb::new(
                (&raw const REPORT_BUFFER.0) as u64,
                length,
                (TYPE_NORMAL << 10) | (1 << 5) | (1 << 2),
            ));
            self.ring_doorbell(self.endpoint_id)
        }
    }

    unsafe fn poll_report(&mut self) -> Option<[u8; 8]> {
        let event = unsafe { self.next_event()? };
        if event.trb.trb_type() != TYPE_TRANSFER_EVENT
            || !completion_ok(event.trb.completion_code())
            || (event.trb.control >> 24) as u8 != self.slot_id
        {
            return None;
        }
        let mut report = [0; 8];
        unsafe {
            for (index, byte) in report.iter_mut().enumerate() {
                *byte = read_volatile((&raw const REPORT_BUFFER.0[index]))
            }
            self.queue_report()
        }
        Some(report)
    }

    unsafe fn ring_doorbell(&self, endpoint_id: u8) {
        unsafe {
            write32(
                self.doorbells,
                self.slot_id as usize * 4,
                endpoint_id as u32,
            )
        }
    }

    unsafe fn wait_event(&mut self, event_type: u32) -> Option<Event> {
        for _ in 0..10_000_000 {
            if let Some(event) = unsafe { self.next_event() } {
                if event.trb.trb_type() == event_type {
                    return Some(event);
                }
            }
            core::hint::spin_loop()
        }
        None
    }

    unsafe fn next_event(&mut self) -> Option<Event> {
        let pointer = unsafe {
            (&raw const EVENT_RING.0)
                .cast::<Trb>()
                .add(self.event_index)
        };
        let trb = unsafe { read_volatile(pointer) };
        if trb.control & 1 != self.event_cycle {
            return None;
        }

        self.event_index += 1;
        if self.event_index == EVENT_COUNT {
            self.event_index = 0;
            self.event_cycle ^= 1
        }
        let dequeue = unsafe {
            (&raw const EVENT_RING.0)
                .cast::<Trb>()
                .add(self.event_index) as u64
        };
        unsafe { write64(self.runtime.add(0x20), 0x18, dequeue | (1 << 3)) }
        Some(Event { trb })
    }
}

fn setup_packet(request_type: u8, request: u8, value: u16, index: u16, length: u16) -> u64 {
    request_type as u64
        | (request as u64) << 8
        | (value as u64) << 16
        | (index as u64) << 32
        | (length as u64) << 48
}

unsafe fn find_keyboard_descriptor(length: usize) -> Option<(u8, u8, u16, u8)> {
    let mut offset = 0;
    let mut keyboard_interface = None;
    while offset + 2 <= length {
        let descriptor_length = unsafe { CONTROL_BUFFER.0[offset] } as usize;
        let descriptor_type = unsafe { CONTROL_BUFFER.0[offset + 1] };
        if descriptor_length < 2 || offset + descriptor_length > length {
            break;
        }
        if descriptor_type == 4 && descriptor_length >= 9 {
            let class = unsafe { CONTROL_BUFFER.0[offset + 5] };
            let subclass = unsafe { CONTROL_BUFFER.0[offset + 6] };
            let protocol = unsafe { CONTROL_BUFFER.0[offset + 7] };
            keyboard_interface = if class == 3 && subclass == 1 && protocol == 1 {
                Some(unsafe { CONTROL_BUFFER.0[offset + 2] })
            } else {
                None
            }
        } else if descriptor_type == 5 && descriptor_length >= 7 && keyboard_interface.is_some() {
            let endpoint = unsafe { CONTROL_BUFFER.0[offset + 2] };
            let attributes = unsafe { CONTROL_BUFFER.0[offset + 3] };
            if endpoint & 0x80 != 0 && attributes & 3 == 3 {
                let packet_size = unsafe {
                    u16::from_le_bytes([CONTROL_BUFFER.0[offset + 4], CONTROL_BUFFER.0[offset + 5]])
                } & 0x7ff;
                let interval = unsafe { CONTROL_BUFFER.0[offset + 6] };
                return Some((keyboard_interface?, endpoint, packet_size, interval));
            }
        }
        offset += descriptor_length
    }
    None
}

fn hid_usage(usage: u8, modifiers: u8, caps_lock: bool) -> Option<u8> {
    let shifted = modifiers & 0x22 != 0;
    let controlled = modifiers & 0x11 != 0;
    if (0x04..=0x1d).contains(&usage) {
        let letter = b'a' + usage - 0x04;
        if controlled {
            return Some(letter & 0x1f);
        }
        return Some(if shifted ^ caps_lock {
            letter.to_ascii_uppercase()
        } else {
            letter
        });
    }

    match usage {
        0x1e..=0x27 => {
            let plain = b"1234567890"[(usage - 0x1e) as usize];
            let upper = b"!@#$%^&*()"[(usage - 0x1e) as usize];
            Some(if shifted { upper } else { plain })
        }
        0x28 => Some(b'\r'),
        0x29 => Some(3),
        0x2a => Some(8),
        0x2b => Some(b'\t'),
        0x2c => Some(b' '),
        0x2d => Some(if shifted { b'_' } else { b'-' }),
        0x2e => Some(if shifted { b'+' } else { b'=' }),
        0x2f => Some(if shifted { b'{' } else { b'[' }),
        0x30 => Some(if shifted { b'}' } else { b']' }),
        0x31 => Some(if shifted { b'|' } else { b'\\' }),
        0x33 => Some(if shifted { b':' } else { b';' }),
        0x34 => Some(if shifted { b'"' } else { b'\'' }),
        0x35 => Some(if shifted { b'~' } else { b'`' }),
        0x36 => Some(if shifted { b'<' } else { b',' }),
        0x37 => Some(if shifted { b'>' } else { b'.' }),
        0x38 => Some(if shifted { b'?' } else { b'/' }),
        0x4c => Some(127),
        _ => None,
    }
}

fn navigation_sequence(usage: u8, shifted: bool) -> Option<&'static [u8]> {
    match usage {
        0x4a => Some(b"\x1b[H"),
        0x4c => Some(b"\x1b[3~"),
        0x4d => Some(b"\x1b[F"),
        0x4f => Some(if shifted { b"\x1b[1;2C" } else { b"\x1b[C" }),
        0x50 => Some(if shifted { b"\x1b[1;2D" } else { b"\x1b[D" }),
        0x51 => Some(if shifted { b"\x1b[1;2B" } else { b"\x1b[B" }),
        0x52 => Some(if shifted { b"\x1b[1;2A" } else { b"\x1b[A" }),
        _ => None,
    }
}

fn completion_ok(code: u8) -> bool {
    matches!(code, COMPLETION_SUCCESS | COMPLETION_SHORT_PACKET)
}

unsafe fn clear_dma() {
    unsafe {
        core::ptr::write_bytes((&raw mut COMMAND_RING.0).cast::<u8>(), 0, 4096);
        core::ptr::write_bytes((&raw mut EVENT_RING.0).cast::<u8>(), 0, 4096);
        core::ptr::write_bytes((&raw mut EP0_RING.0).cast::<u8>(), 0, 4096);
        core::ptr::write_bytes((&raw mut INTERRUPT_RING.0).cast::<u8>(), 0, 4096);
        core::ptr::write_bytes((&raw mut DCBAA.0).cast::<u8>(), 0, 4096);
        core::ptr::write_bytes((&raw mut ERST.0).cast::<u8>(), 0, 4096);
        core::ptr::write_bytes((&raw mut DEVICE_CONTEXT.0).cast::<u8>(), 0, 4096);
        clear_input_context();
        core::ptr::write_bytes((&raw mut CONTROL_BUFFER.0).cast::<u8>(), 0, 4096);
        core::ptr::write_bytes((&raw mut REPORT_BUFFER.0).cast::<u8>(), 0, 4096)
    }
}

unsafe fn clear_input_context() {
    unsafe { core::ptr::write_bytes((&raw mut INPUT_CONTEXT.0).cast::<u8>(), 0, CONTEXT_BYTES) }
}

unsafe fn context_write(context: usize, dword: usize, value: u32) {
    unsafe {
        write_volatile(
            (&raw mut INPUT_CONTEXT.0)
                .cast::<u8>()
                .add(context + dword * 4)
                .cast::<u32>(),
            value,
        )
    }
}

unsafe fn context_read(context: usize, dword: usize) -> u32 {
    unsafe {
        read_volatile(
            (&raw const INPUT_CONTEXT.0)
                .cast::<u8>()
                .add(context + dword * 4)
                .cast::<u32>(),
        )
    }
}

unsafe fn legacy_handoff(capability: *mut u8, hcc: u32) {
    let mut offset = ((hcc >> 16) * 4) as usize;
    for _ in 0..64 {
        if offset == 0 {
            return;
        }
        let header = unsafe { read32(capability, offset) };
        if header & 0xff == 1 {
            unsafe { write32(capability, offset, header | (1 << 24)) }
            for _ in 0..1_000_000 {
                if unsafe { read32(capability, offset) } & (1 << 16) == 0 {
                    return;
                }
                core::hint::spin_loop()
            }
            return;
        }
        let next = ((header >> 8) & 0xff) as usize;
        if next == 0 {
            return;
        }
        offset += next * 4
    }
}

unsafe fn wait_for(base: *mut u8, offset: usize, mask: u32, expected: u32) -> bool {
    for _ in 0..10_000_000 {
        if unsafe { read32(base, offset) } & mask == expected {
            return true;
        }
        core::hint::spin_loop()
    }
    false
}

unsafe fn read8(base: *mut u8, offset: usize) -> u8 {
    unsafe { read_volatile(base.add(offset).cast::<u8>()) }
}

unsafe fn read32(base: *mut u8, offset: usize) -> u32 {
    unsafe { read_volatile(base.add(offset).cast::<u32>()) }
}

unsafe fn write32(base: *mut u8, offset: usize, value: u32) {
    unsafe { write_volatile(base.add(offset).cast::<u32>(), value) }
}

unsafe fn write64(base: *mut u8, offset: usize, value: u64) {
    unsafe { write_volatile(base.add(offset).cast::<u64>(), value) }
}
