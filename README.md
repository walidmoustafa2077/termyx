# Termyx

A fast, native terminal emulator. Phase 1 ships a correct LTR terminal; the
project goal, first-class RTL support (UAX #9 bidi and Arabic/Persian/Urdu
shaping), is Phase 2 work.

Phase 1 is one window, one shell session, scrollback, resize, selection,
copy/paste, and configuration.

## Build

Prerequisites:

- Rust stable via [rustup](https://rustup.rs/)
- Windows: MSVC build tools (Visual Studio Build Tools with the C++ workload)
- Linux: `libxkbcommon`, `libwayland`, and `libx11-xcb` development packages

```bash
cargo build --release
cargo run --release
```

The binary is `target/release/termyx` (`termyx.exe` on Windows).

Phase 1 acceptance was run on Windows 10/11 x64 with MSVC. The Linux and macOS
prerequisites and paths below come from the design spec and were not exercised
in that run.

## Configuration

Pass `--config <path>` to use a specific file. The default path is:

- Windows: `%APPDATA%\termyx\config.toml`
- Linux: `$XDG_CONFIG_HOME/termyx/config.toml` (or `~/.config/...`)
- macOS: `~/Library/Application Support/termyx/config.toml`

A missing default file is fine; built-in defaults are used. An explicitly
passed `--config <path>` that does not exist is an error and aborts startup.
See `docs/superpowers/specs/2026-11-07-termyx-mvp-design.md` for the
schema.

## Phase 1 checklist

Status matches the recorded acceptance run
(`docs/phase-1-acceptance.md`).

- [x] One window, one shell session
- [x] PowerShell prompt renders at the left margin
- [ ] bash/zsh prompt renders at the left margin — not verified on this Windows
      host
- [x] `echo`, `dir`, and ANSI colors
- [ ] `vim`/`nano` open, type, redraw, and exit — not verified on this Windows
      host (no editor installed)
- [x] Window resize reflows the grid (narrowing after scrollback has a caveat;
      see Known limitations)
- [x] 10,000-line scrollback with the mouse wheel
- [x] Mouse selection, Ctrl+Shift+C, Ctrl+Shift+V
- [x] UTF-8 ConPTY output on Windows
- [x] Config file with defaults and validation

## Known limitations and performance

- **Cold start misses its target.** Launch to first prompt measured 405–483 ms
  with `cmd.exe` and 697–734 ms with the default PowerShell, against a
  `< 300 ms` Phase 1 target. Measurements were taken under background load
  (including a fullscreen video); an idle-desktop re-measure is advisable before
  treating them as the baseline.
- **Size and memory.** The release binary is 2.73 MB. 10,000 lines of
  scrollback used 58.6–69.7 MB working set (53.4–64.6 MB private), within the
  80 MB budget.
- **Scrollback is wheel-only.** Phase 1 has no scrollbar UI; the mouse wheel is
  the only way to move through history.
- **Bracketed paste strips its own delimiters.** Clipboard text containing the
  `ESC[200~`/`ESC[201~` markers has them removed before insertion, so a copied
  payload cannot terminate the paste and deliver the rest as typed input.
- **Resize after scrolling.** Before scrollback exists, narrowing reflows into
  a trimmed viewport and restores cleanly. Once output has scrolled, narrowing
  falls back to the terminal engine's bottom-anchored reflow, so the top line of
  the screen may move into scrollback (see `src/term.rs`).

## Development checks

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

## Not in Phase 1

Bidi reordering, Arabic/Persian/Urdu shaping, tabs, splits, profiles, command
palette, search, AI, images, application mouse reporting, and IME preedit.
