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

## Run

Double-click `run.cmd`, or run it from PowerShell:

```powershell
.\run.cmd
```

The script rebuilds the release binary (incremental) and launches it.
The executable itself is:

```
target\release\termyx.exe
```

After changing source, rebuild before running:

```powershell
& "$env:USERPROFILE\.cargo\bin\cargo.exe" build --release
```

## Configuration

Pass `--config <path>` to use a specific file. The default path is:

- Windows: `%APPDATA%\termyx\config.toml`
- Linux: `$XDG_CONFIG_HOME/termyx/config.toml` (or `~/.config/...`)
- macOS: `~/Library/Application Support/termyx/config.toml`

A missing default file is fine; built-in defaults are used.

## RTL configuration

```toml
[font]
family = "Consolas"
size = 17.0
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
from Arial, Tahoma, Noto Naskh Arabic, Noto Sans Arabic, DejaVu Sans, FreeSerif,
or Segoe UI, in that order. A face is only accepted when it shapes `لا` as a
single lam-alef ligature. See `assets/fonts/README.md`.

## Terminal protocol support

The VT parser and grid come from `alacritty_terminal`, so standard terminal
behavior is protocol-level rather than app-specific: ANSI/VT sequences, 16/256/
truecolor, alternate screen, scroll regions, wide cells, combining marks,
bracketed paste, and DEC modes.

- **Application mouse reporting**: 1000 click, 1002 button-drag, 1003 any
  motion, UTF-8 coordinates (1005), and SGR (1006); 1007 alternate scroll turns
  the wheel into arrow keys in the alternate screen. Hold **Shift** to force
  local text selection; otherwise the wheel and clicks go to the application.
- **Keys**: Shift+Tab/backtab, F1–F12, modified arrows/Home/End/navigation, and
  application cursor mode. Ctrl+Alt characters are prefixed with `ESC`.
- **Cursor**: block, beam, and underline shapes (DECSCUSR), and visibility
  (DECTCEM). Hollow block is reserved and not reachable from DECSCUSR.
- **Focus and title**: focus in/out reports when enabled, and OSC 0/2 title
  updates. Reset comes from a title-stack pop or a resize with no title set,
  not from OSC 104/110; an empty OSC title restores the default title.
- **OSC 52**: copy-only clipboard storage; OSC 52 read requests are refused.
  Local `Ctrl+Shift+V` paste works.
- **IME**: committed text is sent to the shell; there is no preedit overlay.

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

Status (controller check, 2026-11-08, Windows 11 x64): passed for the default
`ltr` mode (prompt left, joined Arabic, correct mixed Arabic/`123`/`abc` order,
lam-alef aligned), pure Arabic, right-to-left drag selection with
`Ctrl+Shift+C` returning the exact logical string, `rtl` base direction
(right-anchored prompt, cursor at the logical end), and resizing to 800x500
without a panic. The Persian/Urdu and harakat examples above were not
separately re-run in this check.

## Known limitations

- Bidi is per row. A soft-wrapped RTL paragraph is ordered row by row.
- Joining is not preserved across a bidi direction boundary (for example, a
  Latin number embedded inside an Arabic word).
- A caret or selection boundary inside a two-cell lam-alef draws at the cell
  boundary; the ligature is still drawn once.
- The Kawkab Mono asset is not bundled in this phase; system fallback is used.
- OSC 8 hyperlinks are parsed and stored per cell but are not clickable yet.
- OSC 133 shell integration is not parsed; prompts and command marks are not
  exposed to the terminal.
- IME preedit is not shown; only committed text reaches the shell.
- Tabs, splits, profiles, search, images, and AI are later phases.
