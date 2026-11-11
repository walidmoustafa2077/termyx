//! The command palette: the catalog of commands, its availability rules, fuzzy
//! ranking, and the page text.
//!
//! The palette owns no terminal or tab state. The app captures the tab that was
//! active before the palette opened, and runs the [`PaletteCommand`] a row
//! carries against that tab.

use std::fmt::Write as _;

use crate::config::Config;

/// Profiles that have a numbered key binding (`Ctrl+Shift+1` ... `5`).
pub const PROFILE_SHORTCUTS: usize = 5;

const PROFILE_SHORTCUT_LABELS: [&str; PROFILE_SHORTCUTS] = [
    "Ctrl+Shift+1",
    "Ctrl+Shift+2",
    "Ctrl+Shift+3",
    "Ctrl+Shift+4",
    "Ctrl+Shift+5",
];

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
/// `tabs` counts the strip's tabs *without* the palette itself, and `position`
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

#[derive(Debug, Clone)]
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
            context: PaletteContext {
                tabs: 1,
                position: 0,
            },
        }
    }

    // The app reads `rows` and `command`; these accessors are the tests'
    // window into the model.
    #[allow(dead_code)]
    pub fn query(&self) -> &str {
        &self.query
    }

    #[allow(dead_code)]
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

    // Reopening an existing palette tab must not keep the old query.
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
        ranked.sort_by_key(|entry| std::cmp::Reverse(entry.0));
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
            rows.push(row(
                "Close tabs to right",
                "",
                PaletteCommand::CloseTabsToRight,
            ));
        }
        if tabs >= 2 {
            rows.push(row("Next tab", "Ctrl+Tab", PaletteCommand::NextTab));
            rows.push(row(
                "Previous tab",
                "Ctrl+Shift+Tab",
                PaletteCommand::PreviousTab,
            ));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::default().validate()
    }

    fn labels(palette: &Palette) -> Vec<String> {
        palette
            .rows(&config())
            .into_iter()
            .map(|row| row.label)
            .collect()
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
        assert_eq!(
            labels(&palette).first().map(String::as_str),
            Some("Settings")
        );
    }

    #[test]
    fn word_starts_outrank_letters_found_mid_word() {
        let mut palette = Palette::new();
        palette.set_context(PaletteContext {
            tabs: 2,
            position: 0,
        });
        palette.type_char('c');
        palette.type_char('t');
        let listed = labels(&palette);
        let close = listed
            .iter()
            .position(|label| label == "Close tab")
            .unwrap();
        let duplicate = listed
            .iter()
            .position(|label| label == "Duplicate tab")
            .unwrap();
        assert!(
            close < duplicate,
            "a word-start match ranks higher: {listed:?}"
        );
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
        palette.set_context(PaletteContext {
            tabs: 1,
            position: 0,
        });
        let single = labels(&palette);
        assert!(single.iter().any(|label| label == "Duplicate tab"));
        assert!(!single.iter().any(|label| label == "Move tab left"));
        assert!(!single.iter().any(|label| label == "Close other tabs"));
        assert!(!single.iter().any(|label| label == "Next tab"));
        assert!(!single.iter().any(|label| label == "Previous tab"));

        palette.set_context(PaletteContext {
            tabs: 3,
            position: 2,
        });
        let last = labels(&palette);
        assert!(!last.iter().any(|label| label == "Move tab right"));
        assert!(!last.iter().any(|label| label == "Close tabs to right"));
        assert!(last.iter().any(|label| label == "Move tab left"));
    }

    #[test]
    fn shortcuts_name_only_real_bindings() {
        let config = config();
        let mut palette = Palette::new();
        // "Next tab" only exists with somewhere to go, so the fixture needs two tabs.
        palette.set_context(PaletteContext {
            tabs: 2,
            position: 0,
        });
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
            PaletteContext {
                tabs: 2,
                position: 0
            }
        );
        assert_eq!(
            PaletteContext::resolve(2, Some(0), None),
            PaletteContext {
                tabs: 2,
                position: 0
            }
        );
        // A palette tab left of the origin shifts the origin one slot left.
        assert_eq!(
            PaletteContext::resolve(3, Some(2), Some(1)),
            PaletteContext {
                tabs: 2,
                position: 1
            }
        );
        assert_eq!(
            PaletteContext::resolve(0, None, None),
            PaletteContext {
                tabs: 0,
                position: 0
            }
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
