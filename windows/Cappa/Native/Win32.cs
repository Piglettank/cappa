using System;
using System.Runtime.InteropServices;

namespace Cappa.Native;

internal static class Win32
{
    public const int WM_HOTKEY = 0x0312;

    public const uint MOD_ALT = 0x0001;
    public const uint MOD_CONTROL = 0x0002;
    public const uint MOD_SHIFT = 0x0004;
    public const uint MOD_NOREPEAT = 0x4000;

    public const int GWL_EXSTYLE = -20;
    public const int WS_EX_TRANSPARENT = 0x00000020;
    public const int WS_EX_TOOLWINDOW = 0x00000080;
    public const int WS_EX_NOACTIVATE = 0x08000000;

    public static readonly IntPtr HWND_TOPMOST = new(-1);
    public static readonly IntPtr HWND_MESSAGE = new(-3);
    public const uint SWP_NOACTIVATE = 0x0010;
    public const uint SWP_SHOWWINDOW = 0x0040;

    /// <summary>Hides the window from all screen capture, including Cappa's own recordings (Windows 10 2004+).</summary>
    public const uint WDA_EXCLUDEFROMCAPTURE = 0x00000011;

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool RegisterHotKey(IntPtr hWnd, int id, uint fsModifiers, uint vk);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool UnregisterHotKey(IntPtr hWnd, int id);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool SetWindowDisplayAffinity(IntPtr hWnd, uint dwAffinity);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int x, int y, int cx, int cy, uint uFlags);

    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")]
    private static extern IntPtr GetWindowLongPtr(IntPtr hWnd, int nIndex);

    [DllImport("user32.dll", EntryPoint = "SetWindowLongPtrW")]
    private static extern IntPtr SetWindowLongPtr(IntPtr hWnd, int nIndex, IntPtr dwNewLong);

    public static void AddExtendedStyle(IntPtr hwnd, int style)
    {
        var current = GetWindowLongPtr(hwnd, GWL_EXSTYLE).ToInt64();
        SetWindowLongPtr(hwnd, GWL_EXSTYLE, new IntPtr(current | (uint)style));
    }

    public static void ExcludeFromCapture(IntPtr hwnd)
    {
        if (!SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE))
        {
            Services.Log.Write($"SetWindowDisplayAffinity failed: {Marshal.GetLastWin32Error()}");
        }
    }
}
