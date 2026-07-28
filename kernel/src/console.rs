use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};
use synos_boot_protocol::FramebufferInfo;
#[cfg(target_arch = "x86_64")]
use synos_boot_protocol::{FRAMEBUFFER_PIXEL_BGR, FRAMEBUFFER_PIXEL_RGB};

static LOCKED: AtomicBool = AtomicBool::new(false);
static mut CONSOLE: Console = Console::new();

pub fn init(framebuffer: FramebufferInfo) {
    // Safety: early boot is single-threaded and this is the only initialization.
    unsafe {
        let console = &raw mut CONSOLE;
        (*console).init(framebuffer);
    }
}

#[doc(hidden)]
pub fn _print(args: fmt::Arguments<'_>) {
    while LOCKED
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop()
    }

    // Safety: the spin lock serializes access to the static console.
    unsafe {
        let console = &raw mut CONSOLE;
        let _ = (*console).write_fmt(args);
    }
    LOCKED.store(false, Ordering::Release);
}

pub fn read_byte() -> Option<u8> {
    #[cfg(target_arch = "x86_64")]
    {
        serial::read()
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        None
    }
}

struct Console {
    #[cfg(target_arch = "x86_64")]
    vga_column: usize,
    #[cfg(target_arch = "x86_64")]
    framebuffer: Option<FramebufferConsole>,
}

impl Console {
    const fn new() -> Self {
        Self {
            #[cfg(target_arch = "x86_64")]
            vga_column: 0,
            #[cfg(target_arch = "x86_64")]
            framebuffer: None,
        }
    }

    fn init(&mut self, framebuffer: FramebufferInfo) {
        let _ = framebuffer;
        #[cfg(target_arch = "x86_64")]
        {
            serial::init();
            self.framebuffer = FramebufferConsole::new(framebuffer)
        }
    }

    fn write_byte(&mut self, byte: u8) {
        let _ = byte;
        #[cfg(target_arch = "x86_64")]
        {
            serial::write(byte);
            if let Some(framebuffer) = &mut self.framebuffer {
                framebuffer.write(byte)
            } else {
                vga::write(byte, &mut self.vga_column)
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
struct FramebufferConsole {
    address: *mut u32,
    width: usize,
    height: usize,
    stride: usize,
    column: usize,
    row: usize,
}

#[cfg(target_arch = "x86_64")]
impl FramebufferConsole {
    const GLYPH_WIDTH: usize = 6;
    const GLYPH_HEIGHT: usize = 16;

    fn new(info: FramebufferInfo) -> Option<Self> {
        let required = (info.stride as u64)
            .checked_mul(info.height as u64)?
            .checked_mul(size_of::<u32>() as u64)?;
        if info.address == 0
            || info.width < Self::GLYPH_WIDTH as u32
            || info.height < Self::GLYPH_HEIGHT as u32
            || info.stride < info.width
            || info.size < required
            || !matches!(
                info.pixel_format,
                FRAMEBUFFER_PIXEL_RGB | FRAMEBUFFER_PIXEL_BGR
            )
        {
            return None
        }

        Some(Self {
            address: info.address as *mut u32,
            width: info.width as usize,
            height: info.height as usize,
            stride: info.stride as usize,
            column: 0,
            row: 0,
        })
    }

    fn write(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                self.new_line();
                return
            }
            b'\r' => {
                self.column = 0;
                return
            }
            8 => {
                if self.column != 0 {
                    self.column -= 1;
                    let x = self.column * Self::GLYPH_WIDTH;
                    let y = self.row * Self::GLYPH_HEIGHT;
                    self.draw_glyph(x, y, glyph(b' '))
                }
                return
            }
            _ => {}
        }

        let x = self.column * Self::GLYPH_WIDTH;
        let y = self.row * Self::GLYPH_HEIGHT;
        self.draw_glyph(x, y, glyph(byte));
        self.column += 1;
        if (self.column + 1) * Self::GLYPH_WIDTH > self.width {
            self.new_line()
        }
    }

    fn new_line(&mut self) {
        self.column = 0;
        self.row += 1;
        if (self.row + 1) * Self::GLYPH_HEIGHT > self.height {
            self.scroll();
            self.row = self.height / Self::GLYPH_HEIGHT - 1
        }
    }

    fn draw_glyph(&mut self, x: usize, y: usize, rows: [u8; 7]) {
        for glyph_y in 0..8 {
            let bits = if glyph_y < 7 { rows[glyph_y] } else { 0 };
            for scale_y in 0..2 {
                for glyph_x in 0..Self::GLYPH_WIDTH {
                    let set = glyph_x < 5 && bits & (1 << (4 - glyph_x)) != 0;
                    self.write_pixel(
                        x + glyph_x,
                        y + glyph_y * 2 + scale_y,
                        if set { 0x00ff_ffff } else { 0 },
                    )
                }
            }
        }
    }

    fn scroll(&mut self) {
        let shift = Self::GLYPH_HEIGHT * self.stride;
        let pixels = self.height * self.stride;
        unsafe {
            for index in shift..pixels {
                let pixel = self.address.add(index).read_volatile();
                self.address.add(index - shift).write_volatile(pixel)
            }
            for index in (pixels - shift)..pixels {
                self.address.add(index).write_volatile(0)
            }
        }
    }

    fn write_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            unsafe {
                self.address
                    .add(y * self.stride + x)
                    .write_volatile(color)
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn glyph(byte: u8) -> [u8; 7] {
    let byte = if byte.is_ascii_lowercase() {
        byte.to_ascii_uppercase()
    } else {
        byte
    };
    match byte {
        b'A' => [14, 17, 17, 31, 17, 17, 17],
        b'B' => [30, 17, 17, 30, 17, 17, 30],
        b'C' => [14, 17, 16, 16, 16, 17, 14],
        b'D' => [30, 17, 17, 17, 17, 17, 30],
        b'E' => [31, 16, 16, 30, 16, 16, 31],
        b'F' => [31, 16, 16, 30, 16, 16, 16],
        b'G' => [14, 17, 16, 23, 17, 17, 14],
        b'H' => [17, 17, 17, 31, 17, 17, 17],
        b'I' => [14, 4, 4, 4, 4, 4, 14],
        b'J' => [7, 2, 2, 2, 18, 18, 12],
        b'K' => [17, 18, 20, 24, 20, 18, 17],
        b'L' => [16, 16, 16, 16, 16, 16, 31],
        b'M' => [17, 27, 21, 21, 17, 17, 17],
        b'N' => [17, 25, 21, 19, 17, 17, 17],
        b'O' => [14, 17, 17, 17, 17, 17, 14],
        b'P' => [30, 17, 17, 30, 16, 16, 16],
        b'Q' => [14, 17, 17, 17, 21, 18, 13],
        b'R' => [30, 17, 17, 30, 20, 18, 17],
        b'S' => [15, 16, 16, 14, 1, 1, 30],
        b'T' => [31, 4, 4, 4, 4, 4, 4],
        b'U' => [17, 17, 17, 17, 17, 17, 14],
        b'V' => [17, 17, 17, 17, 17, 10, 4],
        b'W' => [17, 17, 17, 21, 21, 21, 10],
        b'X' => [17, 17, 10, 4, 10, 17, 17],
        b'Y' => [17, 17, 10, 4, 4, 4, 4],
        b'Z' => [31, 1, 2, 4, 8, 16, 31],
        b'0' => [14, 17, 19, 21, 25, 17, 14],
        b'1' => [4, 12, 4, 4, 4, 4, 14],
        b'2' => [14, 17, 1, 2, 4, 8, 31],
        b'3' => [30, 1, 1, 14, 1, 1, 30],
        b'4' => [2, 6, 10, 18, 31, 2, 2],
        b'5' => [31, 16, 16, 30, 1, 1, 30],
        b'6' => [14, 16, 16, 30, 17, 17, 14],
        b'7' => [31, 1, 2, 4, 8, 8, 8],
        b'8' => [14, 17, 17, 14, 17, 17, 14],
        b'9' => [14, 17, 17, 15, 1, 1, 14],
        b'.' => [0, 0, 0, 0, 0, 12, 12],
        b',' => [0, 0, 0, 0, 4, 4, 8],
        b':' => [0, 4, 4, 0, 4, 4, 0],
        b';' => [0, 4, 4, 0, 4, 4, 8],
        b'=' => [0, 0, 31, 0, 31, 0, 0],
        b'-' => [0, 0, 0, 31, 0, 0, 0],
        b'_' => [0, 0, 0, 0, 0, 0, 31],
        b'/' => [1, 2, 2, 4, 8, 8, 16],
        b'(' => [2, 4, 8, 8, 8, 4, 2],
        b')' => [8, 4, 2, 2, 2, 4, 8],
        b'[' => [14, 8, 8, 8, 8, 8, 14],
        b']' => [14, 2, 2, 2, 2, 2, 14],
        b'<' => [2, 4, 8, 16, 8, 4, 2],
        b'>' => [8, 4, 2, 1, 2, 4, 8],
        b'#' => [10, 31, 10, 10, 31, 10, 0],
        b'!' => [4, 4, 4, 4, 4, 0, 4],
        b'?' => [14, 17, 1, 2, 4, 0, 4],
        b' ' => [0; 7],
        _ => [31, 17, 1, 2, 4, 0, 4],
    }
}

impl Write for Console {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        for byte in text.bytes() {
            self.write_byte(byte)
        }
        Ok(())
    }
}

#[cfg(target_arch = "x86_64")]
mod serial {
    const COM1: u16 = 0x3f8;

    pub fn init() {
        unsafe {
            outb(COM1 + 1, 0x00);
            outb(COM1 + 3, 0x80);
            outb(COM1, 0x03);
            outb(COM1 + 1, 0x00);
            outb(COM1 + 3, 0x03);
            outb(COM1 + 2, 0xc7);
            outb(COM1 + 4, 0x0b);
        }
    }

    pub fn write(byte: u8) {
        unsafe {
            let mut attempts = 100_000;
            while attempts != 0 && inb(COM1 + 5) & 0x20 == 0 {
                attempts -= 1;
                core::hint::spin_loop()
            }
            if attempts != 0 {
                outb(COM1, byte)
            }
        }
    }

    pub fn read() -> Option<u8> {
        unsafe {
            if inb(COM1 + 5) & 0x01 == 0 {
                None
            } else {
                Some(inb(COM1))
            }
        }
    }

    unsafe fn outb(port: u16, value: u8) {
        unsafe {
            core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack));
        }
    }

    unsafe fn inb(port: u16) -> u8 {
        let value: u8;
        unsafe {
            core::arch::asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack));
        }
        value
    }
}

#[cfg(target_arch = "x86_64")]
mod vga {
    const WIDTH: usize = 80;
    const HEIGHT: usize = 25;
    const BUFFER: *mut u16 = 0xb8000 as *mut u16;
    const COLOR: u16 = 0x0f00;

    pub fn write(byte: u8, column: &mut usize) {
        match byte {
            b'\n' => *column = ((*column / WIDTH) + 1) * WIDTH,
            b'\r' => *column = (*column / WIDTH) * WIDTH,
            8 => {
                let row_start = (*column / WIDTH) * WIDTH;
                if *column > row_start {
                    *column -= 1;
                    unsafe {
                        BUFFER.add(*column).write_volatile(COLOR | b' ' as u16);
                    }
                }
            }
            byte => {
                unsafe {
                    BUFFER.add(*column).write_volatile(COLOR | byte as u16);
                }
                *column += 1;
            }
        }

        if *column >= WIDTH * HEIGHT {
            scroll();
            *column = WIDTH * (HEIGHT - 1);
        }
    }

    fn scroll() {
        unsafe {
            for index in WIDTH..(WIDTH * HEIGHT) {
                let value = BUFFER.add(index).read_volatile();
                BUFFER.add(index - WIDTH).write_volatile(value);
            }
            for index in (WIDTH * (HEIGHT - 1))..(WIDTH * HEIGHT) {
                BUFFER.add(index).write_volatile(COLOR | b' ' as u16);
            }
        }
    }
}
