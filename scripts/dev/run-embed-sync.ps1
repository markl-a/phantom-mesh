# scripts/run-embed-sync.ps1
# Wrapper script for embed-pipeline scheduling.
# Task Scheduler should call this instead of the old src/ppi/cli.py path.
$ErrorActionPreference = "Stop"
$timestamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
Write-Host "[$timestamp] Starting embed sync..."

try {
    if (Get-Command clawtex -ErrorAction SilentlyContinue) {
        clawtex embed-pipeline
    } else {
        python -m clawtex.cli embed-pipeline
    }
    Write-Host "[$timestamp] Embed sync completed successfully."
} catch {
    Write-Host "[$timestamp] ERROR: $_"
    exit 1
}
