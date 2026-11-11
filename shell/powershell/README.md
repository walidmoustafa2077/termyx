# Termyx

Command-aware autocorrect and spellcheck for Windows PowerShell 5.1.

Press Space after a word and Termyx fixes it in place:

- cmdlet and alias typos (`Get-ChldItem` -> `Get-ChildItem`)
- parameter typos (`-Reurse` -> `-Recurse`)
- misspelled bare arguments (`helo` -> `hello`)

Quoted strings, comments, paths, URLs, numbers, variables, and hyphenated arguments
are never touched. Every fix is silent and a single `Ctrl+Z` undoes it.

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

## Configuration

- `$env:TERMYX_AUTOCORRECT = '0'` disables the module before it loads.
- `%LOCALAPPDATA%\Termyx\exceptions.txt` holds words that are never corrected,
  one per line. Lines starting with `#` are ignored.
- `%LOCALAPPDATA%\Termyx\user-words.txt` holds words added with
  `Add-TermyxWord`.
- `Set-TermyxOption -MaxArgumentDistance 1` disables two-edit word fixes.
- `Set-TermyxOption -VerboseCorrections $true` writes each fix to
  `%LOCALAPPDATA%\Termyx\corrections.log`, never to the console.

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
6. Type `Write-Host helo` and press Space. It becomes `Write-Host hello `.
7. Type `Write-Host C:\temp\foo` and press Space. No correction is made; the
   path and the space are inserted unchanged.
8. Type `Get-ChildItem -Reurse` and press Space. It becomes `-Recurse`.
9. Run `Get-PSReadLineKeyHandler -Bound | Where-Object { $_.Key -eq 'Spacebar' }`.
   The `Spacebar` entry must show `CustomAction`.
10. Run `Disable-TermyxAutocorrect`, type a typo, press Space. No
    correction is made; the typo and the space are inserted unchanged. Run the
    handler query again; `Spacebar` must show `SelfInsert`.
11. Run `Enable-TermyxAutocorrect`. Corrections work again.

## Dictionary

`Termyx\assets\en_US.wordlist.txt` is a sorted, pre-expanded en-US word list.
It is generated from the Hunspell en dictionary published by the
`wooorm/dictionaries` project with:

```powershell
.\tools\Build-WordList.ps1
```

The build downloads the `.dic` and `.aff` files into `tools\.cache\` (git-ignored),
applies the affix rules, keeps lowercase alphabetic entries, sorts them ordinally,
and rewrites the committed list. Users never need the network.

## Development

```powershell
Invoke-Pester -Path .\tests -Output Detailed
```

See `docs/superpowers/specs/2026-11-11-powershell-autocorrect-design.md` for the design.
