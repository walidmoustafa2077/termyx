//! Logical-order selection and clipboard copy.

use crate::term::{Cell, GridSnapshot, LogicalPos, RowSnapshot};
use crate::visual::VisualMap;

/// Inclusive logical cell range of the word containing `col`.
///
/// A width-0 continuation cell resolves to the base cell it continues, so a
/// double-click never selects half of a wide character. Whitespace selects only
/// itself; every other cell expands over its own class.
pub fn word_bounds(line: &RowSnapshot, col: usize) -> (usize, usize) {
    if line.cells.is_empty() {
        return (0, 0);
    }
    let last = line.cells.len() - 1;
    let mut col = col.min(last);
    while col > 0 && line.cells[col].width == 0 {
        col -= 1;
    }
    let Some(class) = cell_class(&line.cells[col]) else {
        return (col, col);
    };

    let mut start = col;
    while let Some(previous) = previous_base(&line.cells, start) {
        if cell_class(&line.cells[previous]) != Some(class) {
            break;
        }
        start = previous;
    }

    let mut end = col;
    while let Some(next) = next_base(&line.cells, end) {
        if cell_class(&line.cells[next]) != Some(class) {
            break;
        }
        end = next;
    }
    while end < last && line.cells[end + 1].width == 0 {
        end += 1;
    }
    (start, end)
}

/// Inclusive logical cell range of the whole row.
pub fn line_bounds(line: &RowSnapshot) -> (usize, usize) {
    (0, line.cells.len().saturating_sub(1))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CellClass {
    Word,
    Punctuation,
}

/// Classifies a base cell. Whitespace, including the empty text of a
/// continuation cell, never expands.
fn cell_class(cell: &Cell) -> Option<CellClass> {
    if cell.text.chars().all(char::is_whitespace) {
        return None;
    }
    let first = cell.text.chars().next()?;
    if first.is_alphanumeric() || first == '_' {
        Some(CellClass::Word)
    } else {
        Some(CellClass::Punctuation)
    }
}

/// Index of the base cell before `from`, skipping continuation cells.
fn previous_base(cells: &[Cell], from: usize) -> Option<usize> {
    (0..from).rev().find(|&index| cells[index].width > 0)
}

/// Index of the base cell after `from`, skipping continuation cells.
fn next_base(cells: &[Cell], from: usize) -> Option<usize> {
    (from + 1..cells.len()).find(|&index| cells[index].width > 0)
}

/// How a selection maps its endpoints to the cells it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    /// Logical text range, used by word and line selection.
    Simple,
    /// Visual span between the drag endpoints.
    Visual,
    Block,
}

/// A selection stored in logical coordinates. Copying always emits logical
/// order; the mode decides which cells the endpoints cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub mode: SelectionMode,
    pub anchor: LogicalPos,
    pub focus: LogicalPos,
}

impl Selection {
    pub fn new(anchor: LogicalPos) -> Self {
        Self {
            mode: SelectionMode::Simple,
            anchor,
            focus: anchor,
        }
    }

    /// A pointer drag. The covered cells follow the visual span between the
    /// endpoints, which is what the user sees while dragging across bidi text.
    pub fn visual(anchor: LogicalPos) -> Self {
        Self {
            mode: SelectionMode::Visual,
            anchor,
            focus: anchor,
        }
    }

    pub fn block(anchor: LogicalPos) -> Self {
        Self {
            mode: SelectionMode::Block,
            anchor,
            focus: anchor,
        }
    }

    pub fn update(&mut self, focus: LogicalPos) {
        self.focus = focus;
    }

    /// Start and end in logical order, regardless of drag direction.
    pub fn ordered(&self) -> (LogicalPos, LogicalPos) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    pub fn contains(&self, row: usize, col: usize) -> bool {
        let (start, end) = self.ordered();
        if row < start.row || row > end.row {
            return false;
        }
        if row == start.row && col < start.col {
            return false;
        }
        if row == end.row && col > end.col {
            return false;
        }
        true
    }

    /// First/last selected rows and left/right visual columns. Block geometry
    /// lives here so highlight and copy cannot drift apart.
    fn visual_bounds(&self, maps: &[VisualMap]) -> Option<(usize, usize, usize, usize)> {
        let anchor_map = maps.get(self.anchor.row)?;
        let focus_map = maps.get(self.focus.row)?;
        let a = anchor_map.visual_of(self.anchor.col);
        let b = focus_map.visual_of(self.focus.col);
        Some((
            self.anchor.row.min(self.focus.row),
            self.anchor.row.max(self.focus.row),
            a.min(b),
            a.max(b),
        ))
    }

    /// Visual columns covered on one row of a pointer drag. Rows between the
    /// endpoints are covered from edge to edge; an endpoint row runs from the
    /// endpoint to the right edge of the line.
    fn visual_range(&self, row: usize, maps: &[VisualMap]) -> Option<(usize, usize)> {
        let (start, end) = self.ordered();
        if row < start.row || row > end.row {
            return None;
        }
        let map = maps.get(row)?;
        let last = map.cols().saturating_sub(1);
        if start.row == end.row {
            let a = map.visual_of(start.col);
            let b = map.visual_of(end.col);
            Some((a.min(b), a.max(b)))
        } else if row == start.row {
            Some((map.visual_of(start.col), last))
        } else if row == end.row {
            Some((0, map.visual_of(end.col)))
        } else {
            Some((0, last))
        }
    }

    /// True when the cell is inside this selection. Block mode compares
    /// visual columns; the maps carry each row's bidi layout.
    pub fn is_selected(
        &self,
        row: usize,
        logical: usize,
        visual: usize,
        maps: &[VisualMap],
    ) -> bool {
        match self.mode {
            SelectionMode::Simple => self.contains(row, logical),
            SelectionMode::Visual => {
                // A plain click parks a collapsed drag without selecting
                // anything; painting it would leave one blue cell behind.
                if self.anchor == self.focus {
                    return false;
                }
                self.visual_range(row, maps)
                    .is_some_and(|(left, right)| (left..=right).contains(&visual))
            }
            SelectionMode::Block => {
                let Some((first, last, left, right)) = self.visual_bounds(maps) else {
                    return false;
                };
                row >= first && row <= last && visual >= left && visual <= right
            }
        }
    }

    /// Copy a visual rectangle row by row, skipping continuation cells.
    pub fn to_text_block(
        &self,
        snapshot: &GridSnapshot,
        maps: &[VisualMap],
        trim_trailing: bool,
    ) -> String {
        let Some((first, last, left, right)) = self.visual_bounds(maps) else {
            return String::new();
        };
        let mut out = String::new();
        for row in first..=last {
            let (Some(line), Some(map)) = (snapshot.lines.get(row), maps.get(row)) else {
                continue;
            };
            if row > first {
                out.push('\n');
            }
            let mut text = String::new();
            for visual in left..=right {
                let logical = map.logical_of(visual);
                if let Some(cell) = line.cells.get(logical) {
                    if cell.width > 0 {
                        text.push_str(&cell.text);
                    }
                }
            }
            if trim_trailing {
                text = text.trim_end_matches(' ').to_string();
            }
            out.push_str(&text);
        }
        out
    }

    /// Copy in logical order, skipping wide-character continuation cells.
    pub fn to_text(&self, snapshot: &GridSnapshot, trim_trailing: bool) -> String {
        let (start, end) = self.ordered();
        let mut out = String::new();

        for row in start.row..=end.row {
            let Some(line) = snapshot.lines.get(row) else {
                continue;
            };
            let first = if row == start.row { start.col } else { 0 };
            let last = if row == end.row {
                end.col.min(line.cells.len().saturating_sub(1))
            } else {
                line.cells.len().saturating_sub(1)
            };

            let mut text = String::new();
            for col in first..=last {
                if let Some(cell) = line.cells.get(col) {
                    if cell.width > 0 {
                        text.push_str(&cell.text);
                    }
                }
            }
            if trim_trailing {
                text = text.trim_end_matches(' ').to_string();
            }
            if row > start.row && !snapshot.lines.get(row - 1).is_some_and(|line| line.wrapped) {
                out.push('\n');
            }
            out.push_str(&text);
        }

        out
    }

    /// Copy the cells covered by a pointer drag in logical order.
    pub fn to_text_visual(
        &self,
        snapshot: &GridSnapshot,
        maps: &[VisualMap],
        trim_trailing: bool,
    ) -> String {
        let (start, end) = self.ordered();
        let mut out = String::new();

        for row in start.row..=end.row {
            let (Some(line), Some(map)) = (snapshot.lines.get(row), maps.get(row)) else {
                continue;
            };
            let Some((left, right)) = self.visual_range(row, maps) else {
                continue;
            };

            // The dragged cells are visual, but the copied text walks their
            // logical columns so it matches the original line.
            let mut columns: Vec<usize> = (left..=right)
                .map(|visual| map.logical_of(visual))
                .collect();
            columns.sort_unstable();
            columns.dedup();

            let mut text = String::new();
            for col in columns {
                if let Some(cell) = line.cells.get(col) {
                    if cell.width > 0 {
                        text.push_str(&cell.text);
                    }
                }
            }
            if trim_trailing {
                text = text.trim_end_matches(' ').to_string();
            }
            if row > start.row && !snapshot.lines.get(row - 1).is_some_and(|line| line.wrapped) {
                out.push('\n');
            }
            out.push_str(&text);
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BaseDirection;
    use crate::term::{Affinity, Cell, Color, CursorShape, CursorState, RowSnapshot, Style};
    use crate::visual::VisualMap;

    fn position(row: usize, col: usize) -> LogicalPos {
        LogicalPos { row, col }
    }

    fn cell(ch: char) -> Cell {
        Cell {
            text: ch.to_string(),
            width: 1,
            style: Style {
                fg: Color::Default,
                bg: None,
                ..Style::default()
            },
        }
    }

    fn row(text: &str) -> RowSnapshot {
        RowSnapshot {
            cells: text.chars().map(cell).collect(),
            wrapped: false,
        }
    }

    fn snapshot() -> GridSnapshot {
        GridSnapshot {
            cols: 5,
            rows: 2,
            lines: vec![row("hello"), row("world")],
            cursor: CursorState {
                row: 0,
                logical: 0,
                affinity: Affinity::Before,
                shape: CursorShape::Block,
                visible: true,
                blinking: false,
            },
            display_offset: 0,
            history: 0,
        }
    }

    #[test]
    fn selects_within_one_line() {
        let mut selection = Selection::new(position(0, 1));
        selection.update(position(0, 3));
        assert_eq!(selection.to_text(&snapshot(), true), "ell");
    }

    #[test]
    fn reverse_selection_is_normalized() {
        let mut selection = Selection::new(position(0, 3));
        selection.update(position(0, 1));
        assert_eq!(selection.to_text(&snapshot(), true), "ell");
    }

    #[test]
    fn selects_across_lines() {
        let mut selection = Selection::new(position(0, 3));
        selection.update(position(1, 2));
        assert_eq!(selection.to_text(&snapshot(), true), "lo\nwor");
    }

    #[test]
    fn contains_includes_selection_edges() {
        let mut selection = Selection::new(position(0, 1));
        selection.update(position(1, 2));
        assert!(selection.contains(0, 1));
        assert!(selection.contains(0, 4));
        assert!(selection.contains(1, 0));
        assert!(selection.contains(1, 2));
        assert!(!selection.contains(0, 0));
        assert!(!selection.contains(1, 3));
        assert!(!selection.contains(2, 0));
    }

    #[test]
    fn trims_trailing_spaces_when_requested() {
        let mut snapshot = snapshot();
        snapshot.lines[0] = row("hi   ");
        let mut selection = Selection::new(position(0, 0));
        selection.update(position(0, 4));
        assert_eq!(selection.to_text(&snapshot, true), "hi");
        assert_eq!(selection.to_text(&snapshot, false), "hi   ");
    }

    fn arabic_snapshot() -> GridSnapshot {
        GridSnapshot {
            cols: 6,
            rows: 1,
            lines: vec![RowSnapshot {
                cells: ["a", "b", "c", "م", "ر", "ح"]
                    .iter()
                    .map(|text| Cell {
                        text: (*text).to_string(),
                        width: 1,
                        style: Style::default(),
                    })
                    .collect(),
                wrapped: false,
            }],
            cursor: CursorState {
                row: 0,
                logical: 0,
                affinity: Affinity::Before,
                shape: CursorShape::Block,
                visible: true,
                blinking: false,
            },
            display_offset: 0,
            history: 0,
        }
    }

    #[test]
    fn copy_returns_logical_order_for_an_rtl_row() {
        let mut selection = Selection::new(position(0, 3));
        selection.update(position(0, 5));
        assert_eq!(selection.to_text(&arabic_snapshot(), true), "مرح");
    }

    #[test]
    fn copy_returns_logical_order_for_a_mixed_row() {
        let mut selection = Selection::new(position(0, 0));
        selection.update(position(0, 5));
        assert_eq!(selection.to_text(&arabic_snapshot(), true), "abcمرح");
    }

    #[test]
    fn drag_selection_highlights_the_visual_span_in_a_mixed_row() {
        let snapshot = arabic_snapshot();
        let maps = vec![VisualMap::from_bidi(&snapshot.lines[0], BaseDirection::Ltr)];
        // Visual order is `a b c ح ر م`; drag across `b c ح ر`.
        let mut selection = Selection::visual(position(0, maps[0].logical_of(1)));
        selection.update(position(0, maps[0].logical_of(4)));

        for visual in 1..=4 {
            let logical = maps[0].logical_of(visual);
            assert!(selection.is_selected(0, logical, visual, &maps));
        }
        for visual in [0, 5] {
            let logical = maps[0].logical_of(visual);
            assert!(!selection.is_selected(0, logical, visual, &maps));
        }
        assert_eq!(selection.to_text_visual(&snapshot, &maps, true), "bcرح");
    }

    #[test]
    fn collapsed_drag_paints_nothing() {
        let maps = identity_maps(1, 4);
        let selection = Selection::visual(position(0, 2));
        assert!(!selection.is_selected(0, 2, 2, &maps));
    }

    #[test]
    fn dragging_across_blank_cells_still_paints() {
        let maps = identity_maps(1, 6);
        let mut selection = Selection::visual(position(0, 1));
        selection.update(position(0, 4));
        assert!(selection.is_selected(0, 3, 3, &maps));
        assert!(!selection.is_selected(0, 5, 5, &maps));
    }

    #[test]
    fn collapsed_word_selection_still_paints() {
        let maps = identity_maps(1, 4);
        let selection = Selection::new(position(0, 2));
        assert!(selection.is_selected(0, 2, 2, &maps));
    }

    #[test]
    fn reverse_drag_still_copies_in_logical_order() {
        let mut selection = Selection::new(position(0, 5));
        selection.update(position(0, 3));
        assert_eq!(selection.to_text(&arabic_snapshot(), true), "مرح");
    }

    #[test]
    fn word_bounds_selects_the_word_under_the_cursor() {
        let line = row("foo.bar baz");
        assert_eq!(word_bounds(&line, 5), (4, 6));
    }

    #[test]
    fn word_bounds_selects_the_punctuation_run() {
        let line = row("foo.bar baz");
        assert_eq!(word_bounds(&line, 3), (3, 3));
    }

    #[test]
    fn word_bounds_on_whitespace_selects_only_that_cell() {
        let line = row("foo.bar baz");
        assert_eq!(word_bounds(&line, 7), (7, 7));
    }

    #[test]
    fn word_bounds_resolves_wide_continuations_to_their_base() {
        let line = RowSnapshot {
            cells: vec![
                Cell {
                    text: "世".into(),
                    width: 2,
                    style: Style::default(),
                },
                Cell {
                    text: String::new(),
                    width: 0,
                    style: Style::default(),
                },
                cell('.'),
            ],
            wrapped: false,
        };
        assert_eq!(word_bounds(&line, 1), (0, 1));
        assert_eq!(word_bounds(&line, 0), (0, 1));
    }

    #[test]
    fn line_bounds_covers_the_whole_row() {
        let line = row("foo.bar baz");
        assert_eq!(line_bounds(&line), (0, 10));
    }

    fn identity_maps(rows: usize, cols: usize) -> Vec<VisualMap> {
        (0..rows).map(|_| VisualMap::identity(cols)).collect()
    }

    #[test]
    fn block_selection_covers_a_visual_rectangle() {
        let snapshot = GridSnapshot {
            cols: 5,
            rows: 2,
            lines: vec![row("hello"), row("world")],
            cursor: CursorState {
                row: 0,
                logical: 0,
                affinity: Affinity::Before,
                shape: CursorShape::Block,
                visible: true,
                blinking: false,
            },
            display_offset: 0,
            history: 0,
        };
        let maps = identity_maps(2, 5);
        let mut selection = Selection::block(position(0, 1));
        selection.update(position(1, 3));

        assert!(selection.is_selected(0, 1, 1, &maps));
        assert!(selection.is_selected(1, 3, 3, &maps));
        assert!(!selection.is_selected(0, 0, 0, &maps));
        assert!(!selection.is_selected(1, 4, 4, &maps));
        assert_eq!(selection.to_text_block(&snapshot, &maps, true), "ell\norl");
    }

    #[test]
    fn reverse_block_drag_covers_the_same_rectangle() {
        let snapshot = GridSnapshot {
            cols: 5,
            rows: 2,
            lines: vec![row("hello"), row("world")],
            cursor: CursorState {
                row: 0,
                logical: 0,
                affinity: Affinity::Before,
                shape: CursorShape::Block,
                visible: true,
                blinking: false,
            },
            display_offset: 0,
            history: 0,
        };
        let maps = identity_maps(2, 5);
        let mut selection = Selection::block(position(1, 3));
        selection.update(position(0, 1));

        assert!(selection.is_selected(0, 1, 1, &maps));
        assert!(selection.is_selected(1, 3, 3, &maps));
        assert!(!selection.is_selected(0, 0, 0, &maps));
        assert!(!selection.is_selected(1, 4, 4, &maps));
        assert_eq!(selection.to_text_block(&snapshot, &maps, true), "ell\norl");
    }

    #[test]
    fn block_selection_uses_visual_columns_in_rtl() {
        let snapshot = GridSnapshot {
            cols: 5,
            rows: 1,
            lines: vec![row("مرحبا")],
            cursor: CursorState {
                row: 0,
                logical: 0,
                affinity: Affinity::Before,
                shape: CursorShape::Block,
                visible: true,
                blinking: false,
            },
            display_offset: 0,
            history: 0,
        };
        let maps = vec![VisualMap::from_bidi(&snapshot.lines[0], BaseDirection::Ltr)];
        let mut selection = Selection::block(position(0, 0));
        selection.update(position(0, 2));

        let anchor_visual = maps[0].visual_of(0);
        let focus_visual = maps[0].visual_of(2);
        let (left, right) = (
            anchor_visual.min(focus_visual),
            anchor_visual.max(focus_visual),
        );
        assert!(selection.is_selected(0, maps[0].logical_of(right), right, &maps));
        let outside = left.saturating_sub(1);
        assert!(!selection.is_selected(0, maps[0].logical_of(outside), outside, &maps));
        assert_eq!(selection.to_text_block(&snapshot, &maps, false), "حرم");
    }

    #[test]
    fn block_copy_skips_wide_continuations() {
        let snapshot = GridSnapshot {
            cols: 3,
            rows: 1,
            lines: vec![RowSnapshot {
                cells: vec![cell('漢'), cell(' '), cell('x')],
                wrapped: false,
            }],
            cursor: CursorState {
                row: 0,
                logical: 0,
                affinity: Affinity::Before,
                shape: CursorShape::Block,
                visible: true,
                blinking: false,
            },
            display_offset: 0,
            history: 0,
        };
        let mut line = snapshot.lines[0].clone();
        line.cells[0].width = 2;
        line.cells[1].text = String::new();
        line.cells[1].width = 0;
        let snapshot = GridSnapshot {
            lines: vec![line],
            ..snapshot
        };
        let maps = identity_maps(1, 3);
        let mut selection = Selection::block(position(0, 0));
        selection.update(position(0, 2));
        assert_eq!(selection.to_text_block(&snapshot, &maps, true), "漢x");
    }
}
