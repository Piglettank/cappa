using System;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Media.Animation;
using System.Windows.Shapes;
using System.Windows.Threading;
using Cappa.Native;

namespace Cappa.UI;

/// <summary>
/// Draws an expanding ring that fades from black to white wherever the mouse is clicked on one monitor.
/// Unlike Cappa's other windows it is *not* excluded from capture, since it's meant to be in the video;
/// it ignores the mouse, so it also shows on screen without getting in the way.
/// </summary>
internal sealed class ClickHighlighter : Window
{
    // In DIPs.
    private const double StartRadius = 6;
    private const double EndRadius = 30;
    private const double LineWidth = 3;
    private static readonly Duration AnimationDuration = new(TimeSpan.FromMilliseconds(500));

    private static readonly int[] MouseButtons = { Win32.VK_LBUTTON, Win32.VK_RBUTTON, Win32.VK_MBUTTON };

    private readonly System.Drawing.Rectangle _monitorBounds;
    private readonly Canvas _canvas = new();
    // Polling rather than a low-level mouse hook, so a busy UI thread can never make the mouse lag.
    private readonly DispatcherTimer _poll = new(DispatcherPriority.Input) { Interval = TimeSpan.FromMilliseconds(15) };
    private readonly bool[] _wasDown = new bool[MouseButtons.Length];

    public ClickHighlighter(System.Drawing.Rectangle monitorBounds)
    {
        _monitorBounds = monitorBounds;
        WindowStyle = WindowStyle.None;
        AllowsTransparency = true;
        Background = Brushes.Transparent;
        ResizeMode = ResizeMode.NoResize;
        ShowInTaskbar = false;
        ShowActivated = false;
        Topmost = true;
        IsHitTestVisible = false;
        Content = _canvas;

        SourceInitialized += (_, _) =>
        {
            var hwnd = new WindowInteropHelper(this).Handle;
            Win32.AddExtendedStyle(hwnd, Win32.WS_EX_TRANSPARENT | Win32.WS_EX_TOOLWINDOW | Win32.WS_EX_NOACTIVATE);
            Win32.SetWindowPos(hwnd, Win32.HWND_TOPMOST, _monitorBounds.X, _monitorBounds.Y,
                _monitorBounds.Width, _monitorBounds.Height, Win32.SWP_NOACTIVATE | Win32.SWP_SHOWWINDOW);
        };
        _poll.Tick += (_, _) => Poll();
        Loaded += (_, _) => _poll.Start();
        Closed += (_, _) => _poll.Stop();
    }

    private void Poll()
    {
        var clicked = false;
        for (var i = 0; i < MouseButtons.Length; i++)
        {
            var isDown = (Win32.GetAsyncKeyState(MouseButtons[i]) & 0x8000) != 0;
            clicked |= isDown && !_wasDown[i];
            _wasDown[i] = isDown;
        }
        if (clicked && Win32.GetCursorPos(out var cursor) && _monitorBounds.Contains(cursor.X, cursor.Y))
        {
            var dpi = VisualTreeHelper.GetDpi(this);
            Highlight(new Point((cursor.X - _monitorBounds.X) / dpi.DpiScaleX, (cursor.Y - _monitorBounds.Y) / dpi.DpiScaleY));
        }
    }

    private void Highlight(Point center)
    {
        var circle = new EllipseGeometry(center, StartRadius, StartRadius);
        var stroke = new SolidColorBrush(Colors.Black);
        var ring = new Path { Data = circle, Stroke = stroke, StrokeThickness = LineWidth };
        _canvas.Children.Add(ring);

        var ease = new QuadraticEase { EasingMode = EasingMode.EaseOut };
        var grow = new DoubleAnimation(StartRadius, EndRadius, AnimationDuration) { EasingFunction = ease };
        var fade = new DoubleAnimation(1, 0, AnimationDuration) { EasingFunction = ease };
        fade.Completed += (_, _) => _canvas.Children.Remove(ring);

        circle.BeginAnimation(EllipseGeometry.RadiusXProperty, grow);
        circle.BeginAnimation(EllipseGeometry.RadiusYProperty, grow);
        stroke.BeginAnimation(SolidColorBrush.ColorProperty, new ColorAnimation(Colors.Black, Colors.White, AnimationDuration) { EasingFunction = ease });
        ring.BeginAnimation(OpacityProperty, fade);
    }
}
