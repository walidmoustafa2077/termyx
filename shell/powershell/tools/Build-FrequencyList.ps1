[CmdletBinding()]
param(
    [string]$SourcePath,
    [string]$OutFile
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot

if (-not $OutFile) {
    $OutFile = Join-Path $repoRoot 'Termyx\assets\en_US.frequency.txt'
}

$cacheDir = Join-Path $PSScriptRoot '.cache'
$sourceUrl = 'https://raw.githubusercontent.com/hermitdave/FrequencyWords/master/content/2018/en/en_50k.txt'

if (-not $SourcePath) {
    New-Item -ItemType Directory -Force -Path $cacheDir | Out-Null
    $SourcePath = Join-Path $cacheDir 'en_50k.txt'
    if (-not (Test-Path -LiteralPath $SourcePath)) {
        $client = New-Object System.Net.WebClient
        $client.Headers.Add('User-Agent', 'Termyx-Build')
        $client.DownloadFile($sourceUrl, $SourcePath)
    }
}

Write-Output ('Reading ' + $SourcePath)

$seen = New-Object 'System.Collections.Generic.HashSet[string]'
$words = New-Object 'System.Collections.Generic.List[string]'

foreach ($line in [System.IO.File]::ReadLines($SourcePath)) {
    $trimmed = $line.Trim()
    if ($trimmed.Length -eq 0) { continue }

    $space = $trimmed.IndexOf(' ')
    $word = $trimmed
    if ($space -ge 0) { $word = $trimmed.Substring(0, $space) }

    if ($word -cnotmatch '^[a-z]+$') { continue }
    if (-not $seen.Add($word)) { continue }

    $words.Add($word)
}

$outputDirectory = Split-Path -Parent $OutFile
if (-not (Test-Path -LiteralPath $outputDirectory)) {
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
}

[System.IO.File]::WriteAllLines($OutFile, $words, (New-Object System.Text.ASCIIEncoding))

Write-Output ('Wrote ' + $words.Count + ' words to ' + $OutFile)
