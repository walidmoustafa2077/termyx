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

Double-click `run.vbs` for a silent launch: it rebuilds the release binary
in a hidden console and then opens the window. Use `run.cmd` from PowerShell
when you want to watch the build output:

```powershell
.\run.cmd
```

Both scripts rebuild the release binary (incremental) and launch it.
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
family = "Cascadia Mono"
size = 16.0
arabic_family = "Arial"

[bidi]
enabled = true
base_direction = "ltr"   # ltr | rtl | auto

[window]
dynamic_title = false    # true lets apps set the title via OSC 0/2
```

- `ltr` (default) keeps the prompt pinned to the left and embeds RTL runs.
- `rtl` right-anchors the whole row.
- `auto` follows the first strong character of each row.
- `enabled = false` disables reordering; Arabic is still shaped, but mixed rows
  are not reordered.

Segoe UI is the default Arabic face, matching the natural connected rendering
used by PowerShell and the Windows UI. Kawkab Mono 0.501 is also bundled in
`assets/fonts/`, embedded in the binary, and available as an explicit opt-in
via `arabic_family = "Kawkab Mono"`. Arabic runs are shaped continuously from
the shaper's natural advances and anchored to the paragraph's start edge, so
adjacent letters connect without a gap next to the surrounding text, RTL lines
stay right-pinned, and the letterforms are never stretched. Kawkab is not
auto-selected because its narrower ink renders disconnected at 16 px with the
current rasterizer. If the
configured family is unavailable, the terminal logs a warning and tries Segoe
UI, Tahoma, Arial, Noto Naskh Arabic, Noto Sans Arabic, DejaVu Sans, FreeSerif,
then Kawkab Mono. A face is accepted when it covers Arabic and shapes `لا`
with contextual forms, either as a single lam-alef ligature or as connected
two-glyph forms. See `assets/fonts/README.md`.

## Tabs

Each tab runs its own shell; the strip above the grid shows one tab per
session.

- `+` opens a tab with the `default_profile` shell, and `⌄` opens the menu.
- `Ctrl+Shift+T` opens a tab and `Ctrl+Shift+W` closes the active one.
- `Ctrl+Tab` and `Ctrl+Shift+Tab` cycle forward and back.
- `Ctrl+Shift+1` … `Ctrl+Shift+5` open the first five profiles directly.
- The `⌄` menu lists every profile, then `Settings` (opens the config file, or
  its folder on a first run), `Command palette` (not available yet; the row
  only logs), and `About`. No key opens `Settings` or the palette.

`Ctrl+Shift+W` closes the shell in the active tab immediately; there is no
confirmation and no undo.

Profiles come from `[[profiles]]` tables, and `default_profile` picks the one a
bare launch, `Ctrl+Shift+T`, and `+` open:

```toml
default_profile = 0

[[profiles]]
name = "PowerShell"
program = "powershell.exe"

[[profiles]]
name = "Command Prompt"
program = "cmd.exe"
args = ["/k"]
```

## Window chrome

The tab strip is the window's title bar on Windows. There the client area starts
at the window's top edge: there is no caption row, no app icon, and no window
title text in the chrome. The restored Windows frame provides the resize border
and edge/corner resizing; Aero Snap and the drop shadow were not verified in
this environment, so they are not claimed. Other platforms keep their native
window decorations, including the native title bar, and the strip is painted
inside the client area below it.

- The three window controls are painted into the right end of the strip: `—`
  minimises, `□` maximises and `❐` restores, and `✕` closes. Each is 46 logical
  px wide and the full strip height.
- Drag the empty part of the strip — after the last tab and its `+`/`⌄` buttons,
  before the controls — to move the window, as a caption would. A 24 logical px
  gutter is reserved there even when the tabs are crowded, so the gesture is
  always reachable.
- Double-click that same empty space to maximise, and again to restore.
- On Windows, the top 6 physical px of the strip is the window's resize border,
  exactly as on a native framed window: a press there starts a resize, not a
  click.
- The window cannot be made smaller than 320 × 200 logical px, the floor at
  which the three controls and the `+`/`⌄` pair still fit without overlapping.

Out of scope for now:

- The Snap Layouts flyout that Windows shows when hovering a native maximise
  button. Painted buttons cannot summon it. Dragging the window to a screen edge
  is expected to snap through the restored Windows frame, but that behaviour was
  not verified here and is not claimed.
- Mirroring the three controls to the left edge under a right-to-left system
  locale.
- Taskbar jump lists, thumbnail toolbars, and DWM Mica/Acrylic backdrops.

## Terminal protocol support

The VT parser and grid come from `alacritty_terminal`, so standard terminal
behavior is protocol-level rather than app-specific: ANSI/VT sequences, 16/256/
truecolor, alternate screen, scroll regions, wide cells, combining marks,
bracketed paste, and DEC modes.

- **Application mouse reporting**: 1000 click, 1002 button-drag, 1003 any
  motion, UTF-8 coordinates (1005), and SGR (1006); 1007 alternate scroll turns
  the wheel into arrow keys in the alternate screen. Hold **Shift** to force
  local text selection; otherwise single clicks and the wheel go to the
  application. Double-click word and triple-click line selection stay local
  even while an application reports the mouse, so they also work inside TUIs
  such as OpenCode.
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

## Experimental: scrollbar

The scrollbar is an experiment, not a shipped feature. The MVP spec defers it
to a later phase, and it has not been accepted or soak-tested.

- A thumb is drawn whenever scrollback exists, including at the live bottom.
  Hovering expands it from 6 px to 12 px inside a reserved 14 px right gutter,
  so it never covers the last text column.
- Drag the thumb to move `display_offset`; click the track to page by
  `rows - 1` lines; the wheel moves 3 lines per notch.
- When scrollback exists, `Shift+PageUp`/`Shift+PageDown` scroll locally.
  Without history those keys still reach the shell or TUI as modified keys.
- Scrolling clears the current selection; selection does not yet stay anchored
  to scrollback content.
- The thumb maps over the rows scrolling can actually reach: leading blank
  scrollback rows are skipped, so dragging to the top stops at the first line
  that carries text.

Open questions: the thumb mapping and page size are provisional, and track
clicks do not repeat while the button is held.

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

## MVP acceptance (M5)

Measured on Windows 11 x64, AMD Ryzen 5 2600 (6 cores / 12 threads,
3.4 GHz), release build, 2026-11-10:

| Check | Target | Measured |
| --- | --- | --- |
| Full-frame redraw at 200x50 | < 8 ms | 7.08 ms fastest, 7.57 ms median over 40 frames |
| Input-to-paint | < 16 ms | 9.6 ms median, 5.9 ms fastest over 24 keystrokes |
| Cold start | < 300 ms | 47 ms warm, 510 ms on the first launch after a build |
| Resident memory, idle | < 80 MB | 41.1 MB working set / 32.2 MB private |
| Resident memory, 10,000 scrollback lines | < 80 MB | 71.7 MB working set / 65.3 MB private |
| Release binary | < 15 MB | 4.05 MB |
| Code page | 65001 | `chcp` reports 65001 and UTF-8 survives the PTY round trip |

The renderer budget is checked by an ignored test; run
`cargo test --release full_frame_redraw -- --ignored --nocapture`. Arabic,
lam-alef, selection, and resize behavior is covered by the test suite and the
Phase 2 acceptance run above. PowerShell was exercised here; bash and zsh use
the same VT path but were not run on this machine.

## Known limitations

- Bidi is per row. A soft-wrapped RTL paragraph is ordered row by row.
- Joining is not preserved across a bidi direction boundary (for example, a
  Latin number embedded inside an Arabic word).
- A caret or selection boundary inside a two-cell lam-alef draws at the cell
  boundary; the ligature is still drawn once.
- Text uses grayscale antialiasing with a mild gamma lift, not ClearType
  subpixel rendering.
- OSC 8 hyperlinks are ignored; links are not shown and cannot be opened.
- OSC 133 shell integration is not parsed; prompts and command marks are not
  exposed to the terminal.
- IME preedit is not shown; only committed text reaches the shell.
- Splits, search, images, and AI are later phases.
