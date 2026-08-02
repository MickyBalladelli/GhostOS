use core::fmt::Write;

use crate::{Error, Text};

pub const MAX_EDITOR_BYTES: usize = 16 * 1024;
pub const MAX_EDITOR_NAME_BYTES: usize = 192;
pub const MAX_EDITOR_STATUS_BYTES: usize = 512;

use super::editor::Key;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditorMode {
    Insert,
    Command,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileEditorAction {
    None,
    Redraw,
    Save,
    SaveExit,
    DiscardExit,
    PromptDiscard,
}

/// Heap-free, UTF-8 full-screen editor buffer.
///
/// The editor keeps cursor and selection positions as byte offsets. All
/// mutations happen on UTF-8 boundaries, so filesystem writes stay valid text.
pub struct FileEditor<const CAPACITY: usize = MAX_EDITOR_BYTES> {
    bytes: [u8; CAPACITY],
    len: usize,
    cursor: usize,
    anchor: Option<usize>,
    scroll_row: usize,
    scroll_column: usize,
    name: Text<MAX_EDITOR_NAME_BYTES>,
    version: u32,
    saved_checksum: u64,
    mode: EditorMode,
}

impl<const CAPACITY: usize> FileEditor<CAPACITY> {
    pub fn new(name: &str, version: u32, contents: &[u8]) -> Result<Self, Error> {
        if contents.len() > CAPACITY || core::str::from_utf8(contents).is_err() {
            return Err(Error::InvalidValue)
        }
        let mut bytes = [0; CAPACITY];
        bytes[..contents.len()].copy_from_slice(contents);
        Ok(Self {
            bytes,
            len: contents.len(),
            cursor: 0,
            anchor: None,
            scroll_row: 0,
            scroll_column: 0,
            name: Text::new(name)?,
            version,
            saved_checksum: checksum(contents),
            mode: EditorMode::Insert,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    pub const fn version(&self) -> u32 {
        self.version
    }

    pub const fn mode(&self) -> EditorMode {
        self.mode
    }

    pub fn is_dirty(&self) -> bool {
        checksum(&self.bytes[..self.len]) != self.saved_checksum
    }

    pub fn selected(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        if anchor == self.cursor {
            None
        } else if anchor < self.cursor {
            Some((anchor, self.cursor))
        } else {
            Some((self.cursor, anchor))
        }
    }

    pub fn mark_saved(&mut self, version: u32) {
        self.version = version;
        self.saved_checksum = checksum(&self.bytes[..self.len])
    }

    pub fn confirm_discard(&mut self, discard: bool) -> FileEditorAction {
        if discard {
            FileEditorAction::DiscardExit
        } else {
            FileEditorAction::Redraw
        }
    }

    pub fn handle(&mut self, key: Key) -> Result<FileEditorAction, Error> {
        let action = match key {
            Key::Save => FileEditorAction::Save,
            Key::SaveExit => FileEditorAction::SaveExit,
            Key::DiscardExit | Key::Cancel => self.request_discard(),
            Key::Escape => {
                if self.mode == EditorMode::Insert {
                    self.mode = EditorMode::Command;
                } else {
                    self.anchor = None;
                }
                FileEditorAction::Redraw
            }
            Key::Character('i') if self.mode == EditorMode::Command => {
                self.mode = EditorMode::Insert;
                FileEditorAction::Redraw
            }
            Key::Character('a') if self.mode == EditorMode::Command => {
                self.move_right(false);
                self.mode = EditorMode::Insert;
                FileEditorAction::Redraw
            }
            Key::Character('s') if self.mode == EditorMode::Command => FileEditorAction::Save,
            Key::Character('e') if self.mode == EditorMode::Command => FileEditorAction::SaveExit,
            Key::Character('q') if self.mode == EditorMode::Command => self.request_discard(),
            Key::Character(value) if self.mode == EditorMode::Insert => {
                self.insert_char(value)?;
                FileEditorAction::Redraw
            }
            Key::Tab if self.mode == EditorMode::Insert => {
                self.insert_char('\t')?;
                FileEditorAction::Redraw
            }
            Key::Enter if self.mode == EditorMode::Insert => {
                self.insert_bytes(b"\n")?;
                FileEditorAction::Redraw
            }
            Key::Left => {
                self.move_left(false);
                FileEditorAction::Redraw
            }
            Key::Right => {
                self.move_right(false);
                FileEditorAction::Redraw
            }
            Key::Up => {
                self.move_vertical(-1, false);
                FileEditorAction::Redraw
            }
            Key::Down => {
                self.move_vertical(1, false);
                FileEditorAction::Redraw
            }
            Key::Home => {
                self.move_home(false);
                FileEditorAction::Redraw
            }
            Key::End => {
                self.move_end(false);
                FileEditorAction::Redraw
            }
            Key::ShiftLeft => {
                self.move_left(true);
                FileEditorAction::Redraw
            }
            Key::ShiftRight => {
                self.move_right(true);
                FileEditorAction::Redraw
            }
            Key::ShiftUp => {
                self.move_vertical(-1, true);
                FileEditorAction::Redraw
            }
            Key::ShiftDown => {
                self.move_vertical(1, true);
                FileEditorAction::Redraw
            }
            Key::PageUp => {
                self.move_vertical(-10, false);
                FileEditorAction::Redraw
            }
            Key::PageDown => {
                self.move_vertical(10, false);
                FileEditorAction::Redraw
            }
            Key::Backspace if self.mode == EditorMode::Insert => {
                self.backspace();
                FileEditorAction::Redraw
            }
            Key::Delete if self.mode == EditorMode::Insert => {
                self.delete_forward();
                FileEditorAction::Redraw
            }
            Key::HistoryPrevious | Key::HistoryNext => {
                FileEditorAction::None
            }
            Key::Character(_) => FileEditorAction::None,
            Key::Backspace | Key::Delete | Key::Tab | Key::Enter => FileEditorAction::None,
        };
        Ok(action)
    }

    pub fn render<const OUTPUT: usize>(
        &mut self,
        columns: usize,
        rows: usize,
    ) -> Result<Text<OUTPUT>, Error> {
        if columns == 0 || rows < 2 {
            return Err(Error::InvalidValue)
        }
        let content_rows = rows - 1;
        self.ensure_visible(columns, content_rows);

        let mut output = Text::empty();
        output.push_str("\x1b[?25l")?;
        for row in 0..content_rows {
            write!(output, "\x1b[{};1H\x1b[2K", row + 1).map_err(|_| Error::Capacity)?;
            self.render_line(&mut output, self.scroll_row + row, columns)?;
        }
        write!(output, "\x1b[{};1H\x1b[2K\x1b[7m", rows)
            .map_err(|_| Error::Capacity)?;
        let mut status = Text::<MAX_EDITOR_STATUS_BYTES>::empty();
        write!(
            status,
            " EDIT {}  SIZE:{}  VERSION:{}  LINES:{}  CURSOR:{}:{}  MODE:{}{}",
            self.name.as_str(),
            self.len,
            self.version,
            self.line_count(),
            self.line_number(),
            self.column_number(),
            match self.mode {
                EditorMode::Insert => "INSERT",
                EditorMode::Command => "COMMAND",
            },
            if self.is_dirty() { "  MODIFIED" } else { "" },
        )
        .map_err(|_| Error::Capacity)?;
        if self.selected().is_some() {
            status.push_str("  SELECTED")?;
        }
        let mut used = 0;
        for character in status.as_str().chars() {
            if used == columns {
                break
            }
            output.push_char(character)?;
            used += 1;
        }
        for _ in used..columns {
            output.push_char(' ')?;
        }
        let cursor_row = self.cursor_line().saturating_sub(self.scroll_row).min(content_rows - 1);
        let cursor_column = self.cursor_column().saturating_sub(self.scroll_column).min(columns - 1);
        write!(
            output,
            "\x1b[{};{}H\x1b[0m\x1b[?25h",
            cursor_row + 1,
            cursor_column + 1,
        )
        .map_err(|_| Error::Capacity)?;
        Ok(output)
    }

    fn request_discard(&self) -> FileEditorAction {
        if self.is_dirty() {
            FileEditorAction::PromptDiscard
        } else {
            FileEditorAction::DiscardExit
        }
    }

    fn insert_char(&mut self, value: char) -> Result<(), Error> {
        let mut encoded = [0; 4];
        let bytes = value.encode_utf8(&mut encoded).as_bytes();
        self.insert_bytes(bytes)
    }

    fn insert_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        if let Some((start, end)) = self.selected() {
            self.remove(start, end);
        }
        let end = self
            .len
            .checked_add(bytes.len())
            .ok_or(Error::Capacity)?;
        if end > CAPACITY {
            return Err(Error::Capacity)
        }
        self.bytes.copy_within(self.cursor..self.len, self.cursor + bytes.len());
        self.bytes[self.cursor..self.cursor + bytes.len()].copy_from_slice(bytes);
        self.cursor = self.cursor.saturating_add(bytes.len());
        self.len = end;
        self.anchor = None;
        Ok(())
    }

    fn remove(&mut self, start: usize, end: usize) {
        self.bytes.copy_within(end..self.len, start);
        self.len -= end - start;
        self.cursor = start;
        self.anchor = None;
    }

    fn backspace(&mut self) {
        if let Some((start, end)) = self.selected() {
            self.remove(start, end);
        } else if self.cursor > 0 {
            let start = previous_boundary(&self.bytes, self.cursor);
            self.remove(start, self.cursor);
        }
    }

    fn delete_forward(&mut self) {
        if let Some((start, end)) = self.selected() {
            self.remove(start, end);
        } else if self.cursor < self.len {
            self.remove(self.cursor, next_boundary(&self.bytes, self.cursor, self.len));
        }
    }

    fn begin_selection(&mut self) {
        if self.anchor.is_none() {
            self.anchor = Some(self.cursor);
        }
    }

    fn finish_movement(&mut self, selecting: bool) {
        if selecting {
            self.begin_selection();
        } else {
            self.anchor = None;
        }
    }

    fn move_left(&mut self, selecting: bool) {
        self.finish_movement(selecting);
        if self.cursor > 0 {
            self.cursor = previous_boundary(&self.bytes, self.cursor);
        }
    }

    fn move_right(&mut self, selecting: bool) {
        self.finish_movement(selecting);
        if self.cursor < self.len {
            self.cursor = next_boundary(&self.bytes, self.cursor, self.len);
        }
    }

    fn move_home(&mut self, selecting: bool) {
        self.finish_movement(selecting);
        self.cursor = self.line_start(self.cursor);
    }

    fn move_end(&mut self, selecting: bool) {
        self.finish_movement(selecting);
        self.cursor = self.line_end(self.cursor);
    }

    fn move_vertical(&mut self, amount: isize, selecting: bool) {
        self.finish_movement(selecting);
        let current_line = self.cursor_line();
        let target_line = (current_line as isize + amount).max(1) as usize;
        let column = self.cursor_column();
        let start = self.line_start_for_number(target_line);
        let end = self.line_end(start);
        self.cursor = offset_for_column(&self.bytes[start..end], column) + start;
    }

    fn line_start(&self, offset: usize) -> usize {
        let mut cursor = offset.min(self.len);
        while cursor > 0 && self.bytes[cursor - 1] != b'\n' {
            cursor -= 1;
        }
        cursor
    }

    fn line_end(&self, offset: usize) -> usize {
        let mut cursor = offset.min(self.len);
        while cursor < self.len && self.bytes[cursor] != b'\n' {
            cursor += 1;
        }
        cursor
    }

    fn line_start_for_number(&self, number: usize) -> usize {
        if number <= 1 {
            return 0
        }
        let mut line = 1;
        for index in 0..self.len {
            if self.bytes[index] == b'\n' {
                line += 1;
                if line == number {
                    return index + 1;
                }
            }
        }
        self.len
    }

    fn cursor_line(&self) -> usize {
        self.bytes[..self.cursor]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
    }

    fn cursor_column(&self) -> usize {
        display_width(&self.bytes[self.line_start(self.cursor)..self.cursor])
    }

    fn line_number(&self) -> usize {
        self.cursor_line() + 1
    }

    fn line_count(&self) -> usize {
        self.bytes[..self.len]
            .iter()
            .filter(|byte| **byte == b'\n')
            .count()
            + 1
    }

    fn column_number(&self) -> usize {
        self.cursor_column() + 1
    }

    fn ensure_visible(&mut self, columns: usize, rows: usize) {
        let line = self.cursor_line();
        if line < self.scroll_row {
            self.scroll_row = line;
        } else if line >= self.scroll_row + rows {
            self.scroll_row = line - rows + 1;
        }
        let column = self.cursor_column();
        if column < self.scroll_column {
            self.scroll_column = column;
        } else if column >= self.scroll_column + columns {
            self.scroll_column = column - columns + 1;
        }
    }

    fn render_line<const OUTPUT: usize>(
        &self,
        output: &mut Text<OUTPUT>,
        line_number: usize,
        columns: usize,
    ) -> Result<(), Error> {
        let start = self.line_start_for_number(line_number + 1);
        if start > self.len {
            return Ok(())
        }
        let end = self.line_end(start);
        let selected = self.selected();
        let mut column = 0;
        let mut offset = start;
        while offset < end && column < self.scroll_column + columns {
            let next = next_boundary(&self.bytes, offset, end);
            let character = core::str::from_utf8(&self.bytes[offset..next])
                .map_err(|_| Error::InvalidValue)?
                .chars()
                .next()
                .ok_or(Error::InvalidValue)?;
            let width = if character == '\t' { 4 } else { 1 };
            let selected_character = selected
                .is_some_and(|(selection_start, selection_end)| offset < selection_end && next > selection_start);
            if column + width > self.scroll_column && column < self.scroll_column + columns {
                if selected_character {
                    output.push_str("\x1b[7m")?;
                }
                for _ in 0..width {
                    if column >= self.scroll_column && column < self.scroll_column + columns {
                        output.push_char(if character == '\t' { ' ' } else { character })?;
                    }
                    column += 1;
                }
                if selected_character {
                    output.push_str("\x1b[27m")?;
                }
            } else {
                column += width;
            }
            offset = next;
        }
        Ok(())
    }
}

impl<const CAPACITY: usize> Default for FileEditor<CAPACITY> {
    fn default() -> Self {
        Self::new("", 0, &[]).expect("empty editor is valid")
    }
}

fn previous_boundary(bytes: &[u8], mut cursor: usize) -> usize {
    cursor -= 1;
    while cursor > 0 && bytes[cursor] & 0xc0 == 0x80 {
        cursor -= 1;
    }
    cursor
}

fn next_boundary(bytes: &[u8], mut cursor: usize, len: usize) -> usize {
    cursor += 1;
    while cursor < len && bytes[cursor] & 0xc0 == 0x80 {
        cursor += 1;
    }
    cursor
}

fn display_width(bytes: &[u8]) -> usize {
    core::str::from_utf8(bytes)
        .map(|text| text.chars().map(|character| if character == '\t' { 4 } else { 1 }).sum())
        .unwrap_or(bytes.len())
}

fn checksum(bytes: &[u8]) -> u64 {
    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    hash
}

fn offset_for_column(bytes: &[u8], column: usize) -> usize {
    let mut offset = 0;
    let mut current = 0;
    while offset < bytes.len() && current < column {
        let next = next_boundary(bytes, offset, bytes.len());
        let character = core::str::from_utf8(&bytes[offset..next])
            .ok()
            .and_then(|value| value.chars().next())
            .unwrap_or(' ');
        current += if character == '\t' { 4 } else { 1 };
        offset = next;
    }
    offset
}
