# Termyx

A fast, native terminal emulator with first-class RTL support.

Phase 1 is a working LTR terminal: one window, one shell session, scrollback,
resize, selection, copy/paste, and configuration. Bidi and Arabic shaping
arrive in Phase 2.

## Build

Prerequisites:

- Rust stable via [rustup](https://rustup.rs/)
- Windows: MSVC build tools (`Visual Studio Build Tools` with the C++ workload)
- Linux: `libxkbcommon`, `libwayland`, and `libx11-xcb` development packages

```bash
cargo build --release
cargo run --release
```

The binary is `target/release/termyx` (`termyx.exe` on Windows).

## Configuration

Pass `--config <path>` to use a specific file. The default path is:

- Windows: `%APPDATA%\termyx\config.toml`
- Linux: `$XDG_CONFIG_HOME/termyx/config.toml` (or `~/.config/...`)
- macOS: `~/Library/Application Support/termyx/config.toml`

A missing default file is fine; built-in defaults are used. See
`docs/superpowers/specs/2026-11-07-termyx-mvp-design.md` for the schema.

## Phase 1 checklist

- [x] One window, one shell session
- [x] PowerShell, bash, and zsh prompts render at the left margin
- [x] `echo`, `dir`/`ls`, and ANSI colors
- [x] `vim`/`nano` open, redraw, and exit
- [x] Window resize reflows the grid without corruption
- [x] 10,000-line scrollback with mouse wheel
- [x] Mouse selection, Ctrl+Shift+C, Ctrl+Shift+V
- [x] UTF-8 ConPTY output on Windows
- [x] Config file with defaults and validation

## Not in Phase 1

Bidi reordering, Arabic/Persian/Urdu shaping, tabs, splits, profiles, command
palette, search, AI, images, application mouse reporting, and IME preedit.
