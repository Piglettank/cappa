//! Builds what the editor shows besides the video itself: filmstrip thumbnails and audio waveforms.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use gtk::{gdk, gio};

use crate::ffmpeg;

pub const THUMBNAIL_COUNT: usize = 24;
const WAVEFORM_BUCKETS: usize = 800;
/// Samples per second decoded for waveforms; plenty for an 800-bar overview.
const WAVEFORM_RATE: usize = 8000;

pub async fn thumbnails(source: &Path, duration: f64, work_folder: &Path) -> Result<Vec<gdk::Texture>> {
    if duration <= 0.0 {
        return Ok(Vec::new());
    }
    let pattern = work_folder.join("thumb_%02d.jpg");
    let fps = ffmpeg::seconds(THUMBNAIL_COUNT as f64 / duration);
    let args = vec![
        "-i".into(),
        source.to_string_lossy().into_owned(),
        "-vf".into(),
        format!("fps={fps},scale=-2:90"),
        "-frames:v".into(),
        THUMBNAIL_COUNT.to_string(),
        "-q:v".into(),
        "4".into(),
        pattern.to_string_lossy().into_owned(),
    ];
    ffmpeg::run(args, 0.0, None).await?;

    let mut paths: Vec<PathBuf> = std::fs::read_dir(work_folder)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("thumb_")))
        .collect();
    paths.sort();
    Ok(paths.iter().filter_map(|path| gdk::Texture::from_filename(path).ok()).collect())
}

/// Peak level per bucket over the whole file, scaled to a 50 dB range (0…1).
pub async fn waveform(source: &Path, stream_index: usize, duration: f64) -> Result<Vec<f32>> {
    if duration <= 0.0 {
        return Ok(Vec::new());
    }
    let source = source.to_string_lossy().into_owned();
    gio::spawn_blocking(move || {
        let map = format!("0:a:{stream_index}");
        let rate = WAVEFORM_RATE.to_string();
        let data = ffmpeg::output_blocking(&["-i", &source, "-map", &map, "-ac", "1", "-ar", &rate, "-f", "f32le", "pipe:1"])?;

        let mut peaks = vec![0f32; WAVEFORM_BUCKETS];
        for (frame, bytes) in data.chunks_exact(4).enumerate() {
            let sample = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).abs();
            let time = frame as f64 / WAVEFORM_RATE as f64;
            let bucket = ((time / duration * WAVEFORM_BUCKETS as f64) as usize).min(WAVEFORM_BUCKETS - 1);
            peaks[bucket] = peaks[bucket].max(sample);
        }
        Ok(peaks
            .into_iter()
            .map(|p| if p <= 0.0 { 0.0 } else { (1.0 + 20.0 * p.log10() / 50.0).max(0.0) })
            .collect())
    })
    .await
    .map_err(|_| anyhow!("Reading the waveform crashed"))?
}
