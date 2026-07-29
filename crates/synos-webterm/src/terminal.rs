#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Color {
    Default = 0,
    Black = 1,
    Red = 2,
    Green = 3,
    Yellow = 4,
    Blue = 5,
    Magenta = 6,
    Cyan = 7,
    White = 8,
    BrightBlack = 9,
    BrightRed = 10,
    BrightGreen = 11,
    BrightYellow = 12,
    BrightBlue = 13,
    BrightMagenta = 14,
    BrightCyan = 15,
    BrightWhite = 16,
}

impl Color {
    fn ansi(index: u16, bright: bool) -> Option<Self> {
        let value = index.checked_add(if bright { 9 } else { 1 })?;
        match value {
            1 => Some(Self::Black),
            2 => Some(Self::Red),
            3 => Some(Self::Green),
            4 => Some(Self::Yellow),
            5 => Some(Self::Blue),
            6 => Some(Self::Magenta),
            7 => Some(Self::Cyan),
            8 => Some(Self::White),
            9 => Some(Self::BrightBlack),
            10 => Some(Self::BrightRed),
            11 => Some(Self::BrightGreen),
            12 => Some(Self::BrightYellow),
            13 => Some(Self::BrightBlue),
            14 => Some(Self::BrightMagenta),
            15 => Some(Self::BrightCyan),
            16 => Some(Self::BrightWhite),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(transparent)]
pub struct CellAttributes(u8);

impl CellAttributes {
    pub const NONE: Self = Self(0);
    pub const BOLD: Self = Self(1 << 0);
    pub const DIM: Self = Self(1 << 1);
    pub const UNDERLINE: Self = Self(1 << 2);
    pub const BLINK: Self = Self(1 << 3);
    pub const INVERSE: Self = Self(1 << 4);
    pub const HIDDEN: Self = Self(1 << 5);

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn contains(self, attribute: Self) -> bool {
        self.0 & attribute.0 == attribute.0
    }

    fn insert(&mut self, attribute: Self) {
        self.0 |= attribute.0
    }

    fn remove(&mut self, attribute: Self) {
        self.0 &= !attribute.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct Cell {
    pub glyph: u32,
    pub foreground: Color,
    pub background: Color,
    pub attributes: CellAttributes,
}

impl Cell {
    pub const EMPTY: Self = Self {
        glyph: b' ' as u32,
        foreground: Color::Default,
        background: Color::Default,
        attributes: CellAttributes::NONE,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Cursor {
    pub column: u16,
    pub row: u16,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalModes {
    pub application_cursor_keys: bool,
    pub automatic_wrap: bool,
    pub insert: bool,
}

impl TerminalModes {
    const DEFAULT: Self = Self {
        application_cursor_keys: false,
        automatic_wrap: true,
        insert: false,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalError {
    EmptyGrid,
}

#[derive(Clone, Copy)]
enum ParserState {
    Ground,
    Escape,
    Csi,
    Osc,
    OscEscape,
}

#[derive(Clone, Copy)]
struct Parser {
    state: ParserState,
    params: [u16; 16],
    param_index: usize,
    private: bool,
}

impl Parser {
    const fn new() -> Self {
        Self {
            state: ParserState::Ground,
            params: [0; 16],
            param_index: 0,
            private: false,
        }
    }

    fn enter_csi(&mut self) {
        self.state = ParserState::Csi;
        self.params = [0; 16];
        self.param_index = 0;
        self.private = false
    }

    fn reset(&mut self) {
        self.state = ParserState::Ground;
        self.params = [0; 16];
        self.param_index = 0;
        self.private = false
    }
}

#[derive(Clone, Copy)]
struct Utf8Decoder {
    codepoint: u32,
    minimum: u32,
    remaining: u8,
}

enum Utf8Result {
    Pending,
    Glyph(u32),
    Invalid,
}

impl Utf8Decoder {
    const fn new() -> Self {
        Self {
            codepoint: 0,
            minimum: 0,
            remaining: 0,
        }
    }

    fn reset(&mut self) {
        *self = Self::new()
    }

    fn push(&mut self, byte: u8) -> Utf8Result {
        if self.remaining == 0 {
            match byte {
                0xc2..=0xdf => {
                    self.codepoint = (byte & 0x1f) as u32;
                    self.minimum = 0x80;
                    self.remaining = 1
                }
                0xe0..=0xef => {
                    self.codepoint = (byte & 0x0f) as u32;
                    self.minimum = 0x800;
                    self.remaining = 2
                }
                0xf0..=0xf4 => {
                    self.codepoint = (byte & 0x07) as u32;
                    self.minimum = 0x1_0000;
                    self.remaining = 3
                }
                _ => return Utf8Result::Invalid,
            }
            return Utf8Result::Pending;
        }
        if byte & 0xc0 != 0x80 {
            self.reset();
            return Utf8Result::Invalid;
        }
        self.codepoint = (self.codepoint << 6) | (byte & 0x3f) as u32;
        self.remaining -= 1;
        if self.remaining != 0 {
            return Utf8Result::Pending;
        }
        let glyph = self.codepoint;
        let valid =
            glyph >= self.minimum && glyph <= 0x10_ffff && !(0xd800..=0xdfff).contains(&glyph);
        self.reset();
        if valid {
            Utf8Result::Glyph(glyph)
        } else {
            Utf8Result::Invalid
        }
    }
}

/// Fixed-size VT100/VT420/DECterm terminal model.
pub struct Terminal<const COLUMNS: usize, const ROWS: usize> {
    cells: [[Cell; COLUMNS]; ROWS],
    dirty: [bool; ROWS],
    cursor: Cursor,
    saved_cursor: Cursor,
    style: Cell,
    modes: TerminalModes,
    parser: Parser,
    utf8: Utf8Decoder,
    scroll_top: usize,
    scroll_bottom: usize,
    wrap_pending: bool,
}

impl<const COLUMNS: usize, const ROWS: usize> Terminal<COLUMNS, ROWS> {
    pub fn new() -> Result<Self, TerminalError> {
        if COLUMNS == 0 || ROWS == 0 || COLUMNS > u16::MAX as usize || ROWS > u16::MAX as usize {
            return Err(TerminalError::EmptyGrid);
        }
        Ok(Self {
            cells: [[Cell::EMPTY; COLUMNS]; ROWS],
            dirty: [true; ROWS],
            cursor: Cursor {
                column: 0,
                row: 0,
                visible: true,
            },
            saved_cursor: Cursor {
                column: 0,
                row: 0,
                visible: true,
            },
            style: Cell::EMPTY,
            modes: TerminalModes::DEFAULT,
            parser: Parser::new(),
            utf8: Utf8Decoder::new(),
            scroll_top: 0,
            scroll_bottom: ROWS - 1,
            wrap_pending: false,
        })
    }

    pub const fn columns(&self) -> usize {
        COLUMNS
    }

    pub const fn rows(&self) -> usize {
        ROWS
    }

    pub const fn cursor(&self) -> Cursor {
        self.cursor
    }

    pub const fn modes(&self) -> TerminalModes {
        self.modes
    }

    pub fn row(&self, row: usize) -> Option<&[Cell; COLUMNS]> {
        self.cells.get(row)
    }

    pub fn row_is_dirty(&self, row: usize) -> bool {
        self.dirty.get(row).copied().unwrap_or(false)
    }

    pub fn mark_row_clean(&mut self, row: usize) {
        if let Some(dirty) = self.dirty.get_mut(row) {
            *dirty = false
        }
    }

    pub fn mark_all_dirty(&mut self) {
        self.dirty.fill(true)
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.advance(*byte)
        }
    }

    pub fn reset(&mut self) {
        self.cells = [[Cell::EMPTY; COLUMNS]; ROWS];
        self.dirty.fill(true);
        self.cursor = Cursor {
            column: 0,
            row: 0,
            visible: true,
        };
        self.saved_cursor = self.cursor;
        self.style = Cell::EMPTY;
        self.modes = TerminalModes::DEFAULT;
        self.scroll_top = 0;
        self.scroll_bottom = ROWS - 1;
        self.wrap_pending = false;
        self.parser.reset();
        self.utf8.reset()
    }

    fn advance(&mut self, byte: u8) {
        match self.parser.state {
            ParserState::Ground => match byte {
                0x1b => self.parser.state = ParserState::Escape,
                0x08 => self.backspace(),
                b'\t' => self.tab(),
                b'\n' | 0x0b | 0x0c => self.line_feed(),
                b'\r' => {
                    self.cursor.column = 0;
                    self.wrap_pending = false
                }
                0x20..=0x7e => {
                    if self.utf8.remaining != 0 {
                        self.put_glyph(0xfffd);
                        self.utf8.reset()
                    }
                    self.put_glyph(byte as u32)
                }
                0x80..=0xff => match self.utf8.push(byte) {
                    Utf8Result::Pending => {}
                    Utf8Result::Glyph(glyph) => self.put_glyph(glyph),
                    Utf8Result::Invalid => self.put_glyph(0xfffd),
                },
                _ => {}
            },
            ParserState::Escape => self.escape(byte),
            ParserState::Csi => self.csi(byte),
            ParserState::Osc => match byte {
                0x07 => self.parser.state = ParserState::Ground,
                0x1b => self.parser.state = ParserState::OscEscape,
                _ => {}
            },
            ParserState::OscEscape => {
                self.parser.state = if byte == b'\\' {
                    ParserState::Ground
                } else {
                    ParserState::Osc
                }
            }
        }
    }

    fn escape(&mut self, byte: u8) {
        self.parser.state = ParserState::Ground;
        match byte {
            b'[' => self.parser.enter_csi(),
            b']' => self.parser.state = ParserState::Osc,
            b'7' => self.saved_cursor = self.cursor,
            b'8' => self.restore_cursor(),
            b'D' => self.line_feed(),
            b'E' => {
                self.cursor.column = 0;
                self.line_feed()
            }
            b'M' => self.reverse_index(),
            b'c' => self.reset(),
            _ => {}
        }
    }

    fn csi(&mut self, byte: u8) {
        match byte {
            b'0'..=b'9' => {
                let value = &mut self.parser.params[self.parser.param_index];
                *value = value
                    .saturating_mul(10)
                    .saturating_add((byte - b'0') as u16)
            }
            b';' => {
                if self.parser.param_index + 1 < self.parser.params.len() {
                    self.parser.param_index += 1
                }
            }
            b'?' if self.parser.param_index == 0 && self.parser.params[0] == 0 => {
                self.parser.private = true
            }
            0x40..=0x7e => {
                let count = self.parser.param_index + 1;
                let params = self.parser.params;
                let private = self.parser.private;
                self.parser.reset();
                self.dispatch_csi(byte, &params[..count], private)
            }
            0x1b => self.parser.state = ParserState::Escape,
            _ => {}
        }
    }

    fn dispatch_csi(&mut self, final_byte: u8, params: &[u16], private: bool) {
        if private {
            match final_byte {
                b'h' => self.set_private_modes(params, true),
                b'l' => self.set_private_modes(params, false),
                _ => {}
            }
            return;
        }

        let first = params.first().copied().unwrap_or(0);
        let amount = first.max(1) as usize;
        match final_byte {
            b'A' => self.move_row(-(amount as isize)),
            b'B' | b'e' => self.move_row(amount as isize),
            b'C' | b'a' => self.move_column(amount as isize),
            b'D' => self.move_column(-(amount as isize)),
            b'E' => {
                self.move_row(amount as isize);
                self.cursor.column = 0
            }
            b'F' => {
                self.move_row(-(amount as isize));
                self.cursor.column = 0
            }
            b'G' | b'`' => self.set_column(amount - 1),
            b'H' | b'f' => {
                let row = first.max(1) as usize - 1;
                let column = params.get(1).copied().unwrap_or(1).max(1) as usize - 1;
                self.set_cursor(row, column)
            }
            b'J' => self.erase_display(first),
            b'K' => self.erase_line(first),
            b'L' => self.insert_lines(amount),
            b'M' => self.delete_lines(amount),
            b'P' => self.delete_characters(amount),
            b'@' => self.insert_characters(amount),
            b'X' => self.erase_characters(amount),
            b'd' => self.set_row(amount - 1),
            b'h' if first == 4 => self.modes.insert = true,
            b'l' if first == 4 => self.modes.insert = false,
            b'm' => self.sgr(params),
            b'r' => self.set_scroll_region(params),
            b's' => self.saved_cursor = self.cursor,
            b'u' => self.restore_cursor(),
            _ => {}
        }
        self.wrap_pending = false
    }

    fn set_private_modes(&mut self, params: &[u16], enabled: bool) {
        for parameter in params {
            match parameter {
                1 => self.modes.application_cursor_keys = enabled,
                7 => self.modes.automatic_wrap = enabled,
                25 => self.cursor.visible = enabled,
                _ => {}
            }
        }
    }

    fn sgr(&mut self, params: &[u16]) {
        let mut index = 0;
        loop {
            let parameter = params.get(index).copied().unwrap_or(0);
            match parameter {
                0 => self.style = Cell::EMPTY,
                1 => self.style.attributes.insert(CellAttributes::BOLD),
                2 => self.style.attributes.insert(CellAttributes::DIM),
                4 | 21 => self.style.attributes.insert(CellAttributes::UNDERLINE),
                5 | 6 => self.style.attributes.insert(CellAttributes::BLINK),
                7 => self.style.attributes.insert(CellAttributes::INVERSE),
                8 => self.style.attributes.insert(CellAttributes::HIDDEN),
                22 => {
                    self.style.attributes.remove(CellAttributes::BOLD);
                    self.style.attributes.remove(CellAttributes::DIM)
                }
                24 => self.style.attributes.remove(CellAttributes::UNDERLINE),
                25 => self.style.attributes.remove(CellAttributes::BLINK),
                27 => self.style.attributes.remove(CellAttributes::INVERSE),
                28 => self.style.attributes.remove(CellAttributes::HIDDEN),
                30..=37 => {
                    self.style.foreground =
                        Color::ansi(parameter - 30, false).unwrap_or(Color::Default)
                }
                39 => self.style.foreground = Color::Default,
                40..=47 => {
                    self.style.background =
                        Color::ansi(parameter - 40, false).unwrap_or(Color::Default)
                }
                49 => self.style.background = Color::Default,
                90..=97 => {
                    self.style.foreground =
                        Color::ansi(parameter - 90, true).unwrap_or(Color::Default)
                }
                100..=107 => {
                    self.style.background =
                        Color::ansi(parameter - 100, true).unwrap_or(Color::Default)
                }
                38 | 48 if params.get(index + 1) == Some(&5) => {
                    if let Some(color) = params
                        .get(index + 2)
                        .and_then(|value| indexed_color(*value))
                    {
                        if parameter == 38 {
                            self.style.foreground = color
                        } else {
                            self.style.background = color
                        }
                    }
                    index += 2
                }
                _ => {}
            }
            index += 1;
            if index >= params.len() {
                break;
            }
        }
    }

    fn put_glyph(&mut self, glyph: u32) {
        if self.wrap_pending {
            self.cursor.column = 0;
            self.line_feed();
            self.wrap_pending = false
        }
        let row = self.cursor.row as usize;
        let column = self.cursor.column as usize;
        if self.modes.insert {
            let line = &mut self.cells[row];
            line.copy_within(column..COLUMNS.saturating_sub(1), column + 1)
        }
        let mut cell = self.style;
        cell.glyph = glyph;
        self.cells[row][column] = cell;
        self.dirty[row] = true;
        if column + 1 == COLUMNS {
            self.wrap_pending = self.modes.automatic_wrap
        } else {
            self.cursor.column += 1
        }
    }

    fn line_feed(&mut self) {
        let row = self.cursor.row as usize;
        if row == self.scroll_bottom {
            self.scroll_up(1)
        } else if row + 1 < ROWS {
            self.cursor.row += 1
        }
    }

    fn reverse_index(&mut self) {
        let row = self.cursor.row as usize;
        if row == self.scroll_top {
            self.scroll_down(1)
        } else if row > 0 {
            self.cursor.row -= 1
        }
    }

    fn scroll_up(&mut self, count: usize) {
        let height = self.scroll_bottom - self.scroll_top + 1;
        let count = count.min(height);
        for row in self.scroll_top..=self.scroll_bottom {
            let source = row + count;
            self.cells[row] = if source <= self.scroll_bottom {
                self.cells[source]
            } else {
                [Cell::EMPTY; COLUMNS]
            };
            self.dirty[row] = true
        }
    }

    fn scroll_down(&mut self, count: usize) {
        let height = self.scroll_bottom - self.scroll_top + 1;
        let count = count.min(height);
        for row in (self.scroll_top..=self.scroll_bottom).rev() {
            self.cells[row] = if row >= self.scroll_top + count {
                self.cells[row - count]
            } else {
                [Cell::EMPTY; COLUMNS]
            };
            self.dirty[row] = true
        }
    }

    fn insert_lines(&mut self, count: usize) {
        let row = self.cursor.row as usize;
        if row >= self.scroll_top && row <= self.scroll_bottom {
            let old_top = self.scroll_top;
            self.scroll_top = row;
            self.scroll_down(count);
            self.scroll_top = old_top
        }
    }

    fn delete_lines(&mut self, count: usize) {
        let row = self.cursor.row as usize;
        if row >= self.scroll_top && row <= self.scroll_bottom {
            let old_top = self.scroll_top;
            self.scroll_top = row;
            self.scroll_up(count);
            self.scroll_top = old_top
        }
    }

    fn insert_characters(&mut self, count: usize) {
        let row = self.cursor.row as usize;
        let column = self.cursor.column as usize;
        let count = count.min(COLUMNS - column);
        self.cells[row].copy_within(column..COLUMNS - count, column + count);
        self.cells[row][column..column + count].fill(Cell::EMPTY);
        self.dirty[row] = true
    }

    fn delete_characters(&mut self, count: usize) {
        let row = self.cursor.row as usize;
        let column = self.cursor.column as usize;
        let count = count.min(COLUMNS - column);
        self.cells[row].copy_within(column + count..COLUMNS, column);
        self.cells[row][COLUMNS - count..].fill(Cell::EMPTY);
        self.dirty[row] = true
    }

    fn erase_characters(&mut self, count: usize) {
        let row = self.cursor.row as usize;
        let column = self.cursor.column as usize;
        let end = column.saturating_add(count).min(COLUMNS);
        self.cells[row][column..end].fill(Cell::EMPTY);
        self.dirty[row] = true
    }

    fn erase_display(&mut self, mode: u16) {
        let row = self.cursor.row as usize;
        let column = self.cursor.column as usize;
        match mode {
            0 => {
                self.cells[row][column..].fill(Cell::EMPTY);
                self.dirty[row] = true;
                for target in row + 1..ROWS {
                    self.cells[target].fill(Cell::EMPTY);
                    self.dirty[target] = true
                }
            }
            1 => {
                for target in 0..row {
                    self.cells[target].fill(Cell::EMPTY);
                    self.dirty[target] = true
                }
                self.cells[row][..=column].fill(Cell::EMPTY);
                self.dirty[row] = true
            }
            2 | 3 => {
                self.cells = [[Cell::EMPTY; COLUMNS]; ROWS];
                self.dirty.fill(true)
            }
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let row = self.cursor.row as usize;
        let column = self.cursor.column as usize;
        match mode {
            0 => self.cells[row][column..].fill(Cell::EMPTY),
            1 => self.cells[row][..=column].fill(Cell::EMPTY),
            2 => self.cells[row].fill(Cell::EMPTY),
            _ => return,
        }
        self.dirty[row] = true
    }

    fn set_scroll_region(&mut self, params: &[u16]) {
        let top = params.first().copied().unwrap_or(1).max(1) as usize - 1;
        let bottom = params.get(1).copied().unwrap_or(ROWS as u16).max(1) as usize - 1;
        if top < bottom && bottom < ROWS {
            self.scroll_top = top;
            self.scroll_bottom = bottom;
            self.set_cursor(0, 0)
        }
    }

    fn backspace(&mut self) {
        self.cursor.column = self.cursor.column.saturating_sub(1);
        self.wrap_pending = false
    }

    fn tab(&mut self) {
        let next = (self.cursor.column as usize / 8 + 1) * 8;
        self.cursor.column = next.min(COLUMNS - 1) as u16;
        self.wrap_pending = false
    }

    fn restore_cursor(&mut self) {
        self.cursor = self.saved_cursor;
        self.set_cursor(self.cursor.row as usize, self.cursor.column as usize)
    }

    fn set_cursor(&mut self, row: usize, column: usize) {
        self.cursor.row = row.min(ROWS - 1) as u16;
        self.cursor.column = column.min(COLUMNS - 1) as u16
    }

    fn set_row(&mut self, row: usize) {
        self.cursor.row = row.min(ROWS - 1) as u16
    }

    fn set_column(&mut self, column: usize) {
        self.cursor.column = column.min(COLUMNS - 1) as u16
    }

    fn move_row(&mut self, amount: isize) {
        let row = self.cursor.row as isize + amount;
        self.cursor.row = row.clamp(0, ROWS as isize - 1) as u16
    }

    fn move_column(&mut self, amount: isize) {
        let column = self.cursor.column as isize + amount;
        self.cursor.column = column.clamp(0, COLUMNS as isize - 1) as u16
    }
}

fn indexed_color(value: u16) -> Option<Color> {
    match value {
        0..=7 => Color::ansi(value, false),
        8..=15 => Color::ansi(value - 8, true),
        _ => None,
    }
}
