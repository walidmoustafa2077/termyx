# Termyx — MVP Design

**Status:** Approved
**Date:** 2026-11-07
**Scope:** Phases 0–3 (MVP)
**Platforms:** Windows 10 1809+, Linux, macOS

> **Batch 1 errata (2026-11-08):** Application mouse reporting, extended keys,
> cursor shape/visibility, focus reporting, window title (OSC 0/2), and OSC 52
> copy are now implemented; see README "Terminal protocol support". Any later
> statement in this document that calls them out of scope is superseded.

---

## 1. Summary

Termyx is a fast, lightweight, standalone terminal emulator with first-class RTL support. The MVP is a **single window with a single shell session** that renders Arabic, Persian, and Urdu correctly: joined glyphs, UAX #9 visual ordering, and a caret, selection, and clipboard that operate on logical text.

RTL is not a display flag. The grid stays in logical (write) order at all times; a per-row `VisualMap` converts logical cells to visual cells at render time, and every interaction path (cursor, hit-testing, selection, copy) goes through that map. Phase 1 ships the same architecture with an identity map, so Phase 2 replaces an algorithm rather than a codebase.

Everything beyond the MVP — tabs, splits, profiles, search, command palette, and the eventual AI layer — is additive on top of the interfaces defined here.

---

## 2. Goals

1. A daily-usable terminal for PowerShell, bash, and zsh on Windows, Linux, and macOS.
2. Correct RTL: UAX #9 bidi ordering, Arabic/Persian/Urdu contextual shaping, lam-alef ligatures, and correct handling of combining marks.
3. Correct interaction in mixed-direction text: caret placement, mouse selection, copy, paste, and typing.
4. Fast and small: native binary, no runtime, event-driven rendering, low memory.
5. An architecture that supports tabs, splits, and an AI layer later without rewriting the core.

## 3. Non-goals (MVP)

- Tabs, splits, panes, profiles, session restore.
- Command palette, in-terminal search, hot config reload.
- AI features. Only a no-op output tap point is reserved.
- Sixel/kitty/iTerm image protocols.
- Programming ligatures (`=>`, `!=`).
- Full IME preedit UI. Committed IME text is supported; a preedit overlay is Phase 3+.
- Application mouse reporting (SGR mouse). Local selection and wheel scrolling only in MVP; mouse forwarding is Phase 3.
- Paragraph-level bidi across wrapped rows. Bidi is per row (see §7.5 and §12).

---

## 4. Platform requirements

| Platform | Requirement |
|---|---|
| Windows | Windows 10 1809+ (ConPTY). MSVC build tools for Rust. |
| Linux | Any modern distro with a UTF-8 locale. `forkpty` via `portable-pty`. |
| macOS | 11+. `forkpty` via `portable-pty`. |
| Toolchain | Rust stable via `rustup`; `cargo`; `rustfmt`; `clippy`. |

Windows Subsystem for Linux is not required for the MVP.

---

## 5. Architecture

### 5.1 Module map

```
src/
  main.rs          bootstrap, CLI args, winit event loop, fatal-error dialog
  app.rs           window state, redraw scheduling, user events, session wiring
  config.rs        TOML load, validation, defaults, platform config paths
  pty.rs           portable-pty spawn, reader thread, resize, child lifecycle
  term.rs          alacritty_terminal wrapper: feed, grid snapshot, damage
  event.rs         SessionEvent / OutputTap (AI extension point)
  visual.rs        VisualMap: logical <-> visual cell mapping
  bidi.rs          UAX #9 per-row reordering (unicode-bidi)
  shape.rs         run shaping (rustybuzz), cluster -> cell mapping
  fonts.rs         fontdb loading, per-grapheme font selection, face cache
  input.rs         key encoding, paste, selection state, mouse hit-testing
  theme.rs         ANSI palette + UI colors
  render/
    mod.rs         frame composition, run drawing, cursor, selection
    glyph_cache.rs glyph key -> atlas slot
    atlas.rs       R8 mask atlas with shelf allocation
```

### 5.2 Data flow

```
PTY reader thread ──bytes──> mpsc ──> EventLoopProxy<UserEvent> ──> Terminal::feed()
                                                                        │
                                                                  damage rows
                                                                        ▼
                                          snapshot (logical) ──> VisualMap ──> bidi ──> shape
                                                                        │
                                                                        ▼
                                                     tiny-skia Pixmap ──> softbuffer ──> window

keyboard: winit KeyEvent ──> input.rs encode ──> PTY writer
mouse:    winit CursorMoved/MouseInput ──> visual -> logical ──> selection / scrollback
```

Threading model:

- The **UI thread** owns the window, renderer, `Terminal`, and all interaction state.
- One **PTY reader thread** per session reads bytes and forwards them through a channel.
- The UI thread drains the channel once per frame, coalescing output (bounded batch, e.g. 256 KB per frame) so a flooding process cannot starve input or painting.
- `SessionEvent::Output` is emitted from `term.rs` to an optional `OutputTap`. The MVP tap is a no-op; Phase 5 attaches an AI consumer without touching the PTY path.

### 5.3 Core interfaces

```rust
// term.rs
pub struct Terminal { /* owns alacritty_terminal::term::Term<EventProxy> */ }

impl Terminal {
    pub fn feed(&mut self, bytes: &[u8]);
    pub fn resize(&mut self, cols: usize, rows: usize, cell_w: u16, cell_h: u16);
    pub fn snapshot(&self) -> GridSnapshot;
    pub fn take_damage(&mut self) -> Damage;   // dirty rows + cursor rows
    pub fn mode(&self) -> TermMode;
}

// Snapshot is in LOGICAL order.
pub struct GridSnapshot {
    pub cols: usize,
    pub rows: usize,
    pub lines: Vec<RowSnapshot>,
    pub cursor: CursorState,
    pub display_offset: usize,
}

pub struct RowSnapshot {
    pub cells: Vec<Cell>,   // len == cols; wide cells occupy two entries
    pub wrapped: bool,
}

pub struct Cell {
    pub text: String,       // grapheme cluster, including combining marks
    pub width: u8,          // 0 = wide-char continuation, 1, or 2
    pub style: Style,
}

// visual.rs
pub struct VisualMap {
    pub cols: usize,
    pub levels: Vec<u8>,                 // bidi embedding level per logical cell
    logical_to_visual: Vec<usize>,
    visual_to_logical: Vec<usize>,
}

impl VisualMap {
    pub fn identity(cols: usize) -> Self;                       // Phase 1
    pub fn from_bidi(row: &[Cell], base: BaseDirection) -> Self; // Phase 2
    pub fn visual_of(&self, logical: usize) -> usize;
    pub fn logical_of(&self, visual: usize) -> usize;
    pub fn runs(&self) -> Vec<VisualRun>;                       // contiguous visual runs
}

// input.rs
pub enum Affinity { Before, After }

pub struct CursorState {
    pub row: usize,              // visible row, 0..rows
    pub logical: usize,          // logical cell boundary, 0..=cols
    pub affinity: Affinity,
}

pub struct Selection {
    pub anchor: LogicalPos,      // stored in LOGICAL coordinates
    pub focus: LogicalPos,
    pub mode: SelectionMode,     // Simple | Block
}
```

`VisualMap` is constructed for a row only when that row is redrawn. It is cached per dirty row for the frame; it is not stored in the grid.

---

## 6. Terminal core

### 6.1 PTY and shell startup

`pty.rs` uses `portable-pty`:

- **Windows:** ConPTY (`PtyPair`). Minimum OS is Windows 10 1809.
- **Linux/macOS:** `forkpty`.

Environment set for the child:

```
TERM=xterm-256color
COLORTERM=truecolor
TERM_PROGRAM=termyx
```

Default shell:

- Windows: `pwsh.exe` if found, else `powershell.exe`.
- Linux/macOS: `$SHELL`, else `/bin/sh`.

**Windows console initialization (required):**

1. Enable `ENABLE_VIRTUAL_TERMINAL_PROCESSING` and `DISABLE_NEWLINE_AUTO_RETURN` on the child console so VT sequences and line-feed semantics are honored.
2. Put the child console in UTF-8 mode (code page 65001):
   - PowerShell: launch with `-NoLogo -NoExit -Command "[Console]::OutputEncoding=[Text.Encoding]::UTF8; [Console]::InputEncoding=[Text.Encoding]::UTF8"`.
   - cmd: launch with `/K chcp 65001>nul`.
   - Other programs: the session contract is UTF-8; programs that ignore it are out of scope for MVP.
3. `pty.rs` performs these steps; they are not the user's responsibility.

**Byte fidelity rule:** PTY output is fed to the VT parser **verbatim**. It is never pre-reversed, reordered, or transformed. ConPTY's legacy edge-wrap heuristics depend on clean VT input; transforming bytes before ConPTY causes spurious corrective wraps. Likewise, keyboard input is sent in logical order with no RTL transformation.

Resize calls `MasterPty::resize(PtySize)` with the new rows/cols. Reflow is handled by `alacritty_terminal::Term::resize`.

### 6.2 VT parsing and grid

`term.rs` wraps `alacritty_terminal`:

- `vte`-based parser, full xterm/VT state machine.
- Grid, alternate screen, scrollback, wide cells, combining marks, DEC modes.
- `TermMode` is exposed to `input.rs` for cursor-key mode, bracketed paste, and application keypad.

`Terminal::feed` passes bytes through the parser, then marks damage. Damage comes from the terminal's damage API when available; if the pinned version does not expose it, `term.rs` marks all rows dirty on feed (contained fallback). Cursor-only changes mark the old and new cursor rows.

### 6.3 Scrollback and reflow

- Default scrollback: 10,000 lines (configurable).
- `Term::resize` reflows soft-wrapped lines. Reflow correctness is an acceptance test.
- Wheel scrolling moves `display_offset`; new output resets the offset to the live view unless the user is scrolled back.
- MVP does not implement a scrollbar UI; wheel and keyboard scrolling only.

---

## 7. RTL rendering pipeline

### 7.1 Cell model and width rules

The grid is authoritative and fixed-pitch. Column pitch is `W` pixels.

- **One grapheme cluster = one cell** unless it is a wide East Asian character (2 cells).
- Combining marks (harakat, etc.) are zero-width: they are stored in `Cell::text` with the base and do not consume a cell.
- Bidi control characters (LRM, RLM, LRI/RLI/FSI/PDI) are preserved in the row string passed to `BidiInfo` but render as zero width and do not consume a cell.
- **Ligatures span their constituent cells.** A lam-alef cluster (`لا`, two codepoints) occupies **two cells** and is drawn as one glyph across `2 * W`. It does not shrink the line or shift following cells.
- **Grid alignment is mandatory.** Each shaped cluster is positioned at its cell-slot origin, not by accumulating natural pen advance:
  - If the cluster's natural advance is within 5% of its slot width, draw it centered in the slot.
  - If it is wider than the slot, scale horizontally to fit the slot.
  - If it is narrower, center it in the slot.
  This keeps every following cell aligned even when the fallback font is proportional.
- **Font contract:** the preferred Arabic face is monospace and cell-compatible (Kawkab Mono). The scaling rule above is a safety net, not the primary path.
- A caret or selection boundary may fall inside a two-cell ligature. The caret is drawn at the cell boundary; the ligature is still drawn once across both cells. This matches other cell terminals and is documented as a limitation.

### 7.2 Bidi reordering

Per row, when the row is dirty:

1. Flatten `RowSnapshot.cells` into a UTF-8 string and a `byte offset -> logical cell index` map. Include bidi controls and combining marks.
2. Build `unicode_bidi::BidiInfo` with the configured base direction.
3. Read the resolved level per logical cell.
4. Call `visual_runs` to obtain visual runs; build `logical_to_visual` and `visual_to_logical`.
5. Cache the `VisualMap` for the row for this frame.

`unicode-bidi` is already conformance-tested against UAX #9. Our tests cover the **mapping**, not the algorithm: round-trip identity, run boundaries, and cursor/selection behavior.

### 7.3 Shaping

Each **visual run** is shaped independently with `rustybuzz`:

- Run direction comes from the bidi level (even = LTR, odd = RTL).
- Style changes (SGR colors, bold) **do not split a run**. A colored prompt inside an Arabic word must not break joining. Colors are applied per glyph cluster after shaping.
- `GlyphInfo::cluster` (byte offset) is mapped to a logical cell via the byte map, then to a visual slot.
- Cluster groups are formed per cell slot; a multi-cell cluster (ligature) is drawn across its full slot span.
- Joining forms and the lam-alef ligature are produced by the shaper; we never assemble presentation forms manually.
- Shaping results are cached per `(text, direction, font_id, size)` for the frame. A persistent shaped-run cache is a Phase 3 optimization.

Known limitation: Arabic joining is not preserved across a bidi direction boundary (for example, a Latin number embedded inside an Arabic word). This matches the behavior of other cell terminals and is acceptable for MVP.

### 7.4 Font selection and fallback

`fonts.rs` loads system fonts with `fontdb` and resolves a face per grapheme:

1. If the grapheme contains an Arabic-block codepoint, use `font.arabic_family` (default: bundled Kawkab Mono).
2. Otherwise use `font.family` (default: Cascadia Mono on Windows, Menlo on macOS, DejaVu Sans Mono on Linux).
3. If the face lacks a glyph, walk a fallback chain (system monospace faces) and use the first face with coverage, checked via `ttf-parser::Face::glyph_index`.
4. If nothing covers it, draw the replacement glyph (U+FFFD).

Regular, bold, italic, and bold-italic faces are loaded from the same family when present. Missing faces are synthesized (embolden / shear).

Bundled fonts live in `assets/fonts/` with their license files. Kawkab Mono is OFL 1.1; the license is included in the repository.

### 7.5 Base direction and prompt pinning

Each terminal row is an independent bidi paragraph.

| `bidi.base_direction` | Behavior |
|---|---|
| `ltr` (default) | Paragraph level 0. RTL text becomes embedded level-1 runs. **The prompt stays pinned to the left margin.** |
| `rtl` | Paragraph level 1. The whole row is right-anchored; prompts move to the right. |
| `auto` | UAX #9 P2/P3 per row. A row whose first strong character is RTL right-anchors. Documented as surprising for prompts. |

The default is **LTR-anchored with embedded RTL runs**. This is the setting that keeps `PS C:\Users\mrlok> ` and `$ ` on the left while typed Arabic flows naturally after them. `auto` is opt-in and explicitly documented as right-pinning.

Bidi is per row. A wrapped RTL paragraph is therefore ordered row by row; paragraph-level bidi across a wrap is impossible in a cell terminal without application cooperation. This is a documented limitation shared by other terminals.

### 7.6 Cursor

The cursor is stored as a **logical boundary plus affinity**:

```rust
CursorState { logical: usize, affinity: Affinity }
```

- `logical` is a boundary in `0..=cols`, in logical order.
- The renderer resolves it to a visual position through `VisualMap`.
- At a bidi run boundary, one logical boundary can map to two visual gaps. `Affinity::Before` selects the gap before the adjacent run in visual order; `Affinity::After` selects the gap after it. Without affinity the caret jumps sides when English and Arabic meet.
- The shell positions the cursor in logical columns; the terminal never mirrors cursor reports.
- Cursor shape (block/bar/underline) and blink come from DECSCUSR; default is a block that follows `TermMode`.
- Cursor blink redraws only the old and new cursor rows.

### 7.7 Selection and copy

- Selection is stored in **logical** coordinates (`anchor`, `focus`) regardless of visual direction.
- Mouse hit-testing converts `(x, y)` to a visual cell, then to a logical cell via `VisualMap`.
- Dragging updates the logical focus. A selection spanning multiple rows stores logical start/end.
- Copy walks the selection in **logical order** and emits the original text. Lines are joined with `\n`; trailing spaces are trimmed per line unless `selection.trim_trailing = false`.
- Block selection is a rectangular **visual** selection; it is converted to logical cells per row for copying. MVP supports simple selection; block selection is Phase 3.
- Double-click selects a word; triple-click selects a line. Word boundaries use `unicode-segmentation`.

### 7.8 Input

- Key events are encoded to xterm sequences in `input.rs` and written to the PTY in logical order.
- Modifiers: Shift, Ctrl, Alt (Alt sends ESC prefix), Super is reserved for app shortcuts.
- Application cursor keys, application keypad, and bracketed paste are honored via `TermMode`.
- Paste: if bracketed paste mode is active, wrap with `\x1b[200~` / `\x1b[201~`. Paste content is otherwise sent verbatim.
- IME: committed text is sent to the PTY; a preedit overlay is not drawn in MVP.
- Shortcuts: copy `Ctrl+Shift+C`, paste `Ctrl+Shift+V`, new window later. On macOS, `Cmd+C` / `Cmd+V`.

---

## 8. Renderer

### 8.1 Backend

- `winit` 0.30 for windowing and input.
- `softbuffer` for the window surface (CPU presentation).
- `tiny-skia` for 2D drawing into a `Pixmap` that is copied into the softbuffer.
- No GPU dependency in MVP. `render::Backend` is kept narrow so a `wgpu` backend can be added later without touching layout or shaping.

Frame composition, per row:

1. Selection and cell backgrounds (run-length merged by color).
2. Shaped glyphs from the atlas.
3. Underline/strikethrough decorations.
4. Cursor.

### 8.2 Glyph cache and atlas

- Atlas: 2048×2048 R8 mask, shelf allocator.
- Key: `(font_id, size_px, glyph_id, synthetic_bold, synthetic_italic)`.
- Value: atlas rect, bearing, advance.
- Eviction: clear the atlas on font/size/DPI change; no LRU in MVP.
- Rasterization: `ab_glyph` outline rasterization by glyph id, grayscale antialiasing (no LCD subpixel in MVP).
- Color glyphs (emoji) are out of scope for MVP; emoji render as monochrome fallback or replacement.

### 8.3 Redraw strategy

- Event-driven: redraw on PTY output, key input, mouse, resize, focus, and cursor blink.
- Damage-driven: `Terminal::take_damage` provides dirty rows; the renderer redraws only those rows plus the cursor rows.
- If damage information is unavailable, fall back to a full-frame redraw. This is a contained change in `term.rs`.
- Output is coalesced per frame so a flood of bytes produces one repaint, not one per byte.

### 8.4 Geometry

- Cell size is derived from the primary font's advance and line height, rounded to whole pixels.
- DPI changes trigger font reload, atlas clear, and a full redraw.
- Grid size = `floor((width - padding) / cell_w)`, `floor((height - padding) / cell_h)`.
- Minimum window size: 20×5 cells.

---

## 9. Configuration

Format: TOML. Loaded once at startup; `--config <path>` overrides the default location.

Default paths:

- Windows: `%APPDATA%\termyx\config.toml`
- Linux: `$XDG_CONFIG_HOME/termyx/config.toml` (fallback `~/.config/...`)
- macOS: `~/Library/Application Support/termyx/config.toml`

Schema:

```toml
[font]
family = "Cascadia Mono"
arabic_family = "Kawkab Mono"
size = 14.0
line_height = 1.2

[bidi]
enabled = true
base_direction = "ltr"   # ltr | rtl | auto

[window]
width = 1200
height = 800
padding = 4

[scrollback]
lines = 10000

[selection]
trim_trailing = true

[theme]
background = "#0c0c0c"
foreground = "#cccccc"
cursor = "#ffffff"
selection = "#264f78"
# black, red, green, yellow, blue, magenta, cyan, white,
# bright_black, ... bright_white

[shell]
program = ""   # empty = platform default
args = []
```

Validation:

- Unknown keys are ignored with a warning.
- Invalid enum values fall back to the default and log a warning.
- Font size is clamped to `6.0..=48.0`; scrollback to `1000..=200000`.

---

## 10. Error handling and logging

- `thiserror` for module-level errors; `anyhow` at the binary boundary.
- Fatal startup errors (PTY spawn failure, renderer init failure, no font available) show a native message dialog via `rfd` and exit non-zero.
- Non-fatal errors (config parse failure, missing configured font) log a warning, use defaults, and show a one-line notice in the window for five seconds.
- Logging: `tracing` with a file appender in the platform data directory, plus stderr when a console is attached. Log level from `TERMYX_LOG`.
- Panics are caught at the top level, logged, and surfaced as a dialog.

---

## 11. Performance targets

| Metric | Target |
|---|---|
| Cold start (release) | < 300 ms |
| Input-to-paint latency | < 16 ms |
| Full-frame redraw at 200×50 | < 8 ms on a mid-range CPU |
| Resident memory, 10k scrollback | < 80 MB |
| Release binary (stripped, LTO) | < 15 MB |
| Typing throughput | No dropped keystrokes under `yes`-style output floods |

Release profile: `lto = "thin"`, `codegen-units = 1`, `strip = true`, `panic = "abort"` after top-level panic handling is verified.

---

## 12. Testing

### Unit tests

- `visual.rs`: identity map; round-trip `logical -> visual -> logical`; run boundaries; RTL/LTR/mixed rows.
- `bidi.rs`: base direction `ltr`/`rtl`/`auto`; digits and punctuation in RTL runs; bidi control characters.
- `shape.rs`: cluster-to-cell mapping; lam-alef spans two cells; combining marks stay in one cell; style changes do not split a run.
- `input.rs`: cursor affinity at run boundaries; visual-to-logical hit-testing; selection-to-text in logical order; key encoding tables.
- `config.rs`: defaults, invalid values, unknown keys.

### Golden-image tests

- A headless harness renders fixed grids (mixed LTR/RTL, ligatures, selection, cursor positions) to PNG.
- Snapshots live in `tests/golden/`; comparison uses a pixel-diff threshold (< 0.1%).
- `insta` for snapshot management; `image` for diffing.

### Integration tests

- Spawn `cmd.exe`, `powershell.exe`, `/bin/sh` in a PTY; send commands; assert grid text through `Terminal::snapshot`.
- Resize tests: shrink and grow with wrapped lines; assert reflow preserves text.
- UTF-8 test on Windows: assert `chcp` reports 65001 and that Arabic `echo` output arrives intact.

### Manual acceptance matrix

- Compare side by side with Windows Terminal / iTerm2 / GNOME Terminal using the same commands.
- RTL cases: prompt + Arabic, pure Arabic line, mixed English/Arabic, Arabic with numbers, lam-alef, harakat, selection across direction changes, resize while scrolled back.

---

## 13. Milestones and acceptance criteria

### M0 — Setup

- `rustup` toolchain installed, `git init`, Cargo project, CI skeleton (Windows + Linux).
- Bundled Kawkab Mono with OFL license.

### M1 — LTR terminal (Phase 1)

- PTY spawn, VT parsing, grid snapshot, plain renderer, keyboard input, scrollback.
- `VisualMap::identity` wired through cursor and hit-testing from the start.
- Acceptance: run `ls`/`dir`, `vim`/`nano` opens and exits cleanly, colors render, resize does not corrupt the grid.

### M2 — RTL rendering (Phase 2)

- `BidiInfo` reordering, rustybuzz shaping, font fallback, cluster-to-cell mapping.
- Acceptance: Arabic/Persian/Urdu `echo` renders joined and in correct visual order; lam-alef occupies two cells and following text stays aligned; prompt stays left in default mode.

### M3 — Interaction correctness

- Cursor affinity, visual-to-logical hit-testing, selection, copy, paste, bracketed paste.
- Acceptance: caret lands correctly at mixed-direction boundaries; mouse selection copies logical text; typing mid-line inserts at the logical position.

### M4 — Polish (Phase 3)

- Config, themes, DPI, cursor blink, wheel scrolling, basic IME commit, error dialogs, logging.
- Acceptance: config changes take effect on restart; high-DPI is crisp; no panics on malformed config.

### M5 — MVP acceptance

1. PowerShell, bash, and zsh render correctly.
2. Arabic, Persian, and Urdu text is joined and correctly ordered.
3. Caret and typing are correct in mixed-direction text.
4. Selection and copy return logical-order text.
5. Resize reflows scrollback without corruption.
6. Performance targets in §11 are met.
7. On Windows, `chcp` reports 65001 and UTF-8 output is intact.
8. Lam-alef spans two cells without shifting subsequent text.

---

## 14. Future phases (not in MVP)

- **Phase 4 — Terminal UI:** tabs, splits, profiles, command palette, in-terminal search, theme hot reload, mouse reporting to applications, block selection, scrollbar. Tabs and the profile menu (the tab strip, `+`/`⌄`, `Ctrl+Tab`, `Ctrl+Shift+T`/`W`, `Ctrl+Shift+1..5`, `[[profiles]]`, and the menu's settings/about rows) are delivered. The custom window chrome (the strip as the title bar, painted minimise/maximise/close, and drag from the empty strip) is implemented on Windows; other platforms keep native decorations. The DPI rebuild is implemented; the real 150 % check is outstanding.
- **Phase 5 — AI layer:** consume `SessionEvent` output, add OSC 133 shell integration for command boundaries, and surface an AI assistant or AI shell. The exact product shape is an open question (§15) and does not affect the MVP.

---

## 15. Open question (post-MVP)

"AI CLI" can mean either:

1. an AI assistant **inside** the terminal (explain output, suggest or draft commands), or
2. an AI **shell** that replaces the prompt and drives execution.

The MVP reserves `SessionEvent` and an `OutputTap` so either can attach later. The decision is deferred until Phase 5 design.

---

## 16. Dependencies

Pin exact versions in `Cargo.lock` during M0.

| Crate | Purpose |
|---|---|
| `winit` | Window, input, DPI, IME events |
| `softbuffer` | CPU window surface |
| `tiny-skia` | 2D rasterization |
| `alacritty_terminal` | VT parser, grid, scrollback, reflow, `TermMode` |
| `portable-pty` | ConPTY / forkpty |
| `rustybuzz` | Arabic shaping and cluster mapping |
| `unicode-bidi` | UAX #9 reordering |
| `unicode-width` | Cell width decisions |
| `unicode-segmentation` | Grapheme clusters and word boundaries |
| `fontdb` | System font discovery |
| `ttf-parser` | Glyph coverage checks |
| `ab_glyph` | Glyph rasterization by glyph id |
| `serde`, `toml` | Configuration |
| `tracing`, `tracing-subscriber` | Logging |
| `thiserror`, `anyhow` | Error handling |
| `rfd` | Native error dialogs |
| `insta`, `image` (dev) | Golden-image tests |

---

## 17. Risks and mitigations

| Risk | Mitigation |
|---|---|
| `alacritty_terminal` API churn between versions | Pin the version; wrap it entirely in `term.rs`; snapshot/damage interfaces are ours. |
| Arabic monospace fonts are scarce | Bundle Kawkab Mono (OFL); configurable `arabic_family`; grid-fit scaling as a fallback. |
| Style changes break Arabic joining | Shape per bidi run, never per cell; apply colors per glyph cluster. |
| ConPTY reflow and wrap quirks | Never transform PTY bytes; resize integration tests; Windows-specific acceptance test. |
| Bidi control characters consume cells | Preserve them in the bidi string; render zero width; test LRM/RLM/isolates. |
| Caret ambiguity at run boundaries | Explicit `Affinity`; unit tests for English/Arabic boundaries. |
| Shaping cost on large repaints | Per-frame run cache; glyph atlas; damage-driven redraw; persistent cache in Phase 3. |
| Emoji and complex graphemes | Out of scope for MVP; monochrome fallback or replacement glyph; revisit later. |
| IME on Windows | Commit-only support in MVP; preedit overlay deferred. |
| Paragraph bidi across wraps | Documented per-row limitation; no app cooperation assumed. |
