using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text;
using System.Threading.Tasks;
using Cappa.Services;

namespace Cappa.Editor;

/// <summary>
/// Writes the edit to a new file with FFmpeg: the kept clips joined back to back, and every audible audio
/// track mixed into one at its chosen level (many players only play the first audio track).
/// </summary>
internal static partial class Exporter
{
    public const int GifFramesPerSecond = 20;
    public const int GifMaximumSize = 800;

    public static Task SaveVideoAsync(
        string source, IReadOnlyList<Segment> segments, IReadOnlyList<ExportAudio> audio, string outputPath, IProgress<double>? progress)
    {
        var graph = new StringBuilder();
        AppendVideo(graph, segments, "vout");

        var audible = audio.Where(t => t.Volume > 0).ToList();
        for (var t = 0; t < audible.Count; t++)
        {
            var track = audible[t];
            for (var i = 0; i < segments.Count; i++)
            {
                var segment = segments[i];
                // apad first, so a track that ends early still fills every clip with (silent) audio.
                graph.Append($"[0:a:{track.StreamIndex}]apad,atrim=start={S(segment.Start)}:end={S(segment.End)},asetpts=PTS-STARTPTS,"
                             + $"aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo[a{t}_{i}];");
            }
            for (var i = 0; i < segments.Count; i++) graph.Append($"[a{t}_{i}]");
            graph.Append($"concat=n={segments.Count}:v=0:a=1,volume={track.Volume.ToString("0.###", CultureInfo.InvariantCulture)}[t{t}];");
        }
        if (audible.Count == 1)
        {
            graph.Append("[t0]anull[aout]");
        }
        else if (audible.Count > 1)
        {
            for (var t = 0; t < audible.Count; t++) graph.Append($"[t{t}]");
            graph.Append($"amix=inputs={audible.Count}:normalize=0:duration=longest[aout]");
        }

        var args = new List<string> { "-i", source, "-filter_complex", graph.ToString().TrimEnd(';'), "-map", "[vout]" };
        if (audible.Count > 0) args.AddRange(new[] { "-map", "[aout]", "-c:a", "aac", "-b:a", "192k" });
        args.AddRange(new[]
        {
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p",
            "-movflags", "+faststart", outputPath,
        });
        return Ffmpeg.RunAsync(args, segments.Sum(s => s.Duration), progress);
    }

    /// <summary>A looping GIF at <see cref="GifFramesPerSecond"/>, scaled to fit <see cref="GifMaximumSize"/>, without audio.</summary>
    public static Task SaveGifAsync(string source, IReadOnlyList<Segment> segments, string outputPath, IProgress<double>? progress)
    {
        var graph = new StringBuilder();
        AppendVideo(graph, segments, "joined");
        graph.Append($";[joined]fps={GifFramesPerSecond},"
                     + $"scale={GifMaximumSize}:{GifMaximumSize}:force_original_aspect_ratio=decrease:flags=lanczos,"
                     + "split[g1][g2];[g1]palettegen=stats_mode=diff[palette];[g2][palette]paletteuse=dither=bayer:bayer_scale=5[gif]");

        return Ffmpeg.RunAsync(new[]
        {
            "-i", source, "-filter_complex", graph.ToString(), "-map", "[gif]", "-loop", "0", outputPath,
        }, segments.Sum(s => s.Duration), progress);
    }

    private static void AppendVideo(StringBuilder graph, IReadOnlyList<Segment> segments, string output)
    {
        for (var i = 0; i < segments.Count; i++)
        {
            graph.Append($"[0:v:0]trim=start={S(segments[i].Start)}:end={S(segments[i].End)},setpts=PTS-STARTPTS[v{i}];");
        }
        for (var i = 0; i < segments.Count; i++) graph.Append($"[v{i}]");
        graph.Append($"concat=n={segments.Count}:v=1:a=0[{output}]");
        if (output == "vout") graph.Append(';');
    }

    private static string S(double seconds) => Ffmpeg.Seconds(seconds);
}

/// <summary>An audio stream of the source and the level it's exported at (0 leaves it out).</summary>
internal sealed record ExportAudio(int StreamIndex, float Volume);
