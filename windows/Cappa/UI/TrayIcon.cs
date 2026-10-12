using System;
using System.Diagnostics;
using System.Drawing;
using System.Reflection;
using System.Windows;
using Cappa.Recording;
using Cappa.Services;
using WinForms = System.Windows.Forms;

namespace Cappa.UI;

/// <summary>
/// The notification-area icon. Left or right click opens the Cappa menu; while recording the icon gets a
/// red dot and the menu offers Stop and Pause.
/// </summary>
internal sealed class TrayIcon : IDisposable
{
    private readonly RecordingController _controller;
    private readonly WinForms.NotifyIcon _icon;
    private readonly WinForms.ContextMenuStrip _menu = new();
    private readonly Icon _idleIcon;
    private readonly Icon _recordingIcon;

    public TrayIcon(RecordingController controller)
    {
        _controller = controller;
        _idleIcon = LoadAppIcon();
        _recordingIcon = WithRecordingDot(_idleIcon);

        _menu.Opening += (_, _) => BuildMenu();
        _icon = new WinForms.NotifyIcon
        {
            Icon = _idleIcon,
            Text = "Cappa",
            ContextMenuStrip = _menu,
            Visible = true,
        };
        _icon.MouseClick += (_, e) =>
        {
            if (e.Button == WinForms.MouseButtons.Left) ShowMenu();
        };
        _controller.StateChanged += UpdateIcon;
    }

    public void ShowBalloon(string title, string text) =>
        _icon.ShowBalloonTip(4000, title, text, WinForms.ToolTipIcon.None);

    private void BuildMenu()
    {
        _menu.Items.Clear();
        var settings = _controller.Settings;

        switch (_controller.State)
        {
            case RecordingState.Idle:
                var region = Item("Record Region…", async () => await _controller.StartRegionRecordingAsync());
                region.ShortcutKeyDisplayString = "Ctrl+Alt+Shift+R";
                region.Font = new Font(region.Font, System.Drawing.FontStyle.Bold);
                _menu.Items.Add(region);
                _menu.Items.Add(Item("Record Screen", async () => await _controller.StartScreenRecordingAsync()));
                break;
            case RecordingState.Selecting:
                _menu.Items.Add(Item("Cancel Selection", async () => await _controller.ToggleRecordingAsync()));
                break;
            case RecordingState.Recording or RecordingState.Paused:
                var stop = Item("Stop Recording", async () => await _controller.StopAsync());
                stop.ShortcutKeyDisplayString = "Ctrl+Alt+Shift+R";
                _menu.Items.Add(stop);
                _menu.Items.Add(Item(_controller.State == RecordingState.Paused ? "Resume" : "Pause", _controller.TogglePause));
                break;
            default:
                _menu.Items.Add(new WinForms.ToolStripMenuItem(_controller.State == RecordingState.Starting ? "Starting…" : "Saving…") { Enabled = false });
                break;
        }

        _menu.Items.Add(new WinForms.ToolStripSeparator());
        var idle = _controller.State == RecordingState.Idle;
        _menu.Items.Add(Toggle("Record System Audio", settings.RecordSystemAudio, idle, v => settings.RecordSystemAudio = v));
        _menu.Items.Add(Toggle("Record Microphone", settings.RecordMicrophone, idle, v => settings.RecordMicrophone = v));
        _menu.Items.Add(Toggle("Show Overlay While Recording", settings.ShowOverlay, true, v => settings.ShowOverlay = v));

        _menu.Items.Add(new WinForms.ToolStripSeparator());
        _menu.Items.Add(Item("Settings…", () => SettingsWindow.ShowSingleton(settings)));
        _menu.Items.Add(Item("Open Video in Editor…", _controller.OpenVideoInEditor));
        _menu.Items.Add(Item("Open Recordings Folder", _controller.OpenRecordingsFolder));
        _menu.Items.Add(Item("Open Log", () => Process.Start(new ProcessStartInfo(Log.FilePath) { UseShellExecute = true })));
        _menu.Items.Add(new WinForms.ToolStripSeparator());
        _menu.Items.Add(Item("Quit Cappa", () => Application.Current.Shutdown()));
    }

    private void ShowMenu()
    {
        // NotifyIcon only opens its menu on right click; this is the standard way to open it on left click too.
        typeof(WinForms.NotifyIcon)
            .GetMethod("ShowContextMenu", BindingFlags.Instance | BindingFlags.NonPublic)
            ?.Invoke(_icon, null);
    }

    private void UpdateIcon()
    {
        var recording = _controller.IsActive;
        _icon.Icon = recording ? _recordingIcon : _idleIcon;
        _icon.Text = recording ? "Cappa — recording" : "Cappa";
    }

    private static WinForms.ToolStripMenuItem Item(string text, Action action)
    {
        var item = new WinForms.ToolStripMenuItem(text);
        item.Click += (_, _) =>
        {
            try { action(); }
            catch (Exception e) { Log.Error($"Menu item '{text}'", e); }
        };
        return item;
    }

    private static WinForms.ToolStripMenuItem Toggle(string text, bool isChecked, bool enabled, Action<bool> set)
    {
        var item = new WinForms.ToolStripMenuItem(text) { Checked = isChecked, Enabled = enabled };
        item.Click += (_, _) => set(!isChecked);
        return item;
    }

    private static Icon LoadAppIcon()
    {
        var resource = Application.GetResourceStream(new Uri("pack://application:,,,/Assets/cappa.ico"));
        using var stream = resource!.Stream;
        return new Icon(stream, WinForms.SystemInformation.SmallIconSize);
    }

    private static Icon WithRecordingDot(Icon icon)
    {
        using var bitmap = icon.ToBitmap();
        using (var graphics = Graphics.FromImage(bitmap))
        {
            graphics.SmoothingMode = System.Drawing.Drawing2D.SmoothingMode.AntiAlias;
            var size = bitmap.Width * 0.55f;
            var x = bitmap.Width - size;
            var y = bitmap.Height - size;
            graphics.FillEllipse(Brushes.White, x - 1, y - 1, size + 2, size + 2);
            using var red = new SolidBrush(System.Drawing.Color.FromArgb(0xFF, 0x3B, 0x30));
            graphics.FillEllipse(red, x, y, size, size);
        }
        return Icon.FromHandle(bitmap.GetHicon());
    }

    public void Dispose()
    {
        _controller.StateChanged -= UpdateIcon;
        _icon.Visible = false;
        _icon.Dispose();
        _menu.Dispose();
    }
}
