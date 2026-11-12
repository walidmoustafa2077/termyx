<#
.SYNOPSIS
    Builds and installs Termyx for the current Windows user.

.DESCRIPTION
    The default flow builds the release binary, copies it under
    %LOCALAPPDATA%\Programs\Termyx, adds that folder to the user PATH, creates a
    Start Menu shortcut, installs the Termyx PowerShell module, and
    registers "Open in Termyx" entries in Explorer's context menu for folders,
    folder backgrounds, and drives. Every step is per-user; no administrator
    rights are needed for the installation itself.

    Use -Uninstall to remove the same pieces. The user's config in
    %APPDATA%\termyx and their spellcheck words are left in place.

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
    [switch]$NoContextMenu,
    [switch]$NoModule
)

$ErrorActionPreference = 'Stop'

$repoRoot = $PSScriptRoot
if (-not $InstallDir) {
    $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\Termyx'
}
$installDir = [System.IO.Path]::GetFullPath($InstallDir)
$exePath = Join-Path $installDir 'termyx.exe'
$startMenuDir = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$shortcutPath = Join-Path $startMenuDir 'Termyx.lnk'
$appPathsKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\termyx.exe'
$moduleInstaller = Join-Path $repoRoot 'shell\powershell\tools\Install.ps1'

# Explorer shows these verbs when a folder, a folder's background, or a drive
# is right-clicked. `%1` is the selected item; `%V` is the background folder.
$contextMenus = @(
    @{ Key = 'Directory\shell\Termyx'; Argument = '%1' },
    @{ Key = 'Directory\Background\shell\Termyx'; Argument = '%V' },
    @{ Key = 'Drive\shell\Termyx'; Argument = '%1' }
)
$adminContextMenus = @(
    @{ Key = 'Directory\shell\TermyxAdmin'; Argument = '%1' },
    @{ Key = 'Directory\Background\shell\TermyxAdmin'; Argument = '%V' },
    @{ Key = 'Drive\shell\TermyxAdmin'; Argument = '%1' }
)

function Set-RegistryDefault {
    param([string]$Path, [string]$Value)

    if (-not (Test-Path -LiteralPath $Path)) {
        New-Item -Path $Path -Force | Out-Null
    }
    Set-Item -Path $Path -Value $Value
}

function Set-RegistryString {
    param([string]$Path, [string]$Name, [string]$Value)

    if (-not (Test-Path -LiteralPath $Path)) {
        New-Item -Path $Path -Force | Out-Null
    }
    New-ItemProperty -Path $Path -Name $Name -Value $Value -PropertyType String -Force | Out-Null
}

function Register-TermyxContextMenu {
    $classes = 'HKCU:\Software\Classes'

    foreach ($menu in $contextMenus) {
        $key = Join-Path $classes $menu.Key
        $commandKey = Join-Path $key 'command'
        Set-RegistryDefault -Path $key -Value 'Open in Termyx'
        Set-RegistryString -Path $key -Name 'Icon' -Value ($exePath + ',0')
        Set-RegistryDefault -Path $commandKey -Value ('"' + $exePath + '" "' + $menu.Argument + '"')
    }

    # The shell has no per-verb elevation flag, so the admin entries start a
    # second, elevated Termyx through Start-Process. The shield icon marks them.
    foreach ($menu in $adminContextMenus) {
        $key = Join-Path $classes $menu.Key
        $commandKey = Join-Path $key 'command'
        Set-RegistryDefault -Path $key -Value 'Open in Termyx (Administrator)'
        Set-RegistryString -Path $key -Name 'Icon' -Value ($exePath + ',0')
        Set-RegistryString -Path $key -Name 'HasLUAShield' -Value ''
        $command = 'powershell.exe -NoProfile -WindowStyle Hidden -Command "Start-Process ' +
            '-FilePath ''' + $exePath + ''' -ArgumentList @(''' + $menu.Argument +
            ''') -Verb RunAs"'
        Set-RegistryDefault -Path $commandKey -Value $command
    }
}

function Unregister-TermyxContextMenu {
    $classes = 'HKCU:\Software\Classes'
    foreach ($menu in ($contextMenus + $adminContextMenus)) {
        $key = Join-Path $classes $menu.Key
        if (Test-Path -LiteralPath $key) {
            Remove-Item -LiteralPath $key -Recurse -Force
        }
    }
}

function Install-Shortcut {
    if (-not (Test-Path -LiteralPath $startMenuDir)) {
        New-Item -ItemType Directory -Force -Path $startMenuDir | Out-Null
    }
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($shortcutPath)
    $shortcut.TargetPath = $exePath
    $shortcut.WorkingDirectory = $env:USERPROFILE
    $shortcut.IconLocation = ($exePath + ',0')
    $shortcut.Description = 'Termyx terminal'
    $shortcut.Save()
}

function Add-UserPath {
    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    $parts = @()
    if ($current) {
        $parts = @($current -split ';' | Where-Object { $_ })
    }
    if ($parts -notcontains $installDir) {
        [Environment]::SetEnvironmentVariable('Path', (@($parts) + $installDir) -join ';', 'User')
    }
    # Make `termyx` available to shells started from this one as well.
    if (($env:Path -split ';') -notcontains $installDir) {
        $env:Path = $env:Path + ';' + $installDir
    }
}

function Remove-UserPath {
    $current = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $current) {
        return
    }
    $parts = @($current -split ';' | Where-Object { $_ -and ($_ -ine $installDir) })
    [Environment]::SetEnvironmentVariable('Path', ($parts -join ';'), 'User')
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

function Invoke-ModuleInstaller {
    param([switch]$Remove)

    if (-not (Test-Path -LiteralPath $moduleInstaller)) {
        return
    }
    try {
        if ($Remove) {
            & $moduleInstaller -Uninstall | Out-Null
        }
        else {
            & $moduleInstaller | Out-Null
        }
    }
    catch {
        Write-Warning ('The Termyx PowerShell module was not updated: ' + $_.Exception.Message)
    }
}

if ($Uninstall) {
    Write-Host '==> Removing Termyx' -ForegroundColor Cyan
    Unregister-TermyxContextMenu
    if (Test-Path -LiteralPath $appPathsKey) {
        Remove-Item -LiteralPath $appPathsKey -Recurse -Force
    }
    if (Test-Path -LiteralPath $shortcutPath) {
        Remove-Item -LiteralPath $shortcutPath -Force
    }
    if (-not $NoPath) {
        Remove-UserPath
    }
    if (-not $NoModule) {
        Invoke-ModuleInstaller -Remove
    }
    if (Test-Path -LiteralPath $installDir) {
        Remove-Item -LiteralPath $installDir -Recurse -Force
    }

    Write-Host ('Removed Termyx from ' + $installDir + '.') -ForegroundColor Green
    Write-Host ('Your config in ' + (Join-Path $env:APPDATA 'termyx') + ' was kept.') -ForegroundColor DarkGray
    return
}

if (-not $NoBuild) {
    Invoke-TermyxBuild
}

$sourceExe = Join-Path $repoRoot 'target\release\termyx.exe'
if (-not (Test-Path -LiteralPath $sourceExe)) {
    throw ('Release binary not found at ' + $sourceExe + '. Run without -NoBuild.')
}

Write-Host ('==> Installing to ' + $installDir) -ForegroundColor Cyan
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
Copy-Item -LiteralPath $sourceExe -Destination $exePath -Force

if (-not $NoPath) {
    Add-UserPath
}

if (-not $NoShortcut) {
    Install-Shortcut
}

if (-not $NoContextMenu) {
    Register-TermyxContextMenu
    Set-RegistryDefault -Path $appPathsKey -Value $exePath
    Set-RegistryString -Path $appPathsKey -Name 'Path' -Value $installDir
}

if (-not $NoModule) {
    Invoke-ModuleInstaller
}

Write-Host ('Installed Termyx to ' + $exePath) -ForegroundColor Green
if (-not $NoContextMenu) {
    Write-Host 'Right-click a folder, a folder background, or a drive for "Open in Termyx".'
}
if (-not $NoPath) {
    Write-Host 'Open a new terminal and run `termyx`, or `termyx <folder>` to start there.'
}
