<#
.SYNOPSIS
    Adds or removes one folder on the current user's PATH.

.DESCRIPTION
    The Inno Setup wizard calls this script so that PATH edits stay idempotent
    and reversible: installing twice does not duplicate the entry, and
    uninstalling removes exactly the folder that was added. The value keeps its
    REG_EXPAND_SZ type and running programs are told to reload the environment.

.EXAMPLE
    .\path.ps1 -Install -Directory "$env:LOCALAPPDATA\Programs\Termyx"
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Directory,
    [switch]$Install,
    [switch]$Remove
)

$ErrorActionPreference = 'Stop'

if ($Install -eq $Remove) {
    throw 'Pass exactly one of -Install or -Remove.'
}

$target = [System.IO.Path]::GetFullPath($Directory)
if ($target.Length -gt 3) {
    $target = $target.TrimEnd('\')
}

$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
if ($null -eq $key) {
    throw 'The Environment key is not accessible.'
}

$changed = $false
try {
    $current = [string]$key.GetValue(
        'Path',
        '',
        [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)

    $parts = @($current -split ';' | Where-Object { $_ })
    if ($Install) {
        if ($parts -notcontains $target) {
            $parts += $target
        }
    }
    else {
        $parts = @($parts | Where-Object { $_ -ine $target })
    }

    $updated = $parts -join ';'
    if ($updated -ne $current) {
        $key.SetValue('Path', $updated, [Microsoft.Win32.RegistryValueKind]::ExpandString)
        $changed = $true
    }
}
finally {
    $key.Close()
}

if (-not $changed) {
    return
}

# Explorer caches the environment it hands to new processes, so broadcast the
# change instead of waiting for the next sign-in.
if (-not ('Termyx.EnvironmentBroadcast' -as [type])) {
    Add-Type -Namespace Termyx -Name EnvironmentBroadcast -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll", CharSet = System.Runtime.InteropServices.CharSet.Auto, SetLastError = true)]
public static extern System.IntPtr SendMessageTimeout(
    System.IntPtr hWnd,
    uint msg,
    System.UIntPtr wParam,
    string lParam,
    uint flags,
    uint timeout,
    out System.UIntPtr result);
'@
}

$result = [System.UIntPtr]::Zero
$broadcast = [Termyx.EnvironmentBroadcast]::SendMessageTimeout(
    [System.IntPtr]0xffff,
    0x001A,
    [System.UIntPtr]::Zero,
    'Environment',
    2,
    5000,
    [ref]$result)
if ($broadcast -eq [System.IntPtr]::Zero) {
    Write-Warning 'The environment change will show up after the next sign-in.'
}
