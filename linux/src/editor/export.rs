//! Writes the edit to a new file with FFmpeg: the kept clips joined back to back, and every audible audio
//! track mixed into one at its chosen level (many players only play the first audio track).

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Result;

use super::model::Segment;
use crate::ffmpeg::{self, seconds as s};

pub const GIF_FRAMES_PER_SECOND: u32 = 20;
pub const GIF_MAXIMUM_SIZE: u32 = 800;

/// An audio stream of the source and the level it's exported at (0 leaves it out).
pub struct ExportAudio {
    pub stream_index: usize,
    pub volume: f32,
}

pub async fn save_video(
    source: &Path,
    segments: &[Segment],
    audio: &[ExportAudio],
    output: &Path,
    progress: async_channel::Sender<f64>,
) -> Result<()> {
    let mut graph = String::new();
    append_video(&mut graph, segments, "vout");
    graph.push(';');

    let audible: Vec<&ExportAudio> = audio.iter().filter(|t| t.volume > 0.0).collect();
    for (t, track) in audible.iter().enumerate() {
        for (i, segment) in segments.iter().enumerate() {
            // apad first, so a track that ends early still fills every clip with (silent) audio.
            let _ = write!(
                graph,
                "[0:a:{}]apad,atrim=start={}:end={},asetpts=PTS-STARTPTS,aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo[a{t}_{i}];",
                track.stream_index,
                s(segment.start),
                s(segment.end)
            );
        }
        for i in 0..segments.len() {
            let _ = write!(graph, "[a{t}_{i}]");
        }
        let _ = write!(graph, "concat=n={}:v=0:a=1,volume={}[t{t}];", segments.len(), s(track.volume as f64));
    }
    match audible.len() {
        0 => {}
        1 => graph.push_str("[t0]anull[aout]"),
        count => {
            for t in 0..count {
                let _ = write!(graph, "[t{t}]");
            }
            let _ = write!(graph, "amix=inputs={count}:normalize=0:duration=longest[aout]");
        }
    }

    let mut args: Vec<String> = vec![
        "-i".into(),
        source.to_string_lossy().into_owned(),
        "-filter_complex".into(),
        graph.trim_end_matches(';').to_owned(),
        "-map".into(),
        "[vout]".into(),
    ];
    if !audible.is_empty() {
        args.extend(["-map", "[aout]", "-c:a", "aac", "-b:a", "192k"].map(String::from));
    }
    args.extend(ffmpeg::h264_encoder_args());
    args.extend(["-movflags".into(), "+faststart".into(), output.to_string_lossy().into_owned()]);
    ffmpeg::run(args, total_duration(segments), Some(progress)).await
}

/// A looping GIF at `GIF_FRAMES_PER_SECOND`, scaled to fit `GIF_MAXIMUM_SIZE`, without audio.
pub async fn save_gif(source: &Path, segments: &[Segment], output: &Path, progress: async_channel::Sender<f64>) -> Result<()> {
    let mut graph = String::new();
    append_video(&mut graph, segments, "joined");
    let _ = write!(
        graph,
        ";[joined]fps={GIF_FRAMES_PER_SECOND},scale={GIF_MAXIMUM_SIZE}:{GIF_MAXIMUM_SIZE}:force_original_aspect_ratio=decrease:flags=lanczos,\
         split[g1][g2];[g1]palettegen=stats_mode=diff[palette];[g2][palette]paletteuse=dither=bayer:bayer_scale=5[gif]"
    );
    let args = vec![
        "-i".into(),
        source.to_string_lossy().into_owned(),
        "-filter_complex".into(),
        graph,
        "-map".into(),
        "[gif]".into(),
        "-loop".into(),
        "0".into(),
        output.to_string_lossy().into_owned(),
    ];
    ffmpeg::run(args, total_duration(segments), Some(progress)).await
}

fn append_video(graph: &mut String, segments: &[Segment], output: &str) {
    for (i, segment) in segments.iter().enumerate() {
        let _ = write!(graph, "[0:v:0]trim=start={}:end={},setpts=PTS-STARTPTS[v{i}];", s(segment.start), s(segment.end));
    }
    for i in 0..segments.len() {
        let _ = write!(graph, "[v{i}]");
    }
    let _ = write!(graph, "concat=n={}:v=1:a=0[{output}]", segments.len());
}

fn total_duration(segments: &[Segment]) -> f64 {
    segments.iter().map(Segment::duration).sum()
}
