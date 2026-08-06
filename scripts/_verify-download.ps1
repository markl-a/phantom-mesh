# scripts/_verify-download.ps1
#
# Shared PowerShell helpers for SHA256-verified binary downloads.
# Dot-sourced by install-spectyn-windows.ps1 and windows-bootstrap.ps1.
#
# Functions provided:
#   Require-Https <url>          -- throws if URL is plain http:// (unless
#                                  $env:SPECTYN_ALLOW_INSECURE -eq '1')
#   Get-Sha256Local <path>       -- returns lowercase hex sha256 of a file
#   Verify-Sha256 <bin> <url>    -- downloads <url>.sha256 over HTTPS,
#                                  compares against Get-Sha256Local <bin>,
#                                  deletes the binary + throws on mismatch
#
# Threat model: docs/install-binary-verification.md
#
# Env opt-outs:
#   $env:SPECTYN_ALLOW_INSECURE='1'  -- allow plain http://
#   $env:SPECTYN_SKIP_VERIFY='1'     -- skip SHA256 verification (loud warn)

function Require-Https {
    param([Parameter(Mandatory)][string]$Url)

    if ($Url -like 'https://*') { return }

    if ($Url -like 'http://*') {
        if ($env:SPECTYN_ALLOW_INSECURE -eq '1') {
            Write-Warning "SPECTYN_ALLOW_INSECURE=1 - accepting plain http:// URL ($Url)"
            Write-Warning "  THIS DISABLES MITM PROTECTION."
            return
        }
        throw "Refusing to download binary over plain http://`n  URL: $Url`n  Use an https:// URL, or set `$env:SPECTYN_ALLOW_INSECURE='1' explicitly`n  (only safe on a trusted tailnet - see docs/install-binary-verification.md)."
    }

    throw "Unsupported URL scheme: $Url"
}

function Get-Sha256Local {
    param([Parameter(Mandatory)][string]$Path)
    if (-not (Test-Path $Path)) { throw "Get-Sha256Local: file not found: $Path" }
    $h = Get-FileHash -Algorithm SHA256 -Path $Path
    return $h.Hash.ToLowerInvariant()
}

function Verify-Sha256 {
    param(
        [Parameter(Mandatory)][string]$BinaryPath,
        [Parameter(Mandatory)][string]$DownloadUrl
    )

    if ($env:SPECTYN_SKIP_VERIFY -eq '1') {
        Write-Warning "SPECTYN_SKIP_VERIFY=1 - SKIPPING SHA256 verification of $BinaryPath"
        Write-Warning "  This means a MITM or compromised mirror can replace the spectyn binary."
        Write-Warning "  Do not use except on an air-gapped first install where the sums file"
        Write-Warning "  isn't published yet."
        return
    }

    if (-not (Test-Path $BinaryPath)) {
        throw "Verify-Sha256: local binary not found: $BinaryPath"
    }

    $sumsUrl = "$DownloadUrl.sha256"
    Require-Https -Url $sumsUrl

    $sumsFile = [System.IO.Path]::GetTempFileName()
    try {
        try {
            Invoke-WebRequest -Uri $sumsUrl `
                              -OutFile $sumsFile `
                              -UseBasicParsing `
                              -TimeoutSec 30 `
                              -Headers @{ 'User-Agent' = 'spectyn-installer/1.0' } | Out-Null
        } catch {
            Remove-Item -Force $BinaryPath -ErrorAction SilentlyContinue
            throw "Could not fetch SHA256 sidecar at $sumsUrl ($_).`n  Refusing to install an unverified binary.`n  Set `$env:SPECTYN_SKIP_VERIFY='1' to bypass (NOT recommended)."
        }

        # sha256sum format is "<hex>  <name>"; take first whitespace-delimited
        # field of first non-empty line.
        # @() forces array wrapping so single-line files don't get indexed as
        # a char array (Get-Content returns a bare String when there's exactly
        # one line, and "$str[0]" yields the first character, not the line).
        $lines = @(Get-Content $sumsFile | Where-Object { $_.Trim() -ne '' })
        if ($lines.Count -eq 0) {
            Remove-Item -Force $BinaryPath -ErrorAction SilentlyContinue
            throw "SHA256 sidecar at $sumsUrl is empty."
        }
        $firstLine = [string]$lines[0]
        $expected = ($firstLine -split '\s+', 2)[0].ToLowerInvariant()
        if ($expected -notmatch '^[0-9a-f]{64}$') {
            Remove-Item -Force $BinaryPath -ErrorAction SilentlyContinue
            throw "SHA256 sidecar at $sumsUrl is malformed (got: '$expected')."
        }

        $actual = Get-Sha256Local -Path $BinaryPath
        if ($expected -ne $actual) {
            Remove-Item -Force $BinaryPath -ErrorAction SilentlyContinue
            throw "SHA256 mismatch for ${BinaryPath}:`n  expected: $expected`n  actual:   $actual`n  Source:   $sumsUrl`n  The downloaded binary has been deleted."
        }

        Write-Host "  sha256 verified ($expected)" -ForegroundColor Green
    }
    finally {
        Remove-Item -Force $sumsFile -ErrorAction SilentlyContinue
    }
}
