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
$script:TermyxUserWordsStamp = $null
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
. (Join-Path $script:TermyxRoot 'private\Prompt.ps1')

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

    try {
        Add-Type -Path $dll
    }
    catch {
        # A cached DLL can be truncated or otherwise invalid while its hash
        # sidecar still matches the source. Delete the whole cache and
        # recompile once; a failed recompile is rethrown to the caller.
        if (Test-Path -LiteralPath $dll) {
            Remove-Item -LiteralPath $dll -Force
        }
        if (Test-Path -LiteralPath $hashFile) {
            Remove-Item -LiteralPath $hashFile -Force
        }

        Add-Type -Path $source -OutputAssembly $dll
        Set-Content -LiteralPath $hashFile -Value $sourceHash -Encoding ASCII
        $script:TermyxEngineSource = 'compiled'

        # PowerShell caches a failed Add-Type load by path, so loading the
        # rebuilt file from the same path would fail again. Load its bytes
        # instead; the type becomes resolvable exactly like a path load.
        [void][System.Reflection.Assembly]::Load([System.IO.File]::ReadAllBytes($dll))
    }
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
    $script:TermyxLiveCommandCache = @{}
    $script:TermyxCommandCacheTime = [datetime]::UtcNow
}

function Enable-TermyxAutocorrect {
    [CmdletBinding()]
    param()

    if ($script:TermyxInitError) {
        throw ('Termyx cannot enable: ' + $script:TermyxInitError)
    }

    Import-Module PSReadLine -ErrorAction Stop
    Set-PSReadLineKeyHandler -Key Spacebar -ScriptBlock (Get-TermyxSpaceHandler)
    $script:TermyxLastCorrection = $null
    $script:TermyxEnabled = $true

    # The prompt marks are an enhancement, never a prerequisite: a prompt that
    # cannot be installed must not take autocorrect down with it.
    try {
        Enable-TermyxPrompt
    }
    catch {
        Write-Warning ('Termyx could not install the prompt marks: ' + $_.Exception.Message)
    }
}

function Disable-TermyxAutocorrect {
    [CmdletBinding()]
    param()

    try {
        if (Get-Command Set-PSReadLineKeyHandler -ErrorAction SilentlyContinue) {
            Set-PSReadLineKeyHandler -Key Spacebar -Function SelfInsert
        }
    }
    catch {
    }

    $script:TermyxEnabled = $false
    $script:TermyxLastCorrection = $null

    try {
        Disable-TermyxPrompt
    }
    catch {
        Write-Warning ('Termyx could not restore the prompt: ' + $_.Exception.Message)
    }
}

function OnRemove {
    # Called when the module leaves the session. PowerShell deletes the
    # module-bound wrapper function during removal, so the user's prompt has to
    # be put back before that cleanup runs.
    try {
        Disable-TermyxPrompt
    }
    catch {
    }
}

# Windows PowerShell 5.1 does not auto-bind a function that is merely named
# OnRemove; the handler has to be registered on the module object. A failure
# here must not take the import down with it.
try {
    $ExecutionContext.SessionState.Module.OnRemove = { OnRemove }
}
catch {
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
        $frequencyList = Join-Path $script:TermyxRoot 'assets\en_US.frequency.txt'
        [Termyx.Corrector]::LoadFrequencyFile($frequencyList)
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
                $script:TermyxEnabled = $false
                $script:TermyxInitError = 'Space handler not installed: ' + $_.Exception.Message
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
