# scripts/install.ps1 — F500 unified first-touch installer for spectyn-mesh.
#
# This is the "stranger-friendly" Windows entry point served at
#   https://phantommesh.io/install.ps1
#
# Use (from any PowerShell — no admin required):
#   irm https://phantommesh.io/install.ps1 | iex
#
# If execution policy blocks the iex form, the user can:
#   Set-ExecutionPolicy -Scope Process Bypass -Force
#   irm https://phantommesh.io/install.ps1 | iex
#
# What it does (no questions asked):
#   1. Detects Windows arch (x86_64 only at v0.6.0).
#   2. Downloads spectyn.exe from
#        $base/dist/spectyn-x86_64-pc-windows.exe
#      (matches publish-spectyn-binary.yml's R2 object naming.)
#   3. Verifies SHA256 via the shared _verify-download.ps1 helper
#      (the F-CRIT-3 / PR #111 contract).
#   4. Installs to $env:USERPROFILE\.spectyn-mesh\bin\spectyn.exe.
#   5. Best-effort: appends ...\bin to the user's PATH if not already
#      there. Never breaks the install if the PATH edit fails.
#   6. Prints exactly one final line: `Run \`spectyn\` to start.`
#
# What it does NOT do:
#   - Require admin (User-scope env edits only; install path is per-user).
#   - Touch agents.toml — the wizard (F501) owns provider config.
#   - Register a Scheduled Task — the wizard offers, user opts in.
#
# Env knobs (PowerShell sets these via $env:NAME = 'value'):
#   SPECTYN_INSTALL_BASE      Base URL. Default: https://phantommesh.io.
#                             Set to a staging URL for pre-L1 testing.
#   SPECTYN_INSTALL_DRY_RUN   If '1': print detected arch + would-download
#                             URL and exit 0 without writing anything.
#   SPECTYN_ALLOW_INSECURE    See _verify-download.ps1 — opt out of HTTPS.
#   SPECTYN_SKIP_VERIFY       See _verify-download.ps1 — opt out of SHA256.
#
# F-CRIT-3 invariants preserved:
#   - Require-Https refuses plain http:// downloads.
#   - SHA256 sidecar verified BEFORE Unblock-File or move into PATH.
#   - Verify-Sha256 deletes the .exe on mismatch and throws.
#
# Compatibility: Windows 10/11, PowerShell 5.1+ (the OS-bundled engine).

$ErrorActionPreference = 'Stop'

# ── Config ──────────────────────────────────────────────────────────────────
$installBase = if ($env:SPECTYN_INSTALL_BASE) { $env:SPECTYN_INSTALL_BASE } else { 'https://phantommesh.io' }
# Strip trailing slash for clean concatenation.
$installBase = $installBase.TrimEnd('/')
$distBase    = "$installBase/dist"

$cfgDir     = Join-Path $env:USERPROFILE '.spectyn-mesh'
$installDir = Join-Path $cfgDir          'bin'
$targetBin  = Join-Path $installDir      'spectyn.exe'

$dryRun = ($env:SPECTYN_INSTALL_DRY_RUN -eq '1')

# Force TLS 1.2+ on PS 5.1 — System.Net.SecurityProtocolType.SystemDefault
# in PS 5.1 still defaults to SSL3/TLS 1.0 on some Windows 10 builds.
try {
    [System.Net.ServicePointManager]::SecurityProtocol = `
        [System.Net.SecurityProtocolType]::Tls12 -bor `
        [System.Net.SecurityProtocolType]::Tls11
} catch { }

# ── Detect arch ─────────────────────────────────────────────────────────────
function Detect-Target {
    $arch = $env:PROCESSOR_ARCHITECTURE
    if (-not $arch) { $arch = 'UNKNOWN' }
    switch -Regex ($arch) {
        '^AMD64$|^x86_64$' {
            $script:archKind  = 'x86_64'
            $script:r2Object  = 'spectyn-x86_64-pc-windows.exe'
        }
        '^ARM64$' {
            throw "Windows on ARM is not yet supported by F500 (v0.6.0).`n  Watch docs/superpowers/features/F500-unified-install-one-liner.md."
        }
        default {
            throw "Unsupported processor architecture: $arch"
        }
    }
}

# ── Load shared verifier helper (F-CRIT-3) ─────────────────────────────────
# Need Require-Https + Verify-Sha256 from _verify-download.ps1. Prefer a
# local copy if we're running from a checkout; otherwise fetch from the
# same base URL we trust for the binary. Fail-closed.
function Load-Verifier {
    $scriptDir = $null
    try {
        if ($MyInvocation.MyCommand.Path) {
            $scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
        }
    } catch { }

    if ($scriptDir -and (Test-Path (Join-Path $scriptDir '_verify-download.ps1'))) {
        . (Join-Path $scriptDir '_verify-download.ps1')
        return
    }

    $helperUrl = "$installBase/scripts/_verify-download.ps1"
    $helperFile = [System.IO.Path]::GetTempFileName()
    try {
        Invoke-WebRequest -Uri $helperUrl `
                          -OutFile $helperFile `
                          -UseBasicParsing `
                          -TimeoutSec 10 `
                          -Headers @{ 'User-Agent' = 'spectyn-installer/1.0' } | Out-Null
    } catch {
        Remove-Item -Force $helperFile -ErrorAction SilentlyContinue
        throw @"
Could not load $helperUrl ($_).
  Refusing to download a binary without the verifier.
  If the R2 publish step has not run yet, see:
    docs/superpowers/runbooks/L1-cloudflare-creds.md
"@
    }
    . $helperFile
    Remove-Item -Force $helperFile -ErrorAction SilentlyContinue
}

# ── Friendly 404 / network failure handling ────────────────────────────────
function Fail-MissingBinary {
    param([string]$Url, [string]$Inner)
    throw @"
Could not download $Url
  ($Inner)

  Most likely cause: the operator has not yet published this target to
  the R2 bucket. The L1 'Publish spectyn binary to R2' workflow needs
  to run for $script:r2Object.

  If you ARE the operator: follow
    docs/superpowers/runbooks/L1-cloudflare-creds.md
  to add CLOUDFLARE_API_TOKEN + CLOUDFLARE_ACCOUNT_ID secrets and trigger
  https://github.com/markl-a/spectyn-mesh/actions/workflows/publish-spectyn-binary.yml

  In the meantime you can build from source:
    cargo install --git https://github.com/markl-a/spectyn-mesh --bin spectyn
"@
}

# ── Best-effort PATH wiring ────────────────────────────────────────────────
# Adds $installDir to the User-scope PATH if not already present. Never
# throws — install is already complete by this point.
function Wire-Path {
    param([string]$Dir)
    try {
        $userPath = [Environment]::GetEnvironmentVariable('PATH', 'User')
        if (-not $userPath) { $userPath = '' }
        $segments = $userPath -split ';' | Where-Object { $_ -ne '' }
        if ($segments -notcontains $Dir) {
            $newPath = if ($userPath) { "$userPath;$Dir" } else { $Dir }
            [Environment]::SetEnvironmentVariable('PATH', $newPath, 'User')
            Write-Host "  added $Dir to User PATH (open a new shell to pick it up)"
        }
    } catch {
        Write-Warning "Could not edit User PATH ($_)"
        Write-Warning "  Add manually:  [Environment]::SetEnvironmentVariable('PATH', `"`$env:PATH;$Dir`", 'User')"
    }
}

# ── Main ───────────────────────────────────────────────────────────────────
Detect-Target

$binUrl = "$distBase/$script:r2Object"

if ($dryRun) {
    Write-Host "spectyn-mesh installer (dry run)"
    Write-Host "  detected OS:   windows"
    Write-Host "  detected arch: $script:archKind"
    Write-Host "  R2 object:     $script:r2Object"
    Write-Host "  base URL:      $installBase"
    Write-Host "  would download: $binUrl"
    Write-Host "  would verify:  $binUrl.sha256"
    Write-Host "  would install: $targetBin"
    Write-Host ""
    Write-Host "  SPECTYN_INSTALL_DRY_RUN=1 — no files written."
    exit 0
}

Write-Host "  spectyn-mesh installer"
Write-Host "    target: $script:r2Object"

Load-Verifier

# F-CRIT-3: Require-Https refuses plain http:// unless the operator
# explicitly opts out with $env:SPECTYN_ALLOW_INSECURE = '1'.
Require-Https -Url $binUrl

New-Item -ItemType Directory -Force $installDir | Out-Null

# Stop a running spectyn (if any) before overwriting the .exe; otherwise
# Windows refuses to replace it. Best-effort.
Get-Process spectyn -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

# Download to a temp path so a mid-stream failure cannot leave a broken
# binary in PATH.
$tmpBin = [System.IO.Path]::GetTempFileName()
# .tmp suffix confuses some AV scanners on Windows; rename to .exe.
$tmpExe = "$tmpBin.exe"
Remove-Item -Force $tmpBin -ErrorAction SilentlyContinue

Write-Host "    downloading $binUrl ..."
try {
    Invoke-WebRequest -Uri $binUrl `
                      -OutFile $tmpExe `
                      -UseBasicParsing `
                      -TimeoutSec 120 `
                      -Headers @{ 'User-Agent' = 'spectyn-installer/1.0' }
} catch {
    Remove-Item -Force $tmpExe -ErrorAction SilentlyContinue
    Fail-MissingBinary -Url $binUrl -Inner $_.Exception.Message
}

if (-not (Test-Path $tmpExe)) {
    Fail-MissingBinary -Url $binUrl -Inner 'download produced no file'
}

# F-CRIT-3: verify SHA256 BEFORE Unblock-File or move into PATH.
# Verify-Sha256 deletes $tmpExe on mismatch and throws.
try {
    Verify-Sha256 -BinaryPath $tmpExe -DownloadUrl $binUrl
} catch {
    Remove-Item -Force $tmpExe -ErrorAction SilentlyContinue
    throw
}

Unblock-File -Path $tmpExe -ErrorAction SilentlyContinue
Move-Item -Force -Path $tmpExe -Destination $targetBin

Wire-Path -Dir $installDir

# Per F500 spec: last stdout line must be EXACTLY this so the wizard (F501)
# takes over cleanly on next `spectyn` invocation. No banners.
Write-Host 'Run `spectyn` to start.'
