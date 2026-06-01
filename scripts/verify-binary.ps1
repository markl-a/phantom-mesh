<#
.SYNOPSIS
  verify-binary.ps1 — health-check a phantom binary (Windows / PowerShell)

.DESCRIPTION
  Runs 5-7 checks against a phantom binary to confirm it works.
  Mirrors scripts/verify-binary.sh for cross-platform parity.

.PARAMETER BinaryPath
  Path to phantom binary (e.g. C:\Users\me\.cargo\bin\phantom.exe)

.PARAMETER ExpectVersion
  If set, fail when `phantom --version --short` does not equal this.

.PARAMETER Quick
  Skip `phantom doctor`. Just version + exists.

.PARAMETER Full
  Include `phantom selftest --p0-only` (requires an LLM provider key in env or agents.toml).

.PARAMETER Json
  Machine-readable JSON output.

.EXAMPLE
  .\scripts\verify-binary.ps1 C:\Users\me\.cargo\bin\phantom.exe

.EXAMPLE
  .\scripts\verify-binary.ps1 -BinaryPath C:\bin\phantom.exe -ExpectVersion 0.6.0 -Json

.OUTPUTS
  Exit 0: all checks passed
  Exit 1: one or more checks failed
  Exit 2: argument / usage error
#>
[CmdletBinding()]
param(
  [Parameter(Mandatory = $true, Position = 0)]
  [string]$BinaryPath,

  [string]$ExpectVersion = "",

  [switch]$Quick,
  [switch]$Full,
  [switch]$Json
)

$script:ScriptName    = "verify-binary.ps1"
$script:ScriptVersion = "0.1.0"
$script:StartTime     = Get-Date
$script:Checks        = @()
$script:ExitCode      = 0

function Record-Check {
  param(
    [string]$Name,
    [ValidateSet("pass", "fail", "skip")]
    [string]$Status,
    [string]$Detail
  )
  $script:Checks += [pscustomobject]@{
    name   = $Name
    status = $Status
    detail = $Detail
  }
  if ($Status -eq "fail") { $script:ExitCode = 1 }
}

function Invoke-Phantom {
  # Note: param name MUST NOT be $Args — that's a PowerShell automatic variable
  # and splatting @Args would pick up the automatic, not our param.
  param([string[]]$Arguments)
  $stdout = & $BinaryPath @Arguments 2>&1
  $exit   = $LASTEXITCODE
  return [pscustomobject]@{
    output   = ($stdout -join "`n")
    exitCode = $exit
  }
}

# Check 1: binary exists + executable
if (Test-Path -LiteralPath $BinaryPath -PathType Leaf) {
  # On Windows, .exe extension implies executable; we just check it can be invoked
  Record-Check "binary_exists_executable" "pass" $BinaryPath
} else {
  Record-Check "binary_exists_executable" "fail" "$BinaryPath does not exist"
  if ($Json) {
    # short-circuit: still emit JSON, then exit
  } else {
    Write-Host "FAIL: binary not found" -ForegroundColor Red
    exit 1
  }
}

$binaryOk = ($script:Checks[0].status -eq "pass")

# Check 2: phantom --version
if ($binaryOk) {
  $r = Invoke-Phantom @("--version")
  if ($r.exitCode -eq 0) {
    $firstLine = ($r.output -split "`n")[0]
    Record-Check "version_runs" "pass" $firstLine
  } else {
    Record-Check "version_runs" "fail" "exit=$($r.exitCode) output=$($r.output)"
  }
}

# Check 3: phantom --version --short matches SemVer
if ($binaryOk -and ($script:Checks[-1].status -eq "pass")) {
  $r = Invoke-Phantom @("--version", "--short")
  $shortVer = ($r.output -replace '\s', '')
  if ($r.exitCode -eq 0 -and $shortVer -match '^\d+\.\d+\.\d+') {
    Record-Check "version_short_semver" "pass" $shortVer
  } else {
    Record-Check "version_short_semver" "fail" "got: '$shortVer' (exit $($r.exitCode))"
  }

  # Check 4: expect-version match (optional)
  if ($ExpectVersion -ne "") {
    if ($shortVer -eq $ExpectVersion) {
      Record-Check "version_match_expected" "pass" "$shortVer == $ExpectVersion"
    } else {
      Record-Check "version_match_expected" "fail" "got '$shortVer', expected '$ExpectVersion'"
    }
  } else {
    Record-Check "version_match_expected" "skip" "no -ExpectVersion given"
  }
} elseif ($binaryOk) {
  Record-Check "version_short_semver" "skip" "version_runs failed"
  Record-Check "version_match_expected" "skip" "version_runs failed"
}

# Check 5: phantom doctor (skipped in -Quick)
if ($Quick) {
  Record-Check "doctor_runs" "skip" "-Quick mode"
  Record-Check "doctor_json_parseable" "skip" "-Quick mode"
} elseif ($binaryOk) {
  $r = Invoke-Phantom @("doctor")
  if ($r.exitCode -eq 0) {
    $lines = ($r.output -split "`n").Count
    Record-Check "doctor_runs" "pass" "exit 0; $lines lines"
  } else {
    Record-Check "doctor_runs" "fail" "exit=$($r.exitCode)"
  }

  # Check 6: doctor --json (best-effort; older binaries may not honor --json)
  $r = Invoke-Phantom @("doctor", "--json")
  $trimmed = $r.output.TrimStart()
  if ($r.exitCode -eq 0 -and $trimmed.StartsWith("{")) {
    try {
      $null = $trimmed | ConvertFrom-Json -ErrorAction Stop
      Record-Check "doctor_json_parseable" "pass" "valid JSON"
    } catch {
      Record-Check "doctor_json_parseable" "fail" "started with { but not valid JSON"
    }
  } elseif ($r.exitCode -eq 0) {
    Record-Check "doctor_json_parseable" "skip" "--json honored but plain output (pre-0.5.0?)"
  } else {
    Record-Check "doctor_json_parseable" "fail" "exit=$($r.exitCode)"
  }
}

# Check 7: phantom selftest --p0-only (only in -Full)
if ($Full -and $binaryOk) {
  $r = Invoke-Phantom @("selftest", "--p0-only")
  if ($r.exitCode -eq 0) {
    Record-Check "selftest_p0" "pass" "exit 0"
  } else {
    Record-Check "selftest_p0" "fail" "exit=$($r.exitCode)"
  }
} else {
  Record-Check "selftest_p0" "skip" "-Full not given"
}

# Summary
$end       = Get-Date
$duration  = [int]($end - $script:StartTime).TotalSeconds
# Wrap in @() so .Count is defined even when 0 or 1 match (PS5.1 quirk).
$passCount = @($script:Checks | Where-Object { $_.status -eq "pass" }).Count
$failCount = @($script:Checks | Where-Object { $_.status -eq "fail" }).Count
$skipCount = @($script:Checks | Where-Object { $_.status -eq "skip" }).Count

if ($Json) {
  $payload = [pscustomobject]@{
    script          = $script:ScriptName
    script_version  = $script:ScriptVersion
    binary          = $BinaryPath
    duration_seconds = $duration
    summary         = [pscustomobject]@{
      pass = $passCount
      fail = $failCount
      skip = $skipCount
    }
    exit_code       = $script:ExitCode
    checks          = $script:Checks
  }
  $payload | ConvertTo-Json -Depth 5
} else {
  Write-Host "phantom verify-binary $($script:ScriptVersion)"
  Write-Host "  binary:   $BinaryPath"
  Write-Host "  duration: ${duration}s"
  Write-Host ""
  foreach ($c in $script:Checks) {
    $sym = switch ($c.status) {
      "pass" { "[+]" }
      "fail" { "[X]" }
      "skip" { "[.]" }
    }
    $color = switch ($c.status) {
      "pass" { "Green" }
      "fail" { "Red" }
      "skip" { "DarkGray" }
    }
    Write-Host ("  {0} {1,-30} {2}" -f $sym, $c.name, $c.detail) -ForegroundColor $color
  }
  Write-Host ""
  if ($script:ExitCode -eq 0) {
    Write-Host "PASS: $passCount/$(($passCount + $failCount)) checks passed ($skipCount skipped)" -ForegroundColor Green
  } else {
    Write-Host "FAIL: $passCount/$(($passCount + $failCount)) checks passed ($skipCount skipped, $failCount failed)" -ForegroundColor Red
  }
}

exit $script:ExitCode
