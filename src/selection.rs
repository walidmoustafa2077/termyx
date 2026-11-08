//! Logical-order selection and clipboard copy.

use crate::term::{Cell, GridSnapshot, LogicalPos, RowSnapshot};

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

/// A logical-order text selection. Visual order never enters this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub anchor: LogicalPos,
    pub focus: LogicalPos,
}

impl Selection {
    pub fn new(anchor: LogicalPos) -> Self {
        Self {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{Affinity, Cell, Color, CursorShape, CursorState, RowSnapshot, Style};

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
}
