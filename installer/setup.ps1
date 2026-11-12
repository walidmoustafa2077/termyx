<#
.SYNOPSIS
    Installs Termyx from a terminal window.

.DESCRIPTION
    Shows a small keyboard-driven setup screen, then installs the chosen
    pieces with a progress bar and one line per step. Everything is per-user,
    so no administrator rights are needed.

    Run setup.cmd to start this script inside a console window. Use -Yes to
    skip the menu and install with the defaults, which is what install.ps1 and
    unattended installs do.

    Use -Uninstall to remove Termyx. The config in %APPDATA%\termyx and the
    user's spellcheck words are kept.

.EXAMPLE
    .\setup.cmd

.EXAMPLE
    .\setup.ps1 -Yes -NoModule

.EXAMPLE
    .\setup.ps1 -Uninstall
#>
[CmdletBinding()]
param(
    [string]$InstallDir,
    [string]$ExePath,
    [switch]$Uninstall,
    [switch]$Yes,
    [switch]$NoPath,
    [switch]$NoShortcut,
    [switch]$Desktop,
    [switch]$NoContextMenu,
    [switch]$NoAdminMenu,
    [switch]$NoModule,
    [switch]$Launch
)

$ErrorActionPreference = 'Stop'

$appName = 'Termyx'
$publisher = 'Walid Mostafa'
$startMenuDir = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$desktopDir = [Environment]::GetFolderPath('Desktop')

if (-not $InstallDir) {
    $InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\Termyx'
}
$script:installDir = [System.IO.Path]::GetFullPath($InstallDir)
if ($script:installDir.Length -gt 3) {
    $script:installDir = $script:installDir.TrimEnd('\')
}

if (-not $ExePath) {
    $ExePath = Join-Path $PSScriptRoot 'termyx.exe'
}
$script:sourceExe = [System.IO.Path]::GetFullPath($ExePath)

$script:version = 'dev'
if (Test-Path -LiteralPath $script:sourceExe) {
    $versionInfo = (Get-Item -LiteralPath $script:sourceExe).VersionInfo
    if ($versionInfo.ProductVersion) {
        $script:version = $versionInfo.ProductVersion
    }
}

function Resolve-Resource {
    param([string[]]$Candidates)

    foreach ($candidate in $Candidates) {
        if (Test-Path -LiteralPath $candidate) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }
    return $null
}

# The downloaded zip keeps everything beside setup.ps1; an installed copy keeps
# the module in <app>\powershell; a checkout keeps it under shell\powershell.
$script:moduleSource = Resolve-Resource @(
    (Join-Path $PSScriptRoot 'powershell\Termyx'),
    (Join-Path $PSScriptRoot '..\powershell\Termyx'),
    (Join-Path $PSScriptRoot '..\shell\powershell\Termyx')
)
$script:moduleInstaller = Resolve-Resource @(
    (Join-Path $PSScriptRoot 'powershell\tools\Install.ps1'),
    (Join-Path $PSScriptRoot '..\powershell\tools\Install.ps1'),
    (Join-Path $PSScriptRoot '..\shell\powershell\tools\Install.ps1')
)
$script:pathScript = Resolve-Resource @(
    (Join-Path $PSScriptRoot 'path.ps1'),
    (Join-Path $PSScriptRoot '..\installer\path.ps1')
)

$script:options = [ordered]@{
    Shortcut    = -not $NoShortcut
    Desktop     = [bool]$Desktop
    Path        = -not $NoPath
    ContextMenu = -not $NoContextMenu
    AdminMenu   = -not $NoAdminMenu
    Module      = -not $NoModule
    Launch      = [bool]$Launch
}

$script:rows = @(
    @{ Key = 'Shortcut'; Label = 'Start Menu shortcut' },
    @{ Key = 'Desktop'; Label = 'Desktop shortcut' },
    @{ Key = 'Path'; Label = 'Add Termyx to PATH (run "termyx" anywhere)' },
    @{ Key = 'ContextMenu'; Label = 'Explorer menu: "Open in Termyx"' },
    @{ Key = 'AdminMenu'; Label = 'Explorer menu: "Open in Termyx (Administrator)"' },
    @{ Key = 'Module'; Label = 'Termyx PowerShell module (prompt and spellcheck)' },
    @{ Key = 'Launch'; Label = 'Launch Termyx when setup finishes' }
)

$script:selected = 0
$script:message = ''

# The animated progress needs a real console window. The menu additionally
# needs an unattended run to be off, so -Yes installs and -Uninstall skip it.
$script:canAnimate = $false
try {
    $script:canAnimate = (-not [Console]::IsInputRedirected) -and
        ($Host.Name -eq 'ConsoleHost') -and
        ([Console]::WindowHeight -ge 16) -and
        ([Console]::WindowWidth -ge 50)
}
catch {
    $script:canAnimate = $false
}
$script:interactive = $script:canAnimate -and (-not $Yes) -and (-not $Uninstall)

$script:defaultFg = [ConsoleColor]::Gray
$script:defaultBg = [ConsoleColor]::Black
if ($script:canAnimate) {
    $script:defaultFg = [Console]::ForegroundColor
    $script:defaultBg = [Console]::BackgroundColor
}

function Get-ConsoleWidth {
    try {
        $width = [Console]::WindowWidth - 1
    }
    catch {
        $width = 79
    }
    if ($width -lt 40) {
        $width = 40
    }
    return $width
}

function Set-Row {
    param([int]$Row)

    try {
        [Console]::SetCursorPosition(0, $Row)
    }
    catch {
    }
}

function Write-At {
    param(
        [int]$Row,
        [string]$Text,
        [ConsoleColor]$Color = [ConsoleColor]::Gray
    )

    $width = Get-ConsoleWidth
    if ($Text.Length -gt $width) {
        $Text = $Text.Substring(0, $width)
    }
    Set-Row $Row
    [Console]::ForegroundColor = $Color
    [Console]::Write($Text.PadRight($width))
    [Console]::ForegroundColor = $script:defaultFg
}

function Get-InstalledExe {
    return (Join-Path $script:installDir 'termyx.exe')
}

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

function Set-RegistryDword {
    param([string]$Path, [string]$Name, [int]$Value)

    if (-not (Test-Path -LiteralPath $Path)) {
        New-Item -Path $Path -Force | Out-Null
    }
    New-ItemProperty -Path $Path -Name $Name -Value $Value -PropertyType DWord -Force | Out-Null
}

# Explorer shows these verbs when a folder, a folder's background, or a drive
# is right-clicked. %1 is the selected item; %V is the background folder.
$script:contextMenus = @(
    @{ Key = 'Directory\shell\Termyx'; Argument = '%1' },
    @{ Key = 'Directory\Background\shell\Termyx'; Argument = '%V' },
    @{ Key = 'Drive\shell\Termyx'; Argument = '%1' }
)
$script:adminContextMenus = @(
    @{ Key = 'Directory\shell\TermyxAdmin'; Argument = '%1' },
    @{ Key = 'Directory\Background\shell\TermyxAdmin'; Argument = '%V' },
    @{ Key = 'Drive\shell\TermyxAdmin'; Argument = '%1' }
)

function New-Shortcut {
    param([string]$Path, [string]$Target)

    $directory = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $directory)) {
        New-Item -ItemType Directory -Force -Path $directory | Out-Null
    }
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($Path)
    $shortcut.TargetPath = $Target
    $shortcut.WorkingDirectory = $env:USERPROFILE
    $shortcut.IconLocation = ($Target + ',0')
    $shortcut.Description = 'Termyx terminal'
    $shortcut.Save()
}

function Get-StartMenuItem {
    return (Join-Path $startMenuDir ($appName + '.lnk'))
}

function Get-DesktopItem {
    return (Join-Path $desktopDir ($appName + '.lnk'))
}

function Copy-SetupFile {
    param([string]$Name, [string]$Destination)

    $source = Resolve-Resource @(
        (Join-Path $PSScriptRoot $Name),
        (Join-Path $PSScriptRoot ('..\installer\' + $Name))
    )
    if (-not $source) {
        return
    }
    if ($source -eq $Destination) {
        return
    }
    Copy-Item -LiteralPath $source -Destination $Destination -Force
}

function Install-Files {
    if (-not (Test-Path -LiteralPath $script:sourceExe)) {
        throw ('termyx.exe was not found at ' + $script:sourceExe + '. Keep it next to setup.ps1.')
    }
    New-Item -ItemType Directory -Force -Path $script:installDir | Out-Null
    Copy-Item -LiteralPath $script:sourceExe -Destination (Get-InstalledExe) -Force

    # Keep the setup script beside the app so Uninstall can run later without
    # the original download.
    $setupDir = Join-Path $script:installDir 'setup'
    New-Item -ItemType Directory -Force -Path $setupDir | Out-Null
    foreach ($name in @('setup.ps1', 'setup.cmd', 'path.ps1')) {
        Copy-SetupFile -Name $name -Destination (Join-Path $setupDir $name)
    }

    if ($script:moduleSource) {
        $target = Join-Path $script:installDir 'powershell\Termyx'
        New-Item -ItemType Directory -Force -Path $target | Out-Null
        Copy-Item -Path (Join-Path $script:moduleSource '*') -Destination $target -Recurse -Force
    }
    if ($script:moduleInstaller) {
        $target = Join-Path $script:installDir 'powershell\tools'
        New-Item -ItemType Directory -Force -Path $target | Out-Null
        $destination = Join-Path $target 'Install.ps1'
        if ($script:moduleInstaller -ne $destination) {
            Copy-Item -LiteralPath $script:moduleInstaller -Destination $destination -Force
        }
    }
}

function Add-PathEntry {
    if (-not $script:pathScript) {
        throw 'path.ps1 was not found next to setup.ps1.'
    }
    & $script:pathScript -Install -Directory $script:installDir
}

function Remove-PathEntry {
    if (-not $script:pathScript) {
        return
    }
    & $script:pathScript -Remove -Directory $script:installDir
}

function Register-ContextMenu {
    $classes = 'HKCU:\Software\Classes'
    $exe = Get-InstalledExe

    foreach ($menu in $script:contextMenus) {
        $key = Join-Path $classes $menu.Key
        $commandKey = Join-Path $key 'command'
        Set-RegistryDefault -Path $key -Value 'Open in Termyx'
        Set-RegistryString -Path $key -Name 'Icon' -Value ($exe + ',0')
        Set-RegistryDefault -Path $commandKey -Value ('"' + $exe + '" "' + $menu.Argument + '"')
    }
}

function Register-AdminMenu {
    $classes = 'HKCU:\Software\Classes'
    $exe = Get-InstalledExe

    # The shell has no per-verb elevation flag, so these entries start a second,
    # elevated Termyx through Start-Process. The shield icon marks them.
    foreach ($menu in $script:adminContextMenus) {
        $key = Join-Path $classes $menu.Key
        $commandKey = Join-Path $key 'command'
        Set-RegistryDefault -Path $key -Value 'Open in Termyx (Administrator)'
        Set-RegistryString -Path $key -Name 'Icon' -Value ($exe + ',0')
        Set-RegistryString -Path $key -Name 'HasLUAShield' -Value ''
        $command = 'powershell.exe -NoProfile -WindowStyle Hidden -Command "Start-Process ' +
            '-FilePath ''' + $exe + ''' -ArgumentList @(''' + $menu.Argument +
            ''') -Verb RunAs"'
        Set-RegistryDefault -Path $commandKey -Value $command
    }
}

function Unregister-Menus {
    $classes = 'HKCU:\Software\Classes'
    foreach ($menu in ($script:contextMenus + $script:adminContextMenus)) {
        $key = Join-Path $classes $menu.Key
        if (Test-Path -LiteralPath $key) {
            Remove-Item -LiteralPath $key -Recurse -Force
        }
    }
}

function Get-UninstallCommand {
    $powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $script = Join-Path $script:installDir 'setup\setup.ps1'
    return ('"' + $powershell + '" -NoProfile -ExecutionPolicy Bypass -File "' + $script + '" -Uninstall -Yes')
}

function Register-AppEntry {
    $exe = Get-InstalledExe
    $appPathsKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\termyx.exe'
    Set-RegistryDefault -Path $appPathsKey -Value $exe
    Set-RegistryString -Path $appPathsKey -Name 'Path' -Value $script:installDir

    # Makes Termyx show up in Settings > Apps so it can be removed from there.
    $uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Termyx'
    $command = Get-UninstallCommand
    Set-RegistryString -Path $uninstallKey -Name 'DisplayName' -Value $appName
    Set-RegistryString -Path $uninstallKey -Name 'DisplayVersion' -Value $script:version
    Set-RegistryString -Path $uninstallKey -Name 'Publisher' -Value $publisher
    Set-RegistryString -Path $uninstallKey -Name 'DisplayIcon' -Value ($exe + ',0')
    Set-RegistryString -Path $uninstallKey -Name 'InstallLocation' -Value $script:installDir
    Set-RegistryString -Path $uninstallKey -Name 'UninstallString' -Value $command
    Set-RegistryString -Path $uninstallKey -Name 'QuietUninstallString' -Value $command
    Set-RegistryDword -Path $uninstallKey -Name 'NoModify' -Value 1
    Set-RegistryDword -Path $uninstallKey -Name 'NoRepair' -Value 1
}

function Unregister-AppEntry {
    foreach ($key in @(
            'HKCU:\Software\Microsoft\Windows\CurrentVersion\App Paths\termyx.exe',
            'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Termyx')) {
        if (Test-Path -LiteralPath $key) {
            Remove-Item -LiteralPath $key -Recurse -Force
        }
    }
}

function Install-Shortcuts {
    New-Shortcut -Path (Get-StartMenuItem) -Target (Get-InstalledExe)
}

function Install-DesktopShortcut {
    New-Shortcut -Path (Get-DesktopItem) -Target (Get-InstalledExe)
}

function Remove-Shortcuts {
    foreach ($path in @((Get-StartMenuItem), (Get-DesktopItem))) {
        if (Test-Path -LiteralPath $path) {
            Remove-Item -LiteralPath $path -Force
        }
    }
}

function Install-PowerShellModule {
    if (-not $script:moduleInstaller) {
        throw 'The Termyx module installer was not found next to setup.ps1.'
    }
    & $script:moduleInstaller -ModuleSource $script:moduleSource | Out-Null
}

function Uninstall-PowerShellModule {
    if (-not $script:moduleInstaller) {
        return
    }
    & $script:moduleInstaller -Uninstall | Out-Null
}

function Remove-InstallFolder {
    if (-not (Test-Path -LiteralPath $script:installDir)) {
        return
    }
    # Do not stand inside the folder that is about to disappear.
    Set-Location -LiteralPath $env:USERPROFILE
    Remove-Item -LiteralPath $script:installDir -Recurse -Force
}

function Get-InstallSteps {
    $steps = New-Object System.Collections.Generic.List[object]
    $steps.Add(@{ Label = 'Copying Termyx'; Action = { Install-Files } })
    if ($script:options.Path) {
        $steps.Add(@{ Label = 'Adding Termyx to PATH'; Action = { Add-PathEntry } })
    }
    if ($script:options.Shortcut) {
        $steps.Add(@{ Label = 'Creating the Start Menu shortcut'; Action = { Install-Shortcuts } })
    }
    if ($script:options.Desktop) {
        $steps.Add(@{ Label = 'Creating the desktop shortcut'; Action = { Install-DesktopShortcut } })
    }
    if ($script:options.ContextMenu) {
        $steps.Add(@{ Label = 'Registering the Explorer menu'; Action = { Register-ContextMenu } })
    }
    if ($script:options.AdminMenu) {
        $steps.Add(@{ Label = 'Registering the administrator menu'; Action = { Register-AdminMenu } })
    }
    $steps.Add(@{ Label = 'Registering Termyx in Windows'; Action = { Register-AppEntry } })
    if ($script:options.Module) {
        $steps.Add(@{ Label = 'Installing the Termyx PowerShell module'; Action = { Install-PowerShellModule } })
    }
    return $steps
}

function Get-UninstallSteps {
    $steps = New-Object System.Collections.Generic.List[object]
    $steps.Add(@{ Label = 'Removing the Explorer menus'; Action = { Unregister-Menus } })
    $steps.Add(@{ Label = 'Removing the Windows registration'; Action = { Unregister-AppEntry } })
    $steps.Add(@{ Label = 'Removing the shortcuts'; Action = { Remove-Shortcuts } })
    if ($script:options.Path) {
        $steps.Add(@{ Label = 'Removing Termyx from PATH'; Action = { Remove-PathEntry } })
    }
    if ($script:options.Module) {
        $steps.Add(@{ Label = 'Removing the Termyx PowerShell module'; Action = { Uninstall-PowerShellModule } })
    }
    $steps.Add(@{ Label = 'Deleting the install folder'; Action = { Remove-InstallFolder } })
    return $steps
}

function Write-ProgressBar {
    param([int]$Percent)

    if (-not $script:canAnimate) {
        return
    }
    $width = Get-ConsoleWidth
    $inner = [Math]::Max(10, $width - 16)
    $filled = [int][Math]::Floor($inner * $Percent / 100)
    $bar = '[' + (-join ('#' * $filled)) + (-join ('-' * ($inner - $filled))) + ']'
    Write-At -Row 5 -Text ('  ' + $bar + ' ' + ([string]$Percent).PadLeft(3) + '%') -Color ([ConsoleColor]::Cyan)
}

function Write-StepResult {
    param([string]$Text, [ConsoleColor]$Color)

    if ($script:canAnimate) {
        Write-At -Row $script:stepRow -Text $Text -Color $Color
        $script:stepRow++
    }
    else {
        Write-Host ('    ' + $Text.Trim())
    }
}

function Invoke-Steps {
    param(
        [string]$Title,
        [object[]]$Steps
    )

    $total = $Steps.Count
    $script:stepRow = 7

    if ($script:canAnimate) {
        try {
            [Console]::CursorVisible = $false
            [Console]::Clear()
        }
        catch {
        }
        Write-At -Row 1 -Text ('  ' + $appName + ' ' + $script:version + ' setup') -Color ([ConsoleColor]::Cyan)
        Write-At -Row 3 -Text ('  ' + $Title) -Color ([ConsoleColor]::White)
        Write-ProgressBar -Percent 0
    }
    else {
        Write-Host ''
        Write-Host ('==> ' + $Title) -ForegroundColor Cyan
    }

    $index = 0
    foreach ($step in $Steps) {
        $index++
        if ($script:canAnimate) {
            Write-At -Row 4 -Text ('  ' + $step.Label + '  (' + $index + '/' + $total + ')') -Color ([ConsoleColor]::DarkGray)
        }
        else {
            Write-Host ('==> ' + $step.Label)
        }

        try {
            & $step.Action
        }
        catch {
            if ($script:canAnimate) {
                Set-Row ($script:stepRow + 1)
                try {
                    [Console]::CursorVisible = $true
                }
                catch {
                }
                [Console]::ResetColor()
            }
            throw
        }

        $percent = [int](($index * 100) / $total)
        Write-StepResult -Text ('  [ok] ' + $step.Label) -Color ([ConsoleColor]::Green)
        Write-ProgressBar -Percent $percent
    }

    Write-ProgressBar -Percent 100
}

function Show-Menu {
    $lines = New-Object System.Collections.Generic.List[object]
    $lines.Add(@{ Text = ''; Color = [ConsoleColor]::Gray; Selected = $false })
    $lines.Add(@{ Text = ('  ' + $appName + ' ' + $script:version + ' setup'); Color = [ConsoleColor]::Cyan; Selected = $false })
    $lines.Add(@{ Text = '  A fast, lightweight terminal with RTL support.'; Color = [ConsoleColor]::DarkGray; Selected = $false })
    $lines.Add(@{ Text = ''; Color = [ConsoleColor]::Gray; Selected = $false })
    $lines.Add(@{ Text = '  Install folder'; Color = [ConsoleColor]::DarkGray; Selected = $false })
    $lines.Add(@{ Text = ('    ' + $script:installDir); Color = [ConsoleColor]::White; Selected = $false })
    $lines.Add(@{ Text = '    Press E to choose another folder.'; Color = [ConsoleColor]::DarkGray; Selected = $false })
    $lines.Add(@{ Text = ''; Color = [ConsoleColor]::Gray; Selected = $false })
    $lines.Add(@{ Text = ('  What to install into ' + $env:USERNAME + '''s account'); Color = [ConsoleColor]::DarkGray; Selected = $false })
    $lines.Add(@{ Text = ''; Color = [ConsoleColor]::Gray; Selected = $false })

    for ($i = 0; $i -lt $script:rows.Count; $i++) {
        $row = $script:rows[$i]
        $mark = if ($script:options[$row.Key]) { '[x]' } else { '[ ]' }
        $lines.Add(@{
                Text     = ('  ' + $mark + ' ' + $row.Label)
                Color    = [ConsoleColor]::White
                Selected = ($i -eq $script:selected)
            })
    }

    $lines.Add(@{ Text = ''; Color = [ConsoleColor]::Gray; Selected = $false })
    $lines.Add(@{ Text = '  Up/Down move   Space toggles   Enter installs   Q quits'; Color = [ConsoleColor]::DarkGray; Selected = $false })
    $lines.Add(@{ Text = ('  ' + $script:message); Color = [ConsoleColor]::Yellow; Selected = $false })

    $height = 20
    try {
        $height = [Console]::WindowHeight - 1
    }
    catch {
    }

    $width = Get-ConsoleWidth
    for ($i = 0; $i -lt $height; $i++) {
        Set-Row $i
        if ($i -lt $lines.Count) {
            $line = $lines[$i]
            $text = $line.Text
            if ($text.Length -gt $width) {
                $text = $text.Substring(0, $width)
            }
            $text = $text.PadRight($width)
            if ($line.Selected) {
                [Console]::BackgroundColor = [ConsoleColor]::Cyan
                [Console]::ForegroundColor = [ConsoleColor]::Black
            }
            else {
                [Console]::BackgroundColor = $script:defaultBg
                [Console]::ForegroundColor = $line.Color
            }
            [Console]::Write($text)
        }
        else {
            [Console]::BackgroundColor = $script:defaultBg
            [Console]::ForegroundColor = $script:defaultFg
            [Console]::Write((' ' * $width))
        }
    }
    [Console]::ResetColor()
}

function Edit-InstallDir {
    try {
        [Console]::CursorVisible = $true
    }
    catch {
    }
    [Console]::ResetColor()
    $row = 19
    try {
        $row = [Math]::Min(19, [Console]::WindowHeight - 2)
    }
    catch {
    }
    Set-Row $row
    $entered = Read-Host -Prompt ('  Install folder [' + $script:installDir + ']')
    if ($entered) {
        try {
            $resolved = [System.IO.Path]::GetFullPath($entered)
            if ($resolved.Length -gt 3) {
                $resolved = $resolved.TrimEnd('\')
            }
            $script:installDir = $resolved
            $script:message = ''
        }
        catch {
            $script:message = 'That folder path is not valid.'
        }
    }
    try {
        [Console]::CursorVisible = $false
    }
    catch {
    }
}

# Returns 'install', 'cancel', or 'uninstall' (the latter never from this menu).
function Read-MenuChoice {
    :menu while ($true) {
        Show-Menu
        $key = [Console]::ReadKey($true)
        switch ($key.Key) {
            'UpArrow' {
                $script:selected = ($script:selected - 1 + $script:rows.Count) % $script:rows.Count
            }
            'DownArrow' {
                $script:selected = ($script:selected + 1) % $script:rows.Count
            }
            'Spacebar' {
                $row = $script:rows[$script:selected]
                $script:options[$row.Key] = -not $script:options[$row.Key]
            }
            'Enter' {
                return 'install'
            }
            'Escape' {
                return 'cancel'
            }
            'Q' {
                return 'cancel'
            }
            'E' {
                Edit-InstallDir
            }
        }
    }
}

function Complete-Setup {
    param([string]$Summary, [string]$Note, [switch]$Launch)

    if ($script:canAnimate) {
        Write-At -Row ($script:stepRow + 1) -Text ('  ' + $Summary) -Color ([ConsoleColor]::Green)
        Write-At -Row ($script:stepRow + 2) -Text ('  ' + $Note) -Color ([ConsoleColor]::DarkGray)
        Write-At -Row ($script:stepRow + 4) -Text '  Press any key to close this window.' -Color ([ConsoleColor]::DarkGray)
        try {
            [Console]::CursorVisible = $true
        }
        catch {
        }
        [Console]::ReadKey($true) | Out-Null
        [Console]::ResetColor()
    }
    else {
        Write-Host ''
        Write-Host ('==> ' + $Summary) -ForegroundColor Green
        Write-Host ('    ' + $Note) -ForegroundColor DarkGray
    }

    if ($Launch) {
        Start-Process -FilePath (Get-InstalledExe) -WorkingDirectory $env:USERPROFILE
    }
}

if ($Uninstall) {
    if (-not $Yes) {
        Write-Host ''
        Write-Host ('This removes ' + $appName + ' from ' + $script:installDir + ', along with its shortcuts, PATH entry,')
        Write-Host 'Explorer menus, and the Termyx PowerShell module.'
        Write-Host 'Your config in %APPDATA%\termyx is kept.'
        $answer = Read-Host ('Remove ' + $appName + '? [y/N]')
        if ($answer -notmatch '^(y|yes)$') {
            Write-Host 'Nothing was removed.'
            exit 0
        }
    }

    Invoke-Steps -Title ('Removing ' + $appName) -Steps (Get-UninstallSteps)
    Complete-Setup -Summary ($appName + ' was removed.') -Note ('Your config in ' + (Join-Path $env:APPDATA 'termyx') + ' was kept.')
    exit 0
}

if ($script:interactive) {
    $choice = Read-MenuChoice
    if ($choice -eq 'cancel') {
        try {
            [Console]::Clear()
        }
        catch {
        }
        [Console]::ResetColor()
        Write-Host 'Nothing was installed.'
        exit 0
    }
}

try {
    Invoke-Steps -Title ('Installing ' + $appName) -Steps (Get-InstallSteps)
}
catch {
    if ($script:canAnimate) {
        [Console]::ResetColor()
        Set-Row ($script:stepRow + 1)
        Write-Host ''
        Write-Host ('  Setup failed: ' + $_.Exception.Message) -ForegroundColor Red
        Write-Host '  Press any key to close this window.' -ForegroundColor DarkGray
        [Console]::ReadKey($true) | Out-Null
    }
    else {
        Write-Host ('Setup failed: ' + $_.Exception.Message) -ForegroundColor Red
    }
    exit 1
}

Complete-Setup -Summary ('Installed ' + $appName + ' to ' + $script:installDir) `
    -Note 'Open a new terminal and run "termyx", or right-click a folder in Explorer.' `
    -Launch:([bool]$script:options.Launch)
exit 0
