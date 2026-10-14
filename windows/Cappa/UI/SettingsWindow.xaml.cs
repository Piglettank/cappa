using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Diagnostics;
using System.IO;
using System.Windows;
using Cappa.Services;
using NAudio.CoreAudioApi;

namespace Cappa.UI;

internal sealed record MicrophoneOption(string Id, string Name);

/// <summary>Settings: start Cappa when Windows starts, system audio and microphone, which microphone, video options, and where recordings are saved.</summary>
internal partial class SettingsWindow : Window
{
    private static SettingsWindow? _shared;
    private readonly AppSettings _settings;
    private bool _updating;

    public static void ShowSingleton(AppSettings settings)
    {
        _shared ??= new SettingsWindow(settings);
        _shared.Show();
        _shared.Activate();
    }

    private SettingsWindow(AppSettings settings)
    {
        _settings = settings;
        InitializeComponent();

        AutostartCheck.Click += (_, _) =>
        {
            Autostart.IsEnabled = AutostartCheck.IsChecked == true;
            AutostartCheck.IsChecked = Autostart.IsEnabled;
        };
        SystemAudioCheck.Click += (_, _) => _settings.RecordSystemAudio = SystemAudioCheck.IsChecked == true;
        MicrophoneCheck.Click += (_, _) => _settings.RecordMicrophone = MicrophoneCheck.IsChecked == true;
        MicrophoneCombo.SelectionChanged += (_, _) =>
        {
            if (!_updating && MicrophoneCombo.SelectedValue is string id) _settings.MicrophoneId = id;
        };

        QualityCombo.SelectionChanged += (_, _) =>
        {
            if (!_updating && Enum.TryParse<RecordingQuality>(QualityCombo.SelectedValue as string, out var quality)) _settings.Quality = quality;
        };
        FrameRateCombo.SelectionChanged += (_, _) =>
        {
            if (!_updating && int.TryParse(FrameRateCombo.SelectedValue as string, out var fps)) _settings.FrameRate = fps;
        };
        ShowCursorCheck.Click += (_, _) => _settings.ShowCursor = ShowCursorCheck.IsChecked == true;
        HighlightClicksCheck.Click += (_, _) => _settings.HighlightClicks = HighlightClicksCheck.IsChecked == true;

        _settings.PropertyChanged += OnSettingChanged;
        // Devices come and go; refresh the list whenever the window is brought forward.
        // The user may also have changed autostart in Task Manager meanwhile.
        Activated += (_, _) =>
        {
            RefreshMicrophones();
            AutostartCheck.IsChecked = Autostart.IsEnabled;
        };
        Closed += (_, _) =>
        {
            _settings.PropertyChanged -= OnSettingChanged;
            _shared = null;
        };
        UpdateFromSettings();
    }

    private void OnSettingChanged(object? sender, PropertyChangedEventArgs e) => Dispatcher.Invoke(UpdateFromSettings);

    private void UpdateFromSettings()
    {
        _updating = true;
        SystemAudioCheck.IsChecked = _settings.RecordSystemAudio;
        MicrophoneCheck.IsChecked = _settings.RecordMicrophone;
        MicrophoneCombo.IsEnabled = _settings.RecordMicrophone;
        MicrophoneCombo.SelectedValue = _settings.MicrophoneId;
        QualityCombo.SelectedValue = _settings.Quality.ToString();
        FrameRateCombo.SelectedValue = _settings.FrameRate.ToString();
        ShowCursorCheck.IsChecked = _settings.ShowCursor;
        HighlightClicksCheck.IsChecked = _settings.HighlightClicks;
        HighlightClicksCheck.IsEnabled = _settings.ShowCursor;
        FolderText.Text = _settings.RecordingsFolder;
        DefaultFolderButton.IsEnabled = !string.Equals(
            Path.GetFullPath(_settings.RecordingsFolder), Path.GetFullPath(AppSettings.DefaultRecordingsFolder),
            StringComparison.OrdinalIgnoreCase);
        _updating = false;
    }

    private void RefreshMicrophones()
    {
        var options = new List<MicrophoneOption>();
        try
        {
            using var enumerator = new MMDeviceEnumerator();
            string? defaultName = null;
            try
            {
                defaultName = enumerator.GetDefaultAudioEndpoint(DataFlow.Capture, Role.Console).FriendlyName;
            }
            catch (Exception)
            {
                // No input devices at all.
            }
            options.Add(new MicrophoneOption("", defaultName is null ? "System default" : $"System default ({defaultName})"));

            var found = false;
            foreach (var device in enumerator.EnumerateAudioEndPoints(DataFlow.Capture, DeviceState.Active))
            {
                options.Add(new MicrophoneOption(device.ID, device.FriendlyName));
                found |= device.ID == _settings.MicrophoneId;
            }
            if (!found && !string.IsNullOrEmpty(_settings.MicrophoneId))
            {
                options.Add(new MicrophoneOption(_settings.MicrophoneId, "Selected microphone (not connected)"));
            }
        }
        catch (Exception e)
        {
            Log.Error("Listing microphones", e);
        }

        _updating = true;
        MicrophoneCombo.ItemsSource = options;
        MicrophoneCombo.SelectedValue = _settings.MicrophoneId;
        _updating = false;
    }

    private void OnChooseFolder(object sender, RoutedEventArgs e)
    {
        var dialog = new Microsoft.Win32.OpenFolderDialog
        {
            Title = "Choose where Cappa saves new recordings",
            InitialDirectory = Directory.Exists(_settings.RecordingsFolder) ? _settings.RecordingsFolder : null,
        };
        if (dialog.ShowDialog(this) == true) _settings.RecordingsFolder = dialog.FolderName;
    }

    private void OnShowFolder(object sender, RoutedEventArgs e)
    {
        Directory.CreateDirectory(_settings.RecordingsFolder);
        Process.Start(new ProcessStartInfo(_settings.RecordingsFolder) { UseShellExecute = true });
    }

    private void OnDefaultFolder(object sender, RoutedEventArgs e) =>
        _settings.RecordingsFolder = AppSettings.DefaultRecordingsFolder;
}
