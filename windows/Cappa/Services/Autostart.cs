using System;
using Microsoft.Win32;

namespace Cappa.Services;

/// <summary>
/// Whether Cappa starts when you sign in, via the per-user Run key. The registry is the source of truth
/// (the user can also turn it off in Task Manager → Startup apps), so it isn't kept in settings.json.
/// </summary>
internal static class Autostart
{
    private const string RunKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string ValueName = "Cappa";

    public static bool IsEnabled
    {
        get
        {
            try
            {
                using var key = Registry.CurrentUser.OpenSubKey(RunKey);
                return key?.GetValue(ValueName) is string;
            }
            catch (Exception e)
            {
                Log.Error("Reading autostart", e);
                return false;
            }
        }
        set
        {
            try
            {
                using var key = Registry.CurrentUser.CreateSubKey(RunKey);
                if (value)
                {
                    key.SetValue(ValueName, $"\"{Environment.ProcessPath}\"");
                }
                else
                {
                    key.DeleteValue(ValueName, throwOnMissingValue: false);
                }
            }
            catch (Exception e)
            {
                Log.Error("Changing autostart", e);
            }
        }
    }
}
