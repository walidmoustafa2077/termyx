# Termyx prompt integration.
#
# Termyx scans the shell output for OSC 133 "A" (prompt start) and "B" (input
# start) marks, and passes every other byte through unchanged. The marks are
# emitted only when the parent terminal sets TERMYX=1; on any other terminal
# they would print as stray characters.

$script:TermyxPromptText = ''
$script:TermyxPromptInstalled = $false
$script:TermyxOriginalPrompt = $null
$script:TermyxInstalledPrompt = $null

function Get-TermyxDefaultPromptText {
    # Reproduce the stock Windows PowerShell prompt, "PS <path>> ", for the
    # very common case where the user has no prompt function of their own. The
    # location comes from the session state instead of Get-Location so no
    # cmdlet runs between the caller's commands.
    $location = $ExecutionContext.SessionState.Path.CurrentLocation
    $level = $nestedPromptLevel + 1
    return ('PS ' + [string]$location + ('>' * $level) + ' ')
}

function Get-TermyxPromptWithMarks {
    if ($env:TERMYX -ne '1') {
        return $script:TermyxPromptText
    }

    $esc = [char]27
    $bel = [char]7
    return ($esc + ']133;A' + $bel) + $script:TermyxPromptText + ($esc + ']133;B' + $bel)
}

function Get-TermyxPromptText {
    if ($null -ne $script:TermyxOriginalPrompt) {
        $text = $null
        try {
            $text = & $script:TermyxOriginalPrompt
            if ($null -ne $text) {
                $text = [string]($text -join '')
            }
        }
        catch {
            $text = $null
        }

        # A prompt that only writes to the host, such as
        # "function prompt { Write-Host -NoNewline 'X> ' }", returns no text
        # because it already rendered its own. Synthesising the stock prompt
        # here would print a second prompt on every line, so the wrapped text
        # is empty. That is correct: A clears the recorded input start and B
        # sets it at the current cursor, so the two marks back to back record
        # exactly where typing begins.
        if ($null -eq $text) {
            $text = ''
        }
    }
    else {
        $text = Get-TermyxDefaultPromptText
    }

    $script:TermyxPromptText = $text
    return (Get-TermyxPromptWithMarks)
}

function Test-TermyxPromptInstalled {
    if (-not (Test-Path Function:\global:prompt)) {
        return $false
    }

    $current = $function:global:prompt
    if ($null -eq $current) {
        return $false
    }

    return $current.ToString().Contains('TERMYX_PROMPT_WRAPPER')
}

function Enable-TermyxPrompt {
    # Trust the cached flag only while the wrapper really is the active prompt.
    # A prompt replaced after enabling must be wrapped, not ignored.
    if ($script:TermyxPromptInstalled -and (Test-TermyxPromptInstalled)) {
        return
    }

    $script:TermyxPromptInstalled = $false
    $script:TermyxInstalledPrompt = $null

    $existing = $null
    if (Test-Path Function:\global:prompt) {
        $existing = $function:global:prompt
    }

    # Belt and braces for a hand-copied wrapper. PowerShell removes the
    # module-bound prompt function when the module is removed, and OnRemove
    # restores the user prompt first, so a forced re-import cannot recurse
    # through this branch; it only matters for a wrapper copied by hand.
    if ($null -ne $existing -and $existing.ToString().Contains('TERMYX_PROMPT_WRAPPER')) {
        $existing = $null
    }

    $script:TermyxOriginalPrompt = $existing

    $installed = {
        # TERMYX_PROMPT_WRAPPER
        $savedExitCode = $global:LASTEXITCODE
        try {
            $module = Get-Module Termyx
            if ($null -ne $module) {
                return (& $module { Get-TermyxPromptText })
            }

            $location = $ExecutionContext.SessionState.Path.CurrentLocation
            return ('PS ' + [string]$location + ('>' * ($nestedPromptLevel + 1)) + ' ')
        }
        catch {
            return 'PS> '
        }
        finally {
            # The prompt runs between the caller's commands, so it must hand
            # LASTEXITCODE back exactly as it found it, including when it was
            # never set and therefore was $null.
            $global:LASTEXITCODE = $savedExitCode
        }
    }

    Set-Item -Path Function:\global:prompt -Value $installed -ErrorAction Stop
    $script:TermyxInstalledPrompt = [scriptblock]$installed
    $script:TermyxPromptInstalled = $true
}

function Disable-TermyxPrompt {
    if (-not $script:TermyxPromptInstalled) {
        return
    }

    # Only touch the prompt when the installed wrapper is still the active one;
    # a prompt the user created after enabling is theirs to keep.
    if (Test-TermyxPromptInstalled) {
        if ($null -ne $script:TermyxOriginalPrompt) {
            Set-Item -Path Function:\global:prompt -Value $script:TermyxOriginalPrompt -ErrorAction SilentlyContinue
        }
        elseif (Test-Path Function:\global:prompt) {
            # The unscoped Function:prompt path is required here. In Windows
            # PowerShell 5.1, Remove-Item Function:\global:prompt silently
            # does nothing, while Test-Path and Set-Item accept the prefix.
            Remove-Item -Path Function:prompt -Force -ErrorAction SilentlyContinue
        }
    }

    $script:TermyxOriginalPrompt = $null
    $script:TermyxInstalledPrompt = $null
    $script:TermyxPromptInstalled = $false
}
