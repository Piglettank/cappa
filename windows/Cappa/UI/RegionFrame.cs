using System.Windows;
using System.Windows.Interop;
using System.Windows.Media;
using System.Windows.Shapes;
using Cappa.Native;

namespace Cappa.UI;

/// <summary>
/// A dashed red outline just outside the recorded region while recording. It ignores the mouse and is
/// excluded from capture, so it never shows up in the video.
/// </summary>
internal sealed class RegionFrame : Window
{
    private const int OutlineGap = 3;

    public RegionFrame(System.Drawing.Rectangle area)
    {
        var bounds = System.Drawing.Rectangle.Inflate(area, OutlineGap, OutlineGap);
        WindowStyle = WindowStyle.None;
        AllowsTransparency = true;
        Background = Brushes.Transparent;
        ResizeMode = ResizeMode.NoResize;
        ShowInTaskbar = false;
        ShowActivated = false;
        Topmost = true;
        IsHitTestVisible = false;
        Content = new Rectangle
        {
            Stroke = new SolidColorBrush(Color.FromArgb(0xD9, 0xFF, 0x3B, 0x30)),
            StrokeThickness = 2,
            StrokeDashArray = new DoubleCollection { 3, 2 },
        };

        SourceInitialized += (_, _) =>
        {
            var hwnd = new WindowInteropHelper(this).Handle;
            Win32.AddExtendedStyle(hwnd, Win32.WS_EX_TRANSPARENT | Win32.WS_EX_TOOLWINDOW | Win32.WS_EX_NOACTIVATE);
            Win32.ExcludeFromCapture(hwnd);
            Win32.SetWindowPos(hwnd, Win32.HWND_TOPMOST, bounds.X, bounds.Y, bounds.Width, bounds.Height,
                Win32.SWP_NOACTIVATE | Win32.SWP_SHOWWINDOW);
        };
    }
}
