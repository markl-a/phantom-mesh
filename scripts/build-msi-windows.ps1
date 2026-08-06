# Spectyn Mesh - Windows MSI / NSIS installer build (Tauri bundle)
#
# Wraps `tauri build --bundles <fmt>` with the node-a Windows gotchas:
#
#   1. Stops any running spectyn.exe / app exe first - Windows refuses to
#      overwrite a binary held open by Defender real-time scan or an active
#      process during the link / bundle step.
#   2. Sets CARGO_TARGET_DIR outside the worktree (default
#      D:\tmp\spectyn-windows-target) so Defender's real-time scan does not
#      lock newly-emitted build-script-build.exe files mid-build
#      (surfaces as os error 5 / access denied otherwise).
#   3. Runs `npm install` only if node_modules is missing (idempotent).
#   4. Optional -Sign: after bundling, calls scripts\codesign-windows.ps1
#      (task-2026052622 dev self-signed cert) on the produced installer.
#
# Usage:
#   .\scripts\build-msi-windows.ps1                  # build MSI bundle
#   .\scripts\build-msi-windows.ps1 -Format nsis     # build NSIS .exe instead
#   .\scripts\build-msi-windows.ps1 -Sign            # build MSI + dev-sign it
#   .\scripts\build-msi-windows.ps1 -SkipNpmInstall  # assume node_modules present
#   .\scripts\build-msi-windows.ps1 -TargetDir D:\x  # override CARGO_TARGET_DIR
#
# Pure ASCII for PowerShell 5.1 on CP950 / CP932 / CP949 locales.
# Exit code mirrors the tauri build step.

[CmdletBinding()]
param(
    [ValidateSet('msi', 'nsis')]
    [string]$Format = 'msi',

    [switch]$Sign,
    [switch]$SkipNpmInstall,
    [string]$TargetDir = 'D:\tmp\spectyn-windows-target'
)

# NOT 'Stop'. Windows PowerShell 5.1 wraps every native-command stderr line
# (npm/npx/cargo emit warnings to stderr - e.g. npm's "Unknown project config"
# note) as a terminating NativeCommandError under 'Stop', which would abort the
# build at the first npm warning. We gate on $LASTEXITCODE explicitly instead,
# and `throw` still terminates regardless of this preference.
$ErrorActionPreference = 'Continue'

# ---- Resolve repo paths ----
$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$AppDir   = Join-Path $RepoRoot 'app'
$TauriDir = Join-Path $AppDir 'src-tauri'
if (-not (Test-Path (Join-Path $TauriDir 'tauri.conf.json'))) {
    throw "Cannot find app/src-tauri/tauri.conf.json at $TauriDir - is the script in <repo>/scripts/?"
}

Write-Host "=== Spectyn Mesh Windows installer build ===" -ForegroundColor Cyan
Write-Host ""
Write-Host "  repo        : $RepoRoot"
Write-Host "  app         : $AppDir"
Write-Host "  format      : $Format"
Write-Host "  CARGO_TARGET: $TargetDir"
Write-Host "  sign        : $($Sign.IsPresent)"
Write-Host ""

# ---- Toolchain probe ----
foreach ($tool in @('node', 'npm', 'cargo')) {
    $found = Get-Command $tool -ErrorAction SilentlyContinue
    if (-not $found) { throw "Required tool '$tool' not on PATH. Install it first." }
}
Write-Host "[probe] node $(node --version), cargo present" -ForegroundColor DarkGray

# ---- Stop running spectyn / app exe so the link/bundle step does not hit access-denied ----
foreach ($procName in @('spectyn', 'spectyn-mesh-app', 'spectyn-mesh')) {
    $running = Get-Process $procName -ErrorAction SilentlyContinue
    if ($running) {
        Write-Host "[pre] Stopping $($running.Count) running $procName instance(s)..." -ForegroundColor Yellow
        $running | Stop-Process -Force -ErrorAction SilentlyContinue
    }
}
Start-Sleep -Seconds 1

# ---- npm install (idempotent) ----
# Always run unless -SkipNpmInstall. Do NOT skip just because node_modules
# exists: a node_modules dir can be stale relative to package.json (e.g. a
# newly-declared @tauri-apps/plugin-* that was never installed), which makes
# the frontend `tsc` build fail with TS2307 "Cannot find module" and aborts
# the whole tauri build. npm install is a fast no-op when already in sync.
if (-not $SkipNpmInstall) {
    Write-Host "[deps] npm install (sync node_modules to package.json)" -ForegroundColor Cyan
    Push-Location $AppDir
    try {
        npm install
        if ($LASTEXITCODE -ne 0) { throw "npm install failed (exit $LASTEXITCODE)" }
    } finally { Pop-Location }
} else {
    Write-Host "[deps] -SkipNpmInstall passed - assuming node_modules in sync" -ForegroundColor DarkGray
}

# ---- Build the bundle ----
$env:CARGO_TARGET_DIR = $TargetDir
Write-Host ""
Write-Host "[build] npx tauri build --bundles $Format" -ForegroundColor Cyan
Push-Location $AppDir
try {
    npx tauri build --bundles $Format
    if ($LASTEXITCODE -ne 0) { throw "tauri build failed (exit $LASTEXITCODE)" }
} finally { Pop-Location }

# ---- Locate the produced installer ----
$bundleDir = Join-Path $TargetDir "release\bundle\$Format"
if (-not (Test-Path $bundleDir)) {
    throw "Bundle dir missing: $bundleDir - did the bundle target change? Check tauri.conf.json bundle.targets."
}
$pattern = if ($Format -eq 'msi') { '*.msi' } else { '*-setup.exe' }
$installer = Get-ChildItem $bundleDir -Filter $pattern -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $installer) {
    # NSIS sometimes emits just *.exe; widen the net
    $installer = Get-ChildItem $bundleDir -Filter '*.exe' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
}
if (-not $installer) { throw "Built but no installer found in $bundleDir" }

$sizeMB = [math]::Round($installer.Length / 1MB, 1)
Write-Host ""
Write-Host "  [+] $($installer.FullName) ($sizeMB MB)" -ForegroundColor Green

# ---- Optional dev-sign ----
if ($Sign) {
    $codesign = Join-Path $PSScriptRoot 'codesign-windows.ps1'
    if (-not (Test-Path $codesign)) {
        Write-Host "[sign] WARN: scripts\codesign-windows.ps1 not found - skipping sign" -ForegroundColor Yellow
    } else {
        Write-Host ""
        Write-Host "[sign] dev-signing $($installer.Name) via codesign-windows.ps1" -ForegroundColor Cyan
        & $codesign -Path $installer.FullName
    }
}

Write-Host ""
Write-Host "Done. Installer: $($installer.FullName)" -ForegroundColor Cyan
