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

/// Escape sequence that hides the cursor, clears the grid, and homes it.
const RESET_SCREEN: &str = "\x1b[?25l\x1b[2J\x1b[H";

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
}

impl Page {
    pub fn new(kind: PageKind) -> Self {
        Self {
            kind,
            query: String::new(),
            selected: 0,
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

    /// The full screen contents for this page.
    pub fn text(&self, config: &Config, config_path: &Path) -> String {
        match self.kind {
            PageKind::Settings => settings_text(config_path),
            PageKind::CommandPalette => self.palette_text(config),
            PageKind::About => about_page_text(),
        }
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

fn settings_text(path: &Path) -> String {
    let mut out = String::with_capacity(2048);
    out.push_str(RESET_SCREEN);
    out.push_str("\x1b[1mSettings\x1b[0m\r\n\r\n");
    let _ = write!(
        out,
        "\x1b[2mConfig file\x1b[0m\r\n{}\r\n\r\n",
        path.display()
    );
    match fs::read_to_string(path) {
        Ok(body) => out.push_str(&terminal_text(&body)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            out.push_str("\x1b[2mNo config file yet; defaults are in effect.\x1b[0m\r\n");
        }
        Err(error) => {
            let _ = write!(out, "\x1b[2mCould not read the file: {error}\x1b[0m\r\n");
        }
    }
    out.push_str("\r\n\x1b[2mSaved changes appear here automatically.\x1b[0m\r\n");
    out
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

/// Terminal rows need `CRLF`; config files are written with plain newlines.
fn terminal_text(raw: &str) -> String {
    raw.replace("\r\n", "\n").replace('\n', "\r\n")
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
    fn the_settings_page_shows_its_path_and_the_file_contents() {
        let path =
            std::env::temp_dir().join(format!("termyx-page-test-{}.toml", std::process::id()));
        fs::write(&path, "font.size = 15\n").expect("write the test config");
        let text = settings_text(&path);
        let _ = fs::remove_file(&path);
        assert!(text.contains(&path.display().to_string()));
        assert!(text.contains("font.size = 15\r\n"));
    }

    #[test]
    fn a_missing_settings_file_says_defaults_are_in_effect() {
        let path = std::env::temp_dir().join("termyx-page-missing.toml");
        let text = settings_text(&path);
        assert!(text.contains("defaults are in effect"));
    }

    #[test]
    fn the_about_page_names_the_app_and_its_version() {
        let text = Page::new(PageKind::About).text(&config(), Path::new("unused"));
        assert!(text.contains("Termyx"));
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
    }
}
