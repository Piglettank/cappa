using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using System.Windows.Media.Imaging;
using Cappa.Services;
using NAudio.Wave;

namespace Cappa.Editor;

/// <summary>Builds what the editor shows: filmstrip thumbnails, playable audio tracks and their waveforms.</summary>
internal static class MediaPreparation
{
    public const int ThumbnailCount = 24;
    private const int WaveformBuckets = 800;

    public static async Task<IReadOnlyList<BitmapSource>> ThumbnailsAsync(string source, double duration, string workFolder)
    {
        if (duration <= 0) return Array.Empty<BitmapSource>();
        var pattern = Path.Combine(workFolder, "thumb_%02d.jpg");
        var fps = (ThumbnailCount / duration).ToString("0.######", CultureInfo.InvariantCulture);
        await Ffmpeg.RunAsync(new[]
        {
            "-i", source, "-vf", $"fps={fps},scale=-2:90", "-frames:v", ThumbnailCount.ToString(), "-q:v", "4", pattern,
        });

        return Directory.GetFiles(workFolder, "thumb_*.jpg")
            .OrderBy(path => path, StringComparer.Ordinal)
            .Select(LoadFrozen)
            .ToList();
    }

    /// <summary>Extracts one audio stream as 48 kHz stereo float WAV, so all tracks can be mixed for playback.</summary>
    public static async Task<string> ExtractAudioAsync(string source, int streamIndex, string workFolder)
    {
        var path = Path.Combine(workFolder, $"track{streamIndex}.wav");
        await Ffmpeg.RunAsync(new[]
        {
            "-i", source, "-map", $"0:a:{streamIndex}", "-ac", "2", "-ar", "48000", "-c:a", "pcm_f32le", path,
        });
        return path;
    }

    /// <summary>Peak level per bucket over the whole file, scaled to a 50 dB range (0..1).</summary>
    public static float[] Waveform(string wavPath, double duration)
    {
        if (duration <= 0) return Array.Empty<float>();
        var peaks = new float[WaveformBuckets];
        using var reader = new AudioFileReader(wavPath);
        var format = reader.WaveFormat;
        var buffer = new float[format.SampleRate * format.Channels / 10];
        long frame = 0;
        int read;
        while ((read = reader.Read(buffer, 0, buffer.Length)) > 0)
        {
            for (var i = 0; i < read; i += format.Channels)
            {
                var time = frame++ / (double)format.SampleRate;
                var bucket = Math.Clamp((int)(time / duration * WaveformBuckets), 0, WaveformBuckets - 1);
                for (var channel = 0; channel < format.Channels && i + channel < read; channel++)
                {
                    peaks[bucket] = Math.Max(peaks[bucket], Math.Abs(buffer[i + channel]));
                }
            }
        }
        return peaks.Select(p => p <= 0 ? 0f : (float)Math.Max(0, 1 + 20 * Math.Log10(p) / 50)).ToArray();
    }

    private static BitmapSource LoadFrozen(string path)
    {
        var image = new BitmapImage();
        image.BeginInit();
        image.CacheOption = BitmapCacheOption.OnLoad; // read now, so the file isn't kept open
        image.UriSource = new Uri(path);
        image.EndInit();
        image.Freeze();
        return image;
    }
}
