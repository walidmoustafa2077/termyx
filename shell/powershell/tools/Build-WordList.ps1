[CmdletBinding()]
param(
    [string]$DicPath,
    [string]$AffPath,
    [string]$OutFile
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot

if (-not $OutFile) {
    $OutFile = Join-Path $repoRoot 'Termyx\assets\en_US.wordlist.txt'
}

. (Join-Path $PSScriptRoot 'Hunspell.ps1')

$cacheDir = Join-Path $PSScriptRoot '.cache'
$dicUrl = 'https://raw.githubusercontent.com/wooorm/dictionaries/main/dictionaries/en/index.dic'
$affUrl = 'https://raw.githubusercontent.com/wooorm/dictionaries/main/dictionaries/en/index.aff'

if (-not $DicPath) {
    New-Item -ItemType Directory -Force -Path $cacheDir | Out-Null
    $DicPath = Join-Path $cacheDir 'en.dic'
    if (-not (Test-Path -LiteralPath $DicPath)) {
        $client = New-Object System.Net.WebClient
        $client.Headers.Add('User-Agent', 'Termyx-Build')
        $client.DownloadFile($dicUrl, $DicPath)
    }
}

if (-not $AffPath) {
    New-Item -ItemType Directory -Force -Path $cacheDir | Out-Null
    $AffPath = Join-Path $cacheDir 'en.aff'
    if (-not (Test-Path -LiteralPath $AffPath)) {
        $client = New-Object System.Net.WebClient
        $client.Headers.Add('User-Agent', 'Termyx-Build')
        $client.DownloadFile($affUrl, $AffPath)
    }
}

Write-Output ('Reading ' + $DicPath)
Write-Output ('Reading ' + $AffPath)

$dicLines = Get-Content -LiteralPath $DicPath -Encoding UTF8
# Blank lines in the downloaded .aff would be rejected by the Mandatory [string[]] parameter.
$affLines = @(Get-Content -LiteralPath $AffPath -Encoding UTF8 | Where-Object { $_.Trim().Length -gt 0 })

$stopwatch = [Diagnostics.Stopwatch]::StartNew()
$words = Expand-HunspellDictionary -DicLines $dicLines -AffLines $affLines
$stopwatch.Stop()

$outputDirectory = Split-Path -Parent $OutFile
if (-not (Test-Path -LiteralPath $outputDirectory)) {
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
}

Set-Content -LiteralPath $OutFile -Value $words -Encoding ASCII

Write-Output ('Wrote ' + $words.Count + ' words to ' + $OutFile + ' in ' + $stopwatch.Elapsed.TotalSeconds.ToString('0.0') + ' s')
