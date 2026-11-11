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

    It 'survives a forced re-import with the engine and commands intact' {
        Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }

        $expected = @(
            'Add-TermyxWord',
            'Disable-TermyxAutocorrect',
            'Enable-TermyxAutocorrect',
            'Get-TermyxStatus',
            'Set-TermyxOption',
            'Update-TermyxCommandCache'
        )
        $exported = @((Get-Module Termyx).ExportedFunctions.Keys)
        foreach ($name in $expected) {
            ($exported -contains $name) | Should -BeTrue
        }
        [Termyx.Corrector]::WordCount | Should -BeGreaterThan 0

        # The already-loaded type short circuit must be taken on re-import; without
        # it the module would fall through to the cached-DLL path on every import.
        (Get-TermyxStatus).EngineSource | Should -Be 'loaded'
    }
}

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

    It 'excludes stems flagged with NOSUGGEST' {
        $aff = @(
            'SFX S Y 1'
            'SFX S 0 s .'
        )
        $dic = @('1', 'badword/S!')
        $words = Expand-HunspellDictionary -DicLines $dic -AffLines $aff
        ($words -contains 'badword') | Should -BeFalse
        ($words -contains 'badwords') | Should -BeFalse
        $words.Count | Should -Be 0
    }
}

Describe 'en_US word list artifact' {
    BeforeAll {
        $script:wordListPath = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx\assets\en_US.wordlist.txt'
        $script:wordLines = Get-Content -LiteralPath $script:wordListPath
    }

    It 'contains at least 75000 words' {
        $script:wordLines.Count | Should -BeGreaterThan 75000
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

Describe 'Token classifier' {
    BeforeAll {
        function Get-TermyxTokenKindForTest {
            param([string]$Line, [int]$Start, [int]$Length)
            return & (Get-Module Termyx) {
                param($l, $s, $n)
                Get-TermyxTokenKind -Line $l -Start $s -Length $n
            } $Line $Start $Length
        }

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

    It 'returns a null owning command for command and argument tokens' {
        $commandToken = Get-TermyxTokenKindForTest -Line 'Get-ChldItem' -Start 0 -Length 12
        $commandToken.Kind | Should -Be 'Command'
        $commandToken.Command | Should -Be $null

        $argumentToken = Get-TermyxTokenKindForTest -Line 'Write-Host hello' -Start 11 -Length 5
        $argumentToken.Kind | Should -Be 'Argument'
        $argumentToken.Command | Should -Be $null
    }

    It 'returns a null owning command for a parameter of an expression command' {
        $r = Get-TermyxTokenKindForTest -Line '& $cmd -x' -Start 8 -Length 1
        $r.Kind | Should -Be 'Parameter'
        $r.Command | Should -Be $null
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

Describe 'Decision function' {
    BeforeAll {
        function Invoke-TermyxDecisionForTest {
            param([string]$Line, [int]$Cursor)
            return & (Get-Module Termyx) {
                param($l, $c)
                Invoke-TermyxCorrection -Line $l -Cursor $c
            } $Line $Cursor
        }

        $script:moduleManifest = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx\Termyx.psd1'
        Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }

        $script:decisionFixture = Join-Path $env:TEMP ('termyx-decision-' + [guid]::NewGuid().ToString('N') + '.txt')
        @(
            'the', 'receive', 'running', 'cat', 'cot', 'hello', 'world',
            'testing', 'powershell', 'file', 'path', 'status', 'commit', 'more',
            'botnet'
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
            $script:TermyxLiveCommandCache = @{}
        } $script:exceptionsFixture
    }

    It 'corrects a first-token command typo' {
        $r = Invoke-TermyxDecisionForTest -Line 'Get-ChldItem' -Cursor 12
        $r.ShouldApply | Should -BeTrue
        $r.Kind | Should -Be 'Command'
        $r.NewLine | Should -Be 'Get-ChildItem'
        $r.NewCursor | Should -Be 13
    }

    It 'protects a live command absent from the command snapshot' {
        function global:Get-ChildItm { 'live' }
        try {
            [Termyx.Corrector]::SetCommands(@('Get-ChildItem'))
            $r = Invoke-TermyxDecisionForTest -Line 'Get-ChildItm' -Cursor 12
            $r.ShouldApply | Should -BeFalse
            $r.Reason | Should -Be 'Known'
        }
        finally {
            Remove-Item -LiteralPath 'function:global:Get-ChildItm' -ErrorAction SilentlyContinue
            [Termyx.Corrector]::SetCommands(@('Get-ChildItem', 'Get-Content', 'Get-Item', 'Write-Host', 'Set-Location'))
        }
    }

    It 'protects an argument that is a live command' {
        # The fixture contains "botnet", so dotnet would otherwise be a unique
        # one-edit fix. The live-command guard must run for Argument tokens too,
        # not only for Command tokens.
        function global:dotnet { }
        try {
            $r = Invoke-TermyxDecisionForTest -Line 'Write-Host dotnet' -Cursor 17
            $r.ShouldApply | Should -BeFalse
            $r.Reason | Should -Be 'Known'
            $r.NewLine | Should -Be 'Write-Host dotnet'
        }
        finally {
            Remove-Item -LiteralPath 'function:global:dotnet' -ErrorAction SilentlyContinue
        }
    }

    It 'still corrects a genuine argument typo with the guard active' {
        $r = Invoke-TermyxDecisionForTest -Line 'Write-Host helllo' -Cursor 17
        $r.ShouldApply | Should -BeTrue
        $r.Kind | Should -Be 'Argument'
        $r.NewLine | Should -Be 'Write-Host hello'
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

    It 'never emits an error when the exceptions file cannot be read' {
        $locked = Join-Path $env:TEMP ('termyx-locked-' + [guid]::NewGuid().ToString('N') + '.txt')
        Set-Content -LiteralPath $locked -Value 'helo' -Encoding ASCII
        $handle = [System.IO.File]::Open($locked, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::None)
        try {
            & (Get-Module Termyx) {
                param($path)
                $script:TermyxExceptionsPath = $path
                $script:TermyxExceptions = $null
                $script:TermyxExceptionsTime = $null
            } $locked

            # Redirect the error stream: an unreadable file must contribute no
            # error output and no exceptions, and the correction still applies.
            $captured = @(
                & (Get-Module Termyx) {
                    Invoke-TermyxCorrection -Line 'Write-Host helllo' -Cursor 17
                } 2>&1
            )

            $errors = @($captured | Where-Object { $_ -is [System.Management.Automation.ErrorRecord] })
            $errors.Count | Should -Be 0

            $result = @($captured | Where-Object { $_ -isnot [System.Management.Automation.ErrorRecord] })[0]
            $result.ShouldApply | Should -BeTrue
            $result.NewLine | Should -Be 'Write-Host hello'
        }
        finally {
            $handle.Close()
            Remove-Item -LiteralPath $locked -ErrorAction SilentlyContinue
        }
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

Describe 'Committed dictionary behavior' {
    BeforeAll {
        $script:moduleManifest = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx\Termyx.psd1'
        Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }

        $script:committedWordList = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx\assets\en_US.wordlist.txt'
        [Termyx.Corrector]::Initialize($script:committedWordList, $null)
        [Termyx.Corrector]::SetCommands(@('Get-ChildItem', 'Write-Host', 'Set-Location'))

        & (Get-Module Termyx) {
            $script:TermyxEnabled = $true
            $script:TermyxOptions = @{
                MaxArgumentDistance = 2
                SkipWords           = @()
                VerboseCorrections  = $false
            }
            $script:TermyxExceptionsPath = Join-Path $env:TEMP 'termyx-missing-exceptions.txt'
            $script:TermyxExceptions = $null
            $script:TermyxExceptionsTime = $null
            $script:TermyxParameterNameCache = @{}
            $script:TermyxLiveCommandCache = @{}
        }

        function Invoke-TermyxCommittedDecisionForTest {
            param([string]$Line, [int]$Cursor)
            return & (Get-Module Termyx) {
                param($l, $c)
                Invoke-TermyxCorrection -Line $l -Cursor $c
            } $Line $Cursor
        }
    }

    It 'applies a documented correction against the committed word list' {
        $line = 'Write-Host helllo'
        $r = Invoke-TermyxCommittedDecisionForTest -Line $line -Cursor $line.Length
        $r.ShouldApply | Should -BeTrue
        $r.Kind | Should -Be 'Argument'
        $r.NewLine | Should -Be 'Write-Host hello'
        [Termyx.Corrector]::WordCount | Should -Be 87928
    }

    It 'leaves a documented tie alone against the committed word list' {
        $line = 'Write-Host helo'
        $r = Invoke-TermyxCommittedDecisionForTest -Line $line -Cursor $line.Length
        $r.ShouldApply | Should -BeFalse
        $r.Reason | Should -Be 'NoCandidate'
        $r.NewLine | Should -Be $line
    }
}

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

    It 'forwards the pressed key to the space insertion' {
        $handler = & (Get-Module Termyx) { Get-TermyxSpaceHandler }
        $body = $handler.ToString()
        $body | Should -Match 'SelfInsert\(\s*\$key'
        $body | Should -Not -Match 'SelfInsert\(\s*\)'
    }

    It 'cannot let a verbose log failure change the handler result' {
        $handlerBody = (& (Get-Module Termyx) { ${function:Invoke-TermyxSpaceKeyPress} }).ToString()
        $logCall = $handlerBody.IndexOf('Write-TermyxVerboseLog')
        $returnTrue = $handlerBody.LastIndexOf('return $true')

        # The log call must happen before the unconditional success return, and
        # the helper itself must swallow write failures.
        ($logCall -ge 0) | Should -BeTrue
        ($returnTrue -gt $logCall) | Should -BeTrue

        $logBody = (& (Get-Module Termyx) { ${function:Write-TermyxVerboseLog} }).ToString()
        $logBody | Should -Match 'Add-Content'
        $logBody | Should -Match 'try'
        $logBody | Should -Match 'catch'

        $originalLogPath = & (Get-Module Termyx) { $script:TermyxLogPath }
        try {
            & (Get-Module Termyx) { $script:TermyxLogPath = $env:TEMP }
            {
                & (Get-Module Termyx) {
                    Write-TermyxVerboseLog -Line 'x' -Result ([pscustomobject]@{
                        Kind = 'Argument'; Reason = 'Corrected'; NewLine = 'y'
                    })
                }
            } | Should -Not -Throw
        }
        finally {
            & (Get-Module Termyx) {
                param($path)
                $script:TermyxLogPath = $path
            } $originalLogPath
        }
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

    It 'uninstalls the module and only its own profile line' {
        # Prepend a cp1252 encoded line so a rewrite that re-encodes or
        # truncates the profile shows up as corruption.
        $legacy = [byte[]](0x23, 0x20, 0x63, 0x61, 0x66, 0xE9, 0x0D, 0x0A)
        $existing = [System.IO.File]::ReadAllBytes($script:profilePath)
        $before = New-Object byte[] ($legacy.Length + $existing.Length)
        [Array]::Copy($legacy, 0, $before, 0, $legacy.Length)
        [Array]::Copy($existing, 0, $before, $legacy.Length, $existing.Length)
        [System.IO.File]::WriteAllBytes($script:profilePath, $before)

        & $script:installer -ModuleSource $script:moduleSource -ModuleRoot $script:moduleRoot -ProfilePath $script:profilePath -Uninstall

        (Test-Path -LiteralPath (Join-Path $script:moduleRoot 'Termyx')) | Should -BeFalse
        $after = [System.IO.File]::ReadAllBytes($script:profilePath)
        $afterText = [System.Text.Encoding]::ASCII.GetString($after)
        $afterText.Contains('Import-Module Termyx') | Should -BeFalse
        $afterText.Contains('# caf') | Should -BeTrue
        ($after -contains 0xE9) | Should -BeTrue
        $after[0] | Should -Be 0x23
    }

    It 'ignores a commented out or similarly named import line' {
        $guardPath = Join-Path $script:sandbox 'guarded.ps1'
        Set-Content -LiteralPath $guardPath -Value '# Import-Module Termyx' -Encoding ASCII
        Add-Content -LiteralPath $guardPath -Value 'Import-Module TermyxHelper' -Encoding ASCII

        & $script:installer -ModuleSource $script:moduleSource -ModuleRoot $script:moduleRoot -ProfilePath $guardPath

        $lines = @(Get-Content -LiteralPath $guardPath)
        ($lines -contains '# Import-Module Termyx') | Should -BeTrue
        ($lines -contains 'Import-Module TermyxHelper') | Should -BeTrue
        ($lines -contains 'if (Get-Module -ListAvailable Termyx) { Import-Module Termyx }') | Should -BeTrue

        & $script:installer -ModuleSource $script:moduleSource -ModuleRoot $script:moduleRoot -ProfilePath $guardPath -Uninstall

        $remaining = @(Get-Content -LiteralPath $guardPath)
        ($remaining -contains '# Import-Module Termyx') | Should -BeTrue
        ($remaining -contains 'Import-Module TermyxHelper') | Should -BeTrue
        ($remaining -contains 'if (Get-Module -ListAvailable Termyx) { Import-Module Termyx }') | Should -BeFalse
    }

    It 'round-trips a UTF-16LE profile with a trailing newline without a blank line' {
        $nl = [Environment]::NewLine
        $importLine = 'if (Get-Module -ListAvailable Termyx) { Import-Module Termyx }'
        $wideProfile = Join-Path $script:sandbox 'wide-profile.ps1'
        $wide = New-Object System.Text.UnicodeEncoding -ArgumentList $false, $true
        [System.IO.File]::WriteAllText($wideProfile, ('# wide profile' + $nl), $wide)
        $before = [System.IO.File]::ReadAllBytes($wideProfile)

        & $script:installer -ModuleSource $script:moduleSource -ModuleRoot $script:moduleRoot -ProfilePath $wideProfile

        $installedText = [System.IO.File]::ReadAllText($wideProfile, $wide)
        $installedText | Should -Be ('# wide profile' + $nl + $importLine + $nl)

        & $script:installer -ModuleSource $script:moduleSource -ModuleRoot $script:moduleRoot -ProfilePath $wideProfile -Uninstall

        $after = [System.IO.File]::ReadAllBytes($wideProfile)
        [System.Convert]::ToBase64String($after) | Should -Be ([System.Convert]::ToBase64String($before))
        $after[0] | Should -Be 0xFF
        $after[1] | Should -Be 0xFE
    }

    It 'round-trips an ASCII profile without a trailing newline' {
        $nl = [Environment]::NewLine
        $importLine = 'if (Get-Module -ListAvailable Termyx) { Import-Module Termyx }'
        $narrowProfile = Join-Path $script:sandbox 'narrow-profile.ps1'
        $beforeText = '# no trailing newline'
        [System.IO.File]::WriteAllBytes($narrowProfile, [System.Text.Encoding]::ASCII.GetBytes($beforeText))
        $before = [System.IO.File]::ReadAllBytes($narrowProfile)

        & $script:installer -ModuleSource $script:moduleSource -ModuleRoot $script:moduleRoot -ProfilePath $narrowProfile

        $installedText = [System.IO.File]::ReadAllText($narrowProfile, [System.Text.Encoding]::ASCII)
        $installedText | Should -Be ($beforeText + $nl + $importLine)

        & $script:installer -ModuleSource $script:moduleSource -ModuleRoot $script:moduleRoot -ProfilePath $narrowProfile -Uninstall

        $after = [System.IO.File]::ReadAllBytes($narrowProfile)
        [System.Convert]::ToBase64String($after) | Should -Be ([System.Convert]::ToBase64String($before))
    }
}

Describe 'Engine cache recovery' {
    BeforeAll {
        $script:sandbox = Join-Path $env:TEMP ('termyx-cache-' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Force -Path $script:sandbox | Out-Null

        $script:moduleSource = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx'
        $script:moduleManifest = Join-Path $script:moduleSource 'Termyx.psd1'
    }

    AfterAll {
        Remove-Item -LiteralPath $script:sandbox -Recurse -Force -ErrorAction SilentlyContinue
    }

    It 'deletes and recompiles a corrupt cached DLL whose hash still matches' {
        # The cache directory is chosen from LOCALAPPDATA at import time, so a
        # child process with a sandboxed LOCALAPPDATA gives a clean cache
        # without touching the user's real one.
        $cacheDir = Join-Path $script:sandbox 'Termyx'
        New-Item -ItemType Directory -Force -Path $cacheDir | Out-Null

        $source = Join-Path $script:moduleSource 'lib\Corrector.cs'
        $sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
        Set-Content -LiteralPath (Join-Path $cacheDir 'Corrector.sha256') -Value $sourceHash -Encoding ASCII
        [System.IO.File]::WriteAllText((Join-Path $cacheDir 'Corrector.dll'), 'not a real DLL')

        $childScript = Join-Path $script:sandbox 'child.ps1'
        $childLines = @(
            ('Import-Module ''' + $script:moduleManifest + ''' -Force -ArgumentList @{ InstallHandler = $false }')
            '$s = Get-TermyxStatus'
            '"source=$($s.EngineSource)"'
            '"words=$($s.WordCount)"'
            '"error=$($s.InitError)"'
        )
        $childLines | Set-Content -LiteralPath $childScript -Encoding ASCII

        $startInfo = New-Object System.Diagnostics.ProcessStartInfo
        $startInfo.FileName = 'powershell.exe'
        $startInfo.Arguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $childScript + '"'
        $startInfo.UseShellExecute = $false
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $startInfo.EnvironmentVariables['LOCALAPPDATA'] = $script:sandbox

        $process = [System.Diagnostics.Process]::Start($startInfo)
        $stdout = $process.StandardOutput.ReadToEnd()
        $stderr = $process.StandardError.ReadToEnd()
        $process.WaitForExit()

        $process.ExitCode | Should -Be 0
        $stdout | Should -Match 'source=compiled'
        $stdout | Should -Match 'words=87928'
        $stdout | Should -Match 'error='
        $stderr | Should -BeNullOrEmpty
    }
}
