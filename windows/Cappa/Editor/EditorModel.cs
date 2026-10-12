using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Linq;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using Cappa.Services;

namespace Cappa.Editor;

/// <summary>One audio track of the source. It always follows the video's clips; only its level is separate.</summary>
internal sealed class AudioTrack : INotifyPropertyChanged
{
    private static readonly Brush MicrophoneBrush = Frozen(new SolidColorBrush(Color.FromRgb(0xFF, 0x95, 0x00)));
    private static readonly Brush ComputerBrush = Frozen(new SolidColorBrush(Color.FromRgb(0x30, 0xB0, 0xC7)));
    private float _volume = 1;
    private bool _isMuted;

    public event PropertyChangedEventHandler? PropertyChanged;

    public required int StreamIndex { get; init; }
    public required string Name { get; init; }
    public required bool IsMicrophone { get; init; }

    /// <summary>0..1; bound to the lane header's slider.</summary>
    public float Volume
    {
        get => _volume;
        set
        {
            var clamped = Math.Clamp(value, 0, 1);
            if (clamped == _volume) return;
            _volume = clamped;
            Notify();
        }
    }

    public bool IsMuted
    {
        get => _isMuted;
        set
        {
            if (value == _isMuted) return;
            _isMuted = value;
            Notify();
        }
    }

    public float EffectiveVolume => IsMuted ? 0 : Volume;
    public string VolumeLabel => IsMuted ? "Muted" : $"{Math.Round(Volume * 100)}%";
    /// <summary>Segoe MDL2 Assets: Mute, Microphone or Volume.</summary>
    public string MuteGlyph => IsMuted ? "\uE74F" : IsMicrophone ? "\uE720" : "\uE767";
    public Brush Brush => IsMuted ? Brushes.Gray : IsMicrophone ? MicrophoneBrush : ComputerBrush;

    /// <summary>Peak levels (0..1) evenly spaced over the whole source; empty until loaded.</summary>
    public float[] Waveform { get; set; } = Array.Empty<float>();
    /// <summary>The track extracted as 48 kHz stereo float WAV, for playback.</summary>
    public string? WavPath { get; set; }

    private void Notify()
    {
        foreach (var name in new[] { nameof(Volume), nameof(IsMuted), nameof(EffectiveVolume), nameof(VolumeLabel), nameof(MuteGlyph), nameof(Brush) })
        {
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
        }
    }

    private static Brush Frozen(SolidColorBrush brush)
    {
        brush.Freeze();
        return brush;
    }
}

/// <summary>
/// Editing state for one video. The edit is a list of kept segments in source order; everything between
/// them is cut. The source file is never modified, and every export writes a new file.
/// </summary>
internal sealed class EditorModel
{
    public const double MinimumSegmentDuration = 0.1;
    /// <summary>How far before a clip's start a reported playback time still counts as inside it.</summary>
    private const double BoundaryTolerance = 0.005;

    private List<Segment> _segments = new();
    private readonly Stack<List<Segment>> _undo = new();
    private readonly Stack<List<Segment>> _redo = new();
    private string? _exportedState;

    public string SourcePath { get; }
    public MediaInfo? Info { get; private set; }
    public double Duration { get; private set; }
    public IReadOnlyList<Segment> Segments => _segments;
    public List<AudioTrack> AudioTracks { get; } = new();
    public IReadOnlyList<BitmapSource> Thumbnails { get; set; } = Array.Empty<BitmapSource>();
    public Guid? SelectedId { get; set; }
    public double CurrentTime { get; set; }

    /// <summary>Raised whenever clips, selection or audio levels change, so views can redraw.</summary>
    public event Action? Changed;

    public EditorModel(string sourcePath) => SourcePath = sourcePath;

    public void Load(MediaInfo info)
    {
        Info = info;
        Duration = info.Duration;
        _segments = new List<Segment> { Segment.Create(0, Duration) };

        foreach (var stream in info.AudioStreams)
        {
            // Cappa titles its tracks; for other videos fall back to channel count (Cappa records the mic in mono).
            var isMic = stream.Title?.Contains("Microphone", StringComparison.OrdinalIgnoreCase)
                        ?? (stream.Channels == 1);
            var name = stream.Title
                       ?? (isMic ? "Microphone" : info.AudioStreams.Count > 1 ? "Computer Audio" : "Audio");
            var track = new AudioTrack { StreamIndex = stream.Index, Name = name, IsMicrophone = isMic };
            track.PropertyChanged += (_, _) => RaiseChanged();
            AudioTracks.Add(track);
        }
        RaiseChanged();
    }

    public double EditedDuration => _segments.Sum(s => s.Duration);
    public Segment? Selected => _segments.FirstOrDefault(s => s.Id == SelectedId);
    public bool CanUndo => _undo.Count > 0;
    public bool CanRedo => _redo.Count > 0;
    public bool CanDelete => _segments.Count > 1 && Selected is not null;

    public bool HasEdits =>
        !(_segments.Count == 1 && _segments[0].Start == 0 && _segments[0].End == Duration)
        || AudioTracks.Any(t => t.EffectiveVolume != 1);

    public bool HasUnexportedEdits => HasEdits && StateKey() != _exportedState;

    public void MarkExported() => _exportedState = StateKey();

    /// <summary>The clip at <paramref name="time"/>, allowing for playback reporting a time just before a clip's start.</summary>
    public Segment? SegmentAt(double time) =>
        _segments.FirstOrDefault(s => time >= s.Start - BoundaryTolerance && time < s.End);

    /// <summary>The first clip starting at or after <paramref name="time"/>.</summary>
    public Segment? NextSegmentAfter(double time) =>
        _segments.FirstOrDefault(s => s.Start >= time - BoundaryTolerance);

    public bool CanSplitAt(double time) =>
        _segments.FirstOrDefault(s => s.Start <= time && time < s.End) is { } segment
        && time - segment.Start >= MinimumSegmentDuration
        && segment.End - time >= MinimumSegmentDuration;

    public void SplitAt(double time)
    {
        time = Snap(time);
        if (!CanSplitAt(time)) return;
        var index = _segments.FindIndex(s => s.Start <= time && time < s.End);
        RecordUndo();
        var original = _segments[index];
        _segments[index] = original with { End = time };
        var second = Segment.Create(time, original.End);
        _segments.Insert(index + 1, second);
        SelectedId = second.Id;
        RaiseChanged();
    }

    public void DeleteSelected()
    {
        if (!CanDelete) return;
        RecordUndo();
        _segments.RemoveAll(s => s.Id == SelectedId);
        SelectedId = null;
        RaiseChanged();
    }

    public void BeginTrim(Guid id)
    {
        SelectedId = id;
        RecordUndo();
    }

    /// <summary>Moves one edge of a clip; clips can't overlap their neighbours or get shorter than the minimum.</summary>
    public double Trim(Guid id, TrimEdge edge, double time)
    {
        time = Snap(time);
        var index = _segments.FindIndex(s => s.Id == id);
        if (index < 0) return time;
        var segment = _segments[index];

        if (edge == TrimEdge.Start)
        {
            var lower = index > 0 ? _segments[index - 1].End : 0;
            var start = Math.Clamp(time, lower, segment.End - MinimumSegmentDuration);
            _segments[index] = segment with { Start = start };
            RaiseChanged();
            return start;
        }

        var upper = index < _segments.Count - 1 ? _segments[index + 1].Start : Duration;
        var end = Math.Clamp(time, segment.Start + MinimumSegmentDuration, upper);
        _segments[index] = segment with { End = end };
        RaiseChanged();
        return end;
    }

    public void EndTrim()
    {
        // Drop the undo step if the drag didn't change anything.
        if (_undo.Count > 0 && _undo.Peek().SequenceEqual(_segments))
        {
            _undo.Pop();
        }
        RaiseChanged();
    }

    public void Undo()
    {
        if (_undo.Count == 0) return;
        _redo.Push(new List<Segment>(_segments));
        _segments = _undo.Pop();
        ClearStaleSelection();
        RaiseChanged();
    }

    public void Redo()
    {
        if (_redo.Count == 0) return;
        _undo.Push(new List<Segment>(_segments));
        _segments = _redo.Pop();
        ClearStaleSelection();
        RaiseChanged();
    }

    public void ToggleMute(AudioTrack track) => track.IsMuted = !track.IsMuted;

    public void RaiseChanged() => Changed?.Invoke();

    /// <summary>Clip edges are kept on a millisecond grid so seeking to a clip's start lands exactly on it.</summary>
    private static double Snap(double time) => Math.Round(time, 3);

    private void RecordUndo()
    {
        _undo.Push(new List<Segment>(_segments));
        _redo.Clear();
    }

    private void ClearStaleSelection()
    {
        if (Selected is null) SelectedId = null;
    }

    private string StateKey() =>
        string.Join(";", _segments.Select(s => $"{s.Start:0.000}-{s.End:0.000}"))
        + "|" + string.Join(";", AudioTracks.Select(t => t.EffectiveVolume.ToString("0.00")));
}
