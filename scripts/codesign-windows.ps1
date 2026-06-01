# Phantom Mesh - Windows code-signing smoke (dev self-signed)
#
# Subcommands (mutually exclusive, default = sign):
#   .\scripts\codesign-windows.ps1 -CreateCert          # ensure dev cert exists in CurrentUser\My
#   .\scripts\codesign-windows.ps1                       # sign default target + verify
#   .\scripts\codesign-windows.ps1 -Path <exe>           # sign explicit path + verify
#   .\scripts\codesign-windows.ps1 -Verify               # read-only Get-AuthenticodeSignature on default target
#   .\scripts\codesign-windows.ps1 -Verify -Path <exe>   # read-only on explicit path
#
# Smoke definition (task-2026052622):
#   - Cert creation idempotent (re-run safe)
#   - Sign attaches SignerCertificate matching the dev cert thumbprint
#   - Status != NotSigned (Valid OR UnknownError both acceptable for self-signed
#     cert that is not yet in Trusted Root; see docs/superpowers/notes/windows-codesign-smoke.md)
#
# Pure ASCII (no smart quotes, no glyphs) so Windows PowerShell 5.1 on
# CP950 / CP932 / CP949 locales can parse this file. If you must add a
# non-ASCII char, also save with UTF-8 BOM.
#
# Production cert / EV cert / Timestamp server DEFERRED for this DEV smoke.
# For real CI Authenticode signing (PFX-from-secret + RFC3161 timestamp,
# secret-gated no-op), see scripts/sign-windows-ci.ps1 (wired into
# .github/workflows/release-windows.yml).

[CmdletBinding(DefaultParameterSetName = 'Sign')]
param(
    [Parameter(ParameterSetName = 'CreateCert')]
    [switch]$CreateCert,

    [Parameter(ParameterSetName = 'Verify')]
    [switch]$Verify,

    [Parameter(ParameterSetName = 'Sign')]
    [Parameter(ParameterSetName = 'Verify')]
    [string]$Path = 'D:\tmp\phantom-windows-target\release\phantom.exe',

    [Parameter(ParameterSetName = 'Sign')]
    [Parameter(ParameterSetName = 'CreateCert')]
    [string]$CertSubject = 'CN=Phantom Mesh Dev Code Signing'
)

$ErrorActionPreference = 'Stop'

# ---------- helpers ----------

function Find-DevCert {
    param([string]$Subject)
    Get-ChildItem -Path Cert:\CurrentUser\My |
        Where-Object { $_.Subject -eq $Subject } |
        Sort-Object NotBefore -Descending |
        Select-Object -First 1
}

function New-DevCert {
    param([string]$Subject)
    # New-SelfSignedCertificate is available on PowerShell 5.1 / Windows 10+
    # KeyUsage DigitalSignature + EKU 1.3.6.1.5.5.7.3.3 = Code Signing
    $cert = New-SelfSignedCertificate `
        -Subject $Subject `
        -CertStoreLocation 'Cert:\CurrentUser\My' `
        -Type CodeSigningCert `
        -KeyUsage DigitalSignature `
        -KeyAlgorithm RSA `
        -KeyLength 2048 `
        -HashAlgorithm SHA256 `
        -NotAfter (Get-Date).AddYears(2)
    return $cert
}

function Ensure-DevCert {
    param([string]$Subject)
    $cert = Find-DevCert -Subject $Subject
    if ($cert) {
        Write-Host "[=] Found existing dev cert: $($cert.Subject)" -ForegroundColor Green
        Write-Host "    Thumbprint: $($cert.Thumbprint)"
        Write-Host "    NotBefore : $($cert.NotBefore)"
        Write-Host "    NotAfter  : $($cert.NotAfter)"
        return $cert
    }
    Write-Host "[+] Creating new dev cert: $Subject" -ForegroundColor Yellow
    $cert = New-DevCert -Subject $Subject
    Write-Host "[+] Created. Thumbprint: $($cert.Thumbprint)" -ForegroundColor Green
    Write-Host "    NotBefore: $($cert.NotBefore)"
    Write-Host "    NotAfter : $($cert.NotAfter)"
    return $cert
}

function Report-Signature {
    param([string]$TargetPath)
    if (-not (Test-Path $TargetPath)) {
        Write-Host "[!] Target not found: $TargetPath" -ForegroundColor Red
        return $null
    }
    $sig = Get-AuthenticodeSignature -FilePath $TargetPath
    Write-Host ""
    Write-Host "Authenticode signature on $TargetPath" -ForegroundColor Cyan
    Write-Host "  Status        : $($sig.Status)"
    Write-Host "  StatusMessage : $($sig.StatusMessage)"
    if ($sig.SignerCertificate) {
        Write-Host "  Signer Subject: $($sig.SignerCertificate.Subject)"
        Write-Host "  Signer Thumb  : $($sig.SignerCertificate.Thumbprint)"
        Write-Host "  Sig Algorithm : $($sig.SignatureType)"
    } else {
        Write-Host "  Signer        : (none - NotSigned)"
    }
    return $sig
}

# ---------- subcommand: -CreateCert ----------

if ($CreateCert) {
    Write-Host "=== Phantom Mesh codesign - ensure dev cert ===" -ForegroundColor Cyan
    $null = Ensure-DevCert -Subject $CertSubject
    exit 0
}

# ---------- subcommand: -Verify ----------

if ($Verify) {
    Write-Host "=== Phantom Mesh codesign - verify only ===" -ForegroundColor Cyan
    $sig = Report-Signature -TargetPath $Path
    if (-not $sig) { exit 2 }
    # NotSigned is the only definitive smoke fail; any other status proves
    # signature attachment worked (UnknownError just means self-signed isn't
    # in Trusted Root, which is expected for dev cert).
    if ($sig.Status -eq 'NotSigned') {
        Write-Host "[FAIL] $Path is NotSigned" -ForegroundColor Red
        exit 1
    }
    Write-Host "[PASS] Signature present (status=$($sig.Status))" -ForegroundColor Green
    exit 0
}

# ---------- default subcommand: sign + verify ----------

Write-Host "=== Phantom Mesh codesign - sign + verify ===" -ForegroundColor Cyan

$cert = Ensure-DevCert -Subject $CertSubject

if (-not (Test-Path $Path)) {
    Write-Host ""
    Write-Host "[!] Target exe not found: $Path" -ForegroundColor Yellow
    Write-Host "    Hint: run .\scripts\build-windows.ps1 first to produce phantom.exe,"
    Write-Host "          or pass -Path <other.exe> to sign a different file."
    Write-Host "    For pure pipeline smoke (no real exe), compile a stub via Add-Type"
    Write-Host "    (do NOT copy a System32 binary - those match the OS catalog and"
    Write-Host "     Get-AuthenticodeSignature then reads the catalog signer, not ours):"
    Write-Host "      `$code = 'public class Smoke { public static void Main() {} }'"
    Write-Host "      Add-Type -TypeDefinition `$code -OutputAssembly `$env:TEMP\smoke.exe -OutputType ConsoleApplication"
    Write-Host "      .\scripts\codesign-windows.ps1 -Path `$env:TEMP\smoke.exe"
    exit 3
}

Write-Host ""
Write-Host "[*] Signing $Path" -ForegroundColor Cyan
$signResult = Set-AuthenticodeSignature `
    -FilePath $Path `
    -Certificate $cert `
    -HashAlgorithm SHA256

Write-Host "    Sign call Status      : $($signResult.Status)"
Write-Host "    Sign call StatusMessage: $($signResult.StatusMessage)"

# Re-read so we report what's actually attached on disk, not the cmdlet's
# in-memory return (the two can differ for HashMismatch / chain failures).
$verifySig = Report-Signature -TargetPath $Path

if (-not $verifySig) { exit 2 }

if ($verifySig.Status -eq 'NotSigned') {
    Write-Host ""
    Write-Host "[FAIL] Sign call succeeded but $Path still reads as NotSigned" -ForegroundColor Red
    exit 1
}

if ($verifySig.SignerCertificate -and
    $verifySig.SignerCertificate.Thumbprint -eq $cert.Thumbprint) {
    Write-Host ""
    Write-Host "[PASS] Signature attached, signer thumbprint matches dev cert" -ForegroundColor Green
    Write-Host "       (Status='$($verifySig.Status)' - UnknownError is OK for self-signed)"
    exit 0
}

Write-Host ""
Write-Host "[FAIL] Signer thumbprint mismatch - expected $($cert.Thumbprint)" -ForegroundColor Red
exit 1
