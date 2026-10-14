using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Cappa.Services;
using ScreenRecorderLib;

namespace Cappa.Recording;

/// <summary>What to record: a monitor, optionally cropped to a region (both in physical pixels).</summary>
internal sealed record CaptureTarget(string DeviceName, System.Drawing.Rectangle MonitorBounds, System.Drawing.Rectangle? Region)
{
    /// <summary>The recorded area in virtual-screen pixels.</summary>
    public System.Drawing.Rectangle ScreenArea => Region is { } region
        ? new System.Drawing.Rectangle(MonitorBounds.X + region.X, MonitorBounds.Y + region.Y, region.Width, region.Height)
        : MonitorBounds;
}

/// <summary>
/// Video-only screen capture to an MP4 file via ScreenRecorderLib (Windows Graphics Capture + hardware H.264).
/// Audio is recorded separately so system audio and microphone end up on their own tracks.
/// </summary>
internal sealed class ScreenCapture : IDisposable
{
    private readonly Recorder _recorder;
    private readonly TaskCompletionSource _started = new(TaskCreationOptions.RunContinuationsAsynchronously);
    private readonly TaskCompletionSource<string> _completed = new(TaskCreationOptions.RunContinuationsAsynchronously);

    /// <summary>Raised (on a background thread) if recording fails after it started.</summary>
    public event Action<string>? Failed;

    public ScreenCapture(CaptureTarget target, AppSettings settings)
    {
        var source = new DisplayRecordingSource(target.DeviceName)
        {
            RecorderApi = RecorderApi.WindowsGraphicsCapture,
            IsBorderRequired = false,
            IsCursorCaptureEnabled = settings.ShowCursor,
        };
        if (target.Region is { } region)
        {
            source.SourceRect = new ScreenRect(region.X, region.Y, region.Width, region.Height);
        }

        var options = new RecorderOptions
        {
            SourceOptions = new SourceOptions
            {
                RecordingSources = new List<RecordingSourceBase> { source },
            },
            OutputOptions = new OutputOptions
            {
                RecorderMode = RecorderMode.Video,
            },
            AudioOptions = new AudioOptions
            {
                IsAudioEnabled = false,
            },
            // Click highlights are drawn by UI/ClickHighlighter, which looks nicer than the library's dot.
            MouseOptions = new MouseOptions { IsMousePointerEnabled = settings.ShowCursor },
            VideoEncoderOptions = new VideoEncoderOptions
            {
                Encoder = new H264VideoEncoder { BitrateMode = H264BitrateControlMode.Quality },
                Quality = settings.Quality switch
                {
                    RecordingQuality.Low => 50,
                    RecordingQuality.Medium => 65,
                    _ => 80,
                },
                Framerate = settings.FrameRate,
                IsFixedFramerate = false,
                IsHardwareEncodingEnabled = true,
            },
        };

        _recorder = Recorder.CreateRecorder(options);
        _recorder.OnStatusChanged += (_, e) =>
        {
            Log.Write($"Screen capture status: {e.Status}");
            if (e.Status == RecorderStatus.Recording) _started.TrySetResult();
        };
        _recorder.OnRecordingComplete += (_, e) =>
        {
            Log.Write($"Screen capture complete: {e.FilePath}");
            _completed.TrySetResult(e.FilePath);
        };
        _recorder.OnRecordingFailed += (_, e) =>
        {
            Log.Write($"Screen capture failed: {e.Error}");
            var error = new InvalidOperationException(e.Error);
            _started.TrySetException(error);
            if (_completed.TrySetException(error)) Failed?.Invoke(e.Error);
        };
    }

    /// <summary>Starts recording and waits until the first frame is being captured.</summary>
    public async Task StartAsync(string filePath)
    {
        _recorder.Record(filePath);
        var finished = await Task.WhenAny(_started.Task, Task.Delay(TimeSpan.FromSeconds(10)));
        if (finished != _started.Task)
        {
            throw new TimeoutException("Screen recording didn't start within 10 seconds.");
        }
        await _started.Task; // surfaces a start failure
    }

    public void Pause() => _recorder.Pause();

    public void Resume() => _recorder.Resume();

    /// <summary>Stops and returns the path of the finished video file.</summary>
    public async Task<string> StopAsync()
    {
        _recorder.Stop();
        var finished = await Task.WhenAny(_completed.Task, Task.Delay(TimeSpan.FromSeconds(30)));
        if (finished != _completed.Task)
        {
            throw new TimeoutException("Screen recording didn't finish writing within 30 seconds.");
        }
        return await _completed.Task;
    }

    public void Dispose() => _recorder.Dispose();
}
