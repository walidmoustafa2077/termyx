# Termyx Spellcheck Overlay Design

**Status:** Approved design, implementation not started
**Date:** 2026-11-11
**Related:** `docs/superpowers/specs/2026-11-11-powershell-autocorrect-design.md` (the PowerShell correction module this feature complements)

## 1. Problem

Termyx corrects nothing. Typos in the shell edit line pass through silently, and a user has no way to see which words the autocorrect module would leave alone because they are ambiguous. The PowerShell module cannot fix this itself: PSReadLine 2.0.0 gives a key handler no way to decorate the edit line without corrupting it, so any visual feedback has to come from the terminal.

## 2. Goals and non-goals

Goals:

- Underline misspelled words on the active shell input line while the user types.
- Offer a right-click menu on an underlined word with ranked suggestions, "Add to dictionary", and a session-only "Hide underlines".
- Share one user-word file with the PowerShell module so a word added in Termyx is respected by the next correction.
- Let the user disable underlining and autocorrect through `config.toml`.

Non-goals:

- Checking output, scrollback, or program text.
- Correcting the first token (the command name) - that belongs to the autocorrect module.
- Changing the buffer, sending keystrokes without an explicit menu click, or rewriting shell input.
- Guessing the input line when the shell does not mark it.
- Persisting UI toggles by rewriting `config.toml`.

## 3. Constraints

| Constraint | Consequence |
|---|---|
| Termyx has no VT parser of its own; it feeds `vte::ansi::Processor` into `alacritty_terminal::Term` | OSC 133 handling must live in `Terminal::feed`, not in a new parser |
| No prompt or input-buffer concept exists anywhere in `src/` | The shell must mark the input line, or nothing is underlined |
| PSReadLine 2.0.0 cannot paint the edit line | All decoration is drawn by the renderer from grid cells |
| Word data must work offline on any machine | The word list and frequency list are embedded at build time |
| Existing repo conventions: inline `#[cfg(test)]` tests, `clippy -D warnings`, `cargo fmt --check` | New code follows the same shape; no new crates |
| Bidi and wide cells are already reordered by `VisualMap` | Underline spans are mapped through `VisualMap::visual_of` |

## 4. Requirements

1. The PowerShell module's prompt wrapper emits `ESC ] 133 ; A BEL` before the prompt text and `ESC ] 133 ; B BEL` after it, only when `$env:TERMYX` is `1`. Termyx sets `TERMYX=1` in every child shell environment.
2. Termyx records the cell where the last `B` mark was seen and the row/column of the cursor. The candidate text is exactly the cells from the `B` cell to the cursor cell on the same row.
3. If there is no `B` mark, or the mark is on a different row than the cursor, nothing is underlined.
4. Tokens checked are bare argument words after the first token: `[A-Za-z][A-Za-z0-9'_-]*`, length at least 3, not inside quotes, not a path/URL/flag/number, not the command name.
5. Unknown tokens get an underline drawn by the existing decoration pass, using a configurable colour.
6. Right-click on an underlined token opens a menu; right-click elsewhere pastes as it does today. Apps that request mouse reporting keep the click.
7. The menu lists up to five suggestions in frequency order, then "Add to dictionary" and "Hide underlines (this session)".
8. Clicking a suggestion sends backspaces and the suggestion only when the clicked token is the last token and the cursor is at its end. Otherwise suggestions are shown disabled with the reason.
9. "Add to dictionary" appends the token to `%LOCALAPPDATA%\Termyx\user-words.txt` and stops underlining it immediately.
10. `[spellcheck] enabled = false` disables underlining; `autocorrect = false` makes Termyx set `TERMYX_AUTOCORRECT=0` for child shells.

## 5. Architecture

```
PowerShell prompt wrapper ──ESC]133;A/B──> PTY ──> Terminal::feed
                                                   │
                              OSC scan + mark capture
                                                   │
                                             GridSnapshot { input_start, cursor }
                                                   │
                          spellcheck::SpellChecker (embedded words + frequency)
                                                   │
                          Vec<Misspelling { row, col, len }> (bidi-mapped)
                                                   │
                          Renderer::draw -> draw_spellcheck_underlines
                                                   │
                          right-click hit test -> ui/spellcheck_menu.rs
```

Six units, each independently testable:

| Unit | Responsibility | Depends on |
|---|---|---|
| `src/spellcheck.rs` | Embedded assets, tokenizer, skip rules, membership, suggestions | nothing in the app |
| `src/term.rs` (extended) | Scan OSC 133, expose `input_start`, compute spans | `spellcheck` |
| `src/render/mod.rs` (extended) | Draw underline rects | grid geometry, `VisualMap` |
| `src/ui/spellcheck_menu.rs` + `src/render/spellcheck_menu.rs` | Menu model and painter | theme, `ui_text` |
| `src/app.rs` (extended) | Hit-test right-click, execute actions, session toggle | all of the above |
| `shell/powershell/Termyx` (extended) | Emit marks, reload the shared user-word file | module internals |

## 6. Data flow

### 6.1 OSC 133 marks

`Terminal::feed` keeps a `pending: Vec<u8>` buffer. On each call it appends the new bytes and scans for complete OSC sequences (`ESC ]` ... `BEL` or `ESC \`). Bytes before an incomplete sequence are fed to the processor immediately; the incomplete tail stays buffered. A complete OSC is fed to the processor unchanged (so title and palette sequences keep working) and then, if its payload is `133;A` or `133;B`, the mark is recorded from the current grid cursor.

- `A` clears `input_start` (a new prompt).
- `B` sets `input_start` to the current cursor point.

No `C` or `D` handling is needed: once the cursor leaves the start row, rule 3 suppresses everything.

The mark stores an absolute `Point` (alacritty line index plus column) so scrollback growth does not shift it.

### 6.2 Span computation

`SpellChecker::scan(row_cells, start_col, cursor_col) -> Vec<Misspelling>` walks the candidate cells, builds tokens with a hand-written state machine (no regex crate), applies the skip rules, and looks tokens up in the embedded `HashSet`. Results are logical columns; the renderer maps them to visual columns.

### 6.3 Menu actions

- Suggestion click: if `is_last_token && cursor_at_end`, write `len` backspaces then the suggestion bytes to the PTY. Otherwise the item is disabled.
- Add to dictionary: append `token + "\n"` to the shared file (create it if missing), insert into the in-memory set, close the menu.
- Hide underlines: set a session flag that clears `enabled` until restart, close the menu.

## 7. Spellcheck core

- Assets embedded with `include_str!` from `shell/powershell/Termyx/assets/en_US.wordlist.txt` (87,928 words) and `en_US.frequency.txt` (46,717 ranked words).
- Loaded lazily into a `OnceLock<WordData>` on the first frame that needs it, so startup cost is not paid on machines that never enable the feature. A background warm-up on the first tab is acceptable if measurement shows a slow first underline.
- Tokenizer rules, in order: skip the first token on the line; reject spans containing `"`, `'`, backtick, `$`, `\`, `/`, `:`, `@`; reject tokens starting with `-`; reject digits; reject length < 3; reject all-uppercase tokens of 4+ characters.
- Membership is case-insensitive; a token that is a known word is not underlined.
- Suggestions use the same policy as the module: distance 1 first; distance 2 only for tokens of 6 characters or fewer; among tied candidates prefer ones at least as long as the token, then the lowest frequency rank; if no tied candidate is ranked, no suggestion. Ported in Rust with a banded Damerau-Levenshtein over the length index.

## 8. Rendering

`Renderer::draw` gains a `spellcheck: &[Misspelling]` argument and calls `draw_spellcheck_underlines` immediately after `draw_decorations` (`render/mod.rs:357`). Each span is converted to visual columns with `VisualMap::visual_of`, then drawn as a 1-pixel (scaled) line under the glyph baseline using the existing geometry helpers.

The underline is drawn over cells but never modifies cell state, so copy, find, selection and scrollback are unaffected.

## 9. UI menu

Model in `src/ui/spellcheck_menu.rs`: `{ anchor_row, anchor_col, token, suggestions: Vec<Suggestion>, disabled_reason: Option<&'static str> }`. Painter mirrors `render/context_menu.rs`. Draw order: after the find bar and before the tab context menu. Dismissal follows the existing overlay rules (Escape, click outside, second right-click).

Hit-testing: right-click maps to a logical cell, then checks whether that cell lies inside any computed span. Interception happens in `App::on_mouse` before `pastes_locally` (`app.rs:2052`), but after mouse-reporting forwarding (`app.rs:2042`).

## 10. Shared dictionary and module changes

- Path: `%LOCALAPPDATA%\Termyx\user-words.txt`, one word per line, already the module's user-word file.
- Termyx appends and reloads its own set immediately.
- The module reloads when the file's `LastWriteTimeUtc` differs from its cached value, checked at most every 5 seconds inside `Invoke-TermyxCorrection`, adding new lines with `Corrector.AddUserWord`. Module test covers the reload.
- The prompt wrapper lives in a new `private/Prompt.ps1`, installed by `Enable-TermyxAutocorrect` and removed by `Disable-TermyxAutocorrect`.

## 11. Configuration

```toml
[spellcheck]
enabled = true            # underline misspelled words on the input line
autocorrect = true        # export TERMYX_AUTOCORRECT=1 to child shells
underline_color = "#d19a66"
```

New section struct with serde defaults, added to `KNOWN_KEYS` and `Config::validate` (colour parsed and clamped), documented in README, plus parse and default tests.

## 12. Failure handling

| Failure | Behaviour |
|---|---|
| No `B` mark | No underline; menu never opens |
| Malformed OSC | Ignored; the last good mark is kept |
| Input line wraps or start row differs from cursor row | No underline |
| Asset missing at build time | Compile error from `include_str!`; no runtime path |
| Word set fails to initialize | Feature disables itself, logs one warning, terminal keeps working |
| User-word file locked when adding | Menu shows the failure, in-memory set unchanged |
| Click-to-replace on a non-final token | Refused with a visible reason |

## 13. Testing

- `spellcheck`: tokenizer and every skip rule, membership casing, suggestion ranking, tie refusal, embedded asset sanity (word count equals the committed file).
- `term`: feeding `ESC]133;A`/`B` sets and clears `input_start`; escapes split across two `feed` calls still register; an unrelated OSC still reaches the processor.
- render: pixel assertion that an underline is drawn under the right cells, and none when the feature is off; bidi case with a mapped span.
- ui menu: item list, disabled state, dismissal.
- config: defaults and parsing for `[spellcheck]`.
- module: prompt wrapper emits marks only when `TERMYX=1`; user-word reload picks up a new line.

## 14. Milestones

1. **M1** - `spellcheck.rs` core, OSC 133 capture, `input_start` in the snapshot, unit tests.
2. **M2** - underline rendering, `[spellcheck]` config, README.
3. **M3** - module prompt marks and shared user-word reload, module tests.
4. **M4** - right-click menu, click-to-replace, add-to-dictionary, session toggle.

Each milestone leaves the terminal usable and the suite green.

## 15. Risks

| Risk | Mitigation |
|---|---|
| Prompt wrappers are fragile across shells | Marks are emitted only for PowerShell with the module; no marks means no feature |
| Re-rendering on resize re-invokes `prompt` | A mark refresh is harmless: `B` just records the new cursor cell |
| Embedded assets grow the binary by ~1.3 MB | Accepted; both are already committed and needed for the module |
| Suggestion parity with the module drifts | Policy is specified in one place (this document) and both sides test the same table of cases |
| Right-click interception surprises users | Only intercepted over an underlined token; every other right-click still pastes |
