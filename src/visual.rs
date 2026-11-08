//! Per-row logical-to-visual mapping.

use crate::bidi::{self, VisualRun};
use crate::config::BaseDirection;
use crate::term::{Affinity, CursorState, RowSnapshot};

/// Maps logical cell columns to visual columns for one row.
///
/// Every caller goes through this type: `identity` is Phase 1 and `from_bidi`
/// is the Phase 2 UAX#9 replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualMap {
    cols: usize,
    levels: Vec<u8>,
    runs: Vec<VisualRun>,
    logical_to_visual: Vec<usize>,
    visual_to_logical: Vec<usize>,
    /// Whether the run order and levels came from UAX #9 analysis. Identity
    /// maps have no levels, so shaping falls back to the text's script.
    analyzed: bool,
}

impl VisualMap {
    pub fn identity(cols: usize) -> Self {
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
        Self {
            cols,
            levels: vec![0; cols],
            runs,
            logical_to_visual: (0..cols).collect(),
            visual_to_logical: (0..cols).collect(),
            analyzed: false,
        }
    }

    pub fn for_row(row: &RowSnapshot, bidi: &crate::config::BidiConfig) -> Self {
        if bidi.enabled {
            Self::from_bidi(row, bidi.base_direction)
        } else {
            Self::identity(row.cells.len())
        }
    }

    pub fn from_bidi(row: &RowSnapshot, base: BaseDirection) -> Self {
        let analysis = bidi::analyze(row, base);
        Self::build(row, analysis.levels, analysis.runs)
    }

    fn build(row: &RowSnapshot, levels: Vec<u8>, runs: Vec<VisualRun>) -> Self {
        let cols = levels.len();
        let mut logical_to_visual = vec![0; cols];
        let mut visual_to_logical = vec![0; cols];
        for run in &runs {
            debug_assert_eq!(run.cells.len(), run.slots.len());
            let mut logical = run.cells.start;
            let mut slot = if run.rtl {
                run.slots.end
            } else {
                run.slots.start
            };
            while logical < run.cells.end {
                // A wide glyph is one glyph over two cells: its base keeps the
                // left slot and its continuation the right slot, even in RTL.
                let width =
                    usize::from(row.cells[logical].width.max(1)).min(run.cells.end - logical);
                if run.rtl {
                    slot -= width;
                }
                for offset in 0..width {
                    let (cell, visual) = (logical + offset, slot + offset);
                    if cell < cols && visual < cols {
                        logical_to_visual[cell] = visual;
                        visual_to_logical[visual] = cell;
                    }
                }
                if !run.rtl {
                    slot += width;
                }
                logical += width;
            }
        }
        Self {
            cols,
            levels,
            runs,
            logical_to_visual,
            visual_to_logical,
            analyzed: true,
        }
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn levels(&self) -> &[u8] {
        &self.levels
    }

    pub fn runs(&self) -> &[VisualRun] {
        &self.runs
    }

    /// Whether the levels and runs came from bidi analysis rather than a
    /// logical-order identity mapping.
    pub fn is_bidi_analyzed(&self) -> bool {
        self.analyzed
    }

    /// Visual column of a logical cell. Clamps to the last cell.
    pub fn visual_of(&self, logical: usize) -> usize {
        self.logical_to_visual
            .get(logical)
            .copied()
            .unwrap_or_else(|| self.cols.saturating_sub(1))
    }

    /// Logical cell at a visual column. Clamps to the last cell.
    pub fn logical_of(&self, visual: usize) -> usize {
        self.visual_to_logical
            .get(visual)
            .copied()
            .unwrap_or_else(|| self.cols.saturating_sub(1))
    }

    /// Whether a logical cell has an odd (right-to-left) embedding level.
    pub fn is_rtl(&self, logical: usize) -> bool {
        self.levels()
            .get(logical)
            .is_some_and(|level| level % 2 == 1)
    }

    /// Visual boundary on the logical-before side of a cell.
    fn cell_edge_before(&self, cell: usize) -> usize {
        let visual = self.visual_of(cell);
        if self.is_rtl(cell) {
            (visual + 1).min(self.cols)
        } else {
            visual
        }
    }

    /// Visual boundary on the logical-after side of a cell.
    fn cell_edge_after(&self, cell: usize) -> usize {
        let visual = self.visual_of(cell);
        if self.is_rtl(cell) {
            visual
        } else {
            (visual + 1).min(self.cols)
        }
    }

    /// Visual position of the cursor boundary (0..=cols).
    ///
    /// A boundary between two cells inside an RTL run has the same visual
    /// position from either side; only a run boundary makes affinity pick a
    /// different side, so the cell's own level decides which edge it owns.
    pub fn cursor_visual(&self, cursor: &CursorState) -> usize {
        if self.cols == 0 {
            return 0;
        }
        match cursor.affinity {
            Affinity::Before => {
                if cursor.logical < self.cols {
                    self.cell_edge_before(cursor.logical)
                } else {
                    self.cell_edge_after(self.cols - 1)
                }
            }
            Affinity::After => {
                if cursor.logical == 0 {
                    self.cell_edge_before(0)
                } else {
                    self.cell_edge_after((cursor.logical - 1).min(self.cols - 1))
                }
            }
        }
    }

    /// Logical cell under a visual column (mouse hit-testing).
    pub fn logical_at(&self, visual: usize) -> usize {
        self.logical_of(visual)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::BaseDirection;
    use crate::term::{Cell, RowSnapshot, Style};
    use unicode_width::UnicodeWidthStr;

    fn cell(text: &str) -> Cell {
        let width = UnicodeWidthStr::width(text).clamp(1, 2) as u8;
        Cell {
            text: text.to_string(),
            width,
            style: Style::default(),
        }
    }

    fn row(texts: &[&str]) -> RowSnapshot {
        RowSnapshot {
            cells: texts.iter().map(|text| cell(text)).collect(),
            wrapped: false,
        }
    }

    fn cursor(logical: usize, affinity: Affinity) -> CursorState {
        CursorState {
            row: 0,
            logical,
            affinity,
        }
    }

    #[test]
    fn identity_round_trips_every_cell() {
        let map = VisualMap::identity(80);
        for col in 0..80 {
            assert_eq!(map.logical_of(map.visual_of(col)), col);
            assert_eq!(map.visual_of(map.logical_of(col)), col);
        }
    }

    #[test]
    fn lookups_clamp_to_the_row() {
        let map = VisualMap::identity(10);
        assert_eq!(map.visual_of(99), 9);
        assert_eq!(map.logical_of(99), 9);
        assert_eq!(map.logical_at(99), 9);
    }

    #[test]
    fn cursor_uses_cell_edges_in_an_ltr_row() {
        let map = VisualMap::identity(10);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::Before)), 3);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::After)), 3);
        assert_eq!(map.cursor_visual(&cursor(10, Affinity::Before)), 10);
        assert_eq!(map.cursor_visual(&cursor(10, Affinity::After)), 10);
        assert_eq!(map.cursor_visual(&cursor(0, Affinity::After)), 0);
    }

    #[test]
    fn affinity_selects_a_side_at_a_bidi_run_boundary() {
        let map = VisualMap::from_bidi(&row(&["a", "b", "c", "م", "ر", "ح"]), BaseDirection::Ltr);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::After)), 3);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::Before)), 6);
        assert_eq!(map.cursor_visual(&cursor(6, Affinity::After)), 3);
        assert_eq!(map.cursor_visual(&cursor(6, Affinity::Before)), 3);
    }

    #[test]
    fn adjacent_cells_inside_a_run_share_one_gap() {
        let map = VisualMap::from_bidi(&row(&["a", "b", "c", "م", "ر", "ح"]), BaseDirection::Ltr);
        assert_eq!(map.cursor_visual(&cursor(1, Affinity::Before)), 1);
        assert_eq!(map.cursor_visual(&cursor(1, Affinity::After)), 1);
        assert_eq!(map.cursor_visual(&cursor(4, Affinity::Before)), 5);
        assert_eq!(map.cursor_visual(&cursor(4, Affinity::After)), 5);
    }

    #[test]
    fn pure_rtl_start_and_end_are_mirrored() {
        let map = VisualMap::from_bidi(&row(&["م", "ر", "ح"]), BaseDirection::Ltr);
        assert_eq!(map.cursor_visual(&cursor(0, Affinity::Before)), 3);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::Before)), 0);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::After)), 0);
    }

    #[test]
    fn hit_testing_returns_the_cell_at_a_visual_column() {
        let map = VisualMap::identity(10);
        assert_eq!(map.logical_at(0), 0);
        assert_eq!(map.logical_at(4), 4);
    }

    #[test]
    fn identity_exposes_one_ltr_run_and_zero_levels() {
        let map = VisualMap::identity(4);
        assert_eq!(map.cols(), 4);
        assert_eq!(map.levels(), &[0, 0, 0, 0]);
        assert_eq!(map.runs().len(), 1);
        assert_eq!(map.runs()[0].cells, 0..4);
        assert_eq!(map.runs()[0].slots, 0..4);
        assert!(!map.runs()[0].rtl);
    }

    #[test]
    fn bidi_runs_are_exposed_in_visual_order() {
        let map = VisualMap::from_bidi(&row(&["a", "b", "c", "م", "ر", "ح"]), BaseDirection::Ltr);
        assert_eq!(map.levels(), &[0, 0, 0, 1, 1, 1]);
        assert_eq!(map.runs().len(), 2);
        assert_eq!(map.runs()[0].slots, 0..3);
        assert_eq!(map.runs()[1].slots, 3..6);
        assert!(map.runs()[1].rtl);
    }

    #[test]
    fn empty_rows_have_no_runs() {
        let map = VisualMap::identity(0);
        assert_eq!(map.cols(), 0);
        assert!(map.runs().is_empty());
    }

    use crate::config::BidiConfig;

    fn bidi(base_direction: BaseDirection) -> BidiConfig {
        BidiConfig {
            enabled: true,
            base_direction,
        }
    }

    #[test]
    fn ltr_base_keeps_the_prompt_on_the_left() {
        let line = row(&["a", "b", " ", "م", "ر"]);
        let map = VisualMap::for_row(&line, &bidi(BaseDirection::Ltr));
        assert_eq!(map.visual_of(0), 0);
        assert_eq!(map.visual_of(2), 2);
        assert_eq!(map.visual_of(3), 4);
        assert_eq!(map.visual_of(4), 3);
    }

    #[test]
    fn rtl_base_right_anchors_latin_text() {
        let line = row(&["a", "b", "c", " ", " ", " ", " ", " ", " ", " "]);
        let map = VisualMap::for_row(&line, &bidi(BaseDirection::Rtl));
        assert_eq!(map.visual_of(0), 7);
        assert_eq!(map.visual_of(2), 9);
    }

    #[test]
    fn auto_base_follows_the_first_strong_character() {
        let rtl_first = row(&["م", "ر", "ح", " ", " ", " ", " "]);
        let map = VisualMap::for_row(&rtl_first, &bidi(BaseDirection::Auto));
        assert_eq!(map.visual_of(0), 6);

        let ltr_first = row(&["a", "b", "c", " ", " ", " ", " "]);
        let map = VisualMap::for_row(&ltr_first, &bidi(BaseDirection::Auto));
        assert_eq!(map.visual_of(0), 0);
    }

    #[test]
    fn only_bidi_maps_report_analysis() {
        assert!(!VisualMap::identity(3).is_bidi_analyzed());
        assert!(
            VisualMap::from_bidi(&row(&["a", "b", "c"]), BaseDirection::Ltr).is_bidi_analyzed()
        );
    }

    #[test]
    fn disabled_bidi_falls_back_to_identity() {
        let line = row(&["م", "ر", "ح"]);
        let map = VisualMap::for_row(
            &line,
            &BidiConfig {
                enabled: false,
                base_direction: BaseDirection::Ltr,
            },
        );
        assert_eq!(map.visual_of(0), 0);
        assert_eq!(map.visual_of(2), 2);
        assert!(map.levels().iter().all(|level| *level == 0));
    }
}
