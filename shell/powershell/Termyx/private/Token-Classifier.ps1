function New-TermyxKindResult {
    param(
        [string]$Kind,
        $Command = $null,
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
