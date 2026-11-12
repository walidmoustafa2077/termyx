//! Terminal grid model and VT state.

use alacritty_terminal::event::Event;
use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::Line;
use alacritty_terminal::term::cell::{Cell as TermCell, Flags};
use alacritty_terminal::term::{Config as TermConfig, Term, TermMode};
use unicode_width::UnicodeWidthStr;
use vte::ansi::{
    Color as AnsiColor, CursorShape as AnsiCursorShape, CursorStyle as AnsiCursorStyle, NamedColor,
    Processor,
};
use winit::event_loop::EventLoopProxy;

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
    pub bold: bool,
    pub inverse: bool,
    pub hidden: bool,
    pub underline: bool,
    pub strikethrough: bool,
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
pub enum CursorShape {
    Block,
    Underline,
    Beam,
    HollowBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorState {
    pub row: usize,
    pub logical: usize,
    pub affinity: Affinity,
    pub shape: CursorShape,
    pub visible: bool,
    /// DECSCUSR and private mode 12 ask for a blinking cursor.
    pub blinking: bool,
}

/// Where the shell's editable input line starts, as reported by an OSC 133 `B`
/// mark. `row` is a grid line (`0` is the viewport's top row, negative values
/// reach into scrollback) and `col` is the cursor column at the mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputStart {
    pub row: i32,
    pub col: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogicalPos {
    pub row: usize,
    pub col: usize,
}

/// One occurrence of a search needle in the grid.
///
/// `line` is a grid line index: `0` is the viewport's top row, negative values
/// reach back into scrollback, and `start`/`end` are logical column indices
/// (half-open) within that line. A match is always row-local: text split across
/// a wrapped line is not reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub line: i32,
    pub start: usize,
    pub end: usize,
}

/// One row-local run of cells sharing an OSC 8 hyperlink.
///
/// A run never crosses a row boundary: a link that wraps produces one run
/// per row, the same split the underline pass paints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperlinkRun {
    /// Index into [`GridSnapshot::lines`].
    pub row: usize,
    /// First logical column, inclusive.
    pub start: usize,
    /// One past the last logical column.
    pub end: usize,
    /// The URI the shell attached to the run.
    pub uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridSnapshot {
    pub cols: usize,
    pub rows: usize,
    pub lines: Vec<RowSnapshot>,
    pub cursor: CursorState,
    /// The shell's input start from the last OSC 133 mark, if one is pending.
    pub input_start: Option<InputStart>,
    pub display_offset: usize,
    /// Scrollback rows the scrollbar can map to; leading blank rows are
    /// excluded so the thumb reaches the top where scrolling stops.
    pub history: usize,
    /// OSC 8 hyperlink runs collected while the cells were read, so link
    /// detection never has to revisit the terminal.
    pub hyperlinks: Vec<HyperlinkRun>,
}

impl GridSnapshot {
    /// Cells of visible row `row`, or an empty vector when the row is outside
    /// the snapshot.
    ///
    /// The clone keeps the accessor's ownership simple: the spellcheck scan
    /// takes a plain slice and cannot alias the snapshot.
    pub fn row_cells(&self, row: usize) -> Vec<Cell> {
        self.lines
            .get(row)
            .map(|line| line.cells.clone())
            .unwrap_or_default()
    }
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
struct EventProxy {
    /// The UI event loop and the session this parser belongs to. `None` in
    /// tests, which have no event loop to notify.
    events: Option<(EventLoopProxy<crate::event::UserEvent>, u64)>,
}

/// Map the terminal events Termyx observes onto UI-thread events, tagged with the
/// session whose parser raised them.
fn user_event_for(session: u64, event: Event) -> Option<crate::event::UserEvent> {
    match event {
        Event::Title(title) => Some(crate::event::UserEvent::Title { session, title }),
        Event::ResetTitle => Some(crate::event::UserEvent::ResetTitle { session }),
        Event::ClipboardStore(_, text) => Some(crate::event::UserEvent::ClipboardStore(text)),
        _ => None,
    }
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        let Some((proxy, session)) = &self.events else {
            return;
        };
        if let Some(event) = user_event_for(*session, event) {
            let _ = proxy.send_event(event);
        }
    }
}

pub struct Terminal {
    term: Term<EventProxy>,
    processor: Processor,
    size: TermSize,
    scrollback: usize,
    /// Bytes held back because they end inside an OSC sequence whose terminator
    /// has not arrived; OSC marks are only visible once the sequence is whole.
    pending: Vec<u8>,
    input_start: Option<InputStart>,
}

/// Past this many held-back bytes the buffer is handed to the parser and our
/// copy is released, so Termyx's retained bytes stay bounded even when an OSC
/// never gets its terminator. The parser then owns those bytes, as it does for
/// any OSC sequence.
const MAX_PENDING: usize = 4096;

impl Terminal {
    /// Test-only constructor: no event bridge, no window to notify.
    #[cfg(test)]
    pub fn new(cols: usize, rows: usize, scrollback: usize) -> Self {
        Self::new_with_events(cols, rows, scrollback, None)
    }

    /// Test-only view of the bytes held back for an unterminated OSC.
    #[cfg(test)]
    fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn new_with_events(
        cols: usize,
        rows: usize,
        scrollback: usize,
        events: Option<(EventLoopProxy<crate::event::UserEvent>, u64)>,
    ) -> Self {
        let size = TermSize {
            cols,
            rows,
            total_lines: rows + scrollback,
        };
        let config = TermConfig {
            scrolling_history: scrollback,
            // A blinking bar matches what users see in Windows Terminal;
            // applications can still override it with DECSCUSR (which sets
            // `cursor_style`).
            default_cursor_style: AnsiCursorStyle {
                shape: AnsiCursorShape::Beam,
                blinking: true,
            },
            ..TermConfig::default()
        };
        let term = Term::new(config, &size, EventProxy { events });
        Self {
            term,
            processor: Processor::new(),
            size,
            scrollback,
            pending: Vec::new(),
            input_start: None,
        }
    }

    /// Feed raw PTY bytes to the VT parser.
    ///
    /// Returns `true` when this feed moved rows, which invalidates row-anchored
    /// state such as a text selection: history grew, the alternate screen
    /// swapped grids, or the scrollback ring rotated at its cap. A TUI that
    /// repaints in place below the cap returns `false`, so a selection made on
    /// its visible content can still be copied.
    ///
    /// At the cap the signal is conservative: `total_lines` stays constant
    /// while the ring rotates, so even an in-place repaint there reports a
    /// scroll and drops the selection.
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        let before = self.term.grid().total_lines();
        self.pending.extend_from_slice(bytes);
        let mut consumed = 0usize;

        while consumed < self.pending.len() {
            let Some(scan) = find_osc(&self.pending[consumed..]) else {
                self.processor
                    .advance(&mut self.term, &self.pending[consumed..]);
                consumed = self.pending.len();
                break;
            };
            let start = consumed + scan.offset;
            if scan.offset > 0 {
                self.processor
                    .advance(&mut self.term, &self.pending[consumed..start]);
            }
            let Some(len) = scan.len else {
                // The tail is an OSC without its terminator yet, or a lone ESC
                // that may still open one. Keep it until the rest arrives.
                consumed = start;
                break;
            };

            let mark = osc_mark(&self.pending[start..start + len]);
            // Forward the sequence unchanged; the parser ignores OSC 133.
            self.processor
                .advance(&mut self.term, &self.pending[start..start + len]);
            self.apply_mark(mark);
            consumed = start + len;
        }

        self.pending.drain(..consumed);

        if self.pending.len() > MAX_PENDING {
            let rest = std::mem::take(&mut self.pending);
            self.processor.advance(&mut self.term, &rest);
        }

        let after = self.term.grid().total_lines();
        after != before || before >= self.size.total_lines
    }

    /// Records an OSC 133 mark that has just been forwarded to the parser.
    fn apply_mark(&mut self, mark: Option<PromptMark>) {
        match mark {
            Some(PromptMark::PromptStart) => self.input_start = None,
            Some(PromptMark::InputStart) => {
                let point = self.term.grid().cursor.point;
                self.input_start = Some(InputStart {
                    row: point.line.0,
                    col: point.column.0,
                });
            }
            None => {}
        }
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

    /// Number of rows currently held in scrollback.
    pub fn history_size(&self) -> usize {
        self.term.grid().history_size()
    }

    /// Number of scrollback rows the viewport can reach. Leading blank rows
    /// are excluded because [`Self::scroll_display`] stops on the first row
    /// that carries text. The scrollbar maps its thumb over this range so the
    /// thumb reaches the track top exactly where scrolling stops.
    pub fn scrollable_history(&self) -> usize {
        let history = self.history_size();
        let mut blank = 0;
        while blank < history && self.history_row_is_blank(blank) {
            blank += 1;
        }
        history - blank
    }

    fn history_row_is_blank(&self, row: usize) -> bool {
        let history = self.history_size();
        let line = Line(-((history - row) as i32));
        self.term.grid()[line][..].iter().all(|cell| cell.c == ' ')
    }

    /// Scrolls the viewport by `delta` lines. Blank history rows are skipped,
    /// so scrolling back stops at the first line that carries text instead of
    /// parking the viewport on empty rows nobody wrote.
    pub fn scroll_display(&mut self, delta: i32) {
        self.term.scroll_display(Scroll::Delta(delta));
        while self.term.grid().display_offset() > 0 && self.top_row_is_blank() {
            self.term.scroll_display(Scroll::Delta(-1));
        }
    }

    fn top_row_is_blank(&self) -> bool {
        self.term
            .renderable_content()
            .display_iter
            .take(self.size.cols)
            .all(|indexed| indexed.cell.c == ' ')
    }

    pub fn snapshot(&self) -> GridSnapshot {
        let content = self.term.renderable_content();
        let cols = self.size.cols;
        let rows = self.size.rows;
        let mut lines = Vec::with_capacity(rows);
        let mut cells = Vec::with_capacity(cols);
        let mut wrapped = false;
        let mut hyperlinks = Vec::new();
        // The OSC 8 run currently being read: its start column and link. A run
        // closes at the row boundary, on a cell without a link, or when the
        // link changes.
        let mut open_run: Option<(usize, alacritty_terminal::term::cell::Hyperlink)> = None;

        for (index, indexed) in content.display_iter.enumerate() {
            let cell = indexed.cell;
            if index > 0 && index % cols == 0 {
                // The boundary lands before the new row's first cell, so the
                // run closes against the row that was just filled.
                if let Some((start, link)) = open_run.take() {
                    hyperlinks.push(HyperlinkRun {
                        row: lines.len(),
                        start,
                        end: cells.len(),
                        uri: link.uri().to_string(),
                    });
                }
                lines.push(RowSnapshot {
                    cells: std::mem::take(&mut cells),
                    wrapped,
                });
                cells = Vec::with_capacity(cols);
            }
            wrapped = cell.flags.contains(Flags::WRAPLINE);
            let col = index % cols;
            match cell.hyperlink() {
                Some(link) => match open_run.take() {
                    Some((start, previous)) if previous == link => {
                        open_run = Some((start, previous));
                    }
                    Some((start, previous)) => {
                        hyperlinks.push(HyperlinkRun {
                            row: lines.len(),
                            start,
                            end: col,
                            uri: previous.uri().to_string(),
                        });
                        open_run = Some((col, link));
                    }
                    None => open_run = Some((col, link)),
                },
                None => {
                    if let Some((start, link)) = open_run.take() {
                        hyperlinks.push(HyperlinkRun {
                            row: lines.len(),
                            start,
                            end: col,
                            uri: link.uri().to_string(),
                        });
                    }
                }
            }
            cells.push(convert_cell(cell));
        }
        if let Some((start, link)) = open_run.take() {
            hyperlinks.push(HyperlinkRun {
                row: lines.len(),
                start,
                end: cells.len(),
                uri: link.uri().to_string(),
            });
        }
        if !cells.is_empty() {
            lines.push(RowSnapshot { cells, wrapped });
        }
        lines.truncate(rows);
        // The run indexes above counted rows before the viewport was clamped;
        // a run past the kept rows has no cells to underline.
        hyperlinks.retain(|run| run.row < lines.len());
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

        let shape = map_cursor_shape(content.cursor.shape);
        let visible = content.cursor.shape != AnsiCursorShape::Hidden
            && content.mode.contains(TermMode::SHOW_CURSOR);
        let blinking = self.term.cursor_style().blinking;

        let cursor = if content.display_offset == 0 {
            let row = content.cursor.point.line.0.max(0) as usize;
            CursorState {
                row: row.min(rows.saturating_sub(1)),
                logical: content.cursor.point.column.0.min(cols),
                affinity: Affinity::Before,
                shape,
                visible,
                blinking,
            }
        } else {
            CursorState {
                row: 0,
                logical: 0,
                affinity: Affinity::Before,
                shape,
                visible,
                blinking,
            }
        };

        GridSnapshot {
            cols,
            rows,
            lines,
            cursor,
            display_offset: content.display_offset,
            history: self.scrollable_history(),
            input_start: self.input_start,
            hyperlinks,
        }
    }

    /// Collects the scrollback and the viewport as plain text, one row per
    /// line. Trailing blanks inside a row and the unused rows below the cursor
    /// are trimmed, and the result keeps a single final newline.
    pub fn all_text(&self) -> String {
        let grid = self.term.grid();
        let history = grid.history_size() as i32;
        let rows = self.size.rows as i32;
        let mut out = String::new();
        for line in -history..rows {
            let row = &grid[Line(line)];
            let mut text = String::new();
            for cell in row[..].iter() {
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    continue;
                }
                text.push(if cell.c == '\0' { ' ' } else { cell.c });
                if let Some(extra) = cell.zerowidth() {
                    for ch in extra {
                        text.push(*ch);
                    }
                }
            }
            out.push_str(text.trim_end());
            out.push('\n');
        }
        while out.ends_with("\n\n") {
            out.pop();
        }
        out
    }

    /// Finds every occurrence of `needle` in the full grid, including scrollback.
    ///
    /// The match is ASCII case-insensitive and row-local: a needle split across
    /// a wrapped line is not reported. Returns one [`Match`] per occurrence in
    /// grid order (history first, then the viewport).
    pub fn find(&self, needle: &str) -> Vec<Match> {
        if needle.is_empty() {
            return Vec::new();
        }
        let needle: Vec<char> = needle.chars().collect();
        let grid = self.term.grid();
        let history = grid.history_size() as i32;
        let rows = self.size.rows as i32;
        let mut hits = Vec::new();
        for line in -history..rows {
            let row = &grid[Line(line)];
            let cells: Vec<char> = row[..].iter().map(|cell| cell.c).collect();
            if cells.len() < needle.len() {
                continue;
            }
            for start in 0..=cells.len() - needle.len() {
                let matched = cells[start..start + needle.len()]
                    .iter()
                    .zip(&needle)
                    .all(|(a, b)| a.eq_ignore_ascii_case(b));
                if matched {
                    hits.push(Match {
                        line,
                        start,
                        end: start + needle.len(),
                    });
                }
            }
        }
        hits
    }

    /// Scrolls the viewport so the grid line `line` is visible.
    ///
    /// Negative `line` values name history rows; a non-negative line is already
    /// in the viewport, so the display snaps back to the bottom.
    pub fn reveal_line(&mut self, line: i32) {
        let history = self.history_size() as i32;
        let current_top = -(self.display_offset() as i32);
        let desired_top = if line < 0 { line.clamp(-history, 0) } else { 0 };
        let delta = current_top - desired_top;
        if delta != 0 {
            self.scroll_display(delta);
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

/// The OSC 133 marks the prompt wrapper emits around the editable input line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PromptMark {
    /// `ESC ] 133 ; A BEL` - a new prompt: the previous input is gone.
    PromptStart,
    /// `ESC ] 133 ; B BEL` - editable input starts at the current cursor.
    InputStart,
}

/// Decodes a complete OSC sequence, ignoring every code but the prompt marks
/// this terminal tracks. The terminator is either BEL or ST (`ESC \`).
fn osc_mark(sequence: &[u8]) -> Option<PromptMark> {
    let end = if sequence.ends_with(&[0x07]) {
        sequence.len().saturating_sub(1)
    } else if sequence.ends_with(b"\x1b\\") {
        sequence.len().saturating_sub(2)
    } else {
        return None;
    };
    match sequence.get(2..end)? {
        b"133;A" => Some(PromptMark::PromptStart),
        b"133;B" => Some(PromptMark::InputStart),
        _ => None,
    }
}

/// Where the next OSC sequence sits inside `bytes`.
struct OscScan {
    /// Bytes before the sequence.
    offset: usize,
    /// Length of the whole sequence including both delimiters, or `None` when
    /// the sequence has not been terminated yet.
    len: Option<usize>,
}

/// Finds the next OSC sequence, stopping at the first unterminated one so its
/// bytes can be held back. A trailing lone `ESC` also reports as unterminated:
/// the next chunk may complete it into an OSC.
fn find_osc(bytes: &[u8]) -> Option<OscScan> {
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != 0x1b {
            index += 1;
            continue;
        }
        if index + 1 == bytes.len() {
            return Some(OscScan {
                offset: index,
                len: None,
            });
        }
        if bytes[index + 1] != b']' {
            index += 1;
            continue;
        }

        let mut cursor = index + 2;
        while cursor < bytes.len() {
            if bytes[cursor] == 0x07 {
                return Some(OscScan {
                    offset: index,
                    len: Some(cursor + 1 - index),
                });
            }
            if bytes[cursor] == 0x1b && cursor + 1 < bytes.len() && bytes[cursor + 1] == b'\\' {
                return Some(OscScan {
                    offset: index,
                    len: Some(cursor + 2 - index),
                });
            }
            cursor += 1;
        }
        return Some(OscScan {
            offset: index,
            len: None,
        });
    }
    None
}

fn map_cursor_shape(shape: AnsiCursorShape) -> CursorShape {
    match shape {
        AnsiCursorShape::Block => CursorShape::Block,
        AnsiCursorShape::Underline => CursorShape::Underline,
        AnsiCursorShape::Beam => CursorShape::Beam,
        AnsiCursorShape::HollowBlock => CursorShape::HollowBlock,
        AnsiCursorShape::Hidden => CursorShape::Block,
    }
}

fn convert_cell(cell: &TermCell) -> Cell {
    // Bold is synthesized from the coverage mask; italic and dim remain
    // deferred because the renderer has no italic face or dim blending.
    let style = Style {
        fg: map_fg(cell.fg),
        bg: map_bg(cell.bg),
        bold: cell.flags.contains(Flags::BOLD),
        inverse: cell.flags.contains(Flags::INVERSE),
        hidden: cell.flags.contains(Flags::HIDDEN),
        underline: cell.flags.contains(Flags::UNDERLINE),
        strikethrough: cell.flags.contains(Flags::STRIKEOUT),
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

    fn terminal(cols: usize, rows: usize, scrollback: usize) -> Terminal {
        Terminal::new(cols, rows, scrollback)
    }

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
    fn row_cells_returns_the_row_or_nothing() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"hello");
        let snapshot = term.snapshot();
        let cells = snapshot.row_cells(0);
        assert_eq!(cells.len(), 20);
        assert_eq!(cells[0].text, "h");
        assert_eq!(cells[4].text, "o");
        assert!(
            snapshot.row_cells(5).is_empty(),
            "a row outside the snapshot has no cells"
        );
        assert!(snapshot.row_cells(usize::MAX).is_empty());
    }

    #[test]
    fn osc8_hyperlinks_become_row_runs() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(
            b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ and \x1b]8;;https://other.example\x1b\\more\x1b]8;;\x1b\\",
        );
        let snapshot = term.snapshot();
        assert_eq!(
            snapshot.hyperlinks,
            vec![
                HyperlinkRun {
                    row: 0,
                    start: 0,
                    end: 4,
                    uri: "https://example.com".into()
                },
                HyperlinkRun {
                    row: 0,
                    start: 9,
                    end: 13,
                    uri: "https://other.example".into()
                },
            ]
        );
    }

    #[test]
    fn a_hyperlink_that_wraps_produces_one_run_per_row() {
        let mut term = Terminal::new(10, 5, 1_000);
        term.feed(b"\x1b]8;;https://example.com\x1b\\abcdefghijkl\x1b]8;;\x1b\\");
        let snapshot = term.snapshot();
        assert_eq!(
            snapshot.hyperlinks,
            vec![
                HyperlinkRun {
                    row: 0,
                    start: 0,
                    end: 10,
                    uri: "https://example.com".into()
                },
                HyperlinkRun {
                    row: 1,
                    start: 0,
                    end: 2,
                    uri: "https://example.com".into()
                },
            ]
        );
    }

    #[test]
    fn plain_output_has_no_hyperlink_runs() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"https://example.com");
        assert!(term.snapshot().hyperlinks.is_empty());
    }

    #[test]
    fn sgr_color_is_captured() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"\x1b[31mred");
        let snapshot = term.snapshot();
        assert_eq!(snapshot.lines[0].cells[0].style.fg, Color::Indexed(1));
    }

    #[test]
    fn sgr_bold_is_captured_and_off_by_default() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"plain\x1b[1mbold");
        let snapshot = term.snapshot();
        assert!(!snapshot.lines[0].cells[0].style.bold);
        assert!(snapshot.lines[0].cells[5].style.bold);
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
    fn scroll_display_stops_at_the_first_text_row() {
        let mut term = Terminal::new(20, 3, 1_000);
        // Two blank lines reach the scrollback before the shell writes anything.
        term.feed(b"\r\n\r\nprompt\r\n");
        for index in 0..10 {
            term.feed(format!("line{index}\r\n").as_bytes());
        }

        term.scroll_display(1_000);
        let snapshot = term.snapshot();
        let top: String = snapshot.lines[0]
            .cells
            .iter()
            .map(|cell| cell.text.as_str())
            .collect();
        assert_eq!(top.trim(), "prompt", "blank history rows must stay hidden");
    }

    #[test]
    fn scrollable_history_ends_at_the_first_text_row() {
        let mut term = Terminal::new(20, 3, 1_000);
        // Two blank lines reach the scrollback before the shell writes anything.
        term.feed(b"\r\n\r\nprompt\r\n");
        for index in 0..10 {
            term.feed(format!("line{index}\r\n").as_bytes());
        }

        let scrollable = term.scrollable_history();
        assert!(scrollable < term.history_size());
        term.scroll_display(1_000);
        assert_eq!(term.display_offset(), scrollable);
        // The scrollbar maps `display_offset` over `snapshot.history`, so the
        // two must agree for the thumb to reach the track top.
        assert_eq!(term.snapshot().history, scrollable);
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

    #[test]
    fn cursor_shape_and_visibility_are_captured() {
        let mut term = Terminal::new(20, 5, 1_000);
        assert_eq!(term.snapshot().cursor.shape, CursorShape::Beam);

        term.feed(b"\x1b[4 q");
        let cursor = term.snapshot().cursor;
        assert_eq!(cursor.shape, CursorShape::Underline);
        assert!(cursor.visible);

        term.feed(b"\x1b[6 q");
        assert_eq!(term.snapshot().cursor.shape, CursorShape::Beam);

        term.feed(b"\x1b[?25l");
        let hidden = term.snapshot().cursor;
        assert!(!hidden.visible);
        assert_eq!(hidden.shape, CursorShape::Block);

        term.feed(b"\x1b[?25h");
        assert!(term.snapshot().cursor.visible);
    }

    #[test]
    fn cursor_blinking_follows_decscusr() {
        let mut term = Terminal::new(20, 5, 1_000);
        assert!(term.snapshot().cursor.blinking, "default is a blinking bar");

        term.feed(b"\x1b[1 q");
        let cursor = term.snapshot().cursor;
        assert_eq!(cursor.shape, CursorShape::Block);
        assert!(cursor.blinking, "DECSCUSR 1 is a blinking block");

        term.feed(b"\x1b[2 q");
        assert!(!term.snapshot().cursor.blinking, "DECSCUSR 2 is steady");

        term.feed(b"\x1b[5 q");
        let cursor = term.snapshot().cursor;
        assert_eq!(cursor.shape, CursorShape::Beam);
        assert!(cursor.blinking, "DECSCUSR 5 is a blinking bar");

        term.feed(b"\x1b[0 q");
        let cursor = term.snapshot().cursor;
        assert_eq!(cursor.shape, CursorShape::Beam);
        assert!(cursor.blinking, "DECSCUSR 0 restores the default");
    }

    #[test]
    fn feed_reports_scroll_when_output_enters_history() {
        let mut term = Terminal::new(20, 5, 100);
        assert!(!term.feed(b"hello"));

        let mut scrolled = false;
        for _ in 0..10 {
            scrolled |= term.feed(b"x\r\n");
        }
        assert!(scrolled);
    }

    #[test]
    fn in_place_repaint_below_the_cap_does_not_report_scroll() {
        let mut term = Terminal::new(20, 5, 100);
        for _ in 0..10 {
            term.feed(b"x\r\n");
        }
        assert!(!term.feed(b"\x1b[HX"));
    }

    #[test]
    fn feed_reports_scroll_at_the_cap_even_without_growth() {
        let mut term = Terminal::new(20, 5, 10);
        for _ in 0..40 {
            term.feed(b"x\r\n");
        }
        assert!(term.feed(b"\x1b[HX"));
    }

    #[test]
    fn entering_the_alternate_screen_reports_a_change() {
        let mut term = Terminal::new(20, 5, 100);
        for _ in 0..10 {
            term.feed(b"x\r\n");
        }
        assert!(term.feed(b"\x1b[?1049h"));
    }

    #[test]
    fn title_events_carry_the_session_id() {
        match user_event_for(7, alacritty_terminal::event::Event::Title("build".into())) {
            Some(crate::event::UserEvent::Title { session, title }) => {
                assert_eq!(session, 7);
                assert_eq!(title, "build");
            }
            _ => panic!("expected a Title event"),
        }
    }

    #[test]
    fn reset_title_and_clipboard_events_carry_the_session_id() {
        match user_event_for(3, alacritty_terminal::event::Event::ResetTitle) {
            Some(crate::event::UserEvent::ResetTitle { session }) => assert_eq!(session, 3),
            _ => panic!("expected a ResetTitle event"),
        }
        match user_event_for(
            4,
            alacritty_terminal::event::Event::ClipboardStore(
                alacritty_terminal::term::ClipboardType::Clipboard,
                "copied".into(),
            ),
        ) {
            Some(crate::event::UserEvent::ClipboardStore(text)) => assert_eq!(text, "copied"),
            _ => panic!("expected a ClipboardStore event"),
        }
    }

    #[test]
    fn unrelated_terminal_events_are_not_bridged() {
        assert!(user_event_for(1, alacritty_terminal::event::Event::Bell).is_none());
    }

    #[test]
    fn all_text_includes_scrollback_and_trimmed_rows() {
        let mut terminal = Terminal::new(10, 3, 100);
        terminal.feed(b"first\r\nsecond   \r\n");
        let text = terminal.all_text();
        assert!(text.contains("first\nsecond"));
        // An ordinary buffer keeps one final newline while the unused
        // viewport rows collapse.
        assert_eq!(text, "first\nsecond\n");
    }

    #[test]
    fn all_text_keeps_rows_above_the_viewport() {
        let mut terminal = Terminal::new(10, 3, 100);
        for index in 0..6 {
            terminal.feed(format!("line{index}\r\n").as_bytes());
        }
        assert!(terminal.history_size() > 0);
        let text = terminal.all_text();
        assert!(text.starts_with("line0\nline1\n"), "got {text:?}");
        assert!(text.ends_with("line5\n"), "got {text:?}");
    }

    #[test]
    fn all_text_keeps_combining_marks() {
        let mut terminal = Terminal::new(10, 3, 100);
        terminal.feed("بَ\r\n".as_bytes());
        let text = terminal.all_text();
        assert!(text.contains("بَ"), "got {text:?}");
    }

    #[test]
    fn find_is_ascii_case_insensitive_and_reports_columns() {
        let mut terminal = terminal(20, 3, 100);
        terminal.feed(b"alpha BETA\r\nbeta\r\n");
        let hits = terminal.find("beta");
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0],
            crate::term::Match {
                line: 0,
                start: 6,
                end: 10
            }
        );
    }

    #[test]
    fn an_empty_query_has_no_matches() {
        let terminal = terminal(20, 3, 100);
        assert!(terminal.find("").is_empty());
    }

    #[test]
    fn history_size_tracks_scrolled_rows() {
        let mut terminal = Terminal::new(10, 5, 100);
        terminal.feed(b"a\r\nb\r\nc\r\nd\r\ne\r\nf\r\ng\r\nh\r\ni\r\nj\r\nk");
        assert!(terminal.history_size() > 0);
        assert_eq!(terminal.snapshot().history, terminal.history_size());
    }

    #[test]
    fn tracks_osc_133_marks() {
        let mut term = Terminal::new(80, 24, 100);
        term.feed(b"\x1b]133;A\x07PS> ");
        term.feed(b"\x1b]133;B\x07teh");
        let snapshot = term.snapshot();
        let start = snapshot.input_start.expect("mark recorded");
        assert_eq!(start.col, 4);
        assert_eq!(snapshot.cursor.logical, 7);
    }

    #[test]
    fn clears_the_mark_on_a_new_prompt() {
        let mut term = Terminal::new(80, 24, 100);
        term.feed(b"\x1b]133;B\x07abc");
        term.feed(b"\r\n\x1b]133;A\x07PS> ");
        assert!(term.snapshot().input_start.is_none());
    }

    #[test]
    fn survives_a_split_escape() {
        let mut term = Terminal::new(80, 24, 100);
        term.feed(b"\x1b]133");
        term.feed(b";B\x07teh");
        assert!(term.snapshot().input_start.is_some());
        assert_eq!(term.snapshot().cursor.logical, 3);
    }

    #[test]
    fn records_the_mark_with_an_st_terminator() {
        let mut term = Terminal::new(80, 24, 100);
        term.feed(b"\x1b]133;A\x1b\\PS> ");
        term.feed(b"\x1b]133;B\x1b\\teh");
        let snapshot = term.snapshot();
        let start = snapshot.input_start.expect("mark recorded");
        assert_eq!(start.col, 4);
        assert_eq!(snapshot.cursor.logical, 7);
    }

    #[test]
    fn keeps_a_split_mark_after_preceding_text() {
        let mut term = Terminal::new(80, 24, 100);
        term.feed(b"PS> \x1b]133;");
        assert!(term.snapshot().input_start.is_none());
        term.feed(b"B\x07teh");
        let snapshot = term.snapshot();
        assert_eq!(snapshot.input_start.expect("mark recorded").col, 4);
        assert_eq!(snapshot.cursor.logical, 7);
    }

    #[test]
    fn keeps_a_mark_split_between_esc_and_the_bracket() {
        let mut term = Terminal::new(80, 24, 100);
        term.feed(b"PS> \x1b");
        term.feed(b"]133;B\x07teh");
        let snapshot = term.snapshot();
        assert_eq!(snapshot.input_start.expect("mark recorded").col, 4);
        assert_eq!(snapshot.cursor.logical, 7);
    }

    #[test]
    fn a_runaway_osc_keeps_the_pending_buffer_bounded() {
        let mut term = Terminal::new(80, 24, 100);
        term.feed(b"\x1b]133;A");
        term.feed(&vec![b'x'; MAX_PENDING * 2]);
        let pending = term.pending_len();
        assert!(pending <= MAX_PENDING, "pending grew to {pending} bytes");

        // Sanity check only: terminating the sequence lets later output through.
        term.feed(b"\x07ok");
        let snapshot = term.snapshot();
        assert_eq!(snapshot.lines[0].cells[0].text, "o");
        assert_eq!(snapshot.cursor.logical, 2);
    }

    #[test]
    fn passes_other_osc_sequences_through() {
        let mut term = Terminal::new(80, 24, 100);
        term.feed(b"\x1b]0;title\x07hello");
        assert_eq!(term.snapshot().cursor.logical, 5);
        assert!(term.snapshot().input_start.is_none());
    }

    #[test]
    fn text_attributes_are_carried_into_the_snapshot() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"\x1b[7mR\x1b[0m\x1b[8mH\x1b[0m\x1b[4mU\x1b[0m\x1b[9mS\x1b[0m");
        let cells = &term.snapshot().lines[0].cells;
        assert!(cells[0].style.inverse);
        assert!(cells[1].style.hidden);
        assert!(cells[2].style.underline);
        assert!(cells[3].style.strikethrough);
        assert!(!cells[4].style.inverse, "reset must clear attributes");
    }
}
