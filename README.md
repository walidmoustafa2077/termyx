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

The script builds the release binary if it is missing, then launches it.
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
- Tabs, splits, profiles, search, images, application mouse reporting, IME
  preedit, and AI are later phases.
