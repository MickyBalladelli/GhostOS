use core::fmt::{self, Write};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use synos_boot_protocol::FramebufferInfo;
#[cfg(target_arch = "x86_64")]
use synos_boot_protocol::{FRAMEBUFFER_PIXEL_BGR, FRAMEBUFFER_PIXEL_RGB};

static LOCKED: AtomicBool = AtomicBool::new(false);
static REMOTE_COLUMNS: AtomicUsize = AtomicUsize::new(0);
static REMOTE_ROWS: AtomicUsize = AtomicUsize::new(0);
static mut CONSOLE: Console = Console::new();

pub fn init(framebuffer: FramebufferInfo) {
    REMOTE_COLUMNS.store(0, Ordering::Relaxed);
    REMOTE_ROWS.store(0, Ordering::Relaxed);
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
        (*console).begin_write();
        let _ = (*console).write_fmt(args);
        (*console).end_write()
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

pub fn clear() {
    while LOCKED
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop()
    }

    unsafe {
        let console = &raw mut CONSOLE;
        (*console).clear()
    }
    LOCKED.store(false, Ordering::Release);
}

pub fn terminal_size() -> (usize, usize) {
    let remote_columns = REMOTE_COLUMNS.load(Ordering::Acquire);
    let remote_rows = REMOTE_ROWS.load(Ordering::Acquire);
    if remote_columns != 0 && remote_rows != 0 {
        return (remote_columns, remote_rows)
    }

    #[cfg(target_arch = "x86_64")]
    {
        while LOCKED
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop()
        }
        let size = unsafe {
            let console = &raw const CONSOLE;
            (*console).terminal_size()
        };
        LOCKED.store(false, Ordering::Release);
        size
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        (80, 25)
    }
}

pub fn set_remote_terminal_size(columns: usize, rows: usize) {
    if columns == 0 || rows == 0 {
        return
    }
    REMOTE_COLUMNS.store(columns, Ordering::Release);
    REMOTE_ROWS.store(rows, Ordering::Release)
}

struct Console {
    #[cfg(target_arch = "x86_64")]
    vga: VgaConsole,
    #[cfg(target_arch = "x86_64")]
    framebuffer: Option<FramebufferConsole>,
}

impl Console {
    const fn new() -> Self {
        Self {
            #[cfg(target_arch = "x86_64")]
            vga: VgaConsole::new(),
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
                self.vga.write(byte)
            }
        }
    }

    fn begin_write(&mut self) {
        #[cfg(target_arch = "x86_64")]
        if let Some(framebuffer) = &mut self.framebuffer {
            framebuffer.hide_cursor()
        }
    }

    fn end_write(&mut self) {
        #[cfg(target_arch = "x86_64")]
        if let Some(framebuffer) = &mut self.framebuffer {
            framebuffer.show_cursor()
        } else {
            self.vga.update_cursor()
        }
    }

    fn clear(&mut self) {
        #[cfg(target_arch = "x86_64")]
        {
            serial::clear();
            if let Some(framebuffer) = &mut self.framebuffer {
                framebuffer.clear()
            } else {
                self.vga.clear()
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn terminal_size(&self) -> (usize, usize) {
        self.framebuffer
            .as_ref()
            .map_or((80, 25), |framebuffer| {
                (framebuffer.columns(), framebuffer.rows())
            })
    }
}

#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
enum AnsiEvent {
    Byte(u8),
    Escape(u8),
    Csi {
        final_byte: u8,
        params: [u16; 8],
        count: usize,
        private: bool,
    },
}

#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
enum AnsiState {
    Ground,
    Escape,
    Csi,
    Osc,
    OscEscape,
}

#[cfg(target_arch = "x86_64")]
struct AnsiParser {
    state: AnsiState,
    params: [u16; 8],
    param_index: usize,
    private: bool,
}

#[cfg(target_arch = "x86_64")]
impl AnsiParser {
    const fn new() -> Self {
        Self {
            state: AnsiState::Ground,
            params: [0; 8],
            param_index: 0,
            private: false,
        }
    }

    fn reset(&mut self) {
        self.state = AnsiState::Ground;
        self.params = [0; 8];
        self.param_index = 0;
        self.private = false
    }

    fn advance(&mut self, byte: u8) -> Option<AnsiEvent> {
        match self.state {
            AnsiState::Ground => {
                if byte == 0x1b {
                    self.state = AnsiState::Escape;
                    None
                } else {
                    Some(AnsiEvent::Byte(byte))
                }
            }
            AnsiState::Escape => match byte {
                b'[' => {
                    self.params = [0; 8];
                    self.param_index = 0;
                    self.private = false;
                    self.state = AnsiState::Csi;
                    None
                }
                b']' => {
                    self.state = AnsiState::Osc;
                    None
                }
                0x1b => None,
                _ => {
                    self.state = AnsiState::Ground;
                    Some(AnsiEvent::Escape(byte))
                }
            },
            AnsiState::Csi => match byte {
                b'0'..=b'9' => {
                    let value = &mut self.params[self.param_index];
                    *value = value
                        .saturating_mul(10)
                        .saturating_add((byte - b'0') as u16);
                    None
                }
                b';' => {
                    if self.param_index + 1 < self.params.len() {
                        self.param_index += 1
                    }
                    None
                }
                b'?' if self.param_index == 0 && self.params[0] == 0 => {
                    self.private = true;
                    None
                }
                0x40..=0x7e => {
                    let event = AnsiEvent::Csi {
                        final_byte: byte,
                        params: self.params,
                        count: self.param_index + 1,
                        private: self.private,
                    };
                    self.reset();
                    Some(event)
                }
                0x1b => {
                    self.state = AnsiState::Escape;
                    None
                }
                _ => {
                    self.reset();
                    None
                }
            },
            AnsiState::Osc => match byte {
                0x07 => {
                    self.reset();
                    None
                }
                0x1b => {
                    self.state = AnsiState::OscEscape;
                    None
                }
                _ => None,
            },
            AnsiState::OscEscape => {
                if byte == b'\\' {
                    self.reset()
                } else {
                    self.state = AnsiState::Osc
                }
                None
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
struct TerminalStyle {
    foreground: u8,
    background: u8,
    bold: bool,
}

#[cfg(target_arch = "x86_64")]
impl TerminalStyle {
    const fn new() -> Self {
        Self {
            foreground: 7,
            background: 0,
            bold: false,
        }
    }

    fn reset(&mut self) {
        *self = Self::new()
    }

    fn apply_sgr(&mut self, params: &[u16]) {
        for parameter in params {
            match *parameter {
                0 => self.reset(),
                1 => self.bold = true,
                2 | 22 => self.bold = false,
                30..=37 => self.foreground = (*parameter - 30) as u8,
                39 => self.foreground = 7,
                40..=47 => self.background = (*parameter - 40) as u8,
                49 => self.background = 0,
                90..=97 => self.foreground = (*parameter - 90 + 8) as u8,
                100..=107 => self.background = (*parameter - 100 + 8) as u8,
                _ => {}
            }
        }
    }

    const fn foreground(&self) -> u8 {
        if self.bold && self.foreground < 8 {
            self.foreground + 8
        } else {
            self.foreground
        }
    }
}

#[cfg(target_arch = "x86_64")]
fn parameter(params: &[u16; 8], index: usize, default: usize) -> usize {
    match params.get(index).copied().unwrap_or(0) as usize {
        0 => default,
        value => value,
    }
}

#[cfg(target_arch = "x86_64")]
struct FramebufferConsole {
    address: *mut u32,
    width: usize,
    height: usize,
    stride: usize,
    pixel_format: u32,
    column: usize,
    row: usize,
    saved_column: usize,
    saved_row: usize,
    parser: AnsiParser,
    style: TerminalStyle,
    cursor_visible: bool,
    cursor_drawn: bool,
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
            return None;
        }

        Some(Self {
            address: info.address as *mut u32,
            width: info.width as usize,
            height: info.height as usize,
            stride: info.stride as usize,
            pixel_format: info.pixel_format,
            column: 0,
            row: 0,
            saved_column: 0,
            saved_row: 0,
            parser: AnsiParser::new(),
            style: TerminalStyle::new(),
            cursor_visible: true,
            cursor_drawn: false,
        })
    }

    fn write(&mut self, byte: u8) {
        let Some(event) = self.parser.advance(byte) else {
            return;
        };
        match event {
            AnsiEvent::Byte(byte) => self.write_plain(byte),
            AnsiEvent::Escape(byte) => self.escape(byte),
            AnsiEvent::Csi {
                final_byte,
                params,
                count,
                private,
            } => self.csi(final_byte, &params, count, private),
        }
    }

    fn write_plain(&mut self, byte: u8) {
        match byte {
            b'\n' => {
                self.new_line();
                return;
            }
            b'\r' => {
                self.column = 0;
                return;
            }
            8 => {
                if self.column != 0 {
                    self.column -= 1;
                    let x = self.column * Self::GLYPH_WIDTH;
                    let y = self.row * Self::GLYPH_HEIGHT;
                    self.draw_glyph(x, y, glyph(b' '))
                }
                return;
            }
            b'\t' => {
                let target = core::cmp::min((self.column + 8) & !7, self.columns() - 1);
                while self.column < target {
                    self.put_byte(b' ')
                }
                return;
            }
            0x00..=0x1f | 0x7f => return,
            _ => {}
        }

        self.put_byte(byte)
    }

    fn put_byte(&mut self, byte: u8) {
        let x = self.column * Self::GLYPH_WIDTH;
        let y = self.row * Self::GLYPH_HEIGHT;
        self.draw_glyph(x, y, glyph(byte));
        self.column += 1;
        if self.column >= self.columns() {
            self.new_line()
        }
    }

    fn escape(&mut self, byte: u8) {
        match byte {
            b'7' => {
                self.saved_column = self.column;
                self.saved_row = self.row
            }
            b'8' => {
                self.column = core::cmp::min(self.saved_column, self.columns() - 1);
                self.row = core::cmp::min(self.saved_row, self.rows() - 1)
            }
            b'c' => {
                self.style.reset();
                self.clear()
            }
            b'D' => self.new_line(),
            b'E' => self.new_line(),
            b'M' => self.row = self.row.saturating_sub(1),
            _ => {}
        }
    }

    fn csi(&mut self, final_byte: u8, params: &[u16; 8], count: usize, private: bool) {
        if private {
            if params[0] == 25 && matches!(final_byte, b'h' | b'l') {
                self.cursor_visible = final_byte == b'h'
            }
            return;
        }
        match final_byte {
            b'A' => self.row = self.row.saturating_sub(parameter(params, 0, 1)),
            b'B' => self.row = core::cmp::min(self.row + parameter(params, 0, 1), self.rows() - 1),
            b'C' => {
                self.column =
                    core::cmp::min(self.column + parameter(params, 0, 1), self.columns() - 1)
            }
            b'D' => self.column = self.column.saturating_sub(parameter(params, 0, 1)),
            b'E' => {
                self.row = core::cmp::min(self.row + parameter(params, 0, 1), self.rows() - 1);
                self.column = 0
            }
            b'F' => {
                self.row = self.row.saturating_sub(parameter(params, 0, 1));
                self.column = 0
            }
            b'G' => self.column = core::cmp::min(parameter(params, 0, 1) - 1, self.columns() - 1),
            b'H' | b'f' => {
                self.row = core::cmp::min(parameter(params, 0, 1) - 1, self.rows() - 1);
                self.column = core::cmp::min(parameter(params, 1, 1) - 1, self.columns() - 1)
            }
            b'J' => self.erase_display(params[0]),
            b'K' => self.erase_line(params[0]),
            b'm' => self.style.apply_sgr(&params[..count]),
            b's' => {
                self.saved_column = self.column;
                self.saved_row = self.row
            }
            b'u' => {
                self.column = core::cmp::min(self.saved_column, self.columns() - 1);
                self.row = core::cmp::min(self.saved_row, self.rows() - 1)
            }
            _ => {}
        }
    }

    fn erase_display(&mut self, mode: u16) {
        let cells = self.columns() * self.rows();
        let cursor = self.row * self.columns() + self.column;
        match mode {
            0 => {
                for cell in cursor..cells {
                    self.erase_cell(cell)
                }
            }
            1 => {
                for cell in 0..=cursor {
                    self.erase_cell(cell)
                }
            }
            2 | 3 => {
                for cell in 0..cells {
                    self.erase_cell(cell)
                }
            }
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let start = self.row * self.columns();
        match mode {
            0 => {
                for cell in start + self.column..start + self.columns() {
                    self.erase_cell(cell)
                }
            }
            1 => {
                for cell in start..=start + self.column {
                    self.erase_cell(cell)
                }
            }
            2 => {
                for cell in start..start + self.columns() {
                    self.erase_cell(cell)
                }
            }
            _ => {}
        }
    }

    fn erase_cell(&mut self, cell: usize) {
        let column = cell % self.columns();
        let row = cell / self.columns();
        self.draw_glyph(
            column * Self::GLYPH_WIDTH,
            row * Self::GLYPH_HEIGHT,
            glyph(b' '),
        )
    }

    fn new_line(&mut self) {
        self.column = 0;
        self.row += 1;
        if self.row >= self.rows() {
            self.scroll();
            self.row = self.rows() - 1
        }
    }

    fn draw_glyph(&mut self, x: usize, y: usize, rows: [u8; 7]) {
        let foreground = self.color(self.style.foreground());
        let background = self.color(self.style.background);
        for glyph_y in 0..8 {
            let bits = if glyph_y < 7 { rows[glyph_y] } else { 0 };
            for scale_y in 0..2 {
                for glyph_x in 0..Self::GLYPH_WIDTH {
                    let set = glyph_x < 5 && bits & (1 << (4 - glyph_x)) != 0;
                    self.write_pixel(
                        x + glyph_x,
                        y + glyph_y * 2 + scale_y,
                        if set { foreground } else { background },
                    )
                }
            }
        }
    }

    fn scroll(&mut self) {
        let shift = Self::GLYPH_HEIGHT * self.stride;
        let pixels = self.height * self.stride;
        let background = self.color(self.style.background);
        unsafe {
            core::ptr::copy(self.address.add(shift), self.address, pixels - shift);
            for index in (pixels - shift)..pixels {
                self.address.add(index).write_volatile(background)
            }
        }
    }

    fn clear(&mut self) {
        let pixels = self.height * self.stride;
        let background = self.color(self.style.background);
        unsafe {
            for index in 0..pixels {
                self.address.add(index).write_volatile(background)
            }
        }
        self.column = 0;
        self.row = 0;
        self.saved_column = 0;
        self.saved_row = 0;
        self.cursor_drawn = false;
        self.parser.reset()
    }

    fn write_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            unsafe { self.address.add(y * self.stride + x).write_volatile(color) }
        }
    }

    const fn columns(&self) -> usize {
        self.width / Self::GLYPH_WIDTH
    }

    const fn rows(&self) -> usize {
        self.height / Self::GLYPH_HEIGHT
    }

    fn color(&self, index: u8) -> u32 {
        let (red, green, blue) = ANSI_COLORS[index.min(15) as usize];
        if self.pixel_format == FRAMEBUFFER_PIXEL_RGB {
            (blue as u32) << 16 | (green as u32) << 8 | red as u32
        } else {
            (red as u32) << 16 | (green as u32) << 8 | blue as u32
        }
    }

    fn hide_cursor(&mut self) {
        if self.cursor_drawn {
            self.invert_cursor();
            self.cursor_drawn = false
        }
    }

    fn show_cursor(&mut self) {
        if self.cursor_visible && !self.cursor_drawn {
            self.invert_cursor();
            self.cursor_drawn = true
        }
    }

    fn invert_cursor(&mut self) {
        let x = self.column * Self::GLYPH_WIDTH;
        let y = self.row * Self::GLYPH_HEIGHT + Self::GLYPH_HEIGHT - 2;
        unsafe {
            for cursor_y in y..y + 2 {
                for cursor_x in x..x + Self::GLYPH_WIDTH - 1 {
                    let pixel = self.address.add(cursor_y * self.stride + cursor_x);
                    pixel.write_volatile(pixel.read_volatile() ^ 0x00ff_ffff)
                }
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
const ANSI_COLORS: [(u8, u8, u8); 16] = [
    (3, 10, 7),
    (190, 55, 65),
    (71, 186, 111),
    (210, 166, 72),
    (67, 126, 196),
    (169, 97, 183),
    (55, 181, 184),
    (194, 210, 198),
    (80, 98, 87),
    (255, 100, 110),
    (125, 231, 154),
    (255, 213, 105),
    (105, 169, 255),
    (220, 140, 234),
    (105, 225, 225),
    (231, 255, 236),
];

#[cfg(target_arch = "x86_64")]
fn glyph(byte: u8) -> [u8; 7] {
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
        b'a' => [0, 0, 14, 1, 15, 17, 15],
        b'b' => [16, 16, 22, 25, 17, 17, 30],
        b'c' => [0, 0, 14, 16, 16, 17, 14],
        b'd' => [1, 1, 13, 19, 17, 17, 15],
        b'e' => [0, 0, 14, 17, 31, 16, 14],
        b'f' => [6, 9, 8, 28, 8, 8, 8],
        b'g' => [0, 0, 15, 17, 15, 1, 14],
        b'h' => [16, 16, 22, 25, 17, 17, 17],
        b'i' => [4, 0, 12, 4, 4, 4, 14],
        b'j' => [2, 0, 6, 2, 2, 18, 12],
        b'k' => [16, 16, 18, 20, 24, 20, 18],
        b'l' => [12, 4, 4, 4, 4, 4, 14],
        b'm' => [0, 0, 26, 21, 21, 21, 21],
        b'n' => [0, 0, 22, 25, 17, 17, 17],
        b'o' => [0, 0, 14, 17, 17, 17, 14],
        b'p' => [0, 0, 30, 17, 30, 16, 16],
        b'q' => [0, 0, 15, 17, 15, 1, 1],
        b'r' => [0, 0, 22, 25, 16, 16, 16],
        b's' => [0, 0, 15, 16, 14, 1, 30],
        b't' => [8, 8, 28, 8, 8, 9, 6],
        b'u' => [0, 0, 17, 17, 17, 19, 13],
        b'v' => [0, 0, 17, 17, 17, 10, 4],
        b'w' => [0, 0, 17, 17, 21, 21, 10],
        b'x' => [0, 0, 17, 10, 4, 10, 17],
        b'y' => [0, 0, 17, 17, 15, 1, 14],
        b'z' => [0, 0, 31, 2, 4, 8, 31],
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
        b'$' => [4, 15, 20, 14, 5, 30, 4],
        b'-' => [0, 0, 0, 31, 0, 0, 0],
        b'_' => [0, 0, 0, 0, 0, 0, 31],
        b'/' => [1, 2, 2, 4, 8, 8, 16],
        b'\\' => [16, 8, 8, 4, 2, 2, 1],
        b'|' => [4, 4, 4, 4, 4, 4, 4],
        b'\'' => [4, 4, 8, 0, 0, 0, 0],
        b'(' => [2, 4, 8, 8, 8, 4, 2],
        b')' => [8, 4, 2, 2, 2, 4, 8],
        b'[' => [14, 8, 8, 8, 8, 8, 14],
        b']' => [14, 2, 2, 2, 2, 2, 14],
        b'<' => [2, 4, 8, 16, 8, 4, 2],
        b'>' => [8, 4, 2, 1, 2, 4, 8],
        b'#' => [10, 31, 10, 10, 31, 10, 0],
        b'+' => [0, 4, 4, 31, 4, 4, 0],
        b'*' => [0, 21, 14, 31, 14, 21, 0],
        b'@' => [14, 17, 23, 21, 23, 16, 14],
        b'%' => [17, 2, 4, 8, 16, 17, 0],
        b'^' => [4, 10, 17, 0, 0, 0, 0],
        b'&' => [12, 18, 20, 8, 21, 18, 13],
        b'"' => [10, 10, 20, 0, 0, 0, 0],
        b'`' => [8, 4, 2, 0, 0, 0, 0],
        b'~' => [0, 0, 9, 22, 0, 0, 0],
        b'{' => [2, 4, 4, 8, 4, 4, 2],
        b'}' => [8, 4, 4, 2, 4, 4, 8],
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

    pub fn clear() {
        for byte in b"\x1b[2J\x1b[H" {
            write(*byte)
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

    pub(super) unsafe fn outb(port: u16, value: u8) {
        unsafe {
            core::arch::asm!("out dx, al", in("dx") port, in("al") value, options(nomem, nostack));
        }
    }

    pub(super) unsafe fn inb(port: u16) -> u8 {
        let value: u8;
        unsafe {
            core::arch::asm!("in al, dx", out("al") value, in("dx") port, options(nomem, nostack));
        }
        value
    }
}

#[cfg(target_arch = "x86_64")]
struct VgaConsole {
    column: usize,
    row: usize,
    saved_column: usize,
    saved_row: usize,
    parser: AnsiParser,
    style: TerminalStyle,
    cursor_visible: bool,
}

#[cfg(target_arch = "x86_64")]
impl VgaConsole {
    const WIDTH: usize = 80;
    const HEIGHT: usize = 25;

    const fn new() -> Self {
        Self {
            column: 0,
            row: 0,
            saved_column: 0,
            saved_row: 0,
            parser: AnsiParser::new(),
            style: TerminalStyle::new(),
            cursor_visible: true,
        }
    }

    fn write(&mut self, byte: u8) {
        let Some(event) = self.parser.advance(byte) else {
            return;
        };
        match event {
            AnsiEvent::Byte(byte) => self.write_plain(byte),
            AnsiEvent::Escape(byte) => self.escape(byte),
            AnsiEvent::Csi {
                final_byte,
                params,
                count,
                private,
            } => self.csi(final_byte, &params, count, private),
        }
    }

    fn write_plain(&mut self, byte: u8) {
        match byte {
            b'\n' => self.new_line(),
            b'\r' => self.column = 0,
            8 => {
                if self.column != 0 {
                    self.column -= 1;
                    self.put_at(self.column, self.row, b' ')
                }
            }
            b'\t' => {
                let target = core::cmp::min((self.column + 8) & !7, Self::WIDTH - 1);
                while self.column < target {
                    self.put_byte(b' ')
                }
            }
            0x00..=0x1f | 0x7f => {}
            _ => self.put_byte(byte),
        }
    }

    fn put_byte(&mut self, byte: u8) {
        self.put_at(self.column, self.row, byte);
        self.column += 1;
        if self.column >= Self::WIDTH {
            self.new_line()
        }
    }

    fn put_at(&self, column: usize, row: usize, byte: u8) {
        let attribute = self.attribute();
        unsafe {
            vga::BUFFER
                .add(row * Self::WIDTH + column)
                .write_volatile((attribute as u16) << 8 | byte as u16)
        }
    }

    fn new_line(&mut self) {
        self.column = 0;
        self.row += 1;
        if self.row >= Self::HEIGHT {
            self.scroll();
            self.row = Self::HEIGHT - 1
        }
    }

    fn escape(&mut self, byte: u8) {
        match byte {
            b'7' => {
                self.saved_column = self.column;
                self.saved_row = self.row
            }
            b'8' => {
                self.column = self.saved_column;
                self.row = self.saved_row
            }
            b'c' => {
                self.style.reset();
                self.clear()
            }
            b'D' | b'E' => self.new_line(),
            b'M' => self.row = self.row.saturating_sub(1),
            _ => {}
        }
    }

    fn csi(&mut self, final_byte: u8, params: &[u16; 8], count: usize, private: bool) {
        if private {
            if params[0] == 25 && matches!(final_byte, b'h' | b'l') {
                self.cursor_visible = final_byte == b'h'
            }
            return;
        }
        match final_byte {
            b'A' => self.row = self.row.saturating_sub(parameter(params, 0, 1)),
            b'B' => self.row = core::cmp::min(self.row + parameter(params, 0, 1), Self::HEIGHT - 1),
            b'C' => {
                self.column = core::cmp::min(self.column + parameter(params, 0, 1), Self::WIDTH - 1)
            }
            b'D' => self.column = self.column.saturating_sub(parameter(params, 0, 1)),
            b'E' => {
                self.row = core::cmp::min(self.row + parameter(params, 0, 1), Self::HEIGHT - 1);
                self.column = 0
            }
            b'F' => {
                self.row = self.row.saturating_sub(parameter(params, 0, 1));
                self.column = 0
            }
            b'G' => self.column = core::cmp::min(parameter(params, 0, 1) - 1, Self::WIDTH - 1),
            b'H' | b'f' => {
                self.row = core::cmp::min(parameter(params, 0, 1) - 1, Self::HEIGHT - 1);
                self.column = core::cmp::min(parameter(params, 1, 1) - 1, Self::WIDTH - 1)
            }
            b'J' => self.erase_display(params[0]),
            b'K' => self.erase_line(params[0]),
            b'm' => self.style.apply_sgr(&params[..count]),
            b's' => {
                self.saved_column = self.column;
                self.saved_row = self.row
            }
            b'u' => {
                self.column = self.saved_column;
                self.row = self.saved_row
            }
            _ => {}
        }
    }

    fn erase_display(&self, mode: u16) {
        let cursor = self.row * Self::WIDTH + self.column;
        let (start, end) = match mode {
            0 => (cursor, Self::WIDTH * Self::HEIGHT),
            1 => (0, cursor + 1),
            2 | 3 => (0, Self::WIDTH * Self::HEIGHT),
            _ => return,
        };
        for cell in start..end {
            self.put_at(cell % Self::WIDTH, cell / Self::WIDTH, b' ')
        }
    }

    fn erase_line(&self, mode: u16) {
        let (start, end) = match mode {
            0 => (self.column, Self::WIDTH),
            1 => (0, self.column + 1),
            2 => (0, Self::WIDTH),
            _ => return,
        };
        for column in start..end {
            self.put_at(column, self.row, b' ')
        }
    }

    fn scroll(&self) {
        unsafe {
            core::ptr::copy(
                vga::BUFFER.add(Self::WIDTH),
                vga::BUFFER,
                Self::WIDTH * (Self::HEIGHT - 1),
            );
        }
        for column in 0..Self::WIDTH {
            self.put_at(column, Self::HEIGHT - 1, b' ')
        }
    }

    fn clear(&mut self) {
        for row in 0..Self::HEIGHT {
            for column in 0..Self::WIDTH {
                self.put_at(column, row, b' ')
            }
        }
        self.column = 0;
        self.row = 0;
        self.saved_column = 0;
        self.saved_row = 0;
        self.parser.reset()
    }

    fn attribute(&self) -> u8 {
        const ANSI_TO_VGA: [u8; 8] = [0, 4, 2, 6, 1, 5, 3, 7];
        let foreground = self.style.foreground();
        let foreground = ANSI_TO_VGA[(foreground & 7) as usize] | (foreground & 8);
        let background = ANSI_TO_VGA[(self.style.background & 7) as usize];
        background << 4 | foreground
    }

    fn update_cursor(&self) {
        vga::set_cursor(
            self.row * Self::WIDTH + self.column,
            self.cursor_visible,
        )
    }
}

#[cfg(target_arch = "x86_64")]
mod vga {
    pub const BUFFER: *mut u16 = 0xb8000 as *mut u16;

    pub fn set_cursor(position: usize, visible: bool) {
        unsafe {
            super::serial::outb(0x3d4, 0x0a);
            let start = super::serial::inb(0x3d5);
            super::serial::outb(
                0x3d5,
                if visible { start & !0x20 } else { start | 0x20 },
            );
            super::serial::outb(0x3d4, 0x0f);
            super::serial::outb(0x3d5, position as u8);
            super::serial::outb(0x3d4, 0x0e);
            super::serial::outb(0x3d5, (position >> 8) as u8)
        }
    }
}
