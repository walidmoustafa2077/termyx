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

        if ($null -eq $text) {
            $text = Get-TermyxDefaultPromptText
        }
    }
    else {
        $text = Get-TermyxDefaultPromptText
    }

    $script:TermyxPromptText = $text
    return (Get-TermyxPromptWithMarks)
}

function Enable-TermyxPrompt {
    if ($script:TermyxPromptInstalled) {
        return
    }

    $existing = $null
    if (Test-Path Function:\global:prompt) {
        $existing = $function:global:prompt
    }

    # A forced re-import resets the module state while an earlier installer may
    # still be bound. Saving that wrapper as the "original" would leave the two
    # wrappers calling each other forever, so treat our own wrapper as absent.
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
            # LASTEXITCODE back exactly as it found it.
            if ($null -ne $savedExitCode) {
                $global:LASTEXITCODE = $savedExitCode
            }
        }
    }

    $script:TermyxInstalledPrompt = [scriptblock]$installed
    Set-Item -Path Function:\global:prompt -Value $installed
    $script:TermyxPromptInstalled = $true
}

function Disable-TermyxPrompt {
    if (-not $script:TermyxPromptInstalled) {
        return
    }

    $current = $null
    if (Test-Path Function:\global:prompt) {
        $current = $function:global:prompt
    }

    $isOurs = $false
    if ($null -ne $current -and $null -ne $script:TermyxInstalledPrompt) {
        $isOurs = ($current.ToString() -eq $script:TermyxInstalledPrompt.ToString())
    }

    # Only touch the prompt when the installed wrapper is still the active one;
    # a prompt the user created after enabling is theirs to keep.
    if ($isOurs) {
        if ($null -ne $script:TermyxOriginalPrompt) {
            Set-Item -Path Function:\global:prompt -Value $script:TermyxOriginalPrompt
        }
        elseif (Test-Path Function:\global:prompt) {
            Remove-Item -Path Function:prompt -Force
        }
    }

    $script:TermyxOriginalPrompt = $null
    $script:TermyxInstalledPrompt = $null
    $script:TermyxPromptInstalled = $false
}
