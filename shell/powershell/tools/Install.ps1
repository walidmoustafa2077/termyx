[CmdletBinding()]
param(
    [string]$ModuleSource = (Join-Path (Split-Path -Parent $PSScriptRoot) 'Termyx'),
    [string]$ModuleRoot = (Join-Path $env:USERPROFILE 'Documents\WindowsPowerShell\Modules'),
    [string]$ProfilePath = (Join-Path $env:USERPROFILE 'Documents\WindowsPowerShell\Microsoft.PowerShell_profile.ps1'),
    [switch]$Uninstall
)

$ErrorActionPreference = 'Stop'

$importLine = 'if (Get-Module -ListAvailable Termyx) { Import-Module Termyx }'
$destination = Join-Path $ModuleRoot 'Termyx'

# Returns the UTF-16 encoding when the profile is UTF-16 with a BOM, otherwise
# $null. A $null result means the profile is edited byte-for-byte, so its
# existing encoding, line endings and content survive untouched.
function Get-TermyxProfileWideEncoding {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return $null
    }

    $bytes = [System.IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -ge 2 -and $bytes[0] -eq 0xFF -and $bytes[1] -eq 0xFE) {
        return (New-Object System.Text.UnicodeEncoding -ArgumentList $false, $true)
    }
    if ($bytes.Length -ge 2 -and $bytes[0] -eq 0xFE -and $bytes[1] -eq 0xFF) {
        return (New-Object System.Text.UnicodeEncoding -ArgumentList $true, $true)
    }

    return $null
}

function Test-TermyxImportLine {
    param([string]$Text)

    # Anchored to the whole line so a commented-out copy or a similarly named
    # module (TermyxHelper) is never mistaken for our import.
    return ($Text.Trim() -ieq $importLine)
}

function Test-TermyxProfileImportLine {
    param([string]$Path)

    if (-not (Test-Path -LiteralPath $Path)) {
        return $false
    }

    $wide = Get-TermyxProfileWideEncoding -Path $Path
    if ($null -ne $wide) {
        foreach ($line in [System.IO.File]::ReadAllLines($Path, $wide)) {
            if (Test-TermyxImportLine -Text $line) { return $true }
        }
        return $false
    }

    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $start = 0
    for ($i = 0; $i -le $bytes.Length; $i++) {
        if ($i -eq $bytes.Length -or $bytes[$i] -eq 10) {
            $end = $i
            if ($end -gt $start -and $bytes[$end - 1] -eq 13) { $end-- }
            if ($end -gt $start) {
                $line = [System.Text.Encoding]::ASCII.GetString($bytes, $start, $end - $start)
                if (Test-TermyxImportLine -Text $line) { return $true }
            }
            $start = $i + 1
        }
    }

    return $false
}

function Remove-TermyxProfileImportLine {
    param([string]$Path)

    $wide = Get-TermyxProfileWideEncoding -Path $Path
    if ($null -ne $wide) {
        $lines = [System.IO.File]::ReadAllLines($Path, $wide)
        $remaining = @($lines | Where-Object { -not (Test-TermyxImportLine -Text $_) })
        if ($remaining.Count -eq $lines.Count) {
            return $false
        }
        [System.IO.File]::WriteAllLines($Path, [string[]]$remaining, $wide)
        return $true
    }

    # Byte-for-byte removal: the matched line and its line terminator are
    # dropped, every other byte is copied through unchanged.
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $keep = New-Object System.Collections.Generic.List[byte]
    $removed = $false
    $start = 0
    for ($i = 0; $i -le $bytes.Length; $i++) {
        if ($i -eq $bytes.Length -or $bytes[$i] -eq 10) {
            $lineEnd = $i
            $contentEnd = $lineEnd
            if ($contentEnd -gt $start -and $bytes[$contentEnd - 1] -eq 13) { $contentEnd-- }

            $isTarget = $false
            if ($contentEnd -gt $start) {
                $line = [System.Text.Encoding]::ASCII.GetString($bytes, $start, $contentEnd - $start)
                if (Test-TermyxImportLine -Text $line) {
                    $isTarget = $true
                    $removed = $true
                }
            }

            if (-not $isTarget) {
                $stop = $i
                if ($i -lt $bytes.Length) { $stop = $i + 1 }
                for ($j = $start; $j -lt $stop; $j++) { $keep.Add($bytes[$j]) }
            }
            $start = $i + 1
        }
    }

    if ($removed) {
        [System.IO.File]::WriteAllBytes($Path, $keep.ToArray())
    }

    return $removed
}

if ($Uninstall) {
    if (Test-Path -LiteralPath $destination) {
        Remove-Item -LiteralPath $destination -Recurse -Force
    }

    if (Test-Path -LiteralPath $ProfilePath) {
        Remove-TermyxProfileImportLine -Path $ProfilePath | Out-Null
    }

    Write-Output ('Removed ' + $destination)
    return
}

if (-not (Test-Path -LiteralPath $ModuleSource)) {
    throw ('Module source not found: ' + $ModuleSource)
}

$manifest = Import-PowerShellDataFile -LiteralPath (Join-Path $ModuleSource 'Termyx.psd1')
$version = [string]$manifest.ModuleVersion
$target = Join-Path $destination $version

if (-not (Test-Path -LiteralPath $target)) {
    New-Item -ItemType Directory -Force -Path $target | Out-Null
}
Copy-Item -Path (Join-Path $ModuleSource '*') -Destination $target -Recurse -Force

$profileDirectory = Split-Path -Parent $ProfilePath
if ($profileDirectory -and -not (Test-Path -LiteralPath $profileDirectory)) {
    New-Item -ItemType Directory -Force -Path $profileDirectory | Out-Null
}

if (-not (Test-TermyxProfileImportLine -Path $ProfilePath)) {
    $wide = Get-TermyxProfileWideEncoding -Path $ProfilePath
    $existingLength = 0
    $endsWithNewline = $false
    if (Test-Path -LiteralPath $ProfilePath) {
        $existingLength = (Get-Item -LiteralPath $ProfilePath).Length
        if ($existingLength -gt 0) {
            $handle = [System.IO.File]::OpenRead($ProfilePath)
            try {
                $handle.Seek(-1, [System.IO.SeekOrigin]::End) | Out-Null
                $endsWithNewline = ($handle.ReadByte() -eq 10)
            }
            finally {
                $handle.Close()
            }
        }
    }

    # Start on a fresh line only when the existing profile does not already end
    # with one, so uninstalling this line restores the profile byte for byte.
    $prefix = ''
    if ($existingLength -gt 0 -and -not $endsWithNewline) {
        $prefix = [Environment]::NewLine
    }
    $text = $prefix + $importLine + [Environment]::NewLine

    $stream = [System.IO.File]::Open($ProfilePath, [System.IO.FileMode]::Append, [System.IO.FileAccess]::Write)
    try {
        if ($null -ne $wide) {
            if ($stream.Length -eq 0) {
                $preamble = $wide.GetPreamble()
                if ($preamble.Length -gt 0) {
                    $stream.Write($preamble, 0, $preamble.Length)
                }
            }
            $bytes = $wide.GetBytes($text)
        }
        else {
            $bytes = [System.Text.Encoding]::ASCII.GetBytes($text)
        }
        $stream.Write($bytes, 0, $bytes.Length)
    }
    finally {
        $stream.Close()
    }
}

Write-Output ('Installed Termyx ' + $version + ' to ' + $target)
