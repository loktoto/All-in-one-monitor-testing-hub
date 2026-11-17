Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class DpiProbe {
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
$p = Get-Process hardware-hub -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $p) { Write-Output 'no window'; exit 1 }
$dpi = [DpiProbe]::GetDpiForWindow($p.MainWindowHandle)
$rect = New-Object DpiProbe+RECT
[DpiProbe]::GetClientRect($p.MainWindowHandle, [ref]$rect) | Out-Null
Write-Output ("DPI=$dpi scale=" + [math]::Round($dpi / 96.0, 2))
Write-Output ("client=" + ($rect.Right - $rect.Left) + "x" + ($rect.Bottom - $rect.Top))
$screen = [System.Windows.Forms.Screen]::PrimaryScreen
Write-Output ("screen=" + $screen.Bounds.Width + "x" + $screen.Bounds.Height + " working=" + $screen.WorkingArea.Width + "x" + $screen.WorkingArea.Height)
