function Add-TermyxWordForm {
    param($Words, [string]$Word)
    if ($null -ne $Word -and $Word.Length -ge 1 -and $Word -cmatch '^[a-z]+$') {
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
        if ($flags.Contains('!')) { continue }
        Add-TermyxWordForm -Words $words -Word $stem.ToLowerInvariant()

        if ($flags.Length -eq 0) { continue }

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
