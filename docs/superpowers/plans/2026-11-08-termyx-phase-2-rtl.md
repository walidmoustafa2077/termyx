# Termyx Phase 2 (RTL) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add per-row UAX #9 bidi reordering, Arabic/Persian/Urdu shaping, Arabic-capable font fallback, and correct cursor/selection/hit-testing through the existing `VisualMap` abstraction, without changing the logical grid.

**Architecture:** `RowSnapshot` stays in logical (write) order. `bidi::analyze` flattens one row to UTF-8, resolves levels with `unicode_bidi::BidiInfo`, and produces visual runs. `VisualMap` stores those levels/runs plus the logical↔visual cell maps. `shape::shape_row` shapes each visual run with `rustybuzz`, groups glyphs into clusters, and maps cluster byte offsets back to logical cells and visual slots. The renderer positions each cluster at its cell-slot origin and draws with `ab_glyph`; the cursor, hit-testing, selection, and copy all continue to use logical coordinates resolved through `VisualMap`.

**Tech Stack:** Rust stable, `unicode-bidi` 0.3, `rustybuzz` 0.20, `ttf-parser` 0.25, existing `fontdb` 0.23, `ab_glyph` 0.2, `tiny-skia`, `winit`, `alacritty_terminal`.

**Spec:** `docs/superpowers/specs/2026-11-07-termyx-mvp-design.md` (§7 RTL pipeline, §8 renderer, §12 testing, §16 dependencies)

## Global Constraints

- Platforms: Windows 10 1809+ (ConPTY, MSVC), Linux (UTF-8 locale, forkpty), macOS 11+.
- Rust stable via rustup. Every task ends with `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean.
- The grid is **logical**. Never reorder cells in `RowSnapshot`. Every visual lookup goes through `VisualMap`.
- One logical codepoint is one cell; wide CJK is 2 cells; `Cell.width == 0` marks a wide-character continuation.
- Bidi control characters (LRM, RLM, ALM, LRE..RLO, PDF, LRI..PDI) stay in the flattened string, occupy their grid cell's visual slot, and paint no glyph.
- A lam-alef cluster occupies two cells and is drawn once across `2 * cell_w`. Following cells never shift.
- Default base direction is `ltr`; the prompt stays pinned to the left margin.
- Bidi is per row. Paragraph-level bidi across soft-wrapped rows is a documented limitation and is not implemented.
- `Cargo.lock` is committed and updated by each dependency change.
- No font binary is added or committed by this plan. `assets/fonts/` receives a README only; the configured `font.arabic_family` ("Kawkab Mono") may be absent, in which case the system fallback path is the shipping path until a separate font-asset task lands.
- Phase 2 adds **only** RTL. No tabs, splits, profiles, command palette, search, AI, images, application mouse reporting, IME preedit, block selection, or persistent shaping cache.
- If `git commit` fails with `Author identity unknown`, STOP and ask the user for `user.name` and `user.email`. Do not invent an identity.

## File Structure

| File | Responsibility |
|---|---|
| `src/bidi.rs` | Flatten a row to UTF-8, run UAX #9, expose per-cell levels and visual runs |
| `src/shape.rs` | Shape visual runs with rustybuzz, group glyphs into cell-mapped clusters |
| `src/visual.rs` | `VisualMap`: levels, runs, logical↔visual maps, cursor resolution |
| `src/fonts.rs` | Primary face metrics, Arabic-capable face selection and fallback |
| `src/render/mod.rs` | Backgrounds, shaped-cluster drawing, decorations, cursor |
| `src/render/glyph_cache.rs` | Font-aware glyph key and rasterization |
| `src/app.rs` | Per-row maps in the frame, bidi-aware hit-testing |
| `src/selection.rs` | Logical selection/copy regression coverage (no behavior change) |
| `assets/fonts/README.md` | Document the expected Kawkab Mono asset and license |
| `README.md` | Phase 2 status, config, acceptance, limitations |

---

### Task 1: Bidi analysis module

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/main.rs`
- Create: `src/bidi.rs`
- Modify: `src/visual.rs`
- Test: `src/bidi.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: `crate::term::RowSnapshot`, `crate::config::BaseDirection`, `unicode_bidi::{BidiInfo, Level}`.
- Produces:
  - `RowText { text: String, cell_starts: Vec<usize>, byte_cells: Vec<usize>, widths: Vec<u8> }`
  - `RowText::flatten(&RowSnapshot) -> RowText`
  - `RowText::cell_of_byte(&self, byte: usize) -> usize`
  - `VisualRun { cells: Range<usize>, slots: Range<usize>, level: u8, rtl: bool }`
  - `BidiRow { levels: Vec<u8>, runs: Vec<VisualRun> }`
  - `bidi::analyze(&RowSnapshot, BaseDirection) -> BidiRow`
  - `bidi::is_bidi_control(char) -> bool`
  - `VisualMap::from_bidi(&RowSnapshot, BaseDirection) -> VisualMap`

- [ ] **Step 1: Add the dependency and module declaration**

In `Cargo.toml`, under `[dependencies]`, add after `unicode-width`:

```toml
unicode-bidi = "0.3"
```

In `src/main.rs`, add the module declaration in alphabetical order:

```rust
mod app;
mod bidi;
mod config;
mod event;
mod fonts;
mod input;
mod pty;
mod render;
mod selection;
mod term;
mod theme;
mod visual;
```

- [ ] **Step 2: Write the failing tests**

Append to `src/bidi.rs` (create the file with only this block for now):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BaseDirection;
    use crate::term::{Cell, RowSnapshot, Style};
    use crate::visual::VisualMap;
    use unicode_width::UnicodeWidthStr;

    fn cell(text: &str) -> Cell {
        let width = UnicodeWidthStr::width(text).clamp(1, 2) as u8;
        Cell {
            text: text.to_string(),
            width,
            style: Style::default(),
        }
    }

    fn spacer() -> Cell {
        Cell {
            text: String::new(),
            width: 0,
            style: Style::default(),
        }
    }

    fn row(texts: &[&str]) -> RowSnapshot {
        RowSnapshot {
            cells: texts.iter().map(|text| cell(text)).collect(),
            wrapped: false,
        }
    }

    #[test]
    fn flatten_records_byte_to_cell_offsets() {
        let row = row(&["a", "م"]);
        let text = RowText::flatten(&row);
        assert_eq!(text.text, "aم");
        assert_eq!(text.cell_starts, vec![0, 1, 3]);
        assert_eq!(text.byte_cells, vec![0, 1, 1]);
        assert_eq!(text.cell_of_byte(0), 0);
        assert_eq!(text.cell_of_byte(1), 1);
        assert_eq!(text.cell_of_byte(2), 1);
    }

    #[test]
    fn wide_char_continuation_shares_the_base_cell() {
        let row = RowSnapshot {
            cells: vec![cell("中"), spacer()],
            wrapped: false,
        };
        let text = RowText::flatten(&row);
        assert_eq!(text.text, "中");
        assert_eq!(text.cell_starts, vec![0, 3, 3]);
        assert_eq!(text.byte_cells, vec![0, 0, 0]);
        let bidi = analyze(&row, BaseDirection::Ltr);
        assert_eq!(bidi.runs.len(), 1);
        assert_eq!(bidi.runs[0].cells, 0..2);
        assert_eq!(bidi.runs[0].slots, 0..2);
    }

    #[test]
    fn ltr_row_produces_an_identity_map() {
        let row = row(&["a", "b", "c"]);
        let bidi = analyze(&row, BaseDirection::Ltr);
        assert_eq!(bidi.levels, vec![0, 0, 0]);
        let map = VisualMap::from_bidi(&row, BaseDirection::Ltr);
        assert_eq!(map.visual_of(0), 0);
        assert_eq!(map.visual_of(2), 2);
        assert_eq!(map.logical_of(2), 2);
    }

    #[test]
    fn rtl_row_reverses_visual_slots() {
        let row = row(&["م", "ر", "ح"]);
        let bidi = analyze(&row, BaseDirection::Ltr);
        assert_eq!(bidi.levels, vec![1, 1, 1]);
        let map = VisualMap::from_bidi(&row, BaseDirection::Ltr);
        assert_eq!(map.visual_of(0), 2);
        assert_eq!(map.visual_of(1), 1);
        assert_eq!(map.visual_of(2), 0);
    }

    #[test]
    fn mixed_row_keeps_latin_left_and_reverses_the_arabic_run() {
        let row = row(&["a", "b", "c", "م", "ر", "ح"]);
        let bidi = analyze(&row, BaseDirection::Ltr);
        assert_eq!(bidi.levels, vec![0, 0, 0, 1, 1, 1]);
        let map = VisualMap::from_bidi(&row, BaseDirection::Ltr);
        assert_eq!(map.visual_of(0), 0);
        assert_eq!(map.visual_of(3), 5);
        assert_eq!(map.visual_of(4), 4);
        assert_eq!(map.visual_of(5), 3);
        assert_eq!(map.logical_of(5), 3);
    }

    #[test]
    fn bidi_controls_stay_in_the_text_and_consume_one_slot() {
        let row = row(&["\u{200F}", "a"]);
        let text = RowText::flatten(&row);
        assert_eq!(text.text, "\u{200F}a");
        assert_eq!(text.cell_starts, vec![0, 3, 4]);
        let map = VisualMap::from_bidi(&row, BaseDirection::Ltr);
        assert_eq!(map.visual_of(0), 0);
        assert_eq!(map.visual_of(1), 1);
    }

    #[test]
    fn mapping_round_trips_every_cell() {
        let row = row(&["a", "b", " ", "م", "ر", "ح", " ", "1"]);
        let map = VisualMap::from_bidi(&row, BaseDirection::Ltr);
        for logical in 0..row.cells.len() {
            assert_eq!(map.logical_of(map.visual_of(logical)), logical);
            assert_eq!(map.visual_of(map.logical_of(logical)), logical);
        }
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test bidi::tests -v`

Expected: FAIL to compile with `cannot find type RowText`, `cannot find function analyze`, and `no method named from_bidi`.

- [ ] **Step 4: Implement `src/bidi.rs`**

Prepend this implementation above the test module:

```rust
//! Per-row UAX #9 analysis: flatten cells to UTF-8 and map visual runs.
//!
// Wired into the frame in Task 3; the tests below are the only callers until
// then.
#![allow(dead_code)]

use std::ops::Range;

use unicode_bidi::{BidiInfo, Level};

use crate::config::BaseDirection;
use crate::term::RowSnapshot;

/// A row flattened to UTF-8 with byte-to-cell and cell-to-byte maps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowText {
    pub text: String,
    /// Byte offset of each cell's first byte; `len == cells + 1`.
    pub cell_starts: Vec<usize>,
    /// Cell index for every byte in `text`; `len == text.len()`.
    pub byte_cells: Vec<usize>,
    /// Grid width of each cell (0 = wide-character continuation).
    pub widths: Vec<u8>,
}

impl RowText {
    pub fn flatten(row: &RowSnapshot) -> Self {
        let mut text = String::new();
        let mut cell_starts = Vec::with_capacity(row.cells.len() + 1);
        let mut widths = Vec::with_capacity(row.cells.len());
        let mut byte_cells = Vec::new();
        for (index, cell) in row.cells.iter().enumerate() {
            cell_starts.push(text.len());
            widths.push(cell.width);
            if cell.width == 0 {
                continue;
            }
            for _ in 0..cell.text.len() {
                byte_cells.push(index);
            }
            text.push_str(&cell.text);
        }
        cell_starts.push(text.len());
        Self {
            text,
            cell_starts,
            byte_cells,
            widths,
        }
    }

    /// Cell containing `byte`. Continuation cells own no bytes, so this never
    /// returns a width-0 cell.
    pub fn cell_of_byte(&self, byte: usize) -> usize {
        if self.widths.is_empty() {
            return 0;
        }
        self.byte_cells
            .get(byte)
            .copied()
            .unwrap_or_else(|| self.widths.len() - 1)
    }
}

/// A contiguous run of cells with one bidi level, in visual order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualRun {
    pub cells: Range<usize>,
    pub slots: Range<usize>,
    pub level: u8,
    pub rtl: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BidiRow {
    pub levels: Vec<u8>,
    pub runs: Vec<VisualRun>,
}

/// Resolves UAX #9 levels for one row and returns its runs in visual order.
pub fn analyze(row: &RowSnapshot, base: BaseDirection) -> BidiRow {
    let text = RowText::flatten(row);
    let cols = row.cells.len();
    if cols == 0 || text.text.is_empty() {
        let runs = if cols == 0 {
            Vec::new()
        } else {
            vec![VisualRun {
                cells: 0..cols,
                slots: 0..cols,
                level: 0,
                rtl: false,
            }]
        };
        return BidiRow {
            levels: vec![0; cols],
            runs,
        };
    }

    let base_level = match base {
        BaseDirection::Ltr => Some(Level::ltr()),
        BaseDirection::Rtl => Some(Level::rtl()),
        BaseDirection::Auto => None,
    };
    let info = BidiInfo::new(&text.text, base_level);
    let Some(paragraph) = info.paragraphs.first() else {
        return BidiRow {
            levels: vec![0; cols],
            runs: Vec::new(),
        };
    };
    let (byte_levels, byte_runs) = info.visual_runs(paragraph, 0..text.text.len());

    let mut levels = vec![0u8; cols];
    for cell in 0..cols {
        levels[cell] = if row.cells[cell].width == 0 && cell > 0 {
            levels[cell - 1]
        } else {
            let byte = text.cell_starts[cell].min(text.text.len().saturating_sub(1));
            byte_levels
                .get(byte)
                .map(|level| level.number())
                .unwrap_or(0)
        };
    }

    let mut runs = Vec::with_capacity(byte_runs.len());
    let mut slot = 0usize;
    for byte_run in byte_runs {
        if byte_run.start >= byte_run.end || slot >= cols {
            continue;
        }
        let first = text.cell_of_byte(byte_run.start);
        let mut last = text.cell_of_byte(byte_run.end - 1);
        last += usize::from(row.cells[last].width.max(1) - 1);
        let last = last.min(cols - 1);
        let span = last.saturating_sub(first) + 1;
        let slot_end = (slot + span).min(cols);
        let level = levels[first];
        runs.push(VisualRun {
            cells: first..last + 1,
            slots: slot..slot_end,
            level,
            rtl: level % 2 == 1,
        });
        slot = slot_end;
    }

    BidiRow { levels, runs }
}

// Consumed by the renderer's invisible-cell check in Task 6.
#[allow(dead_code)]
pub fn is_bidi_control(ch: char) -> bool {
    matches!(
        ch,
        '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}
```

- [ ] **Step 5: Add `VisualMap::from_bidi`**

In `src/visual.rs`, add `use crate::config::BaseDirection;` and `use crate::term::{Affinity, CursorState, RowSnapshot};` (keep the existing `Affinity`/`CursorState` imports), then add this method inside `impl VisualMap` after `identity`:

```rust
    // Wired into the frame in Task 3; tests are the only callers until then.
    #[allow(dead_code)]
    pub fn from_bidi(row: &RowSnapshot, base: BaseDirection) -> Self {
        let analysis = crate::bidi::analyze(row, base);
        let cols = analysis.levels.len();
        let mut logical_to_visual = vec![0; cols];
        let mut visual_to_logical = vec![0; cols];
        for run in &analysis.runs {
            for offset in 0..run.cells.len() {
                let logical = run.cells.start + offset;
                let visual = if run.rtl {
                    run.slots.end - 1 - offset
                } else {
                    run.slots.start + offset
                };
                if logical < cols && visual < cols {
                    logical_to_visual[logical] = visual;
                    visual_to_logical[visual] = logical;
                }
            }
        }
        Self {
            cols,
            logical_to_visual,
            visual_to_logical,
        }
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test bidi::tests -v`

Expected: PASS — 7 tests pass.

Run: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`

Expected: clean.

- [ ] **Step 7: Commit**

```powershell
git add Cargo.toml Cargo.lock src/main.rs src/bidi.rs src/visual.rs
git commit -m "feat: add per-row bidi analysis and visual runs"
```

---

### Task 2: Store levels and runs in `VisualMap`

**Files:**
- Modify: `src/visual.rs`
- Test: `src/visual.rs` (inline)

**Interfaces:**
- Consumes: `bidi::{analyze, VisualRun}`.
- Produces:
  - `VisualMap { cols, levels: Vec<u8>, runs: Vec<VisualRun>, logical_to_visual, visual_to_logical }`
  - `VisualMap::identity(usize) -> VisualMap`
  - `VisualMap::from_bidi(&RowSnapshot, BaseDirection) -> VisualMap`
  - `VisualMap::cols() -> usize`
  - `VisualMap::levels() -> &[u8]`
  - `VisualMap::runs() -> &[VisualRun]`
  - Existing `visual_of`, `logical_of`, `cursor_visual`, `logical_at` keep their signatures and Phase 1 behavior.

- [ ] **Step 1: Write the failing tests**

Add these tests to the existing `mod tests` in `src/visual.rs` and add the needed imports:

```rust
    use crate::config::BaseDirection;
    use crate::term::{Cell, RowSnapshot, Style};
    use unicode_width::UnicodeWidthStr;

    fn cell(text: &str) -> Cell {
        let width = UnicodeWidthStr::width(text).clamp(1, 2) as u8;
        Cell {
            text: text.to_string(),
            width,
            style: Style::default(),
        }
    }

    fn row(texts: &[&str]) -> RowSnapshot {
        RowSnapshot {
            cells: texts.iter().map(|text| cell(text)).collect(),
            wrapped: false,
        }
    }

    #[test]
    fn identity_exposes_one_ltr_run_and_zero_levels() {
        let map = VisualMap::identity(4);
        assert_eq!(map.cols(), 4);
        assert_eq!(map.levels(), &[0, 0, 0, 0]);
        assert_eq!(map.runs().len(), 1);
        assert_eq!(map.runs()[0].cells, 0..4);
        assert_eq!(map.runs()[0].slots, 0..4);
        assert!(!map.runs()[0].rtl);
    }

    #[test]
    fn bidi_runs_are_exposed_in_visual_order() {
        let map = VisualMap::from_bidi(
            &row(&["a", "b", "c", "م", "ر", "ح"]),
            BaseDirection::Ltr,
        );
        assert_eq!(map.levels(), &[0, 0, 0, 1, 1, 1]);
        assert_eq!(map.runs().len(), 2);
        assert_eq!(map.runs()[0].slots, 0..3);
        assert_eq!(map.runs()[1].slots, 3..6);
        assert!(map.runs()[1].rtl);
    }

    #[test]
    fn empty_rows_have_no_runs() {
        let map = VisualMap::identity(0);
        assert_eq!(map.cols(), 0);
        assert!(map.runs().is_empty());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test visual::tests -v`

Expected: FAIL with `no method named cols`, `levels`, or `runs`.

- [ ] **Step 3: Replace `src/visual.rs` with the extended implementation**

```rust
//! Per-row logical-to-visual mapping.

use crate::bidi::{self, VisualRun};
use crate::config::BaseDirection;
use crate::term::{Affinity, CursorState, RowSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualMap {
    cols: usize,
    levels: Vec<u8>,
    runs: Vec<VisualRun>,
    logical_to_visual: Vec<usize>,
    visual_to_logical: Vec<usize>,
}

impl VisualMap {
    pub fn identity(cols: usize) -> Self {
        let runs = if cols == 0 {
            Vec::new()
        } else {
            vec![VisualRun {
                cells: 0..cols,
                slots: 0..cols,
                level: 0,
                rtl: false,
            }]
        };
        Self {
            cols,
            levels: vec![0; cols],
            runs,
            logical_to_visual: (0..cols).collect(),
            visual_to_logical: (0..cols).collect(),
        }
    }

    // Wired into the frame in Task 3; tests are the only callers until then.
    #[allow(dead_code)]
    pub fn from_bidi(row: &RowSnapshot, base: BaseDirection) -> Self {
        let analysis = bidi::analyze(row, base);
        Self::build(analysis.levels, analysis.runs)
    }

    fn build(levels: Vec<u8>, runs: Vec<VisualRun>) -> Self {
        let cols = levels.len();
        let mut logical_to_visual = vec![0; cols];
        let mut visual_to_logical = vec![0; cols];
        for run in &runs {
            for offset in 0..run.cells.len() {
                let logical = run.cells.start + offset;
                let visual = if run.rtl {
                    run.slots.end - 1 - offset
                } else {
                    run.slots.start + offset
                };
                if logical < cols && visual < cols {
                    logical_to_visual[logical] = visual;
                    visual_to_logical[visual] = logical;
                }
            }
        }
        Self {
            cols,
            levels,
            runs,
            logical_to_visual,
            visual_to_logical,
        }
    }

    // These three accessors are consumed by shaping in Task 4; until then only
    // the tests read them.
    #[allow(dead_code)]
    pub fn cols(&self) -> usize {
        self.cols
    }

    #[allow(dead_code)]
    pub fn levels(&self) -> &[u8] {
        &self.levels
    }

    #[allow(dead_code)]
    pub fn runs(&self) -> &[VisualRun] {
        &self.runs
    }

    /// Visual column of a logical cell. Clamps to the last cell.
    pub fn visual_of(&self, logical: usize) -> usize {
        self.logical_to_visual
            .get(logical)
            .copied()
            .unwrap_or_else(|| self.cols.saturating_sub(1))
    }

    /// Logical cell at a visual column. Clamps to the last cell.
    pub fn logical_of(&self, visual: usize) -> usize {
        self.visual_to_logical
            .get(visual)
            .copied()
            .unwrap_or_else(|| self.cols.saturating_sub(1))
    }

    /// Visual position of the cursor boundary (0..=cols).
    pub fn cursor_visual(&self, cursor: &CursorState) -> usize {
        match cursor.affinity {
            Affinity::Before => {
                if cursor.logical >= self.cols {
                    self.cols
                } else {
                    self.visual_of(cursor.logical)
                }
            }
            Affinity::After => {
                if cursor.logical == 0 {
                    0
                } else {
                    (self.visual_of(cursor.logical) + 1).min(self.cols)
                }
            }
        }
    }

    /// Logical cell under a visual column (mouse hit-testing).
    pub fn logical_at(&self, visual: usize) -> usize {
        self.logical_of(visual)
    }
}
```

Keep all existing tests plus the three new ones below the implementation. The existing `cursor_after_a_cell_sits_on_its_right_edge` test stays unchanged in this task.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test visual::tests -v`

Expected: PASS — 8 tests pass.

Run: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`

Expected: clean.

- [ ] **Step 5: Commit**

```powershell
git add src/visual.rs
git commit -m "feat: expose bidi levels and runs from the visual map"
```

---

### Task 3: Base direction config wiring

**Files:**
- Modify: `src/visual.rs`
- Modify: `src/app.rs`
- Test: `src/visual.rs` (inline)

**Interfaces:**
- Consumes: `crate::config::BidiConfig`, `RowSnapshot`.
- Produces: `VisualMap::for_row(&RowSnapshot, &BidiConfig) -> VisualMap`.

- [ ] **Step 1: Write the failing tests**

Add to `src/visual.rs` tests:

```rust
    use crate::config::BidiConfig;

    fn bidi(base_direction: BaseDirection) -> BidiConfig {
        BidiConfig {
            enabled: true,
            base_direction,
        }
    }

    #[test]
    fn ltr_base_keeps_the_prompt_on_the_left() {
        let line = row(&["a", "b", " ", "م", "ر"]);
        let map = VisualMap::for_row(&line, &bidi(BaseDirection::Ltr));
        assert_eq!(map.visual_of(0), 0);
        assert_eq!(map.visual_of(2), 2);
        assert_eq!(map.visual_of(3), 4);
        assert_eq!(map.visual_of(4), 3);
    }

    #[test]
    fn rtl_base_right_anchors_latin_text() {
        let line = row(&["a", "b", "c", " ", " ", " ", " ", " ", " ", " "]);
        let map = VisualMap::for_row(&line, &bidi(BaseDirection::Rtl));
        assert_eq!(map.visual_of(0), 7);
        assert_eq!(map.visual_of(2), 9);
    }

    #[test]
    fn auto_base_follows_the_first_strong_character() {
        let rtl_first = row(&["م", "ر", "ح", " ", " ", " ", " "]);
        let map = VisualMap::for_row(&rtl_first, &bidi(BaseDirection::Auto));
        assert_eq!(map.visual_of(0), 6);

        let ltr_first = row(&["a", "b", "c", " ", " ", " ", " "]);
        let map = VisualMap::for_row(&ltr_first, &bidi(BaseDirection::Auto));
        assert_eq!(map.visual_of(0), 0);
    }

    #[test]
    fn disabled_bidi_falls_back_to_identity() {
        let line = row(&["م", "ر", "ح"]);
        let map = VisualMap::for_row(
            &line,
            &BidiConfig {
                enabled: false,
                base_direction: BaseDirection::Ltr,
            },
        );
        assert_eq!(map.visual_of(0), 0);
        assert_eq!(map.visual_of(2), 2);
        assert!(map.levels().iter().all(|level| *level == 0));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test visual::tests -v`

Expected: FAIL with `no function or associated item named for_row`.

- [ ] **Step 3: Implement `for_row`**

Add to `impl VisualMap` in `src/visual.rs`:

```rust
    pub fn for_row(row: &RowSnapshot, bidi: &crate::config::BidiConfig) -> Self {
        if bidi.enabled {
            Self::from_bidi(row, bidi.base_direction)
        } else {
            Self::identity(row.cells.len())
        }
    }
```

Remove the module-level `#![allow(dead_code)]` from `src/bidi.rs` and the `#[allow(dead_code)]` attribute on `VisualMap::from_bidi` in `src/visual.rs`. `VisualMap::for_row` is now called from the frame, so `from_bidi` and everything in `bidi.rs` is reachable in the binary.

- [ ] **Step 4: Wire the configuration into the frame**

In `src/app.rs`, replace the identity map construction inside `fn render`:

```rust
        let maps = vec![VisualMap::identity(snapshot.cols); snapshot.rows];
```

with:

```rust
        let maps: Vec<VisualMap> = snapshot
            .lines
            .iter()
            .map(|line| VisualMap::for_row(line, &self.config.bidi))
            .collect();
```

- [ ] **Step 5: Run the tests to verify they pass**

Run:

```powershell
cargo test visual::tests -v
cargo test app::tests -v
```

Expected: PASS — 12 visual tests and 7 app tests pass.

- [ ] **Step 6: Manual check of each base direction**

Run: `cargo run --release`

In the shell, type `echo "مرحبا"`. With the default config the Latin prompt stays at the left margin and the Arabic text is reordered inside the row. Stop the app.

Create the config with PowerShell:

```powershell
$config = Join-Path $env:TEMP "termyx-rtl.toml"
Set-Content -Path $config -Value "[bidi]`nbase_direction = `"rtl`"`n"
```

Its contents are:

```toml
[bidi]
base_direction = "rtl"
```

Run: `cargo run --release -- --config $config`

Expected: the prompt and the row are right-anchored. Stop the app. Repeat with `base_direction = "auto"` and a row whose first strong character is Arabic; it right-anchors.

- [ ] **Step 7: Commit**

```powershell
git add src/visual.rs src/app.rs
git commit -m "feat: wire base direction config into per-row visual maps"
```

---

### Task 4: Run shaping with rustybuzz

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/main.rs`
- Create: `src/shape.rs`
- Test: `src/shape.rs` (inline)

**Interfaces:**
- Consumes: `bidi::RowText`, `visual::{VisualMap, VisualRun}`, `rustybuzz::{Face, UnicodeBuffer, Direction, BufferClusterLevel}`.
- Produces:
  - `shape::Glyph { glyph_id: u32, cluster: usize, x_advance: i32, x_offset: i32, y_offset: i32 }`
  - `shape::Cluster { cells: Range<usize>, slots: Range<usize>, glyphs: Vec<Glyph> }`
  - `shape::ShapedRun { slots: Range<usize>, arabic: bool, units_per_em: i32, clusters: Vec<Cluster> }`
  - `shape::Faces<'a> { primary: Face<'a>, arabic: Option<Face<'a>> }`
  - `Faces::new(Face<'a>, Option<Face<'a>>) -> Faces<'a>`
  - `shape::shape_row(&RowText, &VisualMap, &Faces<'_>) -> Vec<ShapedRun>`

- [ ] **Step 1: Add the dependency and module declaration**

In `Cargo.toml`, under `[dependencies]`, add:

```toml
rustybuzz = "0.20"
```

In `src/main.rs`, add `mod shape;` between `render` and `selection`:

```rust
mod render;
mod selection;
mod shape;
mod term;
```

- [ ] **Step 2: Write the failing tests**

Append to `src/shape.rs` (create the file with only this block for now):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::bidi::RowText;
    use crate::config::BaseDirection;
    use crate::term::{Cell, RowSnapshot, Style};
    use crate::visual::VisualMap;
    use rustybuzz::{Direction, Face, UnicodeBuffer};
    use unicode_width::UnicodeWidthStr;

    fn cell(text: &str) -> Cell {
        let width = UnicodeWidthStr::width(text).clamp(1, 2) as u8;
        Cell {
            text: text.to_string(),
            width,
            style: Style::default(),
        }
    }

    fn row(texts: &[&str]) -> RowSnapshot {
        RowSnapshot {
            cells: texts.iter().map(|text| cell(text)).collect(),
            wrapped: false,
        }
    }

    fn find_arabic_font() -> Option<(Vec<u8>, u32)> {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        for family in [
            "Kawkab Mono",
            "Segoe UI",
            "Arial",
            "Tahoma",
            "Noto Naskh Arabic",
            "Noto Sans Arabic",
            "DejaVu Sans",
            "FreeSerif",
        ] {
            let query = fontdb::Query {
                families: &[fontdb::Family::Name(family)],
                weight: fontdb::Weight::NORMAL,
                stretch: fontdb::Stretch::Normal,
                style: fontdb::Style::Normal,
            };
            if let Some(id) = database.query(&query) {
                if let Some((data, index)) =
                    database.with_face_data(id, |data, index| (data.to_vec(), index))
                {
                    return Some((data, index));
                }
            }
        }
        None
    }

    fn with_arabic_font<T>(f: impl FnOnce(&[u8], u32) -> T) -> Option<T> {
        let (data, index) = find_arabic_font()?;
        Some(f(&data, index))
    }

    #[test]
    fn arabic_row_shapes_as_one_run() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["س", "ل", "م"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape_row(&text, &map, &faces);
            (runs.len(), runs[0].clusters.len())
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_eq!(result, (1, 3));
    }

    #[test]
    fn arabic_letters_use_joined_forms() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["س", "ل", "م"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape_row(&text, &map, &faces);
            let joined = runs[0]
                .clusters
                .iter()
                .find(|cluster| cluster.cells.start == 0)
                .expect("first cluster")
                .glyphs[0]
                .glyph_id;
            let face = Face::from_slice(data, index).expect("face");
            let mut buffer = UnicodeBuffer::new();
            buffer.push_str("س");
            buffer.set_direction(Direction::RightToLeft);
            let isolated = rustybuzz::shape(&face, &[], buffer).glyph_infos()[0].glyph_id;
            (joined, isolated)
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_ne!(
            result.0, result.1,
            "the initial form must differ from the isolated form"
        );
    }

    #[test]
    fn lam_alef_is_one_cluster_across_two_cells() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["ل", "ا"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape_row(&text, &map, &faces);
            runs[0].clusters[0].clone()
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_eq!(result.cells, 0..2);
        assert_eq!(result.slots, 0..2);
        assert!(!result.glyphs.is_empty());
    }

    #[test]
    fn combining_marks_stay_in_the_base_cluster() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["مَ"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape_row(&text, &map, &faces);
            (runs[0].clusters.len(), runs[0].clusters[0].cells.clone())
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_eq!(result.0, 1);
        assert_eq!(result.1, 0..1);
    }

    #[test]
    fn mixed_direction_rows_split_at_the_run_boundary() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["a", "b", "c", "م", "ر", "ح"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape_row(&text, &map, &faces);
            let counts: Vec<usize> = runs.iter().map(|run| run.clusters.len()).collect();
            (runs.len(), counts, runs[0].slots.clone(), runs[1].slots.clone())
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_eq!(result.0, 2);
        assert_eq!(result.1, vec![3, 3]);
        assert_eq!(result.2, 0..3);
        assert_eq!(result.3, 3..6);
    }

    #[test]
    fn identity_maps_still_shape_arabic_with_rtl_joining() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["م", "ر", "ح"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::identity(3);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape_row(&text, &map, &faces);
            let joined = runs[0]
                .clusters
                .iter()
                .find(|cluster| cluster.cells.start == 0)
                .expect("first cluster")
                .glyphs[0]
                .glyph_id;
            let face = Face::from_slice(data, index).expect("face");
            let mut buffer = UnicodeBuffer::new();
            buffer.push_str("م");
            buffer.set_direction(Direction::RightToLeft);
            let isolated = rustybuzz::shape(&face, &[], buffer).glyph_infos()[0].glyph_id;
            (joined, isolated)
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_ne!(
            result.0, result.1,
            "identity maps must still use RTL joining forms"
        );
    }

    #[test]
    fn clusters_cover_every_visual_slot() {
        let Some(result) = with_arabic_font(|data, index| {
            let line = row(&["a", "b", "c", "م", "ر", "ح"]);
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            let runs = shape_row(&text, &map, &faces);
            let mut seen = vec![false; 6];
            for run in &runs {
                for cluster in &run.clusters {
                    for slot in cluster.slots.clone() {
                        assert!(!seen[slot], "slot {slot} covered twice");
                        seen[slot] = true;
                    }
                }
            }
            seen
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert!(result.into_iter().all(|covered| covered));
    }

    #[test]
    fn style_changes_do_not_split_a_run() {
        let Some(result) = with_arabic_font(|data, index| {
            let mut first = cell("م");
            first.style.fg = crate::term::Color::Indexed(1);
            let second = cell("ر");
            let line = RowSnapshot {
                cells: vec![first, second],
                wrapped: false,
            };
            let text = RowText::flatten(&line);
            let map = VisualMap::from_bidi(&line, BaseDirection::Ltr);
            let face = Face::from_slice(data, index).expect("face");
            let faces = Faces::new(face, None);
            shape_row(&text, &map, &faces).len()
        }) else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert_eq!(result, 1, "style is applied after shaping, so one bidi run stays one shaped run");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test shape::tests -v`

Expected: FAIL to compile with `cannot find type Faces`, `shape_row`, and `ShapedRun`.

- [ ] **Step 4: Implement `src/shape.rs`**

Prepend this implementation above the test module:

```rust
//! Bidi-run shaping with rustybuzz and cluster-to-cell mapping.
//!
// Consumed by the renderer in Task 6; the tests below are the only callers
// until then.
#![allow(dead_code)]

use std::ops::Range;

use rustybuzz::{BufferClusterLevel, Direction, Face, UnicodeBuffer};

use crate::bidi::RowText;
use crate::visual::VisualMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Glyph {
    pub glyph_id: u32,
    pub cluster: usize,
    pub x_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cluster {
    pub cells: Range<usize>,
    pub slots: Range<usize>,
    pub glyphs: Vec<Glyph>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapedRun {
    pub slots: Range<usize>,
    pub arabic: bool,
    pub units_per_em: i32,
    pub clusters: Vec<Cluster>,
}

pub struct Faces<'a> {
    pub primary: Face<'a>,
    pub arabic: Option<Face<'a>>,
}

impl<'a> Faces<'a> {
    pub fn new(primary: Face<'a>, arabic: Option<Face<'a>>) -> Self {
        Self { primary, arabic }
    }
}

fn is_arabic_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF
    )
}

fn contains_arabic(text: &str) -> bool {
    text.chars().any(is_arabic_char)
}

/// Fallback direction for identity maps (bidi disabled): use the first strong
/// character instead of a bidi level.
fn first_strong_is_rtl(text: &str) -> bool {
    for ch in text.chars() {
        if is_arabic_char(ch) {
            return true;
        }
        if ch.is_ascii_alphabetic() {
            return false;
        }
    }
    false
}

pub fn shape_row(row: &RowText, map: &VisualMap, faces: &Faces<'_>) -> Vec<ShapedRun> {
    let mut out = Vec::new();

    for run in map.runs() {
        if run.slots.is_empty() {
            continue;
        }
        let start = row.cell_starts[run.cells.start];
        let end = row.cell_starts[run.cells.end];
        let text = &row.text[start..end];
        if text.is_empty() {
            continue;
        }

        let arabic = contains_arabic(text) && faces.arabic.is_some();
        let face = if arabic {
            faces.arabic.as_ref().expect("arabic face checked above")
        } else {
            &faces.primary
        };
        debug_assert_eq!(
            run.level % 2 == 1,
            map.levels()
                .get(run.cells.start)
                .is_some_and(|level| level % 2 == 1),
            "run level and map level must agree"
        );
        let rtl = run.level % 2 == 1 || first_strong_is_rtl(text);

        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.set_direction(if rtl {
            Direction::RightToLeft
        } else {
            Direction::LeftToRight
        });
        buffer.set_cluster_level(BufferClusterLevel::MonotoneGraphemes);
        let shaped = rustybuzz::shape(face, &[], buffer);

        let infos = shaped.glyph_infos();
        let positions = shaped.glyph_positions();
        let mut groups: Vec<(usize, Vec<Glyph>)> = Vec::new();
        for (info, position) in infos.iter().zip(positions) {
            let glyph = Glyph {
                glyph_id: info.glyph_id,
                cluster: start + info.cluster as usize,
                x_advance: position.x_advance,
                x_offset: position.x_offset,
                y_offset: position.y_offset,
            };
            let cluster = glyph.cluster;
            match groups.last_mut() {
                Some((existing, glyphs)) if *existing == cluster => glyphs.push(glyph),
                _ => groups.push((cluster, vec![glyph])),
            }
        }

        let mut starts: Vec<usize> = groups.iter().map(|(cluster, _)| *cluster).collect();
        starts.sort_unstable();

        let mut clusters = Vec::with_capacity(groups.len());
        for (cluster, glyphs) in groups {
            let next = starts
                .iter()
                .copied()
                .find(|candidate| *candidate > cluster)
                .unwrap_or(end);
            let first = row.cell_of_byte(cluster);
            let mut last = row.cell_of_byte(next.saturating_sub(1).max(cluster));
            last += usize::from(row.widths[last].max(1) - 1);
            last = last.min(map.cols().saturating_sub(1));
            let cells = first..last + 1;
            let a = map.visual_of(cells.start);
            let b = map.visual_of(cells.end - 1);
            let slots = a.min(b)..a.max(b) + 1;
            clusters.push(Cluster {
                cells,
                slots,
                glyphs,
            });
        }
        clusters.sort_by_key(|cluster| cluster.slots.start);

        out.push(ShapedRun {
            slots: run.slots.clone(),
            arabic,
            units_per_em: face.units_per_em(),
            clusters,
        });
    }

    out
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test shape::tests -v`

Expected: PASS — 8 tests pass when an Arabic-capable system font is installed. On a machine with none, the Arabic tests print `skipping: no Arabic-capable system font installed` and pass.

Keep the three temporary `#[allow(dead_code)]` attributes on `VisualMap::cols`, `VisualMap::levels`, and `VisualMap::runs` in `src/visual.rs` until Task 6: shaping is the only caller, and `shape_row` itself is test-only until the renderer wires it in.

Run: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`

Expected: clean.

- [ ] **Step 6: Commit**

```powershell
git add Cargo.toml Cargo.lock src/main.rs src/shape.rs
git commit -m "feat: shape bidi runs with rustybuzz"
```

---

### Task 5: Arabic-capable font fallback

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/fonts.rs`
- Modify: `src/app.rs`
- Modify: `src/render/mod.rs`
- Modify: `src/render/glyph_cache.rs`
- Create: `assets/fonts/README.md`
- Test: `src/fonts.rs` (inline)

**Interfaces:**
- Consumes: `fontdb`, `ttf-parser`, `ab_glyph`.
- Produces:
  - `FontFace { font: FontVec, data: Vec<u8>, index: u32, family: String }`
  - `FontSet { font, data, index, size_px, cell_w, cell_h, ascent, descent, arabic: Option<FontFace> }`
  - `FontSet::load(family, arabic_family, size_px, line_height) -> Result<FontSet, FontError>`
  - `FontSet::has_arabic() -> bool`
  - `FontSet::font_for(arabic: bool) -> (&FontVec, &[u8], u32)`

- [ ] **Step 1: Add `ttf-parser`**

In `Cargo.toml`, under `[dependencies]`, add after `fontdb`:

```toml
ttf-parser = "0.25"
```

- [ ] **Step 2: Write the failing tests**

Replace the test module in `src/fonts.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::Font;

    fn load(family: &str) -> Option<FontSet> {
        FontSet::load(family, "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
            .ok()
    }

    #[test]
    fn loads_a_font_with_positive_metrics() {
        let font = load("Consolas").expect("a monospace font must exist");
        assert!(font.cell_w > 0);
        assert!(font.cell_h > 0);
        assert!(font.ascent > 0.0);
        assert!(font.descent < 0.0);
        assert!(font.font.glyph_id('A').0 != 0);
        assert!(font.font.glyph_id('M').0 != 0);
    }

    #[test]
    fn missing_configured_arabic_family_still_loads() {
        let font = load("Consolas").expect("a monospace font must exist");
        assert_eq!(font.font_for(false).2, font.index);
        assert!(font.size_px > 0.0);
    }

    #[test]
    fn arabic_face_covers_arabic_letters() {
        let Some(font) = load("Consolas") else {
            eprintln!("skipping: no monospace font installed");
            return;
        };
        let Some(face) = font.arabic.as_ref() else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        assert!(face.font.glyph_id('م').0 != 0);
        assert!(face.font.glyph_id('ل').0 != 0);
        assert!(face.font.glyph_id('ا').0 != 0);
        assert!(!face.family.is_empty());
        assert!(font.has_arabic());
    }

    #[test]
    fn unknown_arabic_family_uses_the_fallback_chain() {
        let fonts = FontSet::load("Consolas", "Definitely Not A Font 12345", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Definitely Not A Font 12345", 14.0, 1.2))
            .expect("a monospace font must exist");
        if let Some(face) = &fonts.arabic {
            assert_ne!(face.family, "Definitely Not A Font 12345");
        }
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test fonts::tests -v`

Expected: FAIL to compile because `FontSet::load` takes three arguments and `arabic`, `has_arabic`, and `font_for` do not exist.

- [ ] **Step 4: Replace `src/fonts.rs` with the fallback implementation**

```rust
//! Font discovery, loading, and metrics.
//!
// Consumed by shaping and the renderer in Task 6; the tests below are the only
// callers of the new fallback API until then.
#![allow(dead_code)]

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use fontdb::{Database, Family, Query, Stretch, Style, Weight};

#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("no usable monospace font found")]
    NoFont,
    #[error("font data is invalid")]
    InvalidFont,
}

/// Families tried, in order, when the configured Arabic family is unavailable.
const ARABIC_FALLBACKS: &[&str] = &[
    "Segoe UI",
    "Arial",
    "Tahoma",
    "Noto Naskh Arabic",
    "Noto Sans Arabic",
    "DejaVu Sans",
    "FreeSerif",
];

pub struct FontFace {
    pub font: FontVec,
    pub data: Vec<u8>,
    pub index: u32,
    pub family: String,
}

pub struct FontSet {
    pub font: FontVec,
    pub data: Vec<u8>,
    pub index: u32,
    pub size_px: f32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub ascent: f32,
    // No runtime consumer; part of the published font-metrics pair.
    #[allow(dead_code)]
    pub descent: f32,
    pub arabic: Option<FontFace>,
}

impl FontSet {
    pub fn load(
        family: &str,
        arabic_family: &str,
        size_px: f32,
        line_height: f32,
    ) -> Result<Self, FontError> {
        let mut database = Database::new();
        database.load_system_fonts();
        let (data, index) = load_primary(&database, family)?;
        let font = FontVec::try_from_vec_and_index(data.clone(), index)
            .map_err(|_| FontError::InvalidFont)?;

        let scale = PxScale::from(size_px);
        let scaled = font.as_scaled(scale);
        let ascent = scaled.ascent();
        let descent = scaled.descent();
        let mut advance = scaled.h_advance(font.glyph_id('M'));
        if advance <= 0.0 {
            advance = scaled.h_advance(font.glyph_id('0'));
        }
        let cell_w = advance.ceil().max(1.0) as u32;
        let cell_h = ((ascent - descent) * line_height).ceil().max(1.0) as u32;

        Ok(Self {
            font,
            data,
            index,
            size_px,
            cell_w,
            cell_h,
            ascent,
            descent,
            arabic: load_arabic(&database, arabic_family),
        })
    }

    pub fn has_arabic(&self) -> bool {
        self.arabic.is_some()
    }

    /// Font data for a shaped run. Falls back to the primary face when the
    /// Arabic face or the requested slot is unavailable.
    pub fn font_for(&self, arabic: bool) -> (&FontVec, &[u8], u32) {
        match (arabic, &self.arabic) {
            (true, Some(face)) => (&face.font, &face.data, face.index),
            _ => (&self.font, &self.data, self.index),
        }
    }
}

fn load_primary(database: &Database, family: &str) -> Result<(Vec<u8>, u32), FontError> {
    let requested = [Family::Name(family)];
    let monospace = [Family::Monospace];
    let mut query = Query {
        families: &requested,
        weight: Weight::NORMAL,
        stretch: Stretch::Normal,
        style: Style::Normal,
    };
    let id = match database.query(&query) {
        Some(id) => id,
        None => {
            tracing::warn!("font family {family:?} not found; using the system monospace font");
            query.families = &monospace;
            database.query(&query).ok_or(FontError::NoFont)?
        }
    };
    database
        .with_face_data(id, |data, index| (data.to_vec(), index))
        .ok_or(FontError::NoFont)
}

fn load_arabic(database: &Database, configured: &str) -> Option<FontFace> {
    let candidates = std::iter::once(configured).chain(ARABIC_FALLBACKS.iter().copied());
    for family in candidates {
        if family.is_empty() {
            continue;
        }
        let families = [Family::Name(family)];
        let query = Query {
            families: &families,
            weight: Weight::NORMAL,
            stretch: Stretch::Normal,
            style: Style::Normal,
        };
        let Some(id) = database.query(&query) else {
            continue;
        };
        let Some((data, index)) =
            database.with_face_data(id, |data, index| (data.to_vec(), index))
        else {
            continue;
        };
        if !has_arabic_coverage(&data, index) {
            continue;
        }
        let Ok(font) = FontVec::try_from_vec_and_index(data.clone(), index) else {
            continue;
        };
        if family != configured {
            tracing::warn!(
                requested = configured,
                using = family,
                "configured Arabic font unavailable; using a system fallback"
            );
        }
        let face = FontFace {
            font,
            data,
            index,
            family: family.to_string(),
        };
        tracing::debug!(family = %face.family, "Arabic face loaded");
        return Some(face);
    }
    tracing::warn!(
        requested = configured,
        "no Arabic-capable system font found; Arabic text uses the primary face"
    );
    None
}

fn has_arabic_coverage(data: &[u8], index: u32) -> bool {
    let Ok(face) = ttf_parser::Face::parse(data, index) else {
        return false;
    };
    ['ا', 'ل', 'م', 'ب', 'س']
        .iter()
        .all(|ch| face.glyph_index(*ch).is_some())
}
```

- [ ] **Step 5: Update every `FontSet::load` call site**

In `src/app.rs`:

```rust
        let fonts = FontSet::load(
            &self.config.font.family,
            &self.config.font.arabic_family,
            self.config.font.size,
            self.config.font.line_height,
        )?;
```

In `src/render/mod.rs` (`default_renderer`) and `src/render/glyph_cache.rs` (both tests), change each call:

```rust
        let fonts = FontSet::load("Consolas", "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
```

- [ ] **Step 6: Document the missing asset**

Create `assets/fonts/README.md`:

```markdown
# Bundled fonts

Termyx prefers a cell-compatible Arabic face. The configured default is
`Kawkab Mono`, which is OFL 1.1.

This directory intentionally contains no font binary yet. Until the asset task
lands, `src/fonts.rs` falls back to an installed Arabic-capable system face
(Segoe UI, Arial, Tahoma, Noto Naskh Arabic, Noto Sans Arabic, DejaVu Sans, or
FreeSerif) and logs a warning naming the requested family.

When the asset is added:

- Place `KawkabMono-Regular.ttf` here.
- Add the OFL 1.1 license as `assets/fonts/KawkabMono-OFL.txt`.
- Load it with `fontdb::Database::load_font_data` before querying system fonts.
- Keep the fallback chain for machines where the file is missing.
```

- [ ] **Step 7: Run the tests to verify they pass**

Run:

```powershell
cargo test fonts::tests -v
cargo test render::tests -v
cargo test render::glyph_cache::tests -v
```

Expected: PASS. The Arabic test prints a skip message only when no Arabic-capable system font exists.

Run: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`

Expected: clean.

- [ ] **Step 8: Commit**

```powershell
git add Cargo.toml Cargo.lock src/fonts.rs src/app.rs src/render/mod.rs src/render/glyph_cache.rs assets/fonts/README.md
git commit -m "feat: add Arabic font fallback selection"
```

---

### Task 6: Render shaped clusters at visual cell slots

**Files:**
- Modify: `src/shape.rs`
- Modify: `src/render/glyph_cache.rs`
- Modify: `src/render/mod.rs`
- Test: `src/render/mod.rs`, `src/render/glyph_cache.rs` (inline)

**Interfaces:**
- Consumes: `shape::{shape_row, ShapedRun, Faces}`, `bidi::RowText`, `fonts::FontSet`.
- Produces:
  - `shape::Faces::from_fonts(&FontSet) -> Option<Faces<'_>>`
  - `GlyphKey { font: u8, glyph: u16, size_px: u32 }`
  - `GlyphCache::get(font_id: u8, font: &FontVec, glyph: GlyphId, size_px: f32) -> Option<GlyphSlot>`
  - `Renderer::draw` keeps its signature and draws shaped clusters, backgrounds, selection, decorations, and the cursor.

- [ ] **Step 1: Write the failing tests**

In `src/render/glyph_cache.rs`, replace both tests with:

```rust
    #[test]
    fn caches_the_same_glyph_slot() {
        let fonts = FontSet::load("Consolas", "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
            .expect("font");
        let glyph = fonts.font.glyph_id('A');
        let mut cache = GlyphCache::new();
        let first = cache.get(0, &fonts.font, glyph, fonts.size_px).unwrap();
        let second = cache.get(0, &fonts.font, glyph, fonts.size_px).unwrap();
        assert_eq!(first.rect, second.rect);
        assert!(first.rect.w > 0 && first.rect.h > 0);
    }

    #[test]
    fn font_id_is_part_of_the_key() {
        let fonts = FontSet::load("Consolas", "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
            .expect("font");
        let glyph = fonts.font.glyph_id('A');
        let mut cache = GlyphCache::new();
        assert!(cache.get(0, &fonts.font, glyph, fonts.size_px).is_some());
        assert!(cache.get(1, &fonts.font, glyph, fonts.size_px).is_some());
        assert_eq!(cache.slots.len(), 2);
    }

    #[test]
    fn missing_glyph_returns_none_and_is_cached() {
        let fonts = FontSet::load("Consolas", "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
            .expect("font");
        let glyph = fonts.font.glyph_id(' ');
        let mut cache = GlyphCache::new();
        assert!(cache.get(0, &fonts.font, glyph, fonts.size_px).is_none());
        assert!(cache.get(0, &fonts.font, glyph, fonts.size_px).is_none());
        let key = GlyphKey {
            font: 0,
            glyph: glyph.0,
            size_px: fonts.size_px.round() as u32,
        };
        assert!(matches!(cache.slots.get(&key), Some(&None)));
    }
```

In `src/render/mod.rs`, replace the three tests `draw_text_paints_foreground_pixels`, `glyphs_advance_exactly_one_cell`, and `combining_marks_do_not_advance_the_pen` with these tests, and add the helpers:

```rust
    use crate::config::BaseDirection;
    use crate::term::{LogicalPos, RowSnapshot};

    fn row_snapshot(cells: Vec<Cell>) -> GridSnapshot {
        let cols = cells.len();
        GridSnapshot {
            cols,
            rows: 1,
            lines: vec![RowSnapshot {
                cells,
                wrapped: false,
            }],
            cursor: CursorState {
                row: 1,
                logical: 0,
                affinity: Affinity::Before,
            },
            display_offset: 0,
        }
    }

    fn arabic_renderer() -> Option<Renderer> {
        let fonts = FontSet::load("Consolas", "Kawkab Mono", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", "Kawkab Mono", 14.0, 1.2))
            .ok()?;
        if !fonts.has_arabic() {
            return None;
        }
        Renderer::new(fonts, Theme::from_config(&ThemeConfig::default()), 4).ok()
    }

    #[test]
    fn fit_cluster_centers_narrow_and_shrinks_wide_clusters() {
        assert_eq!(fit_cluster(5.0, 10.0), (2.5, 1.0));
        let (offset, scale) = fit_cluster(20.0, 10.0);
        assert_eq!(offset, 0.0);
        assert!((scale - 0.5).abs() < f32::EPSILON);
        assert_eq!(fit_cluster(0.0, 10.0), (0.0, 1.0));
    }

    #[test]
    fn latin_cluster_is_painted_at_its_cell_slot() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 4, cell_h * 2);
        let snapshot = row_snapshot(vec![
            Cell { text: " ".into(), width: 1, style: Style::default() },
            Cell { text: " ".into(), width: 1, style: Style::default() },
            Cell { text: "M".into(), width: 1, style: Style::default() },
            Cell { text: " ".into(), width: 1, style: Style::default() },
        ]);
        renderer.draw(&snapshot, &[VisualMap::identity(4)], None);
        renderer.sync();
        let padding = renderer.padding();
        let columns = painted_columns(&renderer);
        assert!(!columns.is_empty());
        assert!(columns
            .iter()
            .all(|column| *column + 1 >= padding + 2 * cell_w && *column < padding + 3 * cell_w));
    }

    #[test]
    fn lam_alef_ligature_spans_two_cells_without_shifting_following_text() {
        let Some(mut renderer) = arabic_renderer() else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 4, cell_h * 2);
        let snapshot = row_snapshot(vec![
            Cell { text: "ل".into(), width: 1, style: Style::default() },
            Cell { text: "ا".into(), width: 1, style: Style::default() },
            Cell { text: "x".into(), width: 1, style: Style::default() },
            Cell { text: " ".into(), width: 1, style: Style::default() },
        ]);
        let maps = [VisualMap::from_bidi(&snapshot.lines[0], BaseDirection::Ltr)];
        renderer.draw(&snapshot, &maps, None);
        renderer.sync();
        let start = renderer.padding();
        let columns = painted_columns(&renderer);
        assert!(
            columns.iter().any(|column| *column < start + cell_w),
            "the ligature must paint in the first cell: {columns:?}"
        );
        assert!(
            columns
                .iter()
                .any(|column| *column >= start + cell_w && *column < start + 2 * cell_w),
            "the ligature must cross into the second cell: {columns:?}"
        );
        assert!(
            columns
                .iter()
                .any(|column| *column >= start + 2 * cell_w && *column < start + 3 * cell_w),
            "the following glyph must stay in cell 2: {columns:?}"
        );
        assert!(
            columns.iter().all(|column| *column < start + 3 * cell_w),
            "nothing may paint past the row: {columns:?}"
        );
    }

    #[test]
    fn selection_fills_visual_slots_of_logical_cells() {
        let Some(mut renderer) = arabic_renderer() else {
            eprintln!("skipping: no Arabic-capable system font installed");
            return;
        };
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 7, cell_h * 2);
        let snapshot = row_snapshot(vec![
            Cell { text: "a".into(), width: 1, style: Style::default() },
            Cell { text: "b".into(), width: 1, style: Style::default() },
            Cell { text: "c".into(), width: 1, style: Style::default() },
            Cell { text: "م".into(), width: 1, style: Style::default() },
            Cell { text: "ر".into(), width: 1, style: Style::default() },
            Cell { text: "ح".into(), width: 1, style: Style::default() },
            Cell { text: " ".into(), width: 1, style: Style::default() },
        ]);
        let mut selection = Selection::new(LogicalPos { row: 0, col: 3 });
        selection.update(LogicalPos { row: 0, col: 5 });
        let maps = [VisualMap::from_bidi(&snapshot.lines[0], BaseDirection::Ltr)];
        renderer.draw(&snapshot, &maps, Some(&selection));
        renderer.sync();
        let selection_color = packed(renderer.theme.selection);
        let padding = renderer.padding();
        let width = renderer.pixmap.width() as usize;
        for slot in 0..7 {
            let x = padding + slot * cell_w + 1;
            let y = padding + 1;
            let pixel = renderer.pixels()[y as usize * width + x as usize];
            let selected = (3..6).contains(&slot);
            assert_eq!(
                pixel == selection_color,
                selected,
                "slot {slot} selection state"
            );
        }
    }

    #[test]
    fn bidi_control_cells_paint_nothing() {
        let mut renderer = default_renderer();
        let cell_w = renderer.fonts.cell_w;
        let cell_h = renderer.fonts.cell_h;
        renderer.resize(cell_w * 2, cell_h * 2);
        let snapshot = row_snapshot(vec![Cell {
            text: "\u{200F}".into(),
            width: 1,
            style: Style::default(),
        }]);
        renderer.draw(&snapshot, &[VisualMap::identity(1)], None);
        renderer.sync();
        assert!(painted_rows(&renderer).is_empty());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run:

```powershell
cargo test render::tests -v
cargo test render::glyph_cache::tests -v
```

Expected: FAIL with `no function fit_cluster`, `GlyphCache::get` argument mismatch, and `Faces::from_fonts` missing.

- [ ] **Step 3: Extend the glyph cache key**

In `src/render/glyph_cache.rs`, replace `GlyphKey` and `get`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    /// 0 = primary face, 1 = Arabic fallback face.
    pub font: u8,
    pub glyph: u16,
    pub size_px: u32,
}
```

```rust
    pub fn get(
        &mut self,
        font_id: u8,
        font: &FontVec,
        glyph: GlyphId,
        size_px: f32,
    ) -> Option<GlyphSlot> {
        let key = GlyphKey {
            font: font_id,
            glyph: glyph.0,
            size_px: size_px.round() as u32,
        };
        if let Some(slot) = self.slots.get(&key) {
            return *slot;
        }
        let slot = self.rasterize(font, glyph, key.size_px as f32);
        self.slots.insert(key, slot);
        slot
    }
```

- [ ] **Step 4: Add `Faces::from_fonts`**

In `src/shape.rs`, add to `impl<'a> Faces<'a>`:

```rust
    pub fn from_fonts(fonts: &'a crate::fonts::FontSet) -> Option<Self> {
        let primary = Face::from_slice(&fonts.data, fonts.index)?;
        let arabic = if fonts.has_arabic() {
            fonts
                .arabic
                .as_ref()
                .and_then(|face| Face::from_slice(&face.data, face.index))
        } else {
            None
        };
        Some(Self::new(primary, arabic))
    }
```

- [ ] **Step 5: Replace the renderer pass**

In `src/render/mod.rs`, add imports:

```rust
use ab_glyph::GlyphId;
use crate::bidi;
use crate::shape;
use crate::term::RowSnapshot;
```

Replace the whole `pub fn draw(...)` body with:

```rust
    pub fn draw(
        &mut self,
        snapshot: &GridSnapshot,
        maps: &[VisualMap],
        selection: Option<&Selection>,
    ) {
        self.draw_background();
        let cell_w = self.fonts.cell_w as f32;
        let cell_h = self.fonts.cell_h as f32;
        let padding = self.padding as f32;

        self.draw_cell_backgrounds(snapshot, maps, selection, cell_w, cell_h);

        let mut shaped_rows: Vec<Vec<shape::ShapedRun>> = Vec::with_capacity(snapshot.lines.len());
        if let Some(faces) = shape::Faces::from_fonts(&self.fonts) {
            for (row, line) in snapshot.lines.iter().enumerate() {
                let Some(map) = maps.get(row) else {
                    shaped_rows.push(Vec::new());
                    continue;
                };
                let text = bidi::RowText::flatten(line);
                shaped_rows.push(shape::shape_row(&text, map, &faces));
            }
        } else {
            shaped_rows.resize_with(snapshot.lines.len(), Vec::new);
        }

        for (row, runs) in shaped_rows.iter().enumerate() {
            let Some(line) = snapshot.lines.get(row) else {
                continue;
            };
            for run in runs {
                self.draw_shaped_run(run, line, row, cell_w, cell_h);
            }
        }

        self.draw_decorations(snapshot, maps, cell_w, cell_h);

        if snapshot.display_offset == 0 {
            if let Some(map) = maps.get(snapshot.cursor.row) {
                let visual = map.cursor_visual(&snapshot.cursor);
                let x = padding + visual as f32 * cell_w;
                let y = padding + snapshot.cursor.row as f32 * cell_h;
                self.fill_rect(x, y, 2.0, cell_h, self.theme.cursor);
            }
        }
    }

    fn draw_cell_backgrounds(
        &mut self,
        snapshot: &GridSnapshot,
        maps: &[VisualMap],
        selection: Option<&Selection>,
        cell_w: f32,
        cell_h: f32,
    ) {
        for (row, line) in snapshot.lines.iter().enumerate() {
            let Some(map) = maps.get(row) else {
                continue;
            };
            let y = self.padding as f32 + row as f32 * cell_h;
            for (logical, cell) in line.cells.iter().enumerate() {
                let x = self.padding as f32 + map.visual_of(logical) as f32 * cell_w;
                let foreground = self.resolve(cell.style.fg);
                let background = cell
                    .style
                    .bg
                    .map(|bg| self.resolve(bg))
                    .unwrap_or(self.theme.background);
                let (foreground, background) = if cell.style.inverse {
                    (background, foreground)
                } else {
                    (foreground, background)
                };
                if cell.style.bg.is_some() || cell.style.inverse {
                    self.fill_rect(x, y, cell_w, cell_h, background);
                }
                if selection.is_some_and(|selection| selection.contains(row, logical)) {
                    self.fill_rect(x, y, cell_w, cell_h, self.theme.selection);
                }
            }
        }
    }

    fn draw_shaped_run(
        &mut self,
        run: &shape::ShapedRun,
        line: &RowSnapshot,
        row: usize,
        cell_w: f32,
        cell_h: f32,
    ) {
        let padding = self.padding as f32;
        let baseline = padding + row as f32 * cell_h + self.fonts.ascent;
        let units = run.units_per_em.max(1) as f32;
        let scale = self.fonts.size_px / units;
        let font_id = u8::from(run.arabic);
        let (font, _, _) = self.fonts.font_for(run.arabic);

        for cluster in &run.clusters {
            let Some(first) = line.cells.get(cluster.cells.start) else {
                continue;
            };
            if first.style.hidden {
                continue;
            }
            if cluster.cells.len() == 1 && bidi::is_bidi_control_only(&first.text) {
                continue;
            }
            let color = self.resolve(first.style.fg);
            let slot_start = cluster.slots.start.max(run.slots.start);
            let slot_end = cluster.slots.end.min(run.slots.end);
            if slot_start >= slot_end {
                continue;
            }
            let slot_w = (slot_end - slot_start) as f32 * cell_w;
            let natural: f32 = cluster
                .glyphs
                .iter()
                .map(|glyph| glyph.x_advance as f32 * scale)
                .sum();
            let (offset, scale_x) = fit_cluster(natural, slot_w);
            let mut pen = padding + slot_start as f32 * cell_w + offset;

            for glyph in &cluster.glyphs {
                let id = GlyphId(u16::try_from(glyph.glyph_id).unwrap_or(0));
                if let Some(slot) = self.cache.get(font_id, font, id, self.fonts.size_px) {
                    let x = pen + glyph.x_offset as f32 * scale * scale_x;
                    let y = baseline - glyph.y_offset as f32 * scale;
                    blit_scaled(
                        &mut self.pixmap,
                        &self.cache.atlas,
                        slot,
                        x,
                        y,
                        color,
                        scale_x,
                    );
                }
                pen += glyph.x_advance as f32 * scale * scale_x;
            }
        }
    }

    fn draw_decorations(
        &mut self,
        snapshot: &GridSnapshot,
        maps: &[VisualMap],
        cell_w: f32,
        cell_h: f32,
    ) {
        for (row, line) in snapshot.lines.iter().enumerate() {
            let Some(map) = maps.get(row) else {
                continue;
            };
            let y = self.padding as f32 + row as f32 * cell_h;
            for (logical, cell) in line.cells.iter().enumerate() {
                if cell.style.hidden {
                    continue;
                }
                let foreground = self.resolve(cell.style.fg);
                let x = self.padding as f32 + map.visual_of(logical) as f32 * cell_w;
                let span = cell_w * cell.width.max(1) as f32;
                if cell.style.underline {
                    self.fill_rect(
                        x,
                        (y + self.fonts.ascent + 1.0).min(y + cell_h - 1.0),
                        span,
                        1.0,
                        foreground,
                    );
                }
                if cell.style.strikethrough {
                    self.fill_rect(x, y + (cell_h / 2.0).round(), span, 1.0, foreground);
                }
            }
        }
    }
```

Add these free functions near `blend`:

```rust
/// Centers a cluster that fits its slot and scales one that does not.
fn fit_cluster(natural: f32, slot_w: f32) -> (f32, f32) {
    if slot_w <= 0.0 || natural <= 0.0 {
        return (0.0, 1.0);
    }
    if natural > slot_w * 1.05 {
        (0.0, slot_w / natural)
    } else {
        ((slot_w - natural) / 2.0, 1.0)
    }
}

fn blit_scaled(
    pixmap: &mut Pixmap,
    atlas: &Atlas,
    slot: GlyphSlot,
    pen_x: f32,
    baseline: f32,
    color: Rgb,
    scale_x: f32,
) {
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let stride = width as usize * 4;
    let dst_w = (slot.rect.w as f32 * scale_x).round().max(1.0) as i32;
    let dst_x = pen_x.round() as i32 + (slot.bearing_x as f32 * scale_x).round() as i32;
    let dst_y = baseline.round() as i32 + slot.bearing_y;
    let atlas_width = atlas.width();
    let data = pixmap.data_mut();
    for row in 0..slot.rect.h as i32 {
        for col in 0..dst_w {
            let x = dst_x + col;
            let y = dst_y + row;
            if x < 0 || y < 0 || x >= width || y >= height {
                continue;
            }
            let src_col = (col as f32 / scale_x)
                .floor()
                .min((slot.rect.w - 1) as f32) as u32;
            let mask_x = slot.rect.x + src_col;
            let mask_y = slot.rect.y as i32 + row;
            let coverage = atlas.data()[(mask_y as u32 * atlas_width + mask_x) as usize];
            if coverage == 0 {
                continue;
            }
            let alpha = coverage as f32 / 255.0;
            let index = y as usize * stride + x as usize * 4;
            data[index] = blend(data[index], color.r, alpha);
            data[index + 1] = blend(data[index + 1], color.g, alpha);
            data[index + 2] = blend(data[index + 2], color.b, alpha);
        }
    }
}
```

Delete the old `draw_text`, `blit`, and `draw_background`-adjacent text path. Keep `draw_background`, `resolve`, `fill_rect`, `sync`, `pixels`, `padding`, and `blend`.

Add `bidi::is_bidi_control_only` in `src/bidi.rs`:

```rust
/// A cell whose entire text is bidi controls paints no glyph.
pub fn is_bidi_control_only(text: &str) -> bool {
    !text.is_empty() && text.chars().all(is_bidi_control)
}
```

Remove `#[allow(dead_code)]` from `is_bidi_control` in `src/bidi.rs`. The renderer now reaches it through `is_bidi_control_only`.

Remove the temporary module-level `#![allow(dead_code)]` lines from `src/shape.rs` and `src/fonts.rs`, and the three `#[allow(dead_code)]` attributes on `VisualMap::{cols, levels, runs}` in `src/visual.rs`. Every one of those items now has a production caller in the renderer.

- [ ] **Step 6: Run the tests to verify they pass**

Run:

```powershell
cargo test render::tests -v
cargo test render::glyph_cache::tests -v
cargo test shape::tests -v
```

Expected: PASS. Arabic renderer tests print a skip message when no Arabic-capable system font exists.

- [ ] **Step 7: Manual check**

Run: `cargo run --release`

Type `echo "مرحبا"` and `echo "test مرحبا 123"`. Expected: the Arabic letters are joined, the Arabic run reads right-to-left, the Latin text and digits keep their order, and the prompt does not move. Type `echo "لا"`; the lam-alef glyph spans two cells and the next character starts exactly at the next cell boundary.

- [ ] **Step 8: Commit**

```powershell
git add src/shape.rs src/bidi.rs src/render/mod.rs src/render/glyph_cache.rs
git commit -m "feat: render shaped glyph clusters at visual cell slots"
```

---

### Task 7: Cursor affinity at bidi run boundaries

**Files:**
- Modify: `src/visual.rs`
- Test: `src/visual.rs` (inline)

**Interfaces:**
- Consumes: `VisualMap::{visual_of, levels}`.
- Produces:
  - `VisualMap::is_rtl(usize) -> bool`
  - `VisualMap::cursor_visual(&CursorState) -> usize` with affinity-correct edges.

- [ ] **Step 1: Write the failing tests**

In `src/visual.rs` tests, replace `cursor_before_a_cell_sits_on_its_left_edge` and `cursor_after_a_cell_sits_on_its_right_edge` with these four tests. Keep every non-cursor test unchanged:

```rust
    #[test]
    fn cursor_uses_cell_edges_in_an_ltr_row() {
        let map = VisualMap::identity(10);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::Before)), 3);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::After)), 3);
        assert_eq!(map.cursor_visual(&cursor(10, Affinity::Before)), 10);
        assert_eq!(map.cursor_visual(&cursor(10, Affinity::After)), 10);
        assert_eq!(map.cursor_visual(&cursor(0, Affinity::After)), 0);
    }

    #[test]
    fn affinity_selects_a_side_at_a_bidi_run_boundary() {
        let map = VisualMap::from_bidi(
            &row(&["a", "b", "c", "م", "ر", "ح"]),
            BaseDirection::Ltr,
        );
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::After)), 3);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::Before)), 6);
        assert_eq!(map.cursor_visual(&cursor(6, Affinity::After)), 3);
        assert_eq!(map.cursor_visual(&cursor(6, Affinity::Before)), 3);
    }

    #[test]
    fn adjacent_cells_inside_a_run_share_one_gap() {
        let map = VisualMap::from_bidi(
            &row(&["a", "b", "c", "م", "ر", "ح"]),
            BaseDirection::Ltr,
        );
        assert_eq!(map.cursor_visual(&cursor(1, Affinity::Before)), 1);
        assert_eq!(map.cursor_visual(&cursor(1, Affinity::After)), 1);
        assert_eq!(map.cursor_visual(&cursor(4, Affinity::Before)), 5);
        assert_eq!(map.cursor_visual(&cursor(4, Affinity::After)), 5);
    }

    #[test]
    fn pure_rtl_start_and_end_are_mirrored() {
        let map = VisualMap::from_bidi(&row(&["م", "ر", "ح"]), BaseDirection::Ltr);
        assert_eq!(map.cursor_visual(&cursor(0, Affinity::Before)), 3);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::Before)), 0);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::After)), 0);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test visual::tests -v`

Expected: FAIL — `affinity_selects_a_side_at_a_bidi_run_boundary` returns the old values (6 vs 3 and 6 vs 3).

- [ ] **Step 3: Implement affinity-correct cursor resolution**

In `src/visual.rs`, add these methods and replace `cursor_visual`:

```rust
    pub fn is_rtl(&self, logical: usize) -> bool {
        self.levels()
            .get(logical)
            .is_some_and(|level| level % 2 == 1)
    }

    fn cell_edge_before(&self, cell: usize) -> usize {
        let visual = self.visual_of(cell);
        if self.is_rtl(cell) {
            (visual + 1).min(self.cols)
        } else {
            visual
        }
    }

    fn cell_edge_after(&self, cell: usize) -> usize {
        let visual = self.visual_of(cell);
        if self.is_rtl(cell) {
            visual
        } else {
            (visual + 1).min(self.cols)
        }
    }

    pub fn cursor_visual(&self, cursor: &CursorState) -> usize {
        if self.cols == 0 {
            return 0;
        }
        match cursor.affinity {
            Affinity::Before => {
                if cursor.logical < self.cols {
                    self.cell_edge_before(cursor.logical)
                } else {
                    self.cell_edge_after(self.cols - 1)
                }
            }
            Affinity::After => {
                if cursor.logical == 0 {
                    self.cell_edge_before(0)
                } else {
                    self.cell_edge_after((cursor.logical - 1).min(self.cols - 1))
                }
            }
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test visual::tests -v`

Expected: PASS — 14 tests pass.

Run: `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`

Expected: clean.

- [ ] **Step 5: Commit**

```powershell
git add src/visual.rs
git commit -m "fix: resolve cursor affinity at bidi run boundaries"
```

---

### Task 8: Selection, copy, and hit-testing through bidi maps

**Files:**
- Modify: `src/app.rs`
- Modify: `src/selection.rs`
- Test: `src/app.rs`, `src/selection.rs`, `src/visual.rs` (inline)

**Interfaces:**
- Consumes: `VisualMap::for_row`, `VisualMap::logical_at`, `Selection::{new, update, to_text}`.
- Produces:
  - `App::pos_at` uses the row's bidi map.
  - `app::logical_pos_at(row, visual, &RowSnapshot, &BidiConfig) -> LogicalPos` (private, testable).
  - Selection copy remains logical-order.

- [ ] **Step 1: Write the failing tests**

In `src/selection.rs` tests, add:

```rust
    fn arabic_snapshot() -> GridSnapshot {
        GridSnapshot {
            cols: 6,
            rows: 1,
            lines: vec![RowSnapshot {
                cells: ["a", "b", "c", "م", "ر", "ح"]
                    .iter()
                    .map(|text| Cell {
                        text: (*text).to_string(),
                        width: 1,
                        style: Style::default(),
                    })
                    .collect(),
                wrapped: false,
            }],
            cursor: CursorState {
                row: 0,
                logical: 0,
                affinity: Affinity::Before,
            },
            display_offset: 0,
        }
    }

    #[test]
    fn copy_returns_logical_order_for_an_rtl_row() {
        let mut selection = Selection::new(position(0, 3));
        selection.update(position(0, 5));
        assert_eq!(selection.to_text(&arabic_snapshot(), true), "مرح");
    }

    #[test]
    fn copy_returns_logical_order_for_a_mixed_row() {
        let mut selection = Selection::new(position(0, 0));
        selection.update(position(0, 5));
        assert_eq!(selection.to_text(&arabic_snapshot(), true), "abcمرح");
    }

    #[test]
    fn reverse_drag_still_copies_in_logical_order() {
        let mut selection = Selection::new(position(0, 5));
        selection.update(position(0, 3));
        assert_eq!(selection.to_text(&arabic_snapshot(), true), "مرح");
    }
```

In `src/visual.rs` tests, add:

```rust
    #[test]
    fn hit_testing_through_an_rtl_run_returns_logical_cells() {
        let map = VisualMap::from_bidi(
            &row(&["a", "b", "c", "م", "ر", "ح"]),
            BaseDirection::Ltr,
        );
        assert_eq!(map.logical_at(0), 0);
        assert_eq!(map.logical_at(2), 2);
        assert_eq!(map.logical_at(3), 5);
        assert_eq!(map.logical_at(4), 4);
        assert_eq!(map.logical_at(5), 3);
    }
```

In `src/app.rs` tests, add:

```rust
    use crate::config::BaseDirection;
    use crate::term::{Cell, RowSnapshot, Style};

    fn bidi_line() -> RowSnapshot {
        RowSnapshot {
            cells: ["a", "b", "c", "م", "ر", "ح"]
                .iter()
                .map(|text| Cell {
                    text: (*text).to_string(),
                    width: 1,
                    style: Style::default(),
                })
                .collect(),
            wrapped: false,
        }
    }

    #[test]
    fn logical_position_uses_the_row_bidi_map() {
        let line = bidi_line();
        let bidi = BidiConfig {
            enabled: true,
            base_direction: BaseDirection::Ltr,
        };
        assert_eq!(
            logical_pos_at(2, 5, &line, &bidi),
            LogicalPos { row: 2, col: 3 }
        );
    }

    #[test]
    fn disabled_bidi_hit_tests_identity() {
        let line = bidi_line();
        let bidi = BidiConfig {
            enabled: false,
            base_direction: BaseDirection::Ltr,
        };
        assert_eq!(
            logical_pos_at(0, 5, &line, &bidi),
            LogicalPos { row: 0, col: 5 }
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run:

```powershell
cargo test selection::tests -v
cargo test visual::tests -v
cargo test app::tests -v
```

Expected: FAIL to compile with `cannot find function logical_pos_at`. The selection and visual assertions are regression coverage for behavior that already exists; the build fails only because the app test module references the new helper.

- [ ] **Step 3: Replace the hit-testing path in `src/app.rs`**

Add `use crate::config::BidiConfig;` and `use crate::term::RowSnapshot;` to the imports.

Add this function next to `cell_at`:

```rust
fn logical_pos_at(
    row: usize,
    visual: usize,
    line: &RowSnapshot,
    bidi: &BidiConfig,
) -> LogicalPos {
    LogicalPos {
        row,
        col: VisualMap::for_row(line, bidi).logical_at(visual),
    }
}
```

Replace `fn pos_at`:

```rust
    fn pos_at(&self, x: f64, y: f64) -> Option<LogicalPos> {
        let renderer = self.renderer.as_ref()?;
        let terminal = self.terminal.as_ref()?;
        let (row, visual) = cell_at(
            x,
            y,
            renderer.padding() as f64,
            renderer.fonts().cell_w as f64,
            renderer.fonts().cell_h as f64,
            self.cols,
            self.rows,
        );
        let snapshot = terminal.snapshot();
        let line = snapshot.lines.get(row)?;
        Some(logical_pos_at(row, visual, line, &self.config.bidi))
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run:

```powershell
cargo test selection::tests -v
cargo test visual::tests -v
cargo test app::tests -v
```

Expected: PASS — selection has 8 tests, visual has 15, app has 9.

- [ ] **Step 5: Manual selection and copy check**

Run: `cargo run --release`

Type `echo "abc مرحبا 123"`. With the mouse, drag from the right side of the Arabic word to its left side. Press `Ctrl+Shift+C`, then paste into Notepad or another editor. Expected: the clipboard contains the Arabic in logical order (`مرحبا`), not the visual order, and the selection highlight covers the same cell slots you dragged over.

Click into the middle of the Arabic word, type a Latin letter, then Backspace. Expected: the edit lands at the logical position under the click and the rest of the row stays aligned.

- [ ] **Step 6: Commit**

```powershell
git add src/app.rs src/selection.rs src/visual.rs
git commit -m "feat: route selection and hit-testing through bidi maps"
```

---

### Task 9: Integration, acceptance, and documentation

**Files:**
- Modify: `README.md`
- Test: manual acceptance matrix (no automated golden images in this phase)

**Interfaces:**
- Consumes: everything from Tasks 1–8.
- Produces: a release build that renders Arabic, Persian, and Urdu joined and correctly ordered; documented limitations.

- [ ] **Step 1: Run the full automated gate**

Run:

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Expected: fmt and clippy clean; all tests pass; the release build succeeds.

- [ ] **Step 2: Run the Windows UTF-8 and RTL acceptance matrix**

Run: `cargo run --release`

In PowerShell, run each command and check the result:

```powershell
echo "مرحبا بالعالم"
echo "سلام دنیا"
echo "السلام علیکم"
echo "test 123 مرحبا 456"
echo "لا"
echo "مُحَمَّد"
```

Expected:
- Arabic, Persian, and Urdu words are joined (no disconnected letter forms).
- Each Arabic sentence reads right-to-left inside its row.
- Latin words and digits keep left-to-right order; an embedded number stays with its adjacent Latin or neutral text and does not reverse the whole row.
- `لا` draws as one lam-alef glyph across two cells; the cell after it is not shifted.
- Harakat sit on their base letters and do not create extra cells.
- The prompt stays at the left margin with the default `ltr` base direction.

- [ ] **Step 3: Resize and scrollback acceptance**

With the Arabic output on screen, drag the window narrower and wider. Expected: each row stays aligned to the cell grid, no glyph spills into the next row, and Arabic words that wrap are ordered per row (this is the documented per-row limitation).

Scroll up with the mouse wheel and back down. Expected: no panic, no misalignment.

- [ ] **Step 4: Selection and copy acceptance**

Select each of the six lines from Step 2 with the mouse, press `Ctrl+Shift+C`, and paste into a UTF-8 editor. Expected: the pasted text matches the logical input exactly, including the lam-alef pair and the harakat.

- [ ] **Step 5: Update `README.md`**

Replace the status paragraph and add a Phase 2 section:

````markdown
# Termyx

A fast, native terminal emulator with first-class RTL support.

Phase 2 adds per-row UAX #9 bidi reordering, Arabic/Persian/Urdu shaping with
rustybuzz, Arabic-capable font fallback, and bidi-aware cursor, selection, and
hit-testing. The grid stays logical; a per-row `VisualMap` performs all visual
ordering.

## Build

```bash
cargo build --release
cargo run --release
```

Windows requires the MSVC build tools. Linux requires `libxkbcommon`,
`libwayland`, and `libx11-xcb` development packages.

## Configuration

Pass `--config <path>` to use a specific file. The default path is:

- Windows: `%APPDATA%\termyx\config.toml`
- Linux: `$XDG_CONFIG_HOME/termyx/config.toml` (or `~/.config/...`)
- macOS: `~/Library/Application Support/termyx/config.toml`

A missing default file is fine; built-in defaults are used.

## RTL configuration

```toml
[font]
family = "Cascadia Mono"
arabic_family = "Kawkab Mono"

[bidi]
enabled = true
base_direction = "ltr"   # ltr | rtl | auto
```

- `ltr` (default) keeps the prompt pinned to the left and embeds RTL runs.
- `rtl` right-anchors the whole row.
- `auto` follows the first strong character of each row.
- `enabled = false` disables reordering; Arabic is still shaped, but mixed rows
  are not reordered.

No Arabic font binary is bundled yet. When `font.arabic_family` is missing,
the terminal logs a warning and uses the first installed Arabic-capable face
from Segoe UI, Arial, Tahoma, Noto Naskh Arabic, Noto Sans Arabic, DejaVu Sans,
or FreeSerif. See `assets/fonts/README.md`.

## Phase 2 acceptance

Run on Windows 10/11 x64 with PowerShell:

```powershell
echo "مرحبا بالعالم"
echo "سلام دنیا"
echo "السلام علیکم"
echo "test 123 مرحبا 456"
echo "لا"
echo "مُحَمَّد"
```

Expected: joined Arabic/Persian/Urdu, right-to-left sentence order, Latin and
digits in logical order, lam-alef across two cells with no shift, harakat on
their base cells, and the prompt pinned left in the default mode. Selection and
`Ctrl+Shift+C` copy the logical text.

## Known limitations

- Bidi is per row. A soft-wrapped RTL paragraph is ordered row by row.
- Joining is not preserved across a bidi direction boundary (for example, a
  Latin number embedded inside an Arabic word).
- A caret or selection boundary inside a two-cell lam-alef draws at the cell
  boundary; the ligature is still drawn once.
- The Kawkab Mono asset is not bundled in this phase; system fallback is used.
- Tabs, splits, profiles, search, images, application mouse reporting, IME
  preedit, and AI are later phases.
````

- [ ] **Step 6: Commit the acceptance documentation**

```powershell
git add README.md
git commit -m "docs: record Phase 2 acceptance and Arabic limitations"
```

---

## Phase 2 scope

- **In:** per-row bidi, shaping, Arabic font fallback, shaped-cluster rendering, cursor affinity, bidi-aware selection/copy/hit-testing.
- **Out:** tabs, splits, profiles, command palette, search, AI, images, application mouse reporting (DEC modes), IME preedit, block selection, a bundled Kawkab Mono binary, and a persistent shaped-run cache.
- **Prerequisite that may land separately:** the Kawkab Mono asset and its OFL license. Until then the system Arabic fallback is the supported path.
- **Documented limitation:** paragraph bidi across soft-wrapped rows is impossible in a cell terminal without application cooperation; each row is its own paragraph.

## Self-Review

- Spec coverage: §7.1 cell/width rules → Tasks 1, 4, 6; §7.2 bidi reordering → Tasks 1, 2; §7.3 shaping → Task 4; §7.4 font selection → Task 5; §7.5 base direction → Task 3; §7.6 cursor → Task 7; §7.7 selection/copy → Task 8; §8 renderer → Task 6; §12 unit tests → inline tests per task; §13 M2/M3 acceptance → Tasks 6, 8, 9; §16 dependencies → Tasks 1, 4, 5.
- Placeholder scan: no TBD/TODO/"similar to Task N"/unspecified error-handling steps. Every code step contains the code to write.
- Type consistency: `RowText` fields are used by `analyze` and `shape_row`; `VisualRun` is defined once in `bidi.rs` and reused by `VisualMap` and `shape.rs`; `FontSet::load(family, arabic_family, size, line_height)` and `font_for(bool)` are updated at every call site; `GlyphCache::get(font_id, font, glyph, size_px)` is updated in the cache tests; `Faces::new`/`Faces::from_fonts` match their call sites; `cursor_visual` is the only cursor resolver.
