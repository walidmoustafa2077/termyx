use crate::term::{GridSnapshot, LogicalPos};

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
                text = text.trim_end().to_string();
            }
            if row > start.row && !snapshot.lines[row - 1].wrapped {
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
    use crate::term::{Affinity, Cell, Color, CursorState, RowSnapshot, Style};

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
            },
            display_offset: 0,
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
}
