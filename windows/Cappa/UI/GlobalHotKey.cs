using System;
using System.Windows.Interop;
using Cappa.Native;
using Cappa.Services;

namespace Cappa.UI;

/// <summary>A system-wide keyboard shortcut, received through a hidden message-only window.</summary>
internal sealed class GlobalHotKey : IDisposable
{
    private const int Id = 0xCA99;
    private readonly HwndSource _window;
    private readonly Action _action;

    /// <summary>Ctrl+Alt+Shift+R.</summary>
    public static GlobalHotKey? RegisterRecordShortcut(Action action) =>
        TryRegister(Win32.MOD_CONTROL | Win32.MOD_ALT | Win32.MOD_SHIFT, 0x52 /* R */, action);

    public static GlobalHotKey? TryRegister(uint modifiers, uint virtualKey, Action action)
    {
        var hotKey = new GlobalHotKey(action);
        if (Win32.RegisterHotKey(hotKey._window.Handle, Id, modifiers | Win32.MOD_NOREPEAT, virtualKey))
        {
            return hotKey;
        }
        Log.Write("Could not register the global hotkey (another app may be using it).");
        hotKey._window.Dispose();
        return null;
    }

    private GlobalHotKey(Action action)
    {
        _action = action;
        _window = new HwndSource(new HwndSourceParameters("CappaHotKey") { ParentWindow = Win32.HWND_MESSAGE, WindowStyle = 0 });
        _window.AddHook(WndProc);
    }

    private IntPtr WndProc(IntPtr hwnd, int msg, IntPtr wParam, IntPtr lParam, ref bool handled)
    {
        if (msg == Win32.WM_HOTKEY && wParam.ToInt32() == Id)
        {
            _action();
            handled = true;
        }
        return IntPtr.Zero;
    }

    public void Dispose()
    {
        Win32.UnregisterHotKey(_window.Handle, Id);
        _window.Dispose();
    }
}
