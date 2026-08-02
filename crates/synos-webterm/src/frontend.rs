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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploads_dirty_rows_once_and_preserves_cell_attributes() {
        let mut terminal = Terminal::<2, 1>::new().unwrap();
        terminal.write(b"A");
        let mut cells = [GpuCell {
            glyph: 0,
            foreground: 0,
            background: 0,
            attributes: 0,
        }; 2];

        let range = WebGpuFrontend::encode_row(&mut terminal, 0, &mut cells)
            .unwrap()
            .expect("dirty row uploads");
        assert_eq!(range, UploadRange { first_cell: 0, cell_count: 2 });
        assert_eq!(cells[0].glyph, 'A' as u32);
        assert!(!terminal.row_is_dirty(0));
        assert_eq!(WebGpuFrontend::encode_row(&mut terminal, 0, &mut cells), Ok(None));
        terminal.mark_all_dirty();
        assert_eq!(
            WebGpuFrontend::encode_row(&mut terminal, 0, &mut cells[..1]),
            Err(UploadError::BufferTooSmall)
        );
        assert_eq!(
            WebGpuFrontend::encode_row(&mut terminal, 1, &mut cells),
            Err(UploadError::InvalidRow)
        );
    }
}
