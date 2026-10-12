using System;
using System.IO;

namespace Cappa.Services;

/// <summary>
/// A plain text log at %LOCALAPPDATA%\Cappa\cappa.log (rolled over at 2 MB), so problems on a
/// machine nobody is debugging can still be diagnosed. "Open Log" in the tray menu shows it.
/// </summary>
internal static class Log
{
    private static readonly object Gate = new();

    public static string FilePath { get; } = Path.Combine(AppPaths.DataFolder, "cappa.log");

    public static void Write(string message)
    {
        try
        {
            lock (Gate)
            {
                var info = new FileInfo(FilePath);
                if (info.Exists && info.Length > 2_000_000)
                {
                    File.Move(FilePath, FilePath + ".old", overwrite: true);
                }
                File.AppendAllText(FilePath, $"{DateTime.Now:yyyy-MM-dd HH:mm:ss.fff}  {message}{Environment.NewLine}");
            }
        }
        catch
        {
            // Logging must never take the app down.
        }
    }

    public static void Error(string context, Exception exception) => Write($"ERROR {context}: {exception}");
}

internal static class AppPaths
{
    /// <summary>%LOCALAPPDATA%\Cappa: log, temporary recording parts, editor caches.</summary>
    public static string DataFolder { get; } = Create(Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "Cappa"));

    /// <summary>%APPDATA%\Cappa: settings.</summary>
    public static string SettingsFolder { get; } = Create(Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "Cappa"));

    public static string TempFolder(string name) => Create(Path.Combine(DataFolder, "Temp", name));

    private static string Create(string path)
    {
        Directory.CreateDirectory(path);
        return path;
    }
}
