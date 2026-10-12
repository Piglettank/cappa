using System;

namespace Cappa.Editor;

/// <summary>A kept part of the source video, in source-time seconds.</summary>
internal sealed record Segment(Guid Id, double Start, double End)
{
    public double Duration => End - Start;

    public static Segment Create(double start, double end) => new(Guid.NewGuid(), start, end);
}

internal enum TrimEdge { Start, End }
