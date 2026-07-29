use crate::{Cell, Terminal};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(C)]
pub struct GpuCell {
    pub glyph: u32,
    pub foreground: u32,
    pub background: u32,
    pub attributes: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UploadRange {
    pub first_cell: u32,
    pub cell_count: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UploadError {
    BufferTooSmall,
    InvalidRow,
}

/// WebAssembly-friendly adapter for a WebGPU instance buffer.
pub struct WebGpuFrontend;

impl WebGpuFrontend {
    pub fn encode_row<const COLUMNS: usize, const ROWS: usize>(
        terminal: &mut Terminal<COLUMNS, ROWS>,
        row: usize,
        output: &mut [GpuCell],
    ) -> Result<Option<UploadRange>, UploadError> {
        if row >= ROWS {
            return Err(UploadError::InvalidRow);
        }
        if !terminal.row_is_dirty(row) {
            return Ok(None);
        }
        if output.len() < COLUMNS {
            return Err(UploadError::BufferTooSmall);
        }
        let cells = terminal.row(row).ok_or(UploadError::InvalidRow)?;
        for (destination, source) in output.iter_mut().zip(cells.iter()) {
            *destination = encode_cell(*source)
        }
        terminal.mark_row_clean(row);
        Ok(Some(UploadRange {
            first_cell: (row * COLUMNS) as u32,
            cell_count: COLUMNS as u32,
        }))
    }
}

fn encode_cell(cell: Cell) -> GpuCell {
    GpuCell {
        glyph: cell.glyph,
        foreground: cell.foreground as u32,
        background: cell.background as u32,
        attributes: cell.attributes.bits() as u32,
    }
}
