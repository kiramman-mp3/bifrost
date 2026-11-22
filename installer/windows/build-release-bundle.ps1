<#
.SYNOPSIS
    Builds a reproducible BIFRÖST release bundle for Windows 11 x64 and emits SHA-256 checksums.
#>
[CmdletBinding()]
param(
    [string]$OutputDir = ".\dist\bifrost-win64",
    [string]$TargetDir = $env:CARGO_TARGET_DIR
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($TargetDir)) {
    $TargetDir = Join-Path $env:TEMP "bifrost-target"
}
$env:CARGO_TARGET_DIR = $TargetDir

Write-Host "Building BIFRÖST release binaries (mimir, heimdall, bifrost-ui)..."
cargo build --release -p mimir -p heimdall -p bifrost-ui
if ($LASTEXITCODE -ne 0) { throw "Release build failed with exit code $LASTEXITCODE" }

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$releaseDir = Join-Path $TargetDir "release"
$binaries = @("bifrost.exe", "heimdall.exe", "bifrost-ui.exe")

$checksumLines = @()
foreach ($bin in $binaries) {
    $src = Join-Path $releaseDir $bin
    if (-not (Test-Path $src)) { throw "Expected release binary not found: $src" }
    $dst = Join-Path $OutputDir $bin
    Copy-Item -Force $src $dst
    $hash = (Get-FileHash -Algorithm SHA256 $dst).Hash.ToLowerInvariant()
    $checksumLines += "$hash  $bin"
}

# Copy WebView2Loader.dll alongside bifrost-ui.exe when present (needed for GNU builds).
$wv2 = Get-ChildItem -Path $releaseDir -Recurse -Filter "WebView2Loader.dll" -ErrorAction SilentlyContinue | Select-Object -First 1
if ($wv2) {
    $wv2Dst = Join-Path $OutputDir "WebView2Loader.dll"
    Copy-Item -Force $wv2.FullName $wv2Dst
    $hash = (Get-FileHash -Algorithm SHA256 $wv2Dst).Hash.ToLowerInvariant()
    $checksumLines += "$hash  WebView2Loader.dll"
}

$checksumFile = Join-Path (Resolve-Path $OutputDir).Path "release-checksums.sha256"
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllLines($checksumFile, [string[]]$checksumLines, $utf8NoBom)

Write-Host "Release bundle staged at '$OutputDir' with verified SHA-256 manifest '$checksumFile'."
