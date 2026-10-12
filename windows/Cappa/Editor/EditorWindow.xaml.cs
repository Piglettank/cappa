using System;
using System.Collections.Generic;
using System.Collections.Specialized;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Threading;
using Cappa.Services;

namespace Cappa.Editor;

/// <summary>
/// One editor window per video: preview, toolbar, and a timeline with a video lane and one lane per audio
/// track. Split/trim/delete apply to all lanes; each audio track has its own mute and volume.
/// </summary>
internal partial class EditorWindow : Window
{
    private static readonly List<EditorWindow> OpenEditors = new();

    private readonly EditorModel _model;
    private readonly EditorPlayback _playback;
    private readonly string _workFolder;
    private readonly DispatcherTimer _noticeTimer = new() { Interval = TimeSpan.FromSeconds(2) };
    private bool _busy;

    public static void Open(string path)
    {
        var fullPath = Path.GetFullPath(path);
        var existing = OpenEditors.FirstOrDefault(e => string.Equals(e._model.SourcePath, fullPath, StringComparison.OrdinalIgnoreCase));
        if (existing is not null)
        {
            existing.Activate();
            return;
        }
        var editor = new EditorWindow(fullPath);
        OpenEditors.Add(editor);
        editor.Show();
        editor.Activate();
    }

    private EditorWindow(string path)
    {
        InitializeComponent();
        Title = Path.GetFileNameWithoutExtension(path) + " — Cappa";
        _model = new EditorModel(path);
        _playback = new EditorPlayback(Video, _model);
        _workFolder = AppPaths.TempFolder("Editor-" + Guid.NewGuid().ToString("N"));

        Timeline.Attach(_model, _playback);
        _model.Changed += UpdateControls;
        _playback.Updated += UpdatePlayState;
        _noticeTimer.Tick += (_, _) =>
        {
            _noticeTimer.Stop();
            Notice.Visibility = Visibility.Collapsed;
        };

        PreviewKeyDown += OnPreviewKeyDown;
        Loaded += async (_, _) => await LoadAsync();
        Closing += OnClosing;
        Closed += (_, _) => OnClosed();
    }

    // MARK: - Loading

    private async Task LoadAsync()
    {
        SetBusy("Loading…", null);
        try
        {
            var info = await Ffmpeg.ProbeAsync(_model.SourcePath);
            _model.Load(info);
            AudioHeaders.ItemsSource = _model.AudioTracks;
            Video.Source = new Uri(_model.SourcePath);
            Video.Pause(); // shows the first frame (Manual loaded behaviour)
            ClearBusy();
            UpdateControls();
        }
        catch (Exception e)
        {
            Log.Error("Opening video in editor", e);
            ClearBusy();
            MessageBox.Show(this, $"Cappa couldn't open this video.\n\n{e.Message}", "Cappa", MessageBoxButton.OK, MessageBoxImage.Warning);
            Close();
            return;
        }

        // Thumbnails and audio come in the background; the editor is usable meanwhile.
        try
        {
            _model.Thumbnails = await MediaPreparation.ThumbnailsAsync(_model.SourcePath, _model.Duration, _workFolder);
            _model.RaiseChanged();
        }
        catch (Exception e)
        {
            Log.Error("Creating thumbnails", e);
        }

        try
        {
            foreach (var track in _model.AudioTracks)
            {
                track.WavPath = await MediaPreparation.ExtractAudioAsync(_model.SourcePath, track.StreamIndex, _workFolder);
                var wavPath = track.WavPath;
                track.Waveform = await Task.Run(() => MediaPreparation.Waveform(wavPath, _model.Duration));
                _model.RaiseChanged();
            }
            _playback.AttachAudio();
        }
        catch (Exception e)
        {
            Log.Error("Preparing audio", e);
        }
    }

    // MARK: - Toolbar

    private void OnUndo(object sender, RoutedEventArgs e) => _model.Undo();

    private void OnRedo(object sender, RoutedEventArgs e) => _model.Redo();

    private void OnSplit(object sender, RoutedEventArgs e) => _model.SplitAt(_model.CurrentTime);

    private void OnDelete(object sender, RoutedEventArgs e) => _model.DeleteSelected();

    private void OnPlay(object sender, RoutedEventArgs e) => _playback.TogglePlay();

    private void OnVideoClick(object sender, MouseButtonEventArgs e) => _playback.TogglePlay();

    private void OnShowInExplorer(object sender, RoutedEventArgs e) =>
        Process.Start("explorer.exe", $"/select,\"{_model.SourcePath}\"");

    private void OnToggleMute(object sender, RoutedEventArgs e)
    {
        if (sender is FrameworkElement { Tag: AudioTrack track }) _model.ToggleMute(track);
    }

    private void OnExportMenu(object sender, RoutedEventArgs e)
    {
        var menu = ExportButton.ContextMenu!;
        menu.PlacementTarget = ExportButton;
        menu.Placement = PlacementMode.Bottom;
        menu.IsOpen = true;
    }

    private void UpdateControls()
    {
        // These stay enabled (so their tooltips always show); unavailable actions are ignored by the model.
        UndoButton.Opacity = _model.CanUndo ? 1 : 0.4;
        RedoButton.Opacity = _model.CanRedo ? 1 : 0.4;
        SplitButton.Opacity = _model.CanSplitAt(_model.CurrentTime) ? 1 : 0.4;
        DeleteButton.Opacity = _model.CanDelete ? 1 : 0.4;
        LengthText.Text = $"Length {TimeFormat.Precise(_model.EditedDuration)}";
        UpdatePlayState();
    }

    private void UpdatePlayState()
    {
        PlayGlyph.Text = _playback.IsPlaying ? "" : ""; // Pause : Play
        TimeText.Text = TimeFormat.Precise(_model.CurrentTime);
        SplitButton.Opacity = _model.CanSplitAt(_model.CurrentTime) ? 1 : 0.4;
    }

    // MARK: - Keyboard

    private void OnPreviewKeyDown(object sender, KeyEventArgs e)
    {
        if (_busy) return;
        var ctrl = (Keyboard.Modifiers & ModifierKeys.Control) != 0;
        var shift = (Keyboard.Modifiers & ModifierKeys.Shift) != 0;

        switch (e.Key)
        {
            case Key.Space when !ctrl:
                _playback.TogglePlay();
                break;
            case Key.B when ctrl:
                _model.SplitAt(_model.CurrentTime);
                break;
            case Key.Delete or Key.Back when !ctrl:
                _model.DeleteSelected();
                break;
            case Key.Z when ctrl && !shift:
                _model.Undo();
                break;
            case Key.Y when ctrl:
            case Key.Z when ctrl && shift:
                _model.Redo();
                break;
            case Key.E when ctrl && !shift:
                OnSaveVideo(this, new RoutedEventArgs());
                break;
            case Key.E when ctrl && shift:
                OnSaveGif(this, new RoutedEventArgs());
                break;
            case Key.C when ctrl && shift:
                OnCopyToClipboard(this, new RoutedEventArgs());
                break;
            default:
                return;
        }
        e.Handled = true;
    }

    // MARK: - Export

    private async void OnSaveVideo(object sender, RoutedEventArgs e)
    {
        if (ChooseDestination("MP4 video|*.mp4", ".mp4") is not { } path) return;
        if (await RunExportAsync("Exporting video…", progress => Exporter.SaveVideoAsync(_model, path, progress)))
        {
            _model.MarkExported();
            RevealInExplorer(path);
        }
    }

    private async void OnSaveGif(object sender, RoutedEventArgs e)
    {
        if (ChooseDestination("GIF image|*.gif", ".gif") is not { } path) return;
        if (await RunExportAsync("Creating GIF…", progress => Exporter.SaveGifAsync(_model, path, progress)))
        {
            _model.MarkExported();
            RevealInExplorer(path);
        }
    }

    /// <summary>Puts the edited video on the clipboard as a file, ready to paste into Explorer, Slack, Teams, mail…</summary>
    private async void OnCopyToClipboard(object sender, RoutedEventArgs e)
    {
        var path = _model.SourcePath;
        // Videos with two audio tracks are exported (which mixes them) even without edits: many apps only play the first track.
        if (_model.HasEdits || _model.AudioTracks.Count > 1)
        {
            var folder = Path.Combine(AppPaths.DataFolder, "Clipboard");
            try
            {
                if (Directory.Exists(folder)) Directory.Delete(folder, recursive: true); // only the latest copy is needed
            }
            catch (Exception exception)
            {
                Log.Error("Clearing old clipboard copies", exception);
            }
            Directory.CreateDirectory(folder);
            path = Path.Combine(folder, Path.GetFileNameWithoutExtension(_model.SourcePath) + " (edited).mp4");
            if (!await RunExportAsync("Preparing for clipboard…", progress => Exporter.SaveVideoAsync(_model, path, progress)))
            {
                return;
            }
        }

        Clipboard.SetFileDropList(new StringCollection { path });
        ShowNotice("Copied to clipboard");
    }

    private string? ChooseDestination(string filter, string extension)
    {
        var dialog = new Microsoft.Win32.SaveFileDialog
        {
            Filter = filter,
            DefaultExt = extension,
            InitialDirectory = Path.GetDirectoryName(_model.SourcePath),
            FileName = Path.GetFileNameWithoutExtension(_model.SourcePath) + " (edited)" + extension,
        };
        if (dialog.ShowDialog(this) != true) return null;

        if (string.Equals(Path.GetFullPath(dialog.FileName), _model.SourcePath, StringComparison.OrdinalIgnoreCase))
        {
            MessageBox.Show(this, "Cappa keeps the original recording, so the edit can't be saved over it.",
                "Choose a different file name", MessageBoxButton.OK, MessageBoxImage.Information);
            return null;
        }
        return dialog.FileName;
    }

    private async Task<bool> RunExportAsync(string title, Func<IProgress<double>, Task> export)
    {
        if (_busy) return false;
        _playback.Pause();
        SetBusy(title, 0);
        try
        {
            await export(new Progress<double>(value => BusyProgress.Value = value));
            return true;
        }
        catch (Exception e)
        {
            Log.Error(title, e);
            MessageBox.Show(this, e.Message, "Export failed", MessageBoxButton.OK, MessageBoxImage.Warning);
            return false;
        }
        finally
        {
            ClearBusy();
        }
    }

    private static void RevealInExplorer(string path) => Process.Start("explorer.exe", $"/select,\"{path}\"");

    // MARK: - Busy overlay and notices

    private void SetBusy(string text, double? progress)
    {
        _busy = true;
        BusyText.Text = text;
        BusyProgress.IsIndeterminate = progress is null;
        BusyProgress.Value = progress ?? 0;
        BusyOverlay.Visibility = Visibility.Visible;
    }

    private void ClearBusy()
    {
        _busy = false;
        BusyOverlay.Visibility = Visibility.Collapsed;
    }

    private void ShowNotice(string text)
    {
        NoticeText.Text = text;
        Notice.Visibility = Visibility.Visible;
        _noticeTimer.Stop();
        _noticeTimer.Start();
    }

    // MARK: - Closing

    private void OnClosing(object? sender, System.ComponentModel.CancelEventArgs e)
    {
        if (_busy)
        {
            e.Cancel = true;
            return;
        }
        if (_model.HasUnexportedEdits)
        {
            var answer = MessageBox.Show(this,
                "Your edits haven't been exported and will be lost. The original recording is kept.",
                "Close without exporting?", MessageBoxButton.OKCancel, MessageBoxImage.Question);
            if (answer != MessageBoxResult.OK) e.Cancel = true;
        }
    }

    private void OnClosed()
    {
        OpenEditors.Remove(this);
        _playback.Dispose();
        try
        {
            Directory.Delete(_workFolder, recursive: true);
        }
        catch (Exception e)
        {
            Log.Error("Removing editor work folder", e);
        }
    }
}
