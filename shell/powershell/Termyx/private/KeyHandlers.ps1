function Get-TermyxSpaceHandler {
    return {
        param($key, $arg)

        $inserted = $false

        try {
            $module = Get-Module Termyx
            if ($null -ne $module) {
                $inserted = & $module { Invoke-TermyxSpaceKeyPress }
            }
        }
        catch {
        }

        # A correction consumes the keypress: it already replaced the token
        # with the fixed word and its trailing space in a single edit item.
        # Otherwise forward the real key so PSReadLine's SelfInsert, which
        # returns immediately when key is null, actually writes the space
        # character. Exactly one insertion happens per keypress.
        if (-not $inserted) {
            if ($null -ne $key) {
                [Microsoft.PowerShell.PSConsoleReadLine]::SelfInsert($key, $arg)
            }
            else {
                [Microsoft.PowerShell.PSConsoleReadLine]::Insert(' ')
            }
        }
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

    if (-not $result.ShouldApply) { return $false }

    $last = $script:TermyxLastCorrection
    if ($null -ne $last -and
        $last.Line -eq $line -and
        $last.Start -eq $result.Start -and
        $last.Fix -eq $result.Fix) {
        return $false
    }

    # Write the corrected word and its trailing space as one edit so that a
    # single Ctrl+Z removes the whole correction instead of only the space.
    [Microsoft.PowerShell.PSConsoleReadLine]::Replace($result.Start, $result.Length, ($result.Fix + ' '))
    [Microsoft.PowerShell.PSConsoleReadLine]::SetCursorPosition($result.Start + $result.Fix.Length + 1)
    $script:TermyxLastCorrection = @{
        Line  = $line
        Start = $result.Start
        Fix   = $result.Fix
    }

    if ($script:TermyxOptions.VerboseCorrections) {
        $entry = [datetime]::Now.ToString('s') + ' ' + $result.Kind + ' ' + $result.Reason + ': ' + $line + ' -> ' + $result.NewLine
        Add-Content -LiteralPath $script:TermyxLogPath -Value $entry -Encoding ASCII
    }

    return $true
}
