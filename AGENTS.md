# Termyx — Agent Instructions

## Project

Native, cross-platform terminal emulator in Rust with first-class RTL support
(UAX#9 bidi + Arabic/Persian/Urdu shaping). The MVP is one window, one shell,
and a correct LTR terminal first; RTL is Phase 2.

Authoritative documents — read both before changing code:

- Spec: `docs/superpowers/specs/2026-11-07-termyx-mvp-design.md`
- Plan: `docs/superpowers/plans/2026-11-07-termyx-phase-0-1.md`

If a request conflicts with them, ask before drifting.

## Non-negotiable code principles

- **Simple first.** Write the smallest code that satisfies the current task.
  No speculative abstraction, no "we'll need it later" fields, modules,
  traits, or config.
- **YAGNI.** Do not add Phase 2+ behavior (bidi, shaping, tabs, splits, AI,
  damage tracking) before its task starts. A placeholder is still code.
- **One responsibility per file and function.** Split functions that do two
  things. Prefer a few small, focused files over one large file.
- **No dead code.** Remove unused fields, parameters, imports, and types.
  `cargo clippy --all-targets -- -D warnings` must pass.
- **Standard library first.** Add a crate only when it removes more code than
  it adds. Reuse existing dependencies.
- **Explicit boundaries.** Public functions have clear inputs, outputs, and
  documented invariants. Helpers stay private.
- **Errors at the edge.** Use `thiserror` per module and `anyhow` in the app.
  No `unwrap`/`expect` in runtime paths; tests may unwrap. Fatal startup
  errors show a dialog and exit; recoverable errors log and continue.
- **Comments explain why, not what.** No commented-out code. No TODO without
  a task in the plan.
- **Behavior-focused tests.** Test observable behavior, not internals. Each
  test should fail for one clear reason.
- **Small diffs.** Change only what the task needs. Do not reformat or
  refactor unrelated code.

## Architecture invariants

- The terminal grid is **logical** (write order). Visual reordering happens
  per row through `VisualMap`; never reorder cells in the grid.
- Cursor is a logical boundary plus `Affinity`. Selection is logical. Copy
  walks logical order. PTY bytes are never pre-reversed.
- Default base direction is `ltr` (prompt pinned left). `auto` is opt-in and
  right-pins on RTL-first text.
- One logical codepoint is one cell; wide CJK is 2. `Cell.width == 0` marks a
  wide-character continuation. Keep the fixed grid aligned.
- PTY output is fed to the VT parser verbatim. The ConPTY child uses UTF-8
  (code page 65001) and `ENABLE_VIRTUAL_TERMINAL_PROCESSING`.
- The UI thread owns the window, renderer, terminal, and interaction state.
  There is one PTY reader thread.

## Current scope (Phases 0–2 + Batches 1–3)

In: one window, one shell session, bidi/RTL rendering with Arabic shaping and
run-level continuous layout, Arial as the default Arabic face (bundled Kawkab
Mono is opt-in), scrollback with a draggable scrollbar, resize, word/line/block
selection, copy-on-select, right- and middle-click paste, plain Ctrl+V paste
and Ctrl+C copy with a selection (bare Ctrl+C still interrupts), middle-click
tab close, application mouse reporting (DEC
modes 1000/1002/1003/1005/1006 with Shift+drag local selection), extended keys
(Shift+Tab, F1–F12, modified navigation), cursor shape/visibility, focus
reporting, window title (OSC 0/2, `dynamic_title` opt-in), OSC 52 copy, tabs
from the configured profiles, clickable links (OSC 8 hyperlinks, plain URLs
and absolute paths) underlined always and opened with the configured gesture,
a check for updates that applies the latest GitHub release and restarts, a
find bar, a command palette and tab context
menus, and symbol-font fallback.

Not yet: Kawkab Mono as an auto-selected face (its narrow metrics render
disconnected at 16 px), exact cell alignment for narrow Arabic runs, full
OSC 133 shell integration, IME preedit, splits, AI,
images.

## Workflow

- Windows: use PowerShell through the shell tool; never use PTY tools.
- TDD: write the failing test, run it, implement, run it, commit.
- Before claiming done: `cargo fmt --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test`,
  `cargo build --release`.
- Commit messages: `feat:`, `fix:`, `test:`, `docs:`, `chore:`.
- Do not commit secrets. Do not invent a git identity; ask the user if it is
  unset.
