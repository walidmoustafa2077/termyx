# Termyx — Autocorrect and Spellcheck for PowerShell 5.1

**Date:** 2026-11-11
**Status:** Approved design, ready for implementation planning
**Repo:** `D:\Development\termyx`

## 1. Problem

Typing in the Windows PowerShell 5.1 console is unforgiving. A transposed letter in a
cmdlet or parameter (`Get-ChldItem`, `-Reurse`) produces a "not recognized" error and
forces a full retype. Free-text arguments (commit messages, search terms, file
descriptions) are not spellchecked at all.

The platform offers no usable spellcheck API on this host: WinRT
`Windows.Data.Text.SpellChecker` cannot be activated from PowerShell 5.1 (throws
`RuntimeException`), and PSReadLine 2.0 has no predictive/fuzzy-command feature.

## 2. Goal

Correct the word being typed at the moment **Space** is pressed, with command awareness:
fix cmdlet and parameter typos using the live PowerShell command surface, and spellcheck
bare argument words against a bundled English dictionary. Never touch quoted strings,
paths, URLs, numbers, variables, or comments.

### Non-goals

- No grammar checking, no sentence-level rewriting.
- No cloud services, no telemetry. Fully offline.
- No PowerShell 7 / VS Code / ISE support in v1 (PSReadLine 2.0 API only).
- No changes to the user's command history entries after acceptance.
- Not a replacement for Tab completion; it is a safety net for typos.

## 3. Verified environment constraints

These were confirmed by probe on the target machine and drive the design:

| Fact | Consequence |
|---|---|
| Windows PowerShell **5.1.26100.9818**; `pwsh` not installed | Target the 5.1 parser and CodeDom `Add-Type` |
| **PSReadLine 2.0.0** | No `Prediction*` options; use `Set-PSReadLineKeyHandler` only |
| PSReadLine 2.0 exposes `GetBufferState`, `Replace`, `SetCursorPosition`, `SelfInsert`, `AcceptLine` | Buffer rewriting from a key handler is possible |
| No `Write-Host`/status API usable inside a key handler | Corrections must be **silent**; feedback limited to undo |
| WinRT `SpellChecker` unusable | Ship a bundled dictionary |
| No profile exists (`Documents\WindowsPowerShell\Microsoft.PowerShell_profile.ps1`) | Install script must create it |
| `Add-Type` compiles in ~0.6–0.9 s | Precompile to a DLL and load with `Add-Type -Path` |

## 4. Requirements (agreed)

1. **Scope:** command-aware, whole line — cmdlets, parameters, and free-text arguments.
2. **Trigger:** on `Space`, phone-style, as you type. Not at Enter, not on a hotkey.
3. **Dictionary:** bundled Hunspell-derived en_US word list, plus a user-extendable list.
4. **Packaging:** a proper module in this repo, imported from the profile, with tests.
5. **Safety:** skip quoted strings, here-strings, comments, paths, URLs, numbers,
   variables, and drive-qualified tokens.
6. **Implementation:** compiled C# engine + thin PSReadLine handler (approach "C"), chosen
   because pure PowerShell dictionary lookups cost 50–200 ms per keypress on 5.1.

## 5. Architecture

Three layers, dependencies flowing one way: handler → classifier → engine.

```
Space keypress
     │
     ▼
[Key handler]  private/KeyHandlers.ps1   ~20 lines, no logic
     │  GetBufferState → line, cursor
     ▼
[Decision]     Invoke-TermyxCorrection -Line -Cursor   ← pure, testable
     │  cheap bail-outs → AST classify → engine suggest
     ▼
[Engine]       lib/Corrector.cs (compiled)  word set + command set + parameter sets
     │  { ShouldApply, Start, Length, Fix, NewLine, NewCursor, Kind, Reason }
     ▼
[Key handler]  Replace(...) if ShouldApply; SetCursorPosition; SelfInsert(' ')
```

### 5.0 Decision contract

`Invoke-TermyxCorrection -Line <string> -Cursor <int>` is a pure function: same
inputs, same outputs, no console access, no module state mutated. It returns an object
with exactly these members:

| Member | Type | Meaning |
|---|---|---|
| `ShouldApply` | bool | A fix was found and should be written |
| `Start` | int | Zero-based index of the token start in `-Line` |
| `Length` | int | Length of the token being replaced |
| `Fix` | string | Replacement text, `$null` when `ShouldApply` is false |
| `NewLine` | string | Full resulting line after replacement (unchanged when skipped) |
| `NewCursor` | int | Cursor position after the fix, before the space is inserted |
| `Kind` | string | `Command`, `Parameter`, `Argument`, or `Skip` |
| `Reason` | string | Short machine-readable cause, e.g. `NoToken`, `InString`, `DistanceTie`, `Known` |

The key handler uses `Start`, `Length`, and `Fix`; tests assert on `NewLine`/`NewCursor`.
`NewLine` and `NewCursor` are always populated, even on `Skip`, so tests can compare a
single object.

### 5.1 File layout

```
D:\Development\termyx\
  Termyx\
    Termyx.psd1
    Termyx.psm1
    lib\Corrector.cs
    private\Token-Classifier.ps1
    private\KeyHandlers.ps1
    assets\en_US.wordlist.txt
  tests\Termyx.Tests.ps1
  tools\Build-WordList.ps1
  tools\Install.ps1
  README.md
  docs\superpowers\specs\2026-11-11-powershell-autocorrect-design.md
```

## 6. Correction pipeline

Executed on every Space press, in this order. Steps 1–2 are cheap bail-outs and must
resolve before any parsing happens.

1. **Extract token.** The maximal run of `[A-Za-z][A-Za-z0-9_-]*` immediately left of the
   cursor. If no token touches the cursor, insert a space and stop.
2. **Bail-out checks** (no AST parse):
   - token shorter than 3 characters (`ls`, `cd`, `ii`);
   - token contains `$`, `\`, `/`, `:`, `.`, `@`, or any quote character;
   - token is all uppercase (`JSON`, `HTTP`, `UTF8`) — treated as an acronym;
   - token consists only of digits or `-` prefixed numerics.
3. **Classify.** Parse the whole line with
   `[System.Management.Automation.Language.Parser]::ParseInput($line, [ref]$tokens, [ref]$errors)`,
   cache tokens keyed by line hash, and find the token spanning the cursor. Result is one
   of `Skip`, `Command`, `Parameter`, `Argument`.
   `Skip` covers: single-quoted strings, double-quoted strings, here-strings, comments,
   variable references (`$foo`, `$env:PATH`), drive-qualified and rooted paths, URLs,
   numeric literals, and anything inside a script block that is not a bare word.
4. **Resolve and suggest**, by kind:

   | Kind | Candidate set | Max distance | Apply rule |
   |---|---|---|---|
   | `Command` | cached command/alias/function names from `Get-Command` | 2 | unique best candidate; distance 2 allowed only for the line's first token |
   | `Parameter` | `Parameters.Keys` of the owning command only | 2 | unique best candidate |
   | `Argument` | bundled word set + user word set | 1, or 2 when token length ≤ 6 | unique best candidate, or the most frequent candidate when several tie |

   Command and parameter ties are never guessed. Argument ties are resolved with the
   bundled English frequency list (`assets/en_US.frequency.txt`): candidates that are as
   long as or longer than the typed token win over shorter ones (a deletion is a last
   resort), then the lowest frequency rank wins. If no tied candidate carries a rank, the
   token is left alone as before.
5. **Rewrite.** If a fix exists: `Replace(start, tokenLength + 1, fix + ' ')` in a single
   edit, then `SetCursorPosition` to the end of `fix`. Otherwise a single
   `SelfInsert(' ')`.

### 6.1 Guard rails

- Never rewrite a token that already resolves to a real command, alias, or an existing
  file path.
- Case-preserving output: `Gett-ChildItem` → `Get-ChildItem`; `Cat` stays `Cat`.
- Corrections are silent. PSReadLine 2.0 corrupts the edit line when a handler writes to
  the console, so no status message and no bell.
- Each correction is exactly one `Replace`, hence one `Ctrl+Z` undo entry.
- Token is replaced only if the replacement is non-empty and differs from the original.
- Never re-correct a word that the user just undid within the same input line: the
  handler tracks the last correction `(line, token, fix)` and skips it if it would repeat
  immediately.

### 6.2 Configuration

Public commands exported by the module:

| Command | Purpose |
|---|---|
| `Enable-TermyxAutocorrect` | Install key handler and set module state active |
| `Disable-TermyxAutocorrect` | Remove handler, restore native Space |
| `Get-TermyxStatus` | Enabled state, word count, command count, engine source (dll/source) |
| `Add-TermyxWord <word>` | Append to the user dictionary and hot-load it |
| `Update-TermyxCommandCache` | Refresh the command/alias snapshot |
| `Set-TermyxOption` | `-Enabled`, `-MaxArgumentDistance`, `-SkipWords`, `-VerboseCorrections` |

Environment kill switch: `$env:TERMYX_AUTOCORRECT = '0'` disables loading entirely.
Exception list: `%LOCALAPPDATA%\Termyx\exceptions.txt`, one token per line, never
corrected.

## 7. Engine (`lib/Corrector.cs`)

Single static class, no instance state exposed to PowerShell.

```csharp
namespace Termyx {
    public static class Corrector {
        public static void Initialize(string wordListPath, string userWordListPath);
        public static bool IsKnownWord(string word);
        public static string SuggestWord(string word, int maxDistance);   // null when none
        public static void SetCommands(IEnumerable<string> commands);
        public static string SuggestCommand(string word, int maxDistance);
        public static void SetParameters(string commandName, IEnumerable<string> names);
        public static string SuggestParameter(string command, string word, int maxDistance);
        public static void AddUserWord(string word);
        public static int WordCount { get; }
        public static int CommandCount { get; }
    }
}
```

- Case-insensitive `HashSet<string>` with `StringComparer.OrdinalIgnoreCase` for lookups;
  original casing preserved on output.
- Suggestion by generate-and-test: for each position, try deletion, transposition,
  substitution (26 letters), insertion (27 slots), probing the hash set — ~54k probes,
  1–3 ms. Distance 2 is reached by applying the distance-1 transform to the verified
  distance-1 neighbours, capped to short tokens.
- Results memoized per `(kind, lowercase word)` in a `Dictionary`, so repeated words are
  effectively free.
- Word list loaded once at `Initialize` into a `HashSet`, ~150k words, ~10–20 MB.
- Thread safety: engine is only touched from the PSReadLine handler thread; a `lock` around
  mutations (`AddUserWord`, `SetCommands`) is still used because `Add-TermyxWord`
  may be called interactively.

## 8. Dictionary

- `assets/en_US.wordlist.txt` — newline-delimited, sorted, lowercase, **pre-expanded** so
  inflections (`cats`, `running`, `happier`) appear as their own entries. The committed
  list holds 87,928 words (~905 KB), the full expansion of the pinned Hunspell source;
  stems flagged NOSUGGEST in the `.aff`/`.dic` are excluded.
- `assets/en_US.frequency.txt` — newline-delimited English words in descending frequency
  order, 46,717 entries (~420 KB), used only to break ties between argument candidates.
  It never adds words to the dictionary: a candidate must already be in the word list.
  Source: `hermitdave/FrequencyWords`, `content/2018/en/en_50k.txt` (OpenSubtitles 2018,
  CC BY-SA 4.0), filtered to `^[a-z]+$` and deduplicated by `tools/Build-FrequencyList.ps1`.
- `tools/Build-WordList.ps1` regenerates the word list from the Hunspell en_US `.dic` +
  `.aff` sources (downloaded once at build time with `System.Net.WebClient`; the repo
  commits the generated list, so users never need network access). The script performs
  affix expansion with the `.aff` rules and writes a stable, sorted output, documented in
  the README for provenance.
- User additions go to `%LOCALAPPDATA%\Termyx\user-words.txt`, never into the repo.

## 9. Command and parameter awareness

- The command set is snapshotted at import with
  `Get-Command -CommandType Cmdlet,Function,Alias,ExternalScript,Filter` and refreshed at
  most once every 30 seconds when a correction misses, or explicitly via
  `Update-TermyxCommandCache`. Alias names are included so `gci` and `gcm` are never
  "corrected".
- Parameter candidates are resolved per owning command from `(Get-Command $cmd).Parameters.Keys`
  and cached by command name.
- A token that already resolves via `Get-Command -Name` is never rewritten,
  regardless of its classification. This also protects external commands in
  argument position, such as `Start-Process dotnet`, which the command snapshot
  does not contain.

## 10. PSReadLine integration

```powershell
Set-PSReadLineKeyHandler -Key Space -ScriptBlock {
    try {
        $line, $cursor = $null, $null
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
        $r = Invoke-TermyxCorrection -Line $line -Cursor $cursor
        if ($r.ShouldApply) {
            [Microsoft.PowerShell.PSConsoleReadLine]::Replace($r.Start, $r.Length, $r.Fix)
            [Microsoft.PowerShell.PSConsoleReadLine]::SetCursorPosition($r.NewCursor)
        }
    } catch { }
    [Microsoft.PowerShell.PSConsoleReadLine]::SelfInsert($null)
}
```

`Disable-TermyxAutocorrect` re-registers the native behavior with
`Set-PSReadLineKeyHandler -Key Space -Function SelfInsert`.

## 11. Performance budget

| Operation | Budget |
|---|---|
| Cheap bail-out path (most presses) | < 0.2 ms |
| Tokenization + classification (uncached line) | 1–3 ms |
| Dictionary suggestion, uncached word | 1–3 ms |
| Memoized suggestion | < 0.05 ms |
| Module import, DLL cached | < 100 ms |
| Module import, first compile | < 1.5 s (one time) |

## 12. Failure handling

A defect in a Space handler makes the console unusable, so failure is contained at every
layer:

1. Handler body is wrapped in `try/catch`; any exception degrades to a plain `SelfInsert(' ')`.
2. Module import failure (missing assets, engine load error) leaves the module loaded but
   disabled, with one `Write-Warning` naming the reason.
3. `Disable-TermyxAutocorrect` never depends on the handler path and always works.
4. Recovery instructions in the README: `Remove-Module Termyx -Force`, or comment
   out the profile import line.
5. Engine DLL cache is validated by source hash; a corrupt cache is deleted and recompiled
   automatically.

## 13. Installation

`tools/Install.ps1`:

1. Copies `Termyx\` to
   `%USERPROFILE%\Documents\WindowsPowerShell\Modules\Termyx\<version>\`.
2. Creates the profile if absent and appends exactly one guarded import line:
   ```powershell
   if (Get-Module -ListAvailable Termyx) { Import-Module Termyx }
   ```
3. Idempotent: detects an existing import line and does not duplicate it.
4. `-Uninstall` removes the module directory and the profile line.

## 14. Testing

Pester 3.4 ships with Windows PowerShell 5.1; the tests stick to the `Describe` / `It` /
`Should` subset so they also run on Pester 5.

**Engine tests (pure C#)** — known-word hits and misses, `SuggestWord` distance-1 results,
case preservation, frequency ranking of argument ties, strict tie rejection for commands and
parameters, memoization returns identical results, `AddUserWord` visible immediately.

**Classifier table tests** — a corpus of ~40 lines asserting `Skip` for:
`git commit -m "hello wrld"`, here-strings, `C:\temp\foo`, `./script.ps1`,
`https://example.com/x`, `42`, `$env:PATH`, `# comment`, and `Correct` for `gti status`,
`Get-ChldItem`, `-Reurse`, and bare misspelled words.

**End-to-end tests over the pure function** — `Invoke-TermyxCorrection` returns exact
`ShouldApply` / `NewLine` / `NewCursor` for representative lines; this carries the bulk of
coverage.

**Manual smoke checklist (README)** — Windows Terminal and conhost; quoted commit message
untouched; one `Ctrl+Z` reverts exactly one correction; typing remains responsive on a
150k-word dictionary; `Disable-` restores native Space.

## 15. Delivery milestones

1. **M1 — Engine + dictionary:** module skeleton, `Corrector.cs`, word list build script,
   engine unit tests.
2. **M2 — Classifier:** token classification and skip rules with table tests.
3. **M3 — Decision function + handler:** `Invoke-TermyxCorrection`, key handler,
   enable/disable, end-to-end tests.
4. **M4 — Packaging:** manifest, install script, README, manual smoke pass.

## 16. Risks

| Risk | Mitigation |
|---|---|
| PSReadLine 2.0 behaves differently across Windows builds | Handler wrapped and manually verified on Windows Terminal and conhost |
| Aggressive corrections annoy or corrupt intent | Defaults conservative (ties rejected for commands and parameters, frequency only breaks argument ties, distance-2 limited to first token); exceptions file; single-key undo |
| Word list size inflates module | Commit compressed? No — keep plain text ~1.2 MB; it is source, and compression would add import cost |
| Command cache goes stale after modules load | 30-second refresh window plus explicit `Update-TermyxCommandCache` |
| Undo granularity differs on some builds | Covered by manual checklist; fallback is a single `Replace` of the whole token including the space |
