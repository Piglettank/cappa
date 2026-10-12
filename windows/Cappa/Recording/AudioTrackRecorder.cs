using System;
using System.Diagnostics;
using System.Threading;
using Cappa.Services;
using NAudio.CoreAudioApi;
using NAudio.Wave;

namespace Cappa.Recording;

/// <summary>
/// Records one audio source (system audio or a microphone) to a WAV file.
///
/// WASAPI loopback delivers no data while nothing is playing, which would make the track shorter than the
/// video and drift out of sync. So the recorder keeps its own clock and fills any gap longer than 50 ms
/// with silence. Paused time is neither recorded nor counted.
/// </summary>
internal sealed class AudioTrackRecorder : IDisposable
{
    private readonly IWaveIn _capture;
    private readonly WaveFileWriter _writer;
    private readonly string _label;
    private readonly object _gate = new();
    private readonly Stopwatch _clock = new();
    private readonly ManualResetEventSlim _stopped = new(false);
    private long _writtenBytes;
    private bool _paused;
    private bool _finished;

    public string FilePath { get; }

    private AudioTrackRecorder(IWaveIn capture, string filePath, string label)
    {
        _capture = capture;
        _label = label;
        FilePath = filePath;
        _writer = new WaveFileWriter(filePath, capture.WaveFormat);
        _capture.DataAvailable += OnDataAvailable;
        _capture.RecordingStopped += (_, e) =>
        {
            if (e.Exception is not null) Log.Error($"{_label} capture stopped", e.Exception);
            _stopped.Set();
        };
        Log.Write($"{_label}: {capture.WaveFormat}");
    }

    /// <summary>Everything the default playback device plays.</summary>
    public static AudioTrackRecorder SystemAudio(string filePath) =>
        new(new WasapiLoopbackCapture(), filePath, "System audio");

    /// <summary>A specific microphone by endpoint ID, or the Windows default input when it's empty or missing.</summary>
    public static AudioTrackRecorder Microphone(string? deviceId, string filePath)
    {
        using var enumerator = new MMDeviceEnumerator();
        MMDevice? device = null;
        if (!string.IsNullOrEmpty(deviceId))
        {
            try
            {
                device = enumerator.GetDevice(deviceId);
                if (device.State != DeviceState.Active) device = null;
            }
            catch (Exception e)
            {
                Log.Error("Chosen microphone not available, using the default", e);
                device = null;
            }
        }
        device ??= enumerator.GetDefaultAudioEndpoint(DataFlow.Capture, Role.Console);
        Log.Write($"Microphone: {device.FriendlyName}");
        return new AudioTrackRecorder(new WasapiCapture(device), filePath, "Microphone");
    }

    public void Start()
    {
        lock (_gate) _clock.Start();
        _capture.StartRecording();
    }

    public void Pause()
    {
        lock (_gate)
        {
            _paused = true;
            _clock.Stop();
        }
    }

    public void Resume()
    {
        lock (_gate)
        {
            _paused = false;
            _clock.Start();
        }
    }

    /// <summary>Stops capturing and closes the WAV file, padded with silence up to the elapsed recording time.</summary>
    public void Stop()
    {
        try
        {
            _capture.StopRecording();
            _stopped.Wait(TimeSpan.FromSeconds(3));
        }
        catch (Exception e)
        {
            Log.Error($"Stopping {_label}", e);
        }

        lock (_gate)
        {
            if (_finished) return;
            _finished = true;
            _clock.Stop();
            PadTo(ExpectedBytes(), threshold: 0);
            _writer.Dispose();
        }
    }

    public void Dispose()
    {
        Stop();
        _capture.Dispose();
        _stopped.Dispose();
    }

    private void OnDataAvailable(object? sender, WaveInEventArgs e)
    {
        lock (_gate)
        {
            if (_paused || _finished || e.BytesRecorded == 0) return;
            PadTo(ExpectedBytes() - e.BytesRecorded, threshold: _writer.WaveFormat.AverageBytesPerSecond / 20);
            _writer.Write(e.Buffer, 0, e.BytesRecorded);
            _writtenBytes += e.BytesRecorded;
        }
    }

    private long ExpectedBytes()
    {
        var format = _writer.WaveFormat;
        var bytes = (long)(_clock.Elapsed.TotalSeconds * format.AverageBytesPerSecond);
        return bytes - bytes % format.BlockAlign;
    }

    /// <summary>Writes silence until the file holds <paramref name="targetBytes"/>, if it's behind by more than the threshold.</summary>
    private void PadTo(long targetBytes, long threshold)
    {
        var missing = targetBytes - _writtenBytes;
        if (missing <= threshold) return;
        var blockAlign = _writer.WaveFormat.BlockAlign;
        missing -= missing % blockAlign;

        var chunkSize = 64 * 1024 - 64 * 1024 % blockAlign;
        var silence = new byte[Math.Min(missing, chunkSize)];
        while (missing > 0 && silence.Length > 0)
        {
            var chunk = (int)Math.Min(missing, silence.Length);
            _writer.Write(silence, 0, chunk);
            _writtenBytes += chunk;
            missing -= chunk;
        }
    }
}
