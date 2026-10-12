using System;
using System.Collections.Generic;
using System.Linq;
using System.Windows.Controls;
using System.Windows.Threading;
using Cappa.Services;
using NAudio.Wave;
using NAudio.Wave.SampleProviders;

namespace Cappa.Editor;

/// <summary>
/// Plays the edit: the video through a muted <see cref="MediaElement"/>, and each audio track through NAudio
/// at its own volume, kept in sync with the video. Cut-away parts are skipped.
/// </summary>
internal sealed class EditorPlayback : IDisposable
{
    private const double JumpSettleSeconds = 0.5;
    private const double MaxAudioDrift = 0.08;

    private readonly MediaElement _video;
    private readonly EditorModel _model;
    private readonly DispatcherTimer _timer;
    private readonly List<(AudioTrack Track, AudioFileReader Reader, VolumeSampleProvider Volume)> _audio = new();
    private WaveOutEvent? _output;
    private double? _jumpTarget;
    private DateTime _jumpStartedAt;

    public bool IsPlaying { get; private set; }

    /// <summary>Raised when the playhead moves or playback starts/stops.</summary>
    public event Action? Updated;

    public EditorPlayback(MediaElement video, EditorModel model)
    {
        _video = video;
        _model = model;
        _video.LoadedBehavior = MediaState.Manual;
        _video.UnloadedBehavior = MediaState.Close;
        _video.ScrubbingEnabled = true;
        _video.IsMuted = true;
        _video.MediaEnded += (_, _) => Pause();
        _timer = new DispatcherTimer(DispatcherPriority.Render) { Interval = TimeSpan.FromMilliseconds(15) };
        _timer.Tick += (_, _) => Tick();
        _model.Changed += UpdateVolumes;
    }

    /// <summary>Hooks up the extracted audio tracks once they're ready.</summary>
    public void AttachAudio()
    {
        try
        {
            var mixer = new MixingSampleProvider(WaveFormat.CreateIeeeFloatWaveFormat(48000, 2)) { ReadFully = true };
            foreach (var track in _model.AudioTracks.Where(t => t.WavPath is not null))
            {
                var reader = new AudioFileReader(track.WavPath);
                var volume = new VolumeSampleProvider(reader) { Volume = track.EffectiveVolume };
                mixer.AddMixerInput(volume);
                _audio.Add((track, reader, volume));
            }
            if (_audio.Count == 0) return;

            _output = new WaveOutEvent { DesiredLatency = 100 };
            _output.Init(mixer);
            SeekAudio(_model.CurrentTime);
            if (IsPlaying) _output.Play();
        }
        catch (Exception e)
        {
            Log.Error("Setting up audio playback", e);
        }
    }

    public void TogglePlay()
    {
        if (IsPlaying) Pause(); else Play();
    }

    public void Play()
    {
        if (_model.Segments.Count == 0) return;
        // Starting in a cut part (or at the very end) jumps to the next kept part, wrapping to the start.
        if (_model.SegmentAt(_model.CurrentTime) is null)
        {
            var next = _model.NextSegmentAfter(_model.CurrentTime) ?? _model.Segments[0];
            Seek(next.Start);
        }
        IsPlaying = true;
        _video.Play();
        SeekAudio(_model.CurrentTime);
        _output?.Play();
        _timer.Start();
        Updated?.Invoke();
    }

    public void Pause()
    {
        if (!IsPlaying) return;
        IsPlaying = false;
        _video.Pause();
        _output?.Pause();
        _timer.Stop();
        _model.CurrentTime = _video.Position.TotalSeconds;
        _jumpTarget = null;
        Updated?.Invoke();
    }

    public void Seek(double time)
    {
        time = Math.Clamp(time, 0, _model.Duration);
        _model.CurrentTime = time;
        _video.Position = TimeSpan.FromSeconds(time);
        SeekAudio(time);
        Updated?.Invoke();
    }

    private void Tick()
    {
        if (!IsPlaying) return;
        var time = _video.Position.TotalSeconds;

        // Right after a jump the reported position can still be the old one for a moment.
        // Ignore it until it lands (or a short timeout), otherwise the jump would repeat forever.
        if (_jumpTarget is { } target)
        {
            if (time < target - 0.05 && (DateTime.UtcNow - _jumpStartedAt).TotalSeconds < JumpSettleSeconds) return;
            _jumpTarget = null;
        }

        _model.CurrentTime = time;
        if (_model.SegmentAt(time) is null)
        {
            if (_model.NextSegmentAfter(time) is { } next)
            {
                JumpTo(next.Start);
            }
            else
            {
                Pause();
                Seek(_model.Segments[^1].End);
                return;
            }
        }
        else if (_audio.Count > 0 && Math.Abs(_audio[0].Reader.CurrentTime.TotalSeconds - time) > MaxAudioDrift)
        {
            SeekAudio(time);
        }
        Updated?.Invoke();
    }

    private void JumpTo(double time)
    {
        _jumpTarget = time;
        _jumpStartedAt = DateTime.UtcNow;
        _model.CurrentTime = time;
        _video.Position = TimeSpan.FromSeconds(time);
        SeekAudio(time);
    }

    private void SeekAudio(double time)
    {
        foreach (var (_, reader, _) in _audio)
        {
            var target = TimeSpan.FromSeconds(Math.Clamp(time, 0, reader.TotalTime.TotalSeconds));
            reader.CurrentTime = target;
        }
    }

    private void UpdateVolumes()
    {
        foreach (var (track, _, volume) in _audio) volume.Volume = track.EffectiveVolume;
    }

    public void Dispose()
    {
        _timer.Stop();
        _model.Changed -= UpdateVolumes;
        _output?.Stop();
        _output?.Dispose();
        foreach (var (_, reader, _) in _audio) reader.Dispose();
        _audio.Clear();
        _video.Close();
    }
}
