using System;
using System.Windows;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using Cappa.Native;
using Cappa.Recording;

namespace Cappa.UI;

/// <summary>
/// The floating, always-on-top recording controls: status dot, elapsed time, pause/resume and stop.
/// Draggable, remembers its position, and excluded from capture so it never appears in the video.
/// </summary>
internal partial class RecordingOverlay : Window
{
    private static readonly Brush RecordingBrush = new SolidColorBrush(Color.FromRgb(0xFF, 0x3B, 0x30));
    private static readonly Brush PausedBrush = new SolidColorBrush(Color.FromRgb(0xFF, 0x9F, 0x0A));

    private readonly RecordingController _controller;

    public RecordingOverlay(RecordingController controller)
    {
        _controller = controller;
        InitializeComponent();

        SourceInitialized += (_, _) =>
        {
            var hwnd = new WindowInteropHelper(this).Handle;
            Win32.AddExtendedStyle(hwnd, Win32.WS_EX_TOOLWINDOW | Win32.WS_EX_NOACTIVATE);
            Win32.ExcludeFromCapture(hwnd);
        };
        Loaded += (_, _) => PlaceInitially();
        LocationChanged += (_, _) =>
        {
            if (IsLoaded)
            {
                _controller.Settings.OverlayLeft = Left;
                _controller.Settings.OverlayTop = Top;
            }
        };
    }

    public void Update(RecordingState state, TimeSpan elapsed)
    {
        var paused = state == RecordingState.Paused;
        StatusDot.Fill = paused ? PausedBrush : RecordingBrush;
        PauseGlyph.Text = paused ? "" : ""; // Play : Pause
        PauseButton.ToolTip = paused ? "Resume" : "Pause";
        ElapsedText.Text = elapsed.TotalHours >= 1 ? elapsed.ToString(@"h\:mm\:ss") : elapsed.ToString(@"mm\:ss");
    }

    private void PlaceInitially()
    {
        var settings = _controller.Settings;
        var area = SystemParameters.WorkArea;
        if (settings.OverlayLeft is { } left && settings.OverlayTop is { } top
            && left >= SystemParameters.VirtualScreenLeft && top >= SystemParameters.VirtualScreenTop
            && left < SystemParameters.VirtualScreenLeft + SystemParameters.VirtualScreenWidth - 40
            && top < SystemParameters.VirtualScreenTop + SystemParameters.VirtualScreenHeight - 20)
        {
            Left = left;
            Top = top;
        }
        else
        {
            Left = area.Left + (area.Width - ActualWidth) / 2;
            Top = area.Top + 12;
        }
    }

    private void OnDrag(object sender, MouseButtonEventArgs e)
    {
        if (e.ButtonState == MouseButtonState.Pressed) DragMove();
    }

    private void OnPauseClick(object sender, RoutedEventArgs e) => _controller.TogglePause();

    private async void OnStopClick(object sender, RoutedEventArgs e) => await _controller.StopAsync();
}
