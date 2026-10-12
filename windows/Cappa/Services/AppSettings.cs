using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.CompilerServices;
using System.Text.Json;

namespace Cappa.Services;

/// <summary>User settings, saved as JSON in %APPDATA%\Cappa\settings.json whenever one changes.</summary>
internal sealed class AppSettings : INotifyPropertyChanged
{
    private static readonly string FilePath = Path.Combine(AppPaths.SettingsFolder, "settings.json");
    private static bool _loading;

    public static string DefaultRecordingsFolder { get; } = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.MyVideos), "Cappa");

    private bool _recordSystemAudio = true;
    private bool _recordMicrophone;
    private string _microphoneId = "";
    private string _recordingsFolder = DefaultRecordingsFolder;
    private bool _showOverlay = true;
    private double? _overlayLeft;
    private double? _overlayTop;

    public event PropertyChangedEventHandler? PropertyChanged;

    public bool RecordSystemAudio { get => _recordSystemAudio; set => Set(ref _recordSystemAudio, value); }
    public bool RecordMicrophone { get => _recordMicrophone; set => Set(ref _recordMicrophone, value); }
    /// <summary>Core Audio endpoint ID of the chosen microphone; empty means the Windows default input.</summary>
    public string MicrophoneId { get => _microphoneId; set => Set(ref _microphoneId, value ?? ""); }
    public string RecordingsFolder { get => _recordingsFolder; set => Set(ref _recordingsFolder, value); }
    public bool ShowOverlay { get => _showOverlay; set => Set(ref _showOverlay, value); }
    public double? OverlayLeft { get => _overlayLeft; set => Set(ref _overlayLeft, value); }
    public double? OverlayTop { get => _overlayTop; set => Set(ref _overlayTop, value); }

    public static AppSettings Load()
    {
        try
        {
            if (File.Exists(FilePath))
            {
                _loading = true; // the property setters run during deserialization; don't save from them
                var loaded = JsonSerializer.Deserialize<AppSettings>(File.ReadAllText(FilePath));
                _loading = false;
                if (loaded is not null)
                {
                    if (string.IsNullOrWhiteSpace(loaded.RecordingsFolder))
                    {
                        loaded.RecordingsFolder = DefaultRecordingsFolder;
                    }
                    return loaded;
                }
            }
        }
        catch (Exception e)
        {
            Log.Error("Loading settings", e);
        }
        finally
        {
            _loading = false;
        }
        return new AppSettings();
    }

    private void Save()
    {
        try
        {
            File.WriteAllText(FilePath, JsonSerializer.Serialize(this, new JsonSerializerOptions { WriteIndented = true }));
        }
        catch (Exception e)
        {
            Log.Error("Saving settings", e);
        }
    }

    private void Set<T>(ref T field, T value, [CallerMemberName] string name = "")
    {
        if (Equals(field, value)) return;
        field = value;
        if (_loading) return;
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
        Save();
    }
}
