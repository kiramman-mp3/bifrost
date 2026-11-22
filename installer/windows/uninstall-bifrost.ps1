<#
.SYNOPSIS
    Cleanly uninstalls BIFRÖST host binaries, user PATH entry, and Start Menu shortcut.
    Preserves Android userdata and configuration by default unless -PurgeUserData is explicitly set.
#>
[CmdletBinding()]
param(
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Programs\BIFRÖST"),
    [switch]$PurgeUserData,
    [switch]$ConfirmPurge
)

$ErrorActionPreference = 'Stop'

if ($PurgeUserData -and -not $ConfirmPurge) {
    throw "Refusing to purge Android userdata without explicit -ConfirmPurge switch."
}

# Attempt graceful Heimdall stop, disable login autostart, and remove HKCU .apk file association if installed.
$bifrostExe = Join-Path $InstallDir "bifrost.exe"
if (Test-Path $bifrostExe) {
    try { & $bifrostExe desktop unregister-apk-handler 2>$null | Out-Null } catch {}
    try { & $bifrostExe daemon autostart --disable 2>$null | Out-Null } catch {}
    try { & $bifrostExe daemon stop 2>$null | Out-Null } catch {}
}

$shortcutPath = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\BIFRÖST.lnk"
if (Test-Path $shortcutPath) {
    Remove-Item -Force $shortcutPath
    Write-Host "Removed Start Menu shortcut."
}

if (Test-Path $InstallDir) {
    Remove-Item -Recurse -Force $InstallDir
    Write-Host "Removed installed binaries at '$InstallDir'."
}

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not [string]::IsNullOrWhiteSpace($userPath)) {
    $filtered = ($userPath -split ';') | Where-Object { -not [string]::IsNullOrWhiteSpace($_) -and $_ -ne $InstallDir }
    [Environment]::SetEnvironmentVariable("Path", ($filtered -join ';'), "User")
    Write-Host "Cleaned user PATH."
}

$dataRoot = Join-Path $env:LOCALAPPDATA "bifrost"
if ($PurgeUserData -and $ConfirmPurge) {
    if (Test-Path $dataRoot) {
        Remove-Item -Recurse -Force $dataRoot
        Write-Host "Purged BIFRÖST config and Android userdata at '$dataRoot'."
    }
} else {
    Write-Host "Preserved BIFRÖST configuration and Android userdata at '$dataRoot'."
}
