# Phantom Mesh - Windows GUI installer build (.msi + .exe), sign + smoke.
#
# Wave H3.x (Windows half, parallels scripts/package-macos.sh + package-linux.sh).
# Builds the Tauri desktop "AI terminal" into Windows installers:
#   - WiX .msi   (per-machine MSI installer)
#   - NSIS .exe  (per-user setup.exe)
# then optionally signs with the dev self-signed cert (reuses
# scripts/codesign-windows.ps1) and copies the artifacts into dist/.
#
# Pure ASCII (no smart quotes, no glyphs) so Windows PowerShell 5.1 on
# CP950 / CP932 / CP949 locales parses it. Same constraint as
# codesign-windows.ps1.
#
# Signing tiers (honest, like the macOS spctl path):
#   - dev self-signed cert -> Authenticode "UnknownError" until the cert is in
#     Trusted Root; structurally signed, fine for local install. Production
#     EV / Authenticode cert + timestamp server DEFERRED.
#
# Usage:
#   .\scripts\package-windows.ps1                 # build msi+exe, no sign
#   .\scripts\package-windows.ps1 -Sign           # build + dev-sign + verify
#   .\scripts\package-windows.ps1 -NoBuild        # reuse existing bundle
#   .\scripts\package-windows.ps1 -TargetDir D:\custom
#   .\scripts\package-windows.ps1 -OutDir D:\out
#
# Exit code mirrors the build; bad args -> 2.

param(
    [switch]$Sign,
    [switch]$NoBuild,
    [string]$TargetDir = 'D:\tmp\phantom-win-app-target',
    [string]$OutDir = ''
)

$ErrorActionPreference = 'Stop'

# ---------- paths ----------
$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
$AppDir   = Join-Path $RepoRoot 'app'
$CoreToml = Join-Path $RepoRoot 'core\Cargo.toml'
if (-not (Test-Path (Join-Path $AppDir 'src-tauri\tauri.conf.json'))) {
    throw "Cannot find app/src-tauri/tauri.conf.json - is the script in <repo>/scripts/?"
}
if (-not $OutDir) { $OutDir = Join-Path $RepoRoot 'dist' }

Write-Host "=== Phantom Mesh Windows installer build ===" -ForegroundColor Cyan
Write-Host "  repo        : $RepoRoot"
Write-Host "  CARGO_TARGET: $TargetDir"
Write-Host "  out         : $OutDir"
Write-Host "  sign        : $Sign"
Write-Host ""

# ---------- 1. build the Tauri Windows bundle ----------
if (-not $NoBuild) {
    # Stop any running phantom so the link/copy step does not hit os error 5.
    $running = Get-Process phantom -ErrorAction SilentlyContinue
    if ($running) {
        Write-Host "[pre] stopping $($running.Count) running phantom process(es)" -ForegroundColor Yellow
        $running | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
    }
    # Target dir outside the worktree dodges Defender real-time scan locks on
    # freshly-emitted build artifacts (see build-windows.ps1).
    $env:CARGO_TARGET_DIR = $TargetDir
    Write-Host "[1/4] npm install + tauri build (msi + nsis)" -ForegroundColor Cyan
    Push-Location $AppDir
    try {
        if (-not (Test-Path (Join-Path $AppDir 'node_modules'))) {
            & npm install
            if ($LASTEXITCODE -ne 0) { throw "npm install failed (exit $LASTEXITCODE)" }
        }
        & npm run tauri build -- --bundles msi,nsis
        if ($LASTEXITCODE -ne 0) { throw "tauri build failed (exit $LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
} else {
    Write-Host "[1/4] skip build (-NoBuild)"
    $env:CARGO_TARGET_DIR = $TargetDir
}

# ---------- 2. locate artifacts ----------
Write-Host "[2/4] locate installer artifacts" -ForegroundColor Cyan
$BundleDir = Join-Path $TargetDir 'release\bundle'
$msi = Get-ChildItem (Join-Path $BundleDir 'msi')  -Filter '*.msi'       -ErrorAction SilentlyContinue | Select-Object -First 1
$exe = Get-ChildItem (Join-Path $BundleDir 'nsis') -Filter '*-setup.exe' -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $msi -and -not $exe) {
    throw "no .msi or *-setup.exe under $BundleDir (build failed or different target dir?)"
}
if ($msi) { Write-Host ("  msi : " + $msi.FullName) }
if ($exe) { Write-Host ("  exe : " + $exe.FullName) }

# ---------- 3. optional dev signing (reuse codesign-windows.ps1) ----------
if ($Sign) {
    Write-Host "[3/4] sign installers (dev self-signed cert)" -ForegroundColor Cyan
    $signer = Join-Path $PSScriptRoot 'codesign-windows.ps1'
    & $signer -CreateCert | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "dev cert creation failed (exit $LASTEXITCODE) -- refusing to ship unsigned installers" }
    foreach ($art in @($msi, $exe)) {
        if ($art) {
            & $signer -Path $art.FullName
            # Don't silently ship an unsigned installer if signing failed (T-WLA-17).
            if ($LASTEXITCODE -ne 0) { throw "signing failed for $($art.Name) (exit $LASTEXITCODE)" }
            Write-Host "  -- Authenticode status --"
            (Get-AuthenticodeSignature $art.FullName) |
                Format-List Status, SignerCertificate | Out-String | Write-Host
        }
    }
    Write-Host "  Note: dev cert -> Gatekeeper-equivalent (SmartScreen) will warn until an" -ForegroundColor Yellow
    Write-Host "        EV/Authenticode cert + timestamp server land (deferred)." -ForegroundColor Yellow
} else {
    Write-Host "[3/4] skip signing (-Sign not given) - artifacts are unsigned"
}

# ---------- 4. publish to dist/ ----------
Write-Host "[4/4] copy to $OutDir" -ForegroundColor Cyan
New-Item -ItemType Directory -Force $OutDir | Out-Null
foreach ($art in @($msi, $exe)) {
    if ($art) {
        Copy-Item $art.FullName $OutDir -Force
        $mb = [math]::Round((Get-Item $art.FullName).Length / 1MB, 1)
        Write-Host ("  copied " + $art.Name + " ($mb MB)") -ForegroundColor Green
    }
}

Write-Host ""
Write-Host "Done - Windows installers ready in $OutDir" -ForegroundColor Cyan
if (-not $Sign) { Write-Host "  (run with -Sign to attach the dev code-signing cert)" }
