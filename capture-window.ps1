param(
    [string]$ProcessName = 'hardware-hub',
    [string]$OutFile = 'shot.png',
    [int]$DelayMs = 2500,
    [int]$Width = 0,
    [int]$Height = 0
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win32Cap {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
    public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hWnd, System.Text.StringBuilder s, int n);
    public static IntPtr Find(uint pid, string title) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((h, l) => {
            uint wpid; GetWindowThreadProcessId(h, out wpid);
            if (wpid != pid) return true;
            var t = new System.Text.StringBuilder(512); GetWindowTextW(h, t, 512);
            if (t.ToString() == title) { found = h; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

[Win32Cap]::SetProcessDPIAware() | Out-Null
$proc = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { Write-Output "NO PROCESS for $ProcessName"; exit 2 }
$hwnd = [Win32Cap]::Find([uint32]$proc.Id, 'Hardware Hub')
if ($hwnd -eq [IntPtr]::Zero) { Write-Output 'NO Hardware Hub WINDOW'; exit 2 }
[Win32Cap]::ShowWindow($hwnd, 9) | Out-Null   # SW_RESTORE
if ($Width -gt 0 -and $Height -gt 0) {
    [Win32Cap]::SetWindowPos($hwnd, [IntPtr](-1), 0, 0, $Width, $Height, 0x0040) | Out-Null
} else {
    [Win32Cap]::SetWindowPos($hwnd, [IntPtr](-1), 0, 0, 0, 0, 0x0001 -bor 0x0002 -bor 0x0040) | Out-Null
}
[Win32Cap]::SetForegroundWindow($hwnd) | Out-Null
Start-Sleep -Milliseconds $DelayMs

$rect = New-Object Win32Cap+RECT
[Win32Cap]::GetWindowRect($hwnd, [ref]$rect) | Out-Null
$w = $rect.Right - $rect.Left
$h = $rect.Bottom - $rect.Top
Write-Output "window $w x $h at $($rect.Left),$($rect.Top)"

$bmp = New-Object System.Drawing.Bitmap $w, $h
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
$ok = [Win32Cap]::PrintWindow($hwnd, $hdc, 2)   # PW_RENDERFULLCONTENT
$g.ReleaseHdc($hdc)
$g.Dispose()
$bmp.Save($OutFile, [System.Drawing.Imaging.ImageFormat]::Png)

# Report how much of the capture is non-black, to detect an OpenGL-only window.
$sample = New-Object System.Drawing.Bitmap $bmp
$nonBlack = 0
for ($y = 0; $y -lt $h; $y += 20) {
    for ($x = 0; $x -lt $w; $x += 20) {
        $c = $sample.GetPixel($x, $y)
        if ($c.R + $c.G + $c.B -gt 24) { $nonBlack++ }
    }
}
$sample.Dispose()
Write-Output "PrintWindow=$ok saved $OutFile nonBlackSamples=$nonBlack"
