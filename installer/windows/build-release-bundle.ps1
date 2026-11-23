<#
.SYNOPSIS
    Builds a BIFRÖST release bundle for Windows 11 x64, optionally Authenticode-signs the
    binaries, and emits SHA-256 checksums (and an optional versioned .zip).

.DESCRIPTION
    Signing happens BEFORE hashing so release-checksums.sha256 matches the signed files.
    Signing is enabled when -SignCertificatePath points to a .pfx file; its password is
    read from the BIFROST_SIGN_PFX_PASSWORD environment variable (never a parameter, so it
    does not end up in shell history or process listings). Without a certificate the
    bundle is built UNSIGNED and a warning is printed.

.EXAMPLE
    .\installer\windows\build-release-bundle.ps1 -OutputDir .\dist\bifrost-win64 -Zip
#>
[CmdletBinding()]
param(
    [string]$OutputDir = ".\dist\bifrost-win64",
    [string]$TargetDir = $env:CARGO_TARGET_DIR,
    [string]$SignCertificatePath = $env:BIFROST_SIGN_PFX_PATH,
    [string]$TimestampServer = "http://timestamp.digicert.com",
    [switch]$Zip,
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
$repoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")

if ([string]::IsNullOrWhiteSpace($TargetDir)) {
    $TargetDir = Join-Path $env:TEMP "bifrost-target"
}
$env:CARGO_TARGET_DIR = $TargetDir

# Workspace version from the root Cargo.toml ([workspace.package] version = "x.y.z").
$cargoToml = Get-Content (Join-Path $repoRoot "Cargo.toml") -Raw
if ($cargoToml -notmatch '(?ms)^\[workspace\.package\].*?^version\s*=\s*"([^"]+)"') {
    throw "Unable to read [workspace.package] version from Cargo.toml"
}
$version = $Matches[1]
Write-Host "BIFRÖST version $version"

if (-not $SkipBuild) {
    Write-Host "Building BIFRÖST release binaries (mimir, heimdall, bifrost-ui)..."
    cargo build --release --locked -p mimir -p heimdall -p bifrost-ui
    if ($LASTEXITCODE -ne 0) { throw "Release build failed with exit code $LASTEXITCODE" }
}

if (Test-Path $OutputDir) {
    # Only wipe a directory that is empty or clearly a previous BIFRÖST bundle.
    $existing = @(Get-ChildItem -Force $OutputDir)
    $isBundle = Test-Path (Join-Path $OutputDir "release-checksums.sha256")
    if ($existing.Count -gt 0 -and -not $isBundle) {
        throw "Refusing to overwrite non-empty directory '$OutputDir' that is not a previous release bundle."
    }
    Remove-Item -Recurse -Force $OutputDir
}
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$releaseDir = Join-Path $TargetDir "release"

$binaries = @("bifrost.exe", "heimdall.exe", "bifrost-ui.exe")
foreach ($bin in $binaries) {
    $src = Join-Path $releaseDir $bin
    if (-not (Test-Path $src)) { throw "Expected release binary not found: $src" }
    Copy-Item -Force $src (Join-Path $OutputDir $bin)
}

# WebView2Loader.dll is only produced by GNU builds; MSVC builds link it statically.
$wv2 = Get-ChildItem -Path $releaseDir -Recurse -Filter "WebView2Loader.dll" -ErrorAction SilentlyContinue | Select-Object -First 1
$payload = [System.Collections.Generic.List[string]]::new()
$payload.AddRange([string[]]$binaries)
if ($wv2) {
    Copy-Item -Force $wv2.FullName (Join-Path $OutputDir "WebView2Loader.dll")
    $payload.Add("WebView2Loader.dll")
}

# Installer scripts and license travel with every bundle.
foreach ($extra in @("installer\windows\install-bifrost.ps1", "installer\windows\uninstall-bifrost.ps1", "LICENSE")) {
    $src = Join-Path $repoRoot $extra
    $name = Split-Path $src -Leaf
    Copy-Item -Force $src (Join-Path $OutputDir $name)
    $payload.Add($name)
}

# Optional Authenticode signing of executables and scripts (before hashing).
$signed = $false
if (-not [string]::IsNullOrWhiteSpace($SignCertificatePath)) {
    if (-not (Test-Path $SignCertificatePath)) { throw "Signing certificate not found: $SignCertificatePath" }
    $password = $env:BIFROST_SIGN_PFX_PASSWORD
    $flags = [System.Security.Cryptography.X509Certificates.X509KeyStorageFlags]::Exportable
    $cert = New-Object System.Security.Cryptography.X509Certificates.X509Certificate2($SignCertificatePath, $password, $flags)
    if (-not $cert.HasPrivateKey) { throw "Signing certificate has no private key: $SignCertificatePath" }
    Write-Host "Authenticode-signing with '$($cert.Subject)' (thumbprint $($cert.Thumbprint))..."
    foreach ($name in $payload) {
        # Only sign files BIFRÖST produces; WebView2Loader.dll keeps Microsoft's signature.
        if ($name -notmatch '\.(exe|ps1)$') { continue }
        $file = Join-Path $OutputDir $name
        $params = @{ FilePath = $file; Certificate = $cert; HashAlgorithm = 'SHA256' }
        if (-not [string]::IsNullOrWhiteSpace($TimestampServer)) { $params.TimestampServer = $TimestampServer }
        $sig = Set-AuthenticodeSignature @params
        if ($null -eq $sig.SignerCertificate) {
            throw "Authenticode signing failed for ${name}: $($sig.StatusMessage)"
        }
        Write-Host "  signed $name ($($sig.Status))"
    }
    $signed = $true
} else {
    Write-Warning "No signing certificate provided: bundle binaries are NOT Authenticode-signed (SmartScreen will warn users)."
}

$checksumLines = foreach ($name in $payload) {
    $hash = (Get-FileHash -Algorithm SHA256 (Join-Path $OutputDir $name)).Hash.ToLowerInvariant()
    "$hash  $name"
}
$checksumFile = Join-Path (Resolve-Path $OutputDir).Path "release-checksums.sha256"
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllLines($checksumFile, [string[]]$checksumLines, $utf8NoBom)
Write-Host "SHA-256 manifest written to '$checksumFile'."

if ($Zip) {
    $zipPath = Join-Path (Split-Path (Resolve-Path $OutputDir).Path -Parent) "bifrost-v$version-win64.zip"
    if (Test-Path $zipPath) { Remove-Item -Force $zipPath }
    Compress-Archive -Path (Join-Path $OutputDir "*") -DestinationPath $zipPath
    $zipHash = (Get-FileHash -Algorithm SHA256 $zipPath).Hash.ToLowerInvariant()
    [System.IO.File]::WriteAllText("$zipPath.sha256", "$zipHash  $(Split-Path $zipPath -Leaf)`n", $utf8NoBom)
    Write-Host "Release archive: $zipPath (sha256 $zipHash)"
}

Write-Host ("Release bundle staged at '{0}' ({1})." -f $OutputDir, $(if ($signed) { "Authenticode-signed" } else { "UNSIGNED" }))
