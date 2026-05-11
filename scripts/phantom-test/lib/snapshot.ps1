# snapshot.ps1 — capture screen pixels to PNG.
#
# Two modes:
#   .\snapshot.ps1                              full virtual desktop (all monitors)
#   .\snapshot.ps1 -Window <window-title-substr> just that window's pixels
#                                                (case-insensitive substring match)
#
# Output: writes PNG to $env:USERPROFILE\.phantom-mesh\snapshots\<timestamp>.png
#         and prints the path to stdout (one line).
#
# Use from bash scenarios via:
#   snap=$(powershell -ExecutionPolicy Bypass -File lib/snapshot.ps1 -Window phantom)
#
# Notes:
#   - The Window mode finds the topmost window matching the title substring.
#   - If the matched window is minimized or off-screen, capture may be empty.
#   - For TUI capture, ensure phantom's Windows Terminal pane is visible.

[CmdletBinding()]
param(
    [string]$Window = '',
    [string]$OutDir = (Join-Path $env:USERPROFILE '.phantom-mesh\snapshots')
)

Add-Type -AssemblyName System.Windows.Forms, System.Drawing

if (-not (Test-Path $OutDir)) {
    New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
}

$ts = Get-Date -Format 'yyyyMMdd-HHmmss'
$tag = if ($Window) { ($Window -replace '[^A-Za-z0-9]', '_') } else { 'desktop' }
$out = Join-Path $OutDir ("snap-{0}-{1}.png" -f $ts, $tag)

if ($Window) {
    Add-Type @"
using System;
using System.Runtime.InteropServices;
public struct RECT { public int Left, Top, Right, Bottom; }
public class Win32 {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetWindowTextLength(IntPtr hWnd);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetWindowText(IntPtr hWnd, System.Text.StringBuilder text, int count);
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc enumProc, IntPtr lParam);
}
"@ -ErrorAction SilentlyContinue

    $script:found = [IntPtr]::Zero
    $script:foundRect = New-Object RECT
    $needle = $Window.ToLower()

    $callback = [Win32+EnumWindowsProc]{
        param($hWnd, $lParam)
        if (-not [Win32]::IsWindowVisible($hWnd)) { return $true }
        $len = [Win32]::GetWindowTextLength($hWnd)
        if ($len -le 0) { return $true }
        $sb = New-Object System.Text.StringBuilder ($len + 1)
        [Win32]::GetWindowText($hWnd, $sb, $sb.Capacity) | Out-Null
        if ($sb.ToString().ToLower().Contains($needle)) {
            $rect = New-Object RECT
            [Win32]::GetWindowRect($hWnd, [ref]$rect) | Out-Null
            if (($rect.Right - $rect.Left) -gt 100 -and ($rect.Bottom - $rect.Top) -gt 100) {
                $script:found = $hWnd
                $script:foundRect = $rect
                return $false
            }
        }
        return $true
    }
    [Win32]::EnumWindows($callback, [IntPtr]::Zero) | Out-Null

    if ($script:found -eq [IntPtr]::Zero) {
        Write-Error "no visible window matched: $Window"
        exit 2
    }
    $r = $script:foundRect
    $w = $r.Right - $r.Left
    $h = $r.Bottom - $r.Top
    $bmp = New-Object System.Drawing.Bitmap $w, $h
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen((New-Object System.Drawing.Point $r.Left, $r.Top),
                      [System.Drawing.Point]::Empty,
                      (New-Object System.Drawing.Size $w, $h))
} else {
    $b = [System.Windows.Forms.SystemInformation]::VirtualScreen
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
}

$bmp.Save($out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Output $out
