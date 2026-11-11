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
        $null -ne $script:TermyxExceptions -and
        ($now - $script:TermyxExceptionsTime).TotalSeconds -lt 5) {
        return ,$script:TermyxExceptions
    }

    $set = New-Object 'System.Collections.Generic.HashSet[string]' -ArgumentList @([StringComparer]::OrdinalIgnoreCase)

    if ($script:TermyxExceptionsPath -and (Test-Path -LiteralPath $script:TermyxExceptionsPath)) {
        # A locked or unreadable file raises a non-terminating error that PS 5.1
        # does not route through try/catch. Silence it at the source, and keep the
        # try/catch as a second guard so the handler can never write to the console.
        try {
            foreach ($line in @(Get-Content -LiteralPath $script:TermyxExceptionsPath -ErrorAction SilentlyContinue)) {
                $entry = $line.Trim()
                if ($entry.Length -gt 0 -and -not $entry.StartsWith('#')) {
                    [void]$set.Add($entry)
                }
            }
        }
        catch {
        }
    }

    foreach ($word in @($script:TermyxOptions.SkipWords)) {
        if ($word) { [void]$set.Add([string]$word) }
    }

    $script:TermyxExceptions = $set
    $script:TermyxExceptionsTime = $now
    return ,$set
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

    if ($Cursor -lt 0) {
        $result.Reason = 'BadCursor'
        return $result
    }

    if ($Cursor -gt $Line.Length) {
        $Cursor = $Line.Length
        $result.NewCursor = $Cursor
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
        $blocked = [char[]]@('\', '/', ':', '.', '$', '@')
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
        if ($kindInfo.Reason -eq 'PartialToken' -and $token -match '[-_]') {
            $result.Reason = 'Compound'
            return $result
        }
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

    # A token that already resolves to a real command, alias, or function is
    # never rewritten. This applies to every kind, including arguments such as
    # "Start-Process dotnet", where the command snapshot does not contain
    # external applications. The guard only ever skips; it never supplies a fix,
    # so a parameter token can never be replaced by a command name.
    if ($fix) {
        if ($null -eq $script:TermyxLiveCommandCache) {
            $script:TermyxLiveCommandCache = @{}
        }

        $liveKey = $token.ToLowerInvariant()
        if (-not $script:TermyxLiveCommandCache.ContainsKey($liveKey)) {
            $isLiveCommand = $false
            try {
                if (Get-Command -Name $token -ErrorAction SilentlyContinue) {
                    $isLiveCommand = $true
                }
            }
            catch {
                $isLiveCommand = $false
            }
            $script:TermyxLiveCommandCache[$liveKey] = $isLiveCommand
        }

        if ($script:TermyxLiveCommandCache[$liveKey]) {
            $result.Reason = 'Known'
            return $result
        }
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
