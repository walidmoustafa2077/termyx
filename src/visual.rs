//! Per-row logical-to-visual mapping.

use crate::config::BaseDirection;
use crate::term::{Affinity, CursorState, RowSnapshot};

/// Maps logical cell columns to visual columns for one row.
///
/// Every caller goes through this type: `identity` is Phase 1 and `from_bidi`
/// is the Phase 2 UAX#9 replacement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualMap {
    cols: usize,
    logical_to_visual: Vec<usize>,
    visual_to_logical: Vec<usize>,
}

impl VisualMap {
    pub fn identity(cols: usize) -> Self {
        Self {
            cols,
            logical_to_visual: (0..cols).collect(),
            visual_to_logical: (0..cols).collect(),
        }
    }

    // Wired into the frame in Task 3; tests are the only callers until then.
    #[allow(dead_code)]
    pub fn from_bidi(row: &RowSnapshot, base: BaseDirection) -> Self {
        let analysis = crate::bidi::analyze(row, base);
        let cols = analysis.levels.len();
        let mut logical_to_visual = vec![0; cols];
        let mut visual_to_logical = vec![0; cols];
        for run in &analysis.runs {
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
            logical_to_visual,
            visual_to_logical,
        }
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

    /// Visual position of the cursor boundary (0..=cols).
    pub fn cursor_visual(&self, cursor: &CursorState) -> usize {
        match cursor.affinity {
            Affinity::Before => {
                if cursor.logical >= self.cols {
                    self.cols
                } else {
                    self.visual_of(cursor.logical)
                }
            }
            Affinity::After => {
                if cursor.logical == 0 {
                    0
                } else {
                    (self.visual_of(cursor.logical) + 1).min(self.cols)
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
    fn cursor_before_a_cell_sits_on_its_left_edge() {
        let map = VisualMap::identity(10);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::Before)), 3);
        assert_eq!(map.cursor_visual(&cursor(10, Affinity::Before)), 10);
    }

    #[test]
    fn cursor_after_a_cell_sits_on_its_right_edge() {
        let map = VisualMap::identity(10);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::After)), 4);
        assert_eq!(map.cursor_visual(&cursor(10, Affinity::After)), 10);
        assert_eq!(map.cursor_visual(&cursor(0, Affinity::After)), 0);
    }

    #[test]
    fn hit_testing_returns_the_cell_at_a_visual_column() {
        let map = VisualMap::identity(10);
        assert_eq!(map.logical_at(0), 0);
        assert_eq!(map.logical_at(4), 4);
    }
}
