# Termyx Command Palette - Design

Date: 2026-11-12
Status: approved design, spec for review

## Goal

Turn the Command palette tab into a real command launcher. It lists profiles,
tab actions, and in-app tools in one filterable, ranked list, and Enter runs the
selected command against the tab the palette was opened from.

The palette stays a page tab (generated terminal text in its own `Terminal`).
No second render path, no new crate.

## Non-goals

- Spellcheck and autocorrect toggles: they would write config from a keypress.
- Tab rename and colour: both need a second input step.
- Split tab and Move to new window: their tab-menu rows stay disabled.
- Aliases, recency ranking, pinning, multi-word queries.
- Any change to how pages are created, refilled, or closed.

## Current state

`src/page.rs` builds the palette as terminal text and filters rows by
case-insensitive substring. The catalog holds only profiles, Settings, and
About. The palette tab is the active tab when Enter is pressed, so it cannot
offer tab commands: they would act on the palette itself. The `⌄` menu row
shows no shortcut because none exists.

## Architecture

Three parts with one job each:

- `src/palette.rs` (new): pure command catalog, query state, ranking, and row
  availability. No window, terminal, or session types.
- `src/page.rs`: owns a `Palette` next to `SettingsState` and renders it as
  text. Keeps the existing `RESET_SCREEN` refill behaviour.
- `src/app.rs`: owns the target tab, opens and reuses the palette tab, and
  dispatches the chosen command through code that already exists
  (`run_tab_command`, `new_tab`, `open_page`, `start_find`, `export_text`,
  `cycle_tab`).

### Command model

```rust
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
    About,
    Find,
    ExportText,
}

pub struct PaletteRow {
    pub label: String,
    pub shortcut: &'static str,
    pub command: PaletteCommand,
}
```

`PaletteRow::shortcut` names a key only when that key really runs the command:

| Command | Shortcut |
| --- | --- |
| profile 0..4 | `Ctrl+Shift+1` .. `Ctrl+Shift+5` |
| profile 5.. | empty |
| Close tab | `Ctrl+Shift+W` |
| Next tab / Previous tab | `Ctrl+Tab` / `Ctrl+Shift+Tab` |
| Find | `Ctrl+F` |
| Settings, About, duplicate, move, close others, close to right, export | empty |

The profile rows are the only "new tab" command; there is no separate New tab
row, so `Ctrl+Shift+T` (open the default profile) is not repeated in the
shortcut column. Duplicate tab, the move commands, the bulk closes, and Export
text are reachable from the tab context menu, not from a key, so their shortcut
column stays empty.

### Catalog order

Profiles in configured order (the first five carry their `Ctrl+Shift+1` .. `5`
hint), then tab commands beginning with `Close tab`, then Settings, Find,
Export text, About. With an empty query this is the displayed order.

### Availability

`PaletteContext { tabs: usize, position: usize }` is computed on every refill:
`tabs` is the number of tabs other than the palette, `position` is how many of
those sit before the target tab. It omits rows that cannot act:

- Move left when `position == 0`.
- Move right and Close tabs to right when `position + 1 >= tabs`.
- Close other tabs when `tabs < 2`.
- Next tab and Previous tab when `tabs < 2`.

Close tab, duplicate, find, export, profiles, and pages are always present.
Omitting a row is honest; showing one that silently does nothing is not.

### Ranking

Case-insensitive subsequence match, scored in one pass:

- start at +20 if the label starts with the query;
- +8 for a matched character at a word start (index 0, after a space, `-`, or
  `_`), +4 for any other matched character;
- +6 for each matched character that continues the previous matched position
  (a contiguous run).

Rows with no match are dropped. Sort by score descending, ties keep catalog
order. `selected` resets to 0 whenever the query changes. An empty query
matches everything with score 0.

### Rendering

`Palette::text(config, cols, rows)` emits, after `RESET_SCREEN`:

1. bold `Command palette`
2. blank
3. `> {query}` followed by a reverse-video block as the caret
4. blank
5. one line per visible row: two leading spaces, label, right-aligned shortcut
   at `cols`, the selected row in reverse video across the width
6. blank
7. dim `Up/Down select, Enter run, Esc close`

At most `rows - 6` rows are shown. When the selection would fall past the
window, the window starts at `selected + 1 - visible` so the highlight stays on
screen. A label too long for the shortcut column is truncated with `..`.
Empty result set: dim `No matching commands` in place of the rows.

## Input and target

- `Ctrl+Shift+P` opens the palette from any tab. It is checked beside the
  existing `Ctrl+F` branch, before the shell fallback, and only when no other
  layer (spellcheck menu, rename, find, tab menu) is open, matching how menus
  already take precedence.
- If a palette tab already exists, focus it and reset its query; do not open a
  second one. The target is recaptured from the currently active tab unless the
  active tab is the palette itself, in which case the stored target is kept.
- `palette_target: Option<SessionId>` is captured before the palette tab opens,
  so Enter knows which tab to act on.
- Enter: read the command, close the palette tab, re-select the origin tab
  explicitly, then dispatch. Closing alone activates the tab to the left of the
  palette, not the origin, so `focus_palette_target` selects the captured tab
  before anything runs. Next tab, Previous tab, Find, and Export text therefore
  behave as if the palette had never been there.
- Escape closes the palette tab, as pages already do.
- Typing, Backspace, and Up/Down keep the existing `page_key` paths, now backed
  by `Palette` instead of the old fields.

## Dispatch

| Command | Action |
| --- | --- |
| `Profile(index)` | `new_tab(index)`, warn on error |
| `CloseTab` | `run_tab_command(target, TabCommand::CloseTab)` |
| `DuplicateTab` | `run_tab_command(target, TabCommand::Duplicate)` |
| `MoveTabLeft` / `MoveTabRight` | `run_tab_command(target, TabCommand::MoveLeft / MoveRight)` |
| `CloseOtherTabs` / `CloseTabsToRight` | `run_tab_command` with the matching variant |
| `NextTab` / `PreviousTab` | `cycle_tab(1)` / `cycle_tab(-1)` after the origin is re-selected |
| `Settings`, `About` | `open_page(PageKind::Settings / About)` |
| `Find` | `start_find()` |
| `ExportText` | `export_text()` |

`run_tab_command` already returns `false` when the target is gone and already
guards the move edges, so the dispatch layer only needs to log at debug when
there is no target. The re-selection runs before dispatch so tab-relative
commands see the origin. The palette never writes config, never sends PTY bytes,
and never starts a shell except through `Profile`.

## Page changes

`Page` drops `PageAction`, `PaletteEntry`, and `selected_action`; it gains a
`palette: Palette` field, `palette_command(config)` for Enter (reading the
context stored by `set_palette_context`), and delegates `palette_text` to
`palette::Palette::text`. The app stops mapping through
`palette_action(PageAction)`. Leaving the old types in place would fail
`clippy -D warnings` as dead code.

## Errors

- Enter with no matches: nothing happens, the palette stays open.
- Target tab vanished: debug log, no action, palette already closed.
- Profile index out of range: `new_tab` keeps its existing substitution rule.
- Dispatch never sees `Split` or `MoveToNewWindow`; they are absent from
  `PaletteCommand`.

## Testing

`src/palette.rs` unit tests:

- `sett` ranks Settings above Close tabs to right.
- A word-start match outranks a mid-word match.
- Empty query returns the full catalog in order.
- Selection clamps after a filter narrows the list.
- Shortcut strings match real bindings, and profiles past 5 and menu-only
  commands are empty.
- Context omits move-left at the first tab, move-right and close-to-right at the
  last tab, and next/previous when one tab remains.

`src/page.rs` tests:

- Palette text contains the query, the highlighted row, the right-aligned
  shortcut, and the footer.
- Empty result text contains `No matching commands`.
- A refill leaves no scrollback history.
- The Settings and About pages still render unchanged.

`src/app.rs` tests:

- `Ctrl+Shift+P` is recognised and does not reach the PTY.

Dispatch against the captured tab and palette-tab reuse are covered by manual
verification: `App` needs a live winit `EventLoopProxy`, and this repo's app
tests never construct one. The dispatch code stays a thin wrapper over
`run_tab_command`, whose targeting is already unit-tested through `TabSet`.

## Manual verification

1. `cargo run`, open a PowerShell tab.
2. Press `Ctrl+Shift+P`; type `sett`; Enter opens Settings.
3. Reopen; type `dup`; Enter duplicates the origin tab.
4. Reopen; run Close tab; the origin closes, the neighbour stays.
5. Reopen from the `⌄` menu row; it shows `Ctrl+Shift+P` and behaves the same.
6. Open the palette twice with profiles open; only one palette tab exists.
7. Esc closes without running anything; resize keeps the list and highlight in
   view.
8. Open the palette while a tab menu is open; the menu closes first, and the
   palette opens.

## Docs

- `README.md`: new Command palette section, Tabs bullet updated to say the
  palette opens, Known limitations drops search from "later phases".
- `AGENTS.md`: Current scope gains the palette and moves tabs, profiles, and
  search out of "Not yet".

## Files touched

- `src/palette.rs` (new)
- `src/page.rs`
- `src/app.rs`
- `src/ui/menu.rs` (shortcut hint on the Command palette row)
- `README.md`, `AGENTS.md`
- this spec and a plan under `docs/superpowers/`
