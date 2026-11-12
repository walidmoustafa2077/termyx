<#
.SYNOPSIS
    Builds and installs Termyx for the current Windows user.

.DESCRIPTION
    Builds the release binary, then runs the terminal setup (installer\setup.ps1)
    without its menu: copies the binary under %LOCALAPPDATA%\Programs\Termyx,
    adds it to the user PATH, creates shortcuts, installs the Termyx
    PowerShell module, and registers "Open in Termyx" in Explorer's context
    menu. Every step is per-user; no administrator rights are needed.

    For the interactive setup screen, double-click installer\setup.cmd or run
    .\installer\setup.ps1 directly.

    Use -Uninstall to remove the same pieces. The config in %APPDATA%\termyx and
    the user's spellcheck words are left in place.

.EXAMPLE
    .\install.ps1

.EXAMPLE
    .\install.ps1 -NoBuild -NoContextMenu

.EXAMPLE
    .\install.ps1 -Uninstall
#>
[CmdletBinding()]
param(
    [string]$InstallDir,
    [switch]$Uninstall,
    [switch]$NoBuild,
    [switch]$NoPath,
    [switch]$NoShortcut,
    [switch]$Desktop,
    [switch]$NoContextMenu,
    [switch]$NoAdminMenu,
    [switch]$NoModule,
    [switch]$Launch
)

$ErrorActionPreference = 'Stop'

$repoRoot = $PSScriptRoot
$setupScript = Join-Path $repoRoot 'installer\setup.ps1'
if (-not (Test-Path -LiteralPath $setupScript)) {
    throw ('The setup script was not found at ' + $setupScript + '.')
}

function Invoke-TermyxBuild {
    $cargo = Get-Command cargo -ErrorAction SilentlyContinue
    if ($cargo) {
        $cargoPath = $cargo.Source
    }
    else {
        $cargoPath = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
        if (-not (Test-Path -LiteralPath $cargoPath)) {
            throw 'cargo was not found. Install Rust from https://rustup.rs and run this script again.'
        }
    }

    Write-Host '==> Building Termyx (release)' -ForegroundColor Cyan
    Push-Location $repoRoot
    try {
        & $cargoPath build --release
        if ($LASTEXITCODE -ne 0) {
            throw ('cargo build failed with exit code ' + $LASTEXITCODE)
        }
    }
    finally {
        Pop-Location
    }
}

if (-not $Uninstall -and -not $NoBuild) {
    Invoke-TermyxBuild
}

$setupArguments = @{ Yes = $true }
if ($InstallDir) {
    $setupArguments.InstallDir = $InstallDir
}
if ($Uninstall) {
    $setupArguments.Uninstall = $true
}
if ($NoPath) {
    $setupArguments.NoPath = $true
}
if ($NoShortcut) {
    $setupArguments.NoShortcut = $true
}
if ($Desktop) {
    $setupArguments.Desktop = $true
}
if ($NoContextMenu) {
    $setupArguments.NoContextMenu = $true
}
if ($NoAdminMenu) {
    $setupArguments.NoAdminMenu = $true
}
if ($NoModule) {
    $setupArguments.NoModule = $true
}
if ($Launch) {
    $setupArguments.Launch = $true
}

if (-not $Uninstall) {
    $releaseExe = Join-Path $repoRoot 'target\release\termyx.exe'
    if (-not (Test-Path -LiteralPath $releaseExe)) {
        throw ('Release binary not found at ' + $releaseExe + '. Run without -NoBuild.')
    }
    $setupArguments.ExePath = $releaseExe
}

& $setupScript @setupArguments
exit $LASTEXITCODE
