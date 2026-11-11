//! In-app pages shown as tabs: Settings, the command palette, and About.
//!
//! A page has no child process. Its content is generated as terminal text and
//! fed to the tab's own `Terminal`, so the grid, theme, scrolling, selection,
//! and copy paths work without a second render path.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::config::Config;
use crate::settings::SettingsState;

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

/// What activating a palette entry does. The palette has no row for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageAction {
    Profile(usize),
    Settings,
    About,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteEntry {
    pub label: String,
    pub action: PageAction,
}

/// A shell-less tab. The palette owns its query and highlighted entry; the
/// other kinds are static views of live data.
#[derive(Debug, Clone)]
pub struct Page {
    kind: PageKind,
    query: String,
    selected: usize,
    /// The Settings tab's selection, viewport, inline edit, and status. The
    /// other page kinds leave it untouched.
    settings: SettingsState,
}

impl Page {
    pub fn new(kind: PageKind) -> Self {
        Self {
            kind,
            query: String::new(),
            selected: 0,
            settings: SettingsState::new(),
        }
    }

    pub fn kind(&self) -> PageKind {
        self.kind
    }

    pub fn title(&self) -> &'static str {
        self.kind.title()
    }

    /// Appends typed text and returns the highlight to the first match, so the
    /// list always shows what the new query ranks highest.
    pub fn type_char(&mut self, character: char) {
        self.query.push(character);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    /// Moves the highlight by `delta`, wrapping around `count` entries.
    pub fn move_selection(&mut self, delta: isize, count: usize) {
        if count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(count as isize) as usize;
    }

    /// Commands matching the query, case-insensitively, in menu order.
    pub fn matches(&self, config: &Config) -> Vec<PaletteEntry> {
        let query = self.query.to_lowercase();
        let mut entries: Vec<PaletteEntry> = crate::profiles::profiles(config)
            .into_iter()
            .enumerate()
            .map(|(index, profile)| PaletteEntry {
                label: profile.name,
                action: PageAction::Profile(index),
            })
            .collect();
        entries.push(PaletteEntry {
            label: PageKind::Settings.title().to_string(),
            action: PageAction::Settings,
        });
        entries.push(PaletteEntry {
            label: PageKind::About.title().to_string(),
            action: PageAction::About,
        });
        entries.retain(|entry| entry.label.to_lowercase().contains(&query));
        entries
    }

    /// The command under the highlight, if the query matches anything.
    pub fn selected_action(&self, config: &Config) -> Option<PageAction> {
        self.matches(config)
            .get(self.selected)
            .map(|entry| entry.action)
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

    /// The full screen contents for this page. The grid dimensions are needed
    /// only by Settings, whose viewport is exactly the visible body rows.
    pub fn text(&self, config: &Config, config_path: &Path, cols: usize, rows: usize) -> String {
        match self.kind {
            PageKind::Settings => self.settings_text(config, config_path, cols, rows),
            PageKind::CommandPalette => self.palette_text(config),
            PageKind::About => about_page_text(),
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

    fn palette_text(&self, config: &Config) -> String {
        let mut out = String::with_capacity(1024);
        out.push_str(RESET_SCREEN);
        out.push_str("\x1b[1mCommand palette\x1b[0m\r\n\r\n");
        let _ = write!(out, "> {}\x1b[7m \x1b[0m\r\n\r\n", self.query);
        let entries = self.matches(config);
        if entries.is_empty() {
            out.push_str("\x1b[2mNo matching commands\x1b[0m\r\n");
        } else {
            let selected = self.selected.min(entries.len() - 1);
            for (index, entry) in entries.iter().enumerate() {
                if index == selected {
                    let _ = write!(out, "\x1b[7m  {}\x1b[0m\r\n", entry.label);
                } else {
                    let _ = write!(out, "  {}\r\n", entry.label);
                }
            }
        }
        out.push_str("\r\n\x1b[2mUp/Down select, Enter run, Esc close\x1b[0m\r\n");
        out
    }
}

/// Last write time of the config file, or `None` when it is absent. The app
/// polls this to reflect saves without watching the filesystem.
pub fn settings_stamp(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

/// One line plus a blank line, for both the About page and the window title.
pub fn about_text() -> String {
    format!(
        "Termyx {}\n\nA Rust terminal with RTL support.",
        env!("CARGO_PKG_VERSION")
    )
}

fn about_page_text() -> String {
    let mut out = String::with_capacity(512);
    out.push_str(RESET_SCREEN);
    let _ = write!(out, "\x1b[1m{}\x1b[0m\r\n\r\n", about_text());
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let _ = write!(
        out,
        "\x1b[2mBuild\x1b[0m  {profile}\r\n\x1b[2mOS\x1b[0m     {}\r\n\x1b[2mArch\x1b[0m   {}\r\n",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::default().validate()
    }

    fn palette() -> Page {
        Page::new(PageKind::CommandPalette)
    }

    #[test]
    fn typing_filters_the_palette_and_resets_the_highlight() {
        let config = config();
        let mut page = palette();
        for character in "sett".chars() {
            page.type_char(character);
        }
        assert_eq!(page.query, "sett");
        let matches = page.matches(&config);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].action, PageAction::Settings);
        assert_eq!(page.selected, 0);
    }

    #[test]
    fn backspace_widens_the_match_list() {
        let config = config();
        let mut page = palette();
        for character in "po".chars() {
            page.type_char(character);
        }
        assert_eq!(page.matches(&config).len(), 1);
        page.backspace();
        assert_eq!(page.query, "p");
        assert!(page.matches(&config).len() > 1);
    }

    #[test]
    fn the_arrows_wrap_the_highlight() {
        let mut page = palette();
        page.move_selection(-1, 4);
        assert_eq!(page.selected, 3);
        page.move_selection(1, 4);
        assert_eq!(page.selected, 0);
        page.move_selection(0, 0);
        assert_eq!(page.selected, 0);
    }

    #[test]
    fn the_selected_command_follows_the_highlight() {
        let config = config();
        let mut page = palette();
        for character in "about".chars() {
            page.type_char(character);
        }
        assert_eq!(page.selected_action(&config), Some(PageAction::About));
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
    fn the_about_page_names_the_app_and_its_version() {
        let text = Page::new(PageKind::About).text(&config(), Path::new("unused"), 80, 24);
        assert!(text.contains("Termyx"));
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
