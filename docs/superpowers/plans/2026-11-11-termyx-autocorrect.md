# Termyx Autocorrect Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `Termyx`, a PowerShell module that corrects cmdlet, parameter, and bare-argument typos in Windows PowerShell 5.1 the moment Space is pressed.

**Architecture:** A thin PSReadLine Space handler calls a pure decision function, which classifies the token under the cursor with the PowerShell parser and asks a compiled C# engine for a unique-best suggestion. The engine holds a bundled en-US word set plus live command and parameter snapshots; every layer fails open.

**Tech Stack:** Windows PowerShell 5.1, PSReadLine 2.0.0, C# 5 compiled by `Add-Type` (CodeDom), Pester 5.7.1, Hunspell en-US source data.

**Spec:** `docs/superpowers/specs/2026-11-11-powershell-autocorrect-design.md`

## Global Constraints

- Target **Windows PowerShell 5.1** (`powershell.exe`) only. `pwsh` may not be used.
- Target **PSReadLine 2.0.0** only. No `Prediction*` options, no PSReadLine 2.1+ APIs.
- **Pester 5.7.1** is the effective test runner on this machine (`C:\Users\mrlok\Documents\WindowsPowerShell\Modules\Pester\5.7.1`). Write tests with `Should -Be` and run them with `Invoke-Pester -Path`. Pester 3.4 is also present but must not be used.
- All C# in `lib/Corrector.cs` must compile with the CodeDom C# 5 compiler: no string interpolation, no `?.`, no `nameof`, no expression-bodied members, no pattern matching, no `out var`.
- All `.ps1` and `.psm1` files must be **ASCII only** (no smart quotes, no em dashes). Write files with `-Encoding ASCII` where the plan says so.
- The Space handler must never write to the console. No `Write-Host`, no `Write-Output`, no `[Console]` calls inside the handler path.
- Corrections must be silent, unique-best only, and exactly one `Replace` per fix so `Ctrl+Z` reverts one fix.
- No runtime network access. Only `tools/Build-WordList.ps1` may download, and only at build time.
- Module import must never make the console unusable: every failure path degrades to native Space and one `Write-Warning`.
- Do not commit `tools/.cache/` or any compiled `Corrector.dll`.
- Repository has no configured git identity. Commit with inline identity:
  `git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "..."`
- Working directory is `D:\Development\termyx`; branch is `master`.

## Verified Facts And Deviations From The Spec

These were probed on the target machine while planning. They are inputs, not open questions.

1. **Pester 5.7.1 wins.** `Get-Module -ListAvailable Pester` returns 5.7.1 before 3.4.0, so `Should Be` (legacy) throws. Use Pester 5 syntax only. The spec's "runs on Pester 3.4 and 5" claim is dropped.
2. **`Add-Type -Path <cs> -OutputAssembly <dll>` writes the DLL but does not load the type** (verified: `typeLoaded=False`). The loader must follow with `Add-Type -Path <dll>`. Loading the same DLL twice is safe (verified).
3. **Tokenizer facts (verified):** command names tokenize as `Generic` or `Identifier`; parameters as `Parameter` (`-Reurse` is one token covering `-Reurse`); paths and URLs tokenize as a single `Generic` token (`C:\temp\foo` covers 0..11); unclosed strings produce `StringExpandable` to end of line; comments, variables, and numbers produce `Comment`, `Variable`, `Number`.
4. **`C:\temp\foo` still parses as a `CommandAst`** whose single element is the whole path. The classifier therefore rejects any element whose extent does not exactly match the extracted token (parameters allow the element to start exactly one character earlier, for the leading `-`).
5. **Network works** with `System.Net.WebClient` against `https://raw.githubusercontent.com/wooorm/dictionaries/main/dictionaries/en/index.dic` (551,762 chars) and `index.aff` (3,084 chars). The affix expander below was prototyped against them and produced 121,275 raw forms in ~27 s.
6. **Planned file additions relative to spec section 5.1:** `private/Decision.ps1` (the pure decision function), `tools/Hunspell.ps1` (pure expander, dot-sourced by the build script and tests), and `.gitignore`. The spec's single test file is kept.
7. **No new engine API beyond the spec.** `SuggestCommand` and `SuggestParameter` return the canonical spelling when the input is already known, so the decision function can detect "known" by comparing the result to the input instead of calling new `IsKnownCommand` methods.

## File Structure

```
D:\Development\termyx\
  .gitignore                              # tools/.cache/
  Termyx\
    Termyx.psd1                     # manifest, version 0.1.0, exports public commands
    Termyx.psm1                     # lifecycle: engine load, word list, snapshots, exports
    lib\Corrector.cs                     # compiled engine (C# 5)
    private\Token-Classifier.ps1         # AST token classification + cache
    private\Decision.ps1                 # pure Invoke-TermyxCorrection + parameter cache
    private\KeyHandlers.ps1              # PSReadLine adapter, repeat guard, log
    assets\en_US.wordlist.txt            # generated, committed, ~120k lines
  tests\Termyx.Tests.ps1           # all Pester tests
  tools\Hunspell.ps1                     # pure Hunspell affix expander
  tools\Build-WordList.ps1               # download + expand + write the committed list
  tools\Install.ps1                      # copy module, add profile import, -Uninstall
  README.md
  docs\superpowers\specs\2026-11-11-powershell-autocorrect-design.md   # already committed
```

Each file has one responsibility. `Termyx.psm1` orchestrates and exports; it contains no correction logic. The decision function in `private/Decision.ps1` is pure; it never touches the console or the key handler.

---

### Task 1: Module Skeleton, Engine Compile, Word Loading

**Files:**
- Create: `.gitignore`
- Create: `Termyx/lib/Corrector.cs`
- Create: `Termyx/Termyx.psm1`
- Create: `Termyx/Termyx.psd1`
- Create: `tests/Termyx.Tests.ps1`

**Interfaces:**
- Consumes: nothing.
- Produces: type `Termyx.Corrector` with `Initialize(string wordListPath, string userWordListPath)`, `IsKnownWord(string)`, `WordCount`; module function `Initialize-TermyxEngine`.

- [ ] **Step 1: Write the failing test**

Create `tests/Termyx.Tests.ps1`:

```powershell
Describe 'Module and engine loading' {
    BeforeAll {
        $script:moduleManifest = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx\Termyx.psd1'
        Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }
    }

    It 'loads the Corrector type' {
        ('Termyx.Corrector' -as [type]) | Should -Not -BeNullOrEmpty
    }

    It 'reports the module version' {
        (Get-Module Termyx).Version | Should -Be ([version]'0.1.0')
    }

    It 'survives a forced re-import without a duplicate type error' {
        {
            Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }
        } | Should -Not -Throw
    }
}
```

- [ ] **Step 2: Run the test and confirm it fails**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: discovery/run failure because `Termyx\Termyx.psd1` does not exist.

- [ ] **Step 3: Create `.gitignore`**

```
tools/.cache/
*.dll
```

- [ ] **Step 4: Create the engine source**

Create `Termyx/lib/Corrector.cs`:

```csharp
using System;
using System.Collections.Generic;
using System.IO;

namespace Termyx
{
    public static class Corrector
    {
        private static readonly object Sync = new object();
        private static HashSet<string> _words = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

        public static void Initialize(string wordListPath, string userWordListPath)
        {
            HashSet<string> words = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            LoadFile(words, wordListPath);
            LoadFile(words, userWordListPath);

            lock (Sync)
            {
                _words = words;
            }
        }

        public static bool IsKnownWord(string word)
        {
            if (string.IsNullOrEmpty(word)) return false;
            lock (Sync) { return _words.Contains(word); }
        }

        public static int WordCount
        {
            get { lock (Sync) { return _words.Count; } }
        }

        private static void LoadFile(HashSet<string> target, string path)
        {
            if (string.IsNullOrEmpty(path) || !File.Exists(path)) return;

            using (StreamReader reader = new StreamReader(path))
            {
                string line;
                while ((line = reader.ReadLine()) != null)
                {
                    line = line.Trim();
                    if (line.Length == 0 || line[0] == '#') continue;
                    target.Add(line);
                }
            }
        }
    }
}
```

- [ ] **Step 5: Create the module script**

Create `Termyx/Termyx.psm1`:

```powershell
param(
    [hashtable]$Options = @{}
)

if ($null -eq $Options -or $Options -isnot [hashtable]) {
    $Options = @{}
}

$script:TermyxRoot = $PSScriptRoot
$script:TermyxEnabled = $true
$script:TermyxInitError = $null
$script:TermyxEngineSource = 'none'
$script:TermyxOptions = @{
    MaxArgumentDistance = 2
    SkipWords           = @()
    VerboseCorrections  = $false
}

$cacheDir = Join-Path $env:LOCALAPPDATA 'Termyx'
$script:TermyxUserWordsPath = Join-Path $cacheDir 'user-words.txt'

function Initialize-TermyxEngine {
    if ('Termyx.Corrector' -as [type]) {
        $script:TermyxEngineSource = 'loaded'
        return
    }

    $source = Join-Path $script:TermyxRoot 'lib\Corrector.cs'
    if (-not (Test-Path -LiteralPath $source)) {
        throw ('Engine source not found: ' + $source)
    }

    if (-not (Test-Path -LiteralPath $cacheDir)) {
        New-Item -ItemType Directory -Force -Path $cacheDir | Out-Null
    }

    $dll = Join-Path $cacheDir 'Corrector.dll'
    $hashFile = Join-Path $cacheDir 'Corrector.sha256'
    $sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash

    $cachedHash = ''
    if (Test-Path -LiteralPath $hashFile) {
        $cachedHash = (Get-Content -LiteralPath $hashFile -Raw).Trim()
    }

    if (-not (Test-Path -LiteralPath $dll) -or $cachedHash -ne $sourceHash) {
        if (Test-Path -LiteralPath $dll) {
            Remove-Item -LiteralPath $dll -Force
        }
        Add-Type -Path $source -OutputAssembly $dll
        Set-Content -LiteralPath $hashFile -Value $sourceHash -Encoding ASCII
        $script:TermyxEngineSource = 'compiled'
    }
    else {
        $script:TermyxEngineSource = 'cached'
    }

    Add-Type -Path $dll
}

Export-ModuleMember -Function @()

if ($env:TERMYX_AUTOCORRECT -eq '0') {
    $script:TermyxEnabled = $false
    $script:TermyxInitError = 'Disabled by TERMYX_AUTOCORRECT=0'
}
else {
    try {
        Initialize-TermyxEngine

        $wordList = Join-Path $script:TermyxRoot 'assets\en_US.wordlist.txt'
        [Termyx.Corrector]::Initialize($wordList, $script:TermyxUserWordsPath)
    }
    catch {
        $script:TermyxEnabled = $false
        $script:TermyxInitError = $_.Exception.Message
        Write-Warning ('Termyx disabled: ' + $_.Exception.Message)
    }
}
```

- [ ] **Step 6: Create the manifest**

Create `Termyx/Termyx.psd1`:

```powershell
@{
    RootModule        = 'Termyx.psm1'
    ModuleVersion     = '0.1.0'
    GUID              = '0f8f5d4c-3f4c-4a71-9b0e-6a2f3c9d1e57'
    Author            = 'Termyx'
    Description       = 'Command-aware autocorrect and spellcheck for Windows PowerShell 5.1.'
    PowerShellVersion = '5.1'
    FunctionsToExport = @()
    CmdletsToExport   = @()
    VariablesToExport = @()
    AliasesToExport   = @()
    PrivateData       = @{
        PSData = @{
            Tags = @('autocorrect', 'spellcheck', 'psreadline')
        }
    }
}
```

- [ ] **Step 7: Run the tests**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: 3 passed, 0 failed.

- [ ] **Step 8: Verify the DLL cache and re-import behavior**

Run:

```powershell
Get-ChildItem (Join-Path $env:LOCALAPPDATA 'Termyx') | Select-Object Name, Length
powershell -NoProfile -ExecutionPolicy Bypass -Command "Import-Module .\Termyx\Termyx.psd1; Import-Module .\Termyx\Termyx.psd1 -Force; 'ok'"
```

Expected: `Corrector.dll` and `Corrector.sha256` exist; the second command prints `ok` with no type-exists error.

- [ ] **Step 9: Commit**

```powershell
git add .gitignore Termyx/lib/Corrector.cs Termyx/Termyx.psm1 Termyx/Termyx.psd1 tests/Termyx.Tests.ps1
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat: add Termyx module skeleton and compiled engine loader"
```

---

### Task 2: Word Suggestions, Ties, Casing, Memoization, User Words

**Files:**
- Modify: `Termyx/lib/Corrector.cs`
- Modify: `tests/Termyx.Tests.ps1`

**Interfaces:**
- Consumes: `Corrector.Initialize`, `Corrector.IsKnownWord`, `Corrector.WordCount` from Task 1.
- Produces: `Corrector.SuggestWord(string word, int maxDistance)` returns `string` (or `$null`); `Corrector.AddUserWord(string word)`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/Termyx.Tests.ps1`:

```powershell
Describe 'Corrector word suggestions' {
    BeforeAll {
        $script:moduleManifest = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx\Termyx.psd1'
        Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }

        $script:wordFixture = Join-Path $env:TEMP ('termyx-words-' + [guid]::NewGuid().ToString('N') + '.txt')
        @(
            'the', 'receive', 'running', 'cat', 'cot', 'hello', 'world',
            'testing', 'powershell', 'cats', 'happier'
        ) | Set-Content -LiteralPath $script:wordFixture -Encoding ASCII
    }

    AfterAll {
        Remove-Item -LiteralPath $script:wordFixture -ErrorAction SilentlyContinue
    }

    BeforeEach {
        [Termyx.Corrector]::Initialize($script:wordFixture, $null)
    }

    It 'finds a one-edit replacement' {
        [Termyx.Corrector]::SuggestWord('teh', 1) | Should -Be 'the'
    }

    It 'treats a transposition as one edit' {
        [Termyx.Corrector]::SuggestWord('recieve', 1) | Should -Be 'receive'
    }

    It 'refuses to guess when two candidates tie' {
        [Termyx.Corrector]::SuggestWord('cit', 1) | Should -BeNullOrEmpty
    }

    It 'preserves the typed casing' {
        [Termyx.Corrector]::SuggestWord('Teh', 1) | Should -Be 'The'
    }

    It 'recognizes known and unknown words' {
        [Termyx.Corrector]::IsKnownWord('hello') | Should -BeTrue
        [Termyx.Corrector]::IsKnownWord('zzqx') | Should -BeFalse
    }

    It 'returns null when nothing is within the distance cap' {
        [Termyx.Corrector]::SuggestWord('zzzz', 2) | Should -BeNullOrEmpty
    }

    It 'memoizes repeated lookups' {
        $first = [Termyx.Corrector]::SuggestWord('teh', 1)
        $second = [Termyx.Corrector]::SuggestWord('teh', 1)
        $second | Should -Be $first
    }

    It 'finds a two-edit replacement for a short word' {
        [Termyx.Corrector]::SuggestWord('hlo', 2) | Should -Be 'hello'
    }

    It 'refuses two-edit matching for long words' {
        [Termyx.Corrector]::SuggestWord('powrshll', 2) | Should -BeNullOrEmpty
    }

    It 'hot-loads a user word' {
        [Termyx.Corrector]::AddUserWord('termyxzz')
        [Termyx.Corrector]::IsKnownWord('termyxzz') | Should -BeTrue
        [Termyx.Corrector]::SuggestWord('termyxz', 1) | Should -Be 'termyxzz'
    }
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: the new Describe fails with `Method invocation failed` / `SuggestWord` not found.

- [ ] **Step 3: Add the fields to `Corrector.cs`**

Replace the field block:

```csharp
        private static readonly object Sync = new object();
        private static HashSet<string> _words = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
```

with:

```csharp
        private static readonly object Sync = new object();
        private const int MaxDistanceTwoLength = 6;

        private static HashSet<string> _words = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        private static Dictionary<string, string> _wordCache = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
```

- [ ] **Step 4: Clear the cache in `Initialize` and add `SuggestWord` and `AddUserWord`**

In `Initialize`, replace:

```csharp
            lock (Sync)
            {
                _words = words;
            }
```

with:

```csharp
            lock (Sync)
            {
                _words = words;
                _wordCache.Clear();
            }
```

Immediately after the `IsKnownWord` method, add:

```csharp
        public static string SuggestWord(string word, int maxDistance)
        {
            if (string.IsNullOrEmpty(word) || word.Length < 2) return null;

            lock (Sync)
            {
                if (_words.Contains(word)) return word;
            }

            string key = maxDistance.ToString() + "|" + word.ToLowerInvariant();
            lock (Sync)
            {
                string cached;
                if (_wordCache.TryGetValue(key, out cached))
                {
                    if (cached.Length == 0) return null;
                    return MatchCase(word, cached);
                }
            }

            HashSet<string> words;
            lock (Sync) { words = _words; }

            string found = SuggestFromSet(words, word.ToLowerInvariant(), maxDistance, MaxDistanceTwoLength);
            lock (Sync) { _wordCache[key] = found == null ? string.Empty : found; }

            if (found == null) return null;
            return MatchCase(word, found);
        }

        public static void AddUserWord(string word)
        {
            if (string.IsNullOrWhiteSpace(word)) return;

            string normalized = word.Trim().ToLowerInvariant();
            lock (Sync)
            {
                _words.Add(normalized);
                _wordCache.Clear();
            }
        }
```

- [ ] **Step 5: Add the private helpers**

Immediately after the `LoadFile` method, add:

```csharp
        private static string SuggestFromSet(HashSet<string> set, string lowerWord, int maxDistance, int distanceTwoMaxLength)
        {
            if (set.Count == 0) return null;

            HashSet<string> found = CollectEdits1(set, lowerWord);
            if (found.Count == 0 && maxDistance >= 2 && lowerWord.Length <= distanceTwoMaxLength)
            {
                foreach (string once in Edits1(lowerWord))
                {
                    foreach (string twice in Edits1(once))
                    {
                        if (set.Contains(twice)) found.Add(twice);
                    }
                }
            }

            if (found.Count != 1) return null;
            foreach (string only in found) return only;
            return null;
        }

        private static HashSet<string> CollectEdits1(HashSet<string> set, string lowerWord)
        {
            HashSet<string> found = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (string candidate in Edits1(lowerWord))
            {
                if (set.Contains(candidate)) found.Add(candidate);
            }
            return found;
        }

        private static IEnumerable<string> Edits1(string word)
        {
            HashSet<string> result = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            int length = word.Length;

            for (int i = 0; i < length; i++)
            {
                string left = word.Substring(0, i);
                string right = word.Substring(i);

                result.Add(left + right.Substring(1));

                if (i < length - 1)
                {
                    result.Add(left + word[i + 1] + word[i] + word.Substring(i + 2));
                }

                for (char c = 'a'; c <= 'z'; c++)
                {
                    if (c != word[i])
                    {
                        result.Add(left + c + right.Substring(1));
                    }
                    result.Add(left + c + right);
                }
            }

            for (char c = 'a'; c <= 'z'; c++)
            {
                result.Add(word + c);
            }

            return result;
        }

        private static string MatchCase(string input, string candidate)
        {
            bool hasUpper = false;
            bool hasLower = false;
            for (int i = 0; i < input.Length; i++)
            {
                if (char.IsUpper(input[i])) hasUpper = true;
                else if (char.IsLower(input[i])) hasLower = true;
            }

            if (hasUpper && !hasLower)
            {
                return candidate.ToUpperInvariant();
            }
            if (char.IsUpper(input[0]))
            {
                return char.ToUpperInvariant(candidate[0]) + candidate.Substring(1);
            }
            return candidate.ToLowerInvariant();
        }
```

- [ ] **Step 6: Run the tests**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: 13 passed, 0 failed.

- [ ] **Step 7: Commit**

```powershell
git add Termyx/lib/Corrector.cs tests/Termyx.Tests.ps1
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat: add word suggestions with edit distance, ties, casing and memoization"
```

---

### Task 3: Command And Parameter Suggestions

**Files:**
- Modify: `Termyx/lib/Corrector.cs`
- Modify: `tests/Termyx.Tests.ps1`

**Interfaces:**
- Consumes: `SuggestWord` machinery (`CollectEdits1`, `Edits1`) from Task 2.
- Produces: `Corrector.SetCommands(IEnumerable<string>)`, `SuggestCommand(string, int)`, `SetParameters(string, IEnumerable<string>)`, `SuggestParameter(string, string, int)`, `CommandCount`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/Termyx.Tests.ps1`:

```powershell
Describe 'Corrector command and parameter suggestions' {
    BeforeAll {
        $script:moduleManifest = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx\Termyx.psd1'
        Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }
    }

    BeforeEach {
        [Termyx.Corrector]::Initialize($null, $null)
        [Termyx.Corrector]::SetCommands(@('Get-ChildItem', 'Get-Content', 'Get-Item', 'Write-Host'))
        [Termyx.Corrector]::SetParameters('Get-ChildItem', @('Path', 'Filter', 'Recurse', 'Force'))
    }

    It 'corrects a command typo' {
        [Termyx.Corrector]::SuggestCommand('Get-ChldItem', 2) | Should -Be 'Get-ChildItem'
    }

    It 'returns the canonical casing of a known command' {
        [Termyx.Corrector]::SuggestCommand('get-childitem', 1) | Should -Be 'Get-ChildItem'
    }

    It 'rejects command ties' {
        [Termyx.Corrector]::SetCommands(@('Get-Foo', 'Get-Fop'))
        [Termyx.Corrector]::SuggestCommand('Get-Foq', 2) | Should -BeNullOrEmpty
    }

    It 'counts the command snapshot' {
        [Termyx.Corrector]::CommandCount | Should -Be 4
    }

    It 'corrects a parameter of the owning command' {
        [Termyx.Corrector]::SuggestParameter('Get-ChildItem', 'Reurse', 2) | Should -Be 'Recurse'
    }

    It 'returns the canonical casing of a known parameter' {
        [Termyx.Corrector]::SuggestParameter('Get-ChildItem', 'recurse', 2) | Should -Be 'Recurse'
    }

    It 'does not suggest parameters for an unknown command' {
        [Termyx.Corrector]::SuggestParameter('Get-Nothing', 'Reurse', 2) | Should -BeNullOrEmpty
    }
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: the new Describe fails because `SetCommands` does not exist.

- [ ] **Step 3: Add the command and parameter fields**

Replace:

```csharp
        private static Dictionary<string, string> _wordCache = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
```

with:

```csharp
        private static Dictionary<string, string> _wordCache = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        private static Dictionary<string, string> _commandCanonical = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        private static Dictionary<string, string> _commandCache = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        private static Dictionary<string, Dictionary<string, string>> _parameters = new Dictionary<string, Dictionary<string, string>>(StringComparer.OrdinalIgnoreCase);
        private static Dictionary<string, string> _parameterCache = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
```

- [ ] **Step 4: Add the public command and parameter methods**

Immediately after `AddUserWord`, add:

```csharp
        public static void SetCommands(IEnumerable<string> commands)
        {
            Dictionary<string, string> map = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            if (commands != null)
            {
                foreach (string name in commands)
                {
                    if (string.IsNullOrWhiteSpace(name)) continue;
                    string trimmed = name.Trim();
                    if (!map.ContainsKey(trimmed)) map[trimmed] = trimmed;
                }
            }

            lock (Sync)
            {
                _commandCanonical = map;
                _commandCache.Clear();
            }
        }

        public static string SuggestCommand(string word, int maxDistance)
        {
            if (string.IsNullOrEmpty(word)) return null;

            string key = maxDistance.ToString() + "|" + word.ToLowerInvariant();
            Dictionary<string, string> map;

            lock (Sync)
            {
                string cached;
                if (_commandCache.TryGetValue(key, out cached))
                {
                    return cached.Length == 0 ? null : cached;
                }
                map = _commandCanonical;
            }

            string canonical;
            if (map.TryGetValue(word, out canonical))
            {
                lock (Sync) { _commandCache[key] = canonical; }
                return canonical;
            }

            string found = SuggestCanonical(map, word.ToLowerInvariant(), maxDistance);
            lock (Sync) { _commandCache[key] = found == null ? string.Empty : found; }
            return found;
        }

        public static void SetParameters(string commandName, IEnumerable<string> names)
        {
            if (string.IsNullOrEmpty(commandName)) return;

            Dictionary<string, string> map = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            if (names != null)
            {
                foreach (string name in names)
                {
                    if (string.IsNullOrWhiteSpace(name)) continue;
                    string trimmed = name.Trim();
                    if (!map.ContainsKey(trimmed)) map[trimmed] = trimmed;
                }
            }

            lock (Sync)
            {
                _parameters[commandName] = map;
                _parameterCache.Clear();
            }
        }

        public static string SuggestParameter(string command, string word, int maxDistance)
        {
            if (string.IsNullOrEmpty(command) || string.IsNullOrEmpty(word)) return null;

            string key = command.ToLowerInvariant() + "|" + maxDistance.ToString() + "|" + word.ToLowerInvariant();
            Dictionary<string, string> map;

            lock (Sync)
            {
                string cached;
                if (_parameterCache.TryGetValue(key, out cached))
                {
                    return cached.Length == 0 ? null : cached;
                }
                if (!_parameters.TryGetValue(command, out map)) return null;
            }

            string found = SuggestCanonical(map, word.ToLowerInvariant(), maxDistance);
            lock (Sync) { _parameterCache[key] = found == null ? string.Empty : found; }
            return found;
        }
```

- [ ] **Step 5: Add `CommandCount`**

Immediately after the `WordCount` property, add:

```csharp
        public static int CommandCount
        {
            get { lock (Sync) { return _commandCanonical.Count; } }
        }
```

- [ ] **Step 6: Add the canonical and distance helpers**

Immediately after `SuggestFromSet`, add:

```csharp
        private static string SuggestCanonical(Dictionary<string, string> map, string lowerWord, int maxDistance)
        {
            if (map.Count == 0) return null;

            HashSet<string> names = new HashSet<string>(map.Keys, StringComparer.OrdinalIgnoreCase);
            HashSet<string> found = CollectEdits1(names, lowerWord);

            if (found.Count == 1)
            {
                foreach (string only in found) return map[only];
            }
            if (found.Count > 1) return null;
            if (maxDistance < 2) return null;

            string best = null;
            int bestDistance = int.MaxValue;
            int bestCount = 0;

            foreach (string name in map.Keys)
            {
                int distance = Distance(lowerWord, name.ToLowerInvariant(), maxDistance);
                if (distance < 0) continue;

                if (distance < bestDistance)
                {
                    bestDistance = distance;
                    best = name;
                    bestCount = 1;
                }
                else if (distance == bestDistance)
                {
                    bestCount++;
                }
            }

            if (bestCount != 1) return null;
            return map[best];
        }

        private static int Distance(string a, string b, int max)
        {
            if (Math.Abs(a.Length - b.Length) > max) return -1;

            int n = a.Length;
            int m = b.Length;
            int[,] d = new int[n + 1, m + 1];

            for (int i = 0; i <= n; i++) d[i, 0] = i;
            for (int j = 0; j <= m; j++) d[0, j] = j;

            for (int i = 1; i <= n; i++)
            {
                for (int j = 1; j <= m; j++)
                {
                    int cost = (a[i - 1] == b[j - 1]) ? 0 : 1;
                    int value = Math.Min(Math.Min(d[i - 1, j] + 1, d[i, j - 1] + 1), d[i - 1, j - 1] + cost);

                    if (i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1])
                    {
                        value = Math.Min(value, d[i - 2, j - 2] + 1);
                    }

                    d[i, j] = value;
                }
            }

            return d[n, m] > max ? -1 : d[n, m];
        }
```

- [ ] **Step 7: Run the tests**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: 20 passed, 0 failed.

- [ ] **Step 8: Commit**

```powershell
git add Termyx/lib/Corrector.cs tests/Termyx.Tests.ps1
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat: add command and parameter aware suggestions"
```

---

### Task 4: Hunspell Expander And Committed Word List

**Files:**
- Create: `tools/Hunspell.ps1`
- Create: `tools/Build-WordList.ps1`
- Create: `Termyx/assets/en_US.wordlist.txt` (generated)
- Modify: `tests/Termyx.Tests.ps1`

**Interfaces:**
- Consumes: nothing.
- Produces: `Expand-HunspellDictionary -DicLines <string[]> -AffLines <string[]>` returns a sorted, de-duplicated `string[]` of lowercase alphabetic words; the committed asset consumed by `Corrector.Initialize`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/Termyx.Tests.ps1`:

```powershell
Describe 'Hunspell expansion' {
    BeforeAll {
        . (Join-Path (Split-Path -Parent $PSScriptRoot) 'tools\Hunspell.ps1')
    }

    It 'expands a simple suffix rule' {
        $aff = @(
            'SET UTF-8'
            'SFX S Y 1'
            'SFX S 0 s .'
        )
        $dic = @('1', 'cat/S')
        $words = Expand-HunspellDictionary -DicLines $dic -AffLines $aff
        ($words -contains 'cat') | Should -BeTrue
        ($words -contains 'cats') | Should -BeTrue
    }

    It 'applies a strip-and-add suffix rule' {
        $aff = @(
            'SFX S Y 1'
            'SFX S y ies [^aeiou]y'
        )
        $dic = @('1', 'try/S')
        $words = Expand-HunspellDictionary -DicLines $dic -AffLines $aff
        ($words -contains 'tries') | Should -BeTrue
        ($words -contains 'trys') | Should -BeFalse
    }

    It 'applies a prefix rule' {
        $aff = @(
            'PFX A Y 1'
            'PFX A 0 re .'
        )
        $dic = @('1', 'do/A')
        $words = Expand-HunspellDictionary -DicLines $dic -AffLines $aff
        ($words -contains 'redo') | Should -BeTrue
    }

    It 'combines prefix and suffix when both allow crossing' {
        $aff = @(
            'PFX A Y 1'
            'PFX A 0 re .'
            'SFX S Y 1'
            'SFX S 0 s .'
        )
        $dic = @('1', 'do/AS')
        $words = Expand-HunspellDictionary -DicLines $dic -AffLines $aff
        ($words -contains 'redos') | Should -BeTrue
    }

    It 'drops numeric and non-letter stems' {
        $aff = @('SET UTF-8')
        $dic = @('2', '1st/p', '0/nm')
        $words = Expand-HunspellDictionary -DicLines $dic -AffLines $aff
        ($words -contains '1st') | Should -BeFalse
        ($words -contains '0') | Should -BeFalse
        $words.Count | Should -Be 0
    }

    It 'returns sorted unique words' {
        $aff = @('SET UTF-8')
        $dic = @('3', 'b', 'a', 'b')
        $words = Expand-HunspellDictionary -DicLines $dic -AffLines $aff
        $words.Count | Should -Be 2
        $words[0] | Should -Be 'a'
        $words[1] | Should -Be 'b'
    }
}

Describe 'en_US word list artifact' {
    BeforeAll {
        $script:wordListPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx\assets\en_US.wordlist.txt'
        $script:wordLines = Get-Content -LiteralPath $script:wordListPath
    }

    It 'contains at least 90000 words' {
        $script:wordLines.Count | Should -BeGreaterThan 90000
    }

    It 'contains common words' {
        ($script:wordLines -contains 'the') | Should -BeTrue
        ($script:wordLines -contains 'receive') | Should -BeTrue
    }

    It 'does not contain common misspellings' {
        ($script:wordLines -contains 'teh') | Should -BeFalse
        ($script:wordLines -contains 'recieve') | Should -BeFalse
    }
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: `tools\Hunspell.ps1` not found, and the word list artifact does not exist.

- [ ] **Step 3: Create the pure expander**

Create `tools/Hunspell.ps1`:

```powershell
function Add-TermyxWordForm {
    param($Words, [string]$Word)
    if ($null -ne $Word -and $Word.Length -ge 2 -and $Word -cmatch '^[a-z]+$') {
        [void]$Words.Add($Word)
    }
}

function Test-TermyxAffixCondition {
    param([string]$Stem, [string]$Condition, [string]$Anchor)

    if ($Condition -eq '.' -or $Condition -eq '0' -or $Condition -eq '') {
        return $true
    }

    try {
        if ($Anchor -eq 'end') {
            return [regex]::IsMatch($Stem, '(?:' + $Condition + ')$')
        }
        return [regex]::IsMatch($Stem, '^(?:' + $Condition + ')')
    }
    catch {
        return $false
    }
}

function Expand-HunspellDictionary {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string[]]$DicLines,
        [Parameter(Mandatory)][string[]]$AffLines
    )

    $prefixes = @{}
    $suffixes = @{}

    foreach ($line in $AffLines) {
        if ($line -match '^(PFX|SFX)\s+(\S+)\s+([YN])\s+(\d+)\s*$') {
            $entry = @{
                Cross = ($Matches[3] -eq 'Y')
                Rules = New-Object System.Collections.ArrayList
            }
            if ($Matches[1] -eq 'PFX') { $prefixes[$Matches[2]] = $entry }
            else { $suffixes[$Matches[2]] = $entry }
            continue
        }

        if ($line -match '^(PFX|SFX)\s+(\S+)\s+(\S+)\s+(\S+)\s*(.*)$') {
            $kind = $Matches[1]
            $flag = $Matches[2]
            $rule = @{
                Strip = $Matches[3]
                Add   = ($Matches[4] -split '/')[0]
                Cond  = $Matches[5].Trim()
            }
            if ($rule.Cond -eq '') { $rule.Cond = '.' }

            if ($kind -eq 'PFX') {
                if ($prefixes.ContainsKey($flag)) { [void]$prefixes[$flag].Rules.Add($rule) }
            }
            elseif ($suffixes.ContainsKey($flag)) {
                [void]$suffixes[$flag].Rules.Add($rule)
            }
        }
    }

    $words = New-Object 'System.Collections.Generic.HashSet[string]' -ArgumentList @([StringComparer]::OrdinalIgnoreCase)

    $start = 0
    if ($DicLines.Count -gt 0 -and $DicLines[0] -match '^\d+$') { $start = 1 }

    for ($index = $start; $index -lt $DicLines.Count; $index++) {
        $raw = $DicLines[$index]
        if ([string]::IsNullOrWhiteSpace($raw)) { continue }

        $raw = ($raw -split "`t")[0]
        $raw = ($raw -split ' ')[0]

        $flags = ''
        $slash = $raw.IndexOf('/')
        if ($slash -ge 0) {
            $flags = $raw.Substring($slash + 1)
            $stem = $raw.Substring(0, $slash)
        }
        else {
            $stem = $raw
        }

        if ($stem.Length -eq 0) { continue }
        Add-TermyxWordForm -Words $words -Word $stem.ToLowerInvariant()

        if ($flags.Length -eq 0) { continue }
        if ($flags.Contains('!')) { continue }

        foreach ($flag in @($prefixes.Keys)) {
            if (-not $flags.Contains($flag)) { continue }
            foreach ($rule in $prefixes[$flag].Rules) {
                if (-not (Test-TermyxAffixCondition -Stem $stem -Condition $rule.Cond -Anchor 'start')) { continue }
                if ($rule.Strip -ne '0' -and -not $stem.StartsWith($rule.Strip)) { continue }

                $base = $stem
                if ($rule.Strip -ne '0') { $base = $stem.Substring($rule.Strip.Length) }
                Add-TermyxWordForm -Words $words -Word (($rule.Add + $base).ToLowerInvariant())
            }
        }

        foreach ($flag in @($suffixes.Keys)) {
            if (-not $flags.Contains($flag)) { continue }
            foreach ($rule in $suffixes[$flag].Rules) {
                if (-not (Test-TermyxAffixCondition -Stem $stem -Condition $rule.Cond -Anchor 'end')) { continue }
                if ($rule.Strip -ne '0' -and -not $stem.EndsWith($rule.Strip)) { continue }

                $base = $stem
                if ($rule.Strip -ne '0') { $base = $stem.Substring(0, $stem.Length - $rule.Strip.Length) }
                Add-TermyxWordForm -Words $words -Word (($base + $rule.Add).ToLowerInvariant())
            }
        }

        foreach ($prefixFlag in @($prefixes.Keys)) {
            if (-not $flags.Contains($prefixFlag)) { continue }
            if (-not $prefixes[$prefixFlag].Cross) { continue }

            foreach ($suffixFlag in @($suffixes.Keys)) {
                if (-not $flags.Contains($suffixFlag)) { continue }
                if (-not $suffixes[$suffixFlag].Cross) { continue }

                foreach ($prefixRule in $prefixes[$prefixFlag].Rules) {
                    if (-not (Test-TermyxAffixCondition -Stem $stem -Condition $prefixRule.Cond -Anchor 'start')) { continue }
                    if ($prefixRule.Strip -ne '0' -and -not $stem.StartsWith($prefixRule.Strip)) { continue }

                    foreach ($suffixRule in $suffixes[$suffixFlag].Rules) {
                        if (-not (Test-TermyxAffixCondition -Stem $stem -Condition $suffixRule.Cond -Anchor 'end')) { continue }
                        if ($suffixRule.Strip -ne '0' -and -not $stem.EndsWith($suffixRule.Strip)) { continue }

                        $base = $stem
                        if ($suffixRule.Strip -ne '0') {
                            $base = $stem.Substring(0, $stem.Length - $suffixRule.Strip.Length)
                        }
                        Add-TermyxWordForm -Words $words -Word (($prefixRule.Add + $base + $suffixRule.Add).ToLowerInvariant())
                    }
                }
            }
        }
    }

    $sorted = @($words)
    [Array]::Sort($sorted, [StringComparer]::Ordinal)
    return ,$sorted
}
```

- [ ] **Step 4: Run the expander tests**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: the 6 expander tests pass; only the 3 word-list artifact tests fail.

- [ ] **Step 5: Create the build script**

Create `tools/Build-WordList.ps1`:

```powershell
[CmdletBinding()]
param(
    [string]$DicPath,
    [string]$AffPath,
    [string]$OutFile
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot

if (-not $OutFile) {
    $OutFile = Join-Path $repoRoot 'Termyx\assets\en_US.wordlist.txt'
}

. (Join-Path $PSScriptRoot 'Hunspell.ps1')

$cacheDir = Join-Path $PSScriptRoot '.cache'
$dicUrl = 'https://raw.githubusercontent.com/wooorm/dictionaries/main/dictionaries/en/index.dic'
$affUrl = 'https://raw.githubusercontent.com/wooorm/dictionaries/main/dictionaries/en/index.aff'

if (-not $DicPath) {
    New-Item -ItemType Directory -Force -Path $cacheDir | Out-Null
    $DicPath = Join-Path $cacheDir 'en.dic'
    if (-not (Test-Path -LiteralPath $DicPath)) {
        $client = New-Object System.Net.WebClient
        $client.Headers.Add('User-Agent', 'Termyx-Build')
        $client.DownloadFile($dicUrl, $DicPath)
    }
}

if (-not $AffPath) {
    New-Item -ItemType Directory -Force -Path $cacheDir | Out-Null
    $AffPath = Join-Path $cacheDir 'en.aff'
    if (-not (Test-Path -LiteralPath $AffPath)) {
        $client = New-Object System.Net.WebClient
        $client.Headers.Add('User-Agent', 'Termyx-Build')
        $client.DownloadFile($affUrl, $AffPath)
    }
}

Write-Output ('Reading ' + $DicPath)
Write-Output ('Reading ' + $AffPath)

$dicLines = Get-Content -LiteralPath $DicPath -Encoding UTF8
$affLines = Get-Content -LiteralPath $AffPath -Encoding UTF8

$stopwatch = [Diagnostics.Stopwatch]::StartNew()
$words = Expand-HunspellDictionary -DicLines $dicLines -AffLines $affLines
$stopwatch.Stop()

$outputDirectory = Split-Path -Parent $OutFile
if (-not (Test-Path -LiteralPath $outputDirectory)) {
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
}

Set-Content -LiteralPath $OutFile -Value $words -Encoding ASCII

Write-Output ('Wrote ' + $words.Count + ' words to ' + $OutFile + ' in ' + $stopwatch.Elapsed.TotalSeconds.ToString('0.0') + ' s')
```

- [ ] **Step 6: Generate the committed word list**

Run (downloads once, then expands for roughly 30-60 seconds):

```powershell
.\tools\Build-WordList.ps1
```

Expected: `Wrote <N> words to D:\Development\termyx\Termyx\assets\en_US.wordlist.txt in <T> s` where `<N>` is greater than 90000.

- [ ] **Step 7: Run the full test file**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: 29 passed, 0 failed.

- [ ] **Step 8: Commit**

```powershell
git add tools/Hunspell.ps1 tools/Build-WordList.ps1 Termyx/assets/en_US.wordlist.txt tests/Termyx.Tests.ps1
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat: add Hunspell expander and generated en_US word list"
```

---

### Task 5: AST Token Classifier

**Files:**
- Create: `Termyx/private/Token-Classifier.ps1`
- Modify: `Termyx/Termyx.psm1`
- Modify: `tests/Termyx.Tests.ps1`

**Interfaces:**
- Consumes: `$script:TermyxTokenKindCache` (initialized by the module).
- Produces: `Get-TermyxTokenKind -Line <string> -Start <int> -Length <int>` returning an object with `Kind` (`Skip`/`Command`/`Parameter`/`Argument`), `Command` (owning command for parameters, otherwise `$null`), `IsFirstToken` (bool), and `Reason`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/Termyx.Tests.ps1`:

```powershell
function Get-TermyxTokenKindForTest {
    param([string]$Line, [int]$Start, [int]$Length)
    return & (Get-Module Termyx) {
        param($l, $s, $n)
        Get-TermyxTokenKind -Line $l -Start $s -Length $n
    } $Line $Start $Length
}

Describe 'Token classifier' {
    BeforeAll {
        $script:moduleManifest = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx\Termyx.psd1'
        Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }
    }

    It 'classifies a command name at the start of the line' {
        $r = Get-TermyxTokenKindForTest -Line 'Get-ChldItem' -Start 0 -Length 12
        $r.Kind | Should -Be 'Command'
        $r.IsFirstToken | Should -BeTrue
    }

    It 'classifies a command name after a statement separator' {
        $r = Get-TermyxTokenKindForTest -Line 'Write-Host x; gti status' -Start 14 -Length 3
        $r.Kind | Should -Be 'Command'
        $r.IsFirstToken | Should -BeFalse
    }

    It 'classifies a bare argument' {
        $r = Get-TermyxTokenKindForTest -Line 'Write-Host hello' -Start 11 -Length 5
        $r.Kind | Should -Be 'Argument'
    }

    It 'classifies a parameter and reports its owner' {
        $r = Get-TermyxTokenKindForTest -Line 'Get-ChildItem -Reurse' -Start 15 -Length 6
        $r.Kind | Should -Be 'Parameter'
        $r.Command | Should -Be 'Get-ChildItem'
    }

    It 'skips a word inside double quotes' {
        $r = Get-TermyxTokenKindForTest -Line 'git commit -m "hello wrld"' -Start 20 -Length 4
        $r.Kind | Should -Be 'Skip'
        $r.Reason | Should -Be 'InString'
    }

    It 'skips a word inside an unclosed double quote' {
        $r = Get-TermyxTokenKindForTest -Line 'git commit -m "hello wrld' -Start 20 -Length 4
        $r.Kind | Should -Be 'Skip'
        $r.Reason | Should -Be 'InString'
    }

    It 'skips a word inside single quotes' {
        $r = Get-TermyxTokenKindForTest -Line "git commit -m 'hello wrld'" -Start 20 -Length 4
        $r.Kind | Should -Be 'Skip'
        $r.Reason | Should -Be 'InString'
    }

    It 'skips a word inside a here-string' {
        $line = "@`"`nhello wrld`n`"@"
        $r = Get-TermyxTokenKindForTest -Line $line -Start 9 -Length 4
        $r.Kind | Should -Be 'Skip'
        $r.Reason | Should -Be 'InString'
    }

    It 'skips a word inside a comment' {
        $r = Get-TermyxTokenKindForTest -Line 'Write-Host hi # comment wrld' -Start 24 -Length 4
        $r.Kind | Should -Be 'Skip'
        $r.Reason | Should -Be 'InComment'
    }

    It 'skips a variable reference' {
        $r = Get-TermyxTokenKindForTest -Line '$env:PATH' -Start 5 -Length 4
        $r.Kind | Should -Be 'Skip'
        $r.Reason | Should -Be 'InVariable'
    }

    It 'skips a numeric literal' {
        $r = Get-TermyxTokenKindForTest -Line '42' -Start 0 -Length 2
        $r.Kind | Should -Be 'Skip'
        $r.Reason | Should -Be 'InNumber'
    }

    It 'skips a word that is only part of a path' {
        $r = Get-TermyxTokenKindForTest -Line 'C:\temp\foo' -Start 8 -Length 3
        $r.Kind | Should -Be 'Skip'
        $r.Reason | Should -Be 'PartialToken'
    }

    It 'skips a word that is only part of a URL' {
        $r = Get-TermyxTokenKindForTest -Line 'https://example.com/x' -Start 8 -Length 7
        $r.Kind | Should -Be 'Skip'
        $r.Reason | Should -Be 'PartialToken'
    }

    It 'skips a word that is only part of a script path' {
        $r = Get-TermyxTokenKindForTest -Line '.\script.ps1' -Start 9 -Length 3
        $r.Kind | Should -Be 'Skip'
        $r.Reason | Should -Be 'PartialToken'
    }

    It 'skips a token that is not inside a command' {
        $r = Get-TermyxTokenKindForTest -Line '$obj.Property' -Start 5 -Length 8
        $r.Kind | Should -Be 'Skip'
        $r.Reason | Should -Be 'NoCommand'
    }

    It 'rejects out-of-range input' {
        $r = Get-TermyxTokenKindForTest -Line 'abc' -Start 2 -Length 9
        $r.Kind | Should -Be 'Skip'
        $r.Reason | Should -Be 'OutOfRange'
    }
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: every classifier test fails with `Get-TermyxTokenKind is not recognized`.

- [ ] **Step 3: Create the classifier**

Create `Termyx/private/Token-Classifier.ps1`:

```powershell
function New-TermyxKindResult {
    param(
        [string]$Kind,
        [string]$Command = $null,
        [bool]$IsFirstToken = $false,
        [string]$Reason = 'None'
    )

    return [pscustomobject]@{
        Kind         = $Kind
        Command      = $Command
        IsFirstToken = $IsFirstToken
        Reason       = $Reason
    }
}

function Save-TermyxKindResult {
    param([string]$Key, $Result)

    if ($script:TermyxTokenKindCache.Count -ge 250) {
        $script:TermyxTokenKindCache.Clear()
    }
    $script:TermyxTokenKindCache[$Key] = $Result
    return $Result
}

function Get-TermyxTokenKind {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string]$Line,
        [Parameter(Mandatory)][int]$Start,
        [Parameter(Mandatory)][int]$Length
    )

    if ($Start -lt 0 -or $Length -le 0 -or ($Start + $Length) -gt $Line.Length) {
        return New-TermyxKindResult -Kind 'Skip' -Reason 'OutOfRange'
    }

    $key = [string]$Start + '|' + [string]$Length + '|' + $Line
    if ($script:TermyxTokenKindCache.ContainsKey($key)) {
        return $script:TermyxTokenKindCache[$key]
    }

    $tokens = $null
    $errors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseInput($Line, [ref]$tokens, [ref]$errors)

    $last = $Start + $Length - 1
    $covering = $null
    foreach ($token in $tokens) {
        if ($token.Extent.StartOffset -le $last -and $token.Extent.EndOffset -gt $last) {
            $covering = $token
            break
        }
    }

    if ($null -ne $covering) {
        $tokenKind = [string]$covering.Kind
        $reason = $null

        if ($tokenKind -eq 'StringExpandable' -or $tokenKind -eq 'StringLiteral' -or
            $tokenKind -eq 'HereStringExpandable' -or $tokenKind -eq 'HereStringLiteral') {
            $reason = 'InString'
        }
        elseif ($tokenKind -eq 'Comment') { $reason = 'InComment' }
        elseif ($tokenKind -eq 'Variable') { $reason = 'InVariable' }
        elseif ($tokenKind -eq 'Number') { $reason = 'InNumber' }

        if ($null -ne $reason) {
            $skip = New-TermyxKindResult -Kind 'Skip' -Reason $reason
            return Save-TermyxKindResult -Key $key -Result $skip
        }
    }

    $commands = @($ast.FindAll({
        param($node)
        $node -is [System.Management.Automation.Language.CommandAst] -and
        $node.Extent.StartOffset -le $Start -and
        $node.Extent.EndOffset -ge $last
    }, $true))

    if ($commands.Count -eq 0) {
        $skip = New-TermyxKindResult -Kind 'Skip' -Reason 'NoCommand'
        return Save-TermyxKindResult -Key $key -Result $skip
    }

    $commandAst = $commands[0]
    foreach ($candidate in $commands) {
        if ($candidate.Extent.StartOffset -gt $commandAst.Extent.StartOffset) {
            $commandAst = $candidate
        }
    }

    $element = $null
    foreach ($candidate in $commandAst.CommandElements) {
        if ($candidate.Extent.StartOffset -le $last -and $candidate.Extent.EndOffset -ge $last) {
            $element = $candidate
            break
        }
    }

    if ($null -eq $element) {
        $skip = New-TermyxKindResult -Kind 'Skip' -Reason 'NoElement'
        return Save-TermyxKindResult -Key $key -Result $skip
    }

    if ($element -is [System.Management.Automation.Language.CommandParameterAst]) {
        if ($element.Extent.StartOffset -ne ($Start - 1)) {
            $skip = New-TermyxKindResult -Kind 'Skip' -Reason 'PartialParameter'
            return Save-TermyxKindResult -Key $key -Result $skip
        }
        $owner = $commandAst.GetCommandName()
        $parameter = New-TermyxKindResult -Kind 'Parameter' -Command $owner
        return Save-TermyxKindResult -Key $key -Result $parameter
    }

    if ($element.Extent.StartOffset -ne $Start -or $element.Extent.EndOffset -ne ($Start + $Length)) {
        $skip = New-TermyxKindResult -Kind 'Skip' -Reason 'PartialToken'
        return Save-TermyxKindResult -Key $key -Result $skip
    }

    $firstElement = $commandAst.CommandElements[0]
    if ($element.Extent.StartOffset -eq $firstElement.Extent.StartOffset) {
        $command = New-TermyxKindResult -Kind 'Command' -IsFirstToken ($Start -eq 0)
        return Save-TermyxKindResult -Key $key -Result $command
    }

    $argument = New-TermyxKindResult -Kind 'Argument'
    return Save-TermyxKindResult -Key $key -Result $argument
}
```

- [ ] **Step 4: Wire the classifier into the module**

In `Termyx/Termyx.psm1`, immediately before `function Initialize-TermyxEngine {`, add:

```powershell
$script:TermyxTokenKindCache = @{}

. (Join-Path $script:TermyxRoot 'private\Token-Classifier.ps1')
```

- [ ] **Step 5: Run the tests**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: all classifier tests pass, 0 failed.

- [ ] **Step 6: Commit**

```powershell
git add Termyx/private/Token-Classifier.ps1 Termyx/Termyx.psm1 tests/Termyx.Tests.ps1
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat: add AST-based token classifier with skip rules"
```

---

### Task 6: Pure Decision Function

**Files:**
- Create: `Termyx/private/Decision.ps1`
- Modify: `Termyx/Termyx.psm1`
- Modify: `tests/Termyx.Tests.ps1`

**Interfaces:**
- Consumes: `Get-TermyxTokenKind` from Task 5; `Corrector.SuggestWord`, `SuggestCommand`, `SuggestParameter`, `IsKnownWord`, `SetParameters` from Tasks 2-3; `$script:TermyxOptions`, `$script:TermyxEnabled`, `$script:TermyxExceptionsPath`.
- Produces: `Invoke-TermyxCorrection -Line <string> -Cursor <int>` returning `ShouldApply` (bool), `Start`, `Length`, `Fix`, `NewLine`, `NewCursor`, `Kind`, `Reason`; `Get-TermyxParameterNames -Command <string>`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/Termyx.Tests.ps1`:

```powershell
function Invoke-TermyxDecisionForTest {
    param([string]$Line, [int]$Cursor)
    return & (Get-Module Termyx) {
        param($l, $c)
        Invoke-TermyxCorrection -Line $l -Cursor $c
    } $Line $Cursor
}

Describe 'Decision function' {
    BeforeAll {
        $script:moduleManifest = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx\Termyx.psd1'
        Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }

        $script:decisionFixture = Join-Path $env:TEMP ('termyx-decision-' + [guid]::NewGuid().ToString('N') + '.txt')
        @(
            'the', 'receive', 'running', 'cat', 'cot', 'hello', 'world',
            'testing', 'powershell', 'file', 'path', 'status', 'commit', 'more'
        ) | Set-Content -LiteralPath $script:decisionFixture -Encoding ASCII

        $script:exceptionsFixture = Join-Path $env:TEMP ('termyx-exceptions-' + [guid]::NewGuid().ToString('N') + '.txt')
    }

    AfterAll {
        Remove-Item -LiteralPath $script:decisionFixture -ErrorAction SilentlyContinue
        Remove-Item -LiteralPath $script:exceptionsFixture -ErrorAction SilentlyContinue
    }

    BeforeEach {
        [Termyx.Corrector]::Initialize($script:decisionFixture, $null)
        [Termyx.Corrector]::SetCommands(@('Get-ChildItem', 'Get-Content', 'Get-Item', 'Write-Host', 'Set-Location'))

        & (Get-Module Termyx) {
            param($exceptionsPath)
            $script:TermyxEnabled = $true
            $script:TermyxOptions = @{
                MaxArgumentDistance = 2
                SkipWords           = @()
                VerboseCorrections  = $false
            }
            $script:TermyxExceptionsPath = $exceptionsPath
            $script:TermyxExceptions = $null
            $script:TermyxExceptionsTime = $null
            $script:TermyxParameterNameCache = @{}
        } $script:exceptionsFixture
    }

    It 'corrects a first-token command typo' {
        $r = Invoke-TermyxDecisionForTest -Line 'Get-ChldItem' -Cursor 12
        $r.ShouldApply | Should -BeTrue
        $r.Kind | Should -Be 'Command'
        $r.NewLine | Should -Be 'Get-ChildItem'
        $r.NewCursor | Should -Be 13
    }

    It 'applies a two-edit command fix only for the first token' {
        $r = Invoke-TermyxDecisionForTest -Line 'Get-ChldItm' -Cursor 11
        $r.ShouldApply | Should -BeTrue
        $r.NewLine | Should -Be 'Get-ChildItem'
    }

    It 'does not apply a two-edit command fix later in the line' {
        $line = 'Get-Content x; Get-ChldItm'
        $r = Invoke-TermyxDecisionForTest -Line $line -Cursor $line.Length
        $r.ShouldApply | Should -BeFalse
        $r.NewLine | Should -Be $line
    }

    It 'corrects a parameter typo' {
        $line = 'Get-ChildItem -Reurse'
        $r = Invoke-TermyxDecisionForTest -Line $line -Cursor $line.Length
        $r.ShouldApply | Should -BeTrue
        $r.Kind | Should -Be 'Parameter'
        $r.NewLine | Should -Be 'Get-ChildItem -Recurse'
        $r.NewCursor | Should -Be 22
    }

    It 'does not expand a parameter abbreviation' {
        $line = 'Get-ChildItem -Rec'
        $r = Invoke-TermyxDecisionForTest -Line $line -Cursor $line.Length
        $r.ShouldApply | Should -BeFalse
        $r.Reason | Should -Be 'Prefix'
    }

    It 'corrects a misspelled argument' {
        $line = 'Write-Host helo'
        $r = Invoke-TermyxDecisionForTest -Line $line -Cursor $line.Length
        $r.ShouldApply | Should -BeTrue
        $r.Kind | Should -Be 'Argument'
        $r.NewLine | Should -Be 'Write-Host hello'
        $r.NewCursor | Should -Be 16
    }

    It 'keeps the cursor position when the token is mid-line' {
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host helo more' -Cursor 15
        $r.ShouldApply | Should -BeTrue
        $r.NewLine | Should -Be 'Write-Host hello more'
        $r.NewCursor | Should -Be 16
    }

    It 'preserves casing for argument fixes' {
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host Teh' -Cursor 15
        $r.NewLine | Should -Be 'Write-Host The'
    }

    It 'refuses an ambiguous argument' {
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host cit' -Cursor 15
        $r.ShouldApply | Should -BeFalse
        $r.NewLine | Should -Be 'Write-Host cit'
    }

    It 'leaves a known word alone' {
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host hello' -Cursor 16
        $r.ShouldApply | Should -BeFalse
        $r.Reason | Should -Be 'Known'
    }

    It 'leaves a hyphenated argument alone' {
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host termyx' -Cursor 22
        $r.ShouldApply | Should -BeFalse
        $r.Reason | Should -Be 'Compound'
    }

    It 'leaves short words alone' {
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host ab' -Cursor 13
        $r.ShouldApply | Should -BeFalse
        $r.Reason | Should -Be 'TooShort'
    }

    It 'leaves acronyms alone' {
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host JSON' -Cursor 15
        $r.ShouldApply | Should -BeFalse
        $r.Reason | Should -Be 'Acronym'
    }

    It 'leaves numbers alone' {
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host 42' -Cursor 13
        $r.ShouldApply | Should -BeFalse
        $r.Reason | Should -Be 'NoToken'
    }

    It 'leaves paths alone' {
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host C:\temp\foo' -Cursor 20
        $r.ShouldApply | Should -BeFalse
        $r.Reason | Should -Be 'InPath'
    }

    It 'leaves variables alone' {
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host $env:PATH' -Cursor 19
        $r.ShouldApply | Should -BeFalse
        $r.Reason | Should -Be 'InPath'
    }

    It 'leaves quoted text alone' {
        $line = 'Write-Host "helo"'
        $r = Invoke-TermyxDecisionForTest -Line $line -Cursor 16
        $r.ShouldApply | Should -BeFalse
        $r.Reason | Should -Be 'InString'
    }

    It 'leaves comments alone' {
        $line = 'Write-Host hi # helo'
        $r = Invoke-TermyxDecisionForTest -Line $line -Cursor $line.Length
        $r.ShouldApply | Should -BeFalse
        $r.Reason | Should -Be 'InComment'
    }

    It 'honors the exceptions file' {
        Set-Content -LiteralPath $script:exceptionsFixture -Value 'helo' -Encoding ASCII
        & (Get-Module Termyx) { $script:TermyxExceptionsTime = $null }
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host helo' -Cursor 15
        $r.ShouldApply | Should -BeFalse
        $r.Reason | Should -Be 'Exception'
    }

    It 'honors the SkipWords option' {
        & (Get-Module Termyx) {
            $script:TermyxOptions.SkipWords = @('helo')
            $script:TermyxExceptionsTime = $null
        }
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host helo' -Cursor 15
        $r.ShouldApply | Should -BeFalse
        $r.Reason | Should -Be 'Exception'
    }

    It 'returns Disabled when the module is off' {
        & (Get-Module Termyx) { $script:TermyxEnabled = $false }
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host helo' -Cursor 15
        $r.ShouldApply | Should -BeFalse
        $r.Reason | Should -Be 'Disabled'
    }

    It 'honors MaxArgumentDistance 1' {
        & (Get-Module Termyx) { $script:TermyxOptions.MaxArgumentDistance = 1 }
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host hlo' -Cursor 14
        $r.ShouldApply | Should -BeFalse
    }
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: every decision test fails with `Invoke-TermyxCorrection is not recognized`.

- [ ] **Step 3: Create the decision function**

Create `Termyx/private/Decision.ps1`:

```powershell
function New-TermyxDecisionResult {
    param([string]$Line, [int]$Cursor)

    return [pscustomobject]@{
        ShouldApply = $false
        Start       = -1
        Length      = 0
        Fix         = $null
        NewLine     = $Line
        NewCursor   = $Cursor
        Kind        = 'Skip'
        Reason      = 'None'
    }
}

function Get-TermyxExceptions {
    $now = [datetime]::UtcNow
    if ($null -ne $script:TermyxExceptionsTime -and
        ($now - $script:TermyxExceptionsTime).TotalSeconds -lt 5) {
        return $script:TermyxExceptions
    }

    $set = New-Object 'System.Collections.Generic.HashSet[string]' -ArgumentList @([StringComparer]::OrdinalIgnoreCase)

    if ($script:TermyxExceptionsPath -and (Test-Path -LiteralPath $script:TermyxExceptionsPath)) {
        foreach ($line in @(Get-Content -LiteralPath $script:TermyxExceptionsPath)) {
            $entry = $line.Trim()
            if ($entry.Length -gt 0 -and -not $entry.StartsWith('#')) {
                [void]$set.Add($entry)
            }
        }
    }

    foreach ($word in @($script:TermyxOptions.SkipWords)) {
        if ($word) { [void]$set.Add([string]$word) }
    }

    $script:TermyxExceptions = $set
    $script:TermyxExceptionsTime = $now
    return $set
}

function Get-TermyxParameterNames {
    param([string]$Command)

    $key = $Command.ToLowerInvariant()
    if ($script:TermyxParameterNameCache.ContainsKey($key)) {
        return ,$script:TermyxParameterNameCache[$key]
    }

    $commandInfo = Get-Command -Name $Command -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($null -eq $commandInfo) {
        return $null
    }

    $names = @($commandInfo.Parameters.Keys)
    $script:TermyxParameterNameCache[$key] = $names
    [Termyx.Corrector]::SetParameters($Command, $names)
    return ,$names
}

function Invoke-TermyxCorrection {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][AllowEmptyString()][string]$Line,
        [Parameter(Mandatory)][int]$Cursor
    )

    $result = New-TermyxDecisionResult -Line $Line -Cursor $Cursor

    if (-not $script:TermyxEnabled) {
        $result.Reason = 'Disabled'
        return $result
    }

    if ($Cursor -lt 0 -or $Cursor -gt $Line.Length) {
        $result.Reason = 'BadCursor'
        return $result
    }

    $prefix = ''
    if ($Cursor -gt 0) { $prefix = $Line.Substring(0, $Cursor) }

    $match = [regex]::Match($prefix, '[A-Za-z][A-Za-z0-9_-]*$')
    if (-not $match.Success) {
        $result.Reason = 'NoToken'
        return $result
    }

    $start = $match.Index
    $token = $match.Value
    $length = $token.Length

    if ($start -gt 0) {
        $previous = $Line[$start - 1]
        $blocked = [char[]]@('\', '/', ':', '.', '$', '@', '"', "'")
        if ($blocked -contains $previous) {
            $result.Reason = 'InPath'
            return $result
        }
    }

    if ($length -lt 3) {
        $result.Reason = 'TooShort'
        return $result
    }

    if ($token -cmatch '^[A-Z0-9]+$') {
        $result.Reason = 'Acronym'
        return $result
    }

    if ($token -match '\d') {
        $result.Reason = 'HasDigit'
        return $result
    }

    if ((Get-TermyxExceptions).Contains($token)) {
        $result.Reason = 'Exception'
        return $result
    }

    $kindInfo = Get-TermyxTokenKind -Line $Line -Start $start -Length $length
    if ($kindInfo.Kind -eq 'Skip') {
        $result.Reason = $kindInfo.Reason
        return $result
    }
    $result.Kind = $kindInfo.Kind

    $fix = $null

    if ($kindInfo.Kind -eq 'Command') {
        $maxDistance = 1
        if ($kindInfo.IsFirstToken) { $maxDistance = 2 }

        $fix = [Termyx.Corrector]::SuggestCommand($token, $maxDistance)
        if ($fix -and $fix -eq $token) {
            $result.Reason = 'Known'
            return $result
        }
        if ($fix -and (Get-Command -Name $token -ErrorAction SilentlyContinue)) {
            $result.Reason = 'Known'
            return $result
        }
    }
    elseif ($kindInfo.Kind -eq 'Parameter') {
        if (-not $kindInfo.Command) {
            $result.Reason = 'NoOwner'
            return $result
        }

        $names = Get-TermyxParameterNames -Command $kindInfo.Command
        if ($null -eq $names) {
            $result.Reason = 'UnknownCommand'
            return $result
        }
        if ($names -contains $token) {
            $result.Reason = 'Known'
            return $result
        }

        $isPrefix = $false
        foreach ($name in $names) {
            if ($name.Length -gt $token.Length -and
                $name.StartsWith($token, [StringComparison]::OrdinalIgnoreCase)) {
                $isPrefix = $true
                break
            }
        }
        if ($isPrefix) {
            $result.Reason = 'Prefix'
            return $result
        }

        $fix = [Termyx.Corrector]::SuggestParameter($kindInfo.Command, $token, 2)
    }
    else {
        if ($token -match '[-_]') {
            $result.Reason = 'Compound'
            return $result
        }
        if ([Termyx.Corrector]::IsKnownWord($token)) {
            $result.Reason = 'Known'
            return $result
        }
        if (Test-Path -LiteralPath $token) {
            $result.Reason = 'PathExists'
            return $result
        }

        $maxDistance = 1
        if ($token.Length -le 6) {
            $maxDistance = [Math]::Min(2, $script:TermyxOptions.MaxArgumentDistance)
        }
        $fix = [Termyx.Corrector]::SuggestWord($token, $maxDistance)
    }

    if ([string]::IsNullOrEmpty($fix) -or $fix -eq $token) {
        $result.Reason = 'NoCandidate'
        return $result
    }

    $result.ShouldApply = $true
    $result.Start = $start
    $result.Length = $length
    $result.Fix = $fix
    $result.NewLine = $Line.Substring(0, $start) + $fix + $Line.Substring($start + $length)
    $result.NewCursor = $start + $fix.Length
    $result.Reason = 'Corrected'
    return $result
}
```

- [ ] **Step 4: Wire the decision function and its state into the module**

In `Termyx/Termyx.psm1`, immediately after the `$script:TermyxUserWordsPath` line, add:

```powershell
$script:TermyxExceptionsPath = Join-Path $cacheDir 'exceptions.txt'
$script:TermyxExceptions = $null
$script:TermyxExceptionsTime = $null
$script:TermyxParameterNameCache = @{}
```

Immediately after the `Token-Classifier.ps1` dot-source line, add:

```powershell
. (Join-Path $script:TermyxRoot 'private\Decision.ps1')
```

- [ ] **Step 5: Run the tests**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: 67 passed, 0 failed. If any classification assertion differs, print the actual object with `$r | Format-List` and correct the classifier, not the test.

- [ ] **Step 6: Commit**

```powershell
git add Termyx/private/Decision.ps1 Termyx/Termyx.psm1 tests/Termyx.Tests.ps1
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat: add pure correction decision function with guard rails"
```

---

### Task 7: Key Handler, Public Commands, Module Lifecycle

**Files:**
- Create: `Termyx/private/KeyHandlers.ps1`
- Modify: `Termyx/Termyx.psm1`
- Modify: `Termyx/Termyx.psd1`
- Modify: `tests/Termyx.Tests.ps1`

**Interfaces:**
- Consumes: `Invoke-TermyxCorrection`, `Update-TermyxCommandCache`, the engine.
- Produces: public commands `Enable-TermyxAutocorrect`, `Disable-TermyxAutocorrect`, `Get-TermyxStatus`, `Add-TermyxWord`, `Update-TermyxCommandCache`, `Set-TermyxOption`; module-internal `Invoke-TermyxSpaceKeyPress`.

- [ ] **Step 1: Write the failing tests**

Append to `tests/Termyx.Tests.ps1`:

```powershell
Describe 'Module commands' {
    BeforeAll {
        $script:moduleManifest = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx\Termyx.psd1'
        Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }

        $script:userWordFixture = Join-Path $env:TEMP ('termyx-userwords-' + [guid]::NewGuid().ToString('N') + '.txt')
        & (Get-Module Termyx) {
            param($path)
            $script:TermyxUserWordsPath = $path
            $script:TermyxExceptionsPath = Join-Path $env:TEMP 'termyx-missing-exceptions.txt'
            $script:TermyxExceptionsTime = $null
            $script:TermyxEnabled = $true
        } $script:userWordFixture
    }

    AfterAll {
        Remove-Item -LiteralPath $script:userWordFixture -ErrorAction SilentlyContinue
    }

    It 'exports exactly the documented functions' {
        $expected = @(
            'Add-TermyxWord',
            'Disable-TermyxAutocorrect',
            'Enable-TermyxAutocorrect',
            'Get-TermyxStatus',
            'Set-TermyxOption',
            'Update-TermyxCommandCache'
        )
        $actual = @((Get-Module Termyx).ExportedFunctions.Keys | Sort-Object)
        ($actual -join ',') | Should -Be (($expected | Sort-Object) -join ',')
    }

    It 'reports status' {
        $status = Get-TermyxStatus
        $status.Enabled | Should -BeTrue
        $status.WordCount | Should -BeGreaterThan 0
        $status.CommandCount | Should -BeGreaterThan 0
        $status.InitError | Should -BeNullOrEmpty
    }

    It 'adds a word to the user dictionary' {
        Add-TermyxWord -Word 'termyxzz'
        [Termyx.Corrector]::IsKnownWord('termyxzz') | Should -BeTrue
        (Get-Content -LiteralPath $script:userWordFixture -Raw).Contains('termyxzz') | Should -BeTrue
    }

    It 'does not duplicate a user word' {
        Add-TermyxWord -Word 'termyxzz'
        @(Get-Content -LiteralPath $script:userWordFixture | Where-Object { $_ -eq 'termyxzz' }).Count | Should -Be 1
    }

    It 'refreshes the command cache' {
        Update-TermyxCommandCache
        [Termyx.Corrector]::CommandCount | Should -BeGreaterThan 0
    }

    It 'disables and re-enables through Set-TermyxOption' {
        Set-TermyxOption -Enabled:$false
        (Get-TermyxStatus).Enabled | Should -BeFalse

        $blocked = & (Get-Module Termyx) {
            Invoke-TermyxCorrection -Line 'Write-Host helo' -Cursor 15
        }
        $blocked.Reason | Should -Be 'Disabled'

        & (Get-Module Termyx) {
            $script:TermyxEnabled = $true
        }
        (Get-TermyxStatus).Enabled | Should -BeTrue
    }
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: exported-function list is empty and `Get-TermyxStatus` is not recognized.

- [ ] **Step 3: Create the key handler adapter**

Create `Termyx/private/KeyHandlers.ps1`:

```powershell
function Get-TermyxSpaceHandler {
    return {
        param($key, $arg)

        try {
            $module = Get-Module Termyx
            if ($null -ne $module) {
                & $module { Invoke-TermyxSpaceKeyPress }
            }
        }
        catch {
        }

        [Microsoft.PowerShell.PSConsoleReadLine]::SelfInsert()
    }
}

function Invoke-TermyxSpaceKeyPress {
    $line = $null
    $cursor = 0
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)

    $result = Invoke-TermyxCorrection -Line $line -Cursor $cursor

    if (-not $result.ShouldApply -and $result.Kind -eq 'Command' -and $result.Reason -eq 'NoCandidate') {
        $age = ([datetime]::UtcNow - $script:TermyxCommandCacheTime).TotalSeconds
        if ($age -gt 30) {
            Update-TermyxCommandCache
            $result = Invoke-TermyxCorrection -Line $line -Cursor $cursor
        }
    }

    if (-not $result.ShouldApply) { return }

    $last = $script:TermyxLastCorrection
    if ($null -ne $last -and
        $last.Line -eq $line -and
        $last.Start -eq $result.Start -and
        $last.Fix -eq $result.Fix) {
        return
    }

    [Microsoft.PowerShell.PSConsoleReadLine]::Replace($result.Start, $result.Length, $result.Fix)
    [Microsoft.PowerShell.PSConsoleReadLine]::SetCursorPosition($result.NewCursor)
    $script:TermyxLastCorrection = @{
        Line  = $line
        Start = $result.Start
        Fix   = $result.Fix
    }

    if ($script:TermyxOptions.VerboseCorrections) {
        $entry = [datetime]::Now.ToString('s') + ' ' + $result.Kind + ' ' + $result.Reason + ': ' + $line + ' -> ' + $result.NewLine
        Add-Content -LiteralPath $script:TermyxLogPath -Value $entry -Encoding ASCII
    }
}
```

- [ ] **Step 4: Replace `Termyx.psm1` with the final module script**

Replace the whole file with:

```powershell
param(
    [hashtable]$Options = @{}
)

if ($null -eq $Options -or $Options -isnot [hashtable]) {
    $Options = @{}
}

$script:TermyxRoot = $PSScriptRoot
$script:TermyxEnabled = $true
$script:TermyxInitError = $null
$script:TermyxEngineSource = 'none'
$script:TermyxOptions = @{
    MaxArgumentDistance = 2
    SkipWords           = @()
    VerboseCorrections  = $false
}

$cacheDir = Join-Path $env:LOCALAPPDATA 'Termyx'
$script:TermyxUserWordsPath = Join-Path $cacheDir 'user-words.txt'
$script:TermyxExceptionsPath = Join-Path $cacheDir 'exceptions.txt'
$script:TermyxLogPath = Join-Path $cacheDir 'corrections.log'
$script:TermyxExceptions = $null
$script:TermyxExceptionsTime = $null
$script:TermyxParameterNameCache = @{}
$script:TermyxTokenKindCache = @{}
$script:TermyxCommandCacheTime = [datetime]::MinValue
$script:TermyxLastCorrection = $null

. (Join-Path $script:TermyxRoot 'private\Token-Classifier.ps1')
. (Join-Path $script:TermyxRoot 'private\Decision.ps1')
. (Join-Path $script:TermyxRoot 'private\KeyHandlers.ps1')

function Initialize-TermyxEngine {
    if ('Termyx.Corrector' -as [type]) {
        $script:TermyxEngineSource = 'loaded'
        return
    }

    $source = Join-Path $script:TermyxRoot 'lib\Corrector.cs'
    if (-not (Test-Path -LiteralPath $source)) {
        throw ('Engine source not found: ' + $source)
    }

    if (-not (Test-Path -LiteralPath $cacheDir)) {
        New-Item -ItemType Directory -Force -Path $cacheDir | Out-Null
    }

    $dll = Join-Path $cacheDir 'Corrector.dll'
    $hashFile = Join-Path $cacheDir 'Corrector.sha256'
    $sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash

    $cachedHash = ''
    if (Test-Path -LiteralPath $hashFile) {
        $cachedHash = (Get-Content -LiteralPath $hashFile -Raw).Trim()
    }

    if (-not (Test-Path -LiteralPath $dll) -or $cachedHash -ne $sourceHash) {
        if (Test-Path -LiteralPath $dll) {
            Remove-Item -LiteralPath $dll -Force
        }
        Add-Type -Path $source -OutputAssembly $dll
        Set-Content -LiteralPath $hashFile -Value $sourceHash -Encoding ASCII
        $script:TermyxEngineSource = 'compiled'
    }
    else {
        $script:TermyxEngineSource = 'cached'
    }

    Add-Type -Path $dll
}

function Update-TermyxCommandCache {
    [CmdletBinding()]
    param()

    $names = @(
        Get-Command -CommandType Cmdlet, Function, Alias, ExternalScript, Filter -ErrorAction SilentlyContinue |
            ForEach-Object { $_.Name }
    )

    [Termyx.Corrector]::SetCommands($names)
    $script:TermyxParameterNameCache = @{}
    $script:TermyxCommandCacheTime = [datetime]::UtcNow
}

function Enable-TermyxAutocorrect {
    [CmdletBinding()]
    param()

    if ($script:TermyxInitError) {
        throw ('Termyx cannot enable: ' + $script:TermyxInitError)
    }

    Import-Module PSReadLine -ErrorAction Stop
    Set-PSReadLineKeyHandler -Key Space -ScriptBlock (Get-TermyxSpaceHandler)
    $script:TermyxLastCorrection = $null
    $script:TermyxEnabled = $true
}

function Disable-TermyxAutocorrect {
    [CmdletBinding()]
    param()

    try {
        if (Get-Command Set-PSReadLineKeyHandler -ErrorAction SilentlyContinue) {
            Set-PSReadLineKeyHandler -Key Space -Function SelfInsert
        }
    }
    catch {
    }

    $script:TermyxEnabled = $false
    $script:TermyxLastCorrection = $null
}

function Get-TermyxStatus {
    [CmdletBinding()]
    param()

    $wordCount = 0
    $commandCount = 0
    if ('Termyx.Corrector' -as [type]) {
        $wordCount = [Termyx.Corrector]::WordCount
        $commandCount = [Termyx.Corrector]::CommandCount
    }

    return [pscustomobject]@{
        Enabled        = [bool]$script:TermyxEnabled
        WordCount      = $wordCount
        CommandCount   = $commandCount
        EngineSource   = $script:TermyxEngineSource
        InitError      = $script:TermyxInitError
        UserWordsPath  = $script:TermyxUserWordsPath
        ExceptionsPath = $script:TermyxExceptionsPath
    }
}

function Add-TermyxWord {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][string]$Word
    )

    $normalized = $Word.Trim().ToLowerInvariant()
    if ($normalized -notmatch '^[a-z][a-z''-]*$') {
        throw ('Not a valid dictionary word: ' + $Word)
    }

    $directory = Split-Path -Parent $script:TermyxUserWordsPath
    if ($directory -and -not (Test-Path -LiteralPath $directory)) {
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
    }

    $existing = @()
    if (Test-Path -LiteralPath $script:TermyxUserWordsPath) {
        $existing = @(Get-Content -LiteralPath $script:TermyxUserWordsPath)
    }
    if ($existing -notcontains $normalized) {
        Add-Content -LiteralPath $script:TermyxUserWordsPath -Value $normalized -Encoding ASCII
    }

    [Termyx.Corrector]::AddUserWord($normalized)
}

function Set-TermyxOption {
    [CmdletBinding()]
    param(
        [bool]$Enabled,
        [int]$MaxArgumentDistance,
        [string[]]$SkipWords,
        [bool]$VerboseCorrections
    )

    if ($PSBoundParameters.ContainsKey('MaxArgumentDistance')) {
        if ($MaxArgumentDistance -lt 1 -or $MaxArgumentDistance -gt 2) {
            throw 'MaxArgumentDistance must be 1 or 2.'
        }
        $script:TermyxOptions.MaxArgumentDistance = $MaxArgumentDistance
    }

    if ($PSBoundParameters.ContainsKey('SkipWords')) {
        $script:TermyxOptions.SkipWords = @($SkipWords)
        $script:TermyxExceptionsTime = $null
    }

    if ($PSBoundParameters.ContainsKey('VerboseCorrections')) {
        $script:TermyxOptions.VerboseCorrections = $VerboseCorrections
    }

    if ($PSBoundParameters.ContainsKey('Enabled')) {
        if ($Enabled) {
            Enable-TermyxAutocorrect
        }
        else {
            Disable-TermyxAutocorrect
        }
    }
}

Export-ModuleMember -Function @(
    'Enable-TermyxAutocorrect',
    'Disable-TermyxAutocorrect',
    'Get-TermyxStatus',
    'Add-TermyxWord',
    'Update-TermyxCommandCache',
    'Set-TermyxOption'
)

if ($env:TERMYX_AUTOCORRECT -eq '0') {
    $script:TermyxEnabled = $false
    $script:TermyxInitError = 'Disabled by TERMYX_AUTOCORRECT=0'
}
else {
    try {
        Initialize-TermyxEngine

        $wordList = Join-Path $script:TermyxRoot 'assets\en_US.wordlist.txt'
        [Termyx.Corrector]::Initialize($wordList, $script:TermyxUserWordsPath)
        Update-TermyxCommandCache

        $installHandler = $true
        if ($Options.ContainsKey('InstallHandler')) {
            $installHandler = [bool]$Options.InstallHandler
        }

        if ($installHandler) {
            try {
                Enable-TermyxAutocorrect
            }
            catch {
                Write-Warning ('Termyx loaded but the Space handler was not installed: ' + $_.Exception.Message)
            }
        }
    }
    catch {
        $script:TermyxEnabled = $false
        $script:TermyxInitError = $_.Exception.Message
        Write-Warning ('Termyx disabled: ' + $_.Exception.Message)
    }
}
```

- [ ] **Step 5: Export the commands in the manifest**

In `Termyx/Termyx.psd1`, replace:

```powershell
    FunctionsToExport = @()
```

with:

```powershell
    FunctionsToExport = @(
        'Enable-TermyxAutocorrect',
        'Disable-TermyxAutocorrect',
        'Get-TermyxStatus',
        'Add-TermyxWord',
        'Update-TermyxCommandCache',
        'Set-TermyxOption'
    )
```

- [ ] **Step 6: Run the tests**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: 73 passed, 0 failed.

- [ ] **Step 7: Verify a clean non-interactive import**

Run:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -Command "Import-Module .\Termyx\Termyx.psd1; Get-TermyxStatus | Format-List"
```

Expected: an object with `Enabled : True`, a `WordCount` above 90000, and a `CommandCount` above 0. Because this host has no PSReadLine console, the handler install may warn; that warning is acceptable in a non-interactive host and must not appear in a normal console.

- [ ] **Step 8: Commit**

```powershell
git add Termyx/private/KeyHandlers.ps1 Termyx/Termyx.psm1 Termyx/Termyx.psd1 tests/Termyx.Tests.ps1
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat: add PSReadLine Space handler, public commands and module lifecycle"
```

---

### Task 8: Install Script, README, Full Verification

**Files:**
- Create: `tools/Install.ps1`
- Create: `README.md`
- Modify: `tests/Termyx.Tests.ps1`

**Interfaces:**
- Consumes: the module directory and manifest.
- Produces: `tools/Install.ps1` with `-ModuleSource`, `-ModuleRoot`, `-ProfilePath`, `-Uninstall`; README.

- [ ] **Step 1: Write the failing install tests**

Append to `tests/Termyx.Tests.ps1`:

```powershell
Describe 'Install script' {
    BeforeAll {
        $script:sandbox = Join-Path $env:TEMP ('termyx-install-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $script:sandbox | Out-Null

        $script:installer = Join-Path (Split-Path -Parent $PSScriptRoot) 'tools\Install.ps1'
        $script:moduleSource = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx'
        $script:moduleRoot = Join-Path $script:sandbox 'Modules'
        $script:profilePath = Join-Path $script:sandbox 'profile.ps1'
    }

    AfterAll {
        Remove-Item -LiteralPath $script:sandbox -Recurse -Force -ErrorAction SilentlyContinue
    }

    It 'copies the module and appends the guarded import line' {
        & $script:installer -ModuleSource $script:moduleSource -ModuleRoot $script:moduleRoot -ProfilePath $script:profilePath

        (Test-Path -LiteralPath (Join-Path $script:moduleRoot 'Termyx')) | Should -BeTrue
        (Test-Path -LiteralPath (Join-Path $script:moduleRoot 'Termyx\0.1.0\Termyx.psd1')) | Should -BeTrue

        $profileText = Get-Content -LiteralPath $script:profilePath -Raw
        $profileText.Contains('Import-Module Termyx') | Should -BeTrue
        $profileText.Contains('Get-Module -ListAvailable Termyx') | Should -BeTrue
    }

    It 'is idempotent' {
        & $script:installer -ModuleSource $script:moduleSource -ModuleRoot $script:moduleRoot -ProfilePath $script:profilePath
        & $script:installer -ModuleSource $script:moduleSource -ModuleRoot $script:moduleRoot -ProfilePath $script:profilePath

        @(Get-Content -LiteralPath $script:profilePath | Where-Object { $_ -match 'Import-Module\s+Termyx' }).Count | Should -Be 1
    }

    It 'uninstalls the module and the profile line' {
        & $script:installer -ModuleSource $script:moduleSource -ModuleRoot $script:moduleRoot -ProfilePath $script:profilePath -Uninstall

        (Test-Path -LiteralPath (Join-Path $script:moduleRoot 'Termyx')) | Should -BeFalse
        $profileText = Get-Content -LiteralPath $script:profilePath -Raw
        $profileText.Contains('Import-Module Termyx') | Should -BeFalse
    }
}
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run:

```powershell
Invoke-Pester -Path .\tests\Termyx.Tests.ps1 -Output Detailed
```

Expected: the install tests fail because `tools\Install.ps1` does not exist.

- [ ] **Step 3: Create the install script**

Create `tools/Install.ps1`:

```powershell
[CmdletBinding()]
param(
    [string]$ModuleSource = (Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx'),
    [string]$ModuleRoot = (Join-Path $env:USERPROFILE 'Documents\WindowsPowerShell\Modules'),
    [string]$ProfilePath = (Join-Path $env:USERPROFILE 'Documents\WindowsPowerShell\Microsoft.PowerShell_profile.ps1'),
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'

$importLine = 'if (Get-Module -ListAvailable Termyx) { Import-Module Termyx }'
$destination = Join-Path $ModuleRoot 'Termyx'

if ($Uninstall) {
    if (Test-Path -LiteralPath $destination) {
        Remove-Item -LiteralPath $destination -Recurse -Force
    }

    if (Test-Path -LiteralPath $ProfilePath) {
        $remaining = @(Get-Content -LiteralPath $ProfilePath | Where-Object { $_ -notmatch 'Import-Module\s+Termyx' })
        Set-Content -LiteralPath $ProfilePath -Value $remaining -Encoding UTF8
    }

    Write-Output ('Removed ' + $destination)
    return
}

if (-not (Test-Path -LiteralPath $ModuleSource)) {
    throw ('Module source not found: ' + $ModuleSource)
}

$manifest = Import-PowerShellDataFile -LiteralPath (Join-Path $ModuleSource 'Termyx.psd1')
$version = [string]$manifest.ModuleVersion
$target = Join-Path $destination $version

if (-not (Test-Path -LiteralPath $target)) {
    New-Item -ItemType Directory -Force -Path $target | Out-Null
}
Copy-Item -Path (Join-Path $ModuleSource '*') -Destination $target -Recurse -Force

$profileDirectory = Split-Path -Parent $ProfilePath
if ($profileDirectory -and -not (Test-Path -LiteralPath $profileDirectory)) {
    New-Item -ItemType Directory -Force -Path $profileDirectory | Out-Null
}

$existing = ''
if (Test-Path -LiteralPath $ProfilePath) {
    $existing = Get-Content -LiteralPath $ProfilePath -Raw
}

if ($existing -notmatch 'Import-Module\s+Termyx') {
    Add-Content -LiteralPath $ProfilePath -Value '' -Encoding UTF8
    Add-Content -LiteralPath $ProfilePath -Value $importLine -Encoding UTF8
}

Write-Output ('Installed Termyx ' + $version + ' to ' + $target)
```

- [ ] **Step 4: Create the README**

Create `README.md`:

```markdown
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
- PSReadLine 2.0 (installed with Windows PowerShell 5.1)
- No admin rights, no network access at runtime

PowerShell 7, VS Code, and the ISE are not supported in this version.

## Install

From the repository root:

```powershell
.\tools\Install.ps1
```

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

The Space handler never writes to the console, so a failure can only skip a
correction; plain Space always still works.

## Manual smoke checklist

Run this after installing, in both Windows Terminal and the legacy console host:

1. Type `Get-ChldItem` and press Space. The line becomes `Get-ChildItem `.
2. Press `Ctrl+Z` once. The line returns to `Get-ChldItem` exactly.
3. Type `git commit -m "helo wrld"` and press Space inside the quotes. Nothing changes.
4. Type `Write-Host helo` and press Space. It becomes `Write-Host hello `.
5. Type `Write-Host C:\temp\foo` and press Space. Nothing changes.
6. Type `Get-ChildItem -Reurse` and press Space. It becomes `-Recurse`.
7. Run `Disable-TermyxAutocorrect`, type a typo, press Space. Nothing changes.
8. Run `Enable-TermyxAutocorrect`. Corrections work again.

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
```

- [ ] **Step 5: Run the full suite**

Run:

```powershell
Invoke-Pester -Path .\tests -Output Detailed
```

Expected: 76 passed, 0 failed.

- [ ] **Step 6: Verify the module imports from a copied install**

Run:

```powershell
$sandbox = Join-Path $env:TEMP ('termyx-verify-' + [guid]::NewGuid().ToString('N'))
.\tools\Install.ps1 -ModuleRoot (Join-Path $sandbox 'Modules') -ProfilePath (Join-Path $sandbox 'profile.ps1')
Import-Module (Join-Path $sandbox 'Modules\Termyx\0.1.0\Termyx.psd1') -Force -ArgumentList @{ InstallHandler = $false }
Get-TermyxStatus | Format-List
Remove-Item -LiteralPath $sandbox -Recurse -Force
```

Expected: status shows `Enabled : True`, `WordCount` above 90000, `CommandCount` above 0, `InitError` empty.

- [ ] **Step 7: Commit**

```powershell
git add tools/Install.ps1 README.md tests/Termyx.Tests.ps1
git -c user.name="Termyx" -c user.email="dev@localhost" commit -m "feat: add installer, README and full verification suite"
```

---

## Manual Verification (Human Required)

Automated tests cannot simulate PSReadLine key dispatch. After Task 8, a human must:

1. Run `.\tools\Install.ps1` on the target machine.
2. Open a **new** Windows Terminal tab and a legacy `conhost` window.
3. Walk the README smoke checklist above in both windows, in order.
4. Confirm that typing is responsive while holding a key, and that a single `Ctrl+Z` reverts exactly one correction.
5. Confirm `Get-PSReadLineKeyHandler -Bound | Where-Object { $_.Key -eq 'Spacebar' }` reports the custom handler while enabled, and `SelfInsert` after `Disable-TermyxAutocorrect`.

If the handler never fires, check that the module actually imported (`Get-Module Termyx`) and that the profile line exists. The handler resolves module functions through `& $module { Invoke-TermyxSpaceKeyPress }`, so a missing module means space still works but no correction occurs.

## Self-Review

- **Spec coverage:** requirements 1-6 map to Tasks 2-7; sections 5-10 map to Tasks 1-7; section 11 performance is preserved by the distance caps and memoization; section 12 failure handling is implemented in Task 7; section 13 install is Task 8; section 14 testing is spread across every task; milestones M1-M4 map to Tasks 1-4, 5, 6-7, and 8.
- **Placeholders:** none. Every code step contains the code to write; every run step contains the command and expected result.
- **Type consistency:** `Get-TermyxTokenKind` returns `Kind`/`Command`/`IsFirstToken`/`Reason` everywhere; `Invoke-TermyxCorrection` returns the eight contract members; engine method names match the spec plus the two cache-clearing behaviors internal to the methods.
- **Known deviations** are listed at the top: Pester 5.7.1, the two-step DLL load, three added files, and canonical-return semantics instead of extra `IsKnown*` engine methods.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-11-11-termyx-autocorrect.md`.

Two execution options:

1. **Subagent-Driven (recommended)** - dispatch a fresh subagent per task, review between tasks, fast iteration. Requires `superpowers:subagent-driven-development`.
2. **Inline Execution** - execute tasks in this session with `superpowers:executing-plans`, batched with checkpoints.

Which approach?
