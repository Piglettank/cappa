using System;
using System.Collections.Generic;
using System.IO;
using System.Threading.Tasks;
using Cappa.Services;

namespace Cappa.Recording;

/// <summary>
/// One recording: screen video plus separate system-audio and microphone tracks, recorded into a temporary
/// folder and combined into the final MP4 when stopped. The audio tracks are titled "Computer Audio" and
/// "Microphone" so the editor can tell them apart.
/// </summary>
internal sealed class RecordingSession : IDisposable
{
    private readonly string _workFolder;
    private readonly ScreenCapture _screen;
    private readonly AudioTrackRecorder? _systemAudio;
    private readonly AudioTrackRecorder? _microphone;

    public CaptureTarget Target { get; }

    /// <summary>Raised (on a background thread) when capture fails mid-recording.</summary>
    public event Action<string>? Failed;

    private RecordingSession(CaptureTarget target, bool recordSystemAudio, bool recordMicrophone, string? microphoneId)
    {
        Target = target;
        _workFolder = AppPaths.TempFolder("Recording-" + Guid.NewGuid().ToString("N"));
        _screen = new ScreenCapture(target);
        _screen.Failed += error => Failed?.Invoke(error);
        if (recordSystemAudio) _systemAudio = AudioTrackRecorder.SystemAudio(Path.Combine(_workFolder, "system.wav"));
        if (recordMicrophone) _microphone = AudioTrackRecorder.Microphone(microphoneId, Path.Combine(_workFolder, "microphone.wav"));
    }

    public static async Task<RecordingSession> StartAsync(CaptureTarget target, AppSettings settings)
    {
        Log.Write($"Starting recording of {target.DeviceName}, area {target.ScreenArea}, system audio {settings.RecordSystemAudio}, mic {settings.RecordMicrophone}");
        var session = new RecordingSession(target, settings.RecordSystemAudio, settings.RecordMicrophone, settings.MicrophoneId);
        try
        {
            await session._screen.StartAsync(Path.Combine(session._workFolder, "video.mp4"));
            // Audio starts once frames are flowing, so both begin at (nearly) the same moment.
            session._systemAudio?.Start();
            session._microphone?.Start();
            return session;
        }
        catch
        {
            session.Dispose();
            throw;
        }
    }

    public void Pause()
    {
        _screen.Pause();
        _systemAudio?.Pause();
        _microphone?.Pause();
    }

    public void Resume()
    {
        _screen.Resume();
        _systemAudio?.Resume();
        _microphone?.Resume();
    }

    /// <summary>Stops all capture and writes the finished recording to <paramref name="outputPath"/>.</summary>
    public async Task StopAsync(string outputPath)
    {
        var videoTask = _screen.StopAsync();
        _systemAudio?.Stop();
        _microphone?.Stop();
        var videoPath = await videoTask;

        if (_systemAudio is null && _microphone is null)
        {
            File.Move(videoPath, outputPath, overwrite: true);
            return;
        }
        await Ffmpeg.RunAsync(MuxCommand.Arguments(videoPath, _systemAudio?.FilePath, _microphone?.FilePath, outputPath));
    }

    public void Dispose()
    {
        _systemAudio?.Dispose();
        _microphone?.Dispose();
        _screen.Dispose();
        try
        {
            Directory.Delete(_workFolder, recursive: true);
        }
        catch (Exception e)
        {
            Log.Error("Removing recording work folder", e);
        }
    }
}
