# Termyx Tab Context Menu Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the Windows Terminal-style tab context menu shown in the supplied reference image: right-click a tab to open a grouped menu with color, rename, duplicate, split, move, export, find, and close actions.

**Architecture:** A new `ui::context_menu` model owns panel geometry, nested submenus, disabled rows, and hit-testing. A new `render::context_menu` painter draws it with the existing menu metrics. `TabSet` gains per-session metadata (profile index, color, title lock) and a reorder operation. Inline rename and a small find bar are separate state machines with their own render overlays. Export uses the already-present `rfd` dependency.

**Tech Stack:** Rust 2021, winit 0.30, softbuffer 0.4, tiny-skia 0.11, alacritty_terminal 0.26, rfd 0.15, existing Termyx UI/render modules.

**Spec:** The tab context-menu reference supplied by the user on 2026-11-11 (Windows Terminal 1.24 dark chrome). There is no separate repository spec for this feature; this plan is additive to the existing MVP design.

---

## Global Constraints

- Work in `D:\Development\termyx`, branch `phase-4` (clean before starting).
- Follow `AGENTS.md`: one responsibility per file, standard library first, no new dependency unless it removes code. `rfd` is already a dependency.
- Coordinates are physical pixels. Scale every logical measurement by `k = strip_h / TAB_STRIP_HEIGHT`; never mix logical and physical units.
- No `unwrap`/`expect` in runtime paths. Tests may unwrap.
- TDD: add the failing test, run it, implement, run it, then commit. Keep commits small and prefixed `feat:`, `fix:`, `test:`, `docs:`, or `chore:`.
- A mouse press that opens or acts on chrome is `press_in_chrome = true`; it must not copy a selection, paste, or reach the PTY.
- Only one menu owns the pointer. The existing `⌄` dropdown and the new tab menu close each other.
- Do not change unrelated formatting or refactor unrelated code.
- Final gates: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo build --release`.

### Out of scope for this plan

Two rows in the reference are deliberately rendered disabled until their own architecture plans exist:

- **Split tab** needs a pane tree per tab, multiple sessions per tab, per-pane PTY resizing, focus routing, and viewport rendering. The current `TabSet` maps one tab to one session.
- **Move tab to new window** needs multiple `WindowState`s and `WindowId` event routing. The current `App` owns exactly one window and one renderer.

This plan ships both rows visible and disabled, matching the reference’s disabled-row treatment, and ends with follow-up design notes for both.

---

## File Map

- `src/ui/context_menu.rs` — new context-menu model, submenu geometry, hit/press logic.
- `src/render/context_menu.rs` — new painter for panels, rows, swatches, disabled ink, submenu arrows.
- `src/ui/tab_menu.rs` — new pure builder for the tab menu’s rows.
- `src/ui/rename.rs` — new inline rename state.
- `src/render/rename.rs` — new inline rename overlay painter.
- `src/ui/find_bar.rs` — new find-bar geometry and hit-testing.
- `src/render/find_bar.rs` — new find-bar painter.
- `src/find.rs` — new `FindState` (query, matches, current match).
- `src/ui/menu.rs` — reuse its existing public menu metrics; no behavior change to the dropdown.
- `src/render/mod.rs` — declare new modules; accept colored tabs and find highlights.
- `src/render/menu.rs` — reuse its row painting primitives where practical.
- `src/session.rs` — per-tab profile/color/title lock, reorder, rename, duplicate metadata.
- `src/app.rs` — open/dismiss/target context menu, rename/find input, export action.
- `src/term.rs` — full-buffer text export, search, and reveal-line scrolling.
- `src/theme.rs` — disabled menu ink, find highlight tokens, contrast helper.
- `src/fonts.rs` — new glyph constants for menu rows.
- `src/main.rs` — declare `mod find;` if app does not already.

---

## Task 1: Add Per-Tab Metadata and Profile Link

**Files:**
- Modify: `src/session.rs`
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: existing `Session`, `TabSlots`, `TabSet`, `TabIcon`, `Page`.
- Produces:
  - `Session { pub title: Option<String>, pub title_locked: bool, pub color: Option<Rgb>, pub profile: Option<usize>, ... }`
  - `TabSet::titles(&self) -> Vec<&str>` backed by sessions.
  - `TabSet::set_title(id, &str) -> bool` unchanged in meaning, now ignores locked titles.
  - `TabSet::rename(id, &str) -> bool` sets a locked title; a blank name clears it and unlocks.
  - `App::new_tab(profile: usize) -> anyhow::Result<SessionId>` so callers can copy metadata to a duplicate.

- [ ] **Step 1: Write failing session tests**

Add tests near the existing `TabSlots`/`TabSet` tests in `src/session.rs`:

```rust
#[test]
fn rename_locks_the_title_against_osc_updates() {
    let mut tabs = TabSet::new();
    let id = tabs.push(test_session(1));
    assert!(tabs.rename(id, "Build"));
    assert_eq!(tabs.titles(), vec!["Build"]);
    assert!(!tabs.set_title(id, "shell title"));
    assert_eq!(tabs.titles(), vec!["Build"]);
}

#[test]
fn a_blank_rename_clears_the_lock() {
    let mut tabs = TabSet::new();
    let id = tabs.push(test_session(1));
    assert!(tabs.rename(id, "Build"));
    assert!(tabs.rename(id, "   "));
    assert_eq!(tabs.titles(), vec![FALLBACK_TITLE]);
    assert!(tabs.set_title(id, "shell title"));
    assert_eq!(tabs.titles(), vec!["shell title"]);
}

#[test]
fn move_tab_keeps_the_active_session_active() {
    let mut tabs = TabSet::new();
    let a = tabs.push(test_session(1));
    let b = tabs.push(test_session(2));
    let c = tabs.push(test_session(3));
    tabs.select(0);
    assert!(tabs.move_tab(0, 2));
    assert_eq!(tabs.titles().len(), 3);
    assert_eq!(tabs.active().map(|s| s.id), Some(a));
    assert_eq!(tabs[0].id, b);
    assert_eq!(tabs[1].id, c);
    assert_eq!(tabs[2].id, a);
}
```

Use the existing test helper that creates a session without spawning a real shell. If the helper is named differently, reuse it rather than adding a second constructor.

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test session::tests::rename_locks -- --nocapture`

Expected: compile error because `rename` does not exist (or the assertions fail).

- [ ] **Step 3: Move tab metadata into `Session`**

Change `Session`:

```rust
pub struct Session {
    pub id: SessionId,
    pub terminal: Terminal,
    pub pty: Option<PtySession>,
    pub icon: Option<TabIcon>,
    pub page: Option<Page>,
    pub title: Option<String>,
    pub title_locked: bool,
    pub color: Option<crate::theme::Rgb>,
    /// Index into `profiles(&config)` for a shell tab; `None` for a page tab.
    pub profile: Option<usize>,
}
```

Initialize the new fields in `Session::new` and `Session::page_tab`.

Let `TabSlots` keep only `ids` and `active`; remove its `titles` vector and its title methods. Move title normalization to `TabSet`:

```rust
fn clean_title(value: &str) -> Option<String> {
    let title = value.trim();
    title_is_usable(title).then(|| title.to_string())
}
```

```rust
pub fn titles(&self) -> Vec<&str> {
    self.sessions
        .iter()
        .map(|session| session.title.as_deref().unwrap_or(FALLBACK_TITLE))
        .collect()
}

pub fn set_title(&mut self, id: SessionId, title: &str) -> bool {
    let Some(session) = self.session_mut(id) else {
        return false;
    };
    if session.title_locked {
        return false;
    }
    let next = clean_title(title);
    let changed = session.title != next;
    session.title = next;
    changed
}

pub fn rename(&mut self, id: SessionId, title: &str) -> bool {
    let Some(session) = self.session_mut(id) else {
        return false;
    };
    let trimmed = title.trim();
    if trimmed.is_empty() {
        let changed = session.title.is_some() || session.title_locked;
        session.title = None;
        session.title_locked = false;
        return changed;
    }
    // An explicit rename is the user's choice, so it bypasses the path-like
    // filter that protects OSC titles from becoming `C:\...`.
    let next = Some(trimmed.to_string());
    let changed = session.title != next || !session.title_locked;
    session.title = next;
    session.title_locked = true;
    changed
}
```

Keep `title_is_usable` exactly as it is (it rejects paths/UNC names). All existing callers of `TabSlots::set_title` must go through `TabSet::set_title`.

- [ ] **Step 4: Record the spawning profile**

In `App::new_tab`, resolve the profile index first, then record it on the session:

```rust
fn new_tab(&mut self, profile: usize) -> anyhow::Result<SessionId> {
    let profiles = crate::profiles::profiles(&self.config);
    if profiles.is_empty() {
        anyhow::bail!("no shell profile is configured");
    }
    let profile_index = if profile < profiles.len() { profile } else { 0 };
    let profile = &profiles[profile_index];

    // existing id/spawn/push body ...

    let mut session = spawn_session(
        id,
        &profile.shell,
        self.cols,
        self.rows,
        self.config.scrollback.lines,
        self.proxy.clone(),
    )?;
    session.profile = Some(profile_index);
    self.tabs.push(session);
    // existing icon/title/layout/redraw body ...
    let id = self.tabs.active().map(|session| session.id).unwrap_or_default();
    Ok(id)
}
```

Do not infer the index from the profile name. Update current call sites that do `if let Err(err) = self.new_tab(...)`; they can ignore the returned id. `open_page` must leave `profile: None`.

- [ ] **Step 5: Run the session tests**

Run: `cargo test session::tests`

Expected: all pass.

- [ ] **Step 6: Commit**

```text
git add src/session.rs src/app.rs
git commit -m "feat: track tab profile, color, and locked title"
```

---

## Task 2: Reorder, Duplicate, and Bulk Close

**Files:**
- Modify: `src/session.rs`
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: Task 1 metadata.
- Produces:
  - `TabSet::move_tab(from: usize, to: usize) -> bool`
  - `TabSet::color(id) -> Option<Rgb>`, `TabSet::set_color(id, Option<Rgb>) -> bool`
  - `App::duplicate_tab(id: SessionId) -> anyhow::Result<()>`
  - `App::close_other_tabs(id: SessionId) -> bool`
  - `App::close_tabs_to_right(id: SessionId) -> bool`

- [ ] **Step 1: Write failing reorder/color tests**

```rust
#[test]
fn move_tab_is_a_noop_at_the_edges() {
    let mut tabs = TabSet::new();
    let a = tabs.push(test_session(1));
    let b = tabs.push(test_session(2));
    assert!(!tabs.move_tab(0, 0));
    assert!(!tabs.move_tab(0, 2));
    assert_eq!(tabs[0].id, a);
    assert_eq!(tabs[1].id, b);
}

#[test]
fn color_is_stored_per_session() {
    let mut tabs = TabSet::new();
    let id = tabs.push(test_session(1));
    assert_eq!(tabs.color(id), None);
    assert!(tabs.set_color(id, Some(crate::theme::Rgb::new(1, 2, 3))));
    assert_eq!(tabs.color(id), Some(crate::theme::Rgb::new(1, 2, 3)));
}
```

Add app-level tests only for pure helpers. Duplicate spawning a real shell is covered by a manual hand check; keep the decision logic in a pure helper:

```rust
fn duplicate_source(tabs: &TabSet, id: SessionId) -> Option<(usize, Option<Rgb>, Option<String>, bool)> {
    let index = tabs.position(id)?;
    let session = &tabs[index];
    Some((session.profile?, session.color, session.title.clone(), session.title_locked))
}
```

Test that a page tab (`profile: None`) returns `None` and a shell tab returns its profile and metadata.

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test session::tests::move_tab`

Expected: compile error for `move_tab`.

- [ ] **Step 3: Implement reorder and color**

```rust
pub fn move_tab(&mut self, from: usize, to: usize) -> bool {
    let len = self.sessions.len();
    if from >= len || to >= len || from == to {
        return false;
    }
    let active_id = self.slots.active_index().map(|index| self.sessions[index].id);
    let session = self.sessions.remove(from);
    let id = self.slots.ids.remove(from);
    self.sessions.insert(to, session);
    self.slots.ids.insert(to, id);
    if let Some(active_id) = active_id {
        self.slots.active = self.slots.ids.iter().position(|id| *id == active_id);
    }
    true
}
```

A remove/insert is a true move, not a swap: moving index 0 to 2 yields `[b, c, a]`, and moving 0 to 1 yields `[b, a, c]`. The parallel `slots.ids` and `sessions` vectors are moved together.

```rust
pub fn color(&self, id: SessionId) -> Option<crate::theme::Rgb> {
    self.session(id).and_then(|session| session.color)
}

pub fn set_color(&mut self, id: SessionId, color: Option<crate::theme::Rgb>) -> bool {
    let Some(session) = self.session_mut(id) else {
        return false;
    };
    if session.color == color {
        return false;
    }
    session.color = color;
    true
}
```

Add a `session(&self, id)` lookup mirroring `session_mut`.

- [ ] **Step 4: Implement duplicate and bulk close in `App`**

```rust
fn duplicate_tab(&mut self, id: SessionId) -> anyhow::Result<()> {
    let Some((profile, color, title, locked)) = duplicate_source(&self.tabs, id) else {
        return Ok(());
    };
    let new_id = self.new_tab(profile)?;
    if let Some(session) = self.tabs.session_mut(new_id) {
        session.color = color;
        session.title = title;
        session.title_locked = locked;
    }
    self.rebuild_layout();
    if let Some(window) = &self.window {
        window.request_redraw();
    }
    Ok(())
}
```

```rust
fn close_other_tabs(&mut self, id: SessionId) -> bool {
    while self.tabs.len() > 1 {
        let Some(current) = self.tabs.position(id) else {
            break;
        };
        let victim = if current == 0 { 1 } else { 0 };
        self.close_tab(victim);
    }
    if let Some(index) = self.tabs.position(id) {
        self.tabs.select(index);
        self.rebuild_layout();
    }
    false
}

fn close_tabs_to_right(&mut self, id: SessionId) -> bool {
    let Some(target) = self.tabs.position(id) else {
        return false;
    };
    while self.tabs.len() > target + 1 {
        self.close_tab(self.tabs.len() - 1);
    }
    self.tabs.select(target);
    self.rebuild_layout();
    false
}
```

Both helpers keep the target tab, so they never empty the set. They return `false` for “do not exit the window”, matching the `on_mouse` convention.

- [ ] **Step 5: Run tests**

Run: `cargo test session::tests`

Expected: all pass.

- [ ] **Step 6: Commit**

```text
git add src/session.rs src/app.rs
git commit -m "feat: reorder tabs and copy tab metadata"
```

---

## Task 3: Add the Context-Menu Model

**Files:**
- Create: `src/ui/context_menu.rs`
- Modify: `src/ui/mod.rs`
- Modify: `src/fonts.rs`

**Interfaces:**
- Consumes: `Rect`, menu metrics, `Rgb`.
- Produces:
  - `TabCommand` with all row actions.
  - `ContextItem { label, shortcut, glyph, command, enabled, swatch, separator_before, children }`.
  - `ContextMenu::{new, open_submenu, hit, hover, press, clear_hover}`.
  - `ContextPress::{Run(TabCommand), OpenSubmenu(usize), KeepOpen, Dismiss}`.
  - New glyph constants.

- [ ] **Step 1: Add failing geometry tests**

Create `src/ui/context_menu.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tab_bar::Rect;

    fn tab() -> Rect {
        Rect { x: 100.0, y: 8.0, w: 180.0, h: 32.0 }
    }

    fn items() -> Vec<ContextItem> {
        vec![
            ContextItem::action("Rename tab", "R", TabCommand::Rename),
            ContextItem::submenu(
                "Move tab",
                "M",
                vec![
                    ContextItem::action("Move right", "R", TabCommand::MoveRight),
                    ContextItem::action("Move left", "L", TabCommand::MoveLeft),
                ],
            ),
            ContextItem::action("Close tab", "X", TabCommand::CloseTab)
                .separator_before(),
        ]
    }

    #[test]
    fn opens_below_the_tab_and_clamps_inside_the_window() {
        let menu = ContextMenu::new(tab(), items(), 800.0, 600.0, 1.0);
        assert!(menu.panels[0].rect.y >= tab().y + tab().h);
        assert!(menu.panels[0].rect.x >= 0.0);
        assert!(menu.panels[0].rect.x + menu.panels[0].rect.w <= 800.0);
    }

    #[test]
    fn a_narrow_tab_near_the_right_pulls_the_panel_left() {
        let mut tab = tab();
        tab.x = 790.0;
        let menu = ContextMenu::new(tab, items(), 800.0, 600.0, 1.0);
        assert_eq!(menu.panels[0].rect.x + menu.panels[0].rect.w, 800.0);
    }

    #[test]
    fn clicking_a_submenu_parent_opens_it_to_the_right() {
        let mut menu = ContextMenu::new(tab(), items(), 800.0, 600.0, 1.0);
        let row = menu.panels[0].rows[1];
        assert_eq!(menu.press(row.x + 5.0, row.y + 5.0), ContextPress::OpenSubmenu(1));
        menu.open_submenu(0, 1);
        assert_eq!(menu.panels.len(), 2);
        assert_eq!(menu.panels[1].rect.x, menu.panels[0].rect.x + menu.panels[0].rect.w + 2.0);
    }

    #[test]
    fn a_disabled_row_never_runs() {
        let mut items = items();
        items[0].enabled = false;
        let menu = ContextMenu::new(tab(), items, 800.0, 600.0, 1.0);
        let row = menu.panels[0].rows[0];
        assert_eq!(menu.press(row.x + 5.0, row.y + 5.0), ContextPress::KeepOpen);
    }

    #[test]
    fn clicks_outside_dismiss() {
        let menu = ContextMenu::new(tab(), items(), 800.0, 600.0, 1.0);
        assert_eq!(menu.press(0.0, 0.0), ContextPress::Dismiss);
    }

    #[test]
    fn separator_space_is_not_a_row() {
        let menu = ContextMenu::new(tab(), items(), 800.0, 600.0, 1.0);
        let separator = menu.panels[0].separators[0];
        assert!(matches!(
            menu.hit(menu.panels[0].rect.x + 5.0, separator),
            ContextHit::Panel(0)
        ));
    }
}
```

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test ui::context_menu`

Expected: module/file missing.

- [ ] **Step 3: Define the model**

```rust
use crate::theme::Rgb;
use crate::ui::menu::{MENU_GAP, MENU_PADDING, MENU_ROW_HEIGHT};
use crate::ui::tab_bar::Rect;

pub const CONTEXT_MENU_WIDTH: f32 = 320.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabCommand {
    ChangeColor(Option<Rgb>),
    Rename,
    Duplicate,
    Split,
    MoveToNewWindow,
    MoveLeft,
    MoveRight,
    ExportText,
    Find,
    CloseOtherTabs,
    CloseTabsToRight,
    CloseTab,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextItem {
    pub label: String,
    pub shortcut: String,
    pub glyph: &'static str,
    pub command: Option<TabCommand>,
    pub enabled: bool,
    /// Outer `Some` means draw an icon swatch; inner `None` is the empty
    /// "Default" swatch. `None` means the row has no swatch.
    pub swatch: Option<Option<Rgb>>,
    pub separator_before: bool,
    pub children: Vec<ContextItem>,
}
```

Constructors:

```rust
impl ContextItem {
    pub fn action(label: &str, glyph: &'static str, command: TabCommand) -> Self {
        Self {
            label: label.into(),
            shortcut: String::new(),
            glyph,
            command: Some(command),
            enabled: true,
            swatch: None,
            separator_before: false,
            children: Vec::new(),
        }
    }

    pub fn submenu(label: &str, glyph: &'static str, children: Vec<ContextItem>) -> Self {
        let mut item = Self::action(label, glyph, TabCommand::Rename);
        item.command = None;
        item.children = children;
        item
    }

    pub fn swatch(label: &str, color: Option<Rgb>) -> Self {
        Self {
            label: label.into(),
            shortcut: String::new(),
            glyph: "",
            command: Some(TabCommand::ChangeColor(color)),
            enabled: true,
            swatch: Some(color),
            separator_before: false,
            children: Vec::new(),
        }
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    pub fn separator_before(mut self) -> Self {
        self.separator_before = true;
        self
    }
}
```

The “Default” swatch is `Some(None)`: the painter draws a bordered empty square instead of a black one.

Geometry:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct ContextPanel {
    pub rect: Rect,
    pub rows: Vec<Rect>,
    pub items: Vec<ContextItem>,
    pub separators: Vec<f32>,
    pub hovered: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContextMenu {
    pub panels: Vec<ContextPanel>,
    pub scale: f32,
    pub window_width: f32,
    pub window_height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextHit {
    Row { panel: usize, row: usize },
    Panel(usize),
    Outside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextPress {
    Run(TabCommand),
    OpenSubmenu(usize),
    KeepOpen,
    Dismiss,
}
```

`ContextMenu::new(anchor: Rect, items, window_width, window_height, scale)`:
- `k = scale.clamp(0.5, 4.0)`.
- width `(CONTEXT_MENU_WIDTH * k).min(window_width)`.
- `x = anchor.x.clamp(0.0, (window_width - width).max(0.0))`.
- `y = anchor.y + anchor.h + MENU_GAP * k`; if `y + height > window_height`, use `y = (window_height - height).max(0.0)`. Do not flip above the strip: the menu is anchored to the tab card and should stay under it.
- If the full height does not fit, drop trailing rows before constructing rects. Dropped rows are absent from `rows` and `items`.
- Add `MENU_PADDING * k` before each `separator_before` row (except the first), record the rule’s y, and add `MENU_PADDING * k` after the last row.
- Row rects use `MENU_ROW_HEIGHT * k`.
- `open_submenu(panel, row)`: replace `panels` after `panel` with one child panel. Its `x` is parent right + `MENU_GAP * k`; if that overflows the window, place it at parent left - child width - gap. Clamp `y` so the child fits the window bottom. If the child is wider than the window, clamp to 0 and truncate text at paint time.
- `hit`: iterate panels in reverse (topmost first); inside a rect, return `Row` when a row contains the point, else `Panel(panel)`. Return `Outside` otherwise.
- `hover(x, y)`: if `Row`, set that panel’s `hovered`; clear the other panels’ hover. If the row has enabled children, call `open_submenu`; if it has none, truncate panels to `panel + 1`. If `Panel`, keep the panel chain. If `Outside`, clear all hovers but keep panels open (the pointer may be crossing the gap into a child).
- `press(x, y)`: `Outside` -> `Dismiss`; `Panel` -> `KeepOpen`; `Row` -> disabled `KeepOpen`; enabled with children -> `OpenSubmenu(row)`; enabled with command -> `Run(command)`; otherwise `KeepOpen`.
- `clear_hover`: set every panel’s `hovered = None`.

The existing menu metrics in `src/ui/menu.rs` are already `pub`; reuse them. Do not change their values or make them crate-private.

- [ ] **Step 4: Add the row-builder tests**

Create `src/ui/tab_menu.rs`:

```rust
pub fn rows(index: usize, total: usize, can_duplicate: bool, colors: &[Rgb]) -> Vec<ContextItem>
```

Expected rows, in order:

1. `Change tab color` (submenu of `Default` + one row per `colors` entry)
2. `Rename tab`
3. `Duplicate tab`
4. `Split tab` (disabled)
5. `Move tab` (submenu: `Move tab to new window` disabled, `Move right` enabled iff `index + 1 < total`, `Move left` enabled iff `index > 0`)
6. `Export text` (separator before)
7. `Find`
8. `Close` (submenu: `Close other tabs` enabled iff `total > 1`, `Close tabs to right` enabled iff `index + 1 < total`; separator before)
9. `Close tab` (separator before)

Tests:

```rust
#[test]
fn edge_tabs_disable_the_matching_move_rows() {
    let colors = [Rgb::new(1, 2, 3); 8];
    let left = tab_menu::rows(0, 3, true, &colors);
    let right = tab_menu::rows(2, 3, true, &colors);
    assert!(find(&left, "Move right").enabled);
    assert!(!find(&left, "Move left").enabled);
    assert!(!find(&right, "Move right").enabled);
    assert!(find(&right, "Move left").enabled);
}

#[test]
fn page_tabs_cannot_duplicate_but_can_close() {
    let colors = [Rgb::new(1, 2, 3); 8];
    let rows = tab_menu::rows(0, 1, false, &colors);
    assert!(!find(&rows, "Duplicate tab").enabled);
    assert!(!find(&rows, "Close other tabs").enabled);
    assert!(find(&rows, "Close tab").enabled);
}
```

`find` is a test helper that recursively looks up a label.

- [ ] **Step 5: Add glyph constants**

In `src/fonts.rs`, add and visually verify:

```rust
pub const GLYPH_TAB_COLOR: &str = "\u{E790}";
pub const GLYPH_RENAME: &str = "\u{E8AC}";
pub const GLYPH_DUPLICATE: &str = "\u{E8C8}";
pub const GLYPH_SPLIT: &str = "\u{E8A9}";
pub const GLYPH_MOVE_TAB: &str = "\u{E8DE}";
pub const GLYPH_EXPORT: &str = "\u{E74E}";
pub const GLYPH_FIND: &str = "\u{E721}";
pub const GLYPH_CHEVRON_RIGHT: &str = "\u{E76C}";
```

If any glyph is blank in Segoe Fluent Icons, use the closest visible MDL2 codepoint and note it in the commit; do not ship an invisible icon.

- [ ] **Step 6: Declare modules and run tests**

`src/ui/mod.rs`:

```rust
pub mod context_menu;
pub mod menu;
pub mod tab_bar;
pub mod tab_menu;
pub mod tooltip;
```

Add `rename` and `find_bar` here only when their files are created in Tasks 6 and 9.

Run: `cargo test ui::context_menu ui::tab_menu`

Expected: all pass.

- [ ] **Step 7: Commit**

```text
git add src/ui src/fonts.rs
git commit -m "feat: model tab context menus and submenus"
```

---

## Task 4: Paint the Context Menu

**Files:**
- Create: `src/render/context_menu.rs`
- Modify: `src/render/mod.rs`
- Modify: `src/theme.rs`

**Interfaces:**
- Consumes: `ContextMenu`, `ContextItem`, `TabCommand`, `Rgb`, existing `fill_round_rect`, `fill_rect`, `blend`, `ui_text`.
- Produces:
  - `Renderer::draw_context_menu(&mut self, menu: &ContextMenu)`.
  - `Theme::menu_disabled: Rgb`.
  - `Theme::ink_on(background: Rgb) -> Rgb` (used later by colored tabs).

- [ ] **Step 1: Write failing painter tests**

Follow the existing `src/render/menu.rs` test style (`Renderer` at a known size, `pixel_at`, `ink_in`):

```rust
#[test]
fn a_disabled_row_uses_the_disabled_ink() {
    let mut item = ContextItem::action("Split tab", GLYPH_SPLIT, TabCommand::Split);
    item.enabled = false;
    let menu = ContextMenu::new(tab(), vec![item], 800.0, 600.0, 1.0);
    renderer.draw_context_menu(&menu);
    let row = menu.panels[0].rows[0];
    assert!(ink_in(row, renderer.pixels(), renderer.theme.menu_disabled));
}

#[test]
fn a_submenu_parent_draws_a_right_chevron() {
    let menu = ContextMenu::new(
        tab(),
        vec![ContextItem::submenu(
            "Move tab",
            GLYPH_MOVE_TAB,
            vec![ContextItem::action("Move right", "R", TabCommand::MoveRight)],
        )],
        800.0,
        600.0,
        1.0,
    );
    renderer.draw_context_menu(&menu);
    let row = menu.panels[0].rows[0];
    assert!(ink_in(row, renderer.pixels(), renderer.theme.foreground));
}
```

Add a swatch test: a color row paints its swatch at the icon inset; a `Default` row paints a bordered empty swatch instead of a filled black one.

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test render::context_menu`

Expected: module missing.

- [ ] **Step 3: Add theme tokens**

```rust
pub menu_disabled: Rgb,
pub find_match: Rgb,
pub find_current: Rgb,
```

Initialize in `from_config`:

```rust
menu_disabled: Rgb::new(138, 138, 138),
find_match: Rgb::new(72, 72, 34),
find_current: Rgb::new(128, 112, 34),
```

Add:

```rust
pub fn ink_on(&self, background: Rgb) -> Rgb {
    let luminance =
        0.299 * f32::from(background.r) + 0.587 * f32::from(background.g) + 0.114 * f32::from(background.b);
    if luminance > 140.0 {
        self.background
    } else {
        self.foreground
    }
}
```

- [ ] **Step 4: Paint panels and rows**

```rust
impl Renderer {
    pub fn draw_context_menu(&mut self, menu: &ContextMenu) {
        for panel in &menu.panels {
            if panel.rect.w <= 0.0 || panel.rect.h <= 0.0 {
                continue;
            }
            self.fill_round_rect(panel.rect, MENU_RADIUS * menu.scale, self.theme.menu_background);
            for y in &panel.separators {
                let line = Rect {
                    x: panel.rect.x + MENU_SEPARATOR_INSET * menu.scale,
                    y: *y,
                    w: panel.rect.w - 2.0 * MENU_SEPARATOR_INSET * menu.scale,
                    h: 1.0,
                };
                self.fill_rect(line.x, line.y, line.w, line.h, self.theme.menu_separator);
            }
            for (index, item) in panel.items.iter().enumerate() {
                let Some(row) = panel.rows.get(index) else { continue };
                self.paint_context_row(panel, row, item, index, menu.scale);
            }
        }
    }
}
```

`paint_context_row`:
- Hover pill only when `item.enabled && panel.hovered == Some(index)`: inset `MENU_HOVER_INSET*k`, radius `MENU_HOVER_RADIUS*k`, fill `theme.menu_hover`.
- Icon rect at `row.x + MENU_ICON_INSET*k`, size `MENU_ICON_SIZE*k`, vertically centred.
- `item.swatch`:
  - `Some(Some(color))`: fill rounded rect radius `3*k` with `color`.
  - `Some(None)`: fill with `menu_background`, then stroke a 1px border `menu_shortcut` (use four 1px `fill_rect`s; the renderer has no stroke helper).
- Glyph: `ui_text::draw_text_centred` in the icon face, size `MENU_ICON_SIZE*k`, color `theme.foreground` or `theme.menu_disabled`.
- Label at `row.x + MENU_LABEL_INSET*k`; use `draw_text_ellipsis_ink_centred` in `UI_FONT_ID`, `MENU_TEXT_PX*k`; width = `row.w - MENU_LABEL_INSET*k - MENU_RIGHT_INSET*k - arrow_width - shortcut_width`, minimum 1.0.
- Shortcut right-aligned in `theme.menu_shortcut` if nonempty.
- If `item.children` is nonempty, draw `GLYPH_CHEVRON_RIGHT` at `row.right - MENU_RIGHT_INSET*k - MENU_ICON_SIZE*k`, size `MENU_ICON_SIZE*k`, color foreground/disabled.
- If `item.enabled` is false, use `theme.menu_disabled` for glyph, label, shortcut, and arrow.

Add `mod context_menu;` to the module list in `src/render/mod.rs`.

- [ ] **Step 5: Run tests**

Run: `cargo test render::context_menu`

Expected: all pass.

- [ ] **Step 6: Commit**

```text
git add src/render src/theme.rs
git commit -m "feat: paint the tab context menu"
```

---

## Task 5: Open the Menu on a Right-Click

**Files:**
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: `ContextMenu`, `ui::tab_menu::rows`, `StripPress`, `StripAction`, `TabSet`.
- Produces:
  - `StripPress::OpenTabMenu(usize)`.
  - `App::tab_menu: Option<TabMenuState>` where `TabMenuState { menu: ContextMenu, target: SessionId }`.
  - `App::open_tab_menu(index)`, `close_tab_menu()`, `run_tab_command(target: SessionId, command: TabCommand) -> bool`.
  - Right-click on a tab or its `✕` opens the menu; right-click elsewhere in the band is swallowed; right-click in the grid still pastes.

- [ ] **Step 1: Write failing pure tests**

Add to the `strip_press` tests:

```rust
#[test]
fn right_press_on_a_tab_opens_its_menu() {
    assert_eq!(
        strip_press(
            ChromePress::Tab,
            MouseButton::Right,
            true,
            false,
            false,
            Some(StripAction::Select(2)),
        ),
        StripPress::OpenTabMenu(2)
    );
}

#[test]
fn right_press_on_a_close_button_opens_the_menu_for_that_tab() {
    assert_eq!(
        strip_press(
            ChromePress::Tab,
            MouseButton::Right,
            true,
            false,
            false,
            Some(StripAction::Close(3)),
        ),
        StripPress::OpenTabMenu(3)
    );
}

#[test]
fn right_press_on_empty_strip_is_ignored() {
    assert_eq!(
        strip_press(ChromePress::Drag, MouseButton::Right, true, false, false, None),
        StripPress::Act(StripAction::Ignore)
    );
}
```

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test app::tests::right_press`

Expected: `OpenTabMenu` variant missing.

- [ ] **Step 3: Extend `StripPress`**

```rust
enum StripPress {
    Ignore,
    Drag,
    ToggleMaximize,
    OpenTabMenu(usize),
    Act(StripAction),
}
```

Change `strip_press` to take `button: MouseButton` instead of `left: bool`:

```rust
fn strip_press(
    press: ChromePress,
    button: MouseButton,
    in_strip: bool,
    menu_open: bool,
    double: bool,
    action: Option<StripAction>,
) -> StripPress {
    if !in_strip || menu_open {
        return StripPress::Ignore;
    }
    if button == MouseButton::Right {
        return match action {
            Some(StripAction::Select(index)) | Some(StripAction::Close(index)) => {
                StripPress::OpenTabMenu(index)
            }
            _ => StripPress::Act(StripAction::Ignore),
        };
    }
    if button != MouseButton::Left {
        return StripPress::Act(StripAction::Ignore);
    }
    // existing empty-band drag / toggle and action handling
}
```

Update every test call site from `true`/`false` to `MouseButton::Left`. Update the `on_mouse` call to pass `button`.

- [ ] **Step 4: Add app state and open/close**

```rust
struct TabMenuState {
    menu: ContextMenu,
    target: SessionId,
}

// in App:
tab_menu: Option<TabMenuState>,
```

Initialize `tab_menu: None`.

Add the imports near the existing `crate::ui::menu` import:

```rust
use crate::ui::context_menu::{ContextHit, ContextMenu, ContextPress, TabCommand};
```

```rust
fn open_tab_menu(&mut self, index: usize) {
    self.close_menu();
    self.clear_tooltip();
    if index >= self.tabs.len() {
        return;
    }
    // A context menu acts on the tab it was opened from. Select it first so
    // rename/color feedback is visible and the strip matches the target.
    self.tabs.select(index);
    self.selection = None;
    self.rebuild_layout();
    let Some(layout) = self.layout.as_ref() else {
        return;
    };
    let Some(tab) = layout.tabs.get(index) else {
        return;
    };
    let Some(session) = self.tabs.active() else {
        return;
    };
    let target = session.id;
    let total = self.tabs.len();
    let can_duplicate = session.profile.is_some();
    let colors = self
        .renderer
        .as_ref()
        .map(|renderer| renderer.theme.normal)
        .unwrap_or([crate::theme::Rgb::new(0, 0, 0); 8]);
    let items = crate::ui::tab_menu::rows(index, total, can_duplicate, &colors);
    let (width, height) = self.window.as_ref().map_or((0.0, 0.0), |window| {
        let size = window.inner_size();
        (size.width as f32, size.height as f32)
    });
    let scale = layout.strip_h / TAB_STRIP_HEIGHT;
    let mut menu = ContextMenu::new(tab.body, items, width, height, scale);
    menu.hover(self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
    self.tab_menu = Some(TabMenuState { menu, target });
    if let Some(window) = &self.window {
        window.request_redraw();
    }
}

fn close_tab_menu(&mut self) {
    if self.tab_menu.take().is_some() {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
```

`self.rebuild_layout()` before reading `layout` is important: selecting a tab changes the active tab body. If `rebuild_layout` is expensive, it is already the path every tab selection takes.

Add the temporary dispatcher so this task compiles before the later actions exist:

```rust
fn run_tab_command(&mut self, _target: SessionId, _command: TabCommand) -> bool {
    false
}
```

Task 10 replaces this stub with the real arms.

- [ ] **Step 5: Route pointer presses**

In `on_mouse`, before the existing `self.menu` branch:

```rust
if state == ElementState::Pressed {
    if let Some(tab_menu) = self.tab_menu.as_mut() {
        let (x, y) = (self.cursor_pos.0 as f32, self.cursor_pos.1 as f32);
        let press = match button {
            MouseButton::Left => tab_menu.menu.press(x, y),
            _ if tab_menu.menu.hit(x, y) == ContextHit::Outside => ContextPress::Dismiss,
            _ => ContextPress::KeepOpen,
        };
        self.release_forwarded_button();
        self.mouse_down = false;
        self.click_count = 0;
        self.last_click = None;
        self.press_in_chrome = true;
        match press {
            ContextPress::Dismiss => self.close_tab_menu(),
            ContextPress::OpenSubmenu(row) => {
                tab_menu.menu.open_submenu(0, row);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            ContextPress::Run(command) => {
                let target = tab_menu.target;
                self.close_tab_menu();
                let exit = self.run_tab_command(target, command);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
                return exit;
            }
            ContextPress::KeepOpen => {}
        }
        return false;
    }
}
```

`ContextHit` and `ContextPress` must be imported from `crate::ui::context_menu`.

In the `StripPress` match:

```rust
StripPress::OpenTabMenu(index) => self.open_tab_menu(index),
```

Update the double-click chain gate to `self.menu.is_none() && self.tab_menu.is_none()` and `strip_press`’s `menu_open` argument to either menu open. Right-click must not seed or pair a double-click chain.

- [ ] **Step 6: Hover, dismissal, and rendering**

- In `on_cursor_moved`, before the dropdown branch:

```rust
if let Some(tab_menu) = self.tab_menu.as_mut() {
    tab_menu.menu.hover(x as f32, y as f32);
    if let Some(window) = &self.window {
        window.request_redraw();
    }
    return;
}
```

- In `on_key`, before the dropdown Escape branch:

```rust
if self.tab_menu.is_some() {
    if matches!(event.logical_key, Key::Named(NamedKey::Escape)) {
        self.close_tab_menu();
        return false;
    }
    // The menu is mouse-driven; any other key belongs to the terminal or a
    // shortcut, so dismiss the layer and let the normal chain handle it.
    self.close_tab_menu();
}
```

- In `WindowEvent::CursorLeft`, add `if let Some(tab_menu) = self.tab_menu.as_mut() { tab_menu.menu.clear_hover(); }` and request a redraw when it was shown.
- On resize, close the tab menu (its anchor and window bounds changed): in the resize arm, call `self.close_tab_menu()`.
- On focus loss, close the tab menu as well.
- In `render`, after the tooltip (menus paint above the grid but below nothing else except the tooltip, which is suppressed):

```rust
if let Some(tab_menu) = self.tab_menu.as_ref() {
    renderer.draw_context_menu(&tab_menu.menu);
}
if let Some(menu) = self.menu.as_ref() {
    renderer.draw_menu(menu, &menu_icons);
}
```

- Suppress tooltips while either menu is open by extending `tooltip_target`’s early return to `if self.menu.is_some() || self.tab_menu.is_some() { return None; }`.

- [ ] **Step 7: Run tests**

Run: `cargo test app::tests::right_press`

Expected: all pass.

- [ ] **Step 8: Commit**

```text
git add src/app.rs
git commit -m "feat: open the tab context menu on right click"
```

---

## Task 6: Implement Inline Rename

**Files:**
- Create: `src/ui/rename.rs`
- Create: `src/render/rename.rs`
- Modify: `src/app.rs`
- Modify: `src/render/mod.rs`

**Interfaces:**
- Consumes: `SessionId`, `TabBarLayout`, `TabSet::rename`.
- Produces:
  - `RenameState { id: SessionId, text: String }` with `push`, `backspace`.
  - `Renderer::draw_rename(&mut self, tab: Rect, current: &str, rename: &RenameState, scale: f32)`.
  - `App::rename: Option<RenameState>`.
  - `App::start_rename(id)`, `rename_key(event) -> bool`, `commit_rename()`.

- [ ] **Step 1: Write failing state tests**

```rust
#[test]
fn typing_appends_and_backspace_removes() {
    let mut state = RenameState::new(7, "Build");
    state.push(' ');
    state.push('x');
    state.backspace();
    assert_eq!(state.text, "Build ");
}

#[test]
fn control_characters_are_ignored() {
    let mut state = RenameState::new(7, "");
    state.push('\n');
    state.push('\t');
    state.push('a');
    assert_eq!(state.text, "a");
}
```

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test ui::rename`

Expected: module missing.

- [ ] **Step 3: Add the state and start/commit**

```rust
pub struct RenameState {
    pub id: SessionId,
    pub text: String,
}

impl RenameState {
    pub fn new(id: SessionId, current: &str) -> Self {
        Self { id, text: current.to_string() }
    }

    pub fn push(&mut self, ch: char) {
        if !ch.is_control() {
            self.text.push(ch);
        }
    }

    pub fn backspace(&mut self) {
        self.text.pop();
    }
}
```

```rust
fn start_rename(&mut self, id: SessionId) {
    let Some(index) = self.tabs.position(id) else { return };
    let current = self.tabs.titles().get(index).copied().unwrap_or(FALLBACK_TITLE).to_string();
    self.close_tab_menu();
    self.rename = Some(RenameState::new(id, &current));
    self.selection = None;
    self.rebuild_layout();
    if let Some(window) = &self.window {
        window.request_redraw();
    }
}

fn commit_rename(&mut self) {
    let Some(rename) = self.rename.take() else { return };
    self.tabs.rename(rename.id, &rename.text);
    self.rebuild_layout();
    if let Some(window) = &self.window {
        window.request_redraw();
    }
}
```

Do not clear the terminal selection in `start_rename` if that creates a borrow conflict with `titles()`; compute `current` first, then mutate.

- [ ] **Step 4: Handle keys and IME**

At the top of `on_key`, immediately after `blink.restart`:

```rust
if self.rename.is_some() {
    return self.rename_key(&event);
}
```

```rust
fn rename_key(&mut self, event: &KeyEvent) -> bool {
    match &event.logical_key {
        Key::Named(NamedKey::Escape) => {
            self.rename = None;
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        Key::Named(NamedKey::Enter) => self.commit_rename(),
        Key::Named(NamedKey::Backspace) => {
            if let Some(rename) = self.rename.as_mut() {
                rename.backspace();
            }
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        Key::Character(text) if !self.modifiers.control_key() && !self.modifiers.alt_key() => {
            if let Some(rename) = self.rename.as_mut() {
                for ch in text.chars() {
                    rename.push(ch);
                }
            }
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        _ => {}
    }
    false
}
```

In `WindowEvent::Ime(Ime::Commit(text))`, replace the direct `send_text` with:

```rust
if let Some(rename) = self.rename.as_mut() {
    for ch in text.chars() {
        rename.push(ch);
    }
    if let Some(window) = &self.window {
        window.request_redraw();
    }
} else {
    self.send_text(&text);
}
```

Task 9 adds the find branch before the final `else`.

- [ ] **Step 5: Paint the inline field**

`src/render/rename.rs`:

```rust
impl Renderer {
    pub fn draw_rename(&mut self, tab: Rect, current: &str, rename: &RenameState, scale: f32) {
        let field = Rect {
            x: tab.x + 6.0 * scale,
            y: tab.y + 5.0 * scale,
            w: (tab.w - 12.0 * scale).max(1.0),
            h: (tab.h - 10.0 * scale).max(1.0),
        };
        self.fill_round_rect(field, 4.0 * scale, self.theme.control_hover);
        let px = TAB_TITLE_SIZE * scale;
        let text = if rename.text.is_empty() { current } else { &rename.text };
        let color = if rename.text.is_empty() {
            self.theme.menu_shortcut
        } else {
            self.theme.tab_title
        };
        let style = TextStyle {
            face: SEMIBOLD_FONT_ID,
            px,
            color,
        };
        let text_x = field.x + 6.0 * scale;
        let max_width = (field.w - 12.0 * scale).max(1.0);
        ui_text::draw_text_ellipsis_in(
            self,
            text,
            text_x,
            field.y + (field.h - px) * 0.5,
            style,
            max_width,
        );
        if !rename.text.is_empty() {
            let caret_x = text_x + self.ui_text_width(style, &rename.text).min(max_width - 2.0 * scale);
            self.fill_rect(
                caret_x,
                field.y + 4.0 * scale,
                1.0,
                field.h - 8.0 * scale,
                self.theme.cursor,
            );
        }
    }
}
```

If `ui_text::draw_text_ellipsis_in` needs the renderer’s font set, call it exactly as `draw_titles` does. Add a small `ui_text_width` helper in `render/mod.rs` if one does not exist:

```rust
fn ui_text_width(&self, style: TextStyle, text: &str) -> f32 {
    self.fonts.width_in(style.face, text, style.px)
}
```

In `render`, after `draw_chrome` and before the menus:

```rust
if let (Some(rename), Some(layout)) = (self.rename.as_ref(), self.layout.as_ref()) {
    if let Some(index) = self.tabs.position(rename.id) {
        if let Some(tab) = layout.tabs.get(index) {
            renderer.draw_rename(tab.body, &current_title, rename, layout.strip_h / TAB_STRIP_HEIGHT);
        }
    }
}
```

`current_title` is the current title for the same index, cloned from `self.tabs.titles()` before the renderer borrow. Do not borrow `self.tabs` while `renderer` is mutably borrowed.

Add `pub mod rename;` to `src/ui/mod.rs` and `mod rename;` to the renderer module list in `src/render/mod.rs`.

- [ ] **Step 6: Run tests and hand check**

Run: `cargo test ui::rename`

Expected: pass.

Manual: right-click a tab, choose Rename, type, Backspace, Enter. Escape restores the old title. A shell OSC title must not overwrite the committed name.

- [ ] **Step 7: Commit**

```text
git add src/ui/rename.rs src/render/rename.rs src/app.rs src/render/mod.rs
git commit -m "feat: rename tabs inline"
```

---

## Task 7: Paint Tab Colors

**Files:**
- Modify: `src/render/mod.rs`
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: `Theme::ink_on`, `TabSet::color`.
- Produces:
  - `draw_chrome(..., colors: &[Option<Rgb>], ...)`
  - `draw_tabs` and `draw_titles` accept colors.
  - Active and inactive tabs fill with their color; title/close ink switches for contrast.

- [ ] **Step 1: Write failing render tests**

In `src/render/mod.rs` tests, build a `TabBarLayout`, pass a color for tab 0, draw chrome, and assert the tab body’s center pixel equals that color. Add a second test: a light color uses `theme.background` as title ink; a dark color uses `theme.foreground`.

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test render::tests::tab_color`

Expected: signature mismatch or missing color.

- [ ] **Step 3: Thread colors through the painter**

In `draw_chrome`:

```rust
pub fn draw_chrome(
    &mut self,
    layout: &TabBarLayout,
    ui_px: f32,
    active: usize,
    icons: &[Option<TabIcon>],
    colors: &[Option<Rgb>],
    hover: Option<StripHover>,
) {
    // existing body; pass colors to draw_tabs
}
```

In `draw_tabs`, add `colors: &[Option<Rgb>]` as a parameter and use it for the fill. In `draw_titles`, add the same parameter and use it for the ink.

```rust
let color = colors.get(index).copied().flatten();
let fill = color.or_else(|| (index == active).then_some(self.theme.tab_active));
if let Some(fill) = fill {
    self.fill_round_top(tab.body, TAB_RADIUS * k, fill);
}
```

Inactive tabs with no color remain unfilled, preserving current behavior.

In `draw_titles`, compute per-tab ink:

```rust
let color = colors.get(tab.index).copied().flatten();
let title_color = match color {
    Some(color) => self.theme.ink_on(color),
    None if tab.index == active => self.theme.tab_title,
    None => self.theme.tab_title_inactive,
};
```

Use `title_color` for the title and the close glyph; keep the existing hover close ink (`theme.tab_close_hover_ink`) when the pill is shown.

- [ ] **Step 4: Pass colors from `App`**

In `render`, clone colors alongside icons before the renderer borrow:

```rust
let colors: Vec<Option<Rgb>> = (0..self.tabs.len())
    .map(|index| self.tabs[index].color)
    .collect();
```

Call `renderer.draw_chrome(layout, ui_px, active, &icons, &colors, self.strip_hover)`.

- [ ] **Step 5: Wire the color command**

In `run_tab_command`, handle `TabCommand::ChangeColor(color)`:

```rust
if let Some(index) = self.tabs.position(target) {
    let id = self.tabs[index].id;
    if self.tabs.set_color(id, color) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
```

No layout rebuild is needed for a color change.

- [ ] **Step 6: Run tests**

Run: `cargo test render::tests`

Expected: all pass, including existing chrome tests updated for the new argument.

- [ ] **Step 7: Commit**

```text
git add src/render/mod.rs src/app.rs
git commit -m "feat: paint per-tab colors"
```

---

## Task 8: Export Buffer Text

**Files:**
- Modify: `src/term.rs`
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: `Terminal` grid, `alacritty_terminal::index::Line`, `Flags`, `rfd::FileDialog`.
- Produces:
  - `Terminal::all_text(&self) -> String`.
  - `App::export_text(&mut self)`.

- [ ] **Step 1: Write failing text-export tests**

```rust
#[test]
fn all_text_includes_scrollback_and_trimmed_rows() {
    let mut terminal = terminal(10, 3, 100);
    terminal.feed(b"first\r\nsecond   \r\n");
    let text = terminal.all_text();
    assert!(text.contains("first\nsecond"));
}
```

Use the existing test terminal helper/`feed` method. If the helper is named differently, reuse it.

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test term::tests::all_text`

Expected: method missing.

- [ ] **Step 3: Implement `all_text`**

```rust
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
            text.push(cell.c);
        }
        out.push_str(text.trim_end());
        out.push('\n');
    }
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}
```

Keep the final newline for an ordinary buffer; only collapse the extra blank lines produced by unused viewport rows. Check the exact trailing behavior in the test and pin it.

- [ ] **Step 4: Implement the export command**

```rust
fn export_text(&mut self) {
    let Some(session) = self.tabs.active() else {
        return;
    };
    let text = session.terminal.all_text();
    let Some(path) = rfd::FileDialog::new()
        .set_title("Export text")
        .set_file_name("termyx.txt")
        .add_filter("Text", &["txt"])
        .save_file()
    else {
        return;
    };
    if let Err(err) = std::fs::write(&path, text) {
        tracing::warn!(%err, "could not export terminal text");
        rfd::MessageDialog::new()
            .set_title("Export failed")
            .set_description(err.to_string())
            .set_level(rfd::MessageLevel::Error)
            .show();
    }
}
```

`FileDialog` is blocking and runs on the UI thread. That is acceptable for a one-shot save; do not add a background thread just for this.

- [ ] **Step 5: Run tests and hand check**

Run: `cargo test term::tests::all_text`

Expected: pass.

Manual: right-click a tab, choose Export text, save to a file, and confirm the current buffer plus scrollback is present.

- [ ] **Step 6: Commit**

```text
git add src/term.rs src/app.rs
git commit -m "feat: export terminal text to a file"
```

---

## Task 9: Find in the Buffer

**Files:**
- Create: `src/find.rs`
- Create: `src/ui/find_bar.rs`
- Create: `src/render/find_bar.rs`
- Modify: `src/main.rs`
- Modify: `src/term.rs`
- Modify: `src/render/mod.rs`
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: `Terminal` grid, `VisualMap`, `Cell`, `Line`.
- Produces:
  - `term::Match { line: i32, start: usize, end: usize }` (column indices, ASCII case-insensitive).
  - `Terminal::find(&self, needle: &str) -> Vec<Match>` and `Terminal::reveal_line(&mut self, line: i32)`.
  - `FindState { query, matches, current }` with `push_str`, `backspace`, `next`, `previous`, `current`.
  - `FindBar::{new, hit}`.
  - `Renderer::draw_find_bar` and find highlights inside `draw`.

- [ ] **Step 1: Write failing search tests**

```rust
#[test]
fn find_is_ascii_case_insensitive_and_reports_columns() {
    let mut terminal = terminal(20, 3, 100);
    terminal.feed(b"alpha BETA\r\nbeta\r\n");
    let hits = terminal.find("beta");
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0], crate::term::Match { line: 0, start: 6, end: 10 });
}

#[test]
fn an_empty_query_has_no_matches() {
    let mut terminal = terminal(20, 3, 100);
    assert!(terminal.find("").is_empty());
}
```

Add a `FindState` test: `next` wraps from the last match to the first, `previous` wraps the other way, and an empty match list yields `None`.

- [ ] **Step 2: Run and confirm failure**

Run: `cargo test term::tests::find`

Expected: `find` missing.

- [ ] **Step 3: Implement buffer search and reveal**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub line: i32,
    pub start: usize,
    pub end: usize,
}

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
                hits.push(Match { line, start, end: start + needle.len() });
            }
        }
    }
    hits
}

pub fn reveal_line(&mut self, line: i32) {
    let history = self.history_size() as i32;
    let current_top = -(self.display_offset() as i32);
    let desired_top = if line < 0 { line.clamp(-history, 0) } else { 0 };
    let delta = current_top - desired_top;
    if delta != 0 {
        self.scroll_display(delta);
    }
}
```

`find` scans the full grid including scrollback. It is row-local: a match split across a wrapped line is not found. Document that limitation in the function comment.

- [ ] **Step 4: Add `FindState`**

`src/find.rs`:

```rust
use crate::term::Match;

#[derive(Debug, Default)]
pub struct FindState {
    pub query: String,
    pub matches: Vec<Match>,
    pub current: usize,
}

impl FindState {
    pub fn push_str(&mut self, text: &str) {
        self.query.push_str(text);
    }

    pub fn backspace(&mut self) {
        self.query.pop();
    }

    pub fn set_matches(&mut self, matches: Vec<Match>) {
        self.matches = matches;
        self.current = 0;
    }

    pub fn current(&self) -> Option<Match> {
        self.matches.get(self.current).copied()
    }

    pub fn next(&mut self) -> Option<Match> {
        if self.matches.is_empty() {
            return None;
        }
        self.current = (self.current + 1) % self.matches.len();
        self.current()
    }

    pub fn previous(&mut self) -> Option<Match> {
        if self.matches.is_empty() {
            return None;
        }
        self.current = (self.current + self.matches.len() - 1) % self.matches.len();
        self.current()
    }
}
```

Add `mod find;` to `main.rs`.

- [ ] **Step 5: Add find-bar geometry and painting**

`src/ui/find_bar.rs`:

```rust
pub const FIND_BAR_WIDTH: f32 = 320.0;
pub const FIND_BAR_HEIGHT: f32 = 36.0;

pub struct FindBar {
    pub rect: Rect,
    pub field: Rect,
    pub close: Rect,
}

pub enum FindHit {
    Field,
    Close,
    Outside,
}

impl FindBar {
    pub fn new(window_width: f32, top: f32, scale: f32) -> Self {
        let width = FIND_BAR_WIDTH * scale;
        let height = FIND_BAR_HEIGHT * scale;
        let rect = Rect {
            x: (window_width - width - 12.0 * scale).max(0.0),
            y: top + 8.0 * scale,
            w: width.min(window_width),
            h: height,
        };
        let close = Rect { x: rect.x + rect.w - 28.0 * scale, y: rect.y + 6.0 * scale, w: 20.0 * scale, h: 24.0 * scale };
        let field = Rect { x: rect.x + 10.0 * scale, y: rect.y + 6.0 * scale, w: rect.w - 44.0 * scale, h: 24.0 * scale };
        Self { rect, field, close }
    }

    pub fn hit(&self, x: f32, y: f32) -> FindHit {
        if self.close.contains(x, y) { FindHit::Close }
        else if self.field.contains(x, y) { FindHit::Field }
        else if self.rect.contains(x, y) { FindHit::Field }
        else { FindHit::Outside }
    }
}
```

Paint: panel `fill_round_rect` with `theme.menu_background`, 1px border `theme.menu_separator`, query text or `"Find"` placeholder in `theme.menu_shortcut`, count at right (`"3/12"` or `"No results"`), close `GLYPH_CLOSE`. Reuse `menu` metrics and `ui_text`.

Add `pub mod find_bar;` to `src/ui/mod.rs`, `mod find_bar;` to `src/render/mod.rs`, and `mod find;` to `src/main.rs` in this task.

- [ ] **Step 6: Integrate with app state**

Add `find: Option<FindState>` to `App`, initialize `None`.

```rust
fn start_find(&mut self) {
    self.close_tab_menu();
    self.rename = None;
    self.find = Some(FindState::default());
    if let Some(window) = &self.window {
        window.request_redraw();
    }
}

fn close_find(&mut self) {
    if self.find.take().is_some() {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

fn refresh_find(&mut self) {
    let Some(find) = self.find.as_ref() else { return };
    let query = find.query.clone();
    let matches = self.tabs.active().map(|session| session.terminal.find(&query)).unwrap_or_default();
    let Some(find) = self.find.as_mut() else { return };
    find.set_matches(matches);
    if let Some(first) = find.current() {
        if let Some(session) = self.tabs.active_mut() {
            session.terminal.reveal_line(first.line);
        }
    }
    if let Some(window) = &self.window {
        window.request_redraw();
    }
}
```

At the top of `on_key`, after the rename branch:

```rust
if self.find.is_some() {
    return self.find_key(&event);
}
```

`find_key`: Escape closes; Enter/Shift+Enter next/previous; Backspace; Character appends; on any query change call `refresh_find` and reveal the current match. Modified shortcuts while the find bar is open are ignored, so typing does not reach the shell.

```rust
fn find_key(&mut self, event: &KeyEvent) -> bool {
    let shift = self.modifiers.shift_key();
    match &event.logical_key {
        Key::Named(NamedKey::Escape) => self.close_find(),
        Key::Named(NamedKey::Enter) => {
            let Some(find) = self.find.as_mut() else { return false };
            let hit = if shift { find.previous() } else { find.next() };
            if let (Some(hit), Some(session)) = (hit, self.tabs.active_mut()) {
                session.terminal.reveal_line(hit.line);
            }
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        Key::Named(NamedKey::Backspace) => {
            if let Some(find) = self.find.as_mut() {
                find.backspace();
            }
            self.refresh_find();
        }
        Key::Character(text) if !self.modifiers.control_key() && !self.modifiers.alt_key() => {
            if let Some(find) = self.find.as_mut() {
                find.push_str(text);
            }
            self.refresh_find();
        }
        _ => {}
    }
    false
}
```

IME commit: add a `find` branch before the final `else` in the `Ime::Commit` handler:

```rust
} else if let Some(find) = self.find.as_mut() {
    find.push_str(&text);
    self.refresh_find();
} else {
```

Mouse: if a press hits `FindBar::Close`, close the find bar and consume the press. A press inside the bar consumes; a press outside falls through to the grid. This lets the user click back into the terminal while the bar stays open.

Also close the find bar when the active tab is selected or closed, and make `start_rename` call `close_find()`; the two overlays must not both be visible.

Add Ctrl+F as a shortcut in `on_key`, after the find/rename branches and before terminal input:

```rust
if self.modifiers.control_key()
    && matches!(&event.logical_key, Key::Character(text) if text.eq_ignore_ascii_case("f"))
{
    self.start_find();
    return false;
}
```

Render find bar after `draw_chrome`/rename and before menus:

```rust
let scale = self.scale_factor() as f32;
let window_width = self.window.as_ref().map_or(0.0, |window| window.inner_size().width as f32);
if let (Some(find), Some(renderer)) = (self.find.as_ref(), self.renderer.as_mut()) {
    let bar = FindBar::new(window_width, renderer.top_inset() as f32, scale);
    renderer.draw_find_bar(&bar, find);
}
```

- [ ] **Step 7: Add find highlights**

Change `Renderer::draw` to accept `find: Option<&FindState>`. After `draw_cell_backgrounds` and before the text runs, draw matches:

```rust
if let Some(find) = find {
    for (index, hit) in find.matches.iter().enumerate() {
        let row = hit.line + snapshot.display_offset as i32;
        if row < 0 || row as usize >= snapshot.lines.len() {
            continue;
        }
        let color = if index == find.current { self.theme.find_current } else { self.theme.find_match };
        // Map logical columns to visual columns with the row's VisualMap.
        let cols = snapshot.lines[row as usize].cells.len();
        let start = hit.start.min(cols);
        let end = hit.end.min(cols);
        let map = &maps[row as usize];
        let mut columns: Vec<usize> = (start..end).map(|col| map.visual_of(col)).collect();
        columns.sort_unstable();
        columns.dedup();
        for col in columns {
            self.fill_rect(
                padding + col as f32 * cell_w,
                top + row as f32 * cell_h,
                cell_w,
                cell_h,
                color,
            );
        }
    }
}
```

`padding` and `top` are the locals already computed in `draw`; do not add a second origin. Highlighting one visual cell at a time is correct for RTL/bidi; merging adjacent columns is a later optimization. Update all `draw` callers and render tests with the extra argument (`None` where not testing find).

- [ ] **Step 8: Run tests and hand check**

Run: `cargo test term::tests::find ui::find_bar`

Expected: all pass.

Manual: Ctrl+F or menu Find, type a string present in scrollback, press Enter repeatedly, confirm the view scrolls to each match and the current match is brighter. Check an RTL row if the config enables bidi.

- [ ] **Step 9: Commit**

```text
git add src/find.rs src/ui/find_bar.rs src/render/find_bar.rs src/term.rs src/render/mod.rs src/app.rs src/main.rs
git commit -m "feat: find text in the terminal buffer"
```

---

## Task 10: Wire Every Row and Finish the Disabled States

**Files:**
- Modify: `src/app.rs`
- Modify: `docs/superpowers/plans/2026-11-11-termyx-tab-context-menu.md` (this file, follow-up notes)

**Interfaces:**
- Consumes: all previous tasks.
- Produces: `App::run_tab_command` dispatch for every `TabCommand`; menu closes before executing.

- [ ] **Step 1: Write failing dispatch tests**

Add a pure test that every `TabCommand` maps to an expected label/enabled state from `ui::tab_menu::rows`. This catches a newly added command with no row.

- [ ] **Step 2: Implement `run_tab_command`**

```rust
fn run_tab_command(&mut self, target: SessionId, command: TabCommand) -> bool {
    let Some(index) = self.tabs.position(target) else {
        return false;
    };
    match command {
        TabCommand::ChangeColor(color) => {
            if self.tabs.set_color(target, color) {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            false
        }
        TabCommand::Rename => {
            self.start_rename(target);
            false
        }
        TabCommand::Duplicate => {
            if let Err(err) = self.duplicate_tab(target) {
                tracing::warn!(%err, "could not duplicate tab");
            }
            false
        }
        // Rows are disabled until panes and multi-window sessions exist.
        TabCommand::Split | TabCommand::MoveToNewWindow => false,
        TabCommand::MoveLeft => {
            if index > 0 {
                self.tabs.move_tab(index, index - 1);
                self.rebuild_layout();
            }
            false
        }
        TabCommand::MoveRight => {
            if index + 1 < self.tabs.len() {
                self.tabs.move_tab(index, index + 1);
                self.rebuild_layout();
            }
            false
        }
        TabCommand::ExportText => {
            self.export_text();
            false
        }
        TabCommand::Find => {
            self.start_find();
            false
        }
        TabCommand::CloseOtherTabs => self.close_other_tabs(target),
        TabCommand::CloseTabsToRight => self.close_tabs_to_right(target),
        TabCommand::CloseTab => self.close_tab(index),
    }
}
```

`run_tab_command` is called after `close_tab_menu()`, so the target id is passed in rather than read from `self.tab_menu`. In `on_mouse`, copy `tab_menu.target` before closing the menu.

- [ ] **Step 3: Hand-check every row**

Run the app: `cargo run`.

- Right-click a tab: menu opens under it, selected tab is active.
- Hover rows: only enabled rows get the pill.
- Move tab >: submenu opens right; at the left edge Move left is grey; at the right edge Move right is grey; with one tab both are grey.
- Close >: Close other tabs and Close tabs to right enable/disable with tab count and position.
- Change tab color: choose a swatch; both active and inactive tab paint with it and title ink stays readable.
- Rename: inline field, IME input, Enter commits, Escape cancels; OSC title no longer overwrites.
- Duplicate tab: new shell tab with the same profile and color; page tabs grey the row.
- Export text: save dialog writes a file.
- Find: highlight/scroll/Enter cycling.
- Split tab and Move tab to new window: grey and inert.
- Escape, click-away, window resize, focus loss: menu closes.
- Existing `⌄` dropdown still opens, closes the tab menu, and behaves as before.

- [ ] **Step 4: Run the full suite**

Run: `cargo test`

Expected: all pass.

- [ ] **Step 5: Commit**

```text
git add src/app.rs
git commit -m "feat: wire tab context menu actions"
```

---

## Task 11: Final Verification and Follow-Up Notes

**Files:**
- Modify: this plan (append the follow-up section if it is not already present)
- No source changes.

- [ ] **Step 1: Run all gates**

```text
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Expected: each command exits 0 with no warnings.

- [ ] **Step 2: Check the tree**

```text
git status --short
git diff HEAD --stat
```

Expected: clean working tree after the final commit; the diff stat names only the files in this plan.

- [ ] **Step 3: Record the deferred work**

Add this section to the plan if it is missing:

```markdown
## Deferred: Split Tab

Split needs a pane tree inside a tab. Sketch:
1. Replace the 1:1 `TabSet` session mapping with `Tab { panes: PaneTree, focused: PaneId, active: PaneId }`.
2. Each leaf owns its own `Terminal`, `PtySession`, and `SessionId`; tab titles stay on the tab.
3. `TabBarLayout`/`TabSet` keep the tab order; the renderer receives a pane viewport for each leaf.
4. Input is routed to the focused leaf; PTY resize is per leaf.
5. The context row becomes a `Split right` / `Split down` submenu and creates a leaf using the focused pane's profile.

## Deferred: Move Tab to New Window

Move-to-new-window needs multiple windows. Sketch:
1. Extract the per-window state (`window`, `surface`, `renderer`, `layout`, menu/rename/find state, cursor) into `WindowState`.
2. `App` holds `windows: HashMap<WindowId, WindowState>` plus the shared `config`, `config_path`, `proxy`, and clipboard.
3. Route `window_event`/`redraw` by `WindowId`.
4. Move the tab's `Session` out of the source `TabSet`, push it into the destination's `TabSet`, and rebuild both strips.
5. Close the source window when its last tab moves, unless it is the only window.

Both are separate plans; this plan must not half-implement either.
```

- [ ] **Step 4: Commit the notes**

```text
git add docs/superpowers/plans/2026-11-11-termyx-tab-context-menu.md
git commit -m "docs: record tab context menu follow-ups"
```

---

## Self-Review

- **Reference coverage:** The menu contains every row in the screenshot. `Split tab` and `Move tab to new window` are visible and disabled, with explicit follow-up architecture; the rest are functional.
- **Placement:** The panel anchors to the tab body, clamps to the window, drops overflow rows, and never opens above the strip. Submenus open right or flip left when the right edge would overflow.
- **Input ownership:** Right-clicking a tab or its `✕` opens the menu; right-clicking other strip chrome is swallowed; right-clicking the grid still pastes. Escape, click-away, resize, and focus loss close the menu.
- **Target stability:** The menu stores a `SessionId`, not an index, so a move/close/retitle cannot make it act on the wrong tab.
- **Title behavior:** Rename locks the title against OSC; a blank rename clears the lock and restores the fallback title.
- **Color behavior:** Colors are per-session and survive reorder/duplicate; no config persistence is added (out of scope).
- **Find scope:** Full scrollback, ASCII case-insensitive, row-local; wrapped-line and Unicode case-folding are documented limitations, not silent bugs.
- **Tests:** Model geometry, submenu flipping, disabled rows, separator gaps, right-click routing, reorder/active preservation, title locking, text export, search, and painter ink are covered. Dialog and real-shell flows are hand checks.
- **No new crates:** `rfd` and all rendering primitives already exist.

---

## Deferred: Split Tab

Split needs a pane tree inside a tab. Sketch:
1. Replace the 1:1 `TabSet` session mapping with `Tab { panes: PaneTree, focused: PaneId, active: PaneId }`.
2. Each leaf owns its own `Terminal`, `PtySession`, and `SessionId`; tab titles stay on the tab.
3. `TabBarLayout`/`TabSet` keep the tab order; the renderer receives a pane viewport for each leaf.
4. Input is routed to the focused leaf; PTY resize is per leaf.
5. The context row becomes a `Split right` / `Split down` submenu and creates a leaf using the focused pane's profile.

## Deferred: Move Tab to New Window

Move-to-new-window needs multiple windows. Sketch:
1. Extract the per-window state (`window`, `surface`, `renderer`, `layout`, menu/rename/find state, cursor) into `WindowState`.
2. `App` holds `windows: HashMap<WindowId, WindowState>` plus the shared `config`, `config_path`, `proxy`, and clipboard.
3. Route `window_event`/`redraw` by `WindowId`.
4. Move the tab's `Session` out of the source `TabSet`, push it into the destination's `TabSet`, and rebuild both strips.
5. Close the source window when its last tab moves, unless it is the only window.

Both are separate plans; this plan must not half-implement either.
