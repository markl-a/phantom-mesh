# Phantom Mesh - Windows production Authenticode signing for CI.
#
# Unlike scripts/codesign-windows.ps1 (dev SELF-SIGNED smoke, Trusted-Root
# warnings expected), this script attaches a REAL Authenticode signature using
# signtool.exe + a production PFX certificate supplied via CI secret, with an
# RFC3161 timestamp so the signature outlives the cert's validity window. A
# binary signed this way passes SmartScreen reputation once the publisher
# builds trust, instead of showing the unknown-publisher warning.
#
# Design goals:
#   - SECRET-GATED, GRACEFUL NO-OP. With no PFX secret present (PRs, forks, the
#     2026-05-17 cost-frozen state) the script prints "skip" and exits 0 so it
#     never breaks an unsigned build. Signing only happens when a cert is wired.
#   - signtool.exe is NOT on PATH; we locate it under the Windows 10/11 SDK and
#     pick the highest version.
#   - The PFX is materialized from a base64 secret to a temp file, used, then
#     securely removed in a finally block (never written to the workspace).
#
# Pure ASCII (no smart quotes, no glyphs) so Windows PowerShell 5.1 on
# CP950 / CP932 / CP949 locales parses it. Same constraint as
# codesign-windows.ps1 / package-windows.ps1.
#
# Usage:
#   # sign one or more files (CI):
#   $env:WINDOWS_CODESIGN_PFX_BASE64 = '<base64 of cert.pfx>'
#   $env:WINDOWS_CODESIGN_PFX_PASSWORD = '<pfx password>'
#   .\scripts\sign-windows-ci.ps1 -Path .\phantom.exe
#   .\scripts\sign-windows-ci.ps1 -Path file1.exe,file2.msi
#
#   # verify only (no secret needed):
#   .\scripts\sign-windows-ci.ps1 -Verify -Path .\phantom.exe
#
# Exit codes:
#   0  signed OK, OR skipped because no cert secret (by design), OR verify pass
#   1  signing/verify failed
#   2  bad args / signtool not found / target missing
#   3  cert secret present but malformed (base64 decode / pfx load failed)

[CmdletBinding(DefaultParameterSetName = 'Sign')]
param(
    [Parameter(Mandatory = $true)]
    [string[]]$Path,

    [Parameter(ParameterSetName = 'Verify')]
    [switch]$Verify,

    # Base64-encoded PFX. Defaults to the CI secret env var so the workflow can
    # just `env:` it in. Empty => graceful skip.
    [string]$PfxBase64 = $env:WINDOWS_CODESIGN_PFX_BASE64,

    [string]$PfxPassword = $env:WINDOWS_CODESIGN_PFX_PASSWORD,

    # RFC3161 timestamp server. DigiCert's is free + reliable; override if the
    # production cert's CA recommends another.
    [string]$TimestampUrl = 'http://timestamp.digicert.com'
)

$ErrorActionPreference = 'Stop'

# ---------- helpers ----------

function Find-Signtool {
    # signtool.exe ships with the Windows SDK, not on PATH. Search the standard
    # SDK bin tree and pick the highest version that has an x64 build.
    $roots = @(
        "${env:ProgramFiles(x86)}\Windows Kits\10\bin",
        "${env:ProgramFiles}\Windows Kits\10\bin"
    ) | Where-Object { $_ -and (Test-Path $_) }

    $candidates = foreach ($root in $roots) {
        Get-ChildItem -Path $root -Recurse -Filter 'signtool.exe' -ErrorAction SilentlyContinue |
            Where-Object { $_.FullName -match '\\x64\\signtool\.exe$' }
    }
    if (-not $candidates) {
        # last resort: maybe it's on PATH after all
        $onPath = Get-Command signtool.exe -ErrorAction SilentlyContinue
        if ($onPath) { return $onPath.Source }
        return $null
    }
    # Sort by the SDK version folder (e.g. 10.0.22621.0) descending.
    $best = $candidates | Sort-Object {
        if ($_.FullName -match '\\10\\bin\\([0-9.]+)\\') { [version]$Matches[1] } else { [version]'0.0' }
    } -Descending | Select-Object -First 1
    return $best.FullName
}

function Report-Signature {
    param([string]$TargetPath)
    $sig = Get-AuthenticodeSignature -FilePath $TargetPath
    Write-Host "  $TargetPath"
    Write-Host "    Status        : $($sig.Status)"
    Write-Host "    StatusMessage : $($sig.StatusMessage)"
    if ($sig.SignerCertificate) {
        Write-Host "    Signer Subject: $($sig.SignerCertificate.Subject)"
        Write-Host "    Signer Thumb  : $($sig.SignerCertificate.Thumbprint)"
    }
    return $sig
}

# ---------- validate targets ----------

$targets = @()
foreach ($p in $Path) {
    if (-not (Test-Path $p)) {
        Write-Host "[!] Target not found: $p" -ForegroundColor Red
        exit 2
    }
    $targets += (Resolve-Path $p).Path
}

# ---------- subcommand: -Verify ----------

if ($Verify) {
    Write-Host "=== sign-windows-ci: verify only ===" -ForegroundColor Cyan
    $fail = $false
    foreach ($t in $targets) {
        $sig = Report-Signature -TargetPath $t
        if ($sig.Status -eq 'NotSigned') {
            Write-Host "[FAIL] $t is NotSigned" -ForegroundColor Red
            $fail = $true
        }
    }
    if ($fail) { exit 1 }
    Write-Host "[PASS] all targets carry a signature" -ForegroundColor Green
    exit 0
}

# ---------- graceful skip when no cert secret ----------

if ([string]::IsNullOrWhiteSpace($PfxBase64)) {
    Write-Host "=== sign-windows-ci: SKIP ===" -ForegroundColor Yellow
    Write-Host "  No WINDOWS_CODESIGN_PFX_BASE64 secret present."
    Write-Host "  Artifacts ship UNSIGNED (SmartScreen will warn). This is expected"
    Write-Host "  on PRs / forks / before a production cert is provisioned."
    exit 0
}

# ---------- locate signtool ----------

Write-Host "=== sign-windows-ci: sign + verify ===" -ForegroundColor Cyan
$signtool = Find-Signtool
if (-not $signtool) {
    Write-Host "[!] signtool.exe not found (install the Windows 10/11 SDK)." -ForegroundColor Red
    exit 2
}
Write-Host "  signtool: $signtool"

# ---------- materialize PFX from secret (temp, cleaned in finally) ----------

$pfxPath = Join-Path ([System.IO.Path]::GetTempPath()) ("phantom-codesign-" + [System.IO.Path]::GetRandomFileName() + ".pfx")
$exitCode = 0
try {
    try {
        [System.IO.File]::WriteAllBytes($pfxPath, [System.Convert]::FromBase64String($PfxBase64))
    } catch {
        Write-Host "[!] Failed to decode WINDOWS_CODESIGN_PFX_BASE64: $($_.Exception.Message)" -ForegroundColor Red
        exit 3
    }

    foreach ($t in $targets) {
        Write-Host ""
        Write-Host "[*] Signing $t" -ForegroundColor Cyan
        # /fd sha256  file digest algorithm
        # /tr <url>   RFC3161 timestamp server  /td sha256 timestamp digest
        # /f <pfx> /p <pass>  cert + password
        $args = @('sign', '/fd', 'sha256', '/tr', $TimestampUrl, '/td', 'sha256', '/f', $pfxPath)
        if (-not [string]::IsNullOrEmpty($PfxPassword)) { $args += @('/p', $PfxPassword) }
        $args += $t

        & $signtool @args
        if ($LASTEXITCODE -ne 0) {
            Write-Host "[FAIL] signtool sign exited $LASTEXITCODE for $t" -ForegroundColor Red
            $exitCode = 1
            continue
        }

        # /pa = use the Default Authenticode verification policy. Do NOT swallow
        # the result: a failed RFC3161 timestamp or an untrusted production-cert
        # chain must FAIL the sign step, not be hidden behind a still-"Valid"
        # Get-AuthenticodeSignature status (QA-sweep finding). This is a
        # PRODUCTION signing path -- a real cert is expected to verify clean; use
        # codesign-windows.ps1 for dev self-signed smoke instead.
        & $signtool verify /pa /v $t
        $verifyExit = $LASTEXITCODE
        $sig = Report-Signature -TargetPath $t
        if ($verifyExit -ne 0) {
            Write-Host "[FAIL] signtool verify exited $verifyExit for $t (chain/timestamp not trusted?)" -ForegroundColor Red
            $exitCode = 1
        } elseif ($sig.Status -eq 'NotSigned') {
            Write-Host "[FAIL] $t still reads NotSigned after signing" -ForegroundColor Red
            $exitCode = 1
        } else {
            Write-Host "[PASS] $t signed + verified (status=$($sig.Status))" -ForegroundColor Green
        }
    }
} finally {
    if (Test-Path $pfxPath) {
        Remove-Item $pfxPath -Force -ErrorAction SilentlyContinue
    }
}

exit $exitCode
