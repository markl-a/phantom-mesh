#!/usr/bin/env pwsh
#requires -version 5
<#
.SYNOPSIS
  check-doc-tree.ps1 — Documentation Charter §E machine-checkable lint (read-only).

.DESCRIPTION
  Implements the subset of Documentation Charter §E that is grep-assertable, per
  docs/superpowers/DOCUMENTATION-CHARTER.md §E and §F.Wave5 slice (2):

    C1  Single apex banner   — only BIG-GOAL.md may claim apex/北極星/唯一真相
                               (others must carry a "從屬/apex" subordination banner). (INV-1/5)
    C4  catalog <-> disk     — SPEC-00-INDEX §4 dashboard "on-disk" total == real SPEC-* files
                               on disk; planned rows == catalog "(planned — no file yet)" count;
                               SPEC-46 directory is registered (ghost reconciled, BRK-4). (C4)
    C7  crosswalk no-TODO    — §3.10 crosswalk data rows (E001-E007) carry no TODO/待填. (C3/C7)
    C8  no bare E00x ambiguity — every features/ table row that cites an epic carries a
                               `_current/` or `_archived/` source-dir qualifier (no bare E00x). (BRK-3/P1-6)
    K3  feature <-> test     — every in-scope (| Y |) feature row in features/INDEX.md has a
                               non-`none` Test 追溯 cell; no ghost test fn leaks into the trace
                               table (ghosts must stay quarantined in §4). (INV-16/BRK-9)
    K7  IP-leak guard        — any docs file that contains a real tailnet IP
                               (100.64.0.x / 100.87.x.x) must also carry a leak-guard marker
                               (LEAK / 洩漏 / public-leak / Tailnet / CGNAT context). (K7)
    T16 cargo-cite ghost guard — every `cargo test --test <file> <filter>` cite in
                               docs/test-cases/*.md must resolve to >=1 real fn in
                               core/tests/<file>.rs (substring semantics, same as cargo);
                               a 0-match filter runs 0 tests and exits 0 = false-green. (INV-16/BRK-9)

  READ-ONLY. Touches nothing outside docs/. Never edits product code. Re-runnable: a green
  tree returns exit 0 every run; any FAIL returns exit 1. NOT wired into CI by design — run by hand.

.PARAMETER RepoRoot
  Repo root. Defaults to the parent of the scripts/ dir holding this file.

.EXAMPLE
  pwsh scripts/check-doc-tree.ps1
#>
[CmdletBinding()]
param(
    [string]$RepoRoot
)

$ErrorActionPreference = 'Stop'

if (-not $RepoRoot) {
    $RepoRoot = Split-Path -Parent $PSScriptRoot
}
$RepoRoot = (Resolve-Path $RepoRoot).Path

$DocsRoot   = Join-Path $RepoRoot 'docs'
$SpecDir    = Join-Path $DocsRoot 'superpowers/specs/v060-deep-spec'
$IndexFile  = Join-Path $SpecDir  'SPEC-00-INDEX.md'
$FeatIndex  = Join-Path $DocsRoot 'superpowers/features/INDEX.md'
$BigGoal    = Join-Path $DocsRoot 'superpowers/BIG-GOAL.md'

# --- result bookkeeping -------------------------------------------------------
$script:Results = @()
function Record([string]$id, [string]$name, [bool]$pass, [string[]]$evidence) {
    $script:Results += [pscustomobject]@{ Id = $id; Name = $name; Pass = $pass; Evidence = $evidence }
}
function ReadLinesOrFail([string]$path, [string]$id, [string]$name) {
    if (-not (Test-Path $path)) {
        Record $id $name $false @("missing file: $path")
        return $null
    }
    return [System.IO.File]::ReadAllLines($path)
}

# =============================================================================
# C1 — single apex
# =============================================================================
function Check-C1 {
    $id = 'C1'; $name = 'single apex banner'
    $apexRe = 'locked apex|唯一真相|北極星'
    $bannerRe = '從屬|apex|錨點|anchor|INV-1'
    # scan all tracked .md under docs/superpowers (the governed tree)
    $root = Join-Path $DocsRoot 'superpowers'
    if (-not (Test-Path $root)) { Record $id $name $false @("missing: $root"); return }
    $offenders = @()
    Get-ChildItem -Path $root -Recurse -Filter *.md -File | ForEach-Object {
        $f = $_.FullName
        $isBigGoal = ($f -ieq $BigGoal)
        $hits = Select-String -Path $f -Pattern $apexRe -ErrorAction SilentlyContinue
        if ($hits) {
            if ($isBigGoal) { return }              # BIG-GOAL is allowed to claim apex
            $hasBanner = Select-String -Path $f -Pattern $bannerRe -ErrorAction SilentlyContinue
            if (-not $hasBanner) {
                $rel = $f.Substring($RepoRoot.Length).TrimStart('\','/')
                $offenders += "$rel (claims apex w/o subordination banner)"
            }
        }
    }
    Record $id $name ($offenders.Count -eq 0) `
        ($(if ($offenders.Count) { $offenders } else { @("only BIG-GOAL.md claims apex; all other apex-mentioning files carry a subordination banner") }))
}

# =============================================================================
# C4 — catalog <-> disk consistency
# =============================================================================
function Check-C4 {
    $id = 'C4'; $name = 'catalog <-> disk (SPEC count + SPEC-46 ghost)'
    $lines = ReadLinesOrFail $IndexFile $id $name
    if (-not $lines) { return }
    $ev = @(); $ok = $true

    # real SPEC artifacts on disk: *.md files SPEC-NN-*  PLUS the SPEC-46 directory (counts as 1 spec id)
    $specFiles = Get-ChildItem -Path $SpecDir -Filter 'SPEC-*.md' -File |
                 Where-Object { $_.Name -ne 'SPEC-00-INDEX.md' -and $_.Name -ne 'SPEC-TEMPLATE.md' }
    $leafMd = $specFiles.Count
    $specDirs = Get-ChildItem -Path $SpecDir -Directory | Where-Object { $_.Name -like 'SPEC-*' }
    # on-disk total per dashboard = 1 INDEX + 1 TEMPLATE + leaf .md + 1 per SPEC-* dir
    $onDiskActual = 2 + $leafMd + $specDirs.Count

    # parse the dashboard "Total 合計" row: | **Total 合計** | **57** | **47** | **10** | **1** |
    $totalRow = $lines | Where-Object { $_ -match 'Total\s*合計' } | Select-Object -First 1
    if (-not $totalRow) { $ok = $false; $ev += "no 'Total 合計' dashboard row found" }
    else {
        $nums = [regex]::Matches($totalRow, '\d+') | ForEach-Object { [int]$_.Value }
        if ($nums.Count -lt 3) { $ok = $false; $ev += "dashboard Total row unparseable: $totalRow" }
        else {
            $catalogTotal   = $nums[0]
            $dashOnDisk     = $nums[1]
            $dashPlanned    = $nums[2]
            # planned rows in catalog == "(planned — no file yet)" markers that sit on a catalog
            # TABLE ROW (line begins with "| `SPEC-"), excluding prose/TL;DR/CHANGELOG mentions.
            $plannedCount = ($lines | Where-Object {
                $_ -match '^\|\s*`SPEC-' -and $_ -match 'planned — no file yet'
            }).Count

            if ($dashOnDisk -ne $onDiskActual) {
                $ok = $false
                $ev += "dashboard on-disk=$dashOnDisk but real on-disk=$onDiskActual (leaf .md=$leafMd + INDEX + TEMPLATE + dirs=$($specDirs.Count))"
            } else {
                $ev += "on-disk dashboard ($dashOnDisk) == real ($onDiskActual): $leafMd leaf .md + INDEX + TEMPLATE + $($specDirs.Count) SPEC dir(s)"
            }
            if ($dashPlanned -ne $plannedCount) {
                $ok = $false
                $ev += "dashboard planned=$dashPlanned but catalog '(planned — no file yet)' markers=$plannedCount"
            } else {
                $ev += "planned dashboard ($dashPlanned) == catalog markers ($plannedCount)"
            }
            if ($catalogTotal -ne ($dashOnDisk + $dashPlanned)) {
                $ok = $false
                $ev += "catalog total=$catalogTotal != on-disk($dashOnDisk)+planned($dashPlanned)"
            }
        }
    }

    # SPEC-46 ghost reconciled: registered in catalog AND present on disk
    $hasSpec46Dir = ($specDirs | Where-Object { $_.Name -like 'SPEC-46*' }).Count -ge 1
    $spec46Registered = (Select-String -Path $IndexFile -Pattern 'SPEC-46-windows-cli-behavior' -ErrorAction SilentlyContinue).Count -ge 1
    if ($hasSpec46Dir -and $spec46Registered) { $ev += "SPEC-46 dir on disk + registered in catalog (BRK-4 reconciled)" }
    else { $ok = $false; $ev += "SPEC-46 ghost NOT reconciled (dir=$hasSpec46Dir, registered=$spec46Registered)" }

    Record $id $name $ok $ev
}

# =============================================================================
# C7 — crosswalk has no TODO
# =============================================================================
function Check-C7 {
    $id = 'C7'; $name = 'crosswalk no-TODO (E001-E007 rows filled)'
    $lines = ReadLinesOrFail $IndexFile $id $name
    if (-not $lines) { return }
    # crosswalk data rows are table rows that start with | **E00 and contain pillar coords.
    $rows = $lines | Where-Object { $_ -match '^\|\s*\*\*E00' }
    $ev = @(); $ok = $true
    if ($rows.Count -lt 7) { $ok = $false; $ev += "expected >=7 crosswalk E00x rows, found $($rows.Count)" }
    $todoRows = $rows | Where-Object { $_ -match 'TODO|待填|TBD' }
    if ($todoRows.Count -gt 0) {
        $ok = $false
        foreach ($r in $todoRows) { $ev += "crosswalk row still has TODO: $($r.Trim())" }
    }
    # also: each crosswalk row's pillar cell must be non-empty (no blank trailing | |)
    foreach ($r in $rows) {
        $cells = $r.Trim('|').Split('|') | ForEach-Object { $_.Trim() }
        if ($cells.Count -ge 5) {
            $pillarCell = $cells[4]
            if ([string]::IsNullOrWhiteSpace($pillarCell)) {
                $ok = $false; $ev += "crosswalk row has empty pillar cell: $($r.Trim())"
            }
        }
    }
    if ($ok) { $ev += "$($rows.Count) crosswalk rows (E001-E007) filled; no TODO/待填 in data rows" }
    Record $id $name $ok $ev
}

# =============================================================================
# C8 — no bare E00x ambiguity in features INDEX
# =============================================================================
function Check-C8 {
    $id = 'C8'; $name = 'no bare E00x (every epic cite qualified by _current/_archived)'
    $lines = ReadLinesOrFail $FeatIndex $id $name
    if (-not $lines) { return }
    # the machine-scan trace rows: start with | Fnnn |
    $rows = $lines | Where-Object { $_ -match '^\|\s*F[0-9]{3}\s*\|' }
    $ev = @(); $ok = $true
    if ($rows.Count -lt 1) { $ok = $false; $ev += "no | Fnnn | trace rows found" }
    $offenders = @()
    foreach ($r in $rows) {
        # the "Parent epic" cell is column 3
        $cells = $r.Trim('|').Split('|') | ForEach-Object { $_.Trim() }
        if ($cells.Count -lt 3) { continue }
        $epicCell = $cells[2]
        if ($epicCell -match 'E00[0-9]') {
            if ($epicCell -notmatch '_current|_archived') {
                $fid = $cells[0]
                $offenders += "$fid epic cite has no _current/_archived qualifier: '$epicCell'"
            }
        }
    }
    if ($offenders.Count) { $ok = $false; $ev += $offenders }
    else { $ev += "all $($rows.Count) feature rows qualify their E00x cite with _current/ or _archived/ (BRK-3 disambiguated)" }
    Record $id $name $ok $ev
}

# =============================================================================
# K3 — feature <-> test (every in-scope feature has a test ID)
# =============================================================================
function Check-K3 {
    $id = 'K3'; $name = 'feature <-> test (in-scope rows bound; no ghost fn in trace)'
    $lines = ReadLinesOrFail $FeatIndex $id $name
    if (-not $lines) { return }
    $rows = $lines | Where-Object { $_ -match '^\|\s*F[0-9]{3}\s*\|' }
    $ev = @(); $ok = $true
    # in-scope == cell "v0.6.0 in-scope?" (col 7) == 'Y'.
    # INV-16 obligation per the INDEX §4 self-check: a `none` Test 追溯 is legitimate ONLY for
    # spec-only features (no impl yet). A feature that claims IMPLEMENTATION
    # (SHIPPED / SHIPPED-flag-gated / PARTIAL / REGRESSED) MUST bind >=1 test id (bound/bound-pending).
    $implRe = 'SHIPPED|PARTIAL|REGRESSED'
    $inScope = @(); $needBound = @()
    foreach ($r in $rows) {
        $cells = $r.Trim('|').Split('|') | ForEach-Object { $_.Trim() }
        if ($cells.Count -lt 8) { continue }
        if ($cells[6] -eq 'Y') {
            $inScope += ,$cells
            if ($cells[5] -match $implRe) { $needBound += ,$cells }
        }
    }
    if ($inScope.Count -lt 1) { $ok = $false; $ev += "found 0 in-scope (| Y |) feature rows" }
    $bad = @()
    foreach ($c in $needBound) {
        $testCell = $c[7]
        if ($testCell -eq 'none' -or [string]::IsNullOrWhiteSpace($testCell)) {
            $bad += "$($c[0]) (status=$($c[5])) claims impl but Test 追溯 == none (INV-16 violation)"
        } elseif ($testCell -notmatch 'bound') {
            $bad += "$($c[0]) (status=$($c[5])) has no bound/bound-pending test: '$testCell'"
        }
    }
    if ($bad.Count) { $ok = $false; $ev += $bad }
    else {
        $specOnly = $inScope.Count - $needBound.Count
        $ev += "$($needBound.Count)/$($inScope.Count) in-scope w/ impl all bound to >=1 test id; $specOnly spec-only in-scope legitimately carry 'none' (INV-16 self-check)"
    }

    # ghost-fn guard: the known ghost fns must NOT appear inside the trace table rows
    # (they are allowed only in the §4 FAIL quarantine).
    $ghostRe = 'floor_char_boundary|fn invalid_slug'
    $ghostInRows = $rows | Where-Object { $_ -match $ghostRe }
    if ($ghostInRows.Count -gt 0) {
        $ok = $false
        foreach ($g in $ghostInRows) { $ev += "ghost fn leaked into trace row: $($g.Trim())" }
    } else {
        $ev += "no ghost test fn (floor_char_boundary/invalid_slug) leaks into the trace table (quarantined in §4)"
    }
    Record $id $name $ok $ev
}

# =============================================================================
# K7 — IP-leak guard
# =============================================================================
function Check-K7 {
    $id = 'K7'; $name = 'IP-leak guard (real IP only in leak-guarded files)'
    $ipRe = '100\.87\.\d+\.\d+|100\.64\.0\.\d'
    # a file that mentions a real IP must also carry a guard marker explaining it's a known leak
    # / tailnet-context, not a fresh hardcode.
    $guardRe = 'LEAK|leak|洩漏|public-leak|Tailnet|tailnet|CGNAT|Tailscale|100\.64\.0\.0/10'
    # K7 guards the SHIPPABLE / public-facing doc tree. Frozen archives (docs/_archive/,
    # docs/spec/history/) are out of scope of the public leak surface and are reported as INFO only.
    $unguarded = @()
    $archiveInfo = @()
    $guardedCount = 0
    Get-ChildItem -Path $DocsRoot -Recurse -Filter *.md -File | ForEach-Object {
        $f = $_.FullName
        $rel = $f.Substring($RepoRoot.Length).TrimStart('\','/')
        $isArchive = ($rel -match '(^|[\\/])_archive[\\/]') -or ($rel -match '(^|[\\/])spec[\\/]history[\\/]')
        $ipHits = Select-String -Path $f -Pattern $ipRe -ErrorAction SilentlyContinue
        if ($ipHits) {
            $line0 = $ipHits[0].LineNumber
            if ($isArchive) {
                $archiveInfo += "${rel}:$line0 (real IP in frozen archive — INFO, out of public-leak scope)"
                return
            }
            $hasGuard = Select-String -Path $f -Pattern $guardRe -ErrorAction SilentlyContinue
            if ($hasGuard) { $guardedCount++ }
            else { $unguarded += "${rel}:$line0 (real IP, no leak/tailnet guard marker)" }
        }
    }
    $ok = ($unguarded.Count -eq 0)
    $ev = @()
    if ($ok) { $ev += "$guardedCount shippable doc file(s) mention a real IP; all carry a leak/tailnet guard marker" }
    else { $ev += $unguarded }
    foreach ($a in $archiveInfo) { $ev += $a }
    Record $id $name $ok $ev
}

# =============================================================================
# T16 — cargo-test cites in case DBs resolve to real fns (INV-16 ghost guard)
# =============================================================================
function Check-T16 {
    $id = 'T16'; $name = 'cargo-test cites resolve to real fns (no false-green)'
    $tcDir = Join-Path $DocsRoot 'test-cases'
    $coreTests = Join-Path $RepoRoot 'core/tests'
    if (-not (Test-Path $tcDir)) { Record $id $name $false @("missing: $tcDir"); return }
    $bad = @(); $checked = 0
    Get-ChildItem -Path $tcDir -Filter *.md -File | ForEach-Object {
        $rel = $_.FullName.Substring($RepoRoot.Length).TrimStart('\','/')
        $hits = Select-String -Path $_.FullName -Pattern 'cargo test --test\s+([A-Za-z0-9_-]+)\s+([A-Za-z0-9_]+)' -AllMatches -ErrorAction SilentlyContinue
        foreach ($h in $hits) {
            foreach ($m in $h.Matches) {
                $tfile  = $m.Groups[1].Value
                $filter = $m.Groups[2].Value
                $checked++
                $rsPath = Join-Path $coreTests "$tfile.rs"
                if (-not (Test-Path $rsPath)) {
                    $bad += "${rel}:$($h.LineNumber) cites --test $tfile but core/tests/$tfile.rs does not exist"
                    continue
                }
                # cargo test filters are SUBSTRING matches against fn names
                $fnNames = Select-String -Path $rsPath -Pattern '^\s*(pub\s+)?(async\s+)?fn\s+([A-Za-z0-9_]+)' -AllMatches -ErrorAction SilentlyContinue |
                           ForEach-Object { $_.Matches } | ForEach-Object { $_.Groups[3].Value }
                $matched = @($fnNames | Where-Object { $_ -like "*$filter*" })
                if ($matched.Count -eq 0) {
                    $bad += "${rel}:$($h.LineNumber) filter '$filter' matches 0 fn in core/tests/$tfile.rs (cargo runs 0 tests, exit 0 = false-green)"
                }
            }
        }
    }
    $ok = ($bad.Count -eq 0)
    $ev = if ($ok) { @("$checked cargo-test cite(s) across docs/test-cases all resolve to >=1 real fn (substring semantics)") } else { $bad }
    Record $id $name $ok $ev
}

# =============================================================================
# A1 — content-accuracy landmines (as-built drift REGRESSION guard)
#      Mermaid-compile proves a diagram RENDERS; it does NOT prove the text is TRUE.
#      This guards the specific false claims an as-built audit already corrected, so they
#      cannot silently reappear. A line that matches ALL `forbid` patterns and NONE of the
#      `allow` qualifiers = a re-introduced false claim → FAIL. Corrected wording carries an
#      allow-qualifier and passes. Extend $rules when a new audit finds a new landmine.
# =============================================================================
function Check-A1 {
    $id = 'A1'; $name = 'content-accuracy landmines (as-built drift regression)'
    $flowDir = Join-Path $DocsRoot 'superpowers/specs/2026-06-12-platform-flows-design'
    if (-not (Test-Path $flowDir)) { Record $id $name $false @("missing: $flowDir"); return }
    $rules = @(
        @{ tag = 'skill-panic';
           forbid = @('panic', '(skill store|skill_store|recall --semantic)');
           # allow = the corrected framing OR a legit non-user-facing context (test spec / fn-existence
           # check / landing plan / audit-history note) where mentioning the stub + panic is accurate.
           allow  = '不會 panic|guarded-unreachable|exit 2|verb 不存在|不存在|接上去|hit→crash|hit unimplemented|非今日 panic|永不裸 panic|前友善降級|cargo test|MEM-MAIN|實存|解 panic|切版後|落地|\[BROKEN\] 標|does not run|do not run|don''t work';
           truth  = 'skill store / recall --semantic are not real verbs (skill=new|run|help; recall has no --semantic); the unimplemented!() stubs are guarded-unreachable — users do NOT panic today' },
        @{ tag = 'api-chat-401';
           forbid = @('/api/chat', '401', '(broken|I3 violation|I3 broken)');
           allow  = 'NO LONGER|require_cluster_auth_local_ui|不再|by design|loopback';
           truth  = '/api/chat uses require_cluster_auth_local_ui (serve.rs:1236) — loopback/tailnet pass, only remote peers 401 (by design)' },
        @{ tag = 'sessions-401';
           forbid = @('sessions', '401', '(I3|broken)');
           allow  = 'by design|broker|Bearer|雲端|無 auth gate|永不回 401|不會回 401|已於.{0,6}修|exists,';
           truth  = 'phantom sessions queries the cloud broker (Bearer, by design); no token -> helpful message, not a 401; not an I3 local-plane violation' }
    )
    $bad = @(); $scanned = 0
    Get-ChildItem -Path $flowDir -Filter *.md -File | ForEach-Object {
        $rel = $_.FullName.Substring($RepoRoot.Length).TrimStart('\','/')
        $ln = 0
        foreach ($line in [System.IO.File]::ReadAllLines($_.FullName)) {
            $ln++
            foreach ($rule in $rules) {
                $hit = $true
                foreach ($f in $rule.forbid) { if ($line -notmatch $f) { $hit = $false; break } }
                if ($hit -and ($line -notmatch $rule.allow)) {
                    $bad += "${rel}:$ln [$($rule.tag)] reintroduced false claim -> $($rule.truth)"
                }
            }
        }
        $scanned++
    }
    $ok = ($bad.Count -eq 0)
    $tags = ($rules | ForEach-Object { $_.tag }) -join ', '
    $ev = if ($ok) { @("$scanned flow docs scanned; no reintroduced false claims ($($rules.Count) landmine rules: $tags)") } else { $bad }
    Record $id $name $ok $ev
}

# =============================================================================
# run
# =============================================================================
Check-C1
Check-C4
Check-C7
Check-C8
Check-K3
Check-K7
Check-T16
Check-A1

# --- report -------------------------------------------------------------------
Write-Host ""
Write-Host "==== check-doc-tree.ps1 — Documentation Charter §E machine lint ====" -ForegroundColor Cyan
Write-Host "repo: $RepoRoot"
Write-Host ""
$fail = 0
foreach ($r in $script:Results) {
    if ($r.Pass) {
        Write-Host ("[PASS] {0,-3} {1}" -f $r.Id, $r.Name) -ForegroundColor Green
    } else {
        $fail++
        Write-Host ("[FAIL] {0,-3} {1}" -f $r.Id, $r.Name) -ForegroundColor Red
    }
    foreach ($e in $r.Evidence) {
        Write-Host ("        - {0}" -f $e) -ForegroundColor DarkGray
    }
}
Write-Host ""
$total = $script:Results.Count
$passN = $total - $fail
if ($fail -eq 0) {
    Write-Host "RESULT: ALL GREEN ($passN/$total)" -ForegroundColor Green
    exit 0
} else {
    Write-Host "RESULT: $fail FAIL / $total checks ($passN passed)" -ForegroundColor Red
    exit 1
}
