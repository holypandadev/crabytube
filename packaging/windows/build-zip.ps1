<#
.SYNOPSIS
Builds CrabyTube for Windows x64 and packs it into CrabyTube-windows-x64.zip.

.DESCRIPTION
Compiles the three programs of the workspace in release mode, then puts them in one folder with
the first-launch notes, the license and the third-party notices, and zips that folder (the files
sit at the top level of the zip). The executables carry the application icon and their version
information, which the build embeds from packaging/windows/crabytube.ico. Nothing is signed.

  powershell -ExecutionPolicy Bypass -File packaging\windows\build-zip.ps1

Needs Rust (stable, MSVC toolchain) and the Visual Studio C++ build tools on an x64 Windows PC.
The result is in target\windows\:
  CrabyTube\                    the folder that goes into the zip
  CrabyTube-windows-x64.zip

.PARAMETER NoBuild
Packs the executables already in target\release instead of building them first.
#>
[CmdletBinding()]
param(
    [switch]$NoBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$here = $PSScriptRoot
$release = Join-Path $root 'target\release'
$out = Join-Path $root 'target\windows'
$stage = Join-Path $out 'CrabyTube'
$zip = Join-Path $out 'CrabyTube-windows-x64.zip'

# The application is shipped as CrabyTube.exe; the two helpers keep their names, which the
# application looks up next to itself.
$programs = [ordered]@{
    'crabytube.exe'           = 'CrabyTube.exe'
    'botguard-helper.exe'     = 'botguard-helper.exe'
    'js-challenge-helper.exe' = 'js-challenge-helper.exe'
}

function Invoke-Native {
    param([string]$Command, [string[]]$Arguments)
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "'$Command $($Arguments -join ' ')' failed with exit code $LASTEXITCODE."
    }
}

# Writes a text file with Windows line endings, so Notepad shows it as intended.
function Copy-TextFile {
    param([string]$From, [string]$To)
    $text = [IO.File]::ReadAllText($From) -replace "`r`n", "`n" -replace "`n", "`r`n"
    [IO.File]::WriteAllText($To, $text, (New-Object Text.UTF8Encoding $false))
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw 'cargo was not found. Install Rust from https://rustup.rs and open a new terminal.'
}

$hostLine = (& rustc -vV) | Where-Object { $_ -like 'host:*' }
if ($hostLine -ne 'host: x86_64-pc-windows-msvc') {
    throw "This script builds with the x86_64-pc-windows-msvc toolchain, but the active one is '$hostLine'."
}

$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.*)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) {
    throw 'Could not read the version from Cargo.toml.'
}

if (-not $NoBuild) {
    Write-Host 'Building the release executables...'
    Push-Location $root
    try {
        Invoke-Native 'cargo' @('build', '--release', '--locked')
    }
    finally {
        Pop-Location
    }
}

foreach ($name in $programs.Keys) {
    if (-not (Test-Path (Join-Path $release $name))) {
        throw "target\release\$name is missing. Build without -NoBuild."
    }
}

if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
if (Test-Path $zip) { Remove-Item $zip -Force }
New-Item -ItemType Directory -Force $stage | Out-Null

foreach ($name in $programs.Keys) {
    Copy-Item (Join-Path $release $name) (Join-Path $stage $programs[$name])
}
Copy-TextFile (Join-Path $here 'README-first-launch.txt') (Join-Path $stage 'README-first-launch.txt')
Copy-TextFile (Join-Path $root 'LICENSE') (Join-Path $stage 'LICENSE.txt')
Copy-TextFile (Join-Path $root 'docs\licenses.md') (Join-Path $stage 'THIRD-PARTY-LICENSES.md')

Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::CreateFromDirectory($stage, $zip, [IO.Compression.CompressionLevel]::Optimal, $false)

$hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
Write-Host ''
Write-Host "CrabyTube $version, Windows x64"
Write-Host 'In the zip:'
$archive = [IO.Compression.ZipFile]::OpenRead($zip)
try {
    foreach ($entry in $archive.Entries) {
        Write-Host ('  {0,-26} {1,12:N0} bytes' -f $entry.FullName, $entry.Length)
    }
}
finally {
    $archive.Dispose()
}
Write-Host "Folder: $stage"
Write-Host "Zip:    $zip"
Write-Host "SHA-256: $hash"
