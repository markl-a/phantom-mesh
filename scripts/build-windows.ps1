# Phantom Mesh - Windows native build helper
#
# Wraps `cargo build --release --bin phantom` with the gotchas the
# node-a (Windows + Android-APK build host) session needs:
#
#   1. Stops any running phantom.exe first - Windows refuses to overwrite
#      a .exe held open by Defender real-time scan or by an active
#      `phantom serve` process.
#   2. Sets CARGO_TARGET_DIR outside the worktree (default
#      D:\tmp\phantom-windows-target). The worktree's own
#      `core/target/` lives inside .worktrees/, where Defender's
#      real-time scan repeatedly locks newly-emitted
#      `build-script-build.exe` files and surfaces as
#      `access denied (os error 5)` mid-build.
#   3. Optionally deploys to ~/.phantom-mesh/bin/phantom.exe (the path
#      install-phantom-windows.ps1 + `phantom service install` both
#      use) and ~/.local/bin/phantom.exe (so PowerShell PATH picks it
#      up without further wiring).
#
# Usage:
#   ./scripts/build-windows.ps1                       # build + verify
#   ./scripts/build-windows.ps1 -Deploy               # build + copy to ~/.phantom-mesh/bin + ~/.local/bin
#   ./scripts/build-windows.ps1 -Test                 # build + cargo test --lib --release
#   ./scripts/build-windows.ps1 -TargetDir D:\custom  # override CARGO_TARGET_DIR
#
# Pure ASCII so Windows PowerShell 5.1 parses it on CP950 / CP932 / CP949
# locale machines (where the default file encoding is not UTF-8). Don't add
# box-drawing glyphs, checkmarks, em-dashes, or CJK unless you also save with
# a UTF-8 BOM - otherwise the parser chokes mid-file and nothing runs.
#
# Exit code mirrors cargo's.

param(
    [switch]$Deploy,
    [switch]$Test,
    [string]$TargetDir = 'D:\tmp\phantom-windows-target'
)

# NOT 'Stop'. Under output redirection (CI / background capture), PS 5.1 wraps
# cargo's stderr progress ("Compiling ...") as a terminating NativeCommandError,
# which would abort right after the build line. We gate on $LASTEXITCODE
# explicitly below, and `throw` still terminates regardless of this preference.
$ErrorActionPreference = 'Continue'

# -- Resolve repo paths -------------------------------------------------------
$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$CoreDir  = Join-Path $RepoRoot 'core'
if (-not (Test-Path (Join-Path $CoreDir 'Cargo.toml'))) {
    throw "Cannot find core/Cargo.toml at $CoreDir - is the script in <repo>/scripts/?"
}

Write-Host "=== Phantom Mesh Windows build ===" -ForegroundColor Cyan
Write-Host ""
Write-Host "  repo        : $RepoRoot"
Write-Host "  CARGO_TARGET: $TargetDir"
Write-Host ""

# -- Stop any running phantom.exe so the link step doesn't hit access-denied --
$running = Get-Process phantom -ErrorAction SilentlyContinue
if ($running) {
    Write-Host "[pre] Stopping $($running.Count) running phantom.exe instance(s)..." -ForegroundColor Yellow
    $running | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 2
}

# -- Build --------------------------------------------------------------------
$env:CARGO_TARGET_DIR = $TargetDir
Write-Host "[build] cargo build --release --bin phantom" -ForegroundColor Cyan
Push-Location $CoreDir
try {
    cargo build --release --bin phantom --message-format=short
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit $LASTEXITCODE)" }
} finally {
    Pop-Location
}

$exe = Join-Path $TargetDir 'release\phantom.exe'
if (-not (Test-Path $exe)) { throw "Built but $exe missing - cargo target layout changed?" }
$size = (Get-Item $exe).Length
Write-Host "  [+] $exe ($([math]::Round($size/1MB, 1)) MB)" -ForegroundColor Green

# -- Optional: cargo test --lib --release -------------------------------------
if ($Test) {
    Write-Host ""
    Write-Host "[test] cargo test --lib --release" -ForegroundColor Cyan
    Push-Location $CoreDir
    try {
        cargo test --lib --release --no-fail-fast
        if ($LASTEXITCODE -ne 0) { throw "cargo test failed (exit $LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
    Write-Host "  [+] tests passed" -ForegroundColor Green
}

# -- Optional: deploy ---------------------------------------------------------
if ($Deploy) {
    Write-Host ""
    Write-Host "[deploy] copying to canonical install paths" -ForegroundColor Cyan
    $serveDir = Join-Path $env:USERPROFILE '.phantom-mesh\bin'
    $localDir = Join-Path $env:USERPROFILE '.local\bin'
    New-Item -ItemType Directory -Force $serveDir | Out-Null
    New-Item -ItemType Directory -Force $localDir | Out-Null
    Copy-Item $exe (Join-Path $serveDir 'phantom.exe') -Force
    Copy-Item $exe (Join-Path $localDir 'phantom.exe') -Force
    Write-Host "  [+] $serveDir\phantom.exe (used by PhantomServe Scheduled Task)" -ForegroundColor Green
    Write-Host "  [+] $localDir\phantom.exe (on PowerShell User PATH)" -ForegroundColor Green

    Write-Host ""
    Write-Host "[verify] phantom --version" -ForegroundColor Cyan
    & (Join-Path $localDir 'phantom.exe') --version
}

Write-Host ""
Write-Host "Done." -ForegroundColor Cyan
