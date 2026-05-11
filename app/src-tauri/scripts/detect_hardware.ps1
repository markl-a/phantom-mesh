# detect_hardware.ps1 — Unified GPU + NPU detection for Windows
# Returns JSON: { "gpus": [...], "npus": [...] }
# GPU strategies: Registry qwMemorySize → WMI (fallback)
# NPU strategy:   PNPClass "ComputeAccelerator"

param()

# ── GPU Detection ──────────────────────────────────────────

function Get-RegistryVram {
    $results = @()
    $basePath = "HKLM:\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}"

    for ($i = 0; $i -lt 10; $i++) {
        $subKey = "{0:D4}" -f $i
        $path = "$basePath\$subKey"
        if (-not (Test-Path $path)) { continue }

        $props = Get-ItemProperty $path -ErrorAction SilentlyContinue
        if ($null -eq $props) { continue }

        $name = $props.'DriverDesc'
        if ([string]::IsNullOrWhiteSpace($name)) { continue }
        if ($name -match 'Microsoft Basic|Software') { continue }

        $vramBytes = 0
        # Try qwMemorySize first (QWORD, supports >4GB for APU shared memory)
        $qw = $props.'HardwareInformation.qwMemorySize'
        if ($null -ne $qw) {
            $vramBytes = [uint64]$qw
        } else {
            # Fallback to MemorySize (DWORD, max ~4GB)
            $ms = $props.'HardwareInformation.MemorySize'
            if ($null -ne $ms) { $vramBytes = [uint64]$ms }
        }

        $results += @{
            name         = $name
            dedicated_mb = [math]::Round($vramBytes / 1MB)
            shared_mb    = 0
        }
    }

    return $results
}

function Get-WmiGpu {
    $results = @()
    $gpus = Get-CimInstance Win32_VideoController -ErrorAction SilentlyContinue
    foreach ($gpu in $gpus) {
        if ($gpu.Name -match 'Microsoft Basic|Software') { continue }
        $results += @{
            name         = $gpu.Name
            dedicated_mb = [math]::Round($gpu.AdapterRAM / 1MB)
            shared_mb    = 0
        }
    }
    return $results
}

# ── NPU Detection ──────────────────────────────────────────

function Get-NpuDevices {
    $results = @()
    $devices = Get-CimInstance Win32_PnPEntity -ErrorAction SilentlyContinue |
        Where-Object { $_.PNPClass -eq 'ComputeAccelerator' }

    foreach ($dev in $devices) {
        if ($null -eq $dev.Name) { continue }

        # Extract VEN and DEV from DeviceID for TOPS lookup
        $vendorId = ""
        $deviceId = ""
        if ($dev.DeviceID -match 'VEN_([0-9A-Fa-f]{4})') { $vendorId = $Matches[1] }
        if ($dev.DeviceID -match 'DEV_([0-9A-Fa-f]{4})') { $deviceId = $Matches[1] }

        $results += @{
            name      = $dev.Name.Trim()
            device_id = "$($dev.DeviceID)"
            vendor    = $vendorId
            device    = $deviceId
        }
    }
    return $results
}

# ── Execute ────────────────────────────────────────────────

$gpus = Get-RegistryVram
if ($null -eq $gpus -or $gpus.Count -eq 0) {
    $gpus = Get-WmiGpu
}
if ($null -eq $gpus -or $gpus.Count -eq 0) {
    $gpus = @(@{ name = "CPU-only"; dedicated_mb = 0; shared_mb = 0 })
}

$npus = Get-NpuDevices
if ($null -eq $npus) { $npus = @() }

# Ensure arrays (PowerShell quirk: single item becomes plain object)
$gpuArray = @($gpus)
$npuArray = @($npus)

@{ gpus = $gpuArray; npus = $npuArray } | ConvertTo-Json -Depth 3 -Compress
