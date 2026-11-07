# Termyx

A fast, lightweight terminal emulator with first-class RTL support.

## Status

Phase 0/1: single window, single shell session, LTR rendering, scrollback, selection, and copy/paste. Bidi and Arabic shaping arrive in Phase 2.

## Build

```bash
rustup default stable
cargo build --release
```

Windows requires the MSVC build tools (`Microsoft.VisualStudio.2022.BuildTools` with the C++ workload).

## Run

```bash
cargo run --release
cargo run --release -- --config path/to/config.toml
```

## Test

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

## Configuration

Default path: `%APPDATA%\termyx\config.toml` on Windows, `$XDG_CONFIG_HOME/termyx/config.toml` (or `~/.config/termyx/config.toml`) on Linux, and `~/Library/Application Support/termyx/config.toml` on macOS.
