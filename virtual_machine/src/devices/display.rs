//! Display subsystem: VGA text mode, VESA linear framebuffer (LFB), a
//! host-side pixel renderer, and UEFI GOP data structures.

use crate::cpu::CpuState;
use crate::devices::{Device, DeviceError, PortDevice};
use crate::memory::Mmu;
use std::cell::RefCell;
use std::rc::Rc;

pub const VGA_TEXT_BASE: u64 = 0xB8000;
pub const VGA_TEXT_SIZE: usize = 0x8000;
pub const VGA_COLS: usize = 80;
pub const VGA_ROWS: usize = 25;
pub const VGA_CELLS: usize = VGA_COLS * VGA_ROWS;
pub const VESA_LFB_BASE: u64 = 0xF000_0000;
pub const VESA_FB_SIZE: usize = 16 * 1024 * 1024;
pub const VGA_PORT_BASE: u16 = 0x3C0;
pub const VGA_PORT_COUNT: u16 = 0x1B;
pub const VBE_MODE_COUNT: usize = 12;

pub const VGA_DAC_READ_INDEX: u16 = 0x3C7;
pub const VGA_DAC_WRITE_INDEX: u16 = 0x3C8;
pub const VGA_DAC_DATA: u16 = 0x3C9;
pub const VGA_CRTC_INDEX: u16 = 0x3D4;
pub const VGA_CRTC_DATA: u16 = 0x3D5;
pub const VGA_INPUT_STATUS: u16 = 0x3DA;

pub const VBE_MODES: [u16; VBE_MODE_COUNT] = [
    0x101, 0x103, 0x105,
    0x110, 0x111, 0x112,
    0x113, 0x114, 0x115,
    0x116, 0x117, 0x118,
];

#[repr(C)]
#[derive(Default)]
struct CDisplayInfo {
    width: u32,
    height: u32,
    pixel_width: u32,
    pixel_height: u32,
    pitch: u32,
    vesa_mode: u16,
    mode: u8,
    current_mode: u8,
    bpp: u8,
    cursor_x: u8,
    cursor_y: u8,
    cursor_visible: bool,
    current_lfb: bool,
    dirty: bool,
}

#[repr(C)]
struct CVideoRegisters {
    rax: u64, rbx: u64, rcx: u64, rdx: u64, rsi: u64, rdi: u64, rflags: u64,
    ds: u16, es: u16,
}

#[repr(C)]
struct CVideoIo {
    read: Option<unsafe extern "C" fn(*mut std::ffi::c_void, u64, *mut u8, usize) -> bool>,
    write: Option<unsafe extern "C" fn(*mut std::ffi::c_void, u64, *const u8, usize) -> bool>,
    context: *mut std::ffi::c_void,
}

type CDisplay = std::ffi::c_void;
const _: () = assert!(std::mem::size_of::<CDisplayInfo>() == 32);
const _: () = assert!(std::mem::size_of::<CVideoRegisters>() == 64);

unsafe extern "C" {
    fn ghostos_vm_display_new() -> *mut CDisplay;
    fn ghostos_vm_display_free(display: *mut CDisplay);
    fn ghostos_vm_display_reset(display: *mut CDisplay);
    fn ghostos_vm_display_info_get(display: *const CDisplay, info: *mut CDisplayInfo);
    fn ghostos_vm_display_cell(display: *const CDisplay, row: usize, col: usize, character: *mut u8, attribute: *mut u8) -> bool;
    fn ghostos_vm_display_pixels(display: *const CDisplay, count: *mut usize) -> *const u32;
    fn ghostos_vm_display_read(display: *const CDisplay, framebuffer: bool, address: u64, size: u8, value: *mut u64) -> u32;
    fn ghostos_vm_display_write(display: *mut CDisplay, framebuffer: bool, address: u64, size: u8, value: u64) -> u32;
    fn ghostos_vm_display_port_read(display: *mut CDisplay, port: u16) -> u8;
    fn ghostos_vm_display_port_write(display: *mut CDisplay, port: u16, value: u8);
    fn ghostos_vm_display_render(display: *mut CDisplay) -> u32;
    fn ghostos_vm_display_ppm(display: *mut CDisplay, output: *mut *mut u8, length: *mut usize) -> u32;
    fn ghostos_vm_display_bytes_free(bytes: *mut u8);
    fn ghostos_vm_display_text_snapshot(display: *const CDisplay, output: *mut u8, capacity: usize, length: *mut usize) -> bool;
    fn ghostos_vm_display_int10(display: *mut CDisplay, registers: *mut CVideoRegisters, io: *const CVideoIo) -> u32;
}

fn display_result(code: u32) -> Result<(), DeviceError> {
    match code {
        0 => Ok(()),
        1 => Err(DeviceError::UnsupportedSize),
        _ => Err(DeviceError::InvalidAddress),
    }
}

unsafe extern "C" fn video_write(context: *mut std::ffi::c_void, address: u64, bytes: *const u8, length: usize) -> bool {
    let mmu = unsafe { &mut *context.cast::<Mmu>() };
    let bytes = unsafe { std::slice::from_raw_parts(bytes, length) };
    mmu.write_phys(address, bytes).is_ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoMode {
    Text,
    Vesa,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum GopPixelFormat {
    BgrxRgb8 = 0,
    BgraRgb8 = 1,
    XbgrRgb8 = 2,
    XrgbRgb8 = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GopMode {
    pub width: u32,
    pub height: u32,
    pub pixel_format: GopPixelFormat,
    pub pixels_per_scanline: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UefiGop {
    pub framebuffer_base: u64,
    pub framebuffer_size: u64,
    pub version: u32,
    pub modes: Vec<GopMode>,
    pub current_mode: usize,
}

/// All guest-visible display state, shared between the text MMIO device, the
/// VESA LFB MMIO device, the port-mapped VGA controller, and the BIOS.
pub struct DisplayState {
    display: std::ptr::NonNull<CDisplay>,
}

// C owns each allocation independently. Mutation requires exclusive access.
unsafe impl Send for DisplayState {}
unsafe impl Sync for DisplayState {}

impl Drop for DisplayState {
    fn drop(&mut self) {
        unsafe { ghostos_vm_display_free(self.display.as_ptr()) }
    }
}

impl Default for DisplayState {
    fn default() -> Self {
        Self::new()
    }
}

impl DisplayState {
    pub fn new() -> Self {
        let display = unsafe { ghostos_vm_display_new() };
        Self { display: std::ptr::NonNull::new(display).expect("C display allocation") }
    }

    fn info(&self) -> CDisplayInfo {
        let mut info = CDisplayInfo::default();
        unsafe { ghostos_vm_display_info_get(self.display.as_ptr(), &mut info) };
        info
    }

    pub fn reset(&mut self) {
        unsafe { ghostos_vm_display_reset(self.display.as_ptr()) }
    }

    pub fn mode(&self) -> VideoMode {
        if self.info().mode == 0 { VideoMode::Text } else { VideoMode::Vesa }
    }

    pub fn current_mode(&self) -> u8 { self.info().current_mode }
    pub fn vesa_mode(&self) -> u16 { self.info().vesa_mode }

    pub fn resolution(&self) -> (u32, u32) {
        let info = self.info();
        if info.mode == 0 { (VGA_COLS as u32, VGA_ROWS as u32) } else { (info.width, info.height) }
    }

    pub fn bpp(&self) -> u8 { self.info().bpp }

    pub fn cursor(&self) -> (u8, u8) {
        let info = self.info();
        (info.cursor_x, info.cursor_y)
    }

    pub fn pixels(&self) -> &[u32] {
        let mut count = 0;
        let pixels = unsafe { ghostos_vm_display_pixels(self.display.as_ptr(), &mut count) };
        if count == 0 { &[] } else { unsafe { std::slice::from_raw_parts(pixels, count) } }
    }

    pub fn text_char(&self, row: usize, col: usize) -> Option<u8> {
        let mut character = 0;
        unsafe { ghostos_vm_display_cell(self.display.as_ptr(), row, col, &mut character, std::ptr::null_mut()) }
            .then_some(character)
    }

    pub fn text_attr(&self, row: usize, col: usize) -> Option<u8> {
        let mut attribute = 0;
        unsafe { ghostos_vm_display_cell(self.display.as_ptr(), row, col, std::ptr::null_mut(), &mut attribute) }
            .then_some(attribute)
    }

    pub fn gop(&self) -> UefiGop {
        let info = self.info();
        let width = info.width.max(1);
        let height = info.height.max(1);
        UefiGop {
            framebuffer_base: VESA_LFB_BASE,
            framebuffer_size: u64::from(width * (u32::from(info.bpp) / 8)) * u64::from(height),
            version: 0x0001_0000,
            modes: vec![GopMode {
                width, height, pixel_format: GopPixelFormat::BgrxRgb8, pixels_per_scanline: width,
            }],
            current_mode: 0,
        }
    }

    fn memory_read(&self, framebuffer: bool, address: u64, size: u8) -> Result<u64, DeviceError> {
        let mut value = 0;
        display_result(unsafe { ghostos_vm_display_read(self.display.as_ptr(), framebuffer, address, size, &mut value) })?;
        Ok(value)
    }

    fn text_read(&self, address: u64, size: u8) -> Result<u64, DeviceError> {
        self.memory_read(false, address, size)
    }

    fn text_write(&mut self, address: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        display_result(unsafe { ghostos_vm_display_write(self.display.as_ptr(), false, address, size, value) })
    }

    fn fb_read(&self, address: u64, size: u8) -> Result<u64, DeviceError> {
        self.memory_read(true, address, size)
    }

    fn fb_write(&mut self, address: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        display_result(unsafe { ghostos_vm_display_write(self.display.as_ptr(), true, address, size, value) })
    }

    pub fn port_read(&mut self, port: u16) -> u8 {
        // Preserve the original debug-build subtraction check for ports below 0x3C0.
        let _ = port - VGA_PORT_BASE;
        unsafe { ghostos_vm_display_port_read(self.display.as_ptr(), port) }
    }

    pub fn port_write(&mut self, port: u16, value: u8) {
        let _ = port - VGA_PORT_BASE;
        unsafe { ghostos_vm_display_port_write(self.display.as_ptr(), port, value) }
    }

    pub fn pixel_width(&self) -> usize { self.info().pixel_width as usize }
    pub fn pixel_height(&self) -> usize { self.info().pixel_height as usize }
    pub fn is_dirty(&self) -> bool { self.info().dirty }

    pub fn render_frame(&mut self) {
        assert_eq!(unsafe { ghostos_vm_display_render(self.display.as_ptr()) }, 0, "C display render allocation");
    }

    pub fn text_snapshot(&self) -> String {
        let mut length = 0;
        assert!(unsafe { ghostos_vm_display_text_snapshot(self.display.as_ptr(), std::ptr::null_mut(), 0, &mut length) });
        let mut bytes = vec![0; length];
        assert!(unsafe { ghostos_vm_display_text_snapshot(self.display.as_ptr(), bytes.as_mut_ptr(), bytes.len(), &mut length) });
        String::from_utf8(bytes).expect("C display snapshot UTF-8")
    }

    pub fn save_ppm(&mut self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let mut bytes = std::ptr::null_mut();
        let mut length = 0;
        assert_eq!(unsafe { ghostos_vm_display_ppm(self.display.as_ptr(), &mut bytes, &mut length) }, 0,
            "C display PPM allocation");
        let result = std::fs::write(path, unsafe { std::slice::from_raw_parts(bytes, length) });
        unsafe { ghostos_vm_display_bytes_free(bytes) };
        result
    }

    pub fn int10(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let mut registers = CVideoRegisters {
            rax: cpu.rax, rbx: cpu.rbx, rcx: cpu.rcx, rdx: cpu.rdx, rsi: cpu.rsi,
            rdi: cpu.rdi, rflags: cpu.rflags, ds: cpu.ds.selector, es: cpu.es.selector,
        };
        let io = CVideoIo { read: None, write: Some(video_write), context: (mmu as *mut Mmu).cast() };
        let code = unsafe { ghostos_vm_display_int10(self.display.as_ptr(), &mut registers, &io) };
        match code {
            0 => {},
            4 => panic!("display scroll rectangle index out of bounds"),
            5 => panic!("copy_from_slice: source slice length (4) does not match destination slice length (2)"),
            _ => panic!("C display BIOS service failed"),
        }
        cpu.rax = registers.rax;
        cpu.rbx = registers.rbx;
    }
}

// ---------------------------------------------------------------------------
// MMIO/port device wrappers (shared state through Rc<RefCell<>>)
// ---------------------------------------------------------------------------

pub struct VgaTextDevice(Rc<RefCell<DisplayState>>);

impl VgaTextDevice {
    pub fn new(state: Rc<RefCell<DisplayState>>) -> Self {
        Self(state)
    }
}

impl Device for VgaTextDevice {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        self.0.borrow().text_read(addr, size)
    }
    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        self.0.borrow_mut().text_write(addr, value, size)
    }
    fn reset(&mut self) {
        self.0.borrow_mut().reset();
    }
}

pub struct VesaFbDevice(Rc<RefCell<DisplayState>>);

impl VesaFbDevice {
    pub fn new(state: Rc<RefCell<DisplayState>>) -> Self {
        Self(state)
    }
}

impl Device for VesaFbDevice {
    fn read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        self.0.borrow().fb_read(addr, size)
    }
    fn write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        self.0.borrow_mut().fb_write(addr, value, size)
    }
    fn reset(&mut self) {
        self.0.borrow_mut().reset();
    }
}

pub struct VgaPorts(Rc<RefCell<DisplayState>>);

impl VgaPorts {
    pub fn new(state: Rc<RefCell<DisplayState>>) -> Self {
        Self(state)
    }
}

impl PortDevice for VgaPorts {
    fn read(&mut self, port: u16, size: u8) -> Result<u64, DeviceError> {
        if size != 1 {
            return Err(DeviceError::UnsupportedSize);
        }
        Ok(self.0.borrow_mut().port_read(port) as u64)
    }
    fn write(&mut self, port: u16, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 1 {
            return Err(DeviceError::UnsupportedSize);
        }
        self.0.borrow_mut().port_write(port, value as u8);
        Ok(())
    }
    fn reset(&mut self) {
        self.0.borrow_mut().reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn text_and_framebuffer_mmio_round_trip_and_bounds() {
        let state = Rc::new(RefCell::new(DisplayState::new()));
        let mut text = VgaTextDevice::new(state.clone());
        let mut fb = VesaFbDevice::new(state.clone());

        text.write(VGA_TEXT_BASE, 0x0741, 2).unwrap();
        assert_eq!(text.read(VGA_TEXT_BASE, 2).unwrap(), 0x0741);
        assert_eq!(state.borrow().text_char(0, 0), Some(b'A'));
        assert_eq!(state.borrow().text_attr(0, 0), Some(0x07));

        fb.write(VESA_LFB_BASE + 3, 0xAABB_CCDD, 4).unwrap();
        assert_eq!(fb.read(VESA_LFB_BASE + 3, 4).unwrap(), 0xAABB_CCDD);
        assert_eq!(fb.read(VESA_LFB_BASE + VESA_FB_SIZE as u64 - 1, 2), Err(DeviceError::InvalidAddress));
        assert_eq!(text.write(VGA_TEXT_BASE, 0, 3), Err(DeviceError::UnsupportedSize));
    }

    #[test]
    fn vga_ports_control_cursor_and_palette() {
        let state = Rc::new(RefCell::new(DisplayState::new()));
        let mut ports = VgaPorts::new(state.clone());

        ports.write(VGA_CRTC_INDEX, 0x0F, 1).unwrap();
        ports.write(VGA_CRTC_DATA, 81, 1).unwrap();
        ports.write(VGA_CRTC_INDEX, 0x0E, 1).unwrap();
        ports.write(VGA_CRTC_DATA, 0, 1).unwrap();
        assert_eq!(state.borrow().cursor(), (1, 1));

        ports.write(VGA_DAC_WRITE_INDEX, 2, 1).unwrap();
        ports.write(VGA_DAC_DATA, 1, 1).unwrap();
        ports.write(VGA_DAC_DATA, 2, 1).unwrap();
        ports.write(VGA_DAC_DATA, 3, 1).unwrap();
        ports.write(VGA_DAC_READ_INDEX, 2, 1).unwrap();
        assert_eq!(ports.read(VGA_DAC_DATA, 1).unwrap(), 1);
        assert_eq!(ports.read(VGA_DAC_DATA, 1).unwrap(), 2);
        assert_eq!(ports.read(VGA_DAC_DATA, 1).unwrap(), 3);

        ports.reset();
        assert_eq!(state.borrow().mode(), VideoMode::Text);
        assert_eq!(state.borrow().cursor(), (0, 0));
    }
}
