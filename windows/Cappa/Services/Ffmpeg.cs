using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace Cappa.Services;

/// <summary>Runs the bundled ffmpeg.exe / ffprobe.exe (in an "ffmpeg" folder next to Cappa.exe).</summary>
internal static class Ffmpeg
{
    public static string FfmpegPath => Locate("ffmpeg.exe");
    public static string FfprobePath => Locate("ffprobe.exe");

    /// <summary>Formats seconds for ffmpeg arguments with a dot decimal separator regardless of locale.</summary>
    public static string Seconds(double value) => value.ToString("0.######", CultureInfo.InvariantCulture);

    /// <summary>
    /// Runs ffmpeg and reports progress as a fraction of <paramref name="expectedDuration"/>.
    /// Throws with the tail of ffmpeg's output if it fails.
    /// </summary>
    public static async Task RunAsync(
        IReadOnlyList<string> arguments,
        double expectedDuration = 0,
        IProgress<double>? progress = null,
        CancellationToken cancellationToken = default)
    {
        var args = new List<string> { "-hide_banner", "-nostdin", "-y" };
        if (progress is not null)
        {
            args.AddRange(new[] { "-progress", "pipe:1", "-nostats" });
        }
        args.AddRange(arguments);

        var start = new ProcessStartInfo(FfmpegPath)
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding = Encoding.UTF8,
        };
        foreach (var argument in args) start.ArgumentList.Add(argument);

        Log.Write("ffmpeg " + string.Join(" ", args.Select(Quote)));
        using var process = new Process { StartInfo = start };
        var errorTail = new Queue<string>();

        process.ErrorDataReceived += (_, e) =>
        {
            if (e.Data is null) return;
            lock (errorTail)
            {
                errorTail.Enqueue(e.Data);
                while (errorTail.Count > 30) errorTail.Dequeue();
            }
        };
        process.OutputDataReceived += (_, e) =>
        {
            // -progress writes key=value lines; out_time_us is the position in the output.
            if (progress is null || expectedDuration <= 0 || e.Data is null) return;
            if (e.Data.StartsWith("out_time_us=", StringComparison.Ordinal)
                && long.TryParse(e.Data.AsSpan("out_time_us=".Length), out var microseconds))
            {
                progress.Report(Math.Clamp(microseconds / 1_000_000.0 / expectedDuration, 0, 1));
            }
        };

        process.Start();
        process.BeginErrorReadLine();
        process.BeginOutputReadLine();

        using (cancellationToken.Register(() => { try { process.Kill(); } catch { } }))
        {
            await process.WaitForExitAsync(CancellationToken.None).ConfigureAwait(false);
        }
        cancellationToken.ThrowIfCancellationRequested();

        if (process.ExitCode != 0)
        {
            string tail;
            lock (errorTail) tail = string.Join(Environment.NewLine, errorTail);
            Log.Write($"ffmpeg failed ({process.ExitCode}):{Environment.NewLine}{tail}");
            throw new InvalidOperationException($"FFmpeg failed (exit code {process.ExitCode}). Details are in the Cappa log.");
        }
        progress?.Report(1);
    }

    /// <summary>Stream and format information for a media file.</summary>
    public static async Task<MediaInfo> ProbeAsync(string path)
    {
        var start = new ProcessStartInfo(FfprobePath)
        {
            UseShellExecute = false,
            CreateNoWindow = true,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
        };
        foreach (var argument in new[] { "-v", "error", "-print_format", "json", "-show_format", "-show_streams", path })
        {
            start.ArgumentList.Add(argument);
        }

        using var process = Process.Start(start) ?? throw new InvalidOperationException("Could not start ffprobe.");
        var json = await process.StandardOutput.ReadToEndAsync().ConfigureAwait(false);
        await process.WaitForExitAsync().ConfigureAwait(false);
        if (process.ExitCode != 0)
        {
            throw new InvalidOperationException($"ffprobe could not read {Path.GetFileName(path)}.");
        }
        return MediaInfo.Parse(json);
    }

    private static string Locate(string executable)
    {
        var bundled = Path.Combine(AppContext.BaseDirectory, "ffmpeg", executable);
        if (File.Exists(bundled)) return bundled;
        var besideExe = Path.Combine(AppContext.BaseDirectory, executable);
        if (File.Exists(besideExe)) return besideExe;
        // Fall back to PATH (without .exe off Windows, so the FFmpeg commands can be tested on macOS too).
        return OperatingSystem.IsWindows() ? executable : Path.GetFileNameWithoutExtension(executable);
    }

    private static string Quote(string argument) =>
        argument.Contains(' ') || argument.Contains(';') ? $"\"{argument}\"" : argument;
}

internal sealed record AudioStreamInfo(int Index, int Channels, string? Title);

internal sealed record MediaInfo(double Duration, int Width, int Height, double FrameRate, IReadOnlyList<AudioStreamInfo> AudioStreams)
{
    public static MediaInfo Parse(string json)
    {
        using var document = JsonDocument.Parse(json);
        var root = document.RootElement;

        double duration = 0;
        if (root.TryGetProperty("format", out var format) && format.TryGetProperty("duration", out var d))
        {
            double.TryParse(d.GetString(), NumberStyles.Float, CultureInfo.InvariantCulture, out duration);
        }

        int width = 0, height = 0;
        double frameRate = 30;
        var audio = new List<AudioStreamInfo>();
        var audioIndex = 0;

        foreach (var stream in root.GetProperty("streams").EnumerateArray())
        {
            var type = stream.TryGetProperty("codec_type", out var t) ? t.GetString() : null;
            if (type == "video" && width == 0)
            {
                width = stream.TryGetProperty("width", out var w) ? w.GetInt32() : 0;
                height = stream.TryGetProperty("height", out var h) ? h.GetInt32() : 0;
                if (stream.TryGetProperty("avg_frame_rate", out var rate)) frameRate = ParseRate(rate.GetString()) ?? frameRate;
            }
            else if (type == "audio")
            {
                var channels = stream.TryGetProperty("channels", out var c) ? c.GetInt32() : 2;
                string? title = null;
                // MP4 reports a track title as "name" (other containers as "title").
                if (stream.TryGetProperty("tags", out var tags)
                    && (tags.TryGetProperty("title", out var titleElement) || tags.TryGetProperty("name", out titleElement)))
                {
                    title = titleElement.GetString();
                }
                audio.Add(new AudioStreamInfo(audioIndex++, channels, title));
            }
        }
        return new MediaInfo(duration, width, height, frameRate, audio);
    }

    private static double? ParseRate(string? rate)
    {
        if (rate is null) return null;
        var parts = rate.Split('/');
        if (parts.Length == 2
            && double.TryParse(parts[0], NumberStyles.Float, CultureInfo.InvariantCulture, out var numerator)
            && double.TryParse(parts[1], NumberStyles.Float, CultureInfo.InvariantCulture, out var denominator)
            && denominator > 0 && numerator > 0)
        {
            return numerator / denominator;
        }
        return null;
    }
}
