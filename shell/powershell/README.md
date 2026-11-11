# Termyx

Command-aware autocorrect and spellcheck for Windows PowerShell 5.1.

Press Space after a word and Termyx fixes it in place:

- cmdlet and alias typos (`Get-ChldItem` -> `Get-ChildItem`)
- parameter typos (`-Reurse` -> `-Recurse`)
- misspelled bare arguments (`Write-Host helllo` -> `Write-Host hello`,
  `cd Documnets` -> `cd Documents`, `Write-Host teh` -> `Write-Host the`)

Quoted strings, comments, paths, URLs, numbers, variables, and hyphenated arguments
are never touched. A fix is applied only when exactly one dictionary or command
candidate is closest. When several argument words are equally close, the most
frequent English word wins (`teh` -> `the`, `aer` -> `are`, `recieve` ->
`receive`), and a candidate that would delete a letter only wins if nothing else
is close. Commands and parameters stay strict: a tie there is always left alone.
Every fix is silent and a single `Ctrl+Z` undoes it.

## Requirements

- Windows PowerShell 5.1 (`powershell.exe`)
- PSReadLine 2.0.0 (installed with Windows PowerShell 5.1)
- No admin rights, no network access at runtime

PowerShell 7, VS Code, and the ISE are not supported in this version.

## Install

From the repository root:

```powershell
.\tools\Install.ps1
```

The script copies the module to
`Documents\WindowsPowerShell\Modules\Termyx\<version>` and adds a guarded
`Import-Module Termyx` line to your profile. It is safe to run more than
once; an existing profile is kept, and the line is never duplicated.

Open a new console. The module is imported by your profile.

To remove it:

```powershell
.\tools\Install.ps1 -Uninstall
```

## Commands

| Command | Purpose |
|---|---|
| `Get-TermyxStatus` | Enabled state, word and command counts, engine source |
| `Enable-TermyxAutocorrect` | Install the Space handler |
| `Disable-TermyxAutocorrect` | Remove the handler and restore native Space |
| `Add-TermyxWord <word>` | Add a word to your personal dictionary |
| `Update-TermyxCommandCache` | Refresh the command and alias snapshot |
| `Set-TermyxOption -Enabled:$false` | Turn correction off without unloading |

`Disable-TermyxAutocorrect` rebinds Space to PSReadLine's default
`SelfInsert`. PSReadLine 2.0 exposes no way to read the binding that was active
before Termyx was enabled, so a pre-existing custom Space binding is not
restored.

## Configuration

- `$env:TERMYX_AUTOCORRECT = '0'` disables the module before it loads. The
  module still imports, but `Get-TermyxStatus` reports an `InitError`, and
  `Enable-TermyxAutocorrect` throws until a new console starts without the
  variable.
- `%LOCALAPPDATA%\Termyx\exceptions.txt` holds words that are never corrected,
  one per line. Lines starting with `#` are ignored. An unreadable file is treated as
  empty and never writes an error to the console.
- `%LOCALAPPDATA%\Termyx\user-words.txt` holds words added with
  `Add-TermyxWord`.
- `Set-TermyxOption -MaxArgumentDistance 1` disables two-edit word fixes.
- `Set-TermyxOption -SkipWords @('kubectl', 'nginx')` adds words that are never
  corrected for the rest of the session. They are merged with the exceptions file.
- `Set-TermyxOption -VerboseCorrections $true` writes each fix to
  `%LOCALAPPDATA%\Termyx\corrections.log`, never to the console. The log stores
  the full input and corrected command lines, so it can contain sensitive text. It is
  append-only and unbounded; no rotation is performed.

## Termyx integration

When the parent terminal is Termyx it sets `TERMYX=1`, and the module wraps the
prompt in OSC 133 marks (`ESC ] 133 ; A BEL` before the prompt text and
`ESC ] 133 ; B BEL` after it) so Termyx can underline the input line. Outside
Termyx no marks are emitted, so the prompt is exactly what it was before. A
`prompt` function that you defined yourself is remembered and called inside the
wrapper; `Disable-TermyxAutocorrect` restores it.

`%LOCALAPPDATA%\Termyx\user-words.txt` is shared with Termyx: one word per
line. The module reads it at import and re-reads it while running whenever its
timestamp changes, and Termyx appends to it from its right-click menu. A word
added on either side is therefore known to both without restarting the shell or
the terminal.

## Recovery

If the prompt misbehaves, press `Ctrl+Z`, then run one of:

```powershell
Disable-TermyxAutocorrect
Remove-Module Termyx -Force
```

If the console is unusable, start a new console and comment out the
`Import-Module Termyx` line in
`Documents\WindowsPowerShell\Microsoft.PowerShell_profile.ps1`.

If `Get-TermyxStatus` shows an `InitError` (for example the Space handler
could not be installed), run `Import-Module Termyx -Force` to retry. The
error state is cleared only by a fresh import, so `-Force` is the way back
without quitting the console.

The compiled engine cache lives at
`%LOCALAPPDATA%\Termyx\Corrector.dll`, with its source hash in
`Corrector.sha256`. If the DLL is truncated or otherwise corrupt, the module
deletes both files and recompiles them automatically on the next import.
`Import-Module Termyx -Force` does not help for this case: while the
`Termyx.Corrector` type is already loaded, a forced import takes the
loaded-type path and never re-reads the DLL. Start a new console instead.

The Space handler never writes to the console, so a failure can only skip a
correction; plain Space always still works.

## Manual smoke checklist

Run this after installing, in both Windows Terminal and the legacy console host:

1. Type `Get-ChldItem` and press Space. The line becomes `Get-ChildItem `.
2. Confirm the corrected word is followed by exactly one space, and that no
   characters were lost or duplicated.
3. Press `Ctrl+Z` once. The line returns to `Get-ChldItem` exactly.
4. Type `Write-Host` and press Space. Exactly one space is inserted, so the line
   reads `Write-Host `.
5. Type `git commit -m "helo wrld"` and press Space inside the quotes. No
   correction is made; the word and the space are inserted unchanged.
6. Type `Write-Host helllo` and press Space. It becomes `Write-Host hello `.
7. Type `Write-Host C:\temp\foo` and press Space. No correction is made; the
   path and the space are inserted unchanged.
8. Type `Get-ChildItem -Reurse` and press Space. It becomes `-Recurse`.
9. Type `cd Documnets` and press Space. It becomes `cd Documents `.
10. Type `Write-Host teh` and press Space. It becomes `Write-Host the `: several
    words are equally close, so the most frequent one wins.
11. Run `Get-PSReadLineKeyHandler -Bound | Where-Object { $_.Key -eq 'Spacebar' }`.
    The `Spacebar` entry must show `CustomAction`.
12. Run `Disable-TermyxAutocorrect`, type a typo, press Space. No
    correction is made; the typo and the space are inserted unchanged. Run the
    handler query again; `Spacebar` must show `SelfInsert`.
13. Run `Enable-TermyxAutocorrect`. Corrections work again.

## Dictionary

`Termyx\assets\en_US.wordlist.txt` is a sorted, pre-expanded en-US word list
containing 87,928 words. It is generated from the Hunspell en dictionary published by
the `wooorm/dictionaries` project with:

```powershell
.\tools\Build-WordList.ps1
```

The build downloads the `.dic` and `.aff` files from the `main` branch of
`wooorm/dictionaries` into `tools\.cache\` (git-ignored), applies the affix rules,
keeps lowercase alphabetic entries, sorts them ordinally, and rewrites the committed
list. Because `main` is a moving branch, a later rebuild may not reproduce the
committed asset byte-for-byte, and its word count can change. Users never need the
network.

`Termyx\assets\en_US.frequency.txt` holds 46,717 lowercase words in descending
frequency order and decides ties between equally close argument candidates. It is
generated with:

```powershell
.\tools\Build-FrequencyList.ps1
```

The source is the English subtitle frequency list from the `hermitdave/FrequencyWords`
project (`content/2018/en/en_50k.txt`, CC BY-SA 4.0), filtered to lowercase alphabetic
words and deduplicated. Frequency only ranks candidates that already exist in the word
list; it never invents a word.

## Development

```powershell
Invoke-Pester -Path .\tests -Output Detailed
```

See `docs/superpowers/specs/2026-11-11-powershell-autocorrect-design.md` for the design.
