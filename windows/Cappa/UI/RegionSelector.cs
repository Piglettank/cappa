using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Input;
using System.Windows.Interop;
using System.Windows.Media;
using Cappa.Native;
using Cappa.Recording;
using WinForms = System.Windows.Forms;

namespace Cappa.UI;

/// <summary>
/// Dims every monitor and lets the user drag out the area to record. A click without dragging picks the
/// whole monitor; Esc cancels. The selection stays on the monitor where the drag started.
/// </summary>
internal sealed class RegionSelector
{
    private readonly List<SelectionWindow> _windows = new();
    private TaskCompletionSource<CaptureTarget?>? _completion;

    public bool IsActive => _completion is not null;

    public Task<CaptureTarget?> SelectAsync()
    {
        if (_completion is not null) return Task.FromResult<CaptureTarget?>(null);
        _completion = new TaskCompletionSource<CaptureTarget?>();

        var cursor = WinForms.Cursor.Position;
        foreach (var screen in WinForms.Screen.AllScreens)
        {
            var window = new SelectionWindow(screen.Bounds);
            window.Began += () =>
            {
                foreach (var other in _windows.Where(w => w != window)) other.ClearSelection();
            };
            window.Completed += region => Finish(new CaptureTarget(screen.DeviceName, screen.Bounds, region));
            window.Cancelled += Cancel;
            _windows.Add(window);
            window.Show();
            if (screen.Bounds.Contains(cursor)) window.Activate();
        }
        return _completion.Task;
    }

    public void Cancel() => Finish(null);

    private void Finish(CaptureTarget? target)
    {
        var completion = _completion;
        if (completion is null) return;
        _completion = null;
        foreach (var window in _windows) window.Close();
        _windows.Clear();
        completion.TrySetResult(target);
    }
}

/// <summary>One full-screen, see-through window per monitor.</summary>
internal sealed class SelectionWindow : Window
{
    private const double MinimumSize = 8; // in DIPs; smaller drags count as a click
    private readonly System.Drawing.Rectangle _monitorBounds;
    private readonly SelectionSurface _surface = new();
    private Point? _dragStart;

    public event Action? Began;
    /// <summary>The selected area in pixels relative to the monitor, or null for the whole monitor.</summary>
    public event Action<System.Drawing.Rectangle?>? Completed;
    public event Action? Cancelled;

    public SelectionWindow(System.Drawing.Rectangle monitorBounds)
    {
        _monitorBounds = monitorBounds;
        WindowStyle = WindowStyle.None;
        AllowsTransparency = true;
        Background = Brushes.Transparent;
        ResizeMode = ResizeMode.NoResize;
        ShowInTaskbar = false;
        Topmost = true;
        Cursor = Cursors.Cross;
        Content = _surface;
        // Rough placement; SourceInitialized moves it to the exact monitor pixels.
        Left = monitorBounds.Left;
        Top = monitorBounds.Top;
        Width = monitorBounds.Width;
        Height = monitorBounds.Height;

        SourceInitialized += (_, _) =>
        {
            var hwnd = new WindowInteropHelper(this).Handle;
            Win32.AddExtendedStyle(hwnd, Win32.WS_EX_TOOLWINDOW);
            Win32.SetWindowPos(hwnd, Win32.HWND_TOPMOST, _monitorBounds.X, _monitorBounds.Y,
                _monitorBounds.Width, _monitorBounds.Height, Win32.SWP_SHOWWINDOW);
        };
        MouseLeftButtonDown += OnMouseDown;
        MouseMove += OnMouseMove;
        MouseLeftButtonUp += OnMouseUp;
        KeyDown += (_, e) =>
        {
            if (e.Key == Key.Escape) Cancelled?.Invoke();
        };
    }

    public void ClearSelection()
    {
        _dragStart = null;
        _surface.Selection = null;
        _surface.IsDragging = false;
    }

    private void OnMouseDown(object sender, MouseButtonEventArgs e)
    {
        Activate();
        Began?.Invoke();
        _dragStart = Clamp(e.GetPosition(_surface));
        _surface.Selection = null;
        _surface.IsDragging = true;
        CaptureMouse();
    }

    private void OnMouseMove(object sender, MouseEventArgs e)
    {
        if (_dragStart is not { } start) return;
        var point = Clamp(e.GetPosition(_surface));
        _surface.Selection = new Rect(start, point);
    }

    private void OnMouseUp(object sender, MouseButtonEventArgs e)
    {
        if (_dragStart is null) return;
        _dragStart = null;
        ReleaseMouseCapture();

        if (_surface.Selection is { Width: >= MinimumSize, Height: >= MinimumSize } selection)
        {
            Completed?.Invoke(ToPixels(selection));
        }
        else
        {
            Completed?.Invoke(null);
        }
    }

    /// <summary>DIPs → monitor pixels, with even dimensions (H.264 needs them).</summary>
    private System.Drawing.Rectangle ToPixels(Rect rect)
    {
        var dpi = VisualTreeHelper.GetDpi(this);
        var x = (int)Math.Round(rect.X * dpi.DpiScaleX);
        var y = (int)Math.Round(rect.Y * dpi.DpiScaleY);
        var width = Math.Min((int)Math.Round(rect.Width * dpi.DpiScaleX), _monitorBounds.Width - x) & ~1;
        var height = Math.Min((int)Math.Round(rect.Height * dpi.DpiScaleY), _monitorBounds.Height - y) & ~1;
        return new System.Drawing.Rectangle(x, y, Math.Max(width, 2), Math.Max(height, 2));
    }

    private Point Clamp(Point point) => new(
        Math.Clamp(point.X, 0, _surface.ActualWidth),
        Math.Clamp(point.Y, 0, _surface.ActualHeight));
}

/// <summary>Draws the dimmed screen, the bright selection, its size and the hint.</summary>
internal sealed class SelectionSurface : FrameworkElement
{
    private static readonly Brush Dim = Frozen(new SolidColorBrush(Color.FromArgb(0x66, 0, 0, 0)));
    // Nearly transparent rather than transparent: fully transparent pixels of a layered window don't receive the mouse.
    private static readonly Brush HitTestable = Frozen(new SolidColorBrush(Color.FromArgb(1, 0, 0, 0)));
    private static readonly Brush LabelBackground = Frozen(new SolidColorBrush(Color.FromArgb(0xB0, 0, 0, 0)));
    private static readonly Pen Border = Frozen(new Pen(Brushes.White, 1));

    private Rect? _selection;
    private bool _isDragging;

    public Rect? Selection
    {
        get => _selection;
        set { _selection = value; InvalidateVisual(); }
    }

    public bool IsDragging
    {
        get => _isDragging;
        set { _isDragging = value; InvalidateVisual(); }
    }

    protected override void OnRender(DrawingContext context)
    {
        var bounds = new Rect(0, 0, ActualWidth, ActualHeight);
        if (_selection is { } selection)
        {
            context.DrawGeometry(Dim, null, new CombinedGeometry(GeometryCombineMode.Exclude,
                new RectangleGeometry(bounds), new RectangleGeometry(selection)));
            context.DrawRectangle(HitTestable, Border, selection);
            DrawSizeLabel(context, selection, bounds);
        }
        else
        {
            context.DrawRectangle(Dim, null, bounds);
            if (!_isDragging) DrawHint(context, bounds);
        }
    }

    private void DrawSizeLabel(DrawingContext context, Rect selection, Rect bounds)
    {
        var dpi = VisualTreeHelper.GetDpi(this);
        var text = Text($"{Math.Round(selection.Width * dpi.DpiScaleX)} × {Math.Round(selection.Height * dpi.DpiScaleY)}", 12);
        var size = new Size(text.Width + 16, text.Height + 8);
        var origin = new Point(selection.Left, selection.Bottom + 6);
        if (origin.Y + size.Height > bounds.Bottom) origin.Y = selection.Bottom - size.Height - 6;
        origin.X = Math.Min(origin.X, bounds.Right - size.Width);
        context.DrawRoundedRectangle(LabelBackground, null, new Rect(origin, size), 5, 5);
        context.DrawText(text, new Point(origin.X + 8, origin.Y + 4));
    }

    private void DrawHint(DrawingContext context, Rect bounds)
    {
        var text = Text("Drag to select the area to record\nClick to record the whole screen · Esc to cancel", 16);
        text.TextAlignment = TextAlignment.Center;
        var size = new Size(text.Width + 40, text.Height + 28);
        var box = new Rect(bounds.Width / 2 - size.Width / 2, bounds.Height / 2 - size.Height / 2, size.Width, size.Height);
        context.DrawRoundedRectangle(LabelBackground, null, box, 10, 10);
        context.DrawText(text, new Point(box.X + size.Width / 2, box.Y + 14));
    }

    private FormattedText Text(string value, double size) => new(
        value, CultureInfo.CurrentUICulture, FlowDirection.LeftToRight,
        new Typeface("Segoe UI"), size, Brushes.White, VisualTreeHelper.GetDpi(this).PixelsPerDip);

    private static T Frozen<T>(T freezable) where T : Freezable
    {
        freezable.Freeze();
        return freezable;
    }
}
