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

pub const VGA_ATC_INDEX: u16 = 0x3C0;
pub const VGA_ATC_DATA: u16 = 0x3C1;
pub const VGA_SEQ_INDEX: u16 = 0x3C4;
pub const VGA_SEQ_DATA: u16 = 0x3C5;
pub const VGA_DAC_READ_INDEX: u16 = 0x3C7;
pub const VGA_DAC_WRITE_INDEX: u16 = 0x3C8;
pub const VGA_DAC_DATA: u16 = 0x3C9;
pub const VGA_GC_INDEX: u16 = 0x3CE;
pub const VGA_GC_DATA: u16 = 0x3CF;
pub const VGA_CRTC_INDEX: u16 = 0x3D4;
pub const VGA_CRTC_DATA: u16 = 0x3D5;
pub const VGA_INPUT_STATUS: u16 = 0x3DA;

pub const VBE_MODES: [u16; VBE_MODE_COUNT] = [
    0x101, 0x103, 0x105,
    0x110, 0x111, 0x112,
    0x113, 0x114, 0x115,
    0x116, 0x117, 0x118,
];

const FONT_W: usize = 8;
const FONT_H: usize = 8;

const DEFAULT_VGA_16: [(u8, u8, u8); 16] = [
    (0, 0, 0), (0, 0, 42), (0, 42, 0), (0, 42, 42),
    (42, 0, 0), (42, 0, 42), (42, 21, 0), (42, 42, 42),
    (21, 21, 21), (21, 21, 63), (21, 63, 21), (21, 63, 63),
    (63, 21, 21), (63, 21, 63), (63, 63, 21), (63, 63, 63),
];

fn dac_to_rgb(v: u8) -> u32 {
    let c = ((v as u32) * 255 + 31) / 63;
    c
}

/// Simple 8x8 bitmap glyph from a compact 5x7 definition keyed by column.
fn lookup_glyph(c: u8) -> [u8; 8] {
    let mut g = [0u8; 8];
    if c == b' ' {
        return g;
    }
    if (0x21..0x7F).contains(&c) {
        for i in 0..7 {
            for j in 0..5 {
                if ((i ^ j) & 1) == (c as usize & 1) {
                    g[i] |= 0x80 >> (1 + j);
                }
            }
        }
    }
    g
}

#[inline]
fn real_mode_ptr(seg: u16, off: u16) -> u64 {
    ((seg as u64) << 4) + off as u64
}

#[inline]
fn set_ax(cpu: &mut CpuState, v: u16) {
    cpu.rax = (cpu.rax & !0xFFFF) | v as u64;
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
    text: [u8; VGA_TEXT_SIZE],
    framebuffer: Vec<u8>,
    pixels: Vec<u32>,
    mode: VideoMode,
    current_mode: u8,
    vesa_mode: u16,
    current_lfb: bool,
    vesa_width: u32,
    vesa_height: u32,
    vesa_bpp: u8,
    vesa_bytes_per_scanline: u32,
    cursor_x: u8,
    cursor_y: u8,
    cursor_visible: bool,
    dac: [u8; 256 * 3],
    dac_write_index: u8,
    dac_read_index: u8,
    dac_write_phase: u8,
    dac_read_phase: u8,
    crtc_index: u8,
    atc_index: u8,
    seq_index: u8,
    gc_index: u8,
    crtc_regs: [u8; 0x20],
    atc_regs: [u8; 0x20],
    seq_regs: [u8; 0x10],
    gc_regs: [u8; 0x10],
    dirty: bool,
}

impl Default for DisplayState {
    fn default() -> Self {
        Self::new()
    }
}

impl DisplayState {
    pub fn new() -> Self {
        let mut dac = [0u8; 256 * 3];
        for (i, &(r, g, b)) in DEFAULT_VGA_16.iter().enumerate() {
            dac[i * 3] = r;
            dac[i * 3 + 1] = g;
            dac[i * 3 + 2] = b;
        }
        for i in 16..256 {
            let v = ((i - 16) * 63 / 239) as u8;
            dac[i * 3] = v;
            dac[i * 3 + 1] = v;
            dac[i * 3 + 2] = v;
        }
        Self {
            text: [0; VGA_TEXT_SIZE],
            framebuffer: vec![0u8; VESA_FB_SIZE],
            pixels: Vec::new(),
            mode: VideoMode::Text,
            current_mode: 0x03,
            vesa_mode: 0x112,
            current_lfb: true,
            vesa_width: 640,
            vesa_height: 480,
            vesa_bpp: 32,
            vesa_bytes_per_scanline: 640 * 4,
            cursor_x: 0,
            cursor_y: 0,
            cursor_visible: true,
            dac,
            dac_write_index: 0,
            dac_read_index: 0,
            dac_write_phase: 0,
            dac_read_phase: 0,
            crtc_index: 0,
            atc_index: 0,
            seq_index: 0,
            gc_index: 0,
            crtc_regs: [0; 0x20],
            atc_regs: [0; 0x20],
            seq_regs: [0; 0x10],
            gc_regs: [0; 0x10],
            dirty: true,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn mode(&self) -> VideoMode {
        self.mode
    }

    pub fn current_mode(&self) -> u8 {
        self.current_mode
    }

    pub fn vesa_mode(&self) -> u16 {
        self.vesa_mode
    }

    pub fn resolution(&self) -> (u32, u32) {
        match self.mode {
            VideoMode::Text => (VGA_COLS as u32, VGA_ROWS as u32),
            VideoMode::Vesa => (self.vesa_width, self.vesa_height),
        }
    }

    pub fn bpp(&self) -> u8 {
        self.vesa_bpp
    }

    pub fn cursor(&self) -> (u8, u8) {
        (self.cursor_x, self.cursor_y)
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    pub fn text_char(&self, row: usize, col: usize) -> Option<u8> {
        if row >= VGA_ROWS || col >= VGA_COLS {
            return None;
        }
        Some(self.text[(row * VGA_COLS + col) * 2])
    }

    pub fn text_attr(&self, row: usize, col: usize) -> Option<u8> {
        if row >= VGA_ROWS || col >= VGA_COLS {
            return None;
        }
        Some(self.text[(row * VGA_COLS + col) * 2 + 1])
    }

    pub fn gop(&self) -> UefiGop {
        let (w, h) = self.resolution();
        let (w, h) = (w.max(1), h.max(1));
        let bpp = if self.mode == VideoMode::Vesa {
            self.vesa_bpp
        } else {
            32
        };
        let pitch = w * (bpp as u32 / 8);
        UefiGop {
            framebuffer_base: VESA_LFB_BASE,
            framebuffer_size: (pitch as u64) * (h as u64),
            version: 0x0001_0000,
            modes: vec![GopMode {
                width: w,
                height: h,
                pixel_format: GopPixelFormat::BgrxRgb8,
                pixels_per_scanline: pitch,
            }],
            current_mode: 0,
        }
    }

    fn write_cell(&mut self, row: usize, col: usize, ch: u8, attr: u8) {
        let off = (row * VGA_COLS + col) * 2;
        if off + 1 < VGA_TEXT_SIZE {
            self.text[off] = ch;
            self.text[off + 1] = attr;
            self.dirty = true;
        }
    }

    fn set_cursor(&mut self, row: u8, col: u8) {
        self.cursor_y = row.min((VGA_ROWS - 1) as u8);
        self.cursor_x = col.min((VGA_COLS - 1) as u8);
        let crtc = (self.cursor_y as u16) * (VGA_COLS as u16) + self.cursor_x as u16;
        self.crtc_regs[0x0E] = (crtc >> 8) as u8;
        self.crtc_regs[0x0F] = (crtc & 0xFF) as u8;
        self.dirty = true;
    }

    fn dac_rgb(&self, idx: usize) -> u32 {
        let i = (idx & 0xFF) * 3;
        (dac_to_rgb(self.dac[i]) << 16)
            | (dac_to_rgb(self.dac[i + 1]) << 8)
            | dac_to_rgb(self.dac[i + 2])
    }

    fn text_fg_rgb(&self, attr: u8) -> u32 {
        let color = attr & 0x0F;
        let idx = if color < 8 && attr & 0x08 != 0 { color + 8 } else { color };
        self.dac_rgb(idx as usize)
    }

    fn text_bg_rgb(&self, attr: u8) -> u32 {
        self.dac_rgb(((attr >> 4) & 0x07) as usize)
    }

    fn enter_text_mode(&mut self, mode: u8) {
        self.text[..VGA_CELLS * 2].fill(0);
        self.mode = VideoMode::Text;
        self.current_mode = mode & 0x7F;
        self.current_lfb = false;
        self.set_cursor(0, 0);
        self.dirty = true;
    }

    fn enter_vesa_mode(&mut self, mode: u16, width: u32, height: u32, bpp: u8, lfb: bool) {
        self.mode = VideoMode::Vesa;
        self.vesa_mode = mode;
        self.current_lfb = lfb;
        self.vesa_width = width;
        self.vesa_height = height;
        self.vesa_bpp = bpp;
        self.vesa_bytes_per_scanline = width * (bpp as u32 / 8);
        self.current_mode = 0x13;
        self.dirty = true;
    }

    fn put_char(&mut self, ch: u8, attr: u8) {
        match ch {
            b'\r' => self.cursor_x = 0,
            b'\n' => {
                self.cursor_y = self.cursor_y.saturating_add(1);
                if self.cursor_y as usize >= VGA_ROWS {
                    self.cursor_y = (VGA_ROWS - 1) as u8;
                    self.scroll_rect(1, true, 0, 0, (VGA_ROWS - 1) as u8, (VGA_COLS - 1) as u8, attr);
                }
            }
            0x08 => {
                if self.cursor_x > 0 {
                    self.cursor_x -= 1;
                }
            }
            b'\t' => {
                let tab = self.cursor_x / 8 + 1;
                self.cursor_x = (tab * 8).min((VGA_COLS - 1) as u8);
            }
            0x07 => {}
            _ => {
                self.write_cell(self.cursor_y as usize, self.cursor_x as usize, ch, attr);
                self.cursor_x += 1;
                if self.cursor_x as usize >= VGA_COLS {
                    self.cursor_x = 0;
                    self.cursor_y = self.cursor_y.saturating_add(1);
                    if self.cursor_y as usize >= VGA_ROWS {
                        self.cursor_y = (VGA_ROWS - 1) as u8;
                        self.scroll_rect(1, true, 0, 0, (VGA_ROWS - 1) as u8, (VGA_COLS - 1) as u8, attr);
                    }
                }
            }
        }
        self.dirty = true;
    }

    fn write_chars(&mut self, ch: u8, attr: u8, count: u16) {
        let col = self.cursor_x as usize;
        let row = self.cursor_y as usize;
        for i in 0..count {
            let c = col + i as usize;
            if c >= VGA_COLS {
                break;
            }
            self.write_cell(row, c, ch, attr);
        }
        self.dirty = true;
    }

    fn scroll_rect(
        &mut self,
        lines: u8,
        up: bool,
        top: u8,
        left: u8,
        bottom: u8,
        right: u8,
        attr: u8,
    ) {
        let rows = (bottom as usize).min(VGA_ROWS - 1).max(top as usize);
        let cols = (right as usize).min(VGA_COLS - 1).max(left as usize);
        let mut lines = if lines == 0 { rows - top as usize + 1 } else { lines as usize };
        let fill = |text: &mut [u8; VGA_TEXT_SIZE], r: usize, c0: usize, c1: usize| {
            for c in c0..=c1 {
                let off = (r * VGA_COLS + c) * 2;
                text[off] = b' ';
                text[off + 1] = attr;
            }
        };
        if up {
            for r in top as usize..=rows {
                let src = r + lines;
                if src <= rows {
                    for c in left as usize..=cols {
                        let so = (src * VGA_COLS + c) * 2;
                        let dst = (r * VGA_COLS + c) * 2;
                        self.text[dst] = self.text[so];
                        self.text[dst + 1] = self.text[so + 1];
                    }
                } else if lines > 0 {
                    fill(&mut self.text, r, left as usize, cols);
                    lines = lines.saturating_sub(1);
                }
            }
        }
        self.dirty = true;
    }

    // ------------------------------------------------------------------
    // Text buffer MMIO
    // ------------------------------------------------------------------

    fn text_offset(addr: u64) -> usize {
        if addr >= VGA_TEXT_BASE {
            (addr - VGA_TEXT_BASE) as usize
        } else {
            (addr & (VGA_TEXT_SIZE as u64 - 1)) as usize
        }
    }

    fn text_read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        if size != 1 && size != 2 && size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        let off = Self::text_offset(addr);
        if off + size as usize > VGA_TEXT_SIZE {
            return Err(DeviceError::InvalidAddress);
        }
        let mut v = 0u64;
        for i in 0..size as usize {
            v |= (self.text[off + i] as u64) << (i * 8);
        }
        Ok(v)
    }

    fn text_write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 1 && size != 2 && size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        let off = Self::text_offset(addr);
        if off + size as usize > VGA_TEXT_SIZE {
            return Err(DeviceError::InvalidAddress);
        }
        let bytes = value.to_le_bytes();
        for i in 0..size as usize {
            self.text[off + i] = bytes[i];
        }
        self.dirty = true;
        Ok(())
    }

    // ------------------------------------------------------------------
    // VESA LFB MMIO
    // ------------------------------------------------------------------

    fn fb_offset(addr: u64) -> usize {
        if addr >= VESA_LFB_BASE {
            (addr - VESA_LFB_BASE) as usize
        } else {
            (addr & (VESA_FB_SIZE as u64 - 1)) as usize
        }
    }

    fn fb_byte(&self, off: usize) -> u8 {
        self.framebuffer.get(off).copied().unwrap_or(0)
    }

    fn fb_read(&self, addr: u64, size: u8) -> Result<u64, DeviceError> {
        if size != 1 && size != 2 && size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        let off = Self::fb_offset(addr);
        if off + size as usize > VESA_FB_SIZE {
            return Err(DeviceError::InvalidAddress);
        }
        let mut v = 0u64;
        for i in 0..size as usize {
            v |= (self.framebuffer[off + i] as u64) << (i * 8);
        }
        Ok(v)
    }

    fn fb_write(&mut self, addr: u64, value: u64, size: u8) -> Result<(), DeviceError> {
        if size != 1 && size != 2 && size != 4 {
            return Err(DeviceError::UnsupportedSize);
        }
        let off = Self::fb_offset(addr);
        if off + size as usize > VESA_FB_SIZE {
            return Err(DeviceError::InvalidAddress);
        }
        let bytes = value.to_le_bytes();
        for i in 0..size as usize {
            self.framebuffer[off + i] = bytes[i];
        }
        self.dirty = true;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Port-mapped VGA controller
    // ------------------------------------------------------------------

    fn port_offset(port: u16) -> u16 {
        port - VGA_PORT_BASE
    }

    pub fn port_read(&mut self, port: u16) -> u8 {
        match Self::port_offset(port) {
            off if off == (VGA_ATC_INDEX - VGA_PORT_BASE) => {
                self.atc_regs[self.atc_index as usize & 0x1F]
            }
            off if off == (VGA_ATC_DATA - VGA_PORT_BASE) => self.atc_index,
            off if off == (VGA_SEQ_INDEX - VGA_PORT_BASE) => {
                self.seq_regs[self.seq_index as usize & 0x0F]
            }
            off if off == (VGA_SEQ_DATA - VGA_PORT_BASE) => self.seq_index,
            off if off == (VGA_DAC_READ_INDEX - VGA_PORT_BASE) => 3,
            off if off == (VGA_DAC_WRITE_INDEX - VGA_PORT_BASE) => self.dac_write_index,
            off if off == (VGA_DAC_DATA - VGA_PORT_BASE) => self.dac_port_read(),
            off if off == (VGA_GC_INDEX - VGA_PORT_BASE) => {
                self.gc_regs[self.gc_index as usize & 0x0F]
            }
            off if off == (VGA_GC_DATA - VGA_PORT_BASE) => self.gc_index,
            off if off == (VGA_CRTC_INDEX - VGA_PORT_BASE) => self.crtc_index,
            off if off == (VGA_CRTC_DATA - VGA_PORT_BASE) => self.crtc_data_read(),
            off if off == (VGA_INPUT_STATUS - VGA_PORT_BASE) => 0x10,
            _ => 0xFF,
        }
    }

    pub fn port_write(&mut self, port: u16, value: u8) {
        match Self::port_offset(port) {
            off if off == (VGA_ATC_INDEX - VGA_PORT_BASE) => self.atc_index = value & 0x1F,
            off if off == (VGA_ATC_DATA - VGA_PORT_BASE) => {
                self.atc_regs[self.atc_index as usize & 0x1F] = value;
            }
            off if off == (VGA_SEQ_INDEX - VGA_PORT_BASE) => self.seq_index = value & 0x07,
            off if off == (VGA_SEQ_DATA - VGA_PORT_BASE) => {
                self.seq_regs[self.seq_index as usize & 0x0F] = value;
            }
            off if off == (VGA_DAC_READ_INDEX - VGA_PORT_BASE) => {
                self.dac_read_index = value;
                self.dac_read_phase = 0;
            }
            off if off == (VGA_DAC_WRITE_INDEX - VGA_PORT_BASE) => {
                self.dac_write_index = value;
                self.dac_write_phase = 0;
            }
            off if off == (VGA_DAC_DATA - VGA_PORT_BASE) => self.dac_port_write(value),
            off if off == (VGA_GC_INDEX - VGA_PORT_BASE) => self.gc_index = value & 0x0F,
            off if off == (VGA_GC_DATA - VGA_PORT_BASE) => {
                self.gc_regs[self.gc_index as usize & 0x0F] = value;
            }
            off if off == (VGA_CRTC_INDEX - VGA_PORT_BASE) => self.crtc_index = value & 0x1F,
            off if off == (VGA_CRTC_DATA - VGA_PORT_BASE) => self.crtc_data_write(value),
            off if off == (VGA_INPUT_STATUS - VGA_PORT_BASE) => {}
            _ => {}
        }
    }

    fn dac_port_read(&mut self) -> u8 {
        let idx = self.dac_read_index as usize;
        let v = self.dac[idx * 3 + self.dac_read_phase as usize];
        self.dac_read_phase += 1;
        if self.dac_read_phase >= 3 {
            self.dac_read_phase = 0;
            self.dac_read_index = self.dac_read_index.wrapping_add(1);
        }
        v
    }

    fn dac_port_write(&mut self, value: u8) {
        let idx = self.dac_write_index as usize;
        self.dac[idx * 3 + self.dac_write_phase as usize] = value & 0x3F;
        self.dac_write_phase += 1;
        if self.dac_write_phase >= 3 {
            self.dac_write_phase = 0;
            self.dac_write_index = self.dac_write_index.wrapping_add(1);
        }
        self.dirty = true;
    }

    fn crtc_data_read(&self) -> u8 {
        match self.crtc_index {
            0x0E => (((self.cursor_y as usize * VGA_COLS + self.cursor_x as usize) >> 8) & 0xFF) as u8,
            0x0F => ((self.cursor_y as usize * VGA_COLS + self.cursor_x as usize) & 0xFF) as u8,
            i => self.crtc_regs[i as usize & 0x1F],
        }
    }

    fn crtc_data_write(&mut self, value: u8) {
        let idx = self.crtc_index as usize & 0x1F;
        self.crtc_regs[idx] = value;
        match self.crtc_index {
            0x0A => self.cursor_visible = value & 0x20 == 0,
            0x0E | 0x0F => {
                let linear = (self.crtc_regs[0x0E] as usize) << 8 | self.crtc_regs[0x0F] as usize;
                self.cursor_y = ((linear / VGA_COLS) % VGA_ROWS) as u8;
                self.cursor_x = (linear % VGA_COLS) as u8;
            }
            _ => {}
        }
        self.dirty = true;
    }

    // ------------------------------------------------------------------
    // Host-side rendering
    // ------------------------------------------------------------------

    pub fn pixel_width(&self) -> usize {
        match self.mode {
            VideoMode::Text => VGA_COLS * FONT_W,
            VideoMode::Vesa => self.vesa_width as usize,
        }
    }

    pub fn pixel_height(&self) -> usize {
        match self.mode {
            VideoMode::Text => VGA_ROWS * FONT_H,
            VideoMode::Vesa => self.vesa_height as usize,
        }
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn render_frame(&mut self) {
        match self.mode {
            VideoMode::Text => self.render_text_to_pixels(),
            VideoMode::Vesa => self.render_vesa_to_pixels(),
        }
        self.dirty = false;
    }

    fn render_text_to_pixels(&mut self) {
        let w = VGA_COLS * FONT_W;
        let h = VGA_ROWS * FONT_H;
        self.pixels.resize(w * h, 0);
        for row in 0..VGA_ROWS {
            for col in 0..VGA_COLS {
                let off = (row * VGA_COLS + col) * 2;
                let ch = self.text[off];
                let attr = self.text[off + 1];
                let fg = self.text_fg_rgb(attr);
                let bg = self.text_bg_rgb(attr);
                let glyph = lookup_glyph(ch);
                let is_cursor =
                    self.cursor_visible && row as u8 == self.cursor_y && col as u8 == self.cursor_x;
                for gy in 0..FONT_H {
                    let bits = if is_cursor && gy >= FONT_H / 2 {
                        0xFF
                    } else {
                        glyph[gy]
                    };
                    for gx in 0..FONT_W {
                        let set = bits & (0x80 >> gx) != 0;
                        let px = (row * FONT_H + gy) * w + (col * FONT_W + gx);
                        self.pixels[px] = if set { fg } else { bg };
                    }
                }
            }
        }
    }

    fn render_vesa_to_pixels(&mut self) {
        let w = self.vesa_width as usize;
        let h = self.vesa_height as usize;
        if w == 0 || h == 0 {
            self.pixels.clear();
            return;
        }
        self.pixels.resize(w * h, 0);
        let bpl = self.vesa_bytes_per_scanline as usize;
        let bpc = (self.vesa_bpp as usize).div_ceil(8);
        for y in 0..h {
            let row_off = y * bpl;
            for x in 0..w {
                let off = row_off + x * bpc;
                let rgb = match self.vesa_bpp {
                    8 => self.dac_rgb(self.fb_byte(off) as usize),
                    15 => {
                        let v = u16::from_le_bytes([self.fb_byte(off), self.fb_byte(off + 1)]);
                        let r = (v >> 10) & 0x1F;
                        let g = (v >> 5) & 0x1F;
                        let b = v & 0x1F;
                        (u32::from(r) * 255 / 31) << 16
                            | (u32::from(g) * 255 / 31) << 8
                            | u32::from(b) * 255 / 31
                    }
                    16 => {
                        let v = u16::from_le_bytes([self.fb_byte(off), self.fb_byte(off + 1)]);
                        let r = (v >> 11) & 0x1F;
                        let g = (v >> 5) & 0x3F;
                        let b = v & 0x1F;
                        (u32::from(r) * 255 / 31) << 16
                            | (u32::from(g) * 255 / 63) << 8
                            | u32::from(b) * 255 / 31
                    }
                    _ => {
                        let r = u32::from(self.fb_byte(off + 2));
                        let g = u32::from(self.fb_byte(off + 1));
                        let b = u32::from(self.fb_byte(off));
                        (r << 16) | (g << 8) | b
                    }
                };
                self.pixels[y * w + x] = rgb;
            }
        }
    }

    pub fn text_snapshot(&self) -> String {
        let mut s = String::with_capacity((VGA_COLS + 1) * VGA_ROWS);
        for row in 0..VGA_ROWS {
            for col in 0..VGA_COLS {
                let ch = self.text[(row * VGA_COLS + col) * 2];
                s.push(if ch == 0 { ' ' } else { ch as char });
            }
            s.push('\n');
        }
        s
    }

    pub fn save_ppm(&mut self, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        self.render_frame();
        let w = self.pixel_width();
        let h = self.pixel_height();
        let mut out = Vec::with_capacity(w * h * 3 + 64);
        out.extend_from_slice(format!("P6\n{} {}\n255\n", w, h).as_bytes());
        for &p in &self.pixels {
            out.push(((p >> 16) & 0xFF) as u8);
            out.push(((p >> 8) & 0xFF) as u8);
            out.push((p & 0xFF) as u8);
        }
        std::fs::write(path, out)
    }

    fn vbe_geometry(mode: u16) -> Option<(u32, u32, u8)> {
        match mode {
            0x101 => Some((640, 480, 8)),
            0x103 => Some((800, 600, 8)),
            0x105 => Some((1024, 768, 8)),
            0x110 => Some((640, 480, 15)),
            0x111 => Some((640, 480, 16)),
            0x112 => Some((640, 480, 32)),
            0x113 => Some((800, 600, 15)),
            0x114 => Some((800, 600, 16)),
            0x115 => Some((800, 600, 32)),
            0x116 => Some((1024, 768, 15)),
            0x117 => Some((1024, 768, 24)),
            0x118 => Some((1024, 768, 32)),
            _ => None,
        }
    }

    fn vbe_color_field(bpp: u8) -> [u8; 8] {
        match bpp {
            15 => [5, 10, 5, 5, 5, 0, 1, 15],
            16 => [5, 11, 6, 5, 5, 0, 0, 0],
            24 => [8, 16, 8, 8, 8, 0, 0, 0],
            32 => [8, 16, 8, 8, 8, 0, 8, 24],
            _ => [0; 8],
        }
    }

    fn vbe_memory_model(bpp: u8) -> u8 {
        match bpp {
            8 => 4,
            _ => 6,
        }
    }

    fn write_vbe_mode_info(&self, mmu: &mut Mmu, addr: u64, mode: u16) {
        let Some((w, h, bpp)) = Self::vbe_geometry(mode) else {
            return;
        };
        let pitch = w * (bpp as u32 / 8);
        let mut buf = [0u8; 256];
        let mut attr: u16 = 0x0001 | 0x0008 | 0x0010 | 0x0020;
        if bpp == 8 {
            attr |= 0x0004;
        }
        buf[0x00..0x02].copy_from_slice(&attr.to_le_bytes());
        buf[0x08..0x0A].copy_from_slice(&pitch.to_le_bytes());
        buf[0x10..0x12].copy_from_slice(&pitch.to_le_bytes());
        buf[0x12..0x14].copy_from_slice(&w.to_le_bytes());
        buf[0x14..0x16].copy_from_slice(&h.to_le_bytes());
        buf[0x16] = 8;
        buf[0x17] = 16;
        buf[0x18] = 1;
        buf[0x19] = bpp;
        buf[0x1A] = 1;
        buf[0x1B] = Self::vbe_memory_model(bpp);
        buf[0x1C] = 0;
        buf[0x1D] = 1;
        buf[0x20..0x28].copy_from_slice(&Self::vbe_color_field(bpp));
        buf[0x28] = 0;
        buf[0x2A..0x2E].copy_from_slice(&(VESA_LFB_BASE as u32).to_le_bytes());
        buf[0x2E..0x32].fill(0);
        buf[0x32..0x34].copy_from_slice(&0u16.to_le_bytes());
        let _ = mmu.write_phys(addr, &buf);
    }

    pub fn int10(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let ah = ((cpu.rax >> 8) & 0xFF) as u8;
        match ah {
            0x00 => {
                let mode = (cpu.rax & 0xFF) as u8;
                match mode & 0x7F {
                    0x00..=0x03 | 0x07 => self.enter_text_mode(mode),
                    0x13 => self.enter_vesa_mode(0x101, 320, 200, 8, true),
                    0x11 | 0x12 => self.enter_vesa_mode(0x112, 640, 480, 32, true),
                    _ => {}
                }
            }
            0x02 => {
                let row = ((cpu.rdx >> 8) & 0xFF) as u8;
                let col = (cpu.rdx & 0xFF) as u8;
                self.set_cursor(row, col);
            }
            0x06 => {
                let lines = (cpu.rax & 0xFF) as u8;
                let attr = ((cpu.rbx >> 8) & 0xFF) as u8;
                let top = ((cpu.rcx >> 8) & 0xFF) as u8;
                let left = (cpu.rcx & 0xFF) as u8;
                let bottom = ((cpu.rdx >> 8) & 0xFF) as u8;
                let right = (cpu.rdx & 0xFF) as u8;
                self.scroll_rect(lines, true, top, left, bottom, right, attr);
            }
            0x09 | 0x0A => {
                let ch = (cpu.rax & 0xFF) as u8;
                let attr = ((cpu.rbx >> 8) & 0xFF) as u8;
                let count = (cpu.rcx & 0xFFFF) as u16;
                self.write_chars(ch, attr, count);
            }
            0x0E => {
                let ch = (cpu.rax & 0xFF) as u8;
                self.put_char(ch, ((cpu.rbx >> 8) & 0xFF) as u8);
            }
            0x0F => {
                cpu.rax =
                    (cpu.rax & !0xFFFF) | ((VGA_COLS as u64) << 8) | self.current_mode as u64;
            }
            0x4F => self.int10_vesa(cpu, mmu),
            _ => {}
        }
    }

    fn int10_vesa(&mut self, cpu: &mut CpuState, mmu: &mut Mmu) {
        let sub = (cpu.rax & 0xFF) as u8;
        match sub {
            0x00 => {
                let es = cpu.es.selector;
                let di = (cpu.rdi & 0xFFFF) as u16;
                let mut buf = [0u8; 512];
                buf[0..4].copy_from_slice(b"VESA");
                buf[4..6].copy_from_slice(&0x0300u16.to_le_bytes());
                let ptr = ((real_mode_ptr(es, di) + 0x100) as u32).to_le_bytes();
                buf[0x0C..0x10].copy_from_slice(&ptr);
                let mut off = 0x100usize;
                for &m in &VBE_MODES {
                    buf[off..off + 2].copy_from_slice(&m.to_le_bytes());
                    off += 2;
                }
                buf[off..off + 2].copy_from_slice(&0xFFFFu16.to_le_bytes());
                let _ = mmu.write_phys(real_mode_ptr(es, di), &buf);
                set_ax(cpu, 0x004F);
            }
            0x01 => {
                let mode = (cpu.rcx & 0xFFFF) as u16;
                let es = cpu.es.selector;
                let di = (cpu.rdi & 0xFFFF) as u16;
                if Self::vbe_geometry(mode).is_some() {
                    self.write_vbe_mode_info(mmu, real_mode_ptr(es, di), mode);
                    set_ax(cpu, 0x004F);
                } else {
                    set_ax(cpu, 0x014F);
                }
            }
            0x02 => {
                let bx = (cpu.rbx & 0xFFFF) as u16;
                let mode = bx & 0x7FFF;
                let lfb = bx & 0x4000 != 0;
                if let Some((w, h, bpp)) = Self::vbe_geometry(mode) {
                    self.enter_vesa_mode(mode, w, h, bpp, lfb);
                    set_ax(cpu, 0x004F);
                } else if mode == 0x03 {
                    self.enter_text_mode(0x03);
                    set_ax(cpu, 0x004F);
                } else {
                    set_ax(cpu, 0x014F);
                }
            }
            0x03 => {
                let bx = if self.mode == VideoMode::Vesa {
                    self.vesa_mode
                } else {
                    0x0003
                };
                cpu.rbx = (cpu.rbx & !0xFFFF) | bx as u64;
                set_ax(cpu, 0x004F);
            }
            _ => set_ax(cpu, 0x004F),
        }
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
