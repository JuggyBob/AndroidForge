param(
    [switch]$SkipBuild,
    [switch]$SkipInstaller
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$cargo = Get-Content Cargo.toml -Raw
if ($cargo -notmatch 'version = "([^"]+)"') { throw 'workspace version not found in Cargo.toml' }
$version = $Matches[1]
Write-Output "Packaging AndroidForge $version"

if (-not $SkipBuild) {
    cargo build --workspace --release
    if ($LASTEXITCODE -ne 0) { throw 'release build failed' }
}

foreach ($name in @('target\release\androidforge.exe', 'target\release\androidforge-desktop.exe', 'LICENSE')) {
    if (-not (Test-Path $name)) { throw "missing required file: $name" }
}

New-Item -ItemType Directory -Force -Path dist | Out-Null

# CLI-only archive used by the desktop update/repair path.
$cliZip = 'dist\androidforge-cli-windows-x86_64.zip'
if (Test-Path $cliZip) { Remove-Item $cliZip -Force }
Compress-Archive -Path 'target\release\androidforge.exe' -DestinationPath $cliZip -Force

# Portable desktop distribution. Both executables must stay adjacent so the
# desktop can discover the CLI without configuration.
$portable = Join-Path $env:TEMP "androidforge-portable-$version"
if (Test-Path $portable) { Remove-Item -Recurse -Force $portable }
New-Item -ItemType Directory -Force -Path $portable | Out-Null
Copy-Item 'target\release\androidforge.exe' $portable
Copy-Item 'target\release\androidforge-desktop.exe' $portable
foreach ($doc in @('LICENSE', 'README.md', 'USER_GUIDE.md', 'CHANGELOG.md')) {
    if (Test-Path $doc) { Copy-Item $doc $portable }
}
@"
AndroidForge $version — portable Windows x86_64

Run androidforge-desktop.exe for the native cockpit. It locates androidforge.exe,
which must remain beside it. The CLI is authoritative; the desktop is its client.

This package is unsigned. Windows SmartScreen may warn on first launch. AndroidForge
does not include code-signing material or a publisher certificate.

User configuration and artifacts live outside this folder:
  %APPDATA%\AndroidForge
  %LOCALAPPDATA%\AndroidForge
  project .artifacts directories

Use the CLI for automation and the JSON contract. See README.md and USER_GUIDE.md.
"@ | Set-Content (Join-Path $portable 'PORTABLE.txt') -Encoding utf8
$desktopZip = 'dist\androidforge-desktop-windows-x86_64.zip'
if (Test-Path $desktopZip) { Remove-Item $desktopZip -Force }
Compress-Archive -Path (Join-Path $portable '*') -DestinationPath $desktopZip -Force

# Versioned UI screenshot gallery, when present.
$screenshotsDir = "docs\screenshots\v$version"
$screenshotsZip = "dist\androidforge-screenshots-v$version.zip"
if (Test-Path $screenshotsDir) {
    if (Test-Path $screenshotsZip) { Remove-Item $screenshotsZip -Force }
    Compress-Archive -Path (Join-Path $screenshotsDir '*') -DestinationPath $screenshotsZip -Force
}

if (-not $SkipInstaller) {
    $iscc = Get-Command iscc.exe -ErrorAction SilentlyContinue
    if (-not $iscc) {
        $candidates = @(
            "$env:LOCALAPPDATA\Programs\Inno Setup 7\ISCC.exe",
            "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
            "${env:ProgramFiles(x86)}\Inno Setup 7\ISCC.exe",
            "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
            "$env:ProgramFiles\Inno Setup 7\ISCC.exe",
            "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
        )
        $iscc = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
    }
    if ($iscc) {
        & $iscc 'packaging\windows\androidforge.iss'
        if ($LASTEXITCODE -ne 0) { throw 'Inno Setup failed' }
    } else {
        Write-Warning 'Inno Setup (iscc.exe) was not found; installer was not built.'
    }
}

$assets = @(
    $cliZip,
    $desktopZip,
    $screenshotsZip,
    'dist\androidforge-windows-x86_64-setup.exe'
) | Where-Object { Test-Path $_ }

$lines = foreach ($asset in $assets) {
    $hash = (Get-FileHash $asset -Algorithm SHA256).Hash.ToLower()
    "$hash  $(Split-Path $asset -Leaf)"
}
$lines | Set-Content 'dist\SHA256SUMS.txt' -Encoding ascii
Get-Content 'dist\SHA256SUMS.txt'
Get-ChildItem dist | Select-Object Name, Length
