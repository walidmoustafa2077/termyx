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

function Write-TermyxVerboseLog {
    param([string]$Line, $Result)

    # Verbose logging is best effort. It must never throw into the key handler:
    # an exception here would abort before the handler reports that it consumed
    # the key, and the wrapper would append a second space.
    try {
        $entry = [datetime]::Now.ToString('s') + ' ' + $Result.Kind + ' ' + $Result.Reason + ': ' + $Line + ' -> ' + $Result.NewLine
        Add-Content -LiteralPath $script:TermyxLogPath -Value $entry -Encoding ASCII -ErrorAction Stop
    }
    catch {
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
        Write-TermyxVerboseLog -Line $line -Result $result
    }

    # The return value is decided before any logging concerns: the wrapper uses
    # it to know whether the keypress was already consumed.
    return $true
}

function Get-TermyxUndoHandler {
    return {
        param($key, $arg)

        $handled = $false

        try {
            $module = Get-Module Termyx
            if ($null -ne $module) {
                & $module { param($k, $a) Invoke-TermyxUndoKeyPress -Key $k -Arg $a } $key $arg
                $handled = $true
            }
        }
        catch {
        }

        # A removed or broken module must never leave Ctrl+Z dead.
        if (-not $handled) {
            [Microsoft.PowerShell.PSConsoleReadLine]::Undo($key, $arg)
        }
    }
}

function Get-TermyxRedoHandler {
    return {
        param($key, $arg)

        $handled = $false

        try {
            $module = Get-Module Termyx
            if ($null -ne $module) {
                & $module { param($k, $a) Invoke-TermyxRedoKeyPress -Key $k -Arg $a } $key $arg
                $handled = $true
            }
        }
        catch {
        }

        if (-not $handled) {
            [Microsoft.PowerShell.PSConsoleReadLine]::Redo($key, $arg)
        }
    }
}

function Test-TermyxUndoComplete {
    [CmdletBinding()]
    param(
        [string]$Before,
        [string]$After
    )

    $removed = $Before.Length - $After.Length

    # No movement means there is nothing left to undo. More than one character
    # means the step was a paste or a correction replacement, which PSReadLine
    # already records as a single editing unit: one undo is the whole block.
    if ($removed -le 0) { return $true }
    if ($removed -gt 1) { return $true }

    # The line is empty, so the word being removed was the only content.
    if ($After.Length -eq 0) { return $true }

    # The character just removed was whitespace; the word before it is whole.
    if ([char]::IsWhiteSpace($Before[$After.Length])) { return $true }

    # The line now ends on whitespace, so the word in front of it is whole.
    if ([char]::IsWhiteSpace($After[$After.Length - 1])) { return $true }

    return $false
}

function Invoke-TermyxUndoKeyPress {
    param($Key, $Arg)

    $line = $null
    $cursor = 0
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)

    $startLine = $line
    $atEnd = ($cursor -eq $line.Length)
    $steps = 0

    # PSReadLine undoes one edit item at a time, so typing costs one step per
    # character. Keep stepping until a whole word has come off; a paste or a
    # replacement comes off in one step and stops the loop by itself.
    while ($steps -lt 256) {
        $stepBefore = $line
        [Microsoft.PowerShell.PSConsoleReadLine]::Undo($Key, $Arg)
        $steps++

        $nextLine = $null
        $nextCursor = 0
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$nextLine, [ref]$nextCursor)

        if ($nextLine -ceq $line -and $nextCursor -eq $cursor) {
            break
        }

        $line = $nextLine
        $cursor = $nextCursor

        # Mid-line undo keeps the native one-step behavior: the word boundary
        # heuristic only applies while typing at the end of the line.
        if (-not $atEnd) { break }
        if (Test-TermyxUndoComplete -Before $stepBefore -After $line) { break }
    }

    if ($line -cne $startLine) {
        $script:TermyxUndo = @{
            Before = $startLine
            After  = $line
        }
    }
    else {
        $script:TermyxUndo = $null
    }
}

function Invoke-TermyxRedoKeyPress {
    param($Key, $Arg)

    $line = $null
    $cursor = 0
    [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)

    $state = $script:TermyxUndo

    # Only replay the exact word-wise undo that was just performed. Anything
    # else (a new edit, another undo, a moved cursor) uses the native redo.
    if ($null -eq $state -or $line -cne $state.After) {
        $script:TermyxUndo = $null
        [Microsoft.PowerShell.PSConsoleReadLine]::Redo($Key, $Arg)
        return
    }

    $steps = 0
    while ($steps -lt 256) {
        [Microsoft.PowerShell.PSConsoleReadLine]::Redo($Key, $Arg)
        $steps++

        $nextLine = $null
        $nextCursor = 0
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$nextLine, [ref]$nextCursor)

        if ($nextLine -ceq $line) {
            break
        }

        $line = $nextLine
        if ($line -ceq $state.Before) { break }
    }

    $script:TermyxUndo = $null
}
