Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class WinEnum {
    public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr lParam);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint pid);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr hWnd, StringBuilder s, int n);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr hWnd, StringBuilder s, int n);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT r);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr hWnd);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }

    public static List<string> List(uint target) {
        var rows = new List<string>();
        EnumWindows((h, l) => {
            uint pid; GetWindowThreadProcessId(h, out pid);
            if (pid != target) return true;
            var t = new StringBuilder(512); GetWindowTextW(h, t, 512);
            var c = new StringBuilder(256); GetClassNameW(h, c, 256);
            RECT r; GetWindowRect(h, out r);
            rows.Add(string.Format("hwnd={0} visible={1} class={2} title='{3}' rect={4},{5} {6}x{7} dpi={8}",
                h, IsWindowVisible(h), c, t, r.Left, r.Top, r.Right - r.Left, r.Bottom - r.Top, GetDpiForWindow(h)));
            return true;
        }, IntPtr.Zero);
        return rows;
    }
}
"@
$p = Get-Process hardware-hub -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $p) { Write-Output 'no process'; exit 1 }
Write-Output ("pid=" + $p.Id + " responding=" + $p.Responding)
[WinEnum]::List([uint32]$p.Id) | ForEach-Object { Write-Output $_ }
