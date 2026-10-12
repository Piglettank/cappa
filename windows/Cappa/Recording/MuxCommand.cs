using System.Collections.Generic;

namespace Cappa.Recording;

/// <summary>
/// FFmpeg arguments that combine the recorded video with the system-audio and microphone WAVs into one MP4.
/// Each audio source becomes its own titled AAC track, so the editor can adjust them separately.
/// </summary>
internal static class MuxCommand
{
    public static IReadOnlyList<string> Arguments(string videoPath, string? systemAudioPath, string? microphonePath, string outputPath)
    {
        var tracks = new List<(string Path, string Title, int Channels, string Bitrate)>();
        if (systemAudioPath is not null) tracks.Add((systemAudioPath, "Computer Audio", 2, "160k"));
        if (microphonePath is not null) tracks.Add((microphonePath, "Microphone", 1, "96k"));

        var args = new List<string> { "-i", videoPath };
        foreach (var track in tracks) args.AddRange(new[] { "-i", track.Path });
        args.AddRange(new[] { "-map", "0:v:0" });
        for (var i = 0; i < tracks.Count; i++) args.AddRange(new[] { "-map", $"{i + 1}:a:0" });
        args.AddRange(new[] { "-c:v", "copy", "-c:a", "aac", "-ar", "48000" });
        for (var i = 0; i < tracks.Count; i++)
        {
            args.AddRange(new[]
            {
                $"-ac:a:{i}", tracks[i].Channels.ToString(),
                $"-b:a:{i}", tracks[i].Bitrate,
                $"-metadata:s:a:{i}", $"title={tracks[i].Title}",
                $"-metadata:s:a:{i}", $"handler_name={tracks[i].Title}", // shown as the track name by most players
            });
        }
        args.AddRange(new[] { "-movflags", "+faststart", outputPath });
        return args;
    }
}
