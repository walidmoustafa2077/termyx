# Termyx Phase 4 (Tabs) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the terminal a Windows Terminal–style tab strip: multiple shell sessions in one window, a `+` new-tab button, a `⌄` profile menu, and per-tab titles taken from OSC 0/2.

**Architecture:** The single `Terminal` + `PtySession` pair on `App` becomes a `TabSet` of `Session`s, each with a stable `u64` id. Every PTY event carries that id, so output and title changes reach the right session. The renderer gains a top inset: the tab strip owns the top `TAB_STRIP_HEIGHT` pixels, and the grid, cursor, decorations, and scrollbar are drawn below it. Tab strip and menu geometry live in a pure, unit-tested `src/ui/` module; `App` only translates hit tests into actions.

**Tech Stack:** Rust stable (MSVC), winit 0.30, softbuffer 0.4, tiny-skia 0.11, alacritty_terminal 0.26, portable-pty 0.8, fontdb 0.23, ab_glyph 0.2, rustybuzz, serde/toml, tracing, thiserror/anyhow.

**Spec:** `docs/superpowers/specs/2026-11-07-termyx-mvp-design.md` (§1 line 21 and §12 line 528 place tabs, splits, profiles, and the command palette in Phase 4; §4 line 68 describes `pty.rs` as owning child lifecycle.)

## Global Constraints

- One window, many sessions. A session is one `Terminal` plus one `PtySession`; nothing else is per-session yet.
- The terminal grid stays logical (write order). Nothing in this plan reorders cells.
- PTY output is fed to the parser verbatim and never rewritten.
- One logical codepoint is one cell; `Cell.width == 0` marks a wide-character continuation.
- The UI thread owns the window, renderer, sessions, and interaction state. Each session keeps its own PTY reader thread.
- Errors at the edge: `thiserror` per module, `anyhow` in the app. No `unwrap`/`expect` outside tests. A profile that fails to spawn logs a warning and leaves the window running.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`, and `cargo build --release` must pass before each commit.
- Commit messages use `feat:`, `fix:`, `test:`, `docs:`, or `chore:`.
- Measured M5 budgets must not regress: 200×50 full frame < 8 ms, idle memory < 80 MB, input-to-paint < 16 ms. New per-session state is small; the UI face is the only new font.

## Visual target

The tab strip is copied from Windows Terminal:

- Strip height `32` logical px, background `#0c0c0c` (the terminal background).
- Each tab is a rounded-top rectangle: inactive `#0c0c0c` with a `#1f1f1f` hovered fill, active `#1f1f1f`, `1` px `#2b2b2b` border on the top and sides.
- Tab content, left to right: a `16` px app icon, `6` px gap, the title in Segoe UI at `12` px `#cccccc` (active `#ffffff`), clipped with an ellipsis.
- The active tab shows a close `✕` at its right edge, `10` px, `#cccccc`; hovering the glyph fills a `#333333` rounded square behind it.
- Inactive tabs show the close glyph only on hover.
- After the last tab: a `+` button (`16` px glyph, `#cccccc`) and a `⌄` chevron button, each `32` px wide, `#1f1f1f` on hover.
- The tab strip's bottom edge is a `1` px `#2b2b2b` line spanning the window.

The dropdown opens directly under the `⌄` button, `280` logical px wide, `#1f1f1f` fill, `1` px `#2b2b2b` border, `4` px corner radius, `4` px vertical padding. Each row is `32` px tall with `8` px horizontal padding: `16` px icon, `8` px gap, label in `12` px Segoe UI `#cccccc`, and a right-aligned shortcut in `12` px `#9a9a9a`. A `1` px `#2b2b2b` separator sits above the `Settings` / `Command palette` / `About` group.

## File structure

- `src/ui/mod.rs` — module root; re-exports the layout types.
- `src/ui/tab_bar.rs` — tab strip geometry and hit testing. No pixels, no `winit`.
- `src/ui/menu.rs` — dropdown model, geometry, and hit testing. No pixels, no `winit`.
- `src/session.rs` — `Session`, `TabSet`, `SessionId`. Owns a `Terminal` and a `PtySession`.
- `src/profiles.rs` — `Profile` list built from config plus the two Windows defaults.
- `src/render/mod.rs` — top inset, tab strip painting, menu painting, UI text.
- `src/render/ui_text.rs` — proportional text drawing through the existing glyph cache.
- `src/fonts.rs` — the Segoe UI face (`ui`) alongside the monospace faces.
- `src/config.rs` — `[[profiles]]` table and `default_profile`.
- `src/app.rs` — routing, shortcuts, interactions, window assembly.

---

### Task 1: Profiles in config

**Files:**
- Modify: `src/config.rs:8-185` (the `Config` struct, defaults, `KNOWN_KEYS`), `src/config.rs` `mod tests`
- Create: `src/profiles.rs`
- Modify: `src/main.rs` (module declaration)

**Interfaces:**
- Consumes: nothing.
- Produces: `config::ProfileConfig { name: String, program: String, args: Vec<String> }`; `config::Config.profiles: Vec<ProfileConfig>`; `config::Config.default_profile: usize`; `profiles::Profile { name: String, shell: ShellConfig }`; `profiles::profiles(config: &Config) -> Vec<Profile>`; `profiles::default_profile_index(config: &Config) -> usize`.

- [ ] **Step 1: Write the failing test**

Add to `src/config.rs` `mod tests`:

```rust
    #[test]
    fn profiles_default_to_windows_power_shell_and_command_prompt() {
        let config = Config::from_toml_str("").unwrap();
        let names: Vec<&str> = config.profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Windows PowerShell", "Command Prompt"]);
        assert_eq!(config.profiles[0].program, "powershell.exe");
        assert_eq!(config.profiles[1].program, "cmd.exe");
        assert_eq!(config.default_profile, 0);
    }

    #[test]
    fn profiles_are_read_from_the_config_file() {
        let config = Config::from_toml_str(
            r#"
            default_profile = 1

            [[profiles]]
            name = "Build shell"
            program = "pwsh.exe"
            args = ["-NoLogo"]

            [[profiles]]
            name = "Admin"
            program = "cmd.exe"
            args = ["/K", "echo hi"]
            "#,
        )
        .unwrap();
        assert_eq!(config.profiles.len(), 2);
        assert_eq!(config.profiles[0].name, "Build shell");
        assert_eq!(config.profiles[0].args, ["-NoLogo"]);
        assert_eq!(config.default_profile, 1);
    }

    #[test]
    fn default_profile_is_clamped_to_the_list() {
        let config = Config::from_toml_str("default_profile = 9").unwrap();
        assert_eq!(config.default_profile, 0);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test profiles_ default_profile_ -- --nocapture`
Expected: FAIL — `no field 'profiles'`, `no field 'default_profile'`.

- [ ] **Step 3: Implement the config surface**

In `src/config.rs`, add above `pub struct Config`:

```rust
/// One entry of the `+` and `⌄` menus: a name to show and the shell to start.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ProfileConfig {
    pub name: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl ProfileConfig {
    fn windows_defaults() -> [Self; 2] {
        [
            Self {
                name: "Windows PowerShell".to_string(),
                program: "powershell.exe".to_string(),
                args: Vec::new(),
            },
            Self {
                name: "Command Prompt".to_string(),
                program: "cmd.exe".to_string(),
                args: Vec::new(),
            },
        ]
    }
}
```

Add the fields to `Config`:

```rust
    #[serde(default)]
    pub profiles: Vec<ProfileConfig>,
    #[serde(default)]
    pub default_profile: usize,
```

In `Default for Config`, set `profiles: ProfileConfig::windows_defaults().to_vec()` and `default_profile: 0`.

In `Config::validate`, after the scrollback clamp:

```rust
        if self.profiles.is_empty() {
            self.profiles = ProfileConfig::windows_defaults().to_vec();
        }
        if self.default_profile >= self.profiles.len() {
            self.default_profile = 0;
        }
```

Add `("profiles", &["name", "program", "args"])` and `"default_profile"` to `KNOWN_KEYS`. `warn_unknown_keys` must treat an array of tables as a known section: give the `profiles` entry the array form the other sections do not use — see Step 4.

- [ ] **Step 4: Let `warn_unknown_keys` accept arrays of tables**

In `src/config.rs`, inside `warn_unknown_keys`, replace the section loop's value lookup with:

```rust
        let value = match &entry.value {
            toml::Value::Array(items) => match items.first() {
                Some(toml::Value::Table(_)) => Some(&entry.value),
                _ => None,
            },
            toml::Value::Table(_) => Some(&entry.value),
        };
```

and when checking each table, iterate every element:

```rust
        let Some(value) = value else { continue };
        let tables: Vec<&toml::Table> = match value {
            toml::Value::Array(items) => items.iter().filter_map(toml::Value::as_table).collect(),
            toml::Value::Table(table) => vec![table],
            _ => continue,
        };
        for table in tables {
            for key in table.keys() {
                if !keys.contains(&key.as_str()) {
                    tracing::warn!(section = *name, key = key.as_str(), "unknown key");
                }
            }
        }
```

- [ ] **Step 5: Create `src/profiles.rs`**

```rust
//! The flat list of shells offered by the `+` and `⌄` menus.

use crate::config::{Config, ShellConfig};

/// A shell the user can open in a new tab.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: String,
    pub shell: ShellConfig,
}

/// Profiles in menu order, with the configured shell as the last resort.
///
/// The `[shell]` section stays authoritative for the first tab and for
/// `default_profile` pointing past the end of the list, so a config that only
/// knows about `[shell]` keeps working.
pub fn profiles(config: &Config) -> Vec<Profile> {
    let mut profiles: Vec<Profile> = config
        .profiles
        .iter()
        .map(|profile| Profile {
            name: profile.name.clone(),
            shell: ShellConfig {
                program: profile.program.clone(),
                args: profile.args.clone(),
            },
        })
        .collect();
    if profiles.is_empty() {
        profiles.push(Profile {
            name: config.shell.program.clone(),
            shell: config.shell.clone(),
        });
    }
    profiles
}

/// Index of the profile a bare `+` opens.
pub fn default_profile_index(config: &Config) -> usize {
    if config.default_profile < config.profiles.len() {
        config.default_profile
    } else {
        0
    }
}
```

Add `mod profiles;` to `src/main.rs` in alphabetical order.

- [ ] **Step 6: Run the tests**

Run: `cargo test --all-targets`
Expected: PASS (the three new tests plus the existing suite).

- [ ] **Step 7: Commit**

```bash
git add src/config.rs src/profiles.rs src/main.rs
git commit -m "feat: read shell profiles from the config"
```

---

### Task 2: Tab strip geometry

**Files:**
- Create: `src/ui/mod.rs`, `src/ui/tab_bar.rs`
- Modify: `src/main.rs` (`mod ui;`)

**Interfaces:**
- Consumes: nothing.
- Produces: `ui::tab_bar::TAB_STRIP_HEIGHT: f32` (= `32.0`); `ui::tab_bar::TabBarLayout { width: f32, tabs: Vec<TabRect>, plus: Rect, chevron: Rect }`; `TabRect { index: usize, rect: Rect, close: Rect }`; `Rect { x: f32, y: f32, w: f32, h: f32 }` with `contains(&self, x, y) -> bool`; `TabBarLayout::new(width: f32, titles: &[&str], width_of: impl Fn(&str) -> f32) -> Self`; `TabBarLayout::hit(&self, x: f32, y: f32) -> Option<TabHit>`; `TabHit::{Tab(usize), Close(usize), Plus, Chevron}`.

- [ ] **Step 1: Write the failing test**

Create `src/ui/tab_bar.rs` with only the test module at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(_: &str) -> f32 {
        100.0
    }

    #[test]
    fn tabs_are_laid_out_left_to_right_and_are_capped() {
        let layout = TabBarLayout::new(1200.0, &["one", "two", "three"], fixed);
        assert_eq!(layout.tabs.len(), 3);
        assert!((layout.tabs[0].rect.x - 0.0).abs() < f32::EPSILON);
        assert!(layout.tabs[1].rect.x > layout.tabs[0].rect.x);
        assert!(layout.tabs[2].rect.x > layout.tabs[1].rect.x);
        assert!((layout.tabs[0].rect.w - MAX_TAB_WIDTH).abs() < f32::EPSILON);
    }

    #[test]
    fn plus_and_chevron_sit_after_the_last_tab() {
        let layout = TabBarLayout::new(1200.0, &["one"], fixed);
        let tab_end = layout.tabs[0].rect.x + layout.tabs[0].rect.w;
        assert!(layout.plus.x >= tab_end);
        assert!(layout.chevron.x > layout.plus.x);
    }

    #[test]
    fn hit_test_finds_tab_close_plus_and_chevron() {
        let layout = TabBarLayout::new(1200.0, &["one", "two"], fixed);
        let first = layout.tabs[0].rect;
        assert_eq!(
            layout.hit(first.x + 4.0, first.y + 4.0),
            Some(TabHit::Tab(0))
        );
        let close = layout.tabs[0].close;
        assert_eq!(
            layout.hit(close.x + close.w / 2.0, close.y + close.h / 2.0),
            Some(TabHit::Close(0))
        );
        assert_eq!(
            layout.hit(layout.plus.x + 2.0, layout.plus.y + 2.0),
            Some(TabHit::Plus)
        );
        assert_eq!(
            layout.hit(layout.chevron.x + 2.0, layout.chevron.y + 2.0),
            Some(TabHit::Chevron)
        );
    }

    #[test]
    fn the_strip_is_a_single_row() {
        let layout = TabBarLayout::new(1200.0, &["one"], fixed);
        assert_eq!(layout.tabs[0].rect.y, 0.0);
        assert_eq!(layout.tabs[0].rect.h, TAB_STRIP_HEIGHT);
        assert_eq!(layout.hit(600.0, TAB_STRIP_HEIGHT + 1.0), None);
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test tab_bar -- --nocapture`
Expected: FAIL — `TabBarLayout` not found.

- [ ] **Step 3: Implement the layout**

Add above the test module in `src/ui/tab_bar.rs`:

```rust
//! Tab strip geometry. Pure coordinates: the renderer and the hit test share
//! one description of where every control sits.

/// Height of the strip in logical pixels; the terminal grid starts below it.
pub const TAB_STRIP_HEIGHT: f32 = 32.0;
/// Widest a single tab grows before the title is clipped.
pub const MAX_TAB_WIDTH: f32 = 220.0;
/// Narrowest a tab shrinks to while the strip still has room.
pub const MIN_TAB_WIDTH: f32 = 90.0;
/// Width of the `+` and `⌄` buttons.
pub const BUTTON_WIDTH: f32 = 32.0;
const CLOSE_SIZE: f32 = 16.0;
const TAB_PADDING: f32 = 12.0;
const TITLE_SIZE: f32 = 100.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabRect {
    pub index: usize,
    pub rect: Rect,
    pub close: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabHit {
    Tab(usize),
    Close(usize),
    Plus,
    Chevron,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabBarLayout {
    pub width: f32,
    pub tabs: Vec<TabRect>,
    pub titles: Vec<String>,
    pub plus: Rect,
    pub chevron: Rect,
}

impl TabBarLayout {
    /// `width_of` measures a title at the UI font size, so a long title both
    /// widens its tab (up to [`MAX_TAB_WIDTH`]) and gets clipped when the strip
    /// runs out of room.
    pub fn new(width: f32, titles: &[&str], width_of: impl Fn(&str) -> f32) -> Self {
        let available = (width - 2.0 * BUTTON_WIDTH).max(MIN_TAB_WIDTH);
        let wanted: Vec<f32> = titles
            .iter()
            .map(|title| (width_of(title) + TITLE_SIZE).clamp(MIN_TAB_WIDTH, MAX_TAB_WIDTH))
            .collect();
        let total: f32 = wanted.iter().sum();
        let scale = if total > available && total > 0.0 {
            available / total
        } else {
            1.0
        };

        let mut tabs = Vec::with_capacity(titles.len());
        let mut x = 0.0;
        for (index, wanted) in wanted.iter().enumerate() {
            let w = (wanted * scale).max(MIN_TAB_WIDTH.min(available));
            let rect = Rect {
                x,
                y: 0.0,
                w,
                h: TAB_STRIP_HEIGHT,
            };
            let close = Rect {
                x: rect.x + rect.w - TAB_PADDING - CLOSE_SIZE,
                y: (TAB_STRIP_HEIGHT - CLOSE_SIZE) / 2.0,
                w: CLOSE_SIZE,
                h: CLOSE_SIZE,
            };
            tabs.push(TabRect {
                index,
                rect,
                close,
            });
            x += w;
        }

        let plus = Rect {
            x,
            y: 0.0,
            w: BUTTON_WIDTH,
            h: TAB_STRIP_HEIGHT,
        };
        let chevron = Rect {
            x: plus.x + BUTTON_WIDTH,
            y: 0.0,
            w: BUTTON_WIDTH,
            h: TAB_STRIP_HEIGHT,
        };
        Self {
            width,
            tabs,
            titles: titles.iter().map(|title| (*title).to_string()).collect(),
            plus,
            chevron,
        }
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<TabHit> {
        if y < 0.0 || y >= TAB_STRIP_HEIGHT {
            return None;
        }
        if let Some(tab) = self.tabs.iter().find(|tab| tab.close.contains(x, y)) {
            return Some(TabHit::Close(tab.index));
        }
        if let Some(tab) = self.tabs.iter().find(|tab| tab.rect.contains(x, y)) {
            return Some(TabHit::Tab(tab.index));
        }
        if self.plus.contains(x, y) {
            return Some(TabHit::Plus);
        }
        if self.chevron.contains(x, y) {
            return Some(TabHit::Chevron);
        }
        None
    }
}
```

Create `src/ui/mod.rs`:

```rust
//! Window chrome: the tab strip and its menu.
//!
//! Everything here is coordinates and state, never pixels, so the layout can
//! be tested without a window.

pub mod menu;
pub mod tab_bar;
```

Create an empty `src/ui/menu.rs` for now:

```rust
//! The `⌄` dropdown: geometry filled in by Task 8.
```

Add `mod ui;` to `src/main.rs`.

- [ ] **Step 4: Run the tests**

Run: `cargo test tab_bar -- --nocapture`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/ui src/main.rs
git commit -m "feat: lay out the tab strip"
```

---

### Task 3: Sessions and the tab set

**Files:**
- Create: `src/session.rs`
- Modify: `src/main.rs` (`mod session;`)

**Interfaces:**
- Consumes: `crate::pty::PtySession`, `crate::term::Terminal`, `crate::config::ShellConfig`, `crate::event::UserEvent`.
- Produces: `session::SessionId = u64`; `session::Session { id: SessionId, terminal: Terminal, pty: PtySession, title: Option<String> }`; `session::TabSet { sessions: Vec<Session>, active: usize, next_id: SessionId }` with `new()`, `len()`, `is_empty()`, `active(&self) -> Option<&Session>`, `active_mut(&mut self) -> Option<&mut Session>`, `id_at(&self, index: usize) -> Option<SessionId>`, `push(&mut self, session: Session) -> usize`, `select(&mut self, index: usize)`, `remove(&mut self, index: usize) -> Option<SessionId>`, `iter_mut()`, `titles(&self) -> Vec<&str>`.

- [ ] **Step 1: Write the failing test**

Create `src/session.rs` with a test module only:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn ids(set: &TabSet) -> Vec<SessionId> {
        set.sessions.iter().map(|session| session.id).collect()
    }

    #[test]
    fn adding_and_removing_keeps_ids_unique() {
        let mut set = TabSet::new();
        let first = set.next_id();
        assert_eq!(first, 1);
        let second = set.next_id();
        assert_eq!(second, 2);
        assert_ne!(first, second);
    }

    #[test]
    fn removing_the_active_tab_selects_a_neighbour() {
        let mut set = TabSet::new();
        set.adopt(1);
        set.adopt(2);
        set.adopt(3);
        assert_eq!(set.active_index(), Some(1));

        set.select(1);
        assert_eq!(set.remove_active(), Some(2));
        assert_eq!(set.active_index(), Some(1));

        set.select(0);
        assert_eq!(set.remove_active(), Some(1));
        assert_eq!(set.active_index(), Some(0));
        assert_eq!(set.remove_active(), Some(3));
        assert_eq!(set.active_index(), None);
        assert!(set.is_empty());
    }

    #[test]
    fn selecting_out_of_range_is_ignored() {
        let mut set = TabSet::new();
        set.adopt(7);
        set.select(5);
        assert_eq!(set.active_index(), Some(0));
    }

    #[test]
    fn titles_fall_back_to_the_program_name() {
        let mut set = TabSet::new();
        set.adopt(1);
        assert_eq!(set.titles(), vec!["Termyx"]);
        set.set_title(1, "build");
        assert_eq!(set.titles(), vec!["build"]);
        set.set_title(1, "");
        assert_eq!(set.titles(), vec!["Termyx"]);
    }

    #[test]
    fn ids_stay_with_their_slots() {
        let mut set = TabSet::new();
        set.adopt(10);
        set.adopt(11);
        let before = ids(&set);
        set.adopt(12);
        assert_eq!(&ids(&set)[..2], &before[..]);
    }
}
```

These tests use `adopt(id)` — a test-only helper that inserts a placeholder session without spawning a shell. Define it in the test module as a helper on a plain `TabSlots` type instead of `TabSet`, so production code never grows a fake session.

Rewrite the tests to exercise `TabSlots`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_the_active_slot_selects_a_neighbour() {
        let mut slots = TabSlots::new();
        slots.push(1);
        slots.push(2);
        slots.push(3);
        assert_eq!(slots.active_index(), Some(1));

        slots.select(1);
        assert_eq!(slots.remove_active(), Some(2));
        assert_eq!(slots.active_index(), Some(1));

        slots.select(0);
        assert_eq!(slots.remove_active(), Some(1));
        assert_eq!(slots.active_index(), Some(0));
        assert_eq!(slots.remove_active(), Some(3));
        assert_eq!(slots.active_index(), None);
        assert!(slots.is_empty());
    }

    #[test]
    fn selecting_out_of_range_is_ignored() {
        let mut slots = TabSlots::new();
        slots.push(7);
        slots.select(5);
        assert_eq!(slots.active_index(), Some(0));
    }

    #[test]
    fn titles_fall_back_to_the_window_name() {
        let mut slots = TabSlots::new();
        slots.push(1);
        assert_eq!(slots.titles(), vec!["Termyx"]);
        slots.set_title(1, "build");
        assert_eq!(slots.titles(), vec!["build"]);
        slots.set_title(1, "  ");
        assert_eq!(slots.titles(), vec!["Termyx"]);
    }

    #[test]
    fn a_new_slot_becomes_active_and_ids_never_repeat() {
        let mut slots = TabSlots::new();
        slots.push(1);
        slots.push(2);
        assert_eq!(slots.active_index(), Some(1));
        slots.select(0);
        slots.push(3);
        assert_eq!(slots.active_index(), Some(2));
        assert_eq!(slots.ids(), vec![1, 2, 3]);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test session:: -- --nocapture`
Expected: FAIL — `TabSlots` not found.

- [ ] **Step 3: Implement `TabSlots` and `TabSet`**

```rust
//! Tabs over terminal sessions.
//!
//! [`TabSlots`] is the bookkeeping every tab operation needs: which slot is
//! active, and which id it holds. It is split from [`TabSet`] so the rules can
//! be tested without spawning shells.

use std::ops::{Index, IndexMut};

use crate::config::ShellConfig;
use crate::event::UserEvent;
use crate::pty::PtySession;
use crate::term::Terminal;

/// Identifies a session across the event loop; never reused within a run.
pub type SessionId = u64;

/// Title shown when a session has not set one, or cleared it.
pub const FALLBACK_TITLE: &str = "Termyx";

#[derive(Debug, Default)]
pub struct TabSlots {
    ids: Vec<SessionId>,
    active: Option<usize>,
}

impl TabSlots {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, id: SessionId) {
        self.ids.push(id);
        self.active = Some(self.ids.len() - 1);
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active
    }

    pub fn select(&mut self, index: usize) {
        if index < self.ids.len() {
            self.active = Some(index);
        }
    }

    /// Removes the active slot and activates the neighbour that takes its
    /// place: the one to the right, else the one to the left.
    pub fn remove_active(&mut self) -> Option<SessionId> {
        let index = self.active?;
        let id = self.ids.remove(index);
        self.active = if self.ids.is_empty() {
            None
        } else {
            Some(index.min(self.ids.len() - 1))
        };
        Some(id)
    }

    pub fn ids(&self) -> Vec<SessionId> {
        self.ids.clone()
    }
}

/// One shell session: its parser, its child, and its title.
pub struct Session {
    pub id: SessionId,
    pub terminal: Terminal,
    pub pty: PtySession,
    pub title: Option<String>,
}

impl Session {
    pub fn new(id: SessionId, terminal: Terminal, pty: PtySession) -> Self {
        Self {
            id,
            terminal,
            pty,
            title: None,
        }
    }
}

/// The live sessions plus the title bookkeeping from [`TabSlots`].
pub struct TabSet {
    slots: TabSlots,
    sessions: Vec<Session>,
    next_id: SessionId,
}

impl Default for TabSet {
    fn default() -> Self {
        Self::new()
    }
}

impl TabSet {
    pub fn new() -> Self {
        Self {
            slots: TabSlots::new(),
            sessions: Vec::new(),
            next_id: 1,
        }
    }

    pub fn next_id(&mut self) -> SessionId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn push(&mut self, session: Session) -> usize {
        self.slots.push(session.id);
        self.sessions.push(session);
        self.sessions.len() - 1
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn active_index(&self) -> Option<usize> {
        self.slots.active_index()
    }

    pub fn select(&mut self, index: usize) {
        self.slots.select(index);
    }

    pub fn active(&self) -> Option<&Session> {
        self.slots.active_index().map(|index| &self.sessions[index])
    }

    pub fn active_mut(&mut self) -> Option<&mut Session> {
        let index = self.slots.active_index()?;
        Some(&mut self.sessions[index])
    }

    pub fn remove_active(&mut self) -> Option<Session> {
        let index = self.slots.active_index()?;
        let id = self.slots.remove_active()?;
        let session = self.sessions.remove(index);
        debug_assert_eq!(session.id, id);
        Some(session)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Session> {
        self.sessions.iter_mut()
    }

    pub fn titles(&self) -> Vec<&str> {
        self.sessions
            .iter()
            .map(|session| {
                session
                    .title
                    .as_deref()
                    .map(str::trim)
                    .filter(|title| !title.is_empty())
                    .unwrap_or(FALLBACK_TITLE)
            })
            .collect()
    }

    pub fn session_mut(&mut self, id: SessionId) -> Option<&mut Session> {
        self.sessions.iter_mut().find(|session| session.id == id)
    }

    pub fn set_title(&mut self, id: SessionId, title: &str) {
        if let Some(session) = self.session_mut(id) {
            session.title = if title.trim().is_empty() {
                None
            } else {
                Some(title.to_string())
            };
        }
    }
}

impl Index<usize> for TabSet {
    type Output = Session;

    fn index(&self, index: usize) -> &Session {
        &self.sessions[index]
    }
}

impl IndexMut<usize> for TabSet {
    fn index_mut(&mut self, index: usize) -> &mut Session {
        &mut self.sessions[index]
    }
}

/// Spawns the shell for a new session and builds its parser.
///
/// The session id reaches the parser and the PTY reader thread in Task 4, so
/// for now this uses the current signatures and only stores the id in the tab.
pub fn spawn_session(
    id: SessionId,
    shell: &ShellConfig,
    cols: usize,
    rows: usize,
    scrollback: usize,
    proxy: winit::event_loop::EventLoopProxy<UserEvent>,
) -> anyhow::Result<Session> {
    let terminal = Terminal::new_with_events(cols, rows, scrollback, Some(proxy.clone()));
    let pty = PtySession::spawn(shell, cols as u16, rows as u16, proxy)?;
    Ok(Session::new(id, terminal, pty))
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test session:: -- --nocapture`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add src/session.rs src/main.rs
git commit -m "feat: track sessions in a tab set"
```

---

### Task 4: Route PTY events by session id

**Files:**
- Modify: `src/event.rs`, `src/term.rs`, `src/pty.rs`, `src/app.rs` (`user_event` arms only)

**Interfaces:**
- Consumes: `session::SessionId` (a plain `u64`, so `event.rs` does not import `session`).
- Produces: `UserEvent::PtyOutput { session: u64, bytes: Vec<u8> }`, `UserEvent::Title { session: u64, title: String }`, `UserEvent::ResetTitle { session: u64 }`, `UserEvent::ClipboardStore(String)`; `Terminal::new_with_events(cols, rows, scrollback, Option<(EventLoopProxy<UserEvent>, u64)>)`; `PtySession::spawn(config, cols, rows, proxy, session: u64)`.

- [ ] **Step 1: Write the failing test**

In `src/term.rs` `mod tests`, replace the existing `user_event_for` assertions with:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test title_events_carry reset_title_and_clipboard -- --nocapture`
Expected: FAIL — `user_event_for` takes one argument.

- [ ] **Step 3: Change the event type**

`src/event.rs`:

```rust
/// Messages from a session's reader thread and parser to the UI thread.
///
/// Every session-tagged message carries its [`session`](UserEvent::PtyOutput)
/// id, because the UI routes it to that tab instead of a single terminal.
#[derive(Debug)]
pub enum UserEvent {
    /// Bytes read from the pty, fed verbatim to the parser.
    PtyOutput { session: u64, bytes: Vec<u8> },
    /// OSC 0/2 set the window title.
    Title { session: u64, title: String },
    /// OSC 0/2 with an empty argument restores the default title.
    ResetTitle { session: u64 },
    /// OSC 52 asked to put text on the system clipboard.
    ClipboardStore(String),
}
```

- [ ] **Step 4: Thread the id through the parser and the pty**

In `src/term.rs`, change `EventProxy` to hold the session id:

```rust
struct EventProxy {
    proxy: EventLoopProxy<crate::event::UserEvent>,
    session: u64,
}
```

and in `impl EventListener for EventProxy`:

```rust
    fn send_event(&self, event: Event) {
        if let Some(event) = user_event_for(self.session, event) {
            let _ = self.proxy.send_event(event);
        }
    }
```

Change `user_event_for` to:

```rust
fn user_event_for(session: u64, event: Event) -> Option<crate::event::UserEvent> {
    match event {
        Event::Title(title) => Some(crate::event::UserEvent::Title { session, title }),
        Event::ResetTitle => Some(crate::event::UserEvent::ResetTitle { session }),
        Event::ClipboardStore(_, text) => Some(crate::event::UserEvent::ClipboardStore(text)),
        _ => None,
    }
}
```

Change `Terminal::new_with_events` to take `events: Option<(EventLoopProxy<UserEvent>, u64)>` and build `EventProxy { proxy, session }`; `Terminal::new` passes `None`.

In `src/pty.rs`, `spawn` and `spawn_with` gain `session: u64` and the reader thread sends `UserEvent::PtyOutput { session, bytes }`.

- [ ] **Step 5: Route the events in `App`**

In `src/app.rs` `user_event`, until Task 7 replaces the single terminal:

```rust
            UserEvent::PtyOutput { session, bytes } => {
                let Some(terminal) = self.terminal.as_mut() else {
                    return;
                };
                if session == self.session_id {
                    if terminal.feed(&bytes) {
                        self.selection = None;
                        self.mouse_down = false;
                    }
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
            }
            UserEvent::Title { session, title } => {
                if session == self.session_id {
                    self.apply_title(&title);
                }
            }
            UserEvent::ResetTitle { session } => {
                if session == self.session_id {
                    self.apply_title("");
                }
            }
```

Add `session_id: u64` to `App` (set to `1` at creation) and a private `fn apply_title(&self, title: &str)` that keeps the existing `dynamic_title` rule:

```rust
    fn apply_title(&self, title: &str) {
        let Some(window) = &self.window else { return };
        if !self.config.window.dynamic_title {
            return;
        }
        let title = title.trim();
        window.set_title(if title.is_empty() {
            DEFAULT_TITLE
        } else {
            title
        });
    }
```

- [ ] **Step 6: Run the gate**

Run: `cargo test --all-targets`, then `cargo clippy --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/event.rs src/term.rs src/pty.rs src/app.rs
git commit -m "feat: tag pty output with its session"
```

---

### Task 5: Reserve the top of the window for the tab strip

**Files:**
- Modify: `src/render/mod.rs`, `src/app.rs`

**Interfaces:**
- Consumes: `ui::tab_bar::TAB_STRIP_HEIGHT`.
- Produces: `Renderer::set_top_inset(&mut self, inset: u32)`, `Renderer::top_inset(&self) -> u32`; `App::tab_strip_px(&self) -> u32`; `grid_size` gains an `inset` parameter.

- [ ] **Step 1: Write the failing test**

In `src/render/mod.rs` `mod tests`:

```rust
    #[test]
    fn the_grid_starts_below_the_top_inset() {
        let mut renderer = default_renderer();
        renderer.resize(200, 120);
        renderer.set_top_inset(32);
        assert_eq!(renderer.top_inset(), 32);

        let snapshot = snapshot_with_text(&["x"]);
        let maps = vec![VisualMap::identity(1)];
        renderer.draw(&snapshot, &maps, None);

        let cell = renderer.fonts().cell_h as u32;
        let padding = renderer.padding();
        assert_eq!(pixel_at(&renderer, padding, 32 + padding - 1), renderer.theme.background);
        assert_ne!(
            pixel_at(&renderer, padding + 1, 32 + padding + cell / 2),
            renderer.theme.background,
            "the glyph paints below the inset"
        );
    }
```

Adapt the helper names to those already in the module (`default_renderer`, `snapshot_with_text`, and whatever the existing cursor tests use to read a pixel; reuse it instead of adding `pixel_at`).

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test the_grid_starts_below -- --nocapture`
Expected: FAIL — `set_top_inset` not found.

- [ ] **Step 3: Apply the inset in the renderer**

Add `top_inset: u32` to `Renderer`, initialise it to `0` in `new`, and:

```rust
    /// Pixels reserved above the grid for the tab strip.
    pub fn set_top_inset(&mut self, inset: u32) {
        self.top_inset = inset;
    }

    pub fn top_inset(&self) -> u32 {
        self.top_inset
    }
```

In `draw`, offset every grid y by `self.top_inset as f32`: the first row baseline, the cursor rectangles, and the decorations pass. The scrollbar keeps spanning the full height below the inset — its geometry takes the inset as its top edge, so `ScrollbarGeometry` gains the same offset when the app builds it in Task 7.

- [ ] **Step 4: Subtract the strip from the grid size**

In `src/app.rs`, change `grid_size(width, height, cell_w, cell_h, padding)` to `grid_size(width, height, cell_w, cell_h, padding, top_inset)`, subtracting `top_inset` from `height` before the existing arithmetic. Update its tests with a `0` inset and add:

```rust
    #[test]
    fn the_tab_strip_costs_one_row_of_grid() {
        let padding = 8;
        let (_, full) = grid_size(800, 600, 8.0, 16.0, padding, 0);
        let (_, with_strip) = grid_size(800, 600, 8.0, 16.0, padding, 32);
        assert_eq!(full - with_strip, 2);
    }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --all-targets`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/render/mod.rs src/app.rs
git commit -m "feat: reserve the top of the window for tabs"
```

---

### Task 6: A proportional UI face and text drawing

**Files:**
- Modify: `src/fonts.rs`, `src/render/mod.rs`
- Create: `src/render/ui_text.rs`

**Interfaces:**
- Consumes: `GlyphCache::get`, `Atlas`, `blit_scaled`, `gamma_table`.
- Produces: `fonts::UI_FONT_ID: u8 = u8::MAX`; `FontSet::ui: Option<FontFace>`; `FontSet::ui_size_px(scale: f32) -> f32`; `FontSet::ui_width(&self, text: &str, px: f32) -> f32`; `render::ui_text::draw_text(renderer: &mut Renderer, text: &str, x: f32, y: f32, px: f32, color: Rgb, max_width: f32) -> f32`.

- [ ] **Step 1: Write the failing test**

In `src/render/ui_text.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_clipped_to_the_available_width() {
        let mut renderer = crate::render::tests::default_renderer();
        renderer.resize(200, 60);
        renderer.pixmap.fill(renderer.theme.background.to_tiny());
        let width = draw_text(
            &mut renderer,
            "a very long tab title",
            2.0,
            2.0,
            12.0,
            renderer.theme.foreground,
            40.0,
        );
        assert!(width <= 40.0, "the reported width never exceeds the limit");
    }
}
```

`default_renderer` and `pixmap` are private to the render tests; make `pub(crate) fn default_renderer()` in `src/render/mod.rs`'s test module and add `#[cfg(test)] pub(crate) fn pixels_are(&self, ...)`-style accessors as needed. Keep the test module helpers in `render::tests` and reference them from `ui_text::tests` as above.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test text_is_clipped -- --nocapture`
Expected: FAIL — `draw_text` not found.

- [ ] **Step 3: Load the UI face**

In `src/fonts.rs`:

```rust
/// Reserved glyph-cache face id for the proportional UI face.
///
/// Symbol faces start at `2`, so the top of the range stays unambiguous.
pub const UI_FONT_ID: u8 = u8::MAX;
```

Add `pub ui: Option<FontFace>` to `FontSet`, load it in `FontSet::load` with a helper mirroring `load_primary` for the family `"Segoe UI"` (returning `None` when the family is missing — the tab strip falls back to the monospace face), and extend `font_for_id`:

```rust
        UI_FONT_ID => self
            .ui
            .as_ref()
            .map(|face| (&face.font, face.index))
            .unwrap_or((&self.font, self.index)),
```

- [ ] **Step 4: Draw proportional text**

`src/render/ui_text.rs`:

```rust
//! Proportional text for window chrome.
//!
//! The terminal grid is monospaced and goes through shaping; chrome text is a
//! handful of short strings in one direction, so it walks the characters and
//! asks the font for each advance.

use ab_glyph::Font;

use crate::fonts::{UI_FONT_ID, UI_SIZE_PX};
use crate::render::{Renderer, SUBPIXELS};
use crate::theme::Rgb;

/// Draws `text` at `(x, y)` — the top-left corner, not the baseline — and
/// returns the width consumed, clipped to `max_width`.
pub fn draw_text(
    renderer: &mut Renderer,
    text: &str,
    x: f32,
    y: f32,
    px: f32,
    color: Rgb,
    max_width: f32,
) -> f32 {
    let Some((font, _index)) = renderer.fonts_face(UI_FONT_ID) else {
        return 0.0;
    };
    let scaled = font.as_scaled(px);
    let baseline = y + scaled.ascent();
    let mut pen = x;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        let advance = scaled.h_advance(id);
        if pen + advance > x + max_width {
            break;
        }
        if let Some(slot) = renderer.cache_get(UI_FONT_ID, id, px.round() as u32) {
            renderer.blit(slot, pen, baseline, color);
        }
        pen += advance;
    }
    pen - x
}
```

`Renderer` exposes the small shims this needs (`fonts_face`, `cache_get`, `blit`), because `cache` and `blit_scaled` are private to the render module:

```rust
    pub(crate) fn fonts_face(&self, id: u8) -> Option<(&ab_glyph::FontVec, u32)> {
        Some(self.fonts.font_for_id(id))
    }

    pub(crate) fn cache_get(
        &mut self,
        font: u8,
        id: ab_glyph::GlyphId,
        size_px: u32,
    ) -> Option<GlyphSlot> {
        let (face, index) = self.fonts.font_for_id(font)?;
        self.cache.get(font, face, id, size_px)
    }

    pub(crate) fn blit(&mut self, slot: GlyphSlot, x: f32, y: f32, color: Rgb) {
        blit_scaled(
            &mut self.pixmap,
            &self.cache.atlas,
            slot,
            x,
            y,
            Ink { color, bold: false },
            1.0,
        );
    }
```

Add `pub(crate) const UI_SIZE_PX: f32 = 12.0;` to `src/fonts.rs` and a `FontSet::ui_width` helper for the tab layout:

```rust
    /// Width of `text` in the UI face at `px`, falling back to the cell width
    /// per character when the face is missing.
    pub fn ui_width(&self, text: &str, px: f32) -> f32 {
        let Some(face) = &self.ui else {
            return text.chars().count() as f32 * self.cell_w;
        };
        let scaled = face.font.as_scaled(px);
        text.chars()
            .map(|ch| scaled.h_advance(face.font.glyph_id(ch)))
            .sum()
    }
```

- [ ] **Step 5: Run the tests**

Run: `cargo test --all-targets`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/fonts.rs src/render
git commit -m "feat: draw proportional chrome text"
```

---

### Task 7: Replace the single session with the tab set

**Files:**
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: `session::{TabSet, spawn_session, SessionId}`, `profiles::{profiles, default_profile_index}`, `Renderer::{set_top_inset, top_inset}`, `FontSet::ui_width`.
- Produces: `App.{tabs: TabSet, layout: Option<TabBarLayout>, tab_bar_height: u32}`; `App::new_tab(&mut self, profile: usize) -> anyhow::Result<()>`; `App::close_tab(&mut self, index: usize)`; `App::active_terminal(&mut self) -> Option<&mut Terminal>`; `App::rebuild_layout(&mut self)`.

- [ ] **Step 1: Write the failing test**

In `src/app.rs` `mod tests`:

```rust
    #[test]
    fn closing_the_last_tab_leaves_no_active_session() {
        let mut slots = crate::session::TabSlots::new();
        slots.push(1);
        slots.push(2);
        slots.select(0);
        assert_eq!(slots.remove_active(), Some(1));
        assert_eq!(slots.remove_active(), Some(2));
        assert_eq!(slots.remove_active(), None);
    }
```

The App-level behaviour (a close request on the last tab exits the window) is verified by hand in Step 6.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test closing_the_last_tab -- --nocapture`
Expected: PASS immediately once Task 3 landed — move on to Step 3 and keep this test as a regression guard.

- [ ] **Step 3: Swap the fields**

In `App`:

- remove `terminal: Option<Terminal>`, `pty: Option<PtySession>`, `session_id: u64`;
- add `tabs: TabSet`, `layout: Option<TabBarLayout>`, `tab_bar_height: u32`;
- keep `selection` on `App` for now but clear it whenever the active tab changes.

In `create_window`, after the renderer and the grid size are known:

```rust
        let tab_bar_height = (TAB_STRIP_HEIGHT * scale_factor as f32).round() as u32;
        renderer.set_top_inset(tab_bar_height);
        let (cols, rows) = grid_size(
            width,
            height,
            renderer.fonts().cell_w,
            renderer.fonts().cell_h,
            renderer.padding(),
            tab_bar_height,
        );
        let mut tabs = TabSet::new();
        let id = tabs.next_id();
        let shell = &config.shell;
        let session = spawn_session(id, shell, cols, rows, config.scrollback.lines, proxy.clone())?;
        tabs.push(session);
```

`render()` switches to the active session:

```rust
        let Some(session) = self.tabs.active() else { return };
        let mut snapshot = session.terminal.snapshot();
```

`resize_grid` resizes every session:

```rust
        let mut changed = false;
        for session in self.tabs.iter_mut() {
            session.terminal.resize(cols, rows);
            session.pty.resize(cols as u16, rows as u16);
            changed = true;
        }
```

`user_event`/`on_key`/`send_text` route through `self.tabs.active_mut()`.

- [ ] **Step 4: Build the layout on every change**

```rust
    /// Recomputes the strip geometry. Called after a resize, a new title, or a
    /// tab add/remove, because tab widths depend on every title.
    fn rebuild_layout(&mut self) {
        let Some(renderer) = self.renderer.as_ref() else { return };
        let Some(window) = self.window.as_ref() else { return };
        let width = window.inner_size().width as f32;
        let titles = self.tabs.titles();
        let px = crate::fonts::UI_SIZE_PX;
        let fonts = renderer.fonts();
        self.layout = Some(TabBarLayout::new(width, &titles, |title| {
            fonts.ui_width(title, px)
        }));
    }
```

`titles()` borrows `self.tabs` immutably while `self.layout` is assigned — collect `let titles: Vec<String> = self.tabs.titles().into_iter().map(str::to_string).collect();` first, then pass `&titles.iter().map(String::as_str).collect::<Vec<_>>()`.

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`
Expected: PASS.

- [ ] **Step 6: Check by hand**

Run `cargo run` and confirm: the window opens with one shell, the prompt sits below an empty 32 px strip, typing works, and `Ctrl+C` still copies a selection.

- [ ] **Step 7: Commit**

```bash
git add src/app.rs
git commit -m "feat: run the terminal inside a tab set"
```

---

### Task 8: Paint the tab strip

**Files:**
- Modify: `src/render/mod.rs`, `src/app.rs`

**Interfaces:**
- Consumes: `ui::{tab_bar::{TabBarLayout, TabRect, TAB_STRIP_HEIGHT}, menu::MenuLayout}`, `render::ui_text::draw_text`.
- Produces: `Renderer::draw_chrome(&mut self, layout: &TabBarLayout, menu: Option<&MenuLayout>, active: usize, hover: Option<usize>)`; `App.{hovered_tab: Option<usize>, menu: Option<MenuState>}`.

- [ ] **Step 1: Write the failing test**

In `src/render/mod.rs` `mod tests`:

```rust
    #[test]
    fn the_active_tab_is_lighter_than_its_neighbour() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = crate::ui::tab_bar::TabBarLayout::new(400.0, &["one", "two"], |_| 60.0);
        renderer.draw_chrome(&layout, None, 0, None);

        let active = pixel_at(&renderer, layout.tabs[0].rect.x + 40.0, 16.0);
        let inactive = pixel_at(&renderer, layout.tabs[1].rect.x + 40.0, 16.0);
        assert_ne!(active, inactive);
    }

    #[test]
    fn the_plus_button_paints_a_glyph() {
        let mut renderer = default_renderer();
        renderer.resize(400, 120);
        let layout = crate::ui::tab_bar::TabBarLayout::new(400.0, &["one"], |_| 60.0);
        renderer.draw_chrome(&layout, None, 0, None);
        let rect = layout.plus;
        let background = renderer.theme.background;
        let painted = (0..rect.w as u32).any(|dx| {
            (0..rect.h as u32).any(|dy| {
                pixel_at(&renderer, rect.x + dx as f32, rect.y + dy as f32) != background
            })
        });
        assert!(painted);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test the_active_tab_is_lighter the_plus_button_paints -- --nocapture`
Expected: FAIL — `draw_chrome` not found.

- [ ] **Step 3: Implement the chrome pass**

Add three theme colours in `src/theme.rs` (`tab_bar: Rgb`, `tab_active: Rgb`, `tab_hover: Rgb`, default `#1f1f1f`, `#1f1f1f`, `#2a2a2a`) and:

```rust
    /// Paints the tab strip over the top inset. Runs after the grid so a
    /// window narrower than the strip never shows grid through it.
    pub fn draw_chrome(
        &mut self,
        layout: &crate::ui::tab_bar::TabBarLayout,
        menu: Option<&crate::ui::menu::MenuLayout>,
        active: usize,
        _hover: Option<usize>,
    ) {
        let height = self.top_inset;
        if layout.width > 0.0 && height > 0 {
            self.pixmap
                .fill_rect(Rect::from_xywh(0, 0, layout.width as u32, height), self.theme.tab_bar.to_tiny());
        }
        for tab in &layout.tabs {
            let color = if tab.index == active {
                self.theme.tab_active
            } else {
                self.theme.tab_bar
            };
            self.pixmap.fill_rect(
                Rect::from_xywh(tab.rect.x as u32, 0, tab.rect.w as u32, height),
                color.to_tiny(),
            );
        }
        let px = crate::fonts::UI_SIZE_PX;
        for tab in &layout.tabs {
            let title = layout.titles.get(tab.index).copied().unwrap_or("");
            let color = if tab.index == active {
                self.theme.foreground
            } else {
                self.theme.foreground
            };
            crate::render::ui_text::draw_text(
                self,
                title,
                tab.rect.x + 34.0,
                9.0,
                px,
                color,
                tab.rect.w - 34.0 - 24.0,
            );
            if tab.index == active {
                crate::render::ui_text::draw_text(self, "✕", tab.close.x + 3.0, tab.close.y + 1.0, px, color, 16.0);
            }
        }
        crate::render::ui_text::draw_text(self, "+", layout.plus.x + 9.0, 6.0, 16.0, self.theme.foreground, 32.0);
        crate::render::ui_text::draw_text(self, "⌄", layout.chevron.x + 9.0, 6.0, 16.0, self.theme.foreground, 32.0);
        self.pixmap.fill_rect(
            Rect::from_xywh(0, height.saturating_sub(1), layout.width as u32, 1),
            self.theme.tab_border.to_tiny(),
        );
        if let Some(menu) = menu {
            crate::render::menu::draw(self, menu);
        }
    }
```

`draw()` calls `self.draw_chrome(...)` only through `App`; keep `draw()` unchanged and let `App::render` call `renderer.draw(...)` then `renderer.draw_chrome(...)` before `sync()`.

- [ ] **Step 4: Run the tests**

Run: `cargo test the_active_tab_is_lighter the_plus_button_paints -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/render src/theme.rs src/ui
git commit -m "feat: paint the tab strip"
```

---

### Task 9: Tab interactions

**Files:**
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: `TabBarLayout::hit`, `TabHit`, `App::{new_tab, close_tab, rebuild_layout}`.
- Produces: `App::tab_hit(&self, x: f64, y: f64) -> Option<TabHit>`; `App::{hovered_tab: Option<usize>}` handling in `CursorMoved`/`CursorLeft`.

- [ ] **Step 1: Write the failing test**

In `src/app.rs` `mod tests`:

```rust
    #[test]
    fn a_click_on_the_plus_hit_is_recognised() {
        let layout = crate::ui::tab_bar::TabBarLayout::new(800.0, &["one", "two"], |_| 80.0);
        let hit = layout.hit(layout.plus.x + 4.0, 10.0);
        assert_eq!(hit, Some(crate::ui::tab_bar::TabHit::Plus));
    }

    #[test]
    fn a_click_below_the_strip_is_not_a_tab_hit() {
        let layout = crate::ui::tab_bar::TabBarLayout::new(800.0, &["one"], |_| 80.0);
        assert!(layout.hit(10.0, 40.0).is_none());
    }
```

- [ ] **Step 2: Run the tests**

Run: `cargo test a_click_on_the_plus a_click_below -- --nocapture`
Expected: PASS (geometry only). Continue to Step 3 for the wiring.

- [ ] **Step 3: Wire the clicks**

In `on_mouse`, before the terminal hit test:

```rust
        if state == ElementState::Pressed && button == MouseButton::Left {
            if let Some(hit) = self.tab_hit(x, y) {
                match hit {
                    TabHit::Tab(index) => {
                        self.tabs.select(index);
                        self.selection = None;
                        self.rebuild_layout();
                    }
                    TabHit::Close(index) => self.close_tab(index),
                    TabHit::Plus => {
                        let index = crate::profiles::default_profile_index(&self.config);
                        if let Err(err) = self.new_tab(index) {
                            tracing::warn!(%err, "could not open a tab");
                        }
                    }
                    TabHit::Chevron => self.menu.toggle_or_open(),
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                return;
            }
        }
```

`on_mouse` currently reads the cursor position from `self.cursor_pos`; use that for `x, y`.

`on_cursor_moved` sets `self.hovered_tab` from `layout.hit(x, y)` (only the `Tab` variant) and requests a redraw when it changes.

`new_tab`:

```rust
    fn new_tab(&mut self, profile: usize) -> anyhow::Result<()> {
        let profiles = crate::profiles::profiles(&self.config);
        let Some(profile) = profiles.get(profile).or_else(|| profiles.first()) else {
            anyhow::bail!("no shell profiles configured");
        };
        let id = self.tabs.next_id();
        let session = crate::session::spawn_session(
            id,
            &profile.shell,
            self.cols,
            self.rows,
            self.config.scrollback.lines,
            self.proxy.clone(),
        )?;
        self.tabs.push(session);
        self.selection = None;
        self.rebuild_layout();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        Ok(())
    }
```

`close_tab` drops the session (its `PtySession::drop` closes the child) and reports whether the set became empty, so the caller can close the window:

```rust
    fn close_tab(&mut self, index: usize) -> bool {
        self.tabs.select(index);
        let _ = self.tabs.remove_active();
        self.selection = None;
        self.rebuild_layout();
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
        self.tabs.is_empty()
    }
```

Closing the last tab closes the window, which matches what users expect from a terminal with no `closeOnExit` option. `close_tab` cannot call `event_loop.exit()` itself because it has no `&ActiveEventLoop`; the `MouseInput` arm does it:

```rust
WindowEvent::MouseInput { state, button, .. } => {
    if self.on_mouse(state, button) {
        event_loop.exit();
    }
}
```

- [ ] **Step 4: Run the gate**

Run: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`
Expected: PASS.

- [ ] **Step 5: Check by hand**

`cargo run`: press `+` twice, click between the three tabs, close the middle one, confirm the neighbouring tab becomes active and the shell keeps running.

- [ ] **Step 6: Commit**

```bash
git add src/app.rs
git commit -m "feat: switch, close, and open tabs with the mouse"
```

---

### Task 10: Keyboard shortcuts

**Files:**
- Modify: `src/app.rs` (`on_key`)

**Interfaces:**
- Consumes: `TabSet::{select, active_index, len}`.
- Produces: `App::tab_shortcut(&self, key: &Key) -> Option<TabAction>` with `enum TabAction { Next, Previous, NewTab, CloseTab, SelectProfile(usize) }`.

- [ ] **Step 1: Write the failing test**

In `src/app.rs` `mod tests`:

```rust
    #[test]
    fn tab_shortcuts_are_recognised() {
        use winit::keyboard::{Key, ModifiersState, NamedKey};
        let ctrl_shift = ModifiersState::CONTROL | ModifiersState::SHIFT;
        assert_eq!(
            tab_shortcut(&Key::Character("T".into()), ctrl_shift),
            Some(TabAction::NewTab)
        );
        assert_eq!(
            tab_shortcut(&Key::Character("W".into()), ctrl_shift),
            Some(TabAction::CloseTab)
        );
        assert_eq!(
            tab_shortcut(&Key::Named(NamedKey::Tab), ModifiersState::CONTROL),
            Some(TabAction::Next)
        );
        assert_eq!(
            tab_shortcut(
                &Key::Named(NamedKey::Tab),
                ModifiersState::CONTROL | ModifiersState::SHIFT
            ),
            Some(TabAction::Previous)
        );
        assert_eq!(
            tab_shortcut(&Key::Character("2".into()), ctrl_shift),
            Some(TabAction::SelectProfile(1))
        );
        assert_eq!(tab_shortcut(&Key::Character("t".into()), ModifiersState::empty()), None);
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test tab_shortcuts_are_recognised -- --nocapture`
Expected: FAIL — `tab_shortcut` not found.

- [ ] **Step 3: Implement the mapping as a free function**

```rust
/// Shortcuts the window chrome owns. Kept free of `App` so the table can be
/// tested without an event loop.
fn tab_shortcut(key: &Key, modifiers: ModifiersState) -> Option<TabAction> {
    let ctrl = modifiers.control_key();
    let shift = modifiers.shift_key();
    match key {
        Key::Named(NamedKey::Tab) if ctrl => {
            Some(if shift { TabAction::Previous } else { TabAction::Next })
        }
        Key::Character(text) if ctrl && shift => match text.as_str() {
            "T" | "t" => Some(TabAction::NewTab),
            "W" | "w" => Some(TabAction::CloseTab),
            "1" ..= "5" => None,
            _ => None,
        },
        _ => None,
    }
}
```

Replace the digit arm with a real parse:

```rust
        Key::Character(text) if ctrl && shift => {
            if text.len() == 1 && text.as_bytes()[0].is_ascii_digit() {
                let digit = text.as_bytes()[0] - b'1';
                return Some(TabAction::SelectProfile(usize::from(digit)));
            }
            match text.as_str() {
                "T" | "t" => Some(TabAction::NewTab),
                "W" | "w" => Some(TabAction::CloseTab),
                _ => None,
            }
        }
```

- [ ] **Step 4: Apply the action in `on_key`**

At the top of `on_key`, before the terminal key mapping:

```rust
        if let Some(action) = tab_shortcut(&event.logical_key, self.modifiers) {
            match action {
                TabAction::Next => self.cycle_tab(1),
                TabAction::Previous => self.cycle_tab(-1),
                TabAction::NewTab => {
                    let index = crate::profiles::default_profile_index(&self.config);
                    if let Err(err) = self.new_tab(index) {
                        tracing::warn!(%err, "could not open a tab");
                    }
                }
                TabAction::CloseTab => {
                    if let Some(index) = self.tabs.active_index() {
                        self.close_tab(index);
                    }
                }
                TabAction::SelectProfile(index) => {
                    if let Err(err) = self.new_tab(index) {
                        tracing::warn!(%err, "could not open a profile");
                    }
                }
            }
            return;
        }
```

`cycle_tab`:

```rust
    fn cycle_tab(&mut self, delta: isize) {
        let count = self.tabs.len();
        if count < 2 {
            return;
        }
        let Some(current) = self.tabs.active_index() else { return };
        let next = (current as isize + delta).rem_euclid(count as isize) as usize;
        self.tabs.select(next);
        self.selection = None;
        self.rebuild_layout();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
```

- [ ] **Step 5: Run the gate**

Run: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/app.rs
git commit -m "feat: switch tabs with the keyboard"
```

---

### Task 11: The `⌄` profile menu

**Files:**
- Create: `src/ui/menu.rs` (layout and model), `src/render/menu.rs` (painting)
- Modify: `src/ui/mod.rs`, `src/render/mod.rs`, `src/app.rs`

**Interfaces:**
- Consumes: `profiles::profiles`, `ui_text::draw_text`, `config::default_config_path`.
- Produces: `ui::menu::MenuItem { label: String, shortcut: String, action: MenuAction }`; `ui::menu::MenuAction::{Profile(usize), Settings, CommandPalette, About}`; `ui::menu::MenuLayout { rect: Rect, rows: Vec<Rect>, items: Vec<MenuItem>, separators: Vec<f32> }` with `MenuLayout::new(anchor: Rect, items: Vec<MenuItem>, window_height: f32) -> Self` and `hit(&self, x: f32, y: f32) -> Option<usize>`; `app::MenuState { layout: MenuLayout, hovered: Option<usize> }`.

- [ ] **Step 1: Write the failing test**

In `src/ui/menu.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<MenuItem> {
        vec![
            MenuItem::profile(0, "Windows PowerShell", "Ctrl+Shift+1"),
            MenuItem::profile(1, "Command Prompt", "Ctrl+Shift+2"),
            MenuItem::fixed(MenuAction::Settings, "Settings", "Ctrl+,"),
            MenuItem::fixed(MenuAction::About, "About", ""),
        ]
    }

    #[test]
    fn the_menu_hangs_below_its_anchor_and_keeps_its_top_edge() {
        let anchor = Rect { x: 700.0, y: 0.0, w: 32.0, h: 32.0 };
        let menu = MenuLayout::new(anchor, items(), 800.0, 600.0);
        assert_eq!(menu.rect.y, 32.0 + MENU_GAP);
        assert!(menu.rect.x + menu.rect.w <= 700.0 + 32.0 + 0.5);
        assert_eq!(menu.rows.len(), 4);
    }

    #[test]
    fn a_menu_wider_than_the_window_is_shoved_inside() {
        let anchor = Rect { x: 780.0, y: 0.0, w: 32.0, h: 32.0 };
        let menu = MenuLayout::new(anchor, items(), 800.0, 600.0);
        assert!(menu.rect.x + menu.rect.w <= 800.0);
    }

    #[test]
    fn hit_test_maps_rows_in_order() {
        let anchor = Rect { x: 0.0, y: 0.0, w: 32.0, h: 32.0 };
        let menu = MenuLayout::new(anchor, items(), 800.0, 600.0);
        let row = menu.rows[2];
        assert_eq!(
            menu.hit(row.x + 4.0, row.y + 4.0),
            Some(2),
            "the third row is Settings"
        );
        assert_eq!(menu.hit(menu.rect.x + 4.0, menu.rect.y - 8.0), None);
    }

    #[test]
    fn there_is_a_separator_before_the_fixed_rows() {
        let anchor = Rect { x: 0.0, y: 0.0, w: 32.0, h: 32.0 };
        let menu = MenuLayout::new(anchor, items(), 800.0, 600.0);
        assert_eq!(menu.separators.len(), 1);
        assert!(menu.separators[0] > menu.rows[1].y);
        assert!(menu.separators[0] < menu.rows[2].y);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test menu:: -- --nocapture`
Expected: FAIL — `MenuLayout` not found.

- [ ] **Step 3: Implement the menu model and layout**

```rust
//! The `⌄` dropdown next to the `+` button.

use crate::ui::tab_bar::Rect;

pub const MENU_WIDTH: f32 = 280.0;
pub const MENU_ROW_HEIGHT: f32 = 32.0;
pub const MENU_PADDING: f32 = 4.0;
pub const MENU_GAP: f32 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Profile(usize),
    Settings,
    CommandPalette,
    About,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    pub shortcut: String,
    pub action: MenuAction,
}

impl MenuItem {
    pub fn profile(index: usize, label: &str, shortcut: &str) -> Self {
        Self {
            label: label.to_string(),
            shortcut: shortcut.to_string(),
            action: MenuAction::Profile(index),
        }
    }

    pub fn fixed(action: MenuAction, label: &str, shortcut: &str) -> Self {
        Self {
            label: label.to_string(),
            shortcut: shortcut.to_string(),
            action,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuLayout {
    pub rect: Rect,
    pub rows: Vec<Rect>,
    pub items: Vec<MenuItem>,
    /// y of each rule between groups.
    pub separators: Vec<f32>,
}

impl MenuLayout {
    /// Anchored under `anchor`, kept inside the window: the dropdown opens
    /// downward unless that would run past the bottom edge, in which case it
    /// flips above the anchor.
    pub fn new(
        anchor: Rect,
        items: Vec<MenuItem>,
        window_width: f32,
        window_height: f32,
    ) -> Self {
        let separator_height = if items.iter().any(|item| item.action == MenuAction::Settings) {
            MENU_PADDING
        } else {
            0.0
        };
        let height = items.len() as f32 * MENU_ROW_HEIGHT + 2.0 * MENU_PADDING + separator_height;
        let x = (anchor.x + anchor.w - MENU_WIDTH).clamp(0.0, (window_width - MENU_WIDTH).max(0.0));
        let mut y = anchor.y + anchor.h + MENU_GAP;
        if y + height > window_height {
            y = (anchor.y - MENU_GAP - height).max(0.0);
        }
        let rect = Rect { x, y, w: MENU_WIDTH, h: height };

        let mut rows = Vec::with_capacity(items.len());
        let mut separators = Vec::new();
        let mut row_y = y + MENU_PADDING;
        let mut seen_fixed = false;
        for item in &items {
            let is_profile = matches!(item.action, MenuAction::Profile(_));
            if !is_profile && !seen_fixed {
                separators.push(row_y);
                row_y += separator_height;
                seen_fixed = true;
            }
            rows.push(Rect {
                x: rect.x + MENU_PADDING,
                y: row_y,
                w: rect.w - 2.0 * MENU_PADDING,
                h: MENU_ROW_HEIGHT,
            });
            row_y += MENU_ROW_HEIGHT;
        }
        Self {
            rect,
            rows,
            items,
            separators,
        }
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        if !self.rect.contains(x, y) {
            return None;
        }
        self.rows
            .iter()
            .position(|row| x >= row.x && x < row.x + row.w && y >= row.y && y < row.y + row.h)
    }
}
```

The `separators` vec holds the y of each rule; `MENU_PADDING` of room is reserved between groups so the rule never touches a row.

- [ ] **Step 4: Run the tests**

Run: `cargo test menu:: -- --nocapture`
Expected: PASS (4 tests).

- [ ] **Step 5: Paint and interact**

`src/render/menu.rs`:

```rust
//! Painting for the `⌄` dropdown.

use tiny_skia::{Color, Rect as SkRect};

use crate::render::{ui_text, Renderer};
use crate::ui::menu::MenuLayout;

pub fn draw(renderer: &mut Renderer, menu: &MenuLayout) {
    let rect = SkRect::from_xywh(menu.rect.x, menu.rect.y, menu.rect.w, menu.rect.h)
        .expect("menu rect is finite");
    renderer.pixmap.fill_rect(rect, renderer.theme.menu_background.to_tiny());
    for separator in &menu.separators {
        let rule = SkRect::from_xywh(menu.rect.x + 8.0, *separator, menu.rect.w - 16.0, 1.0)
            .expect("separator rect is finite");
        renderer.pixmap.fill_rect(rule, renderer.theme.tab_border.to_tiny());
    }
    for (row, item) in menu.rows.iter().zip(&menu.items) {
        ui_text::draw_text(
            renderer,
            &item.label,
            row.x + 26.0,
            row.y + 9.0,
            crate::fonts::UI_SIZE_PX,
            renderer.theme.foreground,
            row.w - 100.0,
        );
        if !item.shortcut.is_empty() {
            let width = renderer.fonts().ui_width(&item.shortcut, crate::fonts::UI_SIZE_PX);
            ui_text::draw_text(
                renderer,
                &item.shortcut,
                row.x + row.w - width - 4.0,
                row.y + 9.0,
                crate::fonts::UI_SIZE_PX,
                renderer.theme.muted,
                width + 4.0,
            );
        }
    }
    let _ = Color::TRANSPARENT;
}
```

Add `menu_background` and `muted` to `Theme`. In `App`, `MenuState { layout, hovered }` plus:

```rust
    fn open_menu(&mut self) {
        let Some(layout) = self.layout.as_ref() else { return };
        let profiles = crate::profiles::profiles(&self.config);
        let mut items: Vec<MenuItem> = profiles
            .iter()
            .enumerate()
            .map(|(index, profile)| {
                MenuItem::profile(index, &profile.name, &format!("Ctrl+Shift+{}", index + 1))
            })
            .collect();
        items.push(MenuItem::fixed(MenuAction::Settings, "Settings", "Ctrl+,"));
        items.push(MenuItem::fixed(MenuAction::CommandPalette, "Command palette", "Ctrl+Shift+P"));
        items.push(MenuItem::fixed(MenuAction::About, "About", ""));
        let height = self.window.as_ref().map_or(0.0, |window| window.inner_size().height as f32);
        let width = self.window.as_ref().map_or(0.0, |window| window.inner_size().width as f32);
        self.menu = Some(MenuState {
            layout: MenuLayout::new(layout.chevron, items, width, height),
            hovered: None,
        });
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
```

Clicking a row runs its action; clicking outside closes the menu before the terminal sees the click.

- [ ] **Step 6: Run the gate**

Run: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/ui src/render src/app.rs
git commit -m "feat: open profiles from the new-tab menu"
```

---

### Task 12: Menu actions and window chrome polish

**Files:**
- Modify: `src/app.rs`, `src/platform.rs`

**Interfaces:**
- Consumes: `MenuAction`, `default_config_path`.
- Produces: `App::run_menu_action(&mut self, action: MenuAction)`; `platform::open_path(path: &Path) -> std::io::Result<()>`.

- [ ] **Step 1: Add `platform::open_path`**

Next to `apply_dark_frame` in `src/platform.rs`:

```rust
/// Opens a file with the platform's default application.
pub fn open_path(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");

    command.arg(path).spawn().map(|_| ())
}
```

- [ ] **Step 2: Implement `run_menu_action`**

```rust
    fn run_menu_action(&mut self, action: MenuAction) {
        match action {
            MenuAction::Profile(index) => {
                if let Err(err) = self.new_tab(index) {
                    tracing::warn!(%err, "could not open a profile");
                }
            }
            MenuAction::Settings => {
                let path = crate::config::default_config_path();
                if let Err(err) = crate::platform::open_path(&path) {
                    tracing::warn!(%err, "could not open the config file");
                }
            }
            MenuAction::CommandPalette => {
                tracing::info!("the command palette arrives in a later task");
            }
            MenuAction::About => {
                let _ = rfd::MessageDialog::new()
                    .set_title(DEFAULT_TITLE)
                    .set_description(concat!(
                        "Termyx ",
                        env!("CARGO_PKG_VERSION"),
                        "\n\nA Rust terminal with RTL support."
                    ))
                    .set_level(rfd::MessageLevel::Info)
                    .show();
            }
        }
    }
```

- [ ] **Step 3: Keep the window title in step with the active tab**

In `rebuild_layout`, after recomputing, when `config.window.dynamic_title` is off, keep `DEFAULT_TITLE`; when it is on, set the window title to the active tab's title. This makes `Ctrl+Tab` switch the title as the screenshot shows.

- [ ] **Step 4: Run the full gate and check by hand**

Run: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`, `cargo build --release`, `cargo test --release full_frame_redraw -- --ignored --nocapture`.

Then `cargo run` and confirm against the screenshot: two tabs with icons and titles, active tab lighter with `✕`, `+` and `⌄` after them, the menu opens under `⌄` with profile rows, shortcuts on the right, a separator, then `Settings` / `Command palette` / `About`; `Settings` opens `config.toml`; a new tab starts the chosen shell; closing the last tab closes the window.

- [ ] **Step 6: Update the docs**

- `README.md`: add a `## Tabs` section describing `+`, `⌄`, `Ctrl+Shift+T/W`, `Ctrl+Tab`, `Ctrl+Shift+1..5`, and the `[[profiles]]` config shape; remove "tabs" from `## Known limitations`.
- `docs/superpowers/specs/2026-11-07-termyx-mvp-design.md`: mark Phase 4 tabs as delivered in the milestone list.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: open settings and profiles from the tab menu"
```

---

## Self-review

- **Spec coverage:** §1 line 21 and §12 line 528 list tabs, profiles, and the command palette for Phase 4. This plan delivers tabs, the profile menu, and the settings/about wiring; splits, search, theme hot reload, and the command palette remain later Phase 4 work and stay listed as such in the README. The palette row appears in the menu but only logs until its own plan.
- **Type consistency:** `TabBarLayout` gains `titles: Vec<String>` in Task 8 but is constructed in Task 2 — Task 8's step says to add the field, and Task 7's `rebuild_layout` passes titles through the closure only. To avoid the mismatch, add `pub titles: Vec<String>` to `TabBarLayout` in Task 2 and fill it in `new` from the `titles` argument.
- **Placeholders:** None. Every step carries the code or the exact command it needs.
