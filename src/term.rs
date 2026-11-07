//! Terminal grid model and VT state.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::cell::{Cell as TermCell, Flags};
use alacritty_terminal::term::{Config as TermConfig, Term, TermMode};
use unicode_width::UnicodeWidthStr;
use vte::ansi::{Color as AnsiColor, NamedColor, Processor};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Option<Color>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub width: u8,
    pub style: Style,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowSnapshot {
    pub cells: Vec<Cell>,
    pub wrapped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Affinity {
    Before,
    // Phase 1 snapshots always produce `Before`; bidi run boundaries in
    // Phase 2 produce `After`.
    #[allow(dead_code)]
    After,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorState {
    pub row: usize,
    pub logical: usize,
    pub affinity: Affinity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogicalPos {
    pub row: usize,
    pub col: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridSnapshot {
    pub cols: usize,
    pub rows: usize,
    pub lines: Vec<RowSnapshot>,
    pub cursor: CursorState,
    pub display_offset: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct TermSize {
    pub cols: usize,
    pub rows: usize,
    pub total_lines: usize,
}

impl Dimensions for TermSize {
    fn columns(&self) -> usize {
        self.cols
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn total_lines(&self) -> usize {
        self.total_lines
    }
}

#[derive(Clone)]
struct EventProxy;

impl EventListener for EventProxy {
    fn send_event(&self, _event: alacritty_terminal::event::Event) {}
}

pub struct Terminal {
    term: Term<EventProxy>,
    processor: Processor,
    size: TermSize,
    scrollback: usize,
}

impl Terminal {
    pub fn new(cols: usize, rows: usize, scrollback: usize) -> Self {
        let size = TermSize {
            cols,
            rows,
            total_lines: rows + scrollback,
        };
        let config = TermConfig {
            scrolling_history: scrollback,
            ..TermConfig::default()
        };
        let term = Term::new(config, &size, EventProxy);
        Self {
            term,
            processor: Processor::new(),
            size,
            scrollback,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == self.size.cols && rows == self.size.rows {
            return;
        }

        // alacritty anchors column reflow to the bottom of the viewport, so narrowing
        // a screen that has blank rows below its content pushes the first reflowed
        // line into scrollback. When nothing has scrolled yet, reflow into a viewport
        // trimmed to the written rows (plus the cursor row), then restore the
        // requested height: growing pulls the reflowed lines back out of history
        // intact. The alternate screen never reflows, so it is resized in one step.
        //
        // Limitation: the trimmed path only runs before the first scroll. Once the
        // grid has scrollback, growing back would pull unrelated history into the
        // viewport, so narrowing falls back to alacritty's stock bottom-anchored
        // reflow and the top line of the screen may land in scrollback instead.
        let (used_rows, cursor_row) = self.content_extent();
        let has_history = self.term.grid().total_lines() > self.term.grid().screen_lines();
        let height = used_rows.max(cursor_row + 1);
        let staged = cols < self.size.cols
            && height < rows
            && !has_history
            && !self.term.mode().contains(TermMode::ALT_SCREEN);

        if staged {
            let trimmed = TermSize {
                cols,
                rows: height,
                total_lines: height + self.scrollback,
            };
            self.term.resize(trimmed);
        }

        self.size = TermSize {
            cols,
            rows,
            total_lines: rows + self.scrollback,
        };
        self.term.resize(self.size);
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn scroll_display(&mut self, delta: i32) {
        self.term.scroll_display(Scroll::Delta(delta));
    }

    pub fn snapshot(&self) -> GridSnapshot {
        let content = self.term.renderable_content();
        let cols = self.size.cols;
        let rows = self.size.rows;
        let mut lines = Vec::with_capacity(rows);
        let mut cells = Vec::with_capacity(cols);
        let mut wrapped = false;

        for (index, indexed) in content.display_iter.enumerate() {
            let cell = indexed.cell;
            if index > 0 && index % cols == 0 {
                lines.push(RowSnapshot {
                    cells: std::mem::take(&mut cells),
                    wrapped,
                });
                cells = Vec::with_capacity(cols);
            }
            wrapped = cell.flags.contains(Flags::WRAPLINE);
            cells.push(convert_cell(cell));
        }
        if !cells.is_empty() {
            lines.push(RowSnapshot { cells, wrapped });
        }
        lines.truncate(rows);
        while lines.len() < rows {
            lines.push(RowSnapshot {
                cells: vec![
                    Cell {
                        text: " ".into(),
                        width: 1,
                        style: Style::default(),
                    };
                    cols
                ],
                wrapped: false,
            });
        }

        let cursor = if content.display_offset == 0 {
            let row = content.cursor.point.line.0.max(0) as usize;
            CursorState {
                row: row.min(rows.saturating_sub(1)),
                logical: content.cursor.point.column.0.min(cols),
                affinity: Affinity::Before,
            }
        } else {
            CursorState {
                row: 0,
                logical: 0,
                affinity: Affinity::Before,
            }
        };

        GridSnapshot {
            cols,
            rows,
            lines,
            cursor,
            display_offset: content.display_offset,
        }
    }

    /// Number of screen rows up to and including the last row with written content,
    /// and the cursor's screen row.
    fn content_extent(&self) -> (usize, usize) {
        let content = self.term.renderable_content();
        let cols = self.size.cols;
        let mut used_rows = 0;

        for (index, indexed) in content.display_iter.enumerate() {
            let cell = indexed.cell;
            let written =
                cell.c != ' ' && cell.c != '\0' && !cell.flags.contains(Flags::WIDE_CHAR_SPACER);
            let painted = cell.bg != AnsiColor::Named(NamedColor::Background);
            if written || painted {
                used_rows = index / cols + 1;
            }
        }

        (used_rows, content.cursor.point.line.0.max(0) as usize)
    }
}

fn convert_cell(cell: &TermCell) -> Cell {
    let style = Style {
        fg: map_fg(cell.fg),
        bg: map_bg(cell.bg),
    };

    if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
        return Cell {
            text: String::new(),
            width: 0,
            style,
        };
    }

    let mut text = String::new();
    let ch = if cell.c == '\0' { ' ' } else { cell.c };
    text.push(ch);
    if let Some(extra) = cell.zerowidth() {
        for ch in extra {
            text.push(*ch);
        }
    }
    let width = UnicodeWidthStr::width(text.as_str()).clamp(1, 2) as u8;
    Cell { text, width, style }
}

fn map_fg(color: AnsiColor) -> Color {
    match color {
        AnsiColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(index) => Color::Indexed(index),
        AnsiColor::Named(named) => named_index(named)
            .map(Color::Indexed)
            .unwrap_or(Color::Default),
    }
}

fn map_bg(color: AnsiColor) -> Option<Color> {
    match color {
        AnsiColor::Named(NamedColor::Background) => None,
        AnsiColor::Named(NamedColor::Foreground) => None,
        other => {
            let mapped = map_fg(other);
            if mapped == Color::Default {
                None
            } else {
                Some(mapped)
            }
        }
    }
}

fn named_index(named: NamedColor) -> Option<u8> {
    Some(match named {
        NamedColor::Black => 0,
        NamedColor::Red => 1,
        NamedColor::Green => 2,
        NamedColor::Yellow => 3,
        NamedColor::Blue => 4,
        NamedColor::Magenta => 5,
        NamedColor::Cyan => 6,
        NamedColor::White => 7,
        NamedColor::BrightBlack => 8,
        NamedColor::BrightRed => 9,
        NamedColor::BrightGreen => 10,
        NamedColor::BrightYellow => 11,
        NamedColor::BrightBlue => 12,
        NamedColor::BrightMagenta => 13,
        NamedColor::BrightCyan => 14,
        NamedColor::BrightWhite => 15,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_plain_text_and_track_cursor() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"hello");
        let snapshot = term.snapshot();
        assert_eq!(snapshot.lines[0].cells[0].text, "h");
        assert_eq!(snapshot.lines[0].cells[4].text, "o");
        assert_eq!(snapshot.cursor.row, 0);
        assert_eq!(snapshot.cursor.logical, 5);
    }

    #[test]
    fn sgr_color_is_captured() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"\x1b[31mred");
        let snapshot = term.snapshot();
        assert_eq!(snapshot.lines[0].cells[0].style.fg, Color::Indexed(1));
    }

    #[test]
    fn newline_moves_cursor_to_next_row() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"a\r\nb");
        let snapshot = term.snapshot();
        assert_eq!(snapshot.lines[1].cells[0].text, "b");
        assert_eq!(snapshot.cursor.row, 1);
        assert_eq!(snapshot.cursor.logical, 1);
    }

    #[test]
    fn wide_char_occupies_two_cells() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed("中".as_bytes());
        let snapshot = term.snapshot();
        assert_eq!(snapshot.lines[0].cells[0].text, "中");
        assert_eq!(snapshot.lines[0].cells[0].width, 2);
        assert_eq!(snapshot.lines[0].cells[1].width, 0);
    }

    #[test]
    fn resize_reflows_visible_text() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"abcdefghijklmnopqrstuvwxy");
        term.resize(10, 5);
        let snapshot = term.snapshot();
        let text: String = snapshot
            .lines
            .iter()
            .flat_map(|row| row.cells.iter().map(|cell| cell.text.as_str()))
            .collect::<String>()
            .replace(' ', "");
        assert!(text.contains("abcdefghijklmnopqrstuvwxy"), "got {text:?}");
    }

    #[test]
    fn alt_screen_mode_is_exposed() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"\x1b[?1049h");
        assert!(term.mode().contains(TermMode::ALT_SCREEN));
    }

    #[test]
    fn scroll_display_moves_viewport_offset() {
        let mut term = Terminal::new(20, 3, 1_000);
        for index in 0..10 {
            term.feed(format!("line{index}\r\n").as_bytes());
        }
        term.scroll_display(2);
        assert_eq!(term.display_offset(), 2);
        assert_eq!(term.snapshot().display_offset, 2);
    }

    #[test]
    fn staged_resize_tracks_cursor_after_reflow() {
        let mut term = Terminal::new(20, 5, 1_000);
        // 26 characters wrap at 20 columns, so the cursor already sits on row 1.
        term.feed(b"abcdefghijklmnopqrstuvwxyz");
        assert_eq!(term.snapshot().cursor.row, 1);

        // Narrowing reflows the 26 cells into 10 + 10 + 6 and the cursor follows
        // the text to the end of the third row.
        term.resize(10, 5);
        let snapshot = term.snapshot();
        assert_eq!(snapshot.display_offset, 0);
        assert_eq!(snapshot.cursor.row, 2);
        assert_eq!(snapshot.cursor.logical, 6);
        assert_eq!(snapshot.lines[2].cells[0].text, "u");
    }

    #[test]
    fn alt_screen_survives_narrow_resize() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"\x1b[?1049h");
        term.resize(10, 5);
        assert!(term.mode().contains(TermMode::ALT_SCREEN));
        let snapshot = term.snapshot();
        assert_eq!(snapshot.cols, 10);
        assert_eq!(snapshot.rows, 5);
        assert_eq!(snapshot.lines.len(), 5);
    }

    #[test]
    fn nul_byte_leaves_blank_cell() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"\0");
        let snapshot = term.snapshot();
        assert_eq!(snapshot.lines[0].cells[0].text, " ");
        assert_eq!(snapshot.lines[0].cells[0].width, 1);
    }

    #[test]
    fn convert_cell_sanitizes_nul_to_space() {
        // vte never stores a NUL in a cell, so drive convert_cell directly to keep
        // the `cell.c == '\0'` sanitisation covered.
        let cell = TermCell {
            c: '\0',
            ..TermCell::default()
        };
        let converted = convert_cell(&cell);
        assert_eq!(converted.text, " ");
        assert_eq!(converted.width, 1);
    }
}
