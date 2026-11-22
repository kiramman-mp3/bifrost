<#
.SYNOPSIS
    Installs BIFRÖST into the current user's profile without requiring Administrator privileges.
    Verifies SHA-256 checksums before installation and preserves existing Android userdata.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$BundleDir,
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA "Programs\BIFRÖST")
)

$ErrorActionPreference = 'Stop'

$checksumFile = Join-Path $BundleDir "release-checksums.sha256"
if (-not (Test-Path $checksumFile)) {
    throw "Missing SHA-256 checksum manifest: $checksumFile"
}

Write-Host "Verifying release bundle SHA-256 checksums..."
$entries = Get-Content $checksumFile | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
foreach ($line in $entries) {
    if ($line -match '^([0-9a-fA-F]{64})\s+(.+)$') {
        $expected = $Matches[1].ToLowerInvariant()
        $fileName = $Matches[2].Trim()
        if ($fileName -match '[\\/]' -or $fileName -eq '..') {
            throw "Rejected unsafe filename in checksum manifest: $fileName"
        }
        $filePath = Join-Path $BundleDir $fileName
        if (-not (Test-Path $filePath)) { throw "Missing release artifact: $filePath" }
        $actual = (Get-FileHash -Algorithm SHA256 $filePath).Hash.ToLowerInvariant()
        if ($actual -ne $expected) {
            throw "SHA-256 mismatch for '$fileName': expected $expected, got $actual"
        }
    } else {
        throw "Malformed checksum line: $line"
    }
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
foreach ($line in $entries) {
    if ($line -match '^([0-9a-fA-F]{64})\s+(.+)$') {
        $fileName = $Matches[2].Trim()
        Copy-Item -Force (Join-Path $BundleDir $fileName) (Join-Path $InstallDir $fileName)
    }
}

# Add InstallDir to user PATH idempotently.
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
$parts = @()
if (-not [string]::IsNullOrWhiteSpace($userPath)) {
    $parts = $userPath -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
}
if ($parts -notcontains $InstallDir) {
    $newPath = ($parts + $InstallDir) -join ';'
    [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
    Write-Host "Added '$InstallDir' to user PATH."
}

# Create Start Menu shortcut for BIFRÖST Desktop App.
$uiExe = Join-Path $InstallDir "bifrost-ui.exe"
if (Test-Path $uiExe) {
    $startMenu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
    $shortcutPath = Join-Path $startMenu "BIFRÖST.lnk"
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($shortcutPath)
    $shortcut.TargetPath = $uiExe
    $shortcut.WorkingDirectory = $InstallDir
    $shortcut.Description = "BIFRÖST — Android Subsystem for Windows"
    $shortcut.Save()
    Write-Host "Created Start Menu shortcut at '$shortcutPath'."
}

# Register user-space .apk Explorer context menu & default double-click handler (HKCU, no admin).
$bifrostExe = Join-Path $InstallDir "bifrost.exe"
if (Test-Path $bifrostExe) {
    try {
        & $bifrostExe desktop register-apk-handler | Out-Null
        Write-Host "Registered Windows 11 .apk context menu and default handler (HKCU)."
    } catch {
        Write-Warning "Could not register .apk file association: $_"
    }
}

Write-Host "BIFRÖST installed cleanly to '$InstallDir'. Existing Android userdata was preserved."
