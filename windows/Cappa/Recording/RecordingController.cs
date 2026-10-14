using System;
using System.Diagnostics;
using System.IO;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Threading;
using Cappa.Editor;
using Cappa.Services;
using Cappa.UI;

namespace Cappa.Recording;

internal enum RecordingState { Idle, Selecting, Starting, Recording, Paused, Stopping }

/// <summary>App-level recording state shared by the tray menu, the hotkey and the overlay. UI thread only.</summary>
internal sealed class RecordingController
{
    private readonly RegionSelector _regionSelector = new();
    private readonly DispatcherTimer _timer;
    private readonly Stopwatch _elapsed = new();
    private RecordingSession? _session;
    private CaptureTarget? _target;
    private RecordingOverlay? _overlay;
    private RegionFrame? _regionFrame;
    private ClickHighlighter? _clickHighlighter;

    public AppSettings Settings { get; }
    public RecordingState State { get; private set; } = RecordingState.Idle;
    public bool IsActive => State is RecordingState.Recording or RecordingState.Paused;
    public TimeSpan Elapsed => _elapsed.Elapsed;

    /// <summary>Raised on the UI thread whenever <see cref="State"/> changes.</summary>
    public event Action? StateChanged;

    public RecordingController(AppSettings settings)
    {
        Settings = settings;
        _timer = new DispatcherTimer { Interval = TimeSpan.FromMilliseconds(250) };
        _timer.Tick += (_, _) => _overlay?.Update(State, Elapsed);
        Settings.PropertyChanged += (_, e) =>
        {
            if (e.PropertyName == nameof(AppSettings.ShowOverlay)) UpdateOverlay();
        };
    }

    /// <summary>Hotkey behaviour: start a region recording, cancel an in-progress selection, or stop.</summary>
    public async Task ToggleRecordingAsync()
    {
        switch (State)
        {
            case RecordingState.Idle: await StartRegionRecordingAsync(); break;
            case RecordingState.Selecting: _regionSelector.Cancel(); break;
            case RecordingState.Recording or RecordingState.Paused: await StopAsync(); break;
        }
    }

    public async Task StartRegionRecordingAsync()
    {
        if (State != RecordingState.Idle) return;
        SetState(RecordingState.Selecting);
        var target = await _regionSelector.SelectAsync();
        SetState(RecordingState.Idle);
        if (target is not null) await BeginRecordingAsync(target);
    }

    public void TogglePause()
    {
        if (_session is null) return;
        if (State == RecordingState.Recording)
        {
            _session.Pause();
            _elapsed.Stop();
            SetState(RecordingState.Paused);
        }
        else if (State == RecordingState.Paused)
        {
            _session.Resume();
            _elapsed.Start();
            SetState(RecordingState.Recording);
        }
        _overlay?.Update(State, Elapsed);
    }

    public async Task StopAsync()
    {
        if (!IsActive || _session is not { } session) return;
        SetState(RecordingState.Stopping);
        _timer.Stop();
        _elapsed.Stop();
        CloseRecordingWindows();

        var outputPath = Path.Combine(Settings.RecordingsFolder, $"Cappa {DateTime.Now:yyyy-MM-dd 'at' HH.mm.ss}.mp4");
        try
        {
            Directory.CreateDirectory(Settings.RecordingsFolder);
            await session.StopAsync(outputPath);
            Log.Write($"Saved {outputPath}");
            EditorWindow.Open(outputPath);
        }
        catch (Exception e)
        {
            Log.Error("Saving recording", e);
            ShowError("Couldn't save the recording", e.Message);
        }
        finally
        {
            session.Dispose();
            _session = null;
            _target = null;
            SetState(RecordingState.Idle);
        }
    }

    public void OpenRecordingsFolder()
    {
        Directory.CreateDirectory(Settings.RecordingsFolder);
        Process.Start(new ProcessStartInfo(Settings.RecordingsFolder) { UseShellExecute = true });
    }

    public void OpenVideoInEditor()
    {
        var dialog = new Microsoft.Win32.OpenFileDialog
        {
            Title = "Open Video in Cappa",
            Filter = "Videos|*.mp4;*.mov;*.m4v;*.mkv|All files|*.*",
            InitialDirectory = Directory.Exists(Settings.RecordingsFolder) ? Settings.RecordingsFolder : null,
        };
        if (dialog.ShowDialog() == true) EditorWindow.Open(dialog.FileName);
    }

    private async Task BeginRecordingAsync(CaptureTarget target)
    {
        SetState(RecordingState.Starting);
        try
        {
            var session = await RecordingSession.StartAsync(target, Settings);
            session.Failed += error => Application.Current.Dispatcher.InvokeAsync(() => OnSessionFailed(error));
            _session = session;
            _target = target;
            _elapsed.Restart();
            _timer.Start();
            SetState(RecordingState.Recording);
            UpdateOverlay();
            if (target.Region is not null)
            {
                _regionFrame = new RegionFrame(target.ScreenArea);
                _regionFrame.Show();
            }
            if (Settings.ShowCursor && Settings.HighlightClicks)
            {
                _clickHighlighter = new ClickHighlighter(target.MonitorBounds);
                _clickHighlighter.Show();
            }
        }
        catch (Exception e)
        {
            Log.Error("Starting recording", e);
            SetState(RecordingState.Idle);
            ShowError("Couldn't start recording", e.Message);
        }
    }

    private async void OnSessionFailed(string error)
    {
        if (!IsActive) return;
        ShowError("Recording stopped unexpectedly", error);
        await StopAsync();
    }

    private void UpdateOverlay()
    {
        if (IsActive && Settings.ShowOverlay && _target is { } target)
        {
            _overlay ??= new RecordingOverlay(this, target);
            _overlay.Update(State, Elapsed);
            _overlay.Show();
        }
        else
        {
            _overlay?.Close();
            _overlay = null;
        }
    }

    private void CloseRecordingWindows()
    {
        _overlay?.Close();
        _overlay = null;
        _regionFrame?.Close();
        _regionFrame = null;
        _clickHighlighter?.Close();
        _clickHighlighter = null;
    }

    private void SetState(RecordingState state)
    {
        if (State == state) return;
        State = state;
        StateChanged?.Invoke();
    }

    private static void ShowError(string title, string message) =>
        MessageBox.Show(message, title, MessageBoxButton.OK, MessageBoxImage.Warning);
}
