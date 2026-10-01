use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU8, Ordering};
use ghostos_legacy_pc_drivers::pci::{Bar, ConfigAccess, PortConfig, enumerate};

#[repr(C)]
struct CKeyboardState {
    previous: [u8; 6],
    caps_lock: u8,
    pending: [u8; 6],
    pending_start: usize,
    pending_count: usize,
}

unsafe extern "C" {
    fn ghostos_usb_keyboard_init(state: *mut CKeyboardState);
    fn ghostos_usb_keyboard_process_report(state: *mut CKeyboardState, report: *const u8);
    fn ghostos_usb_keyboard_take_byte(state: *mut CKeyboardState, byte: *mut u8) -> bool;
    fn ghostos_usb_keyboard_controller_init(mmio: u64) -> bool;
    fn ghostos_usb_keyboard_controller_poll(report: *mut u8) -> bool;
}

pub struct UsbKeyboard {
    state: CKeyboardState,
}

static mut BOOT_KEYBOARD: MaybeUninit<UsbKeyboard> = MaybeUninit::uninit();
static BOOT_KEYBOARD_STATE: AtomicU8 = AtomicU8::new(0);

pub fn read_boot_byte() -> Option<u8> {
    if BOOT_KEYBOARD_STATE.load(Ordering::Acquire) == 0 {
        let Some(keyboard) = UsbKeyboard::new() else {
            BOOT_KEYBOARD_STATE.store(2, Ordering::Release);
            return None
        };
        unsafe { (&raw mut BOOT_KEYBOARD).write(MaybeUninit::new(keyboard)) }
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
        let mmio = unsafe { discover_controller_mmio()? };
        if !unsafe { ghostos_usb_keyboard_controller_init(mmio) } {
            return None;
        }
        let mut state = CKeyboardState {
            previous: [0; 6],
            caps_lock: 0,
            pending: [0; 6],
            pending_start: 0,
            pending_count: 0,
        };
        unsafe { ghostos_usb_keyboard_init(&mut state) };
        Some(Self { state })
    }

    pub fn read_byte(&mut self) -> Option<u8> {
        let mut byte = 0;
        if unsafe { ghostos_usb_keyboard_take_byte(&mut self.state, &mut byte) } {
            return Some(byte)
        }

        let mut report = [0; 8];
        if !unsafe { ghostos_usb_keyboard_controller_poll(report.as_mut_ptr()) } {
            return None;
        }
        unsafe { ghostos_usb_keyboard_process_report(&mut self.state, report.as_ptr()) };
        if unsafe { ghostos_usb_keyboard_take_byte(&mut self.state, &mut byte) } {
            Some(byte)
        } else {
            None
        }
    }
}

unsafe fn discover_controller_mmio() -> Option<u64> {
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
    unsafe { config.write_u32(device.address, 0x04, command | 0x6) };
    Some(mmio)
}
