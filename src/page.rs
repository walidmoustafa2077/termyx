//! In-app pages shown as tabs: Settings, the command palette, and About.
//!
//! A page has no child process. Its content is generated as terminal text and
//! fed to the tab's own `Terminal`, so the grid, theme, scrolling, selection,
//! and copy paths work without a second render path.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use unicode_width::UnicodeWidthStr;

use crate::config::Config;
use crate::palette::{Palette, PaletteCommand, PaletteContext};
use crate::settings::SettingsState;

/// Shown on the About page as the project's author.
const CREATOR: &str = "Walid Mostafa";

/// Escape sequence that hides the cursor, clears the grid and the scrollback,
/// and homes the cursor.
///
/// Pages are refilled in place, so the scrollback erase is what keeps a
/// Settings tab (polled on every config save) or a palette (refilled on every
/// keystroke) from piling the previous grids into history.
const RESET_SCREEN: &str = "\x1b[?25l\x1b[2J\x1b[3J\x1b[H";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    Settings,
    CommandPalette,
    About,
}

impl PageKind {
    pub fn title(self) -> &'static str {
        match self {
            PageKind::Settings => "Settings",
            PageKind::CommandPalette => "Command palette",
            PageKind::About => "About",
        }
    }
}

/// A shell-less tab. The palette owns its query and highlighted entry; the
/// other kinds are static views of live data.
#[derive(Debug, Clone)]
pub struct Page {
    kind: PageKind,
    /// The Settings tab's selection, viewport, inline edit, and status. The
    /// other page kinds leave it untouched.
    settings: SettingsState,
    palette: Palette,
}

impl Page {
    pub fn new(kind: PageKind) -> Self {
        Self {
            kind,
            settings: SettingsState::new(),
            palette: Palette::new(),
        }
    }

    pub fn kind(&self) -> PageKind {
        self.kind
    }

    pub fn title(&self) -> &'static str {
        self.kind.title()
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    pub fn set_palette_context(&mut self, context: PaletteContext) {
        self.palette.set_context(context);
    }

    // Reopening an existing palette tab must not keep the old query.
    pub fn reset_palette(&mut self) {
        self.palette.reset();
    }

    pub fn palette_command(&self, config: &Config) -> Option<PaletteCommand> {
        self.palette.command(config)
    }

    /// Appends typed text and returns the highlight to the first match, so the
    /// list always shows what the new query ranks highest.
    pub fn type_char(&mut self, character: char) {
        self.palette.type_char(character);
    }

    pub fn backspace(&mut self) {
        self.palette.backspace();
    }

    /// Moves the highlight by `delta`, wrapping around `count` entries.
    pub fn move_selection(&mut self, delta: isize, count: usize) {
        self.palette.move_selection(delta, count);
    }

    /// The Settings model, when this is a Settings tab.
    pub fn settings(&self) -> Option<&SettingsState> {
        (self.kind == PageKind::Settings).then_some(&self.settings)
    }

    /// The Settings model for an open Settings tab.
    pub fn settings_mut(&mut self) -> Option<&mut SettingsState> {
        (self.kind == PageKind::Settings).then_some(&mut self.settings)
    }

    /// Keeps the Settings selection inside a body of `rows` grid rows. Other
    /// page kinds have no selection to reveal.
    pub fn ensure_visible(&mut self, rows: usize) {
        if self.kind == PageKind::Settings {
            self.settings.ensure_visible(rows);
        }
    }

    /// The full screen contents for this page. Both Settings and the command
    /// palette use the grid dimensions: Settings sizes its viewport to the
    /// visible body rows, and the palette clamps its rows and width to the grid.
    pub fn text(&self, config: &Config, config_path: &Path, cols: usize, rows: usize) -> String {
        match self.kind {
            PageKind::Settings => self.settings_text(config, config_path, cols, rows),
            PageKind::CommandPalette => {
                let mut out = String::from(RESET_SCREEN);
                out.push_str(&self.palette.text(config, cols, rows));
                out
            }
            PageKind::About => about_page_text(cols, rows),
        }
    }

    fn settings_text(
        &self,
        config: &Config,
        config_path: &Path,
        cols: usize,
        rows: usize,
    ) -> String {
        let mut out = String::from(RESET_SCREEN);
        out.push_str(&self.settings.render(config, config_path, cols, rows));
        out
    }
}

/// Last write time of the config file, or `None` when it is absent. The app
/// polls this to reflect saves without watching the filesystem.
pub fn settings_stamp(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

fn about_page_text(cols: usize, rows: usize) -> String {
    let mut out = String::with_capacity(1_024);
    out.push_str(RESET_SCREEN);

    let title = "T E R M Y X";
    let tagline = "A native terminal with RTL support";
    let version = env!("CARGO_PKG_VERSION");
    let build = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let platform = format!("{} · {}", std::env::consts::OS, std::env::consts::ARCH);
    let fields = [
        ("Version", version.to_string()),
        ("Creator", CREATOR.to_string()),
        ("Build", build.to_string()),
        ("Platform", platform),
    ];
    let label_col = 9;

    let inner = (tagline.width() + 6).max(title.width() + 10);
    let frame_width = inner + 2;
    let mut lines: Vec<String> = Vec::new();

    // The framed card needs five rows plus the tagline, a gap, and the four
    // fields. Smaller grids get the compact form so the page never spills
    // into the scrollback.
    if rows >= 12 && cols >= frame_width {
        let rule = "─".repeat(inner);
        lines.push(centred(
            &format!("\x1b[2m╭{rule}╮\x1b[0m"),
            frame_width,
            cols,
        ));
        lines.push(centred(
            &format!("\x1b[2m│\x1b[0m{}\x1b[2m│\x1b[0m", " ".repeat(inner)),
            frame_width,
            cols,
        ));
        lines.push(centred(
            &format!(
                "\x1b[2m│\x1b[0m\x1b[1;36m{:^inner$}\x1b[0m\x1b[2m│\x1b[0m",
                title
            ),
            frame_width,
            cols,
        ));
        lines.push(centred(
            &format!("\x1b[2m│\x1b[0m{}\x1b[2m│\x1b[0m", " ".repeat(inner)),
            frame_width,
            cols,
        ));
        lines.push(centred(
            &format!("\x1b[2m╰{rule}╯\x1b[0m"),
            frame_width,
            cols,
        ));
        lines.push(centred(
            &format!("\x1b[2m{tagline}\x1b[0m"),
            tagline.width(),
            cols,
        ));
        lines.push(String::new());
    } else {
        lines.push(centred(
            &format!("\x1b[1;36m{title}\x1b[0m"),
            title.width(),
            cols,
        ));
        lines.push(centred(
            &format!("\x1b[2m{tagline}\x1b[0m"),
            tagline.width(),
            cols,
        ));
        lines.push(String::new());
    }

    for (label, value) in fields {
        let width = label_col + value.width();
        lines.push(centred(
            &format!("\x1b[2m{label:<label_col$}\x1b[0m{value}"),
            width,
            cols,
        ));
    }

    out.push_str(&lines.join("\r\n"));
    out
}

/// Pre-pads a styled line so its visible text is centred in `cols`. `width`
/// is the line's visible width, excluding escape sequences.
fn centred(line: &str, width: usize, cols: usize) -> String {
    format!("{}{line}", " ".repeat(cols.saturating_sub(width) / 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::default().validate()
    }

    #[test]
    fn the_palette_page_resets_the_screen_and_shows_the_query() {
        let config = Config::default().validate();
        let mut page = Page::new(PageKind::CommandPalette);
        page.type_char('s');
        let text = page.text(&config, Path::new("config.toml"), 80, 24);
        assert!(text.starts_with(RESET_SCREEN));
        assert!(text.contains("> s"));
        assert!(text.contains("Settings"));
    }

    #[test]
    fn the_palette_page_reports_an_empty_list() {
        let config = Config::default().validate();
        let mut page = Page::new(PageKind::CommandPalette);
        for character in "zzz".chars() {
            page.type_char(character);
        }
        let text = page.text(&config, Path::new("config.toml"), 80, 24);
        assert!(text.contains("No matching commands"));
        assert_eq!(page.palette_command(&config), None);
    }

    #[test]
    fn the_settings_page_fills_exactly_the_terminal_grid() {
        let text = Page::new(PageKind::Settings).text(&config(), Path::new("config.toml"), 80, 24);
        assert_eq!(
            text.split("\r\n").count(),
            24,
            "the viewport must produce one line per grid row"
        );
        assert!(text.contains("Settings"));
        assert!(text.contains("Appearance"));
        assert!(text.contains("Ctrl+S"));
    }

    #[test]
    fn a_settings_tab_owns_an_editable_model() {
        let mut page = Page::new(PageKind::Settings);
        assert!(page.settings().is_some());
        let state = page.settings_mut().expect("a settings model");
        assert_eq!(state.selected(), 0);
        assert!(!state.editing());
        assert!(Page::new(PageKind::About).settings().is_none());
    }

    #[test]
    fn the_about_page_names_the_app_its_version_and_creator() {
        let text = Page::new(PageKind::About).text(&config(), Path::new("unused"), 80, 24);
        assert!(text.contains("T E R M Y X"));
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
        assert!(text.contains("Walid Mostafa"));
    }

    #[test]
    fn the_about_page_fits_a_short_grid() {
        // A page that overflows the grid would leave the hidden lines in the
        // scrollback, so a ten-row terminal gets the compact layout.
        let text = Page::new(PageKind::About).text(&config(), Path::new("unused"), 40, 10);
        assert!(
            text.lines().count() <= 10,
            "the page must not exceed the grid rows"
        );
        assert!(text.contains("Walid Mostafa"));
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn refilling_a_page_drops_the_previous_refills_history() {
        // A Settings tab is refilled on every config save and a palette on
        // every keystroke. Without an explicit scrollback erase the old grids
        // pile up in history and grow a scrollbar over a page that is a single
        // screen.
        let mut terminal = crate::term::Terminal::new(40, 10, 100);
        for line in 0..40 {
            terminal.feed(format!("old output {line}\r\n").as_bytes());
        }
        assert!(
            terminal.history_size() > 0,
            "the fixture must start with rows in the scrollback"
        );

        let page = Page::new(PageKind::About);
        terminal.feed(page.text(&config(), Path::new("unused"), 40, 10).as_bytes());
        assert_eq!(
            terminal.history_size(),
            0,
            "a refill must leave no earlier grid behind"
        );
    }
}
