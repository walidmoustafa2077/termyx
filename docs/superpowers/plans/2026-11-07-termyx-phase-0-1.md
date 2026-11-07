# Termyx Phase 0/1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a working single-window, single-session LTR terminal in Rust (Phase 0 scaffolding + Phase 1 terminal), with the `VisualMap` abstraction wired through rendering, cursor placement, and hit-testing so Phase 2 can add bidi/shaping without restructuring.

**Architecture:** `winit` owns the window and event loop. A `portable-pty` child shell is read on one background thread and delivered to the UI thread through `EventLoopProxy<UserEvent>`. `alacritty_terminal` parses VT bytes into a logical grid; `Terminal::snapshot()` produces a `GridSnapshot` in logical order. Each row is rendered through `VisualMap::identity` (Phase 1) into a `tiny-skia` pixmap, then copied to a `softbuffer` surface. Keyboard input is encoded logically and written to the PTY. Selection is stored logically and copied in logical order.

**Tech Stack:** Rust stable (MSVC on Windows), winit 0.30, softbuffer 0.4, tiny-skia 0.11, alacritty_terminal 0.26, portable-pty 0.8, fontdb 0.23, ttf-parser 0.25, ab_glyph 0.2, serde/toml, arboard 3, tracing, thiserror/anyhow.

**Spec:** `docs/superpowers/specs/2026-11-07-termyx-mvp-design.md`

## Global Constraints

- Platforms: Windows 10 1809+ (ConPTY, MSVC build tools), Linux (UTF-8, forkpty), macOS 11+.
- Rust stable via rustup; `cargo fmt` and `cargo clippy --all-targets -- -D warnings` must pass.
- One window, one shell session. No tabs, splits, profiles, command palette, search, AI, images, programming ligatures, app mouse reporting, or IME preedit UI in this plan.
- Grid stays in logical order. All rendering and mouse interaction goes through `VisualMap`; Phase 1 uses `VisualMap::identity`.
- PTY output is fed to the parser verbatim. Never pre-reverse, reorder, or rewrite terminal output.
- Base direction default is `ltr` (prompt pinned left). `rtl` and `auto` exist in config but only `ltr` is exercised in Phase 1.
- Cell width is `0` for wide-character continuation cells, `1` for normal, `2` for wide. `RowSnapshot::cells.len() == cols` always.
- Scrollback default 10,000 lines, clamped 1,000..=200,000.
- Release profile: `lto = "thin"`, `codegen-units = 1`, `strip = true`. `panic = "abort"` is added only after top-level panic handling is verified.
- Phase 1 scope note: Kawkab Mono bundling and all bidi/shaping code are Phase 2. Phase 1 uses system monospace fonts and renders LTR only. `unicode-bidi`, `rustybuzz`, and `assets/fonts/` are not added by this plan.
- If `git commit` fails with `Author identity unknown`, STOP and ask the user for `user.name` and `user.email`; do not invent an identity.

## Code principles

These are non-negotiable for every task. See `AGENTS.md` for the full list.

- Write the smallest code that satisfies the current task. No speculative
  abstraction, no fields or modules reserved for later phases.
- No dead code. Every field, parameter, import, and type added by a task must
  be used by that task or an earlier one.
- One responsibility per file and function. Split functions that do two
  things; keep modules focused.
- Standard library first. Add a crate only when it removes more code than it
  adds.
- No `unwrap`/`expect` in runtime paths. Use `thiserror` at module boundaries
  and `anyhow` in `main`/`app`. Tests may unwrap.
- Tests assert observable behavior, not internals. Each test fails for one
  clear reason.
- Comments explain why, not what. No commented-out code and no TODOs.

## File Structure

| File | Responsibility |
|---|---|
| `Cargo.toml`, `rust-toolchain.toml`, `.gitignore` | Toolchain, dependencies, release profile |
| `.github/workflows/ci.yml` | Windows + Linux fmt/clippy/test/build |
| `src/main.rs` | Logging, config path parsing, event loop startup |
| `src/config.rs` | TOML schema, defaults, validation, config path |
| `src/theme.rs` | Hex colors, palette resolution |
| `src/fonts.rs` | System font lookup, cell metrics, glyph lookup |
| `src/render/mod.rs` | Pixmap, background/cell/text/cursor drawing, softbuffer pixels |
| `src/render/atlas.rs` | 2048x2048 R8 shelf allocator |
| `src/render/glyph_cache.rs` | Glyph rasterization and cache |
| `src/pty.rs` | ConPTY/forkpty spawn, reader thread, writer, resize |
| `src/term.rs` | alacritty_terminal wrapper, logical snapshots, mode, scroll |
| `src/event.rs` | `UserEvent` delivered to the winit event loop |
| `src/input.rs` | Pure key/paste encoder |
| `src/visual.rs` | `VisualMap`, cursor resolution, hit-testing |
| `src/selection.rs` | Logical selection and copy text |
| `src/app.rs` | Window, surface, PTY/terminal wiring, events, mouse, clipboard |

---

### Task 1: Scaffold, toolchain, CI, and first commit

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `.gitignore`
- Create: `.github/workflows/ci.yml`
- Create: `README.md`
- Create: `src/main.rs`
- Modify: `docs/superpowers/specs/2026-11-07-termyx-mvp-design.md` (already written; commit it)

**Interfaces:**
- Consumes: nothing.
- Produces: `termyx` binary crate; `cargo test` and `cargo clippy` work on Windows and Linux; `src/main.rs` module declarations used by every later task.

- [ ] **Step 1: Verify git identity and Rust toolchain**

Run:
```powershell
git config user.name
git config user.email
rustup --version
cargo --version
```

Expected: both identity values are non-empty and Rust is installed. If identity is empty, STOP and ask the user for the two values, then run `git config user.name "<name>"` and `git config user.email "<email>"`. If Rust is missing, install it:
```powershell
winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
rustup default stable-msvc
```
On Linux/macOS: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && source "$HOME/.cargo/env"`.

- [ ] **Step 2: Create `Cargo.toml`**

```toml
[package]
name = "termyx"
version = "0.1.0"
edition = "2021"
rust-version = "1.80"
description = "A fast, lightweight terminal with RTL support"
license = "MIT"

[dependencies]
winit = "0.30"
softbuffer = "0.4"
tiny-skia = "0.11"
alacritty_terminal = "0.26"
portable-pty = "0.8"
unicode-width = "0.2"
unicode-segmentation = "1"
fontdb = "0.23"
ttf-parser = "0.25"
ab_glyph = "0.2"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
thiserror = "2"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
rfd = "0.15"
arboard = "3"

[dev-dependencies]
insta = "1"
image = "0.25"

[profile.release]
lto = "thin"
codegen-units = 1
strip = true
```

- [ ] **Step 3: Create toolchain, ignore, and CI files**

`rust-toolchain.toml`:
```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

`.gitignore`:
```gitignore
/target
*.pdb
*.log
.idea/
.vscode/
```

`.github/workflows/ci.yml`:
```yaml
name: ci
on: [push, pull_request]
jobs:
  test:
    strategy:
      fail-fast: false
      matrix:
        os: [windows-latest, ubuntu-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - name: Linux windowing deps
        if: runner.os == 'Linux'
        run: sudo apt-get update && sudo apt-get install -y libxkbcommon-dev libwayland-dev libx11-xcb-dev
      - run: cargo fmt --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo test
      - run: cargo build --release
```

- [ ] **Step 4: Create `src/main.rs` with the module map**

```rust
mod app;
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

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();
}
```

Task 3 replaces this with the event-loop entry point once `Config`, `UserEvent`, and `App` exist.

- [ ] **Step 5: Create empty module files**

Create each file with a one-line module doc comment and no items:
```rust
// src/app.rs
//! Window, surface, and event wiring.
```
Repeat the same pattern in `src/event.rs`, `src/fonts.rs`, `src/input.rs`, `src/pty.rs`, `src/render/mod.rs`, `src/selection.rs`, `src/term.rs`, `src/theme.rs`, `src/visual.rs`. Empty modules are valid Rust and do not trigger `dead_code`.

- [ ] **Step 6: Build and lint**

Run:
```powershell
cargo build
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```
Expected: build succeeds; fmt and clippy pass.

- [ ] **Step 7: Write `README.md`**

```markdown
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
```

- [ ] **Step 8: Commit the spec and scaffold**

```powershell
git add Cargo.toml rust-toolchain.toml .gitignore .github/workflows/ci.yml README.md AGENTS.md src docs/superpowers/specs/2026-11-07-termyx-mvp-design.md docs/superpowers/plans/2026-11-07-termyx-phase-0-1.md
git commit -m "chore: scaffold Rust terminal and add MVP design spec"
```

Expected: commit succeeds. If it fails with `Author identity unknown`, return to Step 1.

---

### Task 2: Config schema, defaults, and validation

**Files:**
- Modify: `src/config.rs`
- Test: `src/config.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: `serde`, `toml`, `thiserror`.
- Produces: `Config { font, bidi, window, scrollback, selection, theme, shell }`, `BaseDirection`, `Config::load(Option<&Path>) -> Result<Config, ConfigError>`, `Config::from_toml_str(&str)`, `Config::validate(self) -> Config`, `default_config_path() -> PathBuf`.

- [ ] **Step 1: Write the failing tests**

Append to `src/config.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec() {
        let c = Config::default();
        assert_eq!(c.font.family, "Cascadia Mono");
        assert_eq!(c.font.arabic_family, "Kawkab Mono");
        assert_eq!(c.font.size, 14.0);
        assert_eq!(c.font.line_height, 1.2);
        assert!(c.bidi.enabled);
        assert_eq!(c.bidi.base_direction, BaseDirection::Ltr);
        assert_eq!(c.window.width, 1200);
        assert_eq!(c.window.height, 800);
        assert_eq!(c.window.padding, 4);
        assert_eq!(c.scrollback.lines, 10_000);
        assert!(c.selection.trim_trailing);
    }

    #[test]
    fn font_size_is_clamped() {
        let c = Config::from_toml_str("[font]\nsize = 100.0\n").unwrap().validate();
        assert_eq!(c.font.size, 48.0);
        let c = Config::from_toml_str("[font]\nsize = 1.0\n").unwrap().validate();
        assert_eq!(c.font.size, 6.0);
    }

    #[test]
    fn scrollback_is_clamped() {
        let c = Config::from_toml_str("[scrollback]\nlines = 10\n").unwrap().validate();
        assert_eq!(c.scrollback.lines, 1_000);
        let c = Config::from_toml_str("[scrollback]\nlines = 999999\n").unwrap().validate();
        assert_eq!(c.scrollback.lines, 200_000);
    }

    #[test]
    fn invalid_base_direction_falls_back_to_ltr() {
        let c = Config::from_toml_str("[bidi]\nbase_direction = \"sideways\"\n").unwrap();
        assert_eq!(c.bidi.base_direction, BaseDirection::Ltr);
        let c = Config::from_toml_str("[bidi]\nbase_direction = \"rtl\"\n").unwrap();
        assert_eq!(c.bidi.base_direction, BaseDirection::Rtl);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let c = Config::from_toml_str("[font]\nwat = 1\n").unwrap();
        assert_eq!(c.font.size, 14.0);
    }

    #[test]
    fn default_config_path_contains_app_name() {
        let p = default_config_path();
        assert!(p.to_string_lossy().contains("termyx"));
        assert!(p.to_string_lossy().ends_with("config.toml"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test config::tests -v`
Expected: FAIL with `cannot find type Config` / `BaseDirection` not found.

- [ ] **Step 3: Write the implementation**

Replace `src/config.rs` with:
```rust
use serde::{Deserialize, Deserializer};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub font: FontConfig,
    pub bidi: BidiConfig,
    pub window: WindowConfig,
    pub scrollback: ScrollbackConfig,
    pub selection: SelectionConfig,
    pub theme: ThemeConfig,
    pub shell: ShellConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct FontConfig {
    pub family: String,
    pub arabic_family: String,
    pub size: f32,
    pub line_height: f32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: "Cascadia Mono".into(),
            arabic_family: "Kawkab Mono".into(),
            size: 14.0,
            line_height: 1.2,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct BidiConfig {
    pub enabled: bool,
    pub base_direction: BaseDirection,
}

impl Default for BidiConfig {
    fn default() -> Self {
        Self { enabled: true, base_direction: BaseDirection::Ltr }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BaseDirection {
    #[default]
    Ltr,
    Rtl,
    Auto,
}

impl<'de> Deserialize<'de> for BaseDirection {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "rtl" => Self::Rtl,
            "auto" => Self::Auto,
            _ => Self::Ltr,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
    pub padding: u32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self { width: 1200, height: 800, padding: 4 }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ScrollbackConfig {
    pub lines: usize,
}

impl Default for ScrollbackConfig {
    fn default() -> Self {
        Self { lines: 10_000 }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SelectionConfig {
    pub trim_trailing: bool,
}

impl Default for SelectionConfig {
    fn default() -> Self {
        Self { trim_trailing: true }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    pub selection: String,
    pub black: String,
    pub red: String,
    pub green: String,
    pub yellow: String,
    pub blue: String,
    pub magenta: String,
    pub cyan: String,
    pub white: String,
    pub bright_black: String,
    pub bright_red: String,
    pub bright_green: String,
    pub bright_yellow: String,
    pub bright_blue: String,
    pub bright_magenta: String,
    pub bright_cyan: String,
    pub bright_white: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            background: "#0c0c0c".into(),
            foreground: "#cccccc".into(),
            cursor: "#ffffff".into(),
            selection: "#264f78".into(),
            black: "#0c0c0c".into(),
            red: "#c50f1f".into(),
            green: "#13a10e".into(),
            yellow: "#c19c00".into(),
            blue: "#0037da".into(),
            magenta: "#881798".into(),
            cyan: "#3a96dd".into(),
            white: "#cccccc".into(),
            bright_black: "#767676".into(),
            bright_red: "#e74856".into(),
            bright_green: "#16c60c".into(),
            bright_yellow: "#f9f1a5".into(),
            bright_blue: "#3b78ff".into(),
            bright_magenta: "#b4009e".into(),
            bright_cyan: "#61d6d6".into(),
            bright_white: "#f2f2f2".into(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ShellConfig {
    pub program: String,
    pub args: Vec<String>,
}

impl Config {
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str::<Self>(text)?.validate())
    }

    pub fn validate(mut self) -> Self {
        self.font.size = self.font.size.clamp(6.0, 48.0);
        if !self.font.line_height.is_finite() || self.font.line_height < 0.8 {
            self.font.line_height = 1.2;
        }
        self.scrollback.lines = self.scrollback.lines.clamp(1_000, 200_000);
        self
    }

    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let explicit = path.is_some();
        let path = path.map(Path::to_path_buf).unwrap_or_else(default_config_path);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::from_toml_str(&text),
            Err(source) if !explicit && source.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self::default().validate())
            }
            Err(source) => Err(ConfigError::Io { path, source }),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read config at {path}: {source}")]
    Io { path: PathBuf, #[source] source: std::io::Error },
    #[error("invalid config: {0}")]
    Parse(#[from] toml::de::Error),
}

pub fn default_config_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join("termyx").join("config.toml");
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join("Library/Application Support/termyx/config.toml");
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(xdg).join("termyx").join("config.toml");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".config/termyx/config.toml");
        }
    }
    PathBuf::from("config.toml")
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test config::tests -v`
Expected: 6 tests pass.

- [ ] **Step 5: Commit**

```powershell
git add src/config.rs
git commit -m "feat: add TOML config with defaults and validation"
```

---

### Task 3: Window, softbuffer surface, and background rendering

**Files:**
- Modify: `src/theme.rs`
- Modify: `src/event.rs`
- Modify: `src/render/mod.rs`
- Create: `src/render/atlas.rs` (empty module for now)
- Create: `src/render/glyph_cache.rs` (empty module for now)
- Modify: `src/app.rs`
- Test: `src/theme.rs` (inline)

**Interfaces:**
- Consumes: `Config`, `ThemeConfig`.
- Produces: `Rgb { r, g, b }`, `Rgb::from_hex`, `Theme::from_config`, `Theme::indexed`, `Renderer::new(Theme, padding)`, `Renderer::resize(u32, u32)`, `Renderer::draw()`, `Renderer::pixels() -> &[u32]`, `App::new(Config, EventLoopProxy<UserEvent>)`.

- [ ] **Step 1: Write the failing theme tests**

Append to `src/theme.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_colors() {
        assert_eq!(Rgb::from_hex("#0c0c0c"), Some(Rgb::new(12, 12, 12)));
        assert_eq!(Rgb::from_hex("ccCCcc"), Some(Rgb::new(204, 204, 204)));
        assert_eq!(Rgb::from_hex("#fff"), None);
        assert_eq!(Rgb::from_hex("zzzzzz"), None);
    }

    #[test]
    fn indexed_palette_covers_ansi_and_256_colors() {
        let theme = Theme::from_config(&crate::config::ThemeConfig::default());
        assert_eq!(theme.indexed(1), Rgb::new(197, 15, 31));
        assert_eq!(theme.indexed(9), Rgb::new(231, 72, 86));
        assert_eq!(theme.indexed(16), Rgb::new(0, 0, 0));
        assert_eq!(theme.indexed(231), Rgb::new(255, 255, 255));
        assert_eq!(theme.indexed(232), Rgb::new(8, 8, 8));
        assert_eq!(theme.indexed(255), Rgb::new(238, 238, 238));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test theme::tests -v`
Expected: FAIL with `cannot find type Rgb` / `Theme`.

- [ ] **Step 3: Implement `theme.rs`**

```rust
use crate::config::ThemeConfig;
use tiny_skia::Color as SkColor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn from_hex(value: &str) -> Option<Self> {
        let hex = value.strip_prefix('#').unwrap_or(value);
        if hex.len() != 6 || !hex.is_ascii() {
            return None;
        }
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some(Self { r, g, b })
    }

    pub fn to_tiny(self) -> SkColor {
        SkColor::from_rgba8(self.r, self.g, self.b, 255)
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    pub normal: [Rgb; 8],
    pub bright: [Rgb; 8],
}

impl Theme {
    pub fn from_config(config: &ThemeConfig) -> Self {
        let parse = |value: &str, fallback: Rgb| Rgb::from_hex(value).unwrap_or(fallback);
        Self {
            background: parse(&config.background, Rgb::new(12, 12, 12)),
            foreground: parse(&config.foreground, Rgb::new(204, 204, 204)),
            cursor: parse(&config.cursor, Rgb::new(255, 255, 255)),
            selection: parse(&config.selection, Rgb::new(38, 79, 120)),
            normal: [
                parse(&config.black, Rgb::new(12, 12, 12)),
                parse(&config.red, Rgb::new(197, 15, 31)),
                parse(&config.green, Rgb::new(19, 161, 14)),
                parse(&config.yellow, Rgb::new(193, 156, 0)),
                parse(&config.blue, Rgb::new(0, 55, 218)),
                parse(&config.magenta, Rgb::new(136, 23, 152)),
                parse(&config.cyan, Rgb::new(58, 150, 221)),
                parse(&config.white, Rgb::new(204, 204, 204)),
            ],
            bright: [
                parse(&config.bright_black, Rgb::new(118, 118, 118)),
                parse(&config.bright_red, Rgb::new(231, 72, 86)),
                parse(&config.bright_green, Rgb::new(22, 198, 12)),
                parse(&config.bright_yellow, Rgb::new(249, 241, 165)),
                parse(&config.bright_blue, Rgb::new(59, 120, 255)),
                parse(&config.bright_magenta, Rgb::new(180, 0, 158)),
                parse(&config.bright_cyan, Rgb::new(97, 214, 214)),
                parse(&config.bright_white, Rgb::new(242, 242, 242)),
            ],
        }
    }

    pub fn indexed(&self, index: u8) -> Rgb {
        match index {
            0..=7 => self.normal[index as usize],
            8..=15 => self.bright[(index - 8) as usize],
            16..=231 => {
                let i = index - 16;
                let component = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
                Rgb::new(component(i / 36), component((i % 36) / 6), component(i % 6))
            }
            _ => {
                let v = 8 + (index - 232) * 10;
                Rgb::new(v, v, v)
            }
        }
    }
}
```

- [ ] **Step 4: Implement `event.rs`**

```rust
#[derive(Debug)]
pub enum UserEvent {}
```

- [ ] **Step 5: Implement the background renderer**

`src/render/atlas.rs` and `src/render/glyph_cache.rs` are empty files for now.

`src/render/mod.rs`:
```rust
mod atlas;
mod glyph_cache;

use tiny_skia::Pixmap;

use crate::theme::Theme;

pub struct Renderer {
    pixmap: Pixmap,
    pixels: Vec<u32>,
    theme: Theme,
    _padding: u32,
}

impl Renderer {
    pub fn new(theme: Theme, padding: u32) -> Self {
        Self {
            pixmap: Pixmap::new(1, 1).expect("1x1 pixmap"),
            pixels: vec![0],
            theme,
            _padding: padding,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.pixmap = Pixmap::new(width, height).expect("pixmap allocation");
        self.pixels = vec![0; (width as usize) * (height as usize)];
        self.pixmap.fill(self.theme.background.to_tiny());
        self.sync_pixels();
    }

    pub fn draw(&mut self) {
        self.pixmap.fill(self.theme.background.to_tiny());
        self.sync_pixels();
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    fn sync_pixels(&mut self) {
        for (index, pixel) in self.pixmap.data().chunks_exact(4).enumerate() {
            self.pixels[index] =
                ((pixel[0] as u32) << 16) | ((pixel[1] as u32) << 8) | pixel[2] as u32;
        }
    }
}
```
`padding` is not read yet; Task 4 uses it. To keep clippy green, name it `_padding` for now and rename in Task 4. Do that.

- [ ] **Step 6: Implement the minimal `app.rs`**

```rust
use std::num::NonZeroU32;
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use crate::config::Config;
use crate::event::UserEvent;
use crate::render::Renderer;
use crate::theme::Theme;

pub struct App {
    config: Config,
    _proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    renderer: Option<Renderer>,
}

impl App {
    pub fn new(config: Config, proxy: EventLoopProxy<UserEvent>) -> Self {
        Self { config, _proxy: proxy, window: None, surface: None, renderer: None }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Termyx")
            .with_inner_size(LogicalSize::new(self.config.window.width, self.config.window.height));
        let window = Arc::new(event_loop.create_window(attributes).expect("create window"));
        let context = softbuffer::Context::new(window.clone()).expect("softbuffer context");
        let surface =
            softbuffer::Surface::new(&context, window.clone()).expect("softbuffer surface");
        let mut renderer = Renderer::new(Theme::from_config(&self.config.theme), self.config.window.padding);
        let size = window.inner_size();
        renderer.resize(size.width, size.height);
        self.renderer = Some(renderer);
        self.surface = Some(surface);
        self.window = Some(window);
        self.window.as_ref().unwrap().request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(surface), Some(renderer)) =
                    (self.surface.as_mut(), self.renderer.as_mut())
                {
                    if let (Some(width), Some(height)) =
                        (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
                    {
                        let _ = surface.resize(width, height);
                        renderer.resize(size.width, size.height);
                    }
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                let (Some(surface), Some(renderer)) =
                    (self.surface.as_mut(), self.renderer.as_mut())
                else {
                    return;
                };
                renderer.draw();
                let Ok(mut buffer) = surface.buffer_mut() else {
                    return;
                };
                buffer.copy_from_slice(renderer.pixels());
                let _ = buffer.present();
            }
            _ => {}
        }
    }
}
```

- [ ] **Step 7: Replace `src/main.rs` with the event-loop entry point**

```rust
mod app;
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

use anyhow::Context;
use std::path::PathBuf;
use winit::event_loop::{ControlFlow, EventLoop};

fn config_path_from_args() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config" {
            return args.next().map(PathBuf::from);
        }
    }
    None
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();

    let config = config::Config::load(config_path_from_args().as_deref())
        .context("loading configuration")?;
    let event_loop = EventLoop::<event::UserEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let mut app = app::App::new(config, proxy);
    event_loop.run_app(&mut app)?;
    Ok(())
}
```

- [ ] **Step 8: Build and run manually**

Run:
```powershell
cargo run
```
Expected: a 1200x800 dark window titled "Termyx". Closing it exits the process.

- [ ] **Step 9: Run tests, fmt, clippy**

Run:
```powershell
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```
Expected: tests pass; fmt/clippy clean.

- [ ] **Step 10: Commit**

```powershell
git add src/main.rs src/app.rs src/event.rs src/render/mod.rs src/render/atlas.rs src/render/glyph_cache.rs src/theme.rs
git commit -m "feat: open a winit window and present a solid background"
```

---

### Task 4: Fonts, cell metrics, glyph atlas, and text rendering

**Files:**
- Modify: `src/fonts.rs`
- Modify: `src/render/atlas.rs`
- Modify: `src/render/glyph_cache.rs`
- Modify: `src/render/mod.rs`
- Modify: `src/app.rs`
- Test: `src/fonts.rs`, `src/render/atlas.rs`, `src/render/glyph_cache.rs` (inline)

**Interfaces:**
- Consumes: `Theme`, `Rgb`.
- Produces: `FontSet { font: FontVec, size_px: f32, cell_w: u32, cell_h: u32, ascent: f32, descent: f32 }`, `FontSet::load(&str, f32, f32)`, `FontSet::glyph(char) -> Option<GlyphId>`, `Atlas::new/alloc/upload/data/width`, `GlyphCache::new/get(&FontVec, GlyphId, f32)`, `Renderer::new(FontSet, Theme, padding)`, `Renderer::draw_text(x, baseline, text, Rgb)`, `Renderer::sync()`.

- [ ] **Step 1: Write the failing tests**

Append to `src/fonts.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_a_font_with_positive_metrics() {
        let font = FontSet::load("Consolas", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", 14.0, 1.2))
            .expect("a monospace font must exist");
        assert!(font.cell_w > 0);
        assert!(font.cell_h > 0);
        assert!(font.ascent > 0.0);
        assert!(font.descent < 0.0);
        assert!(font.glyph('A').is_some());
        assert!(font.glyph('M').is_some());
    }
}
```

Append to `src/render/atlas.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocations_do_not_overlap() {
        let mut atlas = Atlas::new(8, 8);
        let a = atlas.alloc(4, 4).unwrap();
        let b = atlas.alloc(4, 4).unwrap();
        let c = atlas.alloc(4, 4).unwrap();
        let d = atlas.alloc(4, 4).unwrap();
        assert_eq!(a, Rect { x: 0, y: 0, w: 4, h: 4 });
        assert_eq!(b, Rect { x: 4, y: 0, w: 4, h: 4 });
        assert_eq!(c, Rect { x: 0, y: 4, w: 4, h: 4 });
        assert_eq!(d, Rect { x: 4, y: 4, w: 4, h: 4 });
        assert!(atlas.alloc(1, 1).is_none());
    }

    #[test]
    fn upload_copies_mask_into_atlas() {
        let mut atlas = Atlas::new(4, 4);
        let rect = atlas.alloc(2, 2).unwrap();
        atlas.upload(rect, &[1, 2, 3, 4]);
        assert_eq!(&atlas.data()[0..2], &[1, 2]);
        assert_eq!(&atlas.data()[4..6], &[3, 4]);
    }
}
```

Append to `src/render/glyph_cache.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::FontSet;

    #[test]
    fn caches_the_same_glyph_slot() {
        let fonts = FontSet::load("Consolas", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", 14.0, 1.2))
            .expect("font");
        let glyph = fonts.font.glyph_id('A');
        let mut cache = GlyphCache::new();
        let first = cache.get(&fonts.font, glyph, fonts.size_px).unwrap();
        let second = cache.get(&fonts.font, glyph, fonts.size_px).unwrap();
        assert_eq!(first.rect, second.rect);
        assert!(first.rect.w > 0 && first.rect.h > 0);
    }

    #[test]
    fn missing_glyph_returns_none_and_is_cached() {
        let fonts = FontSet::load("Consolas", 14.0, 1.2)
            .or_else(|_| FontSet::load("DejaVu Sans Mono", 14.0, 1.2))
            .expect("font");
        let glyph = fonts.font.glyph_id(' ');
        let mut cache = GlyphCache::new();
        assert!(cache.get(&fonts.font, glyph, fonts.size_px).is_none());
        assert!(cache.get(&fonts.font, glyph, fonts.size_px).is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test fonts::tests render::atlas::tests render::glyph_cache::tests -v`
Expected: FAIL with missing `FontSet`, `Atlas`, `GlyphCache`.

- [ ] **Step 3: Implement `fonts.rs`**

```rust
use ab_glyph::{Font, FontVec, GlyphId, PxScale, ScaleFont};
use fontdb::{Database, Family, Query, Stretch, Style, Weight};

#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("no usable monospace font found")]
    NoFont,
    #[error("font data is invalid")]
    InvalidFont,
}

pub struct FontSet {
    pub font: FontVec,
    pub size_px: f32,
    pub cell_w: u32,
    pub cell_h: u32,
    pub ascent: f32,
    pub descent: f32,
}

impl FontSet {
    pub fn load(family: &str, size_px: f32, line_height: f32) -> Result<Self, FontError> {
        let mut database = Database::new();
        database.load_system_fonts();
        let query = Query {
            families: &[Family::Name(family), Family::Monospace],
            weight: Weight::NORMAL,
            stretch: Stretch::Normal,
            style: Style::Normal,
        };
        let id = database
            .query(&query)
            .or_else(|| {
                database.query(&Query {
                    families: &[Family::Monospace],
                    weight: Weight::NORMAL,
                    stretch: Stretch::Normal,
                    style: Style::Normal,
                })
            })
            .ok_or(FontError::NoFont)?;
        let (data, index) = database
            .with_face_data(id, |data, index| (data.to_vec(), index))
            .ok_or(FontError::NoFont)?;
        let font =
            FontVec::try_from_vec_and_index(data, index).map_err(|_| FontError::InvalidFont)?;

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
            size_px,
            cell_w,
            cell_h,
            ascent,
            descent,
        })
    }

    pub fn glyph(&self, ch: char) -> Option<GlyphId> {
        let glyph = self.font.glyph_id(ch);
        (glyph.0 != 0).then_some(glyph)
    }

    pub fn scale(&self) -> PxScale {
        PxScale::from(self.size_px)
    }
}
```

- [ ] **Step 4: Implement `render/atlas.rs`**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, Copy)]
struct Shelf {
    y: u32,
    height: u32,
    x: u32,
}

pub struct Atlas {
    width: u32,
    height: u32,
    data: Vec<u8>,
    shelves: Vec<Shelf>,
}

impl Atlas {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; (width as usize) * (height as usize)],
            shelves: Vec::new(),
        }
    }

    pub fn alloc(&mut self, w: u32, h: u32) -> Option<Rect> {
        if w == 0 || h == 0 || w > self.width || h > self.height {
            return None;
        }
        for shelf in &mut self.shelves {
            if shelf.height >= h && shelf.x + w <= self.width {
                let rect = Rect { x: shelf.x, y: shelf.y, w, h };
                shelf.x += w;
                return Some(rect);
            }
        }
        let y = self.shelves.last().map_or(0, |shelf| shelf.y + shelf.height);
        if y + h > self.height {
            return None;
        }
        self.shelves.push(Shelf { y, height: h, x: w });
        Some(Rect { x: 0, y, w, h })
    }

    pub fn upload(&mut self, rect: Rect, mask: &[u8]) {
        debug_assert_eq!(mask.len(), (rect.w * rect.h) as usize);
        for row in 0..rect.h {
            let src = (row * rect.w) as usize;
            let dst = ((rect.y + row) * self.width + rect.x) as usize;
            self.data[dst..dst + rect.w as usize]
                .copy_from_slice(&mask[src..src + rect.w as usize]);
        }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn width(&self) -> u32 {
        self.width
    }
}
```

- [ ] **Step 5: Implement `render/glyph_cache.rs`**

```rust
use std::collections::HashMap;

use ab_glyph::{Font, FontVec, GlyphId, PxScale};

use super::atlas::{Atlas, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub glyph: u16,
    pub size_px: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct GlyphSlot {
    pub rect: Rect,
    pub bearing_x: i32,
    pub bearing_y: i32,
}

pub struct GlyphCache {
    pub atlas: Atlas,
    slots: HashMap<GlyphKey, Option<GlyphSlot>>,
}

impl GlyphCache {
    pub fn new() -> Self {
        Self { atlas: Atlas::new(2048, 2048), slots: HashMap::new() }
    }

    pub fn get(
        &mut self,
        font: &FontVec,
        glyph: GlyphId,
        size_px: f32,
    ) -> Option<GlyphSlot> {
        let key = GlyphKey {
            glyph: glyph.0,
            size_px: size_px.round() as u32,
        };
        if let Some(slot) = self.slots.get(&key) {
            return *slot;
        }
        let slot = self.rasterize(font, glyph, size_px);
        self.slots.insert(key, slot);
        slot
    }

    fn rasterize(&mut self, font: &FontVec, glyph: GlyphId, size_px: f32) -> Option<GlyphSlot> {
        let scale = PxScale::from(size_px);
        let outlined = font.outline_glyph(glyph.with_scale(scale))?;
        let bounds = outlined.px_bounds();
        let width = bounds.width().ceil() as u32;
        let height = bounds.height().ceil() as u32;
        if width == 0 || height == 0 {
            return None;
        }
        let rect = self.atlas.alloc(width, height)?;
        let mut mask = vec![0u8; (width * height) as usize];
        outlined.draw(|x, y, coverage| {
            if x < width && y < height {
                mask[(y * width + x) as usize] = (coverage * 255.0).round() as u8;
            }
        });
        self.atlas.upload(rect, &mask);
        Some(GlyphSlot {
            rect,
            bearing_x: bounds.min.x.round() as i32,
            bearing_y: bounds.min.y.round() as i32,
        })
    }
}

impl Default for GlyphCache {
    fn default() -> Self {
        Self::new()
    }
}
```


- [ ] **Step 6: Implement text rendering in `render/mod.rs`**

Replace `src/render/mod.rs` with:
```rust
mod atlas;
mod glyph_cache;

use tiny_skia::Pixmap;

use crate::fonts::FontSet;
use crate::theme::{Rgb, Theme};
use atlas::Atlas;
use glyph_cache::{GlyphCache, GlyphSlot};

pub struct Renderer {
    pixmap: Pixmap,
    pixels: Vec<u32>,
    pub fonts: FontSet,
    pub cache: GlyphCache,
    pub theme: Theme,
    padding: u32,
}

impl Renderer {
    pub fn new(fonts: FontSet, theme: Theme, padding: u32) -> Self {
        Self {
            pixmap: Pixmap::new(1, 1).expect("1x1 pixmap"),
            pixels: vec![0],
            fonts,
            cache: GlyphCache::new(),
            theme,
            padding,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.pixmap = Pixmap::new(width, height).expect("pixmap allocation");
        self.pixels = vec![0; (width as usize) * (height as usize)];
        self.pixmap.fill(self.theme.background.to_tiny());
        self.sync();
    }

    pub fn draw(&mut self) {
        self.pixmap.fill(self.theme.background.to_tiny());
    }

    pub fn draw_text(&mut self, x: f32, baseline: f32, text: &str, color: Rgb) {
        let mut pen = x;
        let mut base_pen = x;
        for ch in text.chars() {
            let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            let draw_pen = if width == 0 { base_pen } else { pen };
            if let Some(glyph) = self.fonts.glyph(ch) {
                if let Some(slot) = self.cache.get(&self.fonts.font, glyph, self.fonts.size_px) {
                    blit(&mut self.pixmap, &self.cache.atlas, slot, draw_pen, baseline, color);
                }
            }
            if width > 0 {
                base_pen = pen;
                pen += self.fonts.cell_w as f32;
            }
        }
    }

    pub fn sync(&mut self) {
        for (index, pixel) in self.pixmap.data().chunks_exact(4).enumerate() {
            self.pixels[index] =
                ((pixel[0] as u32) << 16) | ((pixel[1] as u32) << 8) | pixel[2] as u32;
        }
    }

    pub fn pixels(&self) -> &[u32] {
        &self.pixels
    }

    pub fn padding(&self) -> u32 {
        self.padding
    }
}

fn blit(
    pixmap: &mut Pixmap,
    atlas: &Atlas,
    slot: GlyphSlot,
    pen_x: f32,
    baseline: f32,
    color: Rgb,
) {
    let width = pixmap.width() as i32;
    let height = pixmap.height() as i32;
    let stride = width as usize * 4;
    let dst_x = pen_x.round() as i32 + slot.bearing_x;
    let dst_y = baseline.round() as i32 + slot.bearing_y;
    let atlas_width = atlas.width();
    let data = pixmap.data_mut();
    for row in 0..slot.rect.h as i32 {
        for col in 0..slot.rect.w as i32 {
            let x = dst_x + col;
            let y = dst_y + row;
            if x < 0 || y < 0 || x >= width || y >= height {
                continue;
            }
            let mask_x = slot.rect.x as i32 + col;
            let mask_y = slot.rect.y as i32 + row;
            let coverage = atlas.data()[(mask_y as u32 * atlas_width + mask_x as u32) as usize];
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

fn blend(dst: u8, src: u8, alpha: f32) -> u8 {
    (src as f32 * alpha + dst as f32 * (1.0 - alpha)).round() as u8
}
```

- [ ] **Step 7: Wire fonts into `app.rs`**

In `resumed`, replace the renderer construction:
```rust
let fonts = match crate::fonts::FontSet::load(
    &self.config.font.family,
    self.config.font.size,
    self.config.font.line_height,
) {
    Ok(fonts) => fonts,
    Err(error) => {
        rfd::MessageDialog::new()
            .set_title("Termyx")
            .set_description(format!("Could not load a font: {error}"))
            .set_level(rfd::MessageLevel::Error)
            .show();
        event_loop.exit();
        return;
    }
};
let mut renderer = Renderer::new(
    fonts,
    Theme::from_config(&self.config.theme),
    self.config.window.padding,
);
```
In `RedrawRequested`, after `renderer.draw();`, add:
```rust
let padding = renderer.padding() as f32;
let baseline = padding + renderer.fonts.ascent;
let foreground = renderer.theme.foreground;
renderer.draw_text(padding, baseline, "Hello, Termyx", foreground);
renderer.sync();
```

- [ ] **Step 8: Build, test, and inspect**

Run:
```powershell
cargo test
cargo run
```
Expected: tests pass; the window shows "Hello, Termyx" in monospace at the top-left.

- [ ] **Step 9: Commit**

```powershell
git add src/fonts.rs src/render src/app.rs
git commit -m "feat: render monospace text through a glyph atlas"
```

---

### Task 5: PTY spawn, reader thread, writer, and resize

**Files:**
- Modify: `src/event.rs`
- Modify: `src/pty.rs`
- Test: `src/pty.rs` (inline)

**Interfaces:**
- Consumes: `ShellConfig`, `EventLoopProxy<UserEvent>`.
- Produces: `UserEvent::PtyOutput(Vec<u8>)`, `PtySession::spawn(&ShellConfig, u16, u16, EventLoopProxy<UserEvent>)`, `PtySession::spawn_with(&ShellConfig, u16, u16, impl Fn(Vec<u8>) + Send + 'static)`, `PtySession::write(&mut self, &[u8])`, `PtySession::resize(&self, u16, u16)`.

- [ ] **Step 1: Write the failing test**

Append to `src/pty.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn spawns_a_command_and_reads_its_output() {
        let mut shell = ShellConfig::default();
        #[cfg(windows)]
        {
            shell.program = "cmd.exe".into();
            shell.args = vec!["/C".into(), "echo hello".into()];
        }
        #[cfg(not(windows))]
        {
            shell.program = "/bin/sh".into();
            shell.args = vec!["-c".into(), "echo hello".into()];
        }

        let (sender, receiver) = mpsc::channel();
        let _session = PtySession::spawn_with(&shell, 80, 24, move |bytes| {
            let _ = sender.send(bytes);
        })
        .expect("spawn pty");

        let mut output = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match receiver.recv_timeout(Duration::from_millis(200)) {
                Ok(chunk) => {
                    output.extend_from_slice(&chunk);
                    if String::from_utf8_lossy(&output).contains("hello") {
                        break;
                    }
                }
                Err(_) => {}
            }
        }
        assert!(
            String::from_utf8_lossy(&output).contains("hello"),
            "pty output was {:?}",
            String::from_utf8_lossy(&output)
        );
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test pty::tests -v`
Expected: FAIL with `cannot find function spawn_with` / `PtySession`.

- [ ] **Step 3: Implement `event.rs`**

```rust
#[derive(Debug)]
pub enum UserEvent {
    PtyOutput(Vec<u8>),
}
```

- [ ] **Step 4: Implement `pty.rs`**

```rust
use std::io::{Read, Write};
use std::thread;

use anyhow::Result;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use winit::event_loop::EventLoopProxy;

use crate::config::ShellConfig;
use crate::event::UserEvent;

pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    _reader: Option<thread::JoinHandle<()>>,
}

impl PtySession {
    pub fn spawn(
        config: &ShellConfig,
        cols: u16,
        rows: u16,
        proxy: EventLoopProxy<UserEvent>,
    ) -> Result<Self> {
        Self::spawn_with(config, cols, rows, move |bytes| {
            let _ = proxy.send_event(UserEvent::PtyOutput(bytes));
        })
    }

    pub fn spawn_with<F>(
        config: &ShellConfig,
        cols: u16,
        rows: u16,
        on_output: F,
    ) -> Result<Self>
    where
        F: Fn(Vec<u8>) + Send + 'static,
    {
        let (program, args) = build_command(config);
        let pair = native_pty_system().openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut command = CommandBuilder::new(program);
        for arg in args {
            command.arg(arg);
        }
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "termyx");

        let child = pair.slave.spawn_command(command)?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;
        let reader_thread = thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => on_output(buffer[..n].to_vec()),
                }
            }
        });

        Ok(Self {
            master: pair.master,
            writer,
            child,
            _reader: Some(reader_thread),
        })
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn build_command(config: &ShellConfig) -> (String, Vec<String>) {
    if !config.program.is_empty() {
        let mut args = config.args.clone();
        #[cfg(windows)]
        if config.program.to_ascii_lowercase().ends_with("cmd.exe") && args.is_empty() {
            args = vec!["/K".into(), "chcp 65001>nul".into()];
        }
        return (config.program.clone(), args);
    }

    #[cfg(windows)]
    {
        let program = if which("pwsh.exe") { "pwsh.exe" } else { "powershell.exe" };
        let args = vec![
            "-NoLogo".into(),
            "-NoExit".into(),
            "-Command".into(),
            "[Console]::OutputEncoding=[Text.Encoding]::UTF8; [Console]::InputEncoding=[Text.Encoding]::UTF8".into(),
        ];
        return (program.into(), args);
    }

    #[cfg(not(windows))]
    {
        let program = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        (program, config.args.clone())
    }
}

fn which(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|path| path.join(program).is_file()))
        .unwrap_or(false)
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test pty::tests -v`
Expected: PASS. If it times out, verify `cmd.exe` exists and that the test is not running under a restricted environment.

- [ ] **Step 6: Commit**

```powershell
git add src/pty.rs src/event.rs
git commit -m "feat: spawn a shell in a PTY and stream its output"
```

---

### Task 6: Terminal wrapper, logical snapshots, mode, and scroll

**Files:**
- Modify: `Cargo.toml` (add `vte = "0.15"`)
- Modify: `src/term.rs`
- Test: `src/term.rs` (inline)

**Interfaces:**
- Consumes: `alacritty_terminal` 0.26, `vte` 0.15.
- Produces: `Color`, `Style { fg, bg }`, `Cell { text, width, style }`, `RowSnapshot`, `GridSnapshot`, `Affinity`, `CursorState { row, logical, affinity }`, `LogicalPos`, `Terminal::new(cols, rows, scrollback)`, `feed`, `resize`, `snapshot`, `mode`, `scroll_display`.

- [ ] **Step 1: Write the failing tests**

Append to `src/term.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_plain_text_and_track_cursor() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"hello");
        let snapshot = term.snapshot();
        assert_eq!(snapshot.lines[0].cells[0].text, "h");
        assert_eq!(snapshot.lines[0].cells[4].text, "o");
        assert_eq!(snapshot.cursor.row, 0);
        assert_eq!(snapshot.cursor.logical, 5);
    }

    #[test]
    fn sgr_color_is_captured() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"\x1b[31mred");
        let snapshot = term.snapshot();
        assert_eq!(snapshot.lines[0].cells[0].style.fg, Color::Indexed(1));
    }

    #[test]
    fn newline_moves_cursor_to_next_row() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"a\r\nb");
        let snapshot = term.snapshot();
        assert_eq!(snapshot.lines[1].cells[0].text, "b");
        assert_eq!(snapshot.cursor.row, 1);
        assert_eq!(snapshot.cursor.logical, 1);
    }

    #[test]
    fn wide_char_occupies_two_cells() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed("中".as_bytes());
        let snapshot = term.snapshot();
        assert_eq!(snapshot.lines[0].cells[0].text, "中");
        assert_eq!(snapshot.lines[0].cells[0].width, 2);
        assert_eq!(snapshot.lines[0].cells[1].width, 0);
    }

    #[test]
    fn resize_reflows_visible_text() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"abcdefghijklmnopqrstuvwxy");
        term.resize(10, 5);
        let snapshot = term.snapshot();
        let text: String = snapshot
            .lines
            .iter()
            .flat_map(|row| row.cells.iter().map(|cell| cell.text.as_str()))
            .collect::<String>()
            .replace(' ', "");
        assert!(text.contains("abcdefghijklmnopqrstuvwxy"), "got {text:?}");
    }

    #[test]
    fn alt_screen_mode_is_exposed() {
        let mut term = Terminal::new(20, 5, 1_000);
        term.feed(b"\x1b[?1049h");
        assert!(term.mode().contains(TermMode::ALT_SCREEN));
    }

    #[test]
    fn scroll_display_moves_viewport_offset() {
        let mut term = Terminal::new(20, 3, 1_000);
        for index in 0..10 {
            term.feed(format!("line{index}\r\n").as_bytes());
        }
        term.scroll_display(2);
        assert_eq!(term.snapshot().display_offset, 2);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test term::tests -v`
Expected: FAIL with `cannot find type Terminal` / `GridSnapshot`.

- [ ] **Step 3: Add `vte` to `Cargo.toml`**

Add under `[dependencies]`:
```toml
vte = "0.15"
```

- [ ] **Step 4: Implement `term.rs`**

```rust
use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::cell::{Cell as TermCell, Flags};
use alacritty_terminal::term::{Config as TermConfig, Term, TermMode};
use unicode_width::UnicodeWidthStr;
use vte::ansi::{Color as AnsiColor, NamedColor, Processor};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Option<Color>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub text: String,
    pub width: u8,
    pub style: Style,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowSnapshot {
    pub cells: Vec<Cell>,
    pub wrapped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Affinity {
    Before,
    After,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorState {
    pub row: usize,
    pub logical: usize,
    pub affinity: Affinity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LogicalPos {
    pub row: usize,
    pub col: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridSnapshot {
    pub cols: usize,
    pub rows: usize,
    pub lines: Vec<RowSnapshot>,
    pub cursor: CursorState,
    pub display_offset: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct TermSize {
    pub cols: usize,
    pub rows: usize,
    pub total_lines: usize,
}

impl Dimensions for TermSize {
    fn columns(&self) -> usize {
        self.cols
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn total_lines(&self) -> usize {
        self.total_lines
    }
}

#[derive(Clone)]
struct EventProxy;

impl EventListener for EventProxy {
    fn send_event(&self, _event: alacritty_terminal::event::Event) {}
}

pub struct Terminal {
    term: Term<EventProxy>,
    processor: Processor,
    size: TermSize,
    scrollback: usize,
}

impl Terminal {
    pub fn new(cols: usize, rows: usize, scrollback: usize) -> Self {
        let size = TermSize { cols, rows, total_lines: rows + scrollback };
        let config = TermConfig { scrolling_history: scrollback, ..TermConfig::default() };
        let term = Term::new(config, &size, EventProxy);
        Self {
            term,
            processor: Processor::new(),
            size,
            scrollback,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.size = TermSize { cols, rows, total_lines: rows + self.scrollback };
        self.term.resize(self.size);
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn scroll_display(&mut self, delta: i32) {
        self.term.scroll_display(Scroll::Delta(delta));
    }

    pub fn snapshot(&self) -> GridSnapshot {
        let content = self.term.renderable_content();
        let cols = self.size.cols;
        let rows = self.size.rows;
        let mut lines = Vec::with_capacity(rows);
        let mut cells = Vec::with_capacity(cols);
        let mut wrapped = false;

        for (index, indexed) in content.display_iter.enumerate() {
            let cell = indexed.cell;
            if index > 0 && index % cols == 0 {
                lines.push(RowSnapshot { cells: std::mem::take(&mut cells), wrapped });
                cells = Vec::with_capacity(cols);
            }
            wrapped = cell.flags.contains(Flags::WRAPLINE);
            cells.push(convert_cell(cell));
        }
        if !cells.is_empty() {
            lines.push(RowSnapshot { cells, wrapped });
        }
        lines.truncate(rows);
        while lines.len() < rows {
            lines.push(RowSnapshot {
                cells: vec![Cell { text: " ".into(), width: 1, style: Style::default() }; cols],
                wrapped: false,
            });
        }

        let cursor = if content.display_offset == 0 {
            let row = content.cursor.point.line.0.max(0) as usize;
            CursorState {
                row: row.min(rows.saturating_sub(1)),
                logical: content.cursor.point.column.0.min(cols),
                affinity: Affinity::Before,
            }
        } else {
            CursorState { row: 0, logical: 0, affinity: Affinity::Before }
        };

        GridSnapshot {
            cols,
            rows,
            lines,
            cursor,
            display_offset: content.display_offset,
        }
    }
}

fn convert_cell(cell: &TermCell) -> Cell {
    let style = Style {
        fg: map_fg(cell.fg),
        bg: map_bg(cell.bg),
    };

    if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
        return Cell { text: String::new(), width: 0, style };
    }

    let mut text = String::new();
    let ch = if cell.c == '\0' { ' ' } else { cell.c };
    text.push(ch);
    if let Some(extra) = cell.zerowidth() {
        for ch in extra {
            text.push(*ch);
        }
    }
    let width = UnicodeWidthStr::width(text.as_str()).clamp(1, 2) as u8;
    Cell { text, width, style }
}

fn map_fg(color: AnsiColor) -> Color {
    match color {
        AnsiColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
        AnsiColor::Indexed(index) => Color::Indexed(index),
        AnsiColor::Named(named) => named_index(named).map(Color::Indexed).unwrap_or(Color::Default),
    }
}

fn map_bg(color: AnsiColor) -> Option<Color> {
    match color {
        AnsiColor::Named(NamedColor::Background) => None,
        AnsiColor::Named(NamedColor::Foreground) => None,
        other => {
            let mapped = map_fg(other);
            if mapped == Color::Default { None } else { Some(mapped) }
        }
    }
}

fn named_index(named: NamedColor) -> Option<u8> {
    Some(match named {
        NamedColor::Black => 0,
        NamedColor::Red => 1,
        NamedColor::Green => 2,
        NamedColor::Yellow => 3,
        NamedColor::Blue => 4,
        NamedColor::Magenta => 5,
        NamedColor::Cyan => 6,
        NamedColor::White => 7,
        NamedColor::BrightBlack => 8,
        NamedColor::BrightRed => 9,
        NamedColor::BrightGreen => 10,
        NamedColor::BrightYellow => 11,
        NamedColor::BrightBlue => 12,
        NamedColor::BrightMagenta => 13,
        NamedColor::BrightCyan => 14,
        NamedColor::BrightWhite => 15,
        _ => return None,
    })
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test term::tests -v`
Expected: 7 tests pass.

- [ ] **Step 6: Commit**

```powershell
git add Cargo.toml src/term.rs
git commit -m "feat: wrap alacritty_terminal and expose logical snapshots"
```

---

---

### Task 7: Input encoder

**Files:**
- Modify: `src/input.rs`

**Interfaces:**
- Consumes: `alacritty_terminal::term::TermMode` (bitflags).
- Produces: `KeyInput`, `Mods`, `encode(KeyInput, Mods, TermMode) -> Option<Vec<u8>>`,
  `paste(&str, bool) -> Vec<u8>`.

- [ ] **Step 1: Write the failing tests**

Replace `src/input.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::term::TermMode;

    fn plain() -> Mods {
        Mods { ctrl: false, alt: false }
    }

    #[test]
    fn named_keys_use_standard_bytes() {
        let mode = TermMode::empty();
        assert_eq!(encode(KeyInput::Enter, plain(), mode), Some(b"\r".to_vec()));
        assert_eq!(encode(KeyInput::Backspace, plain(), mode), Some(vec![0x7f]));
        assert_eq!(encode(KeyInput::Tab, plain(), mode), Some(b"\t".to_vec()));
        assert_eq!(encode(KeyInput::Escape, plain(), mode), Some(vec![0x1b]));
    }

    #[test]
    fn arrows_follow_application_cursor_mode() {
        let normal = TermMode::empty();
        let app = TermMode::APP_CURSOR;
        assert_eq!(encode(KeyInput::Up, plain(), normal), Some(b"\x1b[A".to_vec()));
        assert_eq!(encode(KeyInput::Up, plain(), app), Some(b"\x1bOA".to_vec()));
        assert_eq!(encode(KeyInput::Left, plain(), normal), Some(b"\x1b[D".to_vec()));
        assert_eq!(encode(KeyInput::Left, plain(), app), Some(b"\x1bOD".to_vec()));
    }

    #[test]
    fn navigation_keys_use_csi_sequences() {
        let mode = TermMode::empty();
        assert_eq!(encode(KeyInput::Home, plain(), mode), Some(b"\x1b[H".to_vec()));
        assert_eq!(encode(KeyInput::End, plain(), mode), Some(b"\x1b[F".to_vec()));
        assert_eq!(encode(KeyInput::Insert, plain(), mode), Some(b"\x1b[2~".to_vec()));
        assert_eq!(encode(KeyInput::Delete, plain(), mode), Some(b"\x1b[3~".to_vec()));
        assert_eq!(encode(KeyInput::PageUp, plain(), mode), Some(b"\x1b[5~".to_vec()));
        assert_eq!(encode(KeyInput::PageDown, plain(), mode), Some(b"\x1b[6~".to_vec()));
    }

    #[test]
    fn ctrl_letter_becomes_a_control_byte() {
        let mode = TermMode::empty();
        let mods = Mods { ctrl: true, alt: false };
        assert_eq!(encode(KeyInput::Char('c'), mods, mode), Some(vec![3]));
        assert_eq!(encode(KeyInput::Char('A'), mods, mode), Some(vec![1]));
        assert_eq!(encode(KeyInput::Char('1'), mods, mode), None);
    }

    #[test]
    fn alt_prefixes_escape() {
        let mode = TermMode::empty();
        let mods = Mods { ctrl: false, alt: true };
        assert_eq!(encode(KeyInput::Char('x'), mods, mode), Some(vec![0x1b, b'x']));
    }

    #[test]
    fn text_is_encoded_as_utf8() {
        let mode = TermMode::empty();
        assert_eq!(encode(KeyInput::Char('é'), plain(), mode), Some(vec![0xc3, 0xa9]));
        assert_eq!(encode(KeyInput::Char(' '), plain(), mode), Some(b" ".to_vec()));
    }

    #[test]
    fn paste_wraps_only_when_bracketed() {
        assert_eq!(paste("hi", false), b"hi".to_vec());
        assert_eq!(paste("hi", true), b"\x1b[200~hi\x1b[201~".to_vec());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test input::tests -v`

Expected: FAIL — `cannot find function 'encode'` and `cannot find function 'paste'`.

- [ ] **Step 3: Write the implementation**

Replace `src/input.rs` with:

```rust
use alacritty_terminal::term::TermMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyInput {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Delete,
    Insert,
    PageUp,
    PageDown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
}

/// Encode a logical key press as bytes for the PTY.
///
/// Shift is already reflected in `KeyInput::Char` by the windowing layer.
pub fn encode(key: KeyInput, mods: Mods, mode: TermMode) -> Option<Vec<u8>> {
    let bytes = match key {
        KeyInput::Char(ch) if mods.ctrl => {
            let lower = ch.to_ascii_lowercase();
            if !lower.is_ascii_lowercase() {
                return None;
            }
            vec![(lower as u8) - b'a' + 1]
        }
        KeyInput::Char(ch) => {
            let mut buf = [0u8; 4];
            ch.encode_utf8(&mut buf).as_bytes().to_vec()
        }
        KeyInput::Enter => b"\r".to_vec(),
        KeyInput::Backspace => vec![0x7f],
        KeyInput::Tab => b"\t".to_vec(),
        KeyInput::Escape => vec![0x1b],
        KeyInput::Up => cursor_key(b'A', mode),
        KeyInput::Down => cursor_key(b'B', mode),
        KeyInput::Right => cursor_key(b'C', mode),
        KeyInput::Left => cursor_key(b'D', mode),
        KeyInput::Home => b"\x1b[H".to_vec(),
        KeyInput::End => b"\x1b[F".to_vec(),
        KeyInput::Insert => b"\x1b[2~".to_vec(),
        KeyInput::Delete => b"\x1b[3~".to_vec(),
        KeyInput::PageUp => b"\x1b[5~".to_vec(),
        KeyInput::PageDown => b"\x1b[6~".to_vec(),
    };

    if mods.alt {
        let mut prefixed = Vec::with_capacity(bytes.len() + 1);
        prefixed.push(0x1b);
        prefixed.extend_from_slice(&bytes);
        Some(prefixed)
    } else {
        Some(bytes)
    }
}

fn cursor_key(final_byte: u8, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::APP_CURSOR) {
        vec![0x1b, b'O', final_byte]
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

/// Encode pasted text, optionally using xterm bracketed paste.
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    if !bracketed {
        return text.as_bytes().to_vec();
    }

    let mut bytes = Vec::with_capacity(text.len() + 12);
    bytes.extend_from_slice(b"\x1b[200~");
    bytes.extend_from_slice(text.as_bytes());
    bytes.extend_from_slice(b"\x1b[201~");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> Mods {
        Mods { ctrl: false, alt: false }
    }

    #[test]
    fn named_keys_use_standard_bytes() {
        let mode = TermMode::empty();
        assert_eq!(encode(KeyInput::Enter, plain(), mode), Some(b"\r".to_vec()));
        assert_eq!(encode(KeyInput::Backspace, plain(), mode), Some(vec![0x7f]));
        assert_eq!(encode(KeyInput::Tab, plain(), mode), Some(b"\t".to_vec()));
        assert_eq!(encode(KeyInput::Escape, plain(), mode), Some(vec![0x1b]));
    }

    #[test]
    fn arrows_follow_application_cursor_mode() {
        let normal = TermMode::empty();
        let app = TermMode::APP_CURSOR;
        assert_eq!(encode(KeyInput::Up, plain(), normal), Some(b"\x1b[A".to_vec()));
        assert_eq!(encode(KeyInput::Up, plain(), app), Some(b"\x1bOA".to_vec()));
        assert_eq!(encode(KeyInput::Left, plain(), normal), Some(b"\x1b[D".to_vec()));
        assert_eq!(encode(KeyInput::Left, plain(), app), Some(b"\x1bOD".to_vec()));
    }

    #[test]
    fn navigation_keys_use_csi_sequences() {
        let mode = TermMode::empty();
        assert_eq!(encode(KeyInput::Home, plain(), mode), Some(b"\x1b[H".to_vec()));
        assert_eq!(encode(KeyInput::End, plain(), mode), Some(b"\x1b[F".to_vec()));
        assert_eq!(encode(KeyInput::Insert, plain(), mode), Some(b"\x1b[2~".to_vec()));
        assert_eq!(encode(KeyInput::Delete, plain(), mode), Some(b"\x1b[3~".to_vec()));
        assert_eq!(encode(KeyInput::PageUp, plain(), mode), Some(b"\x1b[5~".to_vec()));
        assert_eq!(encode(KeyInput::PageDown, plain(), mode), Some(b"\x1b[6~".to_vec()));
    }

    #[test]
    fn ctrl_letter_becomes_a_control_byte() {
        let mode = TermMode::empty();
        let mods = Mods { ctrl: true, alt: false };
        assert_eq!(encode(KeyInput::Char('c'), mods, mode), Some(vec![3]));
        assert_eq!(encode(KeyInput::Char('A'), mods, mode), Some(vec![1]));
        assert_eq!(encode(KeyInput::Char('1'), mods, mode), None);
    }

    #[test]
    fn alt_prefixes_escape() {
        let mode = TermMode::empty();
        let mods = Mods { ctrl: false, alt: true };
        assert_eq!(encode(KeyInput::Char('x'), mods, mode), Some(vec![0x1b, b'x']));
    }

    #[test]
    fn text_is_encoded_as_utf8() {
        let mode = TermMode::empty();
        assert_eq!(encode(KeyInput::Char('é'), plain(), mode), Some(vec![0xc3, 0xa9]));
        assert_eq!(encode(KeyInput::Char(' '), plain(), mode), Some(b" ".to_vec()));
    }

    #[test]
    fn paste_wraps_only_when_bracketed() {
        assert_eq!(paste("hi", false), b"hi".to_vec());
        assert_eq!(paste("hi", true), b"\x1b[200~hi\x1b[201~".to_vec());
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test input::tests -v`

Expected: PASS — 7 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/input.rs
git commit -m "feat: add terminal input encoder"
```

---

### Task 8: Identity visual map

**Files:**
- Modify: `src/visual.rs`

**Interfaces:**
- Consumes: `crate::term::{Affinity, CursorState}`.
- Produces: `VisualMap { identity(usize), visual_of(usize) -> usize,
  logical_of(usize) -> usize, cursor_visual(&CursorState) -> usize,
  logical_at(usize) -> usize }`.

- [ ] **Step 1: Write the failing tests**

Replace `src/visual.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{Affinity, CursorState};

    fn cursor(logical: usize, affinity: Affinity) -> CursorState {
        CursorState { row: 0, logical, affinity }
    }

    #[test]
    fn identity_round_trips_every_cell() {
        let map = VisualMap::identity(80);
        for col in 0..80 {
            assert_eq!(map.logical_of(map.visual_of(col)), col);
            assert_eq!(map.visual_of(map.logical_of(col)), col);
        }
    }

    #[test]
    fn lookups_clamp_to_the_row() {
        let map = VisualMap::identity(10);
        assert_eq!(map.visual_of(99), 9);
        assert_eq!(map.logical_of(99), 9);
        assert_eq!(map.logical_at(99), 9);
    }

    #[test]
    fn cursor_before_a_cell_sits_on_its_left_edge() {
        let map = VisualMap::identity(10);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::Before)), 3);
        assert_eq!(map.cursor_visual(&cursor(10, Affinity::Before)), 10);
    }

    #[test]
    fn cursor_after_a_cell_sits_on_its_right_edge() {
        let map = VisualMap::identity(10);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::After)), 4);
        assert_eq!(map.cursor_visual(&cursor(10, Affinity::After)), 10);
        assert_eq!(map.cursor_visual(&cursor(0, Affinity::After)), 0);
    }

    #[test]
    fn hit_testing_returns_the_cell_at_a_visual_column() {
        let map = VisualMap::identity(10);
        assert_eq!(map.logical_at(0), 0);
        assert_eq!(map.logical_at(4), 4);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test visual::tests -v`

Expected: FAIL — `cannot find type 'VisualMap' in this scope`.

- [ ] **Step 3: Write the implementation**

Replace `src/visual.rs` with:

```rust
use crate::term::{Affinity, CursorState};

/// Maps logical cell columns to visual columns for one row.
///
/// Phase 1 is identity. Phase 2 replaces the constructor with UAX#9 output;
/// every caller already goes through this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualMap {
    cols: usize,
    logical_to_visual: Vec<usize>,
    visual_to_logical: Vec<usize>,
}

impl VisualMap {
    pub fn identity(cols: usize) -> Self {
        Self {
            cols,
            logical_to_visual: (0..cols).collect(),
            visual_to_logical: (0..cols).collect(),
        }
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
                    (self.visual_of(cursor.logical.saturating_sub(1)) + 1).min(self.cols)
                }
            }
        }
    }

    /// Logical cell under a visual column (mouse hit-testing).
    pub fn logical_at(&self, visual: usize) -> usize {
        self.logical_of(visual)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cursor(logical: usize, affinity: Affinity) -> CursorState {
        CursorState { row: 0, logical, affinity }
    }

    #[test]
    fn identity_round_trips_every_cell() {
        let map = VisualMap::identity(80);
        for col in 0..80 {
            assert_eq!(map.logical_of(map.visual_of(col)), col);
            assert_eq!(map.visual_of(map.logical_of(col)), col);
        }
    }

    #[test]
    fn lookups_clamp_to_the_row() {
        let map = VisualMap::identity(10);
        assert_eq!(map.visual_of(99), 9);
        assert_eq!(map.logical_of(99), 9);
        assert_eq!(map.logical_at(99), 9);
    }

    #[test]
    fn cursor_before_a_cell_sits_on_its_left_edge() {
        let map = VisualMap::identity(10);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::Before)), 3);
        assert_eq!(map.cursor_visual(&cursor(10, Affinity::Before)), 10);
    }

    #[test]
    fn cursor_after_a_cell_sits_on_its_right_edge() {
        let map = VisualMap::identity(10);
        assert_eq!(map.cursor_visual(&cursor(3, Affinity::After)), 4);
        assert_eq!(map.cursor_visual(&cursor(10, Affinity::After)), 10);
        assert_eq!(map.cursor_visual(&cursor(0, Affinity::After)), 0);
    }

    #[test]
    fn hit_testing_returns_the_cell_at_a_visual_column() {
        let map = VisualMap::identity(10);
        assert_eq!(map.logical_at(0), 0);
        assert_eq!(map.logical_at(4), 4);
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test visual::tests -v`

Expected: PASS — 5 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src/visual.rs
git commit -m "feat: add identity visual map"
```

---

### Task 9: App wiring (PTY → terminal → renderer, keyboard → PTY)

**Files:**
- Modify: `src/app.rs`
- Modify: `src/render/mod.rs`

**Interfaces:**
- Consumes: `Config`, `UserEvent`, `FontSet`, `input::{encode, KeyInput, Mods}`,
  `PtySession`, `Renderer`, `Terminal`, `Theme`, `VisualMap`.
- Produces: `App::new(Config, EventLoopProxy<UserEvent>)`; `Renderer::draw(&GridSnapshot, &[VisualMap])`,
  `Renderer::fonts() -> &FontSet`, `Renderer::padding() -> u32`.

- [ ] **Step 1: Write the failing tests**

Add this test module to the end of `src/app.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_size_uses_padding_and_minimums() {
        assert_eq!(grid_size(1200, 800, 10, 20, 4), (119, 39));
        assert_eq!(grid_size(100, 100, 10, 20, 4), (20, 5));
    }

    #[test]
    fn map_key_recognizes_text_and_named_keys() {
        assert_eq!(
            map_key(&Key::Character("a".into())),
            Some(KeyInput::Char('a'))
        );
        assert_eq!(map_key(&Key::Named(NamedKey::Enter)), Some(KeyInput::Enter));
        assert_eq!(map_key(&Key::Named(NamedKey::F1)), None);
    }

    #[test]
    fn modifiers_map_to_ctrl_and_alt() {
        let mods = mods_from(ModifiersState::CONTROL | ModifiersState::ALT);
        assert!(mods.ctrl);
        assert!(mods.alt);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test app::tests -v`

Expected: FAIL — `cannot find function 'grid_size'` / `'map_key'`.

- [ ] **Step 3: Replace `src/app.rs` with the full wiring**

```rust
use std::num::NonZeroU32;
use std::sync::Arc;

use alacritty_terminal::term::TermMode;
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use crate::config::Config;
use crate::event::UserEvent;
use crate::fonts::FontSet;
use crate::input::{self, KeyInput, Mods};
use crate::pty::PtySession;
use crate::render::Renderer;
use crate::term::Terminal;
use crate::theme::Theme;
use crate::visual::VisualMap;

pub struct App {
    config: Config,
    proxy: EventLoopProxy<UserEvent>,
    window: Option<Arc<Window>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    renderer: Option<Renderer>,
    terminal: Option<Terminal>,
    pty: Option<PtySession>,
    modifiers: ModifiersState,
    cols: usize,
    rows: usize,
}

impl App {
    pub fn new(config: Config, proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            config,
            proxy,
            window: None,
            surface: None,
            renderer: None,
            terminal: None,
            pty: None,
            modifiers: ModifiersState::empty(),
            cols: 80,
            rows: 24,
        }
    }

    fn create_window(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let attributes = Window::default_attributes()
            .with_title("Termyx")
            .with_inner_size(LogicalSize::new(
                self.config.window.width,
                self.config.window.height,
            ));
        let window = Arc::new(event_loop.create_window(attributes)?);

        let context = Context::new(window.clone())
            .map_err(|err| anyhow::anyhow!("softbuffer context: {err}"))?;
        let mut surface = Surface::new(&context, window.clone())
            .map_err(|err| anyhow::anyhow!("softbuffer surface: {err}"))?;

        let fonts = FontSet::load(
            &self.config.font.family,
            self.config.font.size,
            self.config.font.line_height,
        )?;
        let theme = Theme::from_config(&self.config.theme);
        let mut renderer = Renderer::new(fonts, theme, self.config.window.padding);

        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);
        renderer.resize(width, height);

        let (cols, rows) = grid_size(
            width,
            height,
            renderer.fonts().cell_w,
            renderer.fonts().cell_h,
            renderer.padding(),
        );
        let terminal = Terminal::new(cols, rows, self.config.scrollback.lines);
        let pty = PtySession::spawn(
            &self.config.shell,
            cols as u16,
            rows as u16,
            self.proxy.clone(),
        )?;

        if let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            if let Err(err) = surface.resize(w, h) {
                tracing::warn!(%err, "initial surface resize failed");
            }
        }

        self.window = Some(window);
        self.surface = Some(surface);
        self.renderer = Some(renderer);
        self.terminal = Some(terminal);
        self.pty = Some(pty);
        self.cols = cols;
        self.rows = rows;

        if let Some(window) = &self.window {
            window.request_redraw();
        }
        Ok(())
    }

    fn resize_surface(&mut self, width: u32, height: u32) {
        let (Some(surface), Some(renderer)) = (&mut self.surface, &mut self.renderer) else {
            return;
        };
        let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return;
        };
        if let Err(err) = surface.resize(w, h) {
            tracing::warn!(%err, "surface resize failed");
        }
        renderer.resize(width, height);
    }

    fn resize_grid(&mut self, width: u32, height: u32) {
        let (Some(renderer), Some(terminal), Some(pty)) =
            (&self.renderer, &mut self.terminal, &mut self.pty)
        else {
            return;
        };
        let (cols, rows) = grid_size(
            width,
            height,
            renderer.fonts().cell_w,
            renderer.fonts().cell_h,
            renderer.padding(),
        );
        if cols == self.cols && rows == self.rows {
            return;
        }
        terminal.resize(cols, rows);
        if let Err(err) = pty.resize(cols as u16, rows as u16) {
            tracing::warn!(%err, "pty resize failed");
        }
        self.cols = cols;
        self.rows = rows;
    }

    fn render(&mut self) {
        let (Some(renderer), Some(terminal), Some(surface)) =
            (&mut self.renderer, &self.terminal, &mut self.surface)
        else {
            return;
        };
        let snapshot = terminal.snapshot();
        let maps = vec![VisualMap::identity(snapshot.cols); snapshot.rows];
        renderer.draw(&snapshot, &maps);
        renderer.sync();

        match surface.buffer_mut() {
            Ok(mut buffer) => {
                buffer.copy_from_slice(renderer.pixels());
                if let Err(err) = buffer.present() {
                    tracing::warn!(%err, "present failed");
                }
            }
            Err(err) => tracing::warn!(%err, "surface buffer unavailable"),
        }
    }

    fn on_key(&mut self, event: KeyEvent) {
        let Some(key) = map_key(&event.logical_key) else {
            return;
        };
        let mode = self
            .terminal
            .as_ref()
            .map(Terminal::mode)
            .unwrap_or_else(TermMode::empty);
        let Some(bytes) = input::encode(key, mods_from(self.modifiers), mode) else {
            return;
        };
        if let Some(pty) = &mut self.pty {
            if let Err(err) = pty.write(&bytes) {
                tracing::warn!(%err, "pty write failed");
            }
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err(err) = self.create_window(event_loop) {
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Error)
                .set_title("Termyx")
                .set_description(format!("Startup failed: {err:#}"))
                .show();
            event_loop.exit();
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::PtyOutput(bytes) => {
                if let Some(terminal) = &mut self.terminal {
                    terminal.feed(&bytes);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.resize_surface(size.width, size.height);
                self.resize_grid(size.width, size.height);
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.render(),
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed =>
            {
                self.on_key(event)
            }
            _ => {}
        }
    }
}

fn grid_size(width: u32, height: u32, cell_w: u32, cell_h: u32, padding: u32) -> (usize, usize) {
    let usable_w = width.saturating_sub(padding.saturating_mul(2));
    let usable_h = height.saturating_sub(padding.saturating_mul(2));
    let cols = (usable_w / cell_w.max(1)).max(20) as usize;
    let rows = (usable_h / cell_h.max(1)).max(5) as usize;
    (cols, rows)
}

fn map_key(key: &Key) -> Option<KeyInput> {
    match key {
        Key::Character(text) => text.chars().next().map(KeyInput::Char),
        Key::Named(NamedKey::Space) => Some(KeyInput::Char(' ')),
        Key::Named(NamedKey::Enter) => Some(KeyInput::Enter),
        Key::Named(NamedKey::Backspace) => Some(KeyInput::Backspace),
        Key::Named(NamedKey::Tab) => Some(KeyInput::Tab),
        Key::Named(NamedKey::Escape) => Some(KeyInput::Escape),
        Key::Named(NamedKey::ArrowUp) => Some(KeyInput::Up),
        Key::Named(NamedKey::ArrowDown) => Some(KeyInput::Down),
        Key::Named(NamedKey::ArrowLeft) => Some(KeyInput::Left),
        Key::Named(NamedKey::ArrowRight) => Some(KeyInput::Right),
        Key::Named(NamedKey::Home) => Some(KeyInput::Home),
        Key::Named(NamedKey::End) => Some(KeyInput::End),
        Key::Named(NamedKey::Delete) => Some(KeyInput::Delete),
        Key::Named(NamedKey::Insert) => Some(KeyInput::Insert),
        Key::Named(NamedKey::PageUp) => Some(KeyInput::PageUp),
        Key::Named(NamedKey::PageDown) => Some(KeyInput::PageDown),
        _ => None,
    }
}

fn mods_from(modifiers: ModifiersState) -> Mods {
    Mods {
        ctrl: modifiers.control_key(),
        alt: modifiers.alt_key(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_size_uses_padding_and_minimums() {
        assert_eq!(grid_size(1200, 800, 10, 20, 4), (119, 39));
        assert_eq!(grid_size(100, 100, 10, 20, 4), (20, 5));
    }

    #[test]
    fn map_key_recognizes_text_and_named_keys() {
        assert_eq!(
            map_key(&Key::Character("a".into())),
            Some(KeyInput::Char('a'))
        );
        assert_eq!(map_key(&Key::Named(NamedKey::Enter)), Some(KeyInput::Enter));
        assert_eq!(map_key(&Key::Named(NamedKey::F1)), None);
    }

    #[test]
    fn modifiers_map_to_ctrl_and_alt() {
        let mods = mods_from(ModifiersState::CONTROL | ModifiersState::ALT);
        assert!(mods.ctrl);
        assert!(mods.alt);
    }
}
```

- [ ] **Step 4: Extend `src/render/mod.rs`**

Add these imports at the top of `src/render/mod.rs`:

```rust
use crate::term::{Color, GridSnapshot};
use crate::visual::VisualMap;
```

Replace the existing `pub fn draw(&mut self)` with:

```rust
pub fn draw(&mut self, snapshot: &GridSnapshot, maps: &[VisualMap]) {
    self.draw_background();

    let cell_w = self.fonts.cell_w as f32;
    let cell_h = self.fonts.cell_h as f32;
    let ascent = self.fonts.ascent;
    let padding = self.padding as f32;
    let cursor_color = self.theme.cursor;

    for (row, line) in snapshot.lines.iter().enumerate() {
        let Some(map) = maps.get(row) else {
            continue;
        };
        let y = padding + row as f32 * cell_h;
        for (logical, cell) in line.cells.iter().enumerate() {
            let x = padding + map.visual_of(logical) as f32 * cell_w;
            if let Some(bg) = cell.style.bg {
                let color = self.resolve(bg);
                self.fill_rect(x, y, cell_w, cell_h, color);
            }
            if cell.width > 0 && !cell.text.is_empty() {
                let color = self.resolve(cell.style.fg);
                self.draw_text(x, y + ascent, &cell.text, color);
            }
        }
    }

    if snapshot.display_offset == 0 {
        if let Some(map) = maps.get(snapshot.cursor.row) {
            let visual = map.cursor_visual(&snapshot.cursor);
            let x = padding + visual as f32 * cell_w;
            let y = padding + snapshot.cursor.row as f32 * cell_h;
            self.fill_rect(x, y, 2.0, cell_h, cursor_color);
        }
    }
}

fn draw_background(&mut self) {
    let width = self.pixmap.width() as f32;
    let height = self.pixmap.height() as f32;
    let color = self.theme.background;
    self.fill_rect(0.0, 0.0, width, height, color);
}

fn resolve(&self, color: Color) -> Rgb {
    match color {
        Color::Default => self.theme.foreground,
        Color::Indexed(index) => self.theme.indexed(index),
        Color::Rgb(r, g, b) => Rgb::new(r, g, b),
    }
}

fn fill_rect(&mut self, x: f32, y: f32, width: f32, height: f32, color: Rgb) {
    let pixmap_w = self.pixmap.width() as i32;
    let pixmap_h = self.pixmap.height() as i32;
    let x0 = (x.round() as i32).clamp(0, pixmap_w);
    let y0 = (y.round() as i32).clamp(0, pixmap_h);
    let x1 = ((x + width).round() as i32).clamp(0, pixmap_w);
    let y1 = ((y + height).round() as i32).clamp(0, pixmap_h);
    let data = self.pixmap.data_mut();
    for py in y0..y1 {
        for px in x0..x1 {
            let index = ((py * pixmap_w + px) * 4) as usize;
            data[index] = color.r;
            data[index + 1] = color.g;
            data[index + 2] = color.b;
            data[index + 3] = 255;
        }
    }
}

pub fn fonts(&self) -> &FontSet {
    &self.fonts
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test app::tests -v`

Expected: PASS — 3 tests pass.

Run: `cargo test -v`

Expected: PASS — all unit tests pass.

- [ ] **Step 6: Manual echo check**

Run: `cargo run`

Expected: a window opens with a PowerShell/bash prompt at the top-left. Type `echo hello` and press Enter; the shell prints `hello`. Type `dir` (Windows) or `ls` (Unix); output appears. Resize the window; the prompt stays aligned and text does not corrupt. Close the window; the process exits.

- [ ] **Step 7: Commit**

```bash
git add src/app.rs src/render/mod.rs
git commit -m "feat: wire pty, terminal, input, and renderer"
```

---

### Task 10: Selection and clipboard

**Files:**
- Modify: `src/selection.rs`
- Modify: `src/app.rs`
- Modify: `src/render/mod.rs`

**Interfaces:**
- Consumes: `crate::term::{GridSnapshot, LogicalPos}`.
- Produces: `Selection { new(LogicalPos), update(LogicalPos), ordered() -> (LogicalPos, LogicalPos),
  contains(usize, usize) -> bool, to_text(&GridSnapshot, bool) -> String }`.

- [ ] **Step 1: Write the failing tests**

Replace `src/selection.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{Affinity, Cell, Color, CursorState, RowSnapshot, Style};

    fn position(row: usize, col: usize) -> LogicalPos {
        LogicalPos { row, col }
    }

    fn cell(ch: char) -> Cell {
        Cell {
            text: ch.to_string(),
            width: 1,
            style: Style {
                fg: Color::Default,
                bg: None,
            },
        }
    }

    fn row(text: &str) -> RowSnapshot {
        RowSnapshot {
            cells: text.chars().map(cell).collect(),
            wrapped: false,
        }
    }

    fn snapshot() -> GridSnapshot {
        GridSnapshot {
            cols: 5,
            rows: 2,
            lines: vec![row("hello"), row("world")],
            cursor: CursorState {
                row: 0,
                logical: 0,
                affinity: Affinity::Before,
            },
            display_offset: 0,
        }
    }

    #[test]
    fn selects_within_one_line() {
        let mut selection = Selection::new(position(0, 1));
        selection.update(position(0, 3));
        assert_eq!(selection.to_text(&snapshot(), true), "ell");
    }

    #[test]
    fn reverse_selection_is_normalized() {
        let mut selection = Selection::new(position(0, 3));
        selection.update(position(0, 1));
        assert_eq!(selection.to_text(&snapshot(), true), "ell");
    }

    #[test]
    fn selects_across_lines() {
        let mut selection = Selection::new(position(0, 3));
        selection.update(position(1, 2));
        assert_eq!(selection.to_text(&snapshot(), true), "lo\nwor");
    }

    #[test]
    fn contains_includes_selection_edges() {
        let mut selection = Selection::new(position(0, 1));
        selection.update(position(1, 2));
        assert!(selection.contains(0, 1));
        assert!(selection.contains(0, 4));
        assert!(selection.contains(1, 0));
        assert!(selection.contains(1, 2));
        assert!(!selection.contains(0, 0));
        assert!(!selection.contains(1, 3));
        assert!(!selection.contains(2, 0));
    }

    #[test]
    fn trims_trailing_spaces_when_requested() {
        let mut snapshot = snapshot();
        snapshot.lines[0] = row("hi   ");
        let selection = Selection::new(position(0, 0));
        assert_eq!(selection.to_text(&snapshot, true), "hi");
        assert_eq!(selection.to_text(&snapshot, false), "hi   ");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test selection::tests -v`

Expected: FAIL — `cannot find type 'Selection' in this scope`.

- [ ] **Step 3: Write the implementation**

Replace `src/selection.rs` with:

```rust
use crate::term::{GridSnapshot, LogicalPos};

/// A logical-order text selection. Visual order never enters this type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub anchor: LogicalPos,
    pub focus: LogicalPos,
}

impl Selection {
    pub fn new(anchor: LogicalPos) -> Self {
        Self {
            anchor,
            focus: anchor,
        }
    }

    pub fn update(&mut self, focus: LogicalPos) {
        self.focus = focus;
    }

    /// Start and end in logical order, regardless of drag direction.
    pub fn ordered(&self) -> (LogicalPos, LogicalPos) {
        if self.anchor <= self.focus {
            (self.anchor, self.focus)
        } else {
            (self.focus, self.anchor)
        }
    }

    pub fn contains(&self, row: usize, col: usize) -> bool {
        let (start, end) = self.ordered();
        if row < start.row || row > end.row {
            return false;
        }
        if row == start.row && col < start.col {
            return false;
        }
        if row == end.row && col > end.col {
            return false;
        }
        true
    }

    /// Copy in logical order, skipping wide-character continuation cells.
    pub fn to_text(&self, snapshot: &GridSnapshot, trim_trailing: bool) -> String {
        let (start, end) = self.ordered();
        let mut out = String::new();

        for row in start.row..=end.row {
            let Some(line) = snapshot.lines.get(row) else {
                continue;
            };
            let first = if row == start.row { start.col } else { 0 };
            let last = if row == end.row {
                end.col.min(line.cells.len().saturating_sub(1))
            } else {
                line.cells.len().saturating_sub(1)
            };

            let mut text = String::new();
            for col in first..=last {
                if let Some(cell) = line.cells.get(col) {
                    if cell.width > 0 {
                        text.push_str(&cell.text);
                    }
                }
            }
            if trim_trailing {
                text = text.trim_end().to_string();
            }
            if row > start.row && !snapshot.lines[row - 1].wrapped {
                out.push('\n');
            }
            out.push_str(&text);
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{Affinity, Cell, Color, CursorState, RowSnapshot, Style};

    fn position(row: usize, col: usize) -> LogicalPos {
        LogicalPos { row, col }
    }

    fn cell(ch: char) -> Cell {
        Cell {
            text: ch.to_string(),
            width: 1,
            style: Style {
                fg: Color::Default,
                bg: None,
            },
        }
    }

    fn row(text: &str) -> RowSnapshot {
        RowSnapshot {
            cells: text.chars().map(cell).collect(),
            wrapped: false,
        }
    }

    fn snapshot() -> GridSnapshot {
        GridSnapshot {
            cols: 5,
            rows: 2,
            lines: vec![row("hello"), row("world")],
            cursor: CursorState {
                row: 0,
                logical: 0,
                affinity: Affinity::Before,
            },
            display_offset: 0,
        }
    }

    #[test]
    fn selects_within_one_line() {
        let mut selection = Selection::new(position(0, 1));
        selection.update(position(0, 3));
        assert_eq!(selection.to_text(&snapshot(), true), "ell");
    }

    #[test]
    fn reverse_selection_is_normalized() {
        let mut selection = Selection::new(position(0, 3));
        selection.update(position(0, 1));
        assert_eq!(selection.to_text(&snapshot(), true), "ell");
    }

    #[test]
    fn selects_across_lines() {
        let mut selection = Selection::new(position(0, 3));
        selection.update(position(1, 2));
        assert_eq!(selection.to_text(&snapshot(), true), "lo\nwor");
    }

    #[test]
    fn contains_includes_selection_edges() {
        let mut selection = Selection::new(position(0, 1));
        selection.update(position(1, 2));
        assert!(selection.contains(0, 1));
        assert!(selection.contains(0, 4));
        assert!(selection.contains(1, 0));
        assert!(selection.contains(1, 2));
        assert!(!selection.contains(0, 0));
        assert!(!selection.contains(1, 3));
        assert!(!selection.contains(2, 0));
    }

    #[test]
    fn trims_trailing_spaces_when_requested() {
        let mut snapshot = snapshot();
        snapshot.lines[0] = row("hi   ");
        let selection = Selection::new(position(0, 0));
        assert_eq!(selection.to_text(&snapshot, true), "hi");
        assert_eq!(selection.to_text(&snapshot, false), "hi   ");
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test selection::tests -v`

Expected: PASS — 5 tests pass.

- [ ] **Step 5: Wire mouse selection and clipboard into `src/app.rs`**

Change the import block:

```rust
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
```

Add:

```rust
use crate::selection::Selection;
use crate::term::{LogicalPos, Terminal};
```

Replace the existing `use crate::term::Terminal;` line with the line above.

Add these fields to `App`:

```rust
    selection: Option<Selection>,
    mouse_down: bool,
    cursor_pos: (f64, f64),
```

Initialize them in `App::new`:

```rust
            selection: None,
            mouse_down: false,
            cursor_pos: (0.0, 0.0),
```

In `render`, change the draw call to:

```rust
        renderer.draw(&snapshot, &maps, self.selection.as_ref());
```

At the top of `on_key`, before `map_key`, add:

```rust
        if self.modifiers.control_key() && self.modifiers.shift_key() {
            if let Key::Character(text) = &event.logical_key {
                match text.as_str() {
                    "c" | "C" => {
                        self.copy_selection();
                        return;
                    }
                    "v" | "V" => {
                        self.paste_clipboard();
                        return;
                    }
                    _ => {}
                }
            }
        }
```

Add these methods inside `impl App`, after `on_key`:

```rust
    fn on_cursor_moved(&mut self, x: f64, y: f64) {
        self.cursor_pos = (x, y);
        if !self.mouse_down {
            return;
        }
        let Some(position) = self.pos_at(x, y) else {
            return;
        };
        if let Some(selection) = &mut self.selection {
            selection.update(position);
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn on_mouse(&mut self, state: ElementState) {
        match state {
            ElementState::Pressed => {
                let Some(position) = self.pos_at(self.cursor_pos.0, self.cursor_pos.1) else {
                    return;
                };
                self.selection = Some(Selection::new(position));
                self.mouse_down = true;
            }
            ElementState::Released => self.mouse_down = false,
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn pos_at(&self, x: f64, y: f64) -> Option<LogicalPos> {
        let renderer = self.renderer.as_ref()?;
        let (row, visual) = cell_at(
            x,
            y,
            renderer.padding() as f64,
            renderer.fonts().cell_w as f64,
            renderer.fonts().cell_h as f64,
            self.cols,
            self.rows,
        );
        let map = VisualMap::identity(self.cols);
        Some(LogicalPos {
            row,
            col: map.logical_at(visual),
        })
    }

    fn copy_selection(&mut self) {
        let Some(selection) = &self.selection else {
            return;
        };
        let Some(terminal) = &self.terminal else {
            return;
        };
        let text = selection.to_text(&terminal.snapshot(), self.config.selection.trim_trailing);
        match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text)) {
            Ok(()) => {}
            Err(err) => tracing::warn!(%err, "copy failed"),
        }
    }

    fn paste_clipboard(&mut self) {
        let text = match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
            Ok(text) => text,
            Err(err) => {
                tracing::warn!(%err, "paste failed");
                return;
            }
        };
        let bracketed = self
            .terminal
            .as_ref()
            .map(|terminal| terminal.mode().contains(TermMode::BRACKETED_PASTE))
            .unwrap_or(false);
        let bytes = input::paste(&text, bracketed);
        if let Some(pty) = &mut self.pty {
            if let Err(err) = pty.write(&bytes) {
                tracing::warn!(%err, "pty write failed");
            }
        }
    }
```

Add these arms to the `window_event` match, before `KeyboardInput`:

```rust
            WindowEvent::CursorMoved { position, .. } => {
                self.on_cursor_moved(position.x, position.y)
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => self.on_mouse(state),
```

Add this free function after `map_key`:

```rust
fn cell_at(
    x: f64,
    y: f64,
    padding: f64,
    cell_w: f64,
    cell_h: f64,
    cols: usize,
    rows: usize,
) -> (usize, usize) {
    let visual = (((x - padding) / cell_w).floor() as isize).clamp(0, cols as isize - 1) as usize;
    let row = (((y - padding) / cell_h).floor() as isize).clamp(0, rows as isize - 1) as usize;
    (row, visual)
}
```

Add this test to the `tests` module in `src/app.rs`:

```rust
    #[test]
    fn cell_at_maps_pixels_to_grid_cells() {
        assert_eq!(cell_at(4.0, 4.0, 4.0, 10.0, 20.0, 80, 24), (0, 0));
        assert_eq!(cell_at(13.0, 24.0, 4.0, 10.0, 20.0, 80, 24), (1, 0));
        assert_eq!(
            cell_at(10_000.0, 10_000.0, 4.0, 10.0, 20.0, 80, 24),
            (23, 79)
        );
    }
```

- [ ] **Step 6: Draw the selection in `src/render/mod.rs`**

Add the import:

```rust
use crate::selection::Selection;
```

Change the `draw` signature to:

```rust
pub fn draw(&mut self, snapshot: &GridSnapshot, maps: &[VisualMap], selection: Option<&Selection>) {
```

Inside the cell loop, after the cell background and before the text, add:

```rust
            if selection.is_some_and(|selection| selection.contains(row, logical)) {
                let color = self.theme.selection;
                self.fill_rect(x, y, cell_w, cell_h, color);
            }
```

- [ ] **Step 7: Run the tests**

Run: `cargo test -v`

Expected: PASS — all unit tests pass, including the 5 selection tests and the `cell_at` test.

- [ ] **Step 8: Manual selection and clipboard check**

Run: `cargo run`

Expected: drag across terminal text; the selection is highlighted. Press Ctrl+Shift+C, paste into Notepad; the copied text matches the logical text. Press Ctrl+Shift+V in the terminal; the clipboard text is inserted at the prompt.

- [ ] **Step 9: Commit**

```bash
git add src/selection.rs src/app.rs src/render/mod.rs
git commit -m "feat: add mouse selection and clipboard"
```

---

### Task 11: Scrollback wheel and resize verification

**Files:**
- Modify: `src/app.rs`

**Interfaces:**
- Consumes: `Terminal::scroll_display(i32)`, `Renderer::fonts()`.
- Produces: wheel scrolling; resize already wired in Task 9.

- [ ] **Step 1: Write the failing test**

Add this test to the `tests` module in `src/app.rs`:

```rust
    #[test]
    fn wheel_lines_converts_scroll_to_rows() {
        use winit::dpi::PhysicalPosition;

        assert_eq!(wheel_lines(MouseScrollDelta::LineDelta(0.0, 1.0), 20.0), 3);
        assert_eq!(wheel_lines(MouseScrollDelta::LineDelta(0.0, -1.0), 20.0), -3);
        assert_eq!(
            wheel_lines(
                MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 40.0)),
                20.0
            ),
            2
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test app::tests::wheel_lines_converts_scroll_to_rows -v`

Expected: FAIL — `cannot find function 'wheel_lines'`.

- [ ] **Step 3: Write the implementation**

Add the import:

```rust
use winit::event::MouseScrollDelta;
```

Add this arm to the `window_event` match:

```rust
            WindowEvent::MouseWheel { delta, .. } => self.on_wheel(delta),
```

Add this method inside `impl App`:

```rust
    fn on_wheel(&mut self, delta: MouseScrollDelta) {
        let cell_h = match &self.renderer {
            Some(renderer) => renderer.fonts().cell_h as f64,
            None => return,
        };
        let lines = wheel_lines(delta, cell_h);
        if lines == 0 {
            return;
        }
        if let Some(terminal) = &mut self.terminal {
            terminal.scroll_display(lines);
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
```

Add this free function after `cell_at`:

```rust
fn wheel_lines(delta: MouseScrollDelta, cell_h: f64) -> i32 {
    match delta {
        MouseScrollDelta::LineDelta(_, y) => (y * 3.0).round() as i32,
        MouseScrollDelta::PixelDelta(position) => (position.y / cell_h).round() as i32,
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -v`

Expected: PASS — all unit tests pass, including the wheel test.

- [ ] **Step 5: Manual scrollback and resize check**

Run: `cargo run`

Expected: run `dir` several times (Windows) or `ls` (Unix) to fill the screen. Wheel up; earlier output scrolls into view. Wheel down; the view returns toward the live screen. Resize the window while scrolled back; text reflows without corruption. Type a new command; the terminal still accepts input.

- [ ] **Step 6: Commit**

```bash
git add src/app.rs
git commit -m "feat: add scrollback wheel and resize handling"
```

---

### Task 12: Phase 1 acceptance

**Files:**
- Modify: `README.md`

**Interfaces:**
- Consumes: the complete Phase 1 binary.
- Produces: a documented, verified Phase 1 build.

- [ ] **Step 1: Update `README.md`**

Replace `README.md` with:

```markdown
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
```

- [ ] **Step 2: Verify formatting**

Run: `cargo fmt --check`

Expected: PASS — no output.

- [ ] **Step 3: Verify lints**

Run: `cargo clippy --all-targets -- -D warnings`

Expected: PASS — no warnings. Fix any warnings before continuing.

- [ ] **Step 4: Run all tests**

Run: `cargo test`

Expected: PASS — every unit test passes.

- [ ] **Step 5: Build the release binary**

Run: `cargo build --release`

Expected: PASS — binary at `target/release/termyx` (or `.exe`).

- [ ] **Step 6: Check performance sanity**

Run (PowerShell):

```powershell
(Get-Item .\target\release\termyx.exe).Length / 1MB
```

Expected: less than 15 MB.

Run the release binary, scroll back through 10,000 lines, then run:

```powershell
Get-Process termyx | Select-Object -ExpandProperty WorkingSet64
```

Expected: less than 80 MB. Time launch to the first prompt with a stopwatch;
expected less than 300 ms. Frame-time and input-latency instrumentation,
golden-image tests, and DPI handling are Phase 2/3 work; Phase 1 verifies
rendering with unit tests and the manual matrix below.

- [ ] **Step 7: Manual acceptance matrix**

Run: `cargo run --release`

Check each item:

- The prompt appears at the left margin.
- `echo hello` prints `hello`.
- `dir` (Windows) or `ls` (Unix) prints a directory listing.
- ANSI colors render. Windows PowerShell:
  `"$([char]27)[31mred$([char]27)[0m"`; Unix: `printf '\x1b[31mred\x1b[0m\n'`.
- `vim` or `nano` opens, accepts typing, redraws on scroll, and exits.
- A long command line wraps at the right edge; resizing reflows it.
- Wheel up shows earlier output; wheel down returns toward live output.
- Drag-select text; Ctrl+Shift+C; paste into Notepad or another app.
- Ctrl+Shift+V pastes clipboard text into the prompt.
- Closing the window exits the process and leaves no orphan shell.

- [ ] **Step 8: Commit**

```bash
git add README.md
git commit -m "docs: add phase 1 usage and acceptance checklist"
```
