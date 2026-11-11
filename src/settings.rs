//! The Settings editor's pure state: rows, adjustment, inline editing,
//! validation, viewport, and hit-testing.
//!
//! The model never touches the window, the renderer, or the file. The app
//! feeds it keys and pointer positions, then applies whatever configuration it
//! changed.

use std::path::Path;

use unicode_width::UnicodeWidthChar;

use crate::config::{BaseDirection, Config, ThemeConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnterAction {
    Stay,
    Changed,
    Save,
    Revert,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeAction {
    Stay,
    Revert,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitOutcome {
    Unchanged,
    Changed,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickAction {
    Select,
    Adjust(isize),
    Save,
    Revert,
}

/// Column where a row's value or control begins. Clicking left of it selects
/// the row without changing its value.
pub const VALUE_COL: usize = 22;

/// Inner width of a row label, after the two-space indent.
const LABEL_WIDTH: usize = 20;

/// The scrollback sizes Left/Right step through. A value outside the list
/// moves to the next step in the pressed direction.
const SCROLLBACK_STEPS: [usize; 7] = [1_000, 5_000, 10_000, 25_000, 50_000, 100_000, 200_000];

/// One editable theme colour. The order is the row order in [`ROWS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeColor {
    Background,
    Foreground,
    Cursor,
    Selection,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
}

impl ThemeColor {
    fn label(self) -> &'static str {
        match self {
            Self::Background => "Background",
            Self::Foreground => "Foreground",
            Self::Cursor => "Cursor",
            Self::Selection => "Selection",
            Self::Black => "Black",
            Self::Red => "Red",
            Self::Green => "Green",
            Self::Yellow => "Yellow",
            Self::Blue => "Blue",
            Self::Magenta => "Magenta",
            Self::Cyan => "Cyan",
            Self::White => "White",
            Self::BrightBlack => "Bright black",
            Self::BrightRed => "Bright red",
            Self::BrightGreen => "Bright green",
            Self::BrightYellow => "Bright yellow",
            Self::BrightBlue => "Bright blue",
            Self::BrightMagenta => "Bright magenta",
            Self::BrightCyan => "Bright cyan",
            Self::BrightWhite => "Bright white",
        }
    }

    fn value(self, theme: &ThemeConfig) -> &str {
        match self {
            Self::Background => &theme.background,
            Self::Foreground => &theme.foreground,
            Self::Cursor => &theme.cursor,
            Self::Selection => &theme.selection,
            Self::Black => &theme.black,
            Self::Red => &theme.red,
            Self::Green => &theme.green,
            Self::Yellow => &theme.yellow,
            Self::Blue => &theme.blue,
            Self::Magenta => &theme.magenta,
            Self::Cyan => &theme.cyan,
            Self::White => &theme.white,
            Self::BrightBlack => &theme.bright_black,
            Self::BrightRed => &theme.bright_red,
            Self::BrightGreen => &theme.bright_green,
            Self::BrightYellow => &theme.bright_yellow,
            Self::BrightBlue => &theme.bright_blue,
            Self::BrightMagenta => &theme.bright_magenta,
            Self::BrightCyan => &theme.bright_cyan,
            Self::BrightWhite => &theme.bright_white,
        }
    }

    fn set(self, theme: &mut ThemeConfig, value: String) {
        match self {
            Self::Background => theme.background = value,
            Self::Foreground => theme.foreground = value,
            Self::Cursor => theme.cursor = value,
            Self::Selection => theme.selection = value,
            Self::Black => theme.black = value,
            Self::Red => theme.red = value,
            Self::Green => theme.green = value,
            Self::Yellow => theme.yellow = value,
            Self::Blue => theme.blue = value,
            Self::Magenta => theme.magenta = value,
            Self::Cyan => theme.cyan = value,
            Self::White => theme.white = value,
            Self::BrightBlack => theme.bright_black = value,
            Self::BrightRed => theme.bright_red = value,
            Self::BrightGreen => theme.bright_green = value,
            Self::BrightYellow => theme.bright_yellow = value,
            Self::BrightBlue => theme.bright_blue = value,
            Self::BrightMagenta => theme.bright_magenta = value,
            Self::BrightCyan => theme.bright_cyan = value,
            Self::BrightWhite => theme.bright_white = value,
        }
    }
}

/// One editable row, in display order within its section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsRow {
    FontFamily,
    FontSize,
    LineHeight,
    ArabicFont,
    Color(ThemeColor),
    BidiDirection,
    Padding,
    Scrollback,
    CopyOnSelect,
    TrimTrailing,
    DynamicTitle,
    Save,
    Revert,
}

pub const ROWS: [SettingsRow; 32] = [
    SettingsRow::FontFamily,
    SettingsRow::FontSize,
    SettingsRow::LineHeight,
    SettingsRow::ArabicFont,
    SettingsRow::Color(ThemeColor::Background),
    SettingsRow::Color(ThemeColor::Foreground),
    SettingsRow::Color(ThemeColor::Cursor),
    SettingsRow::Color(ThemeColor::Selection),
    SettingsRow::Color(ThemeColor::Black),
    SettingsRow::Color(ThemeColor::Red),
    SettingsRow::Color(ThemeColor::Green),
    SettingsRow::Color(ThemeColor::Yellow),
    SettingsRow::Color(ThemeColor::Blue),
    SettingsRow::Color(ThemeColor::Magenta),
    SettingsRow::Color(ThemeColor::Cyan),
    SettingsRow::Color(ThemeColor::White),
    SettingsRow::Color(ThemeColor::BrightBlack),
    SettingsRow::Color(ThemeColor::BrightRed),
    SettingsRow::Color(ThemeColor::BrightGreen),
    SettingsRow::Color(ThemeColor::BrightYellow),
    SettingsRow::Color(ThemeColor::BrightBlue),
    SettingsRow::Color(ThemeColor::BrightMagenta),
    SettingsRow::Color(ThemeColor::BrightCyan),
    SettingsRow::Color(ThemeColor::BrightWhite),
    SettingsRow::BidiDirection,
    SettingsRow::Padding,
    SettingsRow::Scrollback,
    SettingsRow::CopyOnSelect,
    SettingsRow::TrimTrailing,
    SettingsRow::DynamicTitle,
    SettingsRow::Save,
    SettingsRow::Revert,
];

impl SettingsRow {
    fn label(self) -> &'static str {
        match self {
            Self::FontFamily => "Font family",
            Self::FontSize => "Font size",
            Self::LineHeight => "Line height",
            Self::ArabicFont => "Arabic font",
            Self::Color(color) => color.label(),
            Self::BidiDirection => "Bidi direction",
            Self::Padding => "Padding",
            Self::Scrollback => "Scrollback",
            Self::CopyOnSelect => "Copy on select",
            Self::TrimTrailing => "Trim trailing",
            Self::DynamicTitle => "Dynamic title",
            Self::Save => "Save",
            Self::Revert => "Revert",
        }
    }

    fn is_text(self) -> bool {
        matches!(self, Self::FontFamily | Self::ArabicFont | Self::Color(_))
    }

    fn is_toggle(self) -> bool {
        matches!(
            self,
            Self::CopyOnSelect | Self::TrimTrailing | Self::DynamicTitle
        )
    }

    fn is_action(self) -> bool {
        matches!(self, Self::Save | Self::Revert)
    }

    fn value(self, config: &Config) -> String {
        match self {
            Self::FontFamily => config.font.family.clone(),
            Self::FontSize => format!("{:.0}", config.font.size),
            Self::LineHeight => format!("{:.2}", config.font.line_height),
            Self::ArabicFont => {
                if config.font.arabic_family.is_empty() {
                    "(auto)".to_string()
                } else {
                    config.font.arabic_family.clone()
                }
            }
            Self::Color(color) => color.value(&config.theme).to_string(),
            Self::BidiDirection => match config.bidi.base_direction {
                BaseDirection::Ltr => "Ltr".to_string(),
                BaseDirection::Rtl => "Rtl".to_string(),
                BaseDirection::Auto => "Auto".to_string(),
            },
            Self::Padding => config.window.padding.to_string(),
            Self::Scrollback => human_scrollback(config.scrollback.lines),
            Self::CopyOnSelect => toggle(config.selection.copy_on_select),
            Self::TrimTrailing => toggle(config.selection.trim_trailing),
            Self::DynamicTitle => toggle(config.window.dynamic_title),
            Self::Save | Self::Revert => String::new(),
        }
    }

    fn edit_value(self, config: &Config) -> String {
        match self {
            Self::FontFamily => config.font.family.clone(),
            Self::ArabicFont => config.font.arabic_family.clone(),
            Self::Color(color) => color.value(&config.theme).to_string(),
            _ => String::new(),
        }
    }

    /// Applies one validated inline-edit value. Returns whether the config
    /// changed.
    fn apply_edit(self, text: &str, config: &mut Config) -> Result<bool, String> {
        match self {
            Self::FontFamily => {
                let value = text.trim();
                if value.is_empty() {
                    return Err("Font family must not be empty".to_string());
                }
                let changed = config.font.family != value;
                config.font.family = value.to_string();
                Ok(changed)
            }
            Self::ArabicFont => {
                let value = text.trim();
                let changed = config.font.arabic_family != value;
                config.font.arabic_family = value.to_string();
                Ok(changed)
            }
            Self::Color(color) => {
                let Some(value) = normalize_color(text) else {
                    return Err(format!("{} must be #RRGGBB", color.label()));
                };
                let changed = color.value(&config.theme) != value;
                color.set(&mut config.theme, value);
                Ok(changed)
            }
            _ => Err("This row does not take text".to_string()),
        }
    }
}

/// A section heading, or the editable row it precedes.
#[derive(Debug, Clone, Copy)]
enum Display {
    Header(&'static str),
    Row(usize),
}

/// The page's whole display list. Scroll offsets index into it.
const DISPLAY: [Display; 36] = [
    Display::Header("Appearance"),
    Display::Row(0),
    Display::Row(1),
    Display::Row(2),
    Display::Row(3),
    Display::Header("Theme"),
    Display::Row(4),
    Display::Row(5),
    Display::Row(6),
    Display::Row(7),
    Display::Row(8),
    Display::Row(9),
    Display::Row(10),
    Display::Row(11),
    Display::Row(12),
    Display::Row(13),
    Display::Row(14),
    Display::Row(15),
    Display::Row(16),
    Display::Row(17),
    Display::Row(18),
    Display::Row(19),
    Display::Row(20),
    Display::Row(21),
    Display::Row(22),
    Display::Row(23),
    Display::Header("Behavior"),
    Display::Row(24),
    Display::Row(25),
    Display::Row(26),
    Display::Row(27),
    Display::Row(28),
    Display::Row(29),
    Display::Header("Actions"),
    Display::Row(30),
    Display::Row(31),
];

#[derive(Debug, Clone)]
struct Edit {
    row: usize,
    buffer: String,
}

#[derive(Debug, Clone)]
pub struct SettingsState {
    selected: usize,
    scroll: usize,
    edit: Option<Edit>,
    dirty: bool,
    saved: bool,
    error: Option<String>,
}

impl SettingsState {
    pub fn new() -> Self {
        Self {
            selected: 0,
            scroll: 0,
            edit: None,
            dirty: false,
            saved: false,
            error: None,
        }
    }

    // Only the tests read the raw selection and scroll; the app paints through
    // `render` and moves through the navigation methods.
    #[cfg(test)]
    pub fn selected(&self) -> usize {
        self.selected
    }

    #[cfg(test)]
    pub fn scroll(&self) -> usize {
        self.scroll
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn editing(&self) -> bool {
        self.edit.is_some()
    }

    /// The body rows between the title and the status/footer pair.
    pub fn body_rows(rows: usize) -> usize {
        rows.saturating_sub(3)
    }

    pub fn move_selection(&mut self, delta: isize, rows: usize) {
        let last = ROWS.len() - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last as isize) as usize;
        self.scroll_into_view(rows);
    }

    pub fn page(&mut self, delta: isize, rows: usize) {
        let step = Self::body_rows(rows).max(1) as isize;
        self.move_selection(delta.signum() * step, rows);
    }

    pub fn home(&mut self, _rows: usize) {
        self.selected = 0;
        self.scroll = 0;
    }

    pub fn end(&mut self, rows: usize) {
        self.selected = ROWS.len() - 1;
        self.scroll_into_view(rows);
    }

    pub fn adjust(&mut self, delta: isize, config: &mut Config) -> bool {
        if delta == 0 {
            return false;
        }
        let row = ROWS[self.selected];
        let changed = match row {
            SettingsRow::FontSize => {
                let next = (config.font.size.round() + delta as f32).clamp(6.0, 48.0);
                if next == config.font.size {
                    false
                } else {
                    config.font.size = next;
                    true
                }
            }
            SettingsRow::LineHeight => {
                let next =
                    ((config.font.line_height + delta as f32 * 0.05) * 100.0).round() / 100.0;
                let next = next.clamp(0.8, 2.0);
                if next == config.font.line_height {
                    false
                } else {
                    config.font.line_height = next;
                    true
                }
            }
            SettingsRow::Padding => {
                let next = (config.window.padding as isize + delta).clamp(0, 64) as u32;
                if next == config.window.padding {
                    false
                } else {
                    config.window.padding = next;
                    true
                }
            }
            SettingsRow::Scrollback => {
                let next = scrollback_step(config.scrollback.lines, delta);
                if next == config.scrollback.lines {
                    false
                } else {
                    config.scrollback.lines = next;
                    true
                }
            }
            SettingsRow::BidiDirection => {
                config.bidi.base_direction = cycle_direction(config.bidi.base_direction, delta);
                true
            }
            SettingsRow::CopyOnSelect => {
                config.selection.copy_on_select = !config.selection.copy_on_select;
                true
            }
            SettingsRow::TrimTrailing => {
                config.selection.trim_trailing = !config.selection.trim_trailing;
                true
            }
            SettingsRow::DynamicTitle => {
                config.window.dynamic_title = !config.window.dynamic_title;
                true
            }
            _ => false,
        };
        if changed {
            self.mark_unsaved();
        }
        changed
    }

    pub fn enter(&mut self, config: &mut Config) -> EnterAction {
        if self.edit.is_some() {
            return match self.commit_edit(config) {
                CommitOutcome::Changed => EnterAction::Changed,
                CommitOutcome::Unchanged | CommitOutcome::Invalid => EnterAction::Stay,
            };
        }
        let row = ROWS[self.selected];
        if row.is_text() {
            self.begin_edit(config);
            return EnterAction::Stay;
        }
        if row.is_toggle() {
            self.adjust(1, config);
            return EnterAction::Changed;
        }
        match row {
            SettingsRow::Save => EnterAction::Save,
            SettingsRow::Revert => EnterAction::Revert,
            _ => EnterAction::Stay,
        }
    }

    pub fn escape(&mut self) -> EscapeAction {
        if self.edit.is_some() {
            self.cancel_edit();
            return EscapeAction::Stay;
        }
        if self.dirty {
            EscapeAction::Revert
        } else {
            EscapeAction::Close
        }
    }

    pub fn edit_char(&mut self, character: char) {
        if character.is_control() {
            return;
        }
        if let Some(edit) = self.edit.as_mut() {
            edit.buffer.push(character);
            self.error = None;
        }
    }

    pub fn edit_backspace(&mut self) {
        if let Some(edit) = self.edit.as_mut() {
            edit.buffer.pop();
            self.error = None;
        }
    }

    pub fn commit_edit(&mut self, config: &mut Config) -> CommitOutcome {
        let Some(edit) = self.edit.clone() else {
            return CommitOutcome::Unchanged;
        };
        match ROWS[edit.row].apply_edit(&edit.buffer, config) {
            Ok(changed) => {
                self.edit = None;
                self.error = None;
                if changed {
                    self.mark_unsaved();
                    CommitOutcome::Changed
                } else {
                    CommitOutcome::Unchanged
                }
            }
            Err(message) => {
                self.error = Some(message);
                CommitOutcome::Invalid
            }
        }
    }

    pub fn cancel_edit(&mut self) {
        self.edit = None;
        self.error = None;
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
        self.saved = true;
        self.error = None;
    }

    /// Recomputes the unsaved marker from the live config. The model cannot
    /// see the last saved config, so the app calls this after every change;
    /// a value that returns to the saved one clears the marker again. The
    /// error line is kept so a rejected edit stays visible.
    pub fn set_dirty(&mut self, dirty: bool) {
        self.dirty = dirty;
        if dirty {
            self.saved = false;
        }
    }

    pub fn mark_clean(&mut self) {
        self.dirty = false;
        self.saved = false;
        self.error = None;
        self.edit = None;
    }

    pub fn set_error(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
        self.saved = false;
    }

    /// Resolves a click at a logical grid row and column. `left_half` is
    /// whether the pointer is in the visual left half of the value zone; the
    /// app resolves it through `VisualMap` because an RTL row is right-
    /// anchored and the logical columns do not run left to right.
    ///
    /// Returns `None` for a section heading, a point outside the rows, or a
    /// click on another row while an inline edit is open. Ignoring those keeps
    /// the highlight, the editor, and the row `Enter` commits on the same row;
    /// the app still consumes the click in every case.
    pub fn click(
        &mut self,
        screen_row: usize,
        col: usize,
        left_half: bool,
        rows: usize,
    ) -> Option<ClickAction> {
        let (row, action) = self.hit(screen_row, col, left_half, rows)?;
        if self.edit.as_ref().is_some_and(|edit| edit.row != row) {
            return None;
        }
        self.selected = row;
        Some(action)
    }

    fn hit(
        &self,
        screen_row: usize,
        col: usize,
        left_half: bool,
        rows: usize,
    ) -> Option<(usize, ClickAction)> {
        let body = Self::body_rows(rows);
        if body == 0 || screen_row == 0 || screen_row > body {
            return None;
        }
        let index = self.clamped_scroll(body) + screen_row - 1;
        let Display::Row(row) = *DISPLAY.get(index)? else {
            return None;
        };
        let kind = ROWS[row];
        if kind.is_action() {
            return Some((
                row,
                if kind == SettingsRow::Save {
                    ClickAction::Save
                } else {
                    ClickAction::Revert
                },
            ));
        }
        if col < VALUE_COL || kind.is_text() {
            return Some((row, ClickAction::Select));
        }
        Some((row, ClickAction::Adjust(if left_half { -1 } else { 1 })))
    }

    /// Keeps the selected row inside the visible body when the grid shrinks.
    pub fn ensure_visible(&mut self, rows: usize) {
        self.scroll_into_view(rows);
    }

    /// Builds exactly `rows` lines, each at most `cols` visible characters.
    /// The title, the body viewport, the status line, and the footer are the
    /// four vertical sections.
    pub fn render(&self, config: &Config, config_path: &Path, cols: usize, rows: usize) -> String {
        if rows == 0 {
            return String::new();
        }
        let mut lines: Vec<String> = Vec::with_capacity(rows);
        lines.push(styled(
            &format!("Settings  {}", config_path.display()),
            "\x1b[1m",
            cols,
        ));
        if rows == 1 {
            return lines.join("\r\n");
        }
        let body = Self::body_rows(rows);
        let scroll = self.clamped_scroll(body);
        for offset in 0..body {
            let Some(entry) = DISPLAY.get(scroll + offset) else {
                lines.push(String::new());
                continue;
            };
            lines.push(match *entry {
                Display::Header(title) => styled(title, "\x1b[1m", cols),
                Display::Row(row) => self.row_line(row, config, cols),
            });
        }
        if rows == 2 {
            lines.push(self.status_line(cols));
            return lines.join("\r\n");
        }
        lines.push(self.status_line(cols));
        lines.push(styled(
            "↑↓ PageUp/PageDown Home/End  ←→ adjust  Enter edit  Esc revert  Ctrl+S save",
            "\x1b[2m",
            cols,
        ));
        lines.join("\r\n")
    }

    fn begin_edit(&mut self, config: &Config) {
        self.edit = Some(Edit {
            row: self.selected,
            buffer: ROWS[self.selected].edit_value(config),
        });
        self.error = None;
    }

    fn row_line(&self, row: usize, config: &Config, cols: usize) -> String {
        let kind = ROWS[row];
        let selected = self.selected == row;
        if kind.is_action() {
            return paint(format!("  {}", kind.label()), selected, cols);
        }
        let value = match &self.edit {
            Some(edit) if edit.row == row => format!("{}_", edit.buffer),
            _ => kind.value(config),
        };
        let mut line = format!("  {:<LABEL_WIDTH$}", kind.label());
        if kind.is_text() || kind.is_toggle() {
            line.push_str(&value);
        } else {
            line.push_str("◀ ");
            line.push_str(&value);
            line.push_str(" ▶");
        }
        if kind == SettingsRow::Scrollback {
            line.push_str("  new tabs only");
        }
        paint(line, selected, cols)
    }

    fn status_line(&self, cols: usize) -> String {
        if let Some(error) = &self.error {
            return styled(error, "\x1b[31m", cols);
        }
        if self.dirty {
            return styled("Unsaved changes", "\x1b[33m", cols);
        }
        if self.saved {
            return styled("Saved", "\x1b[32m", cols);
        }
        String::new()
    }

    fn mark_unsaved(&mut self) {
        self.dirty = true;
        self.saved = false;
        self.error = None;
    }

    fn display_index(&self) -> usize {
        DISPLAY
            .iter()
            .position(|entry| matches!(entry, Display::Row(row) if *row == self.selected))
            .unwrap_or(0)
    }

    fn max_scroll(&self, body: usize) -> usize {
        DISPLAY.len().saturating_sub(body)
    }

    fn clamped_scroll(&self, body: usize) -> usize {
        self.scroll.min(self.max_scroll(body))
    }

    fn scroll_into_view(&mut self, rows: usize) {
        let body = Self::body_rows(rows);
        let index = self.display_index();
        if body == 0 {
            self.scroll = index;
            return;
        }
        if index < self.scroll {
            self.scroll = index;
        } else if index >= self.scroll + body {
            self.scroll = index + 1 - body;
        }
        self.scroll = self.clamped_scroll(body);
    }
}

/// Visual column that divides a row's value zone into halves. `value_visual`
/// is the visual position of [`VALUE_COL`]: under an RTL base direction the
/// whole line is right-anchored, so the zone starts further right and the
/// midpoint moves with it.
pub fn value_midpoint(value_visual: usize, cols: usize) -> usize {
    value_visual + cols.saturating_sub(value_visual) / 2
}

fn human_scrollback(lines: usize) -> String {
    match lines {
        1_000 => "1k".to_string(),
        5_000 => "5k".to_string(),
        10_000 => "10k".to_string(),
        25_000 => "25k".to_string(),
        50_000 => "50k".to_string(),
        100_000 => "100k".to_string(),
        200_000 => "200k".to_string(),
        _ => lines.to_string(),
    }
}

fn scrollback_step(current: usize, delta: isize) -> usize {
    if delta > 0 {
        SCROLLBACK_STEPS
            .iter()
            .copied()
            .find(|step| *step > current)
            .or_else(|| SCROLLBACK_STEPS.last().copied())
            .unwrap_or(current)
    } else {
        SCROLLBACK_STEPS
            .iter()
            .rev()
            .copied()
            .find(|step| *step < current)
            .or_else(|| SCROLLBACK_STEPS.first().copied())
            .unwrap_or(current)
    }
}

fn cycle_direction(current: BaseDirection, delta: isize) -> BaseDirection {
    let directions = [BaseDirection::Ltr, BaseDirection::Rtl, BaseDirection::Auto];
    let index = directions
        .iter()
        .position(|direction| *direction == current)
        .unwrap_or(0) as isize;
    directions[(index + delta).rem_euclid(directions.len() as isize) as usize]
}

fn toggle(on: bool) -> String {
    if on {
        "[x]".to_string()
    } else {
        "[ ]".to_string()
    }
}

fn normalize_color(text: &str) -> Option<String> {
    let text = text.trim();
    if text.len() != 7 || !text.starts_with('#') {
        return None;
    }
    if !text[1..]
        .chars()
        .all(|character| character.is_ascii_hexdigit())
    {
        return None;
    }
    Some(format!("#{}", text[1..].to_ascii_lowercase()))
}

/// Truncates a line to `cols` display cells. A wide character that cannot fit
/// in the last cell is dropped rather than split, so the grid stays aligned.
fn fit(text: &str, cols: usize) -> String {
    let mut out = String::with_capacity(text.len());
    let mut width = 0;
    for character in text.chars() {
        let cells = character.width().unwrap_or(0);
        if width + cells > cols {
            break;
        }
        out.push(character);
        width += cells;
    }
    out
}

fn display_width(text: &str) -> usize {
    text.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Truncates to `cols` display cells, then applies a style prefix.
fn styled(text: &str, prefix: &str, cols: usize) -> String {
    format!("{prefix}{}\x1b[0m", fit(text, cols))
}

/// Truncates to `cols` display cells and pads a selected line to the full
/// width so the inverse band reads as one row.
fn paint(text: String, selected: bool, cols: usize) -> String {
    let text = fit(&text, cols);
    if !selected {
        return text;
    }
    let width = display_width(&text);
    let mut padded = text;
    if width < cols {
        padded.extend(std::iter::repeat(' ').take(cols - width));
    }
    format!("\x1b[7m{padded}\x1b[0m")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::default().validate()
    }

    fn path() -> &'static Path {
        Path::new("unused.toml")
    }

    fn strip_ansi(line: &str) -> String {
        let mut out = String::new();
        let mut chars = line.chars();
        while let Some(character) = chars.next() {
            if character != '\x1b' {
                out.push(character);
                continue;
            }
            for escape in chars.by_ref() {
                if escape == 'm' {
                    break;
                }
            }
        }
        out
    }

    fn rows_of(state: &SettingsState, config: &Config, rows: usize) -> Vec<String> {
        state
            .render(config, path(), 80, rows)
            .split("\r\n")
            .map(strip_ansi)
            .collect()
    }

    fn row_of(state: &SettingsState, config: &Config, rows: usize, label: &str) -> usize {
        rows_of(state, config, rows)
            .iter()
            .position(|line| line.trim_start().starts_with(label))
            .unwrap_or_else(|| panic!("no row labelled {label}"))
    }

    fn select(state: &mut SettingsState, config: &Config, rows: usize, label: &str) {
        let row = row_of(state, config, rows, label);
        let action = state.click(row, 2, false, rows);
        assert!(matches!(action, Some(ClickAction::Select)));
    }

    fn latin_row(text: &str, cols: usize) -> crate::term::RowSnapshot {
        let mut cells: Vec<crate::term::Cell> = text
            .chars()
            .map(|character| crate::term::Cell {
                text: character.to_string(),
                width: 1,
                style: crate::term::Style::default(),
            })
            .collect();
        cells.resize_with(cols, || crate::term::Cell {
            text: " ".to_string(),
            width: 1,
            style: crate::term::Style::default(),
        });
        crate::term::RowSnapshot {
            cells,
            wrapped: false,
        }
    }

    fn clear_edit(state: &mut SettingsState) {
        for _ in 0..64 {
            state.edit_backspace();
        }
    }

    #[test]
    fn arrows_clamp_and_keep_the_selection_visible() {
        let config = config();
        let mut state = SettingsState::new();
        state.move_selection(-1, 10);
        assert_eq!(state.selected(), 0);
        state.move_selection(1, 10);
        assert_eq!(state.selected(), 1);
        state.move_selection(1_000, 10);
        assert_eq!(state.selected(), 31);
        state.move_selection(-1_000, 10);
        assert_eq!(state.selected(), 0);

        // Moving far down must scroll the selection into the seven-row body.
        state.move_selection(20, 10);
        assert!(state.scroll() > 0);
        let text = state.render(&config, path(), 80, 10);
        assert!(text.split("\r\n").any(|line| {
            line.starts_with("\x1b[7m") && strip_ansi(line).contains("Bright blue")
        }));
    }

    #[test]
    fn page_keys_move_by_a_viewport_and_home_end_jump() {
        let mut state = SettingsState::new();
        state.page(1, 10);
        assert_eq!(state.selected(), SettingsState::body_rows(10));
        let after_page = state.selected();
        state.page(-1, 10);
        assert!(state.selected() < after_page);
        state.home(10);
        assert_eq!(state.selected(), 0);
        assert_eq!(state.scroll(), 0);
        state.end(10);
        assert_eq!(state.selected(), 31);
        assert!(state.scroll() > 0);
    }

    #[test]
    fn adjusting_numbers_clamps_and_marks_unsaved() {
        let mut config = config();
        let mut state = SettingsState::new();
        select(&mut state, &config, 40, "Font size");
        assert!(state.adjust(1, &mut config));
        assert_eq!(config.font.size, 17.0);
        assert!(state.is_dirty());

        config.font.size = 48.0;
        assert!(!state.adjust(1, &mut config));
        assert_eq!(config.font.size, 48.0);
        config.font.size = 6.0;
        assert!(!state.adjust(-1, &mut config));
        assert_eq!(config.font.size, 6.0);

        select(&mut state, &config, 40, "Line height");
        state.adjust(1, &mut config);
        assert!((config.font.line_height - 1.05).abs() < f32::EPSILON);
        config.font.line_height = 2.0;
        assert!(!state.adjust(1, &mut config));
        assert_eq!(config.font.line_height, 2.0);

        select(&mut state, &config, 40, "Padding");
        state.adjust(1, &mut config);
        assert_eq!(config.window.padding, 9);
        config.window.padding = 64;
        assert!(!state.adjust(1, &mut config));
        assert_eq!(config.window.padding, 64);
    }

    #[test]
    fn scrollback_cycles_through_its_steps() {
        let mut config = config();
        let mut state = SettingsState::new();
        select(&mut state, &config, 40, "Scrollback");
        assert_eq!(config.scrollback.lines, 10_000);
        state.adjust(1, &mut config);
        assert_eq!(config.scrollback.lines, 25_000);
        state.adjust(-1, &mut config);
        assert_eq!(config.scrollback.lines, 10_000);

        config.scrollback.lines = 200_000;
        assert!(!state.adjust(1, &mut config));
        assert_eq!(config.scrollback.lines, 200_000);
        config.scrollback.lines = 1_000;
        assert!(!state.adjust(-1, &mut config));
        assert_eq!(config.scrollback.lines, 1_000);

        config.scrollback.lines = 3_000;
        state.adjust(1, &mut config);
        assert_eq!(config.scrollback.lines, 5_000);
        state.adjust(-1, &mut config);
        assert_eq!(config.scrollback.lines, 1_000);
    }

    #[test]
    fn bidi_cycles_both_ways_and_toggles_flip() {
        let mut config = config();
        let mut state = SettingsState::new();
        select(&mut state, &config, 40, "Bidi direction");
        assert_eq!(
            config.bidi.base_direction,
            crate::config::BaseDirection::Ltr
        );
        state.adjust(1, &mut config);
        assert_eq!(
            config.bidi.base_direction,
            crate::config::BaseDirection::Rtl
        );
        state.adjust(1, &mut config);
        assert_eq!(
            config.bidi.base_direction,
            crate::config::BaseDirection::Auto
        );
        state.adjust(1, &mut config);
        assert_eq!(
            config.bidi.base_direction,
            crate::config::BaseDirection::Ltr
        );
        state.adjust(-1, &mut config);
        assert_eq!(
            config.bidi.base_direction,
            crate::config::BaseDirection::Auto
        );

        select(&mut state, &config, 40, "Copy on select");
        assert!(config.selection.copy_on_select);
        state.adjust(1, &mut config);
        assert!(!config.selection.copy_on_select);
        state.adjust(-1, &mut config);
        assert!(config.selection.copy_on_select);
    }

    #[test]
    fn inline_edit_commits_text_and_rejects_an_empty_family() {
        let mut config = config();
        let mut state = SettingsState::new();
        select(&mut state, &config, 40, "Font family");
        assert_eq!(state.enter(&mut config), EnterAction::Stay);
        assert!(state.editing());
        clear_edit(&mut state);
        for character in "Fira Code".chars() {
            state.edit_char(character);
        }
        assert_eq!(state.commit_edit(&mut config), CommitOutcome::Changed);
        assert_eq!(config.font.family, "Fira Code");
        assert!(!state.editing());

        assert_eq!(state.enter(&mut config), EnterAction::Stay);
        clear_edit(&mut state);
        assert_eq!(state.commit_edit(&mut config), CommitOutcome::Invalid);
        assert!(state.editing());
        assert_eq!(config.font.family, "Fira Code");
        let text = state.render(&config, path(), 80, 24);
        assert!(text.contains("must not be empty"));
    }

    #[test]
    fn inline_edit_validates_colors_and_escape_cancels() {
        let mut config = config();
        let mut state = SettingsState::new();
        select(&mut state, &config, 40, "Background");
        state.enter(&mut config);
        assert!(state.editing());
        clear_edit(&mut state);
        for character in "#112233".chars() {
            state.edit_char(character);
        }
        assert_eq!(state.commit_edit(&mut config), CommitOutcome::Changed);
        assert_eq!(config.theme.background, "#112233");

        state.enter(&mut config);
        clear_edit(&mut state);
        for character in "#12345".chars() {
            state.edit_char(character);
        }
        assert_eq!(state.commit_edit(&mut config), CommitOutcome::Invalid);
        assert!(state.editing());
        assert_eq!(config.theme.background, "#112233");
        assert!(state.render(&config, path(), 80, 24).contains("#RRGGBB"));

        assert_eq!(state.escape(), EscapeAction::Stay);
        assert!(!state.editing());
        let text = state.render(&config, path(), 80, 24);
        assert!(!text.contains("#12345"));
    }

    #[test]
    fn the_arabic_font_accepts_empty_for_auto() {
        let mut config = config();
        let mut state = SettingsState::new();
        select(&mut state, &config, 40, "Arabic font");
        state.enter(&mut config);
        assert!(state.editing());
        for character in "Arial".chars() {
            state.edit_char(character);
        }
        assert_eq!(state.commit_edit(&mut config), CommitOutcome::Changed);
        assert_eq!(config.font.arabic_family, "Arial");

        state.enter(&mut config);
        clear_edit(&mut state);
        assert_eq!(state.commit_edit(&mut config), CommitOutcome::Changed);
        assert_eq!(config.font.arabic_family, "");
        assert!(state.render(&config, path(), 80, 24).contains("(auto)"));
    }

    #[test]
    fn the_viewport_renders_exactly_the_visible_rows_and_truncates() {
        let mut config = config();
        config.font.family = "A very long font family name that cannot fit".into();
        let state = SettingsState::new();
        let text = state.render(&config, path(), 30, 24);
        let lines: Vec<&str> = text.split("\r\n").collect();
        assert_eq!(lines.len(), 24);
        for line in &lines {
            assert!(strip_ansi(line).chars().count() <= 30, "{line:?}");
        }
        assert!(text.contains("Settings"));
        assert!(text.contains("Appearance"));
        assert!(!text.contains("cannot fit"));
        assert!(state.render(&config, path(), 80, 24).contains("Ctrl+S"));
    }

    #[test]
    fn the_footer_documents_every_key() {
        let config = config();
        let state = SettingsState::new();
        let text = state.render(&config, path(), 80, 24);
        let footer = text.split("\r\n").last().unwrap_or_default();
        for key in ["PageUp/PageDown", "Home/End", "Enter", "Esc", "Ctrl+S"] {
            assert!(footer.contains(key), "footer {footer:?} omits {key}");
        }
    }

    #[test]
    fn wide_characters_truncate_by_display_cells_not_characters() {
        let mut config = config();
        config.font.family = "字体字体字体字体字体字体".into();
        let state = SettingsState::new();
        let text = state.render(&config, path(), 20, 24);
        for line in text.split("\r\n") {
            let plain = strip_ansi(line);
            let width: usize = plain.chars().map(|c| c.width().unwrap_or(0)).sum();
            assert!(width <= 20, "{line:?} is {width} cells wide");
        }
        assert!(!strip_ansi(&text).contains("字体字体字体字体字体字体"));
    }

    #[test]
    fn shrinking_the_body_scrolls_the_selection_back_into_view() {
        let mut state = SettingsState::new();
        state.move_selection(20, 40);
        assert_eq!(state.selected(), 20);
        state.ensure_visible(6);
        let body = SettingsState::body_rows(6);
        assert!(state.display_index() >= state.scroll());
        assert!(state.display_index() < state.scroll() + body);
    }

    #[test]
    fn scrolling_reveals_the_actions_and_the_selected_line_is_inverse() {
        let config = config();
        let mut state = SettingsState::new();
        let first = state.render(&config, path(), 80, 10);
        assert!(first.contains("Appearance"));
        assert!(!first.contains("Actions"));

        let family = first
            .split("\r\n")
            .find(|line| strip_ansi(line).contains("Font family"))
            .expect("the selected family row");
        assert!(family.starts_with("\x1b[7m"));

        state.end(10);
        let last = state.render(&config, path(), 80, 10);
        assert!(last.contains("Actions"));
        assert!(last.contains("Revert"));
        let revert = last
            .split("\r\n")
            .find(|line| strip_ansi(line).contains("Revert"))
            .expect("the selected revert row");
        assert!(revert.starts_with("\x1b[7m"));
    }

    #[test]
    fn the_status_line_reports_unsaved_saved_and_errors() {
        let mut config = config();
        let mut state = SettingsState::new();
        select(&mut state, &config, 40, "Font size");
        state.adjust(1, &mut config);
        assert!(state
            .render(&config, path(), 80, 24)
            .contains("Unsaved changes"));
        state.mark_saved();
        let saved = state.render(&config, path(), 80, 24);
        assert!(saved.contains("Saved"));
        assert!(!saved.contains("Unsaved changes"));
        state.set_error("disk is full");
        assert!(state
            .render(&config, path(), 80, 24)
            .contains("disk is full"));
    }

    #[test]
    fn clicking_selects_adjusts_and_runs_actions() {
        let config = config();
        let mut state = SettingsState::new();

        let size_row = row_of(&state, &config, 40, "Font size");
        assert_eq!(
            state.click(size_row, 70, false, 40),
            Some(ClickAction::Adjust(1))
        );
        assert_eq!(state.selected(), 1);
        assert_eq!(
            state.click(size_row, 24, true, 40),
            Some(ClickAction::Adjust(-1))
        );

        let family_row = row_of(&state, &config, 40, "Font family");
        assert_eq!(
            state.click(family_row, 40, true, 40),
            Some(ClickAction::Select)
        );
        assert_eq!(state.selected(), 0);

        let save_row = row_of(&state, &config, 40, "Save");
        assert_eq!(state.click(save_row, 2, false, 40), Some(ClickAction::Save));
        let revert_row = row_of(&state, &config, 40, "Revert");
        assert_eq!(
            state.click(revert_row, 2, false, 40),
            Some(ClickAction::Revert)
        );
    }

    #[test]
    fn an_rtl_row_decides_the_half_in_visual_space() {
        let cols = 40;
        // The settings text is Latin and the row is right-anchored under an
        // RTL base direction, so logical cell 22 sits far to the right.
        let text = format!("  {:<LABEL_WIDTH$}◀ 16 ▶", "Font size");
        let line = latin_row(&text, cols);
        let map = crate::visual::VisualMap::from_bidi(&line, BaseDirection::Rtl);
        let value_visual = map.visual_of(VALUE_COL);
        let middle = value_midpoint(value_visual, cols);
        assert!(value_visual > VALUE_COL, "the line is not right-anchored");

        // The visual left edge of the value zone decrements.
        let col = map.logical_at(value_visual);
        assert_eq!(col, VALUE_COL);
        let mut state = SettingsState::new();
        assert_eq!(
            state.click(3, col, value_visual < middle, cols),
            Some(ClickAction::Adjust(-1))
        );

        // A click in the visual right half increments, even though its
        // logical column is left of the logical midpoint that the old rule
        // compared against.
        let visual = (middle..cols)
            .find(|visual| map.logical_at(*visual) >= VALUE_COL)
            .expect("a value cell in the visual right half");
        let col = map.logical_at(visual);
        assert!(col < VALUE_COL + (cols - VALUE_COL) / 2);
        assert_eq!(
            state.click(3, col, visual < middle, cols),
            Some(ClickAction::Adjust(1))
        );
    }

    #[test]
    fn clicking_another_row_while_editing_is_ignored() {
        let mut config = config();
        let mut state = SettingsState::new();
        select(&mut state, &config, 40, "Font family");
        state.enter(&mut config);
        assert!(state.editing());

        let size_row = row_of(&state, &config, 40, "Font size");
        assert_eq!(state.click(size_row, 2, false, 40), None);
        assert_eq!(state.selected(), 0);
        assert!(state.editing());

        // Actions are ignored too: the open editor must be committed or
        // cancelled before another row can act.
        let save_row = row_of(&state, &config, 40, "Save");
        assert_eq!(state.click(save_row, 2, false, 40), None);
        assert_eq!(state.selected(), 0);

        // The edited row still accepts a click and can commit.
        let family_row = row_of(&state, &config, 40, "Font family");
        assert_eq!(
            state.click(family_row, 2, false, 40),
            Some(ClickAction::Select)
        );
        assert_eq!(state.commit_edit(&mut config), CommitOutcome::Unchanged);
        assert!(!state.editing());
        assert_eq!(config.font.family, "Cascadia Mono");
    }

    #[test]
    fn hit_testing_maps_screen_rows_through_a_scrolled_viewport() {
        let mut state = SettingsState::new();
        state.end(10);
        assert_eq!(state.scroll(), 29);

        // The first visible body row is Scrollback, the seventh is Revert.
        assert_eq!(state.click(1, 2, false, 10), Some(ClickAction::Select));
        assert_eq!(state.selected(), 26);
        assert_eq!(state.click(7, 2, false, 10), Some(ClickAction::Revert));
        assert_eq!(state.selected(), 31);
    }

    #[test]
    fn clearing_dirty_without_saving_drops_the_unsaved_marker() {
        let mut config = config();
        let mut state = SettingsState::new();
        select(&mut state, &config, 40, "Font size");
        state.adjust(1, &mut config);
        assert!(state.is_dirty());
        state.set_dirty(false);
        assert!(!state.is_dirty());
        assert!(!state
            .render(&config, path(), 80, 24)
            .contains("Unsaved changes"));
    }

    #[test]
    fn clicking_a_header_or_the_status_line_does_nothing() {
        let config = config();
        let mut state = SettingsState::new();
        let theme_row = row_of(&state, &config, 40, "Theme");
        assert_eq!(state.click(theme_row, 2, false, 40), None);
        let status_row = 40 - 2;
        assert_eq!(state.click(status_row, 2, false, 40), None);
        assert_eq!(state.selected(), 0);
    }
}
