# Termyx Command Palette Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the command palette from a four-row placeholder into a real, ranked launcher for profiles, tab commands, and tools, opened from the chevron menu or `Ctrl+Shift+P`.

**Architecture:** A new pure module `src/palette.rs` owns the command catalog, availability rules, fuzzy ranking, and the page text. `Page` holds a `Palette` and delegates to it; the app captures the tab that was active before the palette opened, closes the palette tab on Enter, and dispatches through the existing `run_tab_command`, `new_tab`, `open_page`, `start_find`, `export_text`, and `cycle_tab` methods. No second render path: the page still feeds generated text to its own `Terminal`.

**Tech Stack:** Rust 2021, existing crates only (no new dependencies), existing `cargo test` inline-test convention, PowerShell not involved.

**Spec:** `docs/superpowers/specs/2026-11-12-termyx-command-palette-design.md`

## Global Constraints

- No new crate dependencies. Standard library only.
- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` must pass. No dead code, no unused imports.
- Source files stay ASCII.
- No `unwrap`/`expect` in runtime paths; tests may unwrap.
- Tests live inline under `#[cfg(test)]`; there is no `tests/` directory in this repo.
- The palette never writes configuration, never touches PTY input, and spawns a shell only through the existing `Profile` path.
- Shortcut hints must be truthful: a row names a key only when that key actually does this.
- Commits use the inline identity `git -c user.name=Termyx -c user.email=dev@localhost`.
- Comments explain why, not what. Do not reformat unrelated code.

---

### Task 1: The palette engine and its wiring

**Files:**
- Create: `src/palette.rs`
- Modify: `src/main.rs` (module list)
- Modify: `src/page.rs` (imports, `Page` struct, methods, `text`, tests)
- Modify: `src/app.rs` (imports, `palette_target` field, `open_page` path, `refill_page`, `page_key`, `run_menu_action`, new `open_command_palette`/`run_palette_command`/`run_origin_command`/`palette_context`, `menu_items` const import)

**Interfaces:**
- Consumes: `crate::config::Config`, `crate::profiles::profiles(&Config) -> Vec<Profile>` (each `Profile` has `pub name: String`), `crate::ui::context_menu::TabCommand`, `TabSet::len/index/position/active/active_index/session_mut`, `App::close_tab(usize) -> bool`, `App::run_tab_command(SessionId, TabCommand) -> bool`, `App::start_find()`, `App::export_text()`, `App::cycle_tab(isize)`, `App::open_page(PageKind)`, `App::new_tab(usize)`.
- Produces:
  - `pub const PROFILE_SHORTCUTS: usize`
  - `pub enum PaletteCommand { Profile(usize), CloseTab, DuplicateTab, NextTab, PreviousTab, MoveTabLeft, MoveTabRight, CloseOtherTabs, CloseTabsToRight, Settings, Find, ExportText, About }`
  - `pub struct PaletteRow { pub label: String, pub shortcut: &'static str, pub command: PaletteCommand }`
  - `pub struct PaletteContext { pub tabs: usize, pub position: usize }` with `PaletteContext::resolve(tabs, origin, palette) -> PaletteContext`
  - `pub struct Palette` with `new()`, `query() -> &str`, `selected() -> usize`, `type_char(char)`, `backspace()`, `move_selection(isize, usize)`, `reset()`, `set_context(PaletteContext)`, `rows(&Config) -> Vec<PaletteRow>`, `command(&Config) -> Option<PaletteCommand>`, `text(&Config, usize, usize) -> String`
  - `Page::{palette, set_palette_context, reset_palette, palette_command}`

- [ ] **Step 1: Write `src/palette.rs` with the failing tests**

Create the file with the module doc, the imports, and the whole `#[cfg(test)] mod tests` block. Do not define the catalog, the ranking, or the rendering yet: the tests must not compile until Step 4.

```rust
//! The command palette: the catalog of commands, its availability rules, fuzzy
//! ranking, and the page text.
//!
//! The palette owns no terminal or tab state. The app captures the tab that was
//! active before the palette opened, and runs the [`PaletteCommand`] a row
//! carries against that tab.

use std::fmt::Write as _;

use crate::config::Config;


#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::default().validate()
    }

    fn labels(palette: &Palette) -> Vec<String> {
        palette.rows(&config()).into_iter().map(|row| row.label).collect()
    }

    #[test]
    fn an_empty_query_lists_profiles_before_commands() {
        let config = config();
        let profiles = crate::profiles::profiles(&config);
        let palette = Palette::new();
        let listed = labels(&palette);
        assert_eq!(listed.first(), Some(&profiles[0].name));
        let settings = listed.iter().position(|label| label == "Settings").unwrap();
        let find = listed.iter().position(|label| label == "Find").unwrap();
        assert_eq!(find, settings + 1, "Find follows Settings");
        assert_eq!(listed.last().map(String::as_str), Some("About"));
    }

    #[test]
    fn a_prefix_ranks_its_row_first() {
        let mut palette = Palette::new();
        for character in "sett".chars() {
            palette.type_char(character);
        }
        assert_eq!(labels(&palette).first().map(String::as_str), Some("Settings"));
    }

    #[test]
    fn word_starts_outrank_letters_found_mid_word() {
        let mut palette = Palette::new();
        palette.set_context(PaletteContext { tabs: 2, position: 0 });
        palette.type_char('c');
        palette.type_char('t');
        let listed = labels(&palette);
        let close = listed.iter().position(|label| label == "Close tab").unwrap();
        let duplicate = listed
            .iter()
            .position(|label| label == "Duplicate tab")
            .unwrap();
        assert!(close < duplicate, "a word-start match ranks higher: {listed:?}");
    }

    #[test]
    fn unmatched_letters_remove_the_row() {
        let mut palette = Palette::new();
        for character in "zzz".chars() {
            palette.type_char(character);
        }
        assert!(palette.rows(&config()).is_empty());
        assert_eq!(palette.command(&config()), None);
    }

    #[test]
    fn backspace_widens_the_list_and_resets_the_highlight() {
        let mut palette = Palette::new();
        for character in "sett".chars() {
            palette.type_char(character);
        }
        palette.move_selection(1, 3);
        assert_eq!(palette.selected(), 1);
        palette.type_char('i');
        assert_eq!(palette.selected(), 0);
        palette.backspace();
        assert_eq!(palette.query(), "sett");
    }

    #[test]
    fn selection_wraps_and_resets_when_the_list_is_empty() {
        let mut palette = Palette::new();
        palette.move_selection(1, 4);
        assert_eq!(palette.selected(), 1);
        palette.move_selection(-1, 4);
        assert_eq!(palette.selected(), 0);
        palette.move_selection(-1, 4);
        assert_eq!(palette.selected(), 3);
        palette.move_selection(1, 0);
        assert_eq!(palette.selected(), 0);
    }

    #[test]
    fn only_available_tab_commands_are_listed() {
        let mut palette = Palette::new();
        palette.set_context(PaletteContext { tabs: 1, position: 0 });
        let single = labels(&palette);
        assert!(single.iter().any(|label| label == "Duplicate tab"));
        assert!(!single.iter().any(|label| label == "Move tab left"));
        assert!(!single.iter().any(|label| label == "Close other tabs"));
        assert!(!single.iter().any(|label| label == "Next tab"));
        assert!(!single.iter().any(|label| label == "Previous tab"));

        palette.set_context(PaletteContext { tabs: 3, position: 2 });
        let last = labels(&palette);
        assert!(!last.iter().any(|label| label == "Move tab right"));
        assert!(!last.iter().any(|label| label == "Close tabs to right"));
        assert!(last.iter().any(|label| label == "Move tab left"));
    }

    #[test]
    fn shortcuts_name_only_real_bindings() {
        let config = config();
        let palette = Palette::new();
        let rows = palette.rows(&config);
        for (index, row) in crate::profiles::profiles(&config).iter().enumerate() {
            let listed = rows.iter().find(|entry| entry.label == row.name).unwrap();
            let expected = PROFILE_SHORTCUT_LABELS.get(index).copied().unwrap_or("");
            assert_eq!(listed.shortcut, expected);
        }
        let by_label = |label: &str| rows.iter().find(|row| row.label == label).unwrap();
        assert_eq!(by_label("Close tab").shortcut, "Ctrl+Shift+W");
        assert_eq!(by_label("Next tab").shortcut, "Ctrl+Tab");
        assert_eq!(by_label("Find").shortcut, "Ctrl+F");
        assert_eq!(by_label("Settings").shortcut, "");
        assert_eq!(by_label("Export text").shortcut, "");
        assert_eq!(by_label("About").shortcut, "");
    }

    #[test]
    fn resolve_excludes_the_palette_tab_from_the_context() {
        assert_eq!(
            PaletteContext::resolve(3, Some(0), Some(2)),
            PaletteContext { tabs: 2, position: 0 }
        );
        assert_eq!(
            PaletteContext::resolve(2, Some(0), None),
            PaletteContext { tabs: 2, position: 0 }
        );
        // A palette tab left of the origin shifts the origin one slot left.
        assert_eq!(
            PaletteContext::resolve(3, Some(2), Some(1)),
            PaletteContext { tabs: 2, position: 1 }
        );
        assert_eq!(
            PaletteContext::resolve(0, None, None),
            PaletteContext { tabs: 0, position: 0 }
        );
    }

    #[test]
    fn a_row_spans_the_full_width_for_the_highlight() {
        let entry = PaletteRow {
            label: "Close tabs to right".into(),
            shortcut: "Ctrl+Shift+Tab",
            command: PaletteCommand::CloseTabsToRight,
        };
        assert_eq!(row_line(&entry, 60).chars().count(), 60);
        assert!(row_line(&entry, 60).ends_with("Ctrl+Shift+Tab"));
        // Too narrow for a truthful hint: the shortcut is dropped, not clipped.
        assert_eq!(row_line(&entry, 8).chars().count(), 8);
        assert!(!row_line(&entry, 8).contains("Ctrl"));
    }

    #[test]
    fn long_labels_are_truncated_with_two_dots() {
        assert_eq!(truncate("Settings", 8), "Settings");
        assert_eq!(truncate("Close tabs to right", 10), "Close ta..");
        assert_eq!(truncate("Close tabs to right", 2), "Cl");
    }

    #[test]
    fn the_page_text_carries_the_query_rows_and_footer() {
        let config = config();
        let mut palette = Palette::new();
        palette.type_char('s');
        let text = palette.text(&config, 80, 24);
        assert!(text.contains("Command palette"));
        assert!(text.contains("> s"));
        assert!(text.contains("Settings"));
        assert!(text.contains("Up/Down select, Enter run, Esc close"));
        assert!(!text.contains("\x1b[2J"), "the page owns the screen reset");
    }

    #[test]
    fn the_page_text_explains_an_empty_list() {
        let config = config();
        let mut palette = Palette::new();
        for character in "zzz".chars() {
            palette.type_char(character);
        }
        let text = palette.text(&config, 80, 24);
        assert!(text.contains("No matching commands"));
        assert!(!text.contains("\x1b[7m  "), "nothing is selected");
    }
}
```

- [ ] **Step 2: Register the module**

In `src/main.rs`, the module list is alphabetical. Add the line after `mod page;`:

```rust
mod palette;
```

- [ ] **Step 3: Run the palette tests and confirm they fail to compile**

Run: `cargo test palette`
Expected: FAIL with `cannot find type Palette`/`cannot find function score` in the test block. Compilation errors are the failing state for a new module; there is no runtime behavior yet.

- [ ] **Step 4: Add the catalog, ranking, rendering, and text**

Insert this above `#[cfg(test)]`, below the imports:

```rust
/// Profiles that have a numbered key binding (`Ctrl+Shift+1` ... `5`).
pub const PROFILE_SHORTCUTS: usize = 5;

const PROFILE_SHORTCUT_LABELS: [&str; PROFILE_SHORTCUTS] =
    ["Ctrl+Shift+1", "Ctrl+Shift+2", "Ctrl+Shift+3", "Ctrl+Shift+4", "Ctrl+Shift+5"];

/// One action the palette can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteCommand {
    Profile(usize),
    CloseTab,
    DuplicateTab,
    NextTab,
    PreviousTab,
    MoveTabLeft,
    MoveTabRight,
    CloseOtherTabs,
    CloseTabsToRight,
    Settings,
    Find,
    ExportText,
    About,
}

/// One rendered row: what it says, the key that also runs it, and what it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteRow {
    pub label: String,
    pub shortcut: &'static str,
    pub command: PaletteCommand,
}

/// Which tab commands can do anything right now.
///
/// `tabs` counts the strips's tabs *without* the palette itself, and `position`
/// is the origin tab's index among them. Both are computed once per refill from
/// the app's live tab set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteContext {
    pub tabs: usize,
    pub position: usize,
}

impl PaletteContext {
    /// Resolve the context from live tab positions.
    ///
    /// `tabs` counts every tab, including the palette tab at `palette`. The
    /// palette tab is not part of the context: commands act on the origin tab,
    /// and a tab to the palette's left has one index less without it.
    pub fn resolve(tabs: usize, origin: Option<usize>, palette: Option<usize>) -> Self {
        let tabs = tabs.saturating_sub(usize::from(palette.is_some()));
        let position = origin
            .map(|origin| match palette {
                Some(palette) if palette < origin => origin - 1,
                _ => origin,
            })
            .unwrap_or(0);
        Self { tabs, position }
    }
}

pub struct Palette {
    query: String,
    selected: usize,
    context: PaletteContext,
}

impl Palette {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            selected: 0,
            context: PaletteContext { tabs: 1, position: 0 },
        }
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn set_context(&mut self, context: PaletteContext) {
        self.context = context;
    }

    pub fn type_char(&mut self, character: char) {
        self.query.push(character);
        self.selected = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.selected = 0;
    }

    pub fn reset(&mut self) {
        self.query.clear();
        self.selected = 0;
    }

    pub fn move_selection(&mut self, delta: isize, count: usize) {
        if count == 0 {
            self.selected = 0;
            return;
        }
        self.selected = (self.selected as isize + delta).rem_euclid(count as isize) as usize;
    }

    /// Every row the current context allows, ranked by the current query.
    pub fn rows(&self, config: &Config) -> Vec<PaletteRow> {
        let catalog = self.catalog(config);
        let query = self.query.to_lowercase();
        if query.is_empty() {
            return catalog;
        }
        let mut ranked: Vec<(i32, PaletteRow)> = catalog
            .into_iter()
            .filter_map(|row| score(&query, &row.label).map(|value| (value, row)))
            .collect();
        // Stable sort: equal scores keep catalog order.
        ranked.sort_by(|left, right| right.0.cmp(&left.0));
        ranked.into_iter().map(|(_, row)| row).collect()
    }

    /// The row under the highlight, if any rows survive the query.
    pub fn command(&self, config: &Config) -> Option<PaletteCommand> {
        let rows = self.rows(config);
        let index = self.selected.min(rows.len().checked_sub(1)?);
        rows.get(index).map(|row| row.command)
    }

    /// The rows the palette would show in the current context, before ranking.
    fn catalog(&self, config: &Config) -> Vec<PaletteRow> {
        let mut rows = Vec::new();
        for (index, profile) in crate::profiles::profiles(config).into_iter().enumerate() {
            let shortcut = PROFILE_SHORTCUT_LABELS.get(index).copied().unwrap_or("");
            rows.push(row(profile.name, shortcut, PaletteCommand::Profile(index)));
        }

        let tabs = self.context.tabs;
        let position = self.context.position;
        rows.push(row("Close tab", "Ctrl+Shift+W", PaletteCommand::CloseTab));
        rows.push(row("Duplicate tab", "", PaletteCommand::DuplicateTab));
        if position > 0 {
            rows.push(row("Move tab left", "", PaletteCommand::MoveTabLeft));
        }
        if position + 1 < tabs {
            rows.push(row("Move tab right", "", PaletteCommand::MoveTabRight));
        }
        if tabs >= 2 {
            rows.push(row("Close other tabs", "", PaletteCommand::CloseOtherTabs));
        }
        if position + 1 < tabs {
            rows.push(row("Close tabs to right", "", PaletteCommand::CloseTabsToRight));
        }
        if tabs >= 2 {
            rows.push(row("Next tab", "Ctrl+Tab", PaletteCommand::NextTab));
            rows.push(row("Previous tab", "Ctrl+Shift+Tab", PaletteCommand::PreviousTab));
        }

        rows.push(row("Settings", "", PaletteCommand::Settings));
        rows.push(row("Find", "Ctrl+F", PaletteCommand::Find));
        rows.push(row("Export text", "", PaletteCommand::ExportText));
        rows.push(row("About", "", PaletteCommand::About));
        rows
    }

    /// The screen the palette page shows, without the screen reset.
    pub fn text(&self, config: &Config, cols: usize, rows: usize) -> String {
        let mut out = String::new();
        out.push_str("\x1b[1mCommand palette\x1b[0m\r\n\r\n");
        let _ = write!(out, "> {}\x1b[7m \x1b[0m\r\n\r\n", self.query);

        let entries = self.rows(config);
        if entries.is_empty() {
            out.push_str("\x1b[2mNo matching commands\x1b[0m\r\n");
        } else {
            let visible = rows.saturating_sub(6).max(1);
            let selected = self.selected.min(entries.len() - 1);
            let start = (selected + 1).saturating_sub(visible);
            for (index, entry) in entries.iter().enumerate().skip(start).take(visible) {
                if index == selected {
                    out.push_str("\x1b[7m");
                }
                out.push_str(&row_line(entry, cols));
                if index == selected {
                    out.push_str("\x1b[0m");
                }
                out.push_str("\r\n");
            }
        }

        out.push_str("\r\n\x1b[2mUp/Down select, Enter run, Esc close\x1b[0m\r\n");
        out
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::new()
    }
}

fn row(label: impl Into<String>, shortcut: &'static str, command: PaletteCommand) -> PaletteRow {
    PaletteRow {
        label: label.into(),
        shortcut,
        command,
    }
}

/// A case-insensitive subsequence score. `None` when the query does not fit.
///
/// Prefix and word-start letters outrank letters found mid-word, and a run of
/// adjacent letters outranks a scattered match, so `ct` puts `Close tab` above
/// `Duplicate tab`.
fn score(query: &str, label: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let label = label.to_lowercase();
    let mut total = if label.starts_with(query) { 20 } else { 0 };
    let mut query_chars = query.chars().peekable();
    let mut previous: Option<char> = None;
    let mut last_match: Option<usize> = None;

    for (index, character) in label.chars().enumerate() {
        let Some(&wanted) = query_chars.peek() else {
            break;
        };
        if character == wanted {
            query_chars.next();
            let at_word_start = index == 0 || matches!(previous, Some(' ') | Some('-') | Some('_'));
            total += if at_word_start { 8 } else { 4 };
            if last_match == Some(index.wrapping_sub(1)) && index > 0 {
                total += 6;
            }
            last_match = Some(index);
        }
        previous = Some(character);
    }

    if query_chars.peek().is_some() {
        None
    } else {
        Some(total)
    }
}

/// One full-width row: two-space lead, label, padding, then the shortcut.
///
/// The row always spans exactly `cols` cells so the reverse-video highlight of
/// the selected row reaches the right edge.
fn row_line(entry: &PaletteRow, cols: usize) -> String {
    let lead = "  ";
    let shortcut = if cols >= 12 { entry.shortcut } else { "" };
    let shortcut_width = shortcut.chars().count();
    let label_width = cols.saturating_sub(lead.chars().count() + shortcut_width);
    let label = truncate(&entry.label, label_width);
    let mut line = format!("{lead}{label}");
    let padding = cols.saturating_sub(line.chars().count() + shortcut_width);
    line.push_str(&" ".repeat(padding));
    line.push_str(shortcut);
    line
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width <= 2 {
        return text.chars().take(width).collect();
    }
    let mut out: String = text.chars().take(width - 2).collect();
    out.push_str("..");
    out
}
```

- [ ] **Step 5: Rework `src/page.rs` to delegate to the palette**

Imports become:

```rust
use crate::palette::{Palette, PaletteCommand, PaletteContext};
```

Delete `PageAction` and `PaletteEntry` entirely. The struct and constructor become:

```rust
pub struct Page {
    kind: PageKind,
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
```

Replace `type_char`, `backspace`, and `move_selection` bodies with delegation, and replace `matches`/`selected_action` with:

```rust
    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    pub fn set_palette_context(&mut self, context: PaletteContext) {
        self.palette.set_context(context);
    }

    pub fn reset_palette(&mut self) {
        self.palette.reset();
    }

    pub fn palette_command(&self, config: &Config) -> Option<PaletteCommand> {
        self.palette.command(config)
    }

    pub fn type_char(&mut self, character: char) {
        self.palette.type_char(character);
    }

    pub fn backspace(&mut self) {
        self.palette.backspace();
    }

    pub fn move_selection(&mut self, delta: isize, count: usize) {
        self.palette.move_selection(delta, count);
    }
```

In `text`, the `CommandPalette` arm becomes:

```rust
            PageKind::CommandPalette => {
                let mut out = String::from(RESET_SCREEN);
                out.push_str(&self.palette.text(config, cols, rows));
                out
            }
```

Delete the old private `palette_text`. Keep `settings_text`, `settings_stamp`, `about_text`, `about_page_text`, and `ensure_visible` unchanged.

- [ ] **Step 6: Replace the palette tests in `src/page.rs`**

Delete the tests that touch `Page::matches`, `Page::selected_action`, `page.query`, or `page.selected`. Add:

```rust
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
```

Keep the Settings page tests, the About tests, and the "refill drops previous history" test, adjusting only the calls that no longer exist.

- [ ] **Step 7: Wire the app (imports, field, menu path, refill, Enter)**

In `src/app.rs`:

1. Replace the `PageAction` import with

```rust
use crate::palette::{PaletteCommand, PaletteContext};
```

and use `crate::palette::PROFILE_SHORTCUTS` in `menu_items`, deleting the local `const PROFILE_SHORTCUTS: usize = 5;`.

2. Add a field next to `rename`:

```rust
    /// The tab the palette acts on. Captured before the palette tab exists,
    /// because "Close tab" must never mean "close the palette".
    palette_target: Option<SessionId>,
```

and initialize it as `palette_target: None,` in `App::new`.

3. Delete `palette_action` and change `run_menu_action`'s palette arm to:

```rust
            MenuAction::CommandPalette => self.open_command_palette(),
```

4. Add, next to `open_page`:

```rust
    fn open_command_palette(&mut self) {
        self.palette_target = self.tabs.active().map(|session| session.id);
        self.open_page(PageKind::CommandPalette);
    }

    fn palette_context(&self) -> PaletteContext {
        PaletteContext::resolve(
            self.tabs.len(),
            self.palette_target.and_then(|id| self.tabs.position(id)),
            self.palette_tab_index(),
        )
    }

    fn palette_tab_index(&self) -> Option<usize> {
        (0..self.tabs.len()).find(|&index| {
            self.tabs[index]
                .page
                .as_ref()
                .map_or(false, |page| page.kind() == PageKind::CommandPalette)
        })
    }

    fn run_palette_command(&mut self, command: PaletteCommand) {
        match command {
            PaletteCommand::Profile(index) => self.run_menu_action(MenuAction::Profile(index)),
            PaletteCommand::Settings => self.run_menu_action(MenuAction::Settings),
            PaletteCommand::About => self.run_menu_action(MenuAction::About),
            PaletteCommand::Find => self.start_find(),
            PaletteCommand::ExportText => self.export_text(),
            PaletteCommand::NextTab => self.cycle_tab(1),
            PaletteCommand::PreviousTab => self.cycle_tab(-1),
            PaletteCommand::CloseTab => self.run_origin_command(TabCommand::CloseTab),
            PaletteCommand::DuplicateTab => self.run_origin_command(TabCommand::Duplicate),
            PaletteCommand::MoveTabLeft => self.run_origin_command(TabCommand::MoveLeft),
            PaletteCommand::MoveTabRight => self.run_origin_command(TabCommand::MoveRight),
            PaletteCommand::CloseOtherTabs => self.run_origin_command(TabCommand::CloseOtherTabs),
            PaletteCommand::CloseTabsToRight => {
                self.run_origin_command(TabCommand::CloseTabsToRight)
            }
        }
    }

    fn run_origin_command(&mut self, command: TabCommand) {
        let Some(target) = self.palette_target else {
            log::debug!("the palette target tab is gone");
            return;
        };
        self.run_tab_command(target, command);
    }

    fn focus_palette_target(&mut self) {
        let Some(target) = self.palette_target else {
            return;
        };
        let Some(index) = self.tabs.position(target) else {
            return;
        };
        self.tabs.select(index);
        self.selection = None;
    }
```

`focus_palette_target` is needed because `TabSet::remove` activates the neighbour of the removed slot, not the tab the palette was opened from. Without it, closing the palette from a middle tab would leave the tab to its left active, and `Find`, `Export text`, `Next tab`, and `Previous tab` would act on that tab instead of the origin.

5. In `refill_page`, compute the context before borrowing the session:

```rust
    fn refill_page(&mut self, id: SessionId) {
        let context = self.palette_context();
        let Some(session) = self.tabs.session_mut(id) else {
            return;
        };
        let Some(page) = session.page.as_mut() else {
            return;
        };
        if page.kind() == PageKind::CommandPalette {
            page.set_palette_context(context);
        }
        page.ensure_visible(self.rows);
        let text = page.text(&self.config, &self.config_path, self.cols, self.rows);
        session.terminal.feed(text.as_bytes());
        self.selection = None;
    }
```

6. In `page_key`, the arrow count and the Enter arm become:

```rust
                    Key::Named(NamedKey::ArrowUp) => {
                        let count = self.tabs[index]
                            .page
                            .as_ref()
                            .map_or(0, |page| page.palette().rows(&self.config).len());
                        if let Some(page) = self.tabs[index].page.as_mut() {
                            page.move_selection(-1, count);
                        }
                    }
                    Key::Named(NamedKey::ArrowDown) => {
                        let count = self.tabs[index]
                            .page
                            .as_ref()
                            .map_or(0, |page| page.palette().rows(&self.config).len());
                        if let Some(page) = self.tabs[index].page.as_mut() {
                            page.move_selection(1, count);
                        }
                    }
                    Key::Named(NamedKey::Enter) if !repeat => {
                        let command = self.tabs[index]
                            .page
                            .as_ref()
                            .and_then(|page| page.palette_command(&self.config));
                        let exit = self.close_tab(index);
                        self.focus_palette_target();
                        if let Some(command) = command {
                            self.run_palette_command(command);
                        }
                        return Some(exit || self.tabs.is_empty());
                    }
```

Keep the surrounding `refill_page(id); self.request_redraw();` tail. `Some(exit || ...)` keeps Esc's behavior of exiting once the last tab closes; `close_tab` already returns whether the window should exit, so this is belt and braces. Do not call `refill_page` on the closed id before returning. `focus_palette_target` runs before dispatch so every command that acts on the active tab (`Find`, `ExportText`, `NextTab`, `PreviousTab`) sees the origin.

- [ ] **Step 8: Run the tests**

Run: `cargo test palette && cargo test page && cargo test`
Expected: PASS. Fix any borrow or unused-import errors before moving on.

- [ ] **Step 9: Run the gates**

Run: `cargo fmt --check; cargo clippy --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 10: Commit**

```bash
git add src/palette.rs src/main.rs src/page.rs src/app.rs
git -c user.name=Termyx -c user.email=dev@localhost commit -m "feat(palette): rank a real command catalog on the palette page"
```

---

### Task 2: Open it with `Ctrl+Shift+P` and reuse the tab

**Files:**
- Modify: `src/app.rs` (shortcut helper + branch, `open_command_palette` reuse, `menu_items` hint, tests)
- Modify: `src/page.rs` only if a compile error reveals a missed delegation (no planned change)

**Interfaces:**
- Consumes: `PaletteCommand`, `PaletteContext`, `Palette::reset`, `Page::reset_palette`, `App::{palette_tab_index, close_find, dismiss_spellcheck_menu, refill_page, release_forwarded_button, rebuild_layout}`.
- Produces: `fn is_command_palette_shortcut(logical: &Key, modifiers: ModifiersState) -> bool` (private, unit-tested).

- [ ] **Step 1: Write the failing shortcut test**

In the `#[cfg(test)] mod tests` of `src/app.rs`, next to the tab shortcut tests:

```rust
    #[test]
    fn ctrl_shift_p_is_the_palette_shortcut() {
        let ctrl_shift = ModifiersState::CONTROL | ModifiersState::SHIFT;
        assert!(is_command_palette_shortcut(
            &Key::Character("P".into()),
            ctrl_shift
        ));
        assert!(is_command_palette_shortcut(
            &Key::Character("p".into()),
            ctrl_shift
        ));
        assert!(!is_command_palette_shortcut(
            &Key::Character("P".into()),
            ModifiersState::CONTROL
        ));
        assert!(!is_command_palette_shortcut(
            &Key::Character("P".into()),
            ModifiersState::SHIFT
        ));
        assert!(!is_command_palette_shortcut(
            &Key::Character("p".into()),
            ctrl_shift | ModifiersState::ALT
        ));
    }
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test ctrl_shift_p_is_the_palette_shortcut`
Expected: FAIL with `cannot find function is_command_palette_shortcut`.

- [ ] **Step 3: Add the helper and its branch**

Put the helper near `shortcut_for`:

```rust
/// `Ctrl+Shift+P`, free of the tab shortcut table because the palette is not a
/// tab command. Windows reports `P` when Shift is held; other platforms `p`.
fn is_command_palette_shortcut(logical: &Key, modifiers: ModifiersState) -> bool {
    modifiers.control_key()
        && modifiers.shift_key()
        && !modifiers.alt_key()
        && !modifiers.super_key()
        && matches!(logical, Key::Character(text) if text.eq_ignore_ascii_case("p"))
}
```

In `on_key`, immediately after the existing `Ctrl+F` block:

```rust
        if is_command_palette_shortcut(&event.logical_key, self.modifiers) {
            self.open_command_palette();
            return false;
        }
```

- [ ] **Step 4: Make reopening reuse the existing palette tab**

Replace `open_command_palette` with:

```rust
    fn open_command_palette(&mut self) {
        // Reopening while the palette is already active must keep the target
        // captured on the first open, not the palette tab itself.
        let target = match self.tabs.active_index() {
            Some(index) if Some(index) == self.palette_tab_index() => self.palette_target,
            _ => self.tabs.active().map(|session| session.id),
        };
        if let Some(index) = self.palette_tab_index() {
            let id = self.tabs[index].id;
            if let Some(page) = self.tabs[index].page.as_mut() {
                page.reset_palette();
            }
            self.palette_target = target;
            self.close_find();
            self.dismiss_spellcheck_menu();
            if self.tabs.active_index() != Some(index) {
                self.release_forwarded_button();
                self.tabs.select(index);
                self.selection = None;
                self.rebuild_layout();
            }
            // Recompute the context now that the target is known, then refill.
            self.refill_page(id);
            self.request_redraw();
            return;
        }
        self.palette_target = target;
        self.open_page(PageKind::CommandPalette);
    }
```

`refill_page` reads `palette_target` before borrowing the session, so the context is right on this first refill; no second refill is needed.

- [ ] **Step 5: Name the shortcut on the menu row**

In `menu_items`, the tail becomes:

```rust
        // Settings and About have no key binding, so their hint stays empty: a
        // dead shortcut would be a lie. The palette really does open on Ctrl+Shift+P.
        rows.push(MenuItem::fixed(
            MenuAction::Settings,
            "Settings",
            "",
            GLYPH_SETTINGS,
        ));
        rows.push(MenuItem::fixed(
            MenuAction::CommandPalette,
            "Command palette",
            "Ctrl+Shift+P",
            GLYPH_COMMAND_PALETTE,
        ));
        rows.push(MenuItem::fixed(MenuAction::About, "About", "", GLYPH_HELP));
```

Adjust the exact constructor formatting to whatever `rustfmt` produces; the strings are what matter.

- [ ] **Step 6: Run the tests and gates**

Run: `cargo test ctrl_shift_p && cargo test && cargo fmt --check; cargo clippy --all-targets -- -D warnings`
Expected: PASS, clean.

- [ ] **Step 7: Commit**

```bash
git add src/app.rs
git -c user.name=Termyx -c user.email=dev@localhost commit -m "feat(palette): open it with Ctrl+Shift+P and reuse its tab"
```

---

### Task 3: Documentation and full verification

**Files:**
- Modify: `README.md` (Tabs section, new Command palette section, Known limitations)
- Modify: `AGENTS.md` (Current scope)
- Modify: this plan (tick the boxes)

**Interfaces:**
- Consumes: nothing new.
- Produces: documentation only.

- [ ] **Step 1: Fix the stale palette sentence in `README.md`**

In the Tabs section, the paragraph at `README.md:138-141` currently reads:

> The `⌄` menu lists every profile, then `Settings` (opens the config file, or its folder on a first run), `Command palette` (not available yet; the row only logs), and `About`. No key opens `Settings` or the palette.

Replace it with:

> The `⌄` menu lists every profile, then `Settings` (opens the config file, or its folder on a first run), `Command palette` (or `Ctrl+Shift+P`), and `About`. No key opens `Settings`.

Keep the rest of the Tabs section unchanged.

- [ ] **Step 2: Add a Command palette section after Tabs**

```markdown
## Command palette

`Ctrl+Shift+P`, or `Command palette` in the `⌄` menu, opens a launcher in its own
tab. Type to filter, `Up`/`Down` to move, `Enter` to run, `Esc` to close.

It lists the configured profiles (new tab), tab commands (close, duplicate, move,
close others, close to the right, next, previous), and `Settings`, `Find`,
`Export text`, and `About`. Rows that cannot act on the current strip are
omitted, and the right-hand hints name only keys that really work.

Tab commands act on the tab that was active when the palette opened, so
`Close tab` never closes the palette itself. Opening the palette again while it
is already open reuses its tab and clears the filter.
```

- [ ] **Step 3: Correct Known limitations**

`README.md:303` currently reads `- Splits, search, images, and AI are later phases.` Replace it with `- Splits, images, and AI are later phases.` Do not add a limitation for anything this plan shipped.

- [ ] **Step 4: Update `AGENTS.md` current scope**

`AGENTS.md:68-71` currently reads:

> Not yet: Kawkab Mono as an auto-selected face (its narrow metrics render
> disconnected at 16 px), exact cell alignment for narrow Arabic runs, OSC 8
> clickable links, OSC 133 shell integration, IME preedit, tabs, splits,
> profiles, command palette, search, AI, images.

Move tabs, profiles, search, and the command palette into the "In" paragraph, then rewrite the "Not yet" list without them:

> Not yet: Kawkab Mono as an auto-selected face (its narrow metrics render
> disconnected at 16 px), exact cell alignment for narrow Arabic runs, OSC 8
> clickable links, full OSC 133 shell integration, IME preedit, splits, AI,
> images.

Add to the "In" paragraph, after the spellcheck clause: `a command palette and tab
context menus`. Do not change any other line.

- [ ] **Step 5: Manual verification**

Run `cargo run --release`, then:

1. `Ctrl+Shift+P` from a shell tab. The palette lists profiles, tab commands, Settings/Find/Export/About.
2. Type `sett`. `Settings` is first. `Enter` closes the palette and opens Settings.
3. Type `find`. `Enter` opens the find bar on the origin tab.
4. Open two shell tabs, run `next tab` from the palette. The palette closes and the next tab is active.
5. With three tabs, run `move tab left` and `close to right`; each affects the origin tab, not the palette.
6. `Esc` closes the palette. Reopen with the chevron menu; the same tab reappears with an empty filter.
7. Open the palette twice with `Ctrl+Shift+P`: the second press focuses the existing palette tab rather than adding a second one.
8. Resize the window while the palette is open; rows reflow without leaving history behind.

- [ ] **Step 6: Run every gate**

Run:

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Expected: all clean, zero test failures.

- [ ] **Step 7: Tick this plan and commit**

Mark the completed boxes in this file, then:

```bash
git add README.md AGENTS.md docs/superpowers/plans/2026-11-12-termyx-command-palette.md
git -c user.name=Termyx -c user.email=dev@localhost commit -m "docs: document the command palette"
```
