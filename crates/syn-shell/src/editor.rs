use crate::{Error, MAX_LINE_BYTES, Text};

pub const DEFAULT_HISTORY_CAPACITY: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Character(char),
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    ShiftLeft,
    ShiftRight,
    ShiftUp,
    ShiftDown,
    PageUp,
    PageDown,
    ShiftHome,
    ShiftEnd,
    Backspace,
    Delete,
    HistoryPrevious,
    HistoryNext,
    Tab,
    Enter,
    Escape,
    Save,
    SaveExit,
    DiscardExit,
    Cancel,
    Copy,
    Cut,
    Paste,
    Resize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorAction {
    None,
    Redraw,
    Complete,
    Submit(Text<MAX_LINE_BYTES>),
    Cancel,
}

/// UTF-8 line editor with cursor movement and bounded command history.
pub struct LineEditor<const HISTORY: usize = DEFAULT_HISTORY_CAPACITY> {
    bytes: [u8; MAX_LINE_BYTES],
    len: usize,
    cursor: usize,
    history: [Option<Text<MAX_LINE_BYTES>>; HISTORY],
    history_next: usize,
    history_count: usize,
    history_offset: Option<usize>,
    draft: Text<MAX_LINE_BYTES>,
}

impl<const HISTORY: usize> LineEditor<HISTORY> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; MAX_LINE_BYTES],
            len: 0,
            cursor: 0,
            history: [None; HISTORY],
            history_next: 0,
            history_count: 0,
            history_offset: None,
            draft: Text::empty(),
        }
    }

    pub fn line(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len])
            .expect("line editor UTF-8 invariant")
    }

    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.cursor = 0;
        self.history_offset = None;
        self.draft.clear()
    }

    pub fn replace_line(&mut self, line: &str) -> Result<(), Error> {
        if line.len() > MAX_LINE_BYTES {
            return Err(Error::LineTooLong)
        }
        let source = line.as_bytes();
        let mut index = 0;
        while index < source.len() {
            self.bytes[index] = source[index];
            index += 1;
        }
        self.len = line.len();
        self.cursor = self.len;
        self.history_offset = None;
        Ok(())
    }

    pub fn replace_command(&mut self, command: &str) -> Result<(), Error> {
        self.len = 0;
        for byte in command.bytes() {
            let byte = if byte == b'-' { b' ' } else { byte };
            if self.len == MAX_LINE_BYTES {
                return Err(Error::LineTooLong)
            }
            self.bytes[self.len] = byte;
            self.len += 1;
        }
        self.cursor = self.len;
        self.history_offset = None;
        Ok(())
    }

    pub fn handle(&mut self, key: Key) -> Result<EditorAction, Error> {
        match key {
            Key::Character(value) => {
                self.insert(value)?;
                Ok(EditorAction::Redraw)
            }
            Key::Left => {
                if self.cursor > 0 {
                    self.cursor = previous_boundary(&self.bytes, self.cursor)
                }
                Ok(EditorAction::Redraw)
            }
            Key::Right => {
                if self.cursor < self.len {
                    self.cursor = next_boundary(&self.bytes, self.cursor, self.len)
                }
                Ok(EditorAction::Redraw)
            }
            Key::Up => {
                self.history_previous()?;
                Ok(EditorAction::Redraw)
            }
            Key::Down => {
                self.history_next()?;
                Ok(EditorAction::Redraw)
            }
            Key::ShiftLeft | Key::ShiftRight | Key::ShiftUp | Key::ShiftDown | Key::PageUp
            | Key::PageDown
            | Key::ShiftHome
            | Key::ShiftEnd
            | Key::Escape
            | Key::Save
            | Key::SaveExit
            | Key::DiscardExit
            | Key::Copy
            | Key::Cut
            | Key::Paste
            | Key::Resize => {
                Ok(EditorAction::Redraw)
            }
            Key::Home => {
                self.cursor = 0;
                Ok(EditorAction::Redraw)
            }
            Key::End => {
                self.cursor = self.len;
                Ok(EditorAction::Redraw)
            }
            Key::Backspace => {
                if self.cursor > 0 {
                    let start = previous_boundary(&self.bytes, self.cursor);
                    self.remove(start, self.cursor)
                }
                Ok(EditorAction::Redraw)
            }
            Key::Delete => {
                if self.cursor < self.len {
                    let end = next_boundary(&self.bytes, self.cursor, self.len);
                    self.remove(self.cursor, end)
                }
                Ok(EditorAction::Redraw)
            }
            Key::HistoryPrevious => {
                self.history_previous()?;
                Ok(EditorAction::Redraw)
            }
            Key::HistoryNext => {
                self.history_next()?;
                Ok(EditorAction::Redraw)
            }
            Key::Tab => Ok(EditorAction::Complete),
            Key::Enter => {
                let line = Text::new(self.line())?;
                if !line.as_str().trim().is_empty() {
                    self.remember(line)
                }
                self.clear();
                Ok(EditorAction::Submit(line))
            }
            Key::Cancel => {
                self.clear();
                Ok(EditorAction::Cancel)
            }
        }
    }

    fn insert(&mut self, value: char) -> Result<(), Error> {
        let mut encoded = [0; 4];
        let encoded = value.encode_utf8(&mut encoded).as_bytes();
        if self.len + encoded.len() > MAX_LINE_BYTES {
            return Err(Error::LineTooLong)
        }
        self.bytes
            .copy_within(self.cursor..self.len, self.cursor + encoded.len());
        self.bytes[self.cursor..self.cursor + encoded.len()].copy_from_slice(encoded);
        self.cursor += encoded.len();
        self.len += encoded.len();
        self.history_offset = None;
        Ok(())
    }

    fn remove(&mut self, start: usize, end: usize) {
        self.bytes.copy_within(end..self.len, start);
        self.len -= end - start;
        self.cursor = start;
        self.history_offset = None
    }

    fn remember(&mut self, line: Text<MAX_LINE_BYTES>) {
        if HISTORY == 0 {
            return
        }
        let newest = self.history_count.checked_sub(1).and_then(|_| {
            let index = (self.history_next + HISTORY - 1) % HISTORY;
            self.history[index]
        });
        if newest == Some(line) {
            return
        }
        self.history[self.history_next] = Some(line);
        self.history_next = (self.history_next + 1) % HISTORY;
        self.history_count = core::cmp::min(self.history_count + 1, HISTORY)
    }

    fn history_previous(&mut self) -> Result<(), Error> {
        if HISTORY == 0 || self.history_count == 0 {
            return Ok(())
        }
        let offset = match self.history_offset {
            None => {
                self.draft = Text::new(self.line())?;
                0
            }
            Some(offset) => core::cmp::min(offset + 1, self.history_count - 1),
        };
        self.history_offset = Some(offset);
        let index = (self.history_next + HISTORY - 1 - offset) % HISTORY;
        let line = self.history[index].expect("history occupancy invariant");
        self.load(line);
        Ok(())
    }

    fn history_next(&mut self) -> Result<(), Error> {
        let Some(offset) = self.history_offset else {
            return Ok(())
        };
        if offset == 0 {
            self.history_offset = None;
            self.load(self.draft)
        } else {
            let next = offset - 1;
            self.history_offset = Some(next);
            let index = (self.history_next + HISTORY - 1 - next) % HISTORY;
            self.load(self.history[index].expect("history occupancy invariant"))
        }
        Ok(())
    }

    fn load(&mut self, line: Text<MAX_LINE_BYTES>) {
        self.bytes[..line.len()].copy_from_slice(&line.bytes[..line.len()]);
        self.len = line.len();
        self.cursor = self.len
    }
}

impl<const HISTORY: usize> Default for LineEditor<HISTORY> {
    fn default() -> Self {
        Self::new()
    }
}

fn previous_boundary(bytes: &[u8], mut cursor: usize) -> usize {
    cursor -= 1;
    while cursor > 0 && bytes[cursor] & 0b1100_0000 == 0b1000_0000 {
        cursor -= 1
    }
    cursor
}

fn next_boundary(bytes: &[u8], mut cursor: usize, len: usize) -> usize {
    cursor += 1;
    while cursor < len && bytes[cursor] & 0b1100_0000 == 0b1000_0000 {
        cursor += 1
    }
    cursor
}
