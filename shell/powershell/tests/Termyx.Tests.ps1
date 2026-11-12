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
        $script:committedFrequencyList = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx\assets\en_US.frequency.txt'
        [Termyx.Corrector]::LoadFrequencyFile($script:committedFrequencyList)
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

    It 'breaks an argument tie with the most frequent candidate' {
        $line = 'Write-Host teh'
        $r = Invoke-TermyxCommittedDecisionForTest -Line $line -Cursor $line.Length
        $r.ShouldApply | Should -BeTrue
        $r.Kind | Should -Be 'Argument'
        $r.NewLine | Should -Be 'Write-Host the'
        [Termyx.Corrector]::FrequencyCount | Should -Be 46717
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

Describe 'Word-wise undo' {
    It 'keeps undoing while a typed word is incomplete' {
        & (Get-Module Termyx) {
            Test-TermyxUndoComplete -Before 'if then' -After 'if the'
        } | Should -BeFalse

        & (Get-Module Termyx) {
            Test-TermyxUndoComplete -Before 'if t' -After 'if '
        } | Should -BeTrue
    }

    It 'stops when the line becomes empty' {
        & (Get-Module Termyx) {
            Test-TermyxUndoComplete -Before 'hello' -After ''
        } | Should -BeTrue
    }

    It 'stops after a pasted or corrected block in one step' {
        & (Get-Module Termyx) {
            Test-TermyxUndoComplete -Before 'echo hello world' -After ''
        } | Should -BeTrue

        & (Get-Module Termyx) {
            Test-TermyxUndoComplete -Before 'echo teh ' -After 'echo '
        } | Should -BeTrue
    }

    It 'stops when nothing changed' {
        & (Get-Module Termyx) {
            Test-TermyxUndoComplete -Before 'abc' -After 'abc'
        } | Should -BeTrue
    }

    It 'takes a trailing space by itself' {
        & (Get-Module Termyx) {
            Test-TermyxUndoComplete -Before 'echo ' -After 'echo'
        } | Should -BeTrue
    }

    It 'exposes undo and redo handlers that fall back to PSReadLine' {
        $undo = & (Get-Module Termyx) { Get-TermyxUndoHandler }
        $undo.ToString() | Should -Match 'Invoke-TermyxUndoKeyPress'
        $undo.ToString() | Should -Match 'PSConsoleReadLine\]::Undo'

        $redo = & (Get-Module Termyx) { Get-TermyxRedoHandler }
        $redo.ToString() | Should -Match 'Invoke-TermyxRedoKeyPress'
        $redo.ToString() | Should -Match 'PSConsoleReadLine\]::Redo'
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

    It 'resolves its own defaults when no paths are given' {
        # Windows PowerShell 5.1 leaves $PSScriptRoot empty while parameter
        # defaults bind, so the installer resolves its defaults inside the
        # script body. A child process with a sandboxed USERPROFILE proves the
        # real defaults without touching the real profile.
        $fakeHome = Join-Path $script:sandbox 'fakehome'
        New-Item -ItemType Directory -Force -Path $fakeHome | Out-Null

        $childScript = Join-Path $script:sandbox 'install-defaults.ps1'
        ('& ''' + $script:installer + '''') | Set-Content -LiteralPath $childScript -Encoding ASCII

        $startInfo = New-Object System.Diagnostics.ProcessStartInfo
        $startInfo.FileName = 'powershell.exe'
        $startInfo.Arguments = '-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' + $childScript + '"'
        $startInfo.UseShellExecute = $false
        $startInfo.RedirectStandardOutput = $true
        $startInfo.RedirectStandardError = $true
        $startInfo.EnvironmentVariables['USERPROFILE'] = $fakeHome

        $process = [System.Diagnostics.Process]::Start($startInfo)
        $process.StandardOutput.ReadToEnd() | Out-Null
        $stderr = $process.StandardError.ReadToEnd()
        $process.WaitForExit()

        $process.ExitCode | Should -Be 0
        $stderr | Should -BeNullOrEmpty

        $defaultRoot = Join-Path $fakeHome 'Documents\WindowsPowerShell\Modules\Termyx'
        (Test-Path -LiteralPath $defaultRoot) | Should -BeTrue
        @(Get-ChildItem -LiteralPath $defaultRoot -Directory).Count | Should -Be 1

        $defaultProfile = Join-Path $fakeHome 'Documents\WindowsPowerShell\Microsoft.PowerShell_profile.ps1'
        (Get-Content -LiteralPath $defaultProfile -Raw).Contains('Import-Module Termyx') | Should -BeTrue
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

Describe 'Frequency ranked suggestions' {
    BeforeEach {
        $script:freqWords = Join-Path $TestDrive 'freq-words.txt'
        Set-Content -LiteralPath $script:freqWords -Value @('hello', 'halo', 'hell', 'help', 'the', 'teh', 'are', 'ear', 'era')
        [Termyx.Corrector]::Initialize($script:freqWords, $null)
    }

    AfterEach {
        [Termyx.Corrector]::Initialize($script:freqWords, $null)
    }

    It 'picks the most frequent candidate when several are one edit away' {
        $ranks = Join-Path $TestDrive 'freq-ranks.txt'
        Set-Content -LiteralPath $ranks -Value @('the', 'hello', 'are', 'ear', 'era', 'halo', 'help', 'hell')
        [Termyx.Corrector]::LoadFrequencyFile($ranks)

        [Termyx.Corrector]::SuggestWord('helo', 1) | Should -Be 'hello'
    }

    It 'preserves the typed casing when a frequency winner is chosen' {
        $ranks = Join-Path $TestDrive 'freq-ranks.txt'
        Set-Content -LiteralPath $ranks -Value @('the', 'hello', 'are', 'ear', 'era', 'halo', 'help', 'hell')
        [Termyx.Corrector]::LoadFrequencyFile($ranks)

        [Termyx.Corrector]::SuggestWord('Helo', 1) | Should -Be 'Hello'
    }

    It 'leaves the tie alone when no candidate has a frequency rank' {
        [Termyx.Corrector]::LoadFrequencyFile('')

        [Termyx.Corrector]::SuggestWord('helo', 1) | Should -BeNullOrEmpty
        [Termyx.Corrector]::FrequencyCount | Should -Be 0
    }

    It 'keeps command ties strict even when both candidates are ranked' {
        [Termyx.Corrector]::SetCommands(@('alpha', 'alphi'))
        $ranks = Join-Path $TestDrive 'freq-ranks.txt'
        Set-Content -LiteralPath $ranks -Value @('alpha', 'alphi')
        [Termyx.Corrector]::LoadFrequencyFile($ranks)

        [Termyx.Corrector]::SuggestCommand('alphx', 1) | Should -BeNullOrEmpty
    }
}

Describe 'Spellcheck integration' {
    BeforeAll {
        $script:moduleManifest = Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx\Termyx.psd1'
        Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }
    }

    BeforeEach {
        # Reset the prompt bookkeeping without touching the prompt itself; an
        # earlier Describe may have left stale state behind.
        & (Get-Module Termyx) {
            $script:TermyxPromptInstalled = $false
            $script:TermyxOriginalPrompt = $null
        }
    }

    It 'wraps the prompt with OSC 133 marks when TERMYX is set' {
        $env:TERMYX = '1'
        $module = Get-Module Termyx
        $saved = $null
        try {
            $saved = & $module { $script:TermyxPromptText }
            $text = & $module { $script:TermyxPromptText = 'PS> '; Get-TermyxPromptWithMarks }

            $esc = [char]27
            $bel = [char]7
            ($text.Contains($esc + ']133;A' + $bel)) | Should -BeTrue
            ($text.Contains($esc + ']133;B' + $bel)) | Should -BeTrue
            ($text.StartsWith($esc + ']133;A' + $bel)) | Should -BeTrue
            ($text.Contains('PS> ' + $esc + ']133;B' + $bel)) | Should -BeTrue
            ($text.EndsWith($esc + ']133;B' + $bel)) | Should -BeTrue
        }
        finally {
            Remove-Item Env:TERMYX -ErrorAction SilentlyContinue
            & $module { param($value) $script:TermyxPromptText = $value } $saved
        }
    }

    It 'leaves the prompt untouched outside Termyx' {
        Remove-Item Env:TERMYX -ErrorAction SilentlyContinue
        $module = Get-Module Termyx
        $saved = $null
        try {
            $saved = & $module { $script:TermyxPromptText }
            $text = & $module { $script:TermyxPromptText = 'PS> '; Get-TermyxPromptWithMarks }

            $text | Should -Be 'PS> '
        }
        finally {
            & $module { param($value) $script:TermyxPromptText = $value } $saved
        }
    }

    It 'loads words added to the shared file' {
        $path = Join-Path $TestDrive 'user-words.txt'
        Set-Content -LiteralPath $path -Value 'zqxjv' -Encoding ASCII

        $module = Get-Module Termyx
        $savedPath = $null
        $savedStamp = $null
        try {
            $savedPath = & $module { $script:TermyxUserWordsPath }
            $savedStamp = & $module { $script:TermyxUserWordsStamp }
            & $module { param($p) $script:TermyxUserWordsPath = $p; $script:TermyxUserWordsStamp = $null } $path

            & $module { Update-TermyxUserWords }

            ([Termyx.Corrector]::IsKnownWord('zqxjv')) | Should -BeTrue
        }
        finally {
            & $module {
                param($p, $s)
                $script:TermyxUserWordsPath = $p
                $script:TermyxUserWordsStamp = $s
            } $savedPath $savedStamp
        }
    }

    It 'installs the prompt idempotently and restores the original' {
        $module = Get-Module Termyx
        $hadPrompt = Test-Path Function:\global:prompt
        $previous = $null
        if ($hadPrompt) { $previous = $function:prompt }

        try {
            Set-Item -Path Function:\global:prompt -Value { 'original> ' }

            & $module { Enable-TermyxPrompt }
            & $module { Enable-TermyxPrompt }

            (& $function:prompt) | Should -Be 'original> '
            (& $module { $script:TermyxPromptInstalled }) | Should -BeTrue

            & $module { Disable-TermyxPrompt }
            & $module { Disable-TermyxPrompt }

            (& $function:prompt) | Should -Be 'original> '
        }
        finally {
            & $module { Disable-TermyxPrompt }
            if ($hadPrompt) {
                Set-Item -Path Function:\global:prompt -Value $previous
            }
            elseif (Test-Path Function:\global:prompt) {
                Remove-Item -Path Function:prompt -Force
            }
        }
    }

    It 'does not synthesise stock text when the user prompt only writes to the host' {
        $env:TERMYX = '1'
        $module = Get-Module Termyx
        $savedOriginal = $null
        $savedText = $null
        try {
            $savedOriginal = & $module { $script:TermyxOriginalPrompt }
            $savedText = & $module { $script:TermyxPromptText }
            & $module { $script:TermyxOriginalPrompt = { Write-Host -NoNewline 'X> ' } }

            $text = & $module { Get-TermyxPromptText }

            $esc = [char]27
            $bel = [char]7
            ($text.Contains('PS ')) | Should -BeFalse
            ($text.StartsWith($esc + ']133;A' + $bel)) | Should -BeTrue
            ($text.EndsWith($esc + ']133;B' + $bel)) | Should -BeTrue
            ($text.Contains($esc + ']133;A' + $bel + $esc + ']133;B' + $bel)) | Should -BeTrue
        }
        finally {
            Remove-Item Env:TERMYX -ErrorAction SilentlyContinue
            & $module {
                param($original, $promptText)
                $script:TermyxOriginalPrompt = $original
                $script:TermyxPromptText = $promptText
            } $savedOriginal $savedText
        }
    }

    It 'uses the stock prompt when no prompt function exists' {
        $env:TERMYX = '1'
        $module = Get-Module Termyx
        $hadPrompt = Test-Path Function:\global:prompt
        $previous = $null
        if ($hadPrompt) { $previous = $function:prompt }
        $savedOriginal = $null
        $savedText = $null
        try {
            $savedOriginal = & $module { $script:TermyxOriginalPrompt }
            $savedText = & $module { $script:TermyxPromptText }
            if ($hadPrompt) {
                Remove-Item -Path Function:prompt -Force
            }
            & $module {
                $script:TermyxPromptInstalled = $false
                $script:TermyxOriginalPrompt = $null
            }
            & $module { Enable-TermyxPrompt }

            $text = & $function:prompt

            $esc = [char]27
            $bel = [char]7
            ($text.Contains($esc + ']133;A' + $bel + 'PS ')) | Should -BeTrue
            ($text.EndsWith($esc + ']133;B' + $bel)) | Should -BeTrue
        }
        finally {
            Remove-Item Env:TERMYX -ErrorAction SilentlyContinue
            & $module { Disable-TermyxPrompt }
            if ($hadPrompt) {
                Set-Item -Path Function:\global:prompt -Value $previous
            }
            elseif (Test-Path Function:\global:prompt) {
                Remove-Item -Path Function:prompt -Force
            }
            & $module {
                param($original, $promptText)
                $script:TermyxOriginalPrompt = $original
                $script:TermyxPromptText = $promptText
            } $savedOriginal $savedText
        }
    }

    It 'emits the marks from the installed global prompt' {
        $env:TERMYX = '1'
        $module = Get-Module Termyx
        $hadPrompt = Test-Path Function:\global:prompt
        $previous = $null
        if ($hadPrompt) { $previous = $function:prompt }
        $savedOriginal = $null
        $savedText = $null
        try {
            $savedOriginal = & $module { $script:TermyxOriginalPrompt }
            $savedText = & $module { $script:TermyxPromptText }
            Set-Item -Path Function:\global:prompt -Value { 'wired> ' }
            & $module {
                $script:TermyxPromptInstalled = $false
                $script:TermyxOriginalPrompt = $null
            }
            & $module { Enable-TermyxPrompt }

            $text = & $function:prompt

            $esc = [char]27
            $bel = [char]7
            ($text.Contains($esc + ']133;A' + $bel + 'wired> ' + $esc + ']133;B' + $bel)) | Should -BeTrue
        }
        finally {
            Remove-Item Env:TERMYX -ErrorAction SilentlyContinue
            & $module { Disable-TermyxPrompt }
            if ($hadPrompt) {
                Set-Item -Path Function:\global:prompt -Value $previous
            }
            elseif (Test-Path Function:\global:prompt) {
                Remove-Item -Path Function:prompt -Force
            }
            & $module {
                param($original, $promptText)
                $script:TermyxOriginalPrompt = $original
                $script:TermyxPromptText = $promptText
            } $savedOriginal $savedText
        }
    }

    It 'installs the prompt through enable and restores it through disable' {
        $module = Get-Module Termyx
        $hadPrompt = Test-Path Function:\global:prompt
        $previous = $null
        if ($hadPrompt) { $previous = $function:prompt }
        $savedOriginal = $null
        $savedText = $null
        try {
            $savedOriginal = & $module { $script:TermyxOriginalPrompt }
            $savedText = & $module { $script:TermyxPromptText }
            Set-Item -Path Function:\global:prompt -Value { 'wired> ' }

            Enable-TermyxAutocorrect

            (($function:prompt).ToString().Contains('TERMYX_PROMPT_WRAPPER')) | Should -BeTrue
            (& $function:prompt) | Should -Be 'wired> '

            Disable-TermyxAutocorrect

            (($function:prompt).ToString().Contains('TERMYX_PROMPT_WRAPPER')) | Should -BeFalse
            (& $function:prompt) | Should -Be 'wired> '
        }
        finally {
            Disable-TermyxAutocorrect
            & $module { Disable-TermyxPrompt }
            if ($hadPrompt) {
                Set-Item -Path Function:\global:prompt -Value $previous
            }
            elseif (Test-Path Function:\global:prompt) {
                Remove-Item -Path Function:prompt -Force
            }
            & $module {
                param($original, $promptText)
                $script:TermyxOriginalPrompt = $original
                $script:TermyxPromptText = $promptText
            } $savedOriginal $savedText
        }
    }

    It 'wraps a prompt that replaced the wrapper after enabling' {
        $module = Get-Module Termyx
        $hadPrompt = Test-Path Function:\global:prompt
        $previous = $null
        if ($hadPrompt) { $previous = $function:prompt }
        $savedOriginal = $null
        $savedText = $null
        try {
            $savedOriginal = & $module { $script:TermyxOriginalPrompt }
            $savedText = & $module { $script:TermyxPromptText }
            & $module {
                $script:TermyxPromptInstalled = $false
                $script:TermyxOriginalPrompt = $null
            }
            Set-Item -Path Function:\global:prompt -Value { 'first> ' }
            & $module { Enable-TermyxPrompt }
            (& $function:prompt) | Should -Be 'first> '

            Set-Item -Path Function:\global:prompt -Value { 'second> ' }
            & $module { Enable-TermyxPrompt }

            ((Get-Command prompt).ScriptBlock.ToString().Contains('TERMYX_PROMPT_WRAPPER')) | Should -BeTrue
            (& $function:prompt) | Should -Be 'second> '
        }
        finally {
            & $module { Disable-TermyxPrompt }
            if ($hadPrompt) {
                Set-Item -Path Function:\global:prompt -Value $previous
            }
            elseif (Test-Path Function:\global:prompt) {
                Remove-Item -Path Function:prompt -Force
            }
            & $module {
                param($original, $promptText)
                $script:TermyxOriginalPrompt = $original
                $script:TermyxPromptText = $promptText
            } $savedOriginal $savedText
        }
    }

    It 'keeps autocorrect enabled when the prompt cannot be installed' {
        $module = Get-Module Termyx
        $hadPrompt = Test-Path Function:\global:prompt
        $previous = $null
        if ($hadPrompt) { $previous = $function:prompt }
        $savedOriginal = $null
        $savedText = $null
        try {
            $savedOriginal = & $module { $script:TermyxOriginalPrompt }
            $savedText = & $module { $script:TermyxPromptText }
            & $module {
                $script:TermyxPromptInstalled = $false
                $script:TermyxOriginalPrompt = $null
                $script:TermyxInstalledPrompt = $null
            }
            Set-Item -Path Function:\global:prompt -Value { 'locked> ' }
            (Get-Item Function:\prompt).Options = [System.Management.Automation.ScopedItemOptions]::ReadOnly

            Enable-TermyxAutocorrect

            (Get-TermyxStatus).Enabled | Should -BeTrue
            (& $module { $script:TermyxPromptInstalled }) | Should -BeFalse
        }
        finally {
            Disable-TermyxAutocorrect
            if (Test-Path Function:\prompt) {
                (Get-Item Function:\prompt).Options = [System.Management.Automation.ScopedItemOptions]::None
            }
            & $module { Disable-TermyxPrompt }
            if ($hadPrompt) {
                Set-Item -Path Function:\global:prompt -Value $previous
            }
            elseif (Test-Path Function:\global:prompt) {
                Remove-Item -Path Function:prompt -Force
            }
            & $module {
                param($original, $promptText)
                $script:TermyxOriginalPrompt = $original
                $script:TermyxPromptText = $promptText
            } $savedOriginal $savedText
        }
    }

    It 'installs the prompt marks when autocorrect is disabled' {
        $hadPrompt = Test-Path Function:\global:prompt
        $previous = $null
        if ($hadPrompt) { $previous = $function:prompt }
        $oldTermyx = $env:TERMYX
        $oldAutocorrect = $env:TERMYX_AUTOCORRECT

        try {
            Set-Item -Path Function:\global:prompt -Value { 'disabled> ' }
            $env:TERMYX = '1'
            $env:TERMYX_AUTOCORRECT = '0'
            Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }

            $text = & $function:prompt
            $esc = [char]27
            $bel = [char]7
            ($text.Contains($esc + ']133;A' + $bel + 'disabled> ' + $esc + ']133;B' + $bel)) | Should -BeTrue
            (Get-TermyxStatus).Enabled | Should -BeFalse
            (Get-TermyxStatus).InitError | Should -Be 'Disabled by TERMYX_AUTOCORRECT=0'
        }
        finally {
            if ($null -eq $oldTermyx) { Remove-Item Env:TERMYX -ErrorAction SilentlyContinue }
            else { $env:TERMYX = $oldTermyx }
            if ($null -eq $oldAutocorrect) { Remove-Item Env:TERMYX_AUTOCORRECT -ErrorAction SilentlyContinue }
            else { $env:TERMYX_AUTOCORRECT = $oldAutocorrect }
            Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }
            if ($hadPrompt) {
                Set-Item -Path Function:\global:prompt -Value $previous
            }
            elseif (Test-Path Function:\global:prompt) {
                Remove-Item -Path Function:prompt -Force
            }
        }
    }

    It 'leaves the prompt byte-identical when autocorrect is disabled outside Termyx' {
        $hadPrompt = Test-Path Function:\global:prompt
        $previous = $null
        if ($hadPrompt) { $previous = $function:prompt }
        $oldTermyx = $env:TERMYX
        $oldAutocorrect = $env:TERMYX_AUTOCORRECT

        try {
            Set-Item -Path Function:\global:prompt -Value { 'plain> ' }
            Remove-Item Env:TERMYX -ErrorAction SilentlyContinue
            $env:TERMYX_AUTOCORRECT = '0'
            $before = (& $function:prompt)

            Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }

            (& $function:prompt) | Should -BeExactly $before
            (Get-TermyxStatus).Enabled | Should -BeFalse
        }
        finally {
            if ($null -eq $oldTermyx) { Remove-Item Env:TERMYX -ErrorAction SilentlyContinue }
            else { $env:TERMYX = $oldTermyx }
            if ($null -eq $oldAutocorrect) { Remove-Item Env:TERMYX_AUTOCORRECT -ErrorAction SilentlyContinue }
            else { $env:TERMYX_AUTOCORRECT = $oldAutocorrect }
            Import-Module $script:moduleManifest -Force -ArgumentList @{ InstallHandler = $false }
            if ($hadPrompt) {
                Set-Item -Path Function:\global:prompt -Value $previous
            }
            elseif (Test-Path Function:\global:prompt) {
                Remove-Item -Path Function:prompt -Force
            }
        }
    }

    It 'skips the shared file when its timestamp has not changed' {
        $path = Join-Path $TestDrive 'stamp-words.txt'
        Set-Content -LiteralPath $path -Value 'zqxjs' -Encoding ASCII
        $stamp = (Get-Item -LiteralPath $path).LastWriteTimeUtc

        $module = Get-Module Termyx
        $savedPath = $null
        $savedStamp = $null
        try {
            $savedPath = & $module { $script:TermyxUserWordsPath }
            $savedStamp = & $module { $script:TermyxUserWordsStamp }
            & $module { param($p) $script:TermyxUserWordsPath = $p; $script:TermyxUserWordsStamp = $null } $path

            & $module { Update-TermyxUserWords }
            ([Termyx.Corrector]::IsKnownWord('zqxjs')) | Should -BeTrue

            # Same content on disk, but the stamp is put back to the value the
            # module already saw: the reload must be skipped.
            Set-Content -LiteralPath $path -Value 'zqxjt' -Encoding ASCII
            (Get-Item -LiteralPath $path).LastWriteTimeUtc = $stamp
            & $module { Update-TermyxUserWords }

            ([Termyx.Corrector]::IsKnownWord('zqxjt')) | Should -BeFalse
        }
        finally {
            & $module {
                param($p, $s)
                $script:TermyxUserWordsPath = $p
                $script:TermyxUserWordsStamp = $s
            } $savedPath $savedStamp
        }
    }

    It 'does not throw when the shared file cannot be read' {
        $path = Join-Path $TestDrive 'locked-words.txt'
        Set-Content -LiteralPath $path -Value 'zqxju' -Encoding ASCII

        $module = Get-Module Termyx
        $handle = $null
        $savedPath = $null
        $savedStamp = $null
        try {
            $savedPath = & $module { $script:TermyxUserWordsPath }
            $savedStamp = & $module { $script:TermyxUserWordsStamp }
            & $module { param($p) $script:TermyxUserWordsPath = $p; $script:TermyxUserWordsStamp = $null } $path

            $handle = [System.IO.File]::Open(
                $path,
                [System.IO.FileMode]::Open,
                [System.IO.FileAccess]::Read,
                [System.IO.FileShare]::None)

            { & $module { Update-TermyxUserWords } } | Should -Not -Throw
            ([Termyx.Corrector]::IsKnownWord('zqxju')) | Should -BeFalse
        }
        finally {
            if ($null -ne $handle) { $handle.Dispose() }
            & $module {
                param($p, $s)
                $script:TermyxUserWordsPath = $p
                $script:TermyxUserWordsStamp = $s
            } $savedPath $savedStamp
        }
    }

    It 'restores a user prompt when the module is removed' {
        $childScript = Join-Path $TestDrive 'prompt-remove.ps1'
        $childLines = @(
            'function global:prompt { ''CUSTOM> '' }'
            ('Import-Module ''' + $script:moduleManifest + ''' -ArgumentList @{ InstallHandler = $false }')
            '$module = Get-Module Termyx'
            '& $module { Enable-TermyxPrompt }'
            'if (($function:prompt).ToString().Contains(''TERMYX_PROMPT_WRAPPER'')) { ''wrapped=True'' } else { ''wrapped=False'' }'
            '''rendered='' + (& $function:prompt)'
            'Remove-Module Termyx -Force'
            'if (Test-Path Function:\prompt) { ''exists=True'' } else { ''exists=False'' }'
            '''restored='' + (& $function:prompt)'
        )
        $childLines | Set-Content -LiteralPath $childScript -Encoding ASCII

        $output = @(& powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $childScript)
        $exitCode = $LASTEXITCODE
        $joined = $output -join "`n"

        $exitCode | Should -Be 0
        ($joined.Contains('wrapped=True')) | Should -BeTrue
        ($joined.Contains('exists=True')) | Should -BeTrue
        ($joined.Contains('restored=CUSTOM> ')) | Should -BeTrue
    }
}

Describe 'Undo and redo bindings' {
    BeforeAll {
        $script:moduleText = Get-Content -Raw (Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx\Termyx.psm1')
    }

    It 'binds the lowercase chord names PSReadLine dispatches' {
        # PSReadLine 2.0.0 dispatches Ctrl+Z as `Ctrl+z`, so a `Ctrl+Z` entry is
        # never consulted and the default single-step Undo keeps running.
        $script:moduleText | Should -Match '-Key Ctrl\+z -ScriptBlock \(Get-TermyxUndoHandler\)'
        $script:moduleText | Should -Match '-Key Ctrl\+y -ScriptBlock \(Get-TermyxRedoHandler\)'
        # Case-sensitive on purpose: `-match` would also accept `Ctrl+z`.
        ($script:moduleText -cmatch '-Key Ctrl\+Z') | Should -BeFalse
        ($script:moduleText -cmatch '-Key Ctrl\+Y') | Should -BeFalse
    }

    It 'restores the native handlers with the same chord names' {
        $script:moduleText | Should -Match '-Key Ctrl\+z -Function Undo'
        $script:moduleText | Should -Match '-Key Ctrl\+y -Function Redo'
    }
}