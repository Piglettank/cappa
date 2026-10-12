using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Windows;
using System.Windows.Input;
using System.Windows.Media;

namespace Cappa.Editor;

/// <summary>
/// The timeline maps the whole source at a fixed scale. Kept clips are bright, cut-away parts stay visible
/// but dimmed, so an edge can be dragged back to restore them. Audio tracks get lanes below the video; they
/// follow the video's clips. Lane headers (mute/volume) live in the editor window, aligned to these rows.
/// </summary>
internal sealed class TimelineControl : FrameworkElement
{
    public const double RulerHeight = 22;
    public const double VideoHeight = 72;
    public const double AudioHeight = 38;
    public const double LaneSpacing = 6;
    private const double HandleWidth = 10;

    private static readonly Brush HandleBrush = Frozen(new SolidColorBrush(Color.FromArgb(0xE6, 0xFF, 0xFF, 0xFF)));
    private static readonly Brush SelectedBrush = Frozen(new SolidColorBrush(Color.FromRgb(0xFF, 0xCC, 0x00)));
    private static readonly Brush LaneBackground = Frozen(new SolidColorBrush(Color.FromArgb(0x14, 0x80, 0x80, 0x80)));
    private static readonly Brush PlayheadBrush = Frozen(new SolidColorBrush(Color.FromRgb(0xFF, 0x3B, 0x30)));
    private static readonly Brush RulerTextBrush = Frozen(new SolidColorBrush(Color.FromRgb(0x80, 0x80, 0x80)));
    private static readonly Color ComputerAudioColor = Color.FromRgb(0x30, 0xB0, 0xC7);
    private static readonly Color MicrophoneColor = Color.FromRgb(0xFF, 0x95, 0x00);

    private EditorModel? _model;
    private EditorPlayback? _playback;
    private List<ImageBrush> _tiles = new();
    private (Guid Id, TrimEdge Edge, double GrabOffset)? _trim;
    private bool _scrubbing;
    private bool _scrubSelectsClip;

    public void Attach(EditorModel model, EditorPlayback playback)
    {
        _model = model;
        _playback = playback;
        _model.Changed += Refresh;
        _playback.Updated += InvalidateVisual;
        Refresh();
    }

    public static double TotalHeight(int audioTrackCount) =>
        RulerHeight + LaneSpacing + VideoHeight + audioTrackCount * (LaneSpacing + AudioHeight);

    private void Refresh()
    {
        if (_model is null) return;
        if (_tiles.Count != _model.Thumbnails.Count)
        {
            _tiles = _model.Thumbnails.Select(t => Frozen(new ImageBrush(t) { Stretch = Stretch.UniformToFill })).ToList();
        }
        Height = TotalHeight(_model.AudioTracks.Count);
        InvalidateVisual();
    }

    // MARK: - Geometry

    private double X(double time) => _model is { Duration: > 0 } m ? time / m.Duration * ActualWidth : 0;

    private double TimeAt(double x) =>
        _model is { Duration: > 0 } m && ActualWidth > 0 ? Math.Clamp(x / ActualWidth, 0, 1) * m.Duration : 0;

    private static double VideoTop => RulerHeight + LaneSpacing;

    private static double AudioTop(int index) => VideoTop + VideoHeight + LaneSpacing + index * (AudioHeight + LaneSpacing);

    // MARK: - Drawing

    protected override void OnRender(DrawingContext context)
    {
        var model = _model;
        if (model is null || ActualWidth <= 0) return;

        context.DrawRectangle(Brushes.Transparent, null, new Rect(0, 0, ActualWidth, ActualHeight)); // hit-testable everywhere
        DrawRuler(context, model);
        DrawVideoLane(context, model);
        for (var i = 0; i < model.AudioTracks.Count; i++) DrawAudioLane(context, model, model.AudioTracks[i], AudioTop(i));
        DrawPlayhead(context, model);
    }

    private void DrawRuler(DrawingContext context, EditorModel model)
    {
        if (model.Duration <= 0) return;
        double[] steps = { 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1800 };
        var step = steps.FirstOrDefault(s => s / model.Duration * ActualWidth >= 64, 3600);
        var tick = Frozen(new Pen(RulerTextBrush, 1));
        var pixelsPerDip = VisualTreeHelper.GetDpi(this).PixelsPerDip;

        for (var t = 0.0; t <= model.Duration; t += step)
        {
            var x = Math.Round(X(t)) + 0.5;
            context.DrawLine(tick, new Point(x, RulerHeight - 6), new Point(x, RulerHeight));
            var label = new FormattedText(TimeFormat.Short(t), CultureInfo.CurrentCulture, FlowDirection.LeftToRight,
                new Typeface("Segoe UI"), 10, RulerTextBrush, pixelsPerDip);
            context.DrawText(label, new Point(x + 3, RulerHeight - 8 - label.Height));
        }
    }

    private void DrawVideoLane(DrawingContext context, EditorModel model)
    {
        var lane = new Rect(0, VideoTop, ActualWidth, VideoHeight);

        // The whole source, dimmed: what's been cut away.
        context.PushClip(new RectangleGeometry(lane, 6, 6));
        context.DrawRectangle(LaneBackground, null, lane);
        context.PushOpacity(0.25);
        DrawFilmstrip(context, lane);
        context.Pop();
        context.Pop();

        foreach (var segment in model.Segments)
        {
            var clip = ClipRect(segment);
            var selected = segment.Id == model.SelectedId;

            context.PushClip(new RectangleGeometry(clip, 6, 6));
            DrawFilmstrip(context, lane);
            var handleBrush = selected ? SelectedBrush : HandleBrush;
            context.DrawRectangle(handleBrush, null, new Rect(clip.Left, clip.Top, Math.Min(HandleWidth, clip.Width / 2), clip.Height));
            context.DrawRectangle(handleBrush, null, new Rect(clip.Right - Math.Min(HandleWidth, clip.Width / 2), clip.Top, Math.Min(HandleWidth, clip.Width / 2), clip.Height));
            context.Pop();

            var border = selected ? new Pen(SelectedBrush, 3) : new Pen(HandleBrush, 1.5);
            context.DrawRoundedRectangle(null, border, Rect.Inflate(clip, -border.Thickness / 2, -border.Thickness / 2), 6, 6);
        }
    }

    private void DrawFilmstrip(DrawingContext context, Rect lane)
    {
        if (_tiles.Count == 0) return;
        var width = lane.Width / _tiles.Count;
        for (var i = 0; i < _tiles.Count; i++)
        {
            context.DrawRectangle(_tiles[i], null, new Rect(lane.Left + i * width, lane.Top, width + 0.5, lane.Height));
        }
    }

    private void DrawAudioLane(DrawingContext context, EditorModel model, AudioTrack track, double top)
    {
        var lane = new Rect(0, top, ActualWidth, AudioHeight);
        var color = track.IsMuted ? Color.FromRgb(0x8E, 0x8E, 0x93) : track.IsMicrophone ? MicrophoneColor : ComputerAudioColor;
        var bright = Frozen(new SolidColorBrush(color));
        var dim = Frozen(new SolidColorBrush(Color.FromArgb(0x40, color.R, color.G, color.B)));
        var fill = Frozen(new SolidColorBrush(Color.FromArgb(0x2E, color.R, color.G, color.B)));

        context.DrawRoundedRectangle(LaneBackground, null, lane, 5, 5);
        foreach (var segment in model.Segments)
        {
            var clip = new Rect(X(segment.Start), top, Math.Max(X(segment.End) - X(segment.Start), 2), AudioHeight);
            var selected = segment.Id == model.SelectedId;
            context.DrawRoundedRectangle(fill, selected ? new Pen(SelectedBrush, 2) : new Pen(dim, 1), clip, 5, 5);
        }

        var levels = track.Waveform;
        if (levels.Length == 0 || model.Duration <= 0) return;
        var gain = track.IsMuted ? 1 : track.Volume;
        var middle = top + AudioHeight / 2;
        for (var x = 1.0; x < ActualWidth - 1; x += 2.5)
        {
            var fraction = x / ActualWidth;
            var level = levels[Math.Min((int)(fraction * levels.Length), levels.Length - 1)];
            var height = Math.Max(level * gain * (AudioHeight - 8), 1);
            var kept = model.SegmentAt(fraction * model.Duration) is not null;
            context.DrawRectangle(kept ? bright : dim, null, new Rect(x, middle - height / 2, 1.5, height));
        }
    }

    private void DrawPlayhead(DrawingContext context, EditorModel model)
    {
        var x = X(model.CurrentTime);
        context.DrawRectangle(PlayheadBrush, null, new Rect(x - 1, 2, 2, ActualHeight - 2));
        var triangle = new StreamGeometry();
        using (var g = triangle.Open())
        {
            g.BeginFigure(new Point(x - 6, 0), isFilled: true, isClosed: true);
            g.LineTo(new Point(x + 6, 0), true, false);
            g.LineTo(new Point(x, 9), true, false);
        }
        triangle.Freeze();
        context.DrawGeometry(PlayheadBrush, null, triangle);
    }

    private Rect ClipRect(Segment segment) =>
        new(X(segment.Start), VideoTop, Math.Max(X(segment.End) - X(segment.Start), 2), VideoHeight);

    // MARK: - Mouse

    protected override void OnMouseLeftButtonDown(MouseButtonEventArgs e)
    {
        if (_model is null || _playback is null) return;
        Focus();
        var point = e.GetPosition(this);
        _playback.Pause();

        if (HitHandle(point) is { } handle)
        {
            var edgeX = X(handle.Edge == TrimEdge.Start ? handle.Segment.Start : handle.Segment.End);
            _trim = (handle.Segment.Id, handle.Edge, point.X - edgeX);
            _model.BeginTrim(handle.Segment.Id);
        }
        else
        {
            // Clicking a clip (in any lane) selects it; the ruler and cut-away parts deselect.
            _scrubbing = true;
            _scrubSelectsClip = point.Y >= VideoTop;
            ScrubTo(point.X);
        }
        CaptureMouse();
        e.Handled = true;
    }

    protected override void OnMouseMove(MouseEventArgs e)
    {
        if (_model is null || _playback is null) return;
        var point = e.GetPosition(this);

        if (_trim is { } trim && e.LeftButton == MouseButtonState.Pressed)
        {
            var time = _model.Trim(trim.Id, trim.Edge, TimeAt(point.X - trim.GrabOffset));
            _playback.Seek(time);
        }
        else if (_scrubbing && e.LeftButton == MouseButtonState.Pressed)
        {
            ScrubTo(point.X);
        }
        else
        {
            Cursor = HitHandle(point) is not null ? Cursors.SizeWE : Cursors.Arrow;
        }
    }

    protected override void OnMouseLeftButtonUp(MouseButtonEventArgs e)
    {
        if (_trim is not null) _model?.EndTrim();
        _trim = null;
        _scrubbing = false;
        ReleaseMouseCapture();
    }

    private void ScrubTo(double x)
    {
        if (_model is null || _playback is null) return;
        var time = TimeAt(x);
        var selection = _scrubSelectsClip ? _model.SegmentAt(time)?.Id : null;
        if (_model.SelectedId != selection)
        {
            _model.SelectedId = selection;
            _model.RaiseChanged();
        }
        _playback.Seek(time);
    }

    private (Segment Segment, TrimEdge Edge)? HitHandle(Point point)
    {
        if (_model is null || point.Y < VideoTop || point.Y > VideoTop + VideoHeight) return null;
        foreach (var segment in _model.Segments)
        {
            var clip = ClipRect(segment);
            var handle = Math.Min(HandleWidth, clip.Width / 2);
            if (point.X >= clip.Left - 2 && point.X <= clip.Left + handle) return (segment, TrimEdge.Start);
            if (point.X >= clip.Right - handle && point.X <= clip.Right + 2) return (segment, TrimEdge.End);
        }
        return null;
    }

    private static T Frozen<T>(T freezable) where T : Freezable
    {
        if (freezable.CanFreeze) freezable.Freeze();
        return freezable;
    }
}

internal static class TimeFormat
{
    /// <summary>m:ss.t, or h:mm:ss.t for long videos.</summary>
    public static string Precise(double seconds)
    {
        var tenths = (long)Math.Floor(Math.Max(seconds, 0) * 10);
        var time = TimeSpan.FromSeconds(tenths / 10);
        var fraction = tenths % 10;
        return time.TotalHours >= 1
            ? $"{(int)time.TotalHours}:{time.Minutes:00}:{time.Seconds:00}.{fraction}"
            : $"{time.Minutes}:{time.Seconds:00}.{fraction}";
    }

    /// <summary>m:ss, for ruler labels.</summary>
    public static string Short(double seconds)
    {
        var total = (int)Math.Round(seconds);
        return $"{total / 60}:{total % 60:00}";
    }
}
