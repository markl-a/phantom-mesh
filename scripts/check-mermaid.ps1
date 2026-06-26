#!/usr/bin/env pwsh
#requires -version 5
<#
.SYNOPSIS
  check-mermaid.ps1 — Validate all Mermaid diagrams in spec files.

.DESCRIPTION
  Extracts Mermaid diagrams from Markdown files in docs/superpowers/specs/2026-06-12-platform-flows-design/
  and validates them using @mermaid-js/mermaid-cli.

.PARAMETER RepoRoot
  Repo root. Defaults to the parent of the scripts/ dir holding this file.
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

$ValidateDir = Join-Path $RepoRoot ".mmd-validate"
if (Test-Path $ValidateDir) {
    Remove-Item $ValidateDir -Recurse -Force
}
New-Item -ItemType Directory -Path $ValidateDir -Force | Out-Null

$PptrFile = Join-Path $ValidateDir "pptr.json"
'{"args":["--no-sandbox","--disable-gpu"]}' | Out-File -FilePath $PptrFile -Encoding utf8 -NoNewline

$FlowsDir = Join-Path $RepoRoot "docs/superpowers/specs/2026-06-12-platform-flows-design"
if (-not (Test-Path $FlowsDir)) {
    Write-Error "Design flows directory not found: $FlowsDir"
    exit 1
}

Write-Host "Extracting mermaid diagrams from $FlowsDir..." -ForegroundColor Cyan

$mdFiles = Get-ChildItem -Path $FlowsDir -Filter *.md -File
$diagramCount = 0

foreach ($file in $mdFiles) {
    $content = [System.IO.File]::ReadAllText($file.FullName)
    # Match ```mermaid\r?\n([\s\S]*?)```
    $regex = [regex]'(?ms)```mermaid\r?\n(.*?)\r?\n```'
    $matches = $regex.Matches($content)
    $index = 0
    foreach ($match in $matches) {
        $index++
        $diagramCount++
        $mermaidCode = $match.Groups[1].Value
        $baseName = $file.BaseName
        $outName = "$($baseName)__$($index.ToString('D2')).mmd"
        $outPath = Join-Path $ValidateDir $outName
        [System.IO.File]::WriteAllText($outPath, $mermaidCode, [System.Text.Encoding]::UTF8)
    }
}

Write-Host "Extracted $diagramCount diagrams. Running validation..." -ForegroundColor Cyan

$passed = 0
$failed = 0
$failedFiles = @()

$mmdFiles = Get-ChildItem -Path $ValidateDir -Filter *.mmd -File
foreach ($mmdFile in $mmdFiles) {
    $baseName = $mmdFile.BaseName
    $svgFile = Join-Path $ValidateDir "$baseName.svg"
    
    $processStartInfo = New-Object System.Diagnostics.ProcessStartInfo
    $mmdcArgs = "-y @mermaid-js/mermaid-cli@latest -p `"$PptrFile`" -i `"$($mmdFile.FullName)`" -o `"$svgFile`""
    if ($env:OS -eq 'Windows_NT') {
        # Windows: npx is npx.cmd, must go through cmd /c
        $processStartInfo.FileName = "cmd.exe"
        $processStartInfo.Arguments = "/c npx $mmdcArgs"
    } else {
        # macOS / Linux (incl. GitHub Actions ubuntu): npx resolves via PATH
        $processStartInfo.FileName = "npx"
        $processStartInfo.Arguments = $mmdcArgs
    }
    $processStartInfo.RedirectStandardOutput = $true
    $processStartInfo.RedirectStandardError = $true
    $processStartInfo.UseShellExecute = $false
    $processStartInfo.CreateNoWindow = $true
    
    $process = New-Object System.Diagnostics.Process
    $process.StartInfo = $processStartInfo
    
    $stdout = ""
    $stderr = ""
    
    try {
        [void]$process.Start()
        if ($process.WaitForExit(120000)) {
            $stdout = $process.StandardOutput.ReadToEnd()
            $stderr = $process.StandardError.ReadToEnd()
            $exitCode = $process.ExitCode
        } else {
            $process.Kill()
            $exitCode = -1
            $stderr = "Timeout waiting for mmdc to compile diagram"
        }
    } catch {
        $exitCode = -1
        $stderr = $_.Exception.Message
    }
    
    $hasParseError = $stderr -match "Parse error" -or $stdout -match "Parse error"
    if ($exitCode -ne 0 -or $hasParseError) {
        $failed++
        $failedFiles += $baseName
        Write-Host "FAIL: $baseName" -ForegroundColor Red
        if ($stderr) {
            Write-Host "  Error output: $stderr" -ForegroundColor DarkGray
        }
    } else {
        $passed++
        Write-Host "PASS: $baseName" -ForegroundColor Green
    }
}

Write-Host ""
Write-Host "==== check-mermaid.ps1 results ====" -ForegroundColor Cyan
Write-Host "Passed: $passed" -ForegroundColor Green
Write-Host "Failed: $failed" -ForegroundColor Red

if ($failed -gt 0) {
    Write-Host "Failed diagrams: $($failedFiles -join ', ')" -ForegroundColor Red
    exit 1
} else {
    Write-Host "All $passed diagrams compiled successfully!" -ForegroundColor Green
    exit 0
}
