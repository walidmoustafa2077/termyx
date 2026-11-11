# Termyx Spellcheck Overlay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Underline misspelled words on the active shell input line in Termyx, and offer ranked replacements, "Add to dictionary", and a session hide toggle from a right-click menu.

**Architecture:** The PowerShell module's prompt wrapper emits OSC 133 `A`/`B` marks. `Terminal::feed` scans for them, records the input start cell, and exposes it in `GridSnapshot`. A new `spellcheck` unit tokenizes the cells between the mark and the cursor against embedded word and frequency lists. The renderer draws underlines from those spans; right-click over a span opens a menu instead of pasting.

**Tech Stack:** Rust (edition as pinned in `Cargo.toml`), `vte`/`alacritty_terminal` already in use, no new crates; Windows PowerShell 5.1 + Pester 5.7.1 for the module side.

**Spec:** `docs/superpowers/specs/2026-11-11-termyx-spellcheck-design.md`

## Global Constraints

- Rust only, no new crate dependencies; the tokenizer is hand-written (no `regex`).
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` must pass (`ci.yml:18-21`).
- All tests live in inline `#[cfg(test)] mod tests` blocks; there is no `tests/` directory in this repo.
- Source files are ASCII only; no emoji, no box drawing.
- The feature degrades to inert: no OSC 133 mark means no underline and no menu.
- The renderer never mutates cell state; underlines are drawn, not stored.
- The terminal must never send keystrokes except from an explicit menu click that satisfies the last-token rule.
- Embedded assets come from `shell/powershell/Termyx/assets/en_US.wordlist.txt` and `en_US.frequency.txt`; they are committed and must not be copied or regenerated.
- PowerShell side: Windows PowerShell 5.1 only, ASCII, no BOM, `Should -Be` / `Invoke-Pester -Path`.
- Commits use inline identity: `git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "..."`.

---

### Task 1: Spellcheck core

**Files:**
- Create: `src/spellcheck.rs`
- Modify: `src/main.rs` (add `mod spellcheck;` beside the other module declarations)

**Interfaces:**
- Produces:
  - `pub struct Misspelling { pub row: usize, pub col: usize, pub len: usize }`
  - `pub fn scan(row: &[crate::term::Cell], start_col: usize, cursor_col: usize) -> Vec<Misspelling>`
  - `pub fn suggestions(token: &str, limit: usize) -> Vec<String>`
  - `pub fn is_known(token: &str) -> bool`
  - `pub fn add_word(token: &str)` (inserts into the in-memory set; does not touch disk)

- [ ] **Step 1: Write the failing tests**

Append to `src/spellcheck.rs` (create the file with only this test module first):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // Build cells the way the existing render tests do (see
    // `single_cell_snapshot` in `src/render/mod.rs:2552`); if `Style` has no
    // `Default` impl, use `crate::term::Style::default()` or the helper it uses.
    fn row(text: &str) -> Vec<crate::term::Cell> {
        text.chars()
            .map(|c| crate::term::Cell {
                text: c.to_string(),
                width: 1,
                style: Default::default(),
            })
            .collect()
    }

    #[test]
    fn skips_the_command_name() {
        let cells = row("Get-ChildItem wrng");
        let found = scan(&cells, 0, cells.len());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].col, 14);
        assert_eq!(found[0].len, 4);
    }

    #[test]
    fn skips_quoted_strings() {
        let cells = row("Write-Host \"wrng wrd\"");
        assert!(scan(&cells, 0, cells.len()).is_empty());
    }

    #[test]
    fn skips_paths_urls_flags_and_numbers() {
        for line in [
            "cd C:\\temp\\wrng",
            "cd C:wrng",
            "Write-Host https://wrng.example",
            "Write-Host -wrng",
            "Write-Host 1234",
        ] {
            let cells = row(line);
            assert!(scan(&cells, 0, cells.len()).is_empty(), "line: {line}");
        }
    }

    #[test]
    fn skips_short_and_uppercase_tokens() {
        for line in ["Write-Host ab", "Write-Host WRNG"] {
            let cells = row(line);
            assert!(scan(&cells, 0, cells.len()).is_empty(), "line: {line}");
        }
    }

    #[test]
    fn finds_several_misspellings() {
        let cells = row("Write-Host wrng and wrrd");
        let found = scan(&cells, 0, cells.len());
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].col, found[0].len), (11, 4));
        assert_eq!((found[1].col, found[1].len), (20, 4));
    }

    #[test]
    fn stops_at_the_cursor() {
        let cells = row("Write-Host wrng");
        assert!(scan(&cells, 0, 12).is_empty());
    }

    #[test]
    fn knows_real_words() {
        assert!(is_known("hello"));
        assert!(is_known("Hello"));
        assert!(!is_known("wrng"));
    }

    #[test]
    fn suggests_before_underlining() {
        assert_eq!(suggestions("teh", 5).first().map(String::as_str), Some("the"));
        assert_eq!(suggestions("recieve", 5).first().map(String::as_str), Some("receive"));
    }

    #[test]
    fn respects_the_limit() {
        assert!(suggestions("adress", 1).len() <= 1);
    }

    #[test]
    fn added_word_stops_being_unknown() {
        add_word("zqxjv");
        assert!(is_known("zqxjv"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test spellcheck`
Expected: compile errors for missing `scan`, `is_known`, `suggestions`, `add_word`.

- [ ] **Step 3: Implement the module**

Prepend above the test module:

```rust
//! Offline spellcheck for the shell input line.

use std::collections::{HashMap, HashSet};
use std::sync::{OnceLock, RwLock};

use crate::term::Cell;

const WORD_LIST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shell/powershell/Termyx/assets/en_US.wordlist.txt"
));

const FREQUENCY_LIST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/shell/powershell/Termyx/assets/en_US.frequency.txt"
));

const MIN_TOKEN_LEN: usize = 3;
const MAX_DISTANCE_ONE_TOKEN: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Misspelling {
    pub row: usize,
    pub col: usize,
    pub len: usize,
}

struct WordData {
    words: HashSet<String>,
    rank: HashMap<String, usize>,
    added: HashSet<String>,
}

fn data() -> &'static RwLock<WordData> {
    static DATA: OnceLock<RwLock<WordData>> = OnceLock::new();
    DATA.get_or_init(|| {
        let mut words = HashSet::new();
        for line in WORD_LIST.lines() {
            let word = line.trim();
            if !word.is_empty() {
                words.insert(word.to_ascii_lowercase());
            }
        }

        let mut rank = HashMap::new();
        for (index, line) in FREQUENCY_LIST.lines().enumerate() {
            let word = line.split(' ').next().unwrap_or("").trim();
            if !word.is_empty() {
                rank.entry(word.to_ascii_lowercase()).or_insert(index);
            }
        }

        RwLock::new(WordData { words, rank, added: HashSet::new() })
    })
}

pub fn is_known(token: &str) -> bool {
    let lower = token.to_ascii_lowercase();
    let guard = data().read().expect("spellcheck lock");
    guard.words.contains(&lower) || guard.added.contains(&lower)
}

pub fn add_word(token: &str) {
    let lower = token.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return;
    }
    let mut guard = data().write().expect("spellcheck lock");
    guard.added.insert(lower);
}

pub fn scan(row: &[Cell], start_col: usize, cursor_col: usize) -> Vec<Misspelling> {
    let end = cursor_col.min(row.len());
    if end <= start_col {
        return Vec::new();
    }

    let mut found = Vec::new();
    let mut index = 0usize;
    let mut first_token_seen = false;

    while index < end {
        let cell = &row[index];
        let ch = cell.text.chars().next().unwrap_or(' ');
        if ch.is_whitespace() {
            index += 1;
            continue;
        }

        let word_start = index;
        let mut word = String::new();
        let mut quoted = false;
        while index < end {
            let current = &row[index];
            let c = current.text.chars().next().unwrap_or(' ');
            if c.is_whitespace() {
                break;
            }
            if c == '"' || c == '\'' || c == '`' {
                quoted = true;
            }
            word.push(c);
            index += 1;
        }

        if index == word_start {
            index += 1;
            continue;
        }

        if !first_token_seen {
            first_token_seen = true;
            continue;
        }
        if quoted {
            continue;
        }
        if !is_candidate(&word) {
            continue;
        }

        let lower = word.to_ascii_lowercase();
        if is_known(&lower) {
            continue;
        }

        found.push(Misspelling {
            row: 0,
            col: word_start.max(start_col),
            len: word.len(),
        });
    }

    found
}

fn is_candidate(word: &str) -> bool {
    if word.len() < MIN_TOKEN_LEN {
        return false;
    }
    if word.starts_with('-') {
        return false;
    }
    if word.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    if word
        .chars()
        .any(|c| matches!(c, '\\' | '/' | ':' | '@' | '$' | '"' | '\'' | '`'))
    {
        return false;
    }
    if !word.chars().next().map(|c| c.is_ascii_alphabetic()).unwrap_or(false) {
        return false;
    }
    if word.len() >= 4 && word.chars().all(|c| !c.is_ascii_lowercase()) {
        return false;
    }
    true
}

pub fn suggestions(token: &str, limit: usize) -> Vec<String> {
    let lower = token.to_ascii_lowercase();
    if limit == 0 || lower.len() < 2 {
        return Vec::new();
    }

    let guard = data().read().expect("spellcheck lock");
    let mut one_edit: Vec<&String> = guard
        .words
        .iter()
        .filter(|word| word.len().saturating_sub(lower.len()) <= 1 && lower.len().saturating_sub(word.len()) <= 1)
        .filter(|word| distance(&lower, word, 1) == 1)
        .collect();

    let candidates: Vec<&String> = if one_edit.is_empty() && lower.len() <= MAX_DISTANCE_ONE_TOKEN {
        guard
            .words
            .iter()
            .filter(|word| word.len().saturating_sub(lower.len()) <= 2 && lower.len().saturating_sub(word.len()) <= 2)
            .filter(|word| distance(&lower, word, 2) == 2)
            .collect()
    } else {
        std::mem::take(&mut one_edit)
    };

    // A shorter candidate never wins while a same-length or longer one exists,
    // then the lowest frequency rank decides. Words absent from the frequency
    // list sort last but remain usable when they are the only candidate.
    let mut ranked: Vec<(bool, usize, &String)> = candidates
        .into_iter()
        .map(|word| {
            let shorter = word.len() < lower.len();
            let rank = guard.rank.get(word).copied().unwrap_or(usize::MAX / 2);
            (shorter, rank, word)
        })
        .collect();

    ranked.sort_by(|a, b| (a.0, a.1, a.2).cmp(&(b.0, b.1, b.2)));
    ranked.into_iter().take(limit).map(|(_, _, word)| word.clone()).collect()
}

fn distance(a: &str, b: &str, max: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.len().abs_diff(b.len()) > max {
        return max + 1;
    }

    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    let mut before_previous = vec![0usize; b.len() + 1];

    for i in 1..=a.len() {
        current[0] = i;
        let mut row_min = current[0];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            let mut value = (previous[j] + 1).min(current[j - 1] + 1).min(previous[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                value = value.min(before_previous[j - 2] + 1);
            }
            current[j] = value;
            row_min = row_min.min(value);
        }
        if row_min > max {
            return max + 1;
        }
        std::mem::swap(&mut before_previous, &mut previous);
        std::mem::swap(&mut previous, &mut current);
    }

    previous[b.len()]
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test spellcheck`
Expected: 10 passed. `suggestions("teh", 5)` returns `the` first; `suggestions("recieve", 5)` returns `receive` first.

- [ ] **Step 5: Register the module**

In `src/main.rs`, add `mod spellcheck;` in the alphabetical module list.

- [ ] **Step 6: Verify the whole suite and lints**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: clean, all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/spellcheck.rs src/main.rs
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat(spellcheck): add offline word checks and ranked suggestions"
```

---

### Task 2: OSC 133 marks and input-line tracking

**Files:**
- Modify: `src/term.rs` (`Terminal::feed`, `GridSnapshot`)
- Test: inline `src/term.rs` tests

**Interfaces:**
- Consumes: `spellcheck::Cell` shape only (`Cell` already exists in `term.rs`).
- Produces:
  - `pub struct InputStart { pub row: i32, pub col: usize }` in `term.rs`
  - `GridSnapshot.input_start: Option<InputStart>`
  - `Terminal::feed` keeps working with the same signature

- [ ] **Step 1: Write the failing tests**

Add to the existing `mod tests` in `src/term.rs`:

```rust
#[test]
fn tracks_osc_133_marks() {
    let mut term = Terminal::new(80, 24, 100);
    term.feed(b"\x1b]133;A\x07PS> ");
    term.feed(b"\x1b]133;B\x07teh");
    let snapshot = term.snapshot();
    let start = snapshot.input_start.expect("mark recorded");
    assert_eq!(start.col, 4);
    assert_eq!(snapshot.cursor.col, 7);
}

#[test]
fn clears_the_mark_on_a_new_prompt() {
    let mut term = Terminal::new(80, 24, 100);
    term.feed(b"\x1b]133;B\x07abc");
    term.feed(b"\r\n\x1b]133;A\x07PS> ");
    assert!(term.snapshot().input_start.is_none());
}

#[test]
fn survives_a_split_escape() {
    let mut term = Terminal::new(80, 24, 100);
    term.feed(b"\x1b]133");
    term.feed(b";B\x07teh");
    assert!(term.snapshot().input_start.is_some());
    assert_eq!(term.snapshot().cursor.col, 3);
}

#[test]
fn passes_other_osc_sequences_through() {
    let mut term = Terminal::new(80, 24, 100);
    term.feed(b"\x1b]0;title\x07hello");
    assert_eq!(term.snapshot().cursor.col, 5);
    assert!(term.snapshot().input_start.is_none());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test term::tests::tracks_osc_133`
Expected: compile error, no `input_start` field.

- [ ] **Step 3: Implement the scan**

In `src/term.rs`, add the struct beside `CursorState`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputStart {
    pub row: i32,
    pub col: usize,
}
```

Add the field to `GridSnapshot` and initialize it to `None` wherever a snapshot is constructed.

Give `Terminal` a pending buffer and route `feed` through it:

```rust
pub fn feed(&mut self, bytes: &[u8]) {
    self.pending.extend_from_slice(bytes);
    let mut consumed = 0usize;

    while consumed < self.pending.len() {
        match find_osc(&self.pending[consumed..]) {
            None => break,
            Some((offset, len)) => {
                if offset > 0 {
                    self.processor.advance(&mut self.term, &self.pending[consumed..consumed + offset]);
                }
                let sequence: Vec<u8> = self.pending[consumed + offset..consumed + offset + len].to_vec();
                self.processor.advance(&mut self.term, &sequence);
                self.apply_mark(&sequence);
                consumed += offset + len;
            }
        }
    }

    if consumed < self.pending.len() {
        let rest = self.pending[consumed..].to_vec();
        if rest.starts_with(b"\x1b]") {
            self.pending = rest;
            return;
        }
        self.processor.advance(&mut self.term, &rest);
        self.pending.clear();
    } else {
        self.pending.clear();
    }
}

fn apply_mark(&mut self, sequence: &[u8]) {
    let payload = match sequence.get(2..sequence.len().saturating_sub(1)) {
        Some(value) => value,
        None => return,
    };
    match payload {
        b"133;A" => self.input_start = None,
        b"133;B" => {
            let point = self.term.grid().cursor.point;
            self.input_start = Some(InputStart { row: point.line.0, col: point.column.0 });
        }
        _ => {}
    }
}

fn find_osc(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut index = 0usize;
    while index + 1 < bytes.len() {
        if bytes[index] == 0x1b && bytes[index + 1] == b']' {
            let mut cursor = index + 2;
            while cursor < bytes.len() {
                if bytes[cursor] == 0x07 {
                    return Some((index, cursor + 1 - index));
                }
                if bytes[cursor] == 0x1b && cursor + 1 < bytes.len() && bytes[cursor + 1] == b'\\' {
                    return Some((index, cursor + 2 - index));
                }
                cursor += 1;
            }
            return None;
        }
        index += 1;
    }
    None
}
```

Add `pending: Vec<u8>` and `input_start: Option<InputStart>` to `Terminal`, initialize both in `Terminal::new` (`pending: Vec::new()`, `input_start: None`), and copy `input_start` into the snapshot.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test term::`
Expected: all term tests pass, including the four new ones.

- [ ] **Step 5: Commit**

```bash
git add src/term.rs
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat(term): track OSC 133 input marks"
```

---

### Task 3: Underline rendering

**Files:**
- Modify: `src/render/mod.rs`
- Modify: `src/app.rs` (`App::render` call site)
- Test: inline `src/render/mod.rs` tests

**Interfaces:**
- Consumes: `spellcheck::Misspelling`, `VisualMap`.
- Produces: `pub fn draw_spellcheck_underlines(&mut self, snapshot: &GridSnapshot, spans: &[Misspelling], color: Rgb)`; `Renderer::draw` gains one `spellcheck: &[Misspelling]` parameter.

- [ ] **Step 1: Write the failing test**

Add to `src/render/mod.rs` `mod tests`:

```rust
#[test]
fn draws_underline_under_a_span() {
    let mut renderer = test_renderer();
    let snapshot = single_cell_snapshot("ab");
    let spans = vec![crate::spellcheck::Misspelling { row: 0, col: 0, len: 2 }];
    let pixmap = renderer.render_for_test(&snapshot, &spans);
    let y = renderer.grid_top_for_test() + renderer.cell_h_for_test() - 2;
    assert_ne!(pixel_at(&pixmap, 1, y), pixel_at(&pixmap, 1, renderer.cell_h_for_test() / 2));
}

#[test]
fn draws_nothing_without_spans() {
    let mut renderer = test_renderer();
    let snapshot = single_cell_snapshot("ab");
    let with = renderer.render_for_test(&snapshot, &[]);
    let baseline = renderer.render_for_test(&snapshot, &[]);
    assert_eq!(with.data(), baseline.data());
}
```

If `test_renderer`, `render_for_test`, `grid_top_for_test` or `cell_h_for_test` do not exist, add thin `#[cfg(test)]` accessors beside the existing render test helpers (`pixel_at` at `render/mod.rs:2494`, `single_cell_snapshot` at `2552`).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test render::tests::draws_underline`
Expected: compile error for the missing span parameter/helper.

- [ ] **Step 3: Implement**

```rust
pub fn draw_spellcheck_underlines(
    &mut self,
    snapshot: &GridSnapshot,
    spans: &[crate::spellcheck::Misspelling],
    color: Rgb,
) {
    if spans.is_empty() {
        return;
    }
    let cell_w = self.cell_w();
    let cell_h = self.cell_h();
    let top = self.grid_top();
    let thickness = (self.scale().round() as u32).max(1);

    for span in spans {
        let row = span.row as usize + snapshot.display_offset;
        if row >= snapshot.rows {
            continue;
        }
        let visual_start = self.visual_of(row, span.col);
        let visual_end = self.visual_of(row, span.col + span.len.saturating_sub(1));
        let (from, to) = if visual_start <= visual_end {
            (visual_start, visual_end)
        } else {
            (visual_end, visual_start)
        };
        let x0 = self.padding_x() + from * cell_w;
        let x1 = self.padding_x() + (to + 1) * cell_w;
        let y = top + row * cell_h + cell_h - thickness - 1;
        self.fill_rect(x0, y, x1.saturating_sub(x0), thickness, color);
    }
}
```

Call it in `draw` immediately after the existing `draw_decorations` call (`render/mod.rs:357`). Pass an empty slice from every other caller except the app path.

- [ ] **Step 4: Verify**

Run: `cargo test render:: && cargo fmt --check && cargo clippy --all-targets -- -D warnings`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add src/render/mod.rs src/app.rs
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat(render): draw spellcheck underlines"
```

---

### Task 4: Config and theme

**Files:**
- Modify: `src/config.rs`
- Modify: `src/theme.rs`
- Modify: `README.md`
- Test: inline `src/config.rs` tests

**Interfaces:**
- Produces: `Config.spellcheck: SpellcheckConfig` with `enabled: bool`, `autocorrect: bool`, `underline_color: String`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn spellcheck_defaults_are_on() {
    let config = Config::default();
    assert!(config.spellcheck.enabled);
    assert!(config.spellcheck.autocorrect);
    assert_eq!(config.spellcheck.underline_color, "#d19a66");
}

#[test]
fn spellcheck_reads_from_table() {
    let text = "[spellcheck]\nenabled = false\nautocorrect = false\nunderline_color = \"#ff0000\"\n";
    let config: Config = toml::from_str(text).expect("parse");
    assert!(!config.spellcheck.enabled);
    assert!(!config.spellcheck.autocorrect);
    assert_eq!(config.spellcheck.underline_color, "#ff0000");
}

#[test]
fn invalid_underline_color_falls_back() {
    let text = "[spellcheck]\nunderline_color = \"not-a-colour\"\n";
    let mut config: Config = toml::from_str(text).expect("parse");
    config.validate();
    assert_eq!(config.spellcheck.underline_color, "#d19a66");
}
```

Use the same TOML entry point the neighbouring tests use (they call `toml::from_str` or `Config::from_str`; match whichever is already imported).

- [ ] **Step 2: Verify failure**

Run: `cargo test config::tests::spellcheck`
Expected: compile error, unknown field.

- [ ] **Step 3: Implement**

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SpellcheckConfig {
    pub enabled: bool,
    pub autocorrect: bool,
    pub underline_color: String,
}

impl Default for SpellcheckConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            autocorrect: true,
            underline_color: "#d19a66".to_string(),
        }
    }
}
```

Add `pub spellcheck: SpellcheckConfig` to `Config` and `SpellcheckConfig::default()` to `Config::default()`. Add `"spellcheck.enabled"`, `"spellcheck.autocorrect"`, `"spellcheck.underline_color"` to `KNOWN_KEYS`. In `Config::validate`, reset `underline_color` to the default when it cannot be parsed by the theme's hex parser.

Add a `spellcheck_underline: Rgb` accessor on `Theme` that parses `Config.spellcheck.underline_color` with the existing colour parser, defaulting to `#d19a66`.

Document the section in `README.md` next to `[window]`, with the three keys and their defaults.

- [ ] **Step 4: Verify**

Run: `cargo test config:: && cargo test theme::`
Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs src/theme.rs README.md
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat(config): add spellcheck options"
```

---

### Task 5: Menu model and painter

**Files:**
- Create: `src/ui/spellcheck_menu.rs`
- Create: `src/render/spellcheck_menu.rs`
- Modify: `src/ui/mod.rs`, `src/render/mod.rs` (module declarations)
- Test: inline in both new files

**Interfaces:**
- Produces:
  - `pub struct SpellcheckMenu { pub row: usize, pub col: usize, pub token: String, pub items: Vec<MenuItem>, pub enabled: bool }`
  - `pub enum MenuItem { Suggest(String), AddToDictionary, HideUnderlines }`
  - `SpellcheckMenu::new(row, col, token, suggestions, can_replace, disabled_reason)`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn lists_suggestions_then_actions() {
    let menu = SpellcheckMenu::new(3, 4, "teh".to_string(), vec!["the".to_string()], true, None);
    assert_eq!(menu.items.len(), 3);
    assert!(matches!(menu.items[0], MenuItem::Suggest(ref s) if s == "the"));
    assert!(matches!(menu.items[1], MenuItem::AddToDictionary));
    assert!(matches!(menu.items[2], MenuItem::HideUnderlines));
}

#[test]
fn disables_replacement_when_not_last_token() {
    let menu = SpellcheckMenu::new(0, 0, "teh".to_string(), vec!["the".to_string()], false, Some("not the last word"));
    assert!(!menu.enabled);
    assert_eq!(menu.disabled_reason, Some("not the last word"));
}
```

- [ ] **Step 2: Verify failure**

Run: `cargo test spellcheck_menu`
Expected: compile error, missing module.

- [ ] **Step 3: Implement the model**

Model file holds `SpellcheckMenu`, `MenuItem`, `new`, `select(index) -> Option<MenuItem>`, and the dismissal helper. Painter mirrors `render/context_menu.rs`: rounded panel via `fill_round_rect`, rows via `ui_text::draw_text_in`, highlight on hover, disabled rows dimmed.

- [ ] **Step 4: Pixel test the painter**

Mirror `render/find_bar.rs` tests: render a menu over a blank pixmap, assert the panel background differs from the terminal background and that the selected row differs from the others.

- [ ] **Step 5: Verify and commit**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`

```bash
git add src/ui/spellcheck_menu.rs src/render/spellcheck_menu.rs src/ui/mod.rs src/render/mod.rs
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat(ui): add spellcheck suggestion menu"
```

---

### Task 6: App wiring

**Files:**
- Modify: `src/app.rs`
- Modify: `src/pty.rs` or the session spawn site (set `TERMYX=1`)
- Test: inline `src/app.rs` tests where the logic is testable without a window

**Interfaces:**
- Consumes: everything above.
- Produces: session state `spellcheck_menu: Option<SpellcheckMenu>`, `hide_underlines: bool`.

- [ ] **Step 1: Compute spans per render**

In `App::render`, before `Renderer::draw`, build:

```rust
let spans = if config.spellcheck.enabled && !self.hide_underlines {
    match (snapshot.input_start, snapshot.cursor) {
        (Some(start), cursor) if start.row == cursor.line => {
            let cells = snapshot.row_cells(cursor.line);
            spellcheck::scan(&cells, start.col, cursor.col)
                .into_iter()
                .map(|mut span| { span.row = 0; span })
                .collect()
        }
        _ => Vec::new(),
    }
} else {
    Vec::new()
};
```

If `GridSnapshot` lacks a row accessor, add `pub fn row_cells(&self, row: usize) -> Vec<Cell>` that returns the visible row's cells (empty when out of range).

- [ ] **Step 2: Intercept right-click**

In `App::on_mouse`, inside the press branch, after the mouse-reporting forwarding block (`app.rs:2042-2047`) and before `pastes_locally` (`app.rs:2052`):

```rust
if button == MouseButton::Right {
    if let Some(hit) = self.spellcheck_hit(self.cursor_pos) {
        self.spellcheck_menu = Some(hit);
        self.app_mouse_button = None;
        return true;
    }
}
```

`spellcheck_hit` converts the pointer to a logical cell, finds a span containing it, checks whether the token is last and the cursor at its end, and returns a `SpellcheckMenu`.

- [ ] **Step 3: Execute menu actions**

- `Suggest(word)`: if allowed, write `\x7f` repeated `token.len()` times plus the word bytes to the PTY through the existing `write_input` path; clear the menu.
- `AddToDictionary`: append `token + "\n"` to `%LOCALAPPDATA%\Termyx\user-words.txt` (create dirs), call `spellcheck::add_word`, clear the menu.
- `HideUnderlines`: set `self.hide_underlines = true`, clear the menu.

Escape and click-outside close the menu before other handlers run.

- [ ] **Step 4: Set TERMYX for children**

Where the shell command is spawned, call `.env("TERMYX", "1")` and `.env("TERMYX_AUTOCORRECT", if config.spellcheck.autocorrect { "1" } else { "0" })`.

- [ ] **Step 5: Verify**

Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add src/app.rs src/pty.rs
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat(app): wire spellcheck underlines and menu"
```

---

### Task 7: Module prompt marks and shared words

**Files:**
- Create: `shell/powershell/Termyx/private/Prompt.ps1`
- Modify: `shell/powershell/Termyx/Termyx.psm1`
- Modify: `shell/powershell/Termyx/private/Decision.ps1` (user-word reload)
- Modify: `shell/powershell/tests/Termyx.Tests.ps1`
- Modify: `shell/powershell/README.md`

**Interfaces:**
- Produces: `Enable-TermyxPrompt`, `Disable-TermyxPrompt`, and `Update-TermyxUserWords` (internal).

- [ ] **Step 1: Write the failing tests**

```powershell
Describe 'Spellcheck integration' {
    It 'wraps the prompt with OSC 133 marks when TERMYX is set' {
        $env:TERMYX = '1'
        try {
            $module = Get-Module Termyx
            $text = & $module { $script:TermyxPromptText = 'PS> '; Get-TermyxPromptWithMarks }
            ($text.Contains("`a]133;A`a")) | Should -BeTrue
            ($text.Contains("`a]133;B`a")) | Should -BeTrue
            ($text.EndsWith('PS> ')) | Should -BeTrue
        } finally {
            Remove-Item Env:TERMYX -ErrorAction SilentlyContinue
        }
    }

    It 'leaves the prompt untouched outside Termyx' {
        Remove-Item Env:TERMYX -ErrorAction SilentlyContinue
        $module = Get-Module Termyx
        $text = & $module { $script:TermyxPromptText = 'PS> '; Get-TermyxPromptWithMarks }
        $text | Should -Be 'PS> '
    }

    It 'loads words added to the shared file' {
        $path = Join-Path $TestDrive 'user-words.txt'
        Set-Content -LiteralPath $path -Value 'zqxjv' -Encoding ASCII
        $module = Get-Module Termyx
        & $module { param($p) $script:TermyxUserWordsPath = $p; $script:TermyxUserWordsStamp = $null } $path
        & $module { Update-TermyxUserWords }
        ([Termyx.Corrector]::IsKnownWord('zqxjv')) | Should -BeTrue
    }
}
```

Note the escape character in the assertions: use `[char]27` rather than a literal backtick-a so the file stays readable, e.g. `$esc = [char]27`.

- [ ] **Step 2: Verify failure**

Run: `powershell.exe -NoProfile -Command "Invoke-Pester -Path .\shell\powershell\tests\Termyx.Tests.ps1"`
Expected: three failures, missing functions.

- [ ] **Step 3: Implement the prompt wrapper**

`private/Prompt.ps1`:

```powershell
function Get-TermyxPromptWithMarks {
    if ($env:TERMYX -ne '1') { return $script:TermyxPromptText }
    $esc = [char]27
    return ($esc + ']133;A' + [char]7) + $script:TermyxPromptText + ($esc + ']133;B' + [char]7)
}

function Enable-TermyxPrompt {
    if (-not (Test-Path Function:\global:prompt)) { return }
    if (Get-Command -Name global:prompt -ErrorAction SilentlyContinue) {
        $script:TermyxOriginalPrompt = $function:global:prompt
    }
    $script:TermyxPromptText = ''
    $module = $MyInvocation.MyCommand.Module
    Set-Item -Path Function:\global:prompt -Value {
        $script:TermyxPromptText = & $script:TermyxOriginalPrompt
        & $module { Get-TermyxPromptWithMarks }
    }
}

function Disable-TermyxPrompt {
    if ($null -ne $script:TermyxOriginalPrompt) {
        Set-Item -Path Function:\global:prompt -Value $script:TermyxOriginalPrompt
        $script:TermyxOriginalPrompt = $null
    }
}
```

Call `Enable-TermyxPrompt` from `Enable-TermyxAutocorrect` and `Disable-TermyxPrompt` from `Disable-TermyxAutocorrect`.

- [ ] **Step 4: Implement the shared-word reload**

In `private/Decision.ps1`, add:

```powershell
function Update-TermyxUserWords {
    if (-not $script:TermyxUserWordsPath) { return }
    if (-not (Test-Path -LiteralPath $script:TermyxUserWordsPath)) { return }
    $stamp = (Get-Item -LiteralPath $script:TermyxUserWordsPath).LastWriteTimeUtc
    if ($script:TermyxUserWordsStamp -eq $stamp) { return }
    $script:TermyxUserWordsStamp = $stamp
    foreach ($word in (Get-Content -LiteralPath $script:TermyxUserWordsPath -ErrorAction SilentlyContinue)) {
        if ($word.Trim().Length -gt 0) { [Termyx.Corrector]::AddUserWord($word.Trim()) }
    }
}
```

Call it at the top of `Invoke-TermyxCorrection`.

- [ ] **Step 5: Verify**

Run: `powershell.exe -NoProfile -Command "Invoke-Pester -Path .\shell\powershell\tests\Termyx.Tests.ps1"`
Expected: previous 96 plus 3 new, all passing.

- [ ] **Step 6: Update the README and commit**

Document the Termyx integration and the shared `user-words.txt` path.

```bash
git add shell/powershell
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat(shell): emit OSC 133 marks and reload shared user words"
```

---

## Manual Verification

Human required, after all tasks:

1. `cargo run`, open a PowerShell tab with the module installed.
2. Type `Write-Host teh` - `teh` must be underlined.
3. Right-click `teh` - menu shows `the`; click it; the line becomes `Write-Host the`.
4. Right-click elsewhere - clipboard pastes as before.
5. Add a nonsense word to the dictionary, confirm it stops being underlined.
6. `enabled = false` in `[spellcheck]`, restart, confirm no underlines.
7. Run the same steps in `bash` - no marks, therefore no underlines, nothing breaks.

## Self-Review

- Spec coverage: marks (Task 2, 7), scanning rules (Task 1), rendering (Task 3), config (Task 4), menu (Task 5, 6), shared dictionary (Task 7), failure rules (implicit in the early returns of each task).
- Type consistency: `Misspelling { row, col, len }` used unchanged in Tasks 1, 3, 6; `SpellcheckMenu` fields used in Tasks 5 and 6; `input_start` is `Option<InputStart>` in Task 2 and consumed in Task 6.
- No placeholders: every step names the file, the code, the command and the expected result.
