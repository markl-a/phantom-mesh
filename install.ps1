#!/usr/bin/env pwsh
<#
.SYNOPSIS
    Build-from-source installer for the `phantom` CLI (Phantom Mesh) on Windows.

.DESCRIPTION
    Builds the optimized `phantom` binary from source with cargo and installs it
    to a per-user bin directory (no admin required). Idempotent: re-running
    overwrites the installed binary cleanly.

    This is the SOURCE-BUILD installer and therefore REQUIRES a Rust toolchain
    (cargo). The hosted prebuilt-download one-liner advertised in older docs
    (iwr .../install.ps1 | iex) is a FUTURE release-pipeline item and is not
    wired up yet — use this script from a checkout instead.

.PARAMETER Prefix
    Install root. The binary lands in "<Prefix>\bin\phantom.exe".
    Default: "$env:LOCALAPPDATA\phantom-mesh".

.EXAMPLE
    pwsh -File install.ps1
    # builds + installs to $env:LOCALAPPDATA\phantom-mesh\bin

.EXAMPLE
    pwsh -File install.ps1 -Prefix C:\tmp\phantom-test
    # builds + installs to C:\tmp\phantom-test\bin (handy for testing)
#>
[CmdletBinding()]
param(
    [string]$Prefix = (Join-Path $env:LOCALAPPDATA 'phantom-mesh')
)

$ErrorActionPreference = 'Stop'

function Write-Step($msg) { Write-Host "==> $msg" -ForegroundColor Cyan }
function Write-Ok($msg)   { Write-Host "    $msg" -ForegroundColor Green }

# Repo layout: this script lives at the repo root; the Rust crate is in core/.
$RepoRoot = $PSScriptRoot
$CrateDir = Join-Path $RepoRoot 'core'
$Manifest = Join-Path $CrateDir 'Cargo.toml'

if (-not (Test-Path $Manifest)) {
    Write-Error "Cannot find core\Cargo.toml next to install.ps1 (looked in '$CrateDir'). Run this from a phantom-mesh checkout."
    exit 1
}

# 1. Toolchain check ---------------------------------------------------------
Write-Step 'Checking for the Rust toolchain (cargo)...'
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (-not $cargo) {
    Write-Host ''
    Write-Error @"
cargo (the Rust toolchain) was not found on PATH.

This is the build-from-source installer, so Rust is required. Install it via rustup:

    https://rustup.rs
    # or, with winget:
    winget install Rustlang.Rustup

Then restart your shell (so cargo is on PATH) and re-run this script.
"@
    exit 1
}
Write-Ok "found cargo: $($cargo.Source)"

# 2. Build the optimized binary from source ----------------------------------
Write-Step 'Building phantom (cargo build --release --bin phantom) — this is slow on a cold build, please wait...'

# Best-effort: stamp the real commit into the binary so `phantom --version`
# reports provenance instead of "nogit". Never fatal if git is unavailable.
$gitHash = $null
try {
    $gitHash = (& git -C $RepoRoot rev-parse --short HEAD 2>$null)
    if ($LASTEXITCODE -ne 0) { $gitHash = $null }
} catch { $gitHash = $null }
if ($gitHash) { $env:PHANTOM_GIT_HASH = $gitHash.Trim() }

Push-Location $CrateDir
try {
    & cargo build --release --bin phantom
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

$SrcBin = Join-Path $CrateDir 'target\release\phantom.exe'
if (-not (Test-Path $SrcBin)) {
    Write-Error "Build reported success but '$SrcBin' is missing. Aborting."
    exit 1
}
Write-Ok "built: $SrcBin"

# 3. Install to the per-user bin dir (idempotent) ----------------------------
$BinDir = Join-Path $Prefix 'bin'
Write-Step "Installing to $BinDir ..."
New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
$DestBin = Join-Path $BinDir 'phantom.exe'
Copy-Item -Path $SrcBin -Destination $DestBin -Force
Write-Ok "installed: $DestBin"

# 4. Create the data dir if absent -------------------------------------------
$DataDir = Join-Path $env:USERPROFILE '.phantom-mesh'
Write-Step "Ensuring data dir $DataDir ..."
New-Item -ItemType Directory -Force -Path $DataDir | Out-Null
Write-Ok "data dir ready: $DataDir"

# 5. Next steps --------------------------------------------------------------
Write-Host ''
Write-Host 'phantom installed.' -ForegroundColor Green
Write-Host ''
Write-Host 'Add the bin dir to your PATH for this user (persists across sessions):'
Write-Host "    [Environment]::SetEnvironmentVariable('Path', `"`$env:Path;$BinDir`", 'User')" -ForegroundColor Yellow
Write-Host ''
Write-Host 'Or for the current session only:'
Write-Host "    `$env:Path += ';$BinDir'" -ForegroundColor Yellow
Write-Host ''
Write-Host 'Then verify and start the daemon:'
Write-Host '    phantom --version' -ForegroundColor Yellow
Write-Host '    phantom --help' -ForegroundColor Yellow
Write-Host '    phantom serve' -ForegroundColor Yellow
