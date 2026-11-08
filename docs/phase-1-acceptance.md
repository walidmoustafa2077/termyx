# Phase 1 acceptance record

Task 12 acceptance run for the Phase 0/1 plan, branch `phase-0-1`. Short,
factual record of what was exercised and what was not. Screenshots and
throwaway scripts were kept outside the repository (see the end of this file).

## Environment

- Windows 10/11 x64, MSVC toolchain; release build of `8a979d3`.
- Default font/theme/scrollback from `src/config.rs` (Cascadia Mono 14,
  10,000 scrollback lines); window default 1200x800.
- Acceptance host had no `vim`, `nvim`, `nano`, bash, or zsh, and no WSL.
- A fullscreen video and external window manager were active on the desktop
  during the run (see "Environment caveats").

## Interactive matrix

Shell config used for the matrix (a local `task12-accept.toml`, not committed):

```toml
[shell]
program = "powershell.exe"
args = ["-NoLogo", "-NoProfile", "-NoExit", "-Command", "[Console]::OutputEncoding=[Text.Encoding]::UTF8; Clear-Host"]
```

Launch: `target\release\termyx.exe --config task12-accept.toml`

| Check | Result | Command / action | Evidence |
|---|---|---|---|
| Prompt at left margin | PASS | launch and observe | leftmost ink column 5 px with 4 px padding |
| `echo` output | PASS | `echo hello` | screenshot shows `hello` and a new prompt |
| Directory listing | PASS | `dir` | full 45-entry listing |
| ANSI colors | PASS | `$e=[char]27; Write-Output "$e[31mred$e[0m"` | `red` in theme red `#c50f1f` |
| UTF-8 output | PASS | `Write-Output "café ünïcode"` | both strings rendered correctly |
| Long line wraps | PASS | `echo ` + 250 `A` characters | wraps at the right edge |
| Resize reflow | PASS | resize 1100→700→1100 px | text re-wraps and restores; narrowing once scrollback exists may push the top line into history (`src/term.rs`) |
| Wheel scrollback | PASS | `1..400 \| ForEach-Object { "line $_" }`, 10 wheel notches up/down | live 361–400 → up 331–371 (30 rows) → down pixel-identical |
| Drag-select + copy | PASS | drag rows 0–3, Ctrl+Shift+C | clipboard contains `SELECTTARGET` |
| Paste into prompt | PASS | clipboard `cmd /c echo PASTED12>C:\Users\mrlok\AppData\Local\Temp\opencode\task12-paste.txt`, Ctrl+Shift+V, Enter | file contains `PASTED12` |
| Close leaves no orphan shell | PASS | `CloseMainWindow`, inspect process tree | tree (2× conhost + powershell) fully gone; 0 stray `termyx` |
| `vim`/`nano` open, type, redraw, exit | NOT RUN | — | no editor installed (`Get-Command vim`, `nvim`, `nano` all NOT FOUND); no WSL |
| bash/zsh prompt at left margin | NOT RUN | — | Windows-only host; no bash/zsh and no WSL |
| Cold start below 300 ms | FAIL | see performance below | measured 405–483 ms (cmd), 697–734 ms (PowerShell) |

Automation: Win32 `PostMessage` for keys/wheel, `SendKeys`, real
`mouse_event` drag, and client-area screen capture. No test harness is
committed; the matrix above is the durable record.

## Performance

| Metric | Target | Measured | Result |
|---|---|---|---|
| Release binary | < 15 MB | 2,861,568 bytes (2.73 MB) | PASS |
| RAM, 10,000 lines | < 80 MB | 58.6 MB working set / 53.4 MB private (69.7 / 64.6 peak in another run) | PASS |
| Cold start, `cmd.exe` | < 300 ms | 405–483 ms (8 runs, median ≈ 452 ms) | FAIL |
| Cold start, default PowerShell | < 300 ms | 697–734 ms (3 runs) | FAIL |

10,000-line config:

```toml
[shell]
program = "cmd.exe"
args = ["/C", "for /L %i in (1,1,10000) do @echo line %i"]
```

The live view showed `line 9961`…`line 10000`; 1,100 wheel notches up (3,300
rows) showed `line 6661`…`line 6701`, confirming the history depth. Memory was
read with `WorkingSet64`/`PrivateMemorySize64` while the window was visible.

Cold start was timed from just before process creation to the first frame
(background covers > 80 % of a 480×90 client probe) and to the first prompt
(bright pixels spanning > 40 px). Process-creation overhead was 5–15 ms.
Window creation alone took 228–282 ms and the first frame 387–453 ms, so the
miss is in startup/first paint, not in the shell.

All performance numbers were taken with the desktop under load; an
idle-desktop re-measure is advisable before citing them as the project
baseline.

## Environment caveats

- An external window manager minimized the terminal window ~3.6–4.6 s after
  launch when it was not being interacted with. Blank captures in the first
  attempts were caused by this, not by the app.
- In a handful of unattended 10,000-line runs the app exited cleanly (exit
  code 0, empty stderr, no WER dump) after 1.3–2.2 s. It did not reproduce
  afterwards (6 consecutive 10,000-line runs survived 12 s; 3 default-shell
  runs survived 10 s), and exit code 0 implies an external `WM_CLOSE`, not a
  crash. Re-check on a clean desktop.

## Artifacts

Screenshots and scripts live in `%TEMP%\opencode` (`task12-*.png`,
`task12_*.ps1`, `task12-*.toml`) and are intentionally **not committed**. The
durable acceptance record is this file plus the gate results in
`README.md` and the task report.
