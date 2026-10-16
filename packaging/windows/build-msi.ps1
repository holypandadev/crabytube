<#
.SYNOPSIS
Builds the Windows Installer package of CrabyTube, CrabyTube-<version>-windows-x64.msi.

.DESCRIPTION
Packs the folder build-zip.ps1 makes (target\windows\CrabyTube) with crabytube.wxs. Run build-zip.ps1
first. Needs the WiX Toolset 5 command-line tool:

  dotnet tool install --global wix --version 5.0.2
  powershell -ExecutionPolicy Bypass -File packaging\windows\build-msi.ps1

The package is in target\windows\. Nothing is signed.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$here = $PSScriptRoot
$out = Join-Path $root 'target\windows'
$stage = Join-Path $out 'CrabyTube'

if (-not (Test-Path (Join-Path $stage 'CrabyTube.exe'))) {
    throw 'target\windows\CrabyTube is missing. Run packaging\windows\build-zip.ps1 first.'
}
if (-not (Get-Command wix -ErrorAction SilentlyContinue)) {
    throw "wix was not found. Install it with 'dotnet tool install --global wix --version 5.0.2'."
}

$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.*)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) {
    throw 'Could not read the version from Cargo.toml.'
}

$msi = Join-Path $out "CrabyTube-$version-windows-x64.msi"
if (Test-Path $msi) { Remove-Item $msi -Force }
& wix build (Join-Path $here 'crabytube.wxs') -arch x64 `
    -d "Version=$version" `
    -d "Stage=$stage" `
    -d "IconPath=$(Join-Path $here 'crabytube.ico')" `
    -o $msi
if ($LASTEXITCODE -ne 0) {
    throw "wix build failed with exit code $LASTEXITCODE."
}

$hash = (Get-FileHash $msi -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Host ''
Write-Host "CrabyTube $version, Windows Installer package"
Write-Host "Package: $msi ($((Get-Item $msi).Length) bytes)"
Write-Host "SHA-256: $hash"
