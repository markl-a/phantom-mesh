<#
.SYNOPSIS
    PowerShell mirror of epic-acceptance-score.sh.

.DESCRIPTION
    Counts acceptance-criterion checkboxes across docs/superpowers/specs/_current/E00[1-6]-*.md,
    emits a Markdown table byte-identical to the POSIX sh sibling, and exits non-zero if
    the total is below the freeze-week threshold.

    Spec:    docs/superpowers/features/F600-freeze-week-protocol-runbook.md
    Runbook: docs/superpowers/runbooks/E007-freeze-week.md

.PARAMETER Strict
    Fail (exit 2) if any E00[1-6]-*.md lacks an acceptance-criteria H2 —
    either the English "## Acceptance criteria" or the Chinese "## <U+9A57><U+6536><U+6A19><U+6E96>"
    (yan shou biao zhun) used by the canonical specs in docs/superpowers/specs/_current.

.PARAMETER IncludeE007
    Include E007 in the total (default off — gate is about E001-E006 per F600).

.PARAMETER Threshold
    Integer 0-100. Total must be >= this to exit 0. Default 80
    (or env PHANTOM_E007_MIN_PERCENT).

.PARAMETER SpecsDir
    Directory containing the epic spec files. Default docs/superpowers/specs/_current
    (or env PHANTOM_E007_SPECS_DIR).

.NOTES
    Exit codes:
      0   total >= threshold → SHIP (S1)
      1   total <  threshold → SLIP_TO=2026-06-17 (S2)
      2   spec format drift  → do NOT cut on a broken scoreboard
      64  usage error

    Env contract (parity with shell sibling):
      PHANTOM_E007_MIN_PERCENT   integer 0-100, default 80
      PHANTOM_E007_SPECS_DIR     path, default docs/superpowers/specs/_current
      PHANTOM_E007_INCLUDE_E007  "1" to include E007 in the total
#>
[CmdletBinding()]
param(
    [switch]$Strict,
    [switch]$IncludeE007,
    [int]$Threshold = -1,
    [string]$SpecsDir = ""
)

$ErrorActionPreference = 'Stop'

# ---------- resolve config (param > env > default) ----------
if ($Threshold -lt 0) {
    if ($env:PHANTOM_E007_MIN_PERCENT) {
        $Threshold = [int]$env:PHANTOM_E007_MIN_PERCENT
    } else {
        $Threshold = 80
    }
}
if (-not $SpecsDir) {
    if ($env:PHANTOM_E007_SPECS_DIR) {
        $SpecsDir = $env:PHANTOM_E007_SPECS_DIR
    } else {
        $SpecsDir = 'docs/superpowers/specs/_current'
    }
}
$includeE007Effective = $IncludeE007.IsPresent -or ($env:PHANTOM_E007_INCLUDE_E007 -eq '1')

if ($Threshold -lt 0 -or $Threshold -gt 100) {
    Write-Error "threshold must be 0-100, got: $Threshold"
    exit 64
}
if (-not (Test-Path -LiteralPath $SpecsDir -PathType Container)) {
    Write-Error "specs dir does not exist: $SpecsDir"
    exit 64
}

if ($includeE007Effective) {
    $regex = '^E00[1-7]-.*\.md$'
    $patternLabel = 'E00[1-7]-*.md'
    $scopeLabel = 'E001-E007'
} else {
    $regex = '^E00[1-6]-.*\.md$'
    $patternLabel = 'E00[1-6]-*.md'
    $scopeLabel = 'E001-E006'
}

# PowerShell's -Filter does not handle POSIX-glob bracket char classes; use a
# regex match on the file name instead.
$epicFiles = Get-ChildItem -LiteralPath $SpecsDir -File |
    Where-Object { $_.Name -match $regex } |
    Sort-Object Name
if (-not $epicFiles -or $epicFiles.Count -eq 0) {
    Write-Error "no epic specs matched ${SpecsDir}/${patternLabel}"
    exit 64
}

# ---------- score one file ----------
# The acceptance-criteria H2 is either English "## Acceptance criteria" or the
# Chinese heading (code points U+9A57 U+6536 U+6A19 U+6E96, "yan shou biao
# zhun") used by the canonical specs in docs/superpowers/specs/_current. The
# Chinese chars are built from [char] code points so this script stays pure
# ASCII (immune to script-file encoding misreads on PS 5.1, which assumes ANSI
# for BOM-less files).
$zhAcceptanceHeading = -join ([char]0x9A57, [char]0x6536, [char]0x6A19, [char]0x6E96)
$acceptanceHeadingRegex = '^## (Acceptance criteria|' + [regex]::Escape($zhAcceptanceHeading) + ')'

function Score-OneSpec {
    param([string]$Path)
    $inSection = $false
    $haveSection = $false
    $done = 0
    $total = 0
    # -Encoding UTF8: the specs are UTF-8 (Chinese headings); Windows
    # PowerShell 5.1 would otherwise read BOM-less files as ANSI.
    foreach ($line in Get-Content -LiteralPath $Path -Encoding UTF8) {
        if ($line -match $script:acceptanceHeadingRegex) {
            $inSection = $true
            $haveSection = $true
            continue
        }
        if ($inSection -and $line -match '^## ') {
            $inSection = $false
            continue
        }
        if ($inSection) {
            if ($line -match '^- \[[xX]\]') {
                $done++
                $total++
            } elseif ($line -match '^- \[ \]') {
                $total++
            }
        }
    }
    return [pscustomobject]@{
        HaveSection = $haveSection
        Done        = $done
        Total       = $total
    }
}

# ---------- emit table ----------
$totalDone = 0
$totalTotal = 0
$drift = $false

Write-Output '| Epic | Done | Total | %   | Status |'
Write-Output '|------|------|-------|-----|--------|'

foreach ($f in $epicFiles) {
    $epicId = $f.Name.Substring(0, 4)
    $r = Score-OneSpec -Path $f.FullName

    if (-not $r.HaveSection) {
        $drift = $true
        Write-Output ('| {0} | -    | -     | -   | DRIFT  |' -f $epicId)
        Write-Warning ("$($f.Name) has no acceptance-criteria H2 (English or Chinese; strict-mode failure)")
        continue
    }
    if ($r.Total -eq 0) {
        $drift = $true
        Write-Output ('| {0} | 0    | 0     | -   | DRIFT  |' -f $epicId)
        Write-Warning ("$($f.Name) has an acceptance-criteria H2 but no checkboxes")
        continue
    }

    $pct = [int]([math]::Floor(($r.Done * 100) / $r.Total))
    if ($pct -ge $Threshold) { $status = 'GREEN' }
    elseif ($pct -gt 0)      { $status = 'AMBER' }
    else                     { $status = 'RED' }

    Write-Output ('| {0} | {1,-4} | {2,-5} | {3,-3} | {4,-6} |' -f $epicId, $r.Done, $r.Total, $pct, $status)
    $totalDone  += $r.Done
    $totalTotal += $r.Total
}

if ($totalTotal -eq 0) {
    $totalPct = 0
} else {
    $totalPct = [int]([math]::Floor(($totalDone * 100) / $totalTotal))
}

if ($totalPct -ge $Threshold) { $totalStatus = 'GREEN' }
elseif ($totalPct -gt 0)      { $totalStatus = 'AMBER' }
else                          { $totalStatus = 'RED' }

Write-Output '|------|------|-------|-----|--------|'
Write-Output ('| TOTAL| {0,-4} | {1,-5} | {2,-3} | {3,-6} |' -f $totalDone, $totalTotal, $totalPct, $totalStatus)

Write-Output ''
Write-Output ('Scope:     {0} (set PHANTOM_E007_INCLUDE_E007=1 to include E007)' -f $scopeLabel)
Write-Output ('Threshold: {0}% (set via PHANTOM_E007_MIN_PERCENT or -Threshold)' -f $Threshold)

if ($Strict.IsPresent -and $drift) {
    Write-Output ('Result:    TOTAL >= {0}% ? STRICT-FAIL (spec format drift)' -f $Threshold)
    Write-Output 'Action:    Fix the spec(s) flagged above; re-run.'
    exit 2
}

if ($totalPct -ge $Threshold) {
    Write-Output ('Result:    TOTAL >= {0}% ? YES -> SHIP (S1, tag v0.6.0)' -f $Threshold)
    Write-Output 'Action:    Proceed with section 6 tag-and-release in runbook.'
    exit 0
} else {
    Write-Output ('Result:    TOTAL >= {0}% ? NO  -> SLIP_TO=2026-06-17 (S2)' -f $Threshold)
    Write-Output 'Action:    Post section 4.2 slip announcement; enter section 4.3 first-24h-of-S2 plan.'
    exit 1
}
