//! Per-row UAX #9 analysis: flatten cells to UTF-8 and map visual runs.
//!
// Wired into the frame in Task 3; the tests below are the only callers until
// then.

use std::ops::Range;

use unicode_bidi::{BidiInfo, Level};

use crate::config::BaseDirection;
use crate::term::RowSnapshot;

/// A row flattened to UTF-8 with byte-to-cell and cell-to-byte maps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowText {
    pub text: String,
    /// Byte offset of each cell's first byte; `len == cells + 1`.
    pub cell_starts: Vec<usize>,
    /// Cell index for every byte in `text`; `len == text.len()`.
    pub byte_cells: Vec<usize>,
    /// Grid width of each cell (0 = wide-character continuation).
    pub widths: Vec<u8>,
}

impl RowText {
    pub fn flatten(row: &RowSnapshot) -> Self {
        let mut text = String::new();
        let mut cell_starts = Vec::with_capacity(row.cells.len() + 1);
        let mut widths = Vec::with_capacity(row.cells.len());
        let mut byte_cells = Vec::new();
        for (index, cell) in row.cells.iter().enumerate() {
            cell_starts.push(text.len());
            widths.push(cell.width);
            if cell.width == 0 {
                continue;
            }
            for _ in 0..cell.text.len() {
                byte_cells.push(index);
            }
            text.push_str(&cell.text);
        }
        cell_starts.push(text.len());
        Self {
            text,
            cell_starts,
            byte_cells,
            widths,
        }
    }

    /// Cell containing `byte`; an out-of-range byte clamps to the last cell.
    /// Continuation cells own no bytes, so in-range lookups never return a
    /// width-0 cell.
    pub fn cell_of_byte(&self, byte: usize) -> usize {
        if self.widths.is_empty() {
            return 0;
        }
        self.byte_cells
            .get(byte)
            .copied()
            .unwrap_or_else(|| self.widths.len() - 1)
    }
}

/// A contiguous run of cells with one bidi level, in visual order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualRun {
    pub cells: Range<usize>,
    pub slots: Range<usize>,
    pub level: u8,
    pub rtl: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BidiRow {
    pub levels: Vec<u8>,
    pub runs: Vec<VisualRun>,
}

/// Resolves UAX #9 levels for one row and returns its runs in visual order.
pub fn analyze(row: &RowSnapshot, base: BaseDirection) -> BidiRow {
    let text = RowText::flatten(row);
    let cols = row.cells.len();
    if cols == 0 || text.text.is_empty() {
        let runs = if cols == 0 {
            Vec::new()
        } else {
            vec![VisualRun {
                cells: 0..cols,
                slots: 0..cols,
                level: 0,
                rtl: false,
            }]
        };
        return BidiRow {
            levels: vec![0; cols],
            runs,
        };
    }

    let base_level = match base {
        BaseDirection::Ltr => Some(Level::ltr()),
        BaseDirection::Rtl => Some(Level::rtl()),
        BaseDirection::Auto => None,
    };
    let info = BidiInfo::new(&text.text, base_level);
    let Some(paragraph) = info.paragraphs.first() else {
        return BidiRow {
            levels: vec![0; cols],
            runs: Vec::new(),
        };
    };
    let (byte_levels, byte_runs) = info.visual_runs(paragraph, 0..text.text.len());

    let mut levels = vec![0u8; cols];
    for cell in 0..cols {
        levels[cell] = if row.cells[cell].width == 0 && cell > 0 {
            levels[cell - 1]
        } else {
            let byte = text.cell_starts[cell].min(text.text.len().saturating_sub(1));
            byte_levels
                .get(byte)
                .map(|level| level.number())
                .unwrap_or(0)
        };
    }

    let mut runs = Vec::with_capacity(byte_runs.len());
    let mut slot = 0usize;
    for byte_run in byte_runs {
        if byte_run.start >= byte_run.end || slot >= cols {
            continue;
        }
        let first = text.cell_of_byte(byte_run.start);
        let mut last = text.cell_of_byte(byte_run.end - 1);
        last += usize::from(row.cells[last].width.max(1) - 1);
        let last = last.min(cols - 1);
        let span = last.saturating_sub(first) + 1;
        let slot_end = (slot + span).min(cols);
        let level = levels[first];
        runs.push(VisualRun {
            cells: first..last + 1,
            slots: slot..slot_end,
            level,
            rtl: level % 2 == 1,
        });
        slot = slot_end;
    }

    BidiRow { levels, runs }
}

// Consumed by the renderer's invisible-cell check in Task 6.
#[allow(dead_code)]
pub fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{Cell, Style};
    use crate::visual::VisualMap;
    use unicode_width::UnicodeWidthStr;

    fn cell(text: &str) -> Cell {
        let width = UnicodeWidthStr::width(text).clamp(1, 2) as u8;
        Cell {
            text: text.to_string(),
            width,
            style: Style::default(),
        }
    }

    fn spacer() -> Cell {
        Cell {
            text: String::new(),
            width: 0,
            style: Style::default(),
        }
    }

    fn row(texts: &[&str]) -> RowSnapshot {
        RowSnapshot {
            cells: texts.iter().map(|text| cell(text)).collect(),
            wrapped: false,
        }
    }

    #[test]
    fn flatten_records_byte_to_cell_offsets() {
        let row = row(&["a", "م"]);
        let text = RowText::flatten(&row);
        assert_eq!(text.text, "aم");
        assert_eq!(text.cell_starts, vec![0, 1, 3]);
        assert_eq!(text.byte_cells, vec![0, 1, 1]);
        assert_eq!(text.cell_of_byte(0), 0);
        assert_eq!(text.cell_of_byte(1), 1);
        assert_eq!(text.cell_of_byte(2), 1);
    }

    #[test]
    fn combining_mark_bytes_belong_to_the_base_cell() {
        let row = row(&["a\u{064E}"]);
        let text = RowText::flatten(&row);
        assert_eq!(text.text, "a\u{064E}");
        assert_eq!(text.cell_starts, vec![0, 3]);
        assert_eq!(text.byte_cells, vec![0, 0, 0]);
        assert_eq!(text.cell_of_byte(2), 0);
    }

    #[test]
    fn wide_char_continuation_shares_the_base_cell() {
        let row = RowSnapshot {
            cells: vec![cell("中"), spacer()],
            wrapped: false,
        };
        let text = RowText::flatten(&row);
        assert_eq!(text.text, "中");
        assert_eq!(text.cell_starts, vec![0, 3, 3]);
        assert_eq!(text.byte_cells, vec![0, 0, 0]);
        let bidi = analyze(&row, BaseDirection::Ltr);
        assert_eq!(bidi.runs.len(), 1);
        assert_eq!(bidi.runs[0].cells, 0..2);
        assert_eq!(bidi.runs[0].slots, 0..2);
    }

    #[test]
    fn wide_glyph_base_stays_left_of_its_continuation_in_an_rtl_run() {
        let row = RowSnapshot {
            cells: vec![cell("م"), cell("😀"), spacer(), cell("ر")],
            wrapped: false,
        };
        let bidi = analyze(&row, BaseDirection::Ltr);
        assert_eq!(bidi.levels, vec![1, 1, 1, 1]);
        let map = VisualMap::from_bidi(&row, BaseDirection::Ltr);
        assert_eq!(map.visual_of(0), 3);
        assert_eq!(map.visual_of(1), 1);
        assert_eq!(map.visual_of(2), 2);
        assert_eq!(map.visual_of(3), 0);
        for logical in 0..row.cells.len() {
            assert_eq!(map.logical_of(map.visual_of(logical)), logical);
        }
    }

    #[test]
    fn ltr_row_produces_an_identity_map() {
        let row = row(&["a", "b", "c"]);
        let bidi = analyze(&row, BaseDirection::Ltr);
        assert_eq!(bidi.levels, vec![0, 0, 0]);
        let map = VisualMap::from_bidi(&row, BaseDirection::Ltr);
        assert_eq!(map.visual_of(0), 0);
        assert_eq!(map.visual_of(2), 2);
        assert_eq!(map.logical_of(2), 2);
    }

    #[test]
    fn rtl_row_reverses_visual_slots() {
        let row = row(&["م", "ر", "ح"]);
        let bidi = analyze(&row, BaseDirection::Ltr);
        assert_eq!(bidi.levels, vec![1, 1, 1]);
        let map = VisualMap::from_bidi(&row, BaseDirection::Ltr);
        assert_eq!(map.visual_of(0), 2);
        assert_eq!(map.visual_of(1), 1);
        assert_eq!(map.visual_of(2), 0);
    }

    #[test]
    fn rtl_base_direction_raises_latin_to_level_two() {
        let row = row(&["a", "b", "c"]);
        let bidi = analyze(&row, BaseDirection::Rtl);
        assert_eq!(bidi.levels, vec![2, 2, 2]);
    }

    #[test]
    fn auto_base_direction_follows_the_first_strong_character() {
        let row = row(&["م", "ر", "ح"]);
        let bidi = analyze(&row, BaseDirection::Auto);
        assert_eq!(bidi.levels, vec![1, 1, 1]);
    }

    #[test]
    fn mixed_row_keeps_latin_left_and_reverses_the_arabic_run() {
        let row = row(&["a", "b", "c", "م", "ر", "ح"]);
        let bidi = analyze(&row, BaseDirection::Ltr);
        assert_eq!(bidi.levels, vec![0, 0, 0, 1, 1, 1]);
        let map = VisualMap::from_bidi(&row, BaseDirection::Ltr);
        assert_eq!(map.visual_of(0), 0);
        assert_eq!(map.visual_of(3), 5);
        assert_eq!(map.visual_of(4), 4);
        assert_eq!(map.visual_of(5), 3);
        assert_eq!(map.logical_of(5), 3);
    }

    #[test]
    fn bidi_controls_stay_in_the_text_and_consume_one_slot() {
        let row = row(&["\u{200F}", "a"]);
        let text = RowText::flatten(&row);
        assert_eq!(text.text, "\u{200F}a");
        assert_eq!(text.cell_starts, vec![0, 3, 4]);
        let map = VisualMap::from_bidi(&row, BaseDirection::Ltr);
        assert_eq!(map.visual_of(0), 0);
        assert_eq!(map.visual_of(1), 1);
    }

    #[test]
    fn bidi_controls_are_recognized() {
        for ch in [
            '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
            '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ] {
            assert!(is_bidi_control(ch), "{ch:?} should be a bidi control");
        }
        for ch in ['a', 'م', ' ', '\u{200B}'] {
            assert!(!is_bidi_control(ch), "{ch:?} is not a bidi control");
        }
    }

    #[test]
    fn right_to_left_override_reverses_latin_text() {
        let row = row(&["\u{202E}", "a", "b", "c", "\u{202C}"]);
        let map = VisualMap::from_bidi(&row, BaseDirection::Ltr);
        assert_eq!(map.visual_of(1), 3);
        assert_eq!(map.visual_of(2), 2);
        assert_eq!(map.visual_of(3), 1);
    }

    #[test]
    fn mapping_round_trips_every_cell() {
        let row = row(&["a", "b", " ", "م", "ر", "ح", " ", "1"]);
        let map = VisualMap::from_bidi(&row, BaseDirection::Ltr);
        for logical in 0..row.cells.len() {
            assert_eq!(map.logical_of(map.visual_of(logical)), logical);
            assert_eq!(map.visual_of(map.logical_of(logical)), logical);
        }
    }
}
