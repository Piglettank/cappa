using System;
using System.Windows;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using Cappa.Native;
using Cappa.Recording;
using WinForms = System.Windows.Forms;

namespace Cappa.UI;

/// <summary>
/// The floating, always-on-top recording controls: status dot, elapsed time, pause/resume and stop.
/// Starts next to what's being recorded, can be dragged away, and is excluded from capture so it never
/// appears in the video.
/// </summary>
internal partial class RecordingOverlay : Window
{
    private static readonly Brush RecordingBrush = new SolidColorBrush(Color.FromRgb(0xFF, 0x3B, 0x30));
    private static readonly Brush PausedBrush = new SolidColorBrush(Color.FromRgb(0xFF, 0x9F, 0x0A));

    // In DIPs.
    private const double RegionGap = 12; // the region's red outline sits 3 px outside it
    private const double ScreenTopMargin = 40;
    private const double EdgeMargin = 8;

    private readonly RecordingController _controller;
    private readonly CaptureTarget _target;

    public RecordingOverlay(RecordingController controller, CaptureTarget target)
    {
        _controller = controller;
        _target = target;
        InitializeComponent();

        SourceInitialized += (_, _) =>
        {
            var hwnd = new WindowInteropHelper(this).Handle;
            Win32.AddExtendedStyle(hwnd, Win32.WS_EX_TOOLWINDOW | Win32.WS_EX_NOACTIVATE);
            Win32.ExcludeFromCapture(hwnd);
        };
        Loaded += (_, _) => Place();
        // Moving onto a monitor with a different scale resizes the window, so place it again at its new size.
        DpiChanged += (_, _) => Dispatcher.InvokeAsync(Place);
    }

    public void Update(RecordingState state, TimeSpan elapsed)
    {
        var paused = state == RecordingState.Paused;
        StatusDot.Fill = paused ? PausedBrush : RecordingBrush;
        PauseGlyph.Text = paused ? "" : ""; // Play : Pause
        PauseButton.ToolTip = paused ? "Resume" : "Pause";
        ElapsedText.Text = elapsed.TotalHours >= 1 ? elapsed.ToString(@"h\:mm\:ss") : elapsed.ToString(@"mm\:ss");
    }

    /// <summary>
    /// Centres the controls below a region in the top half of the monitor, above one in the bottom half, and
    /// near the top of the monitor for a full-screen recording. Works in pixels, like the capture target.
    /// </summary>
    private void Place()
    {
        var hwnd = new WindowInteropHelper(this).Handle;
        if (hwnd == IntPtr.Zero || !Win32.GetWindowRect(hwnd, out var rect)) return;
        var width = rect.Right - rect.Left;
        var height = rect.Bottom - rect.Top;
        var scale = VisualTreeHelper.GetDpi(this).DpiScaleY;
        int Px(double dips) => (int)Math.Round(dips * scale);

        var monitor = _target.MonitorBounds;
        var work = WinForms.Screen.FromRectangle(monitor).WorkingArea;
        int x, y;
        if (_target.Region is null)
        {
            x = work.Left + (work.Width - width) / 2;
            y = work.Top + Px(ScreenTopMargin);
        }
        else
        {
            var area = _target.ScreenArea;
            var isInTopHalf = area.Top + area.Height / 2 < monitor.Top + monitor.Height / 2;
            x = area.Left + (area.Width - width) / 2;
            y = isInTopHalf ? area.Bottom + Px(RegionGap) : area.Top - Px(RegionGap) - height;
        }
        // A region filling most of the monitor leaves no room outside it; the controls then overlap the
        // region, which is fine since they never show up in the video.
        var margin = Px(EdgeMargin);
        x = Math.Clamp(x, work.Left + margin, Math.Max(work.Left + margin, work.Right - width - margin));
        y = Math.Clamp(y, work.Top + margin, Math.Max(work.Top + margin, work.Bottom - height - margin));
        Win32.SetWindowPos(hwnd, IntPtr.Zero, x, y, 0, 0, Win32.SWP_NOSIZE | Win32.SWP_NOZORDER | Win32.SWP_NOACTIVATE);
    }

    private void OnDrag(object sender, MouseButtonEventArgs e)
    {
        if (e.ButtonState == MouseButtonState.Pressed) DragMove();
    }

    private void OnPauseClick(object sender, RoutedEventArgs e) => _controller.TogglePause();

    private async void OnStopClick(object sender, RoutedEventArgs e) => await _controller.StopAsync();
}
