use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, Ordering};

static LOCKED: AtomicBool = AtomicBool::new(false);
static mut CONSOLE: Console = Console::new();

pub fn init() {
    // Safety: early boot is single-threaded and this is the only initialization.
    unsafe {
        let console = &raw mut CONSOLE;
        (*console).init();
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

struct Console {
    #[cfg(target_arch = "x86_64")]
    vga_column: usize,
}

impl Console {
    const fn new() -> Self {
        Self {
            #[cfg(target_arch = "x86_64")]
            vga_column: 0,
        }
    }

    fn init(&mut self) {
        #[cfg(target_arch = "x86_64")]
        serial::init();
    }

    fn write_byte(&mut self, byte: u8) {
        let _ = byte;
        #[cfg(target_arch = "x86_64")]
        {
            serial::write(byte);
            vga::write(byte, &mut self.vga_column);
        }
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
