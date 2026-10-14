//! Runs `ffmpeg` and `ffprobe` from the system (every distribution packages them). They merge recordings
//! and do everything the editor needs: export, GIF, thumbnails and waveforms.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Context, Result};
use gtk::gio;
use serde_json::Value;

/// Formats seconds for ffmpeg arguments with a dot decimal separator and no trailing zeros.
pub fn seconds(value: f64) -> String {
    let text = format!("{value:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() || text == "-" { "0".into() } else { text.into() }
}

/// Runs ffmpeg off the UI thread. When `progress` is given, it receives the fraction (0…1) of
/// `expected_duration` written so far. Fails with the tail of ffmpeg's output (also logged).
pub async fn run(args: Vec<String>, expected_duration: f64, progress: Option<async_channel::Sender<f64>>) -> Result<()> {
    gio::spawn_blocking(move || run_blocking(&args, expected_duration, progress.as_ref()))
        .await
        .map_err(|_| anyhow!("ffmpeg crashed"))?
}

pub fn run_blocking(arguments: &[String], expected_duration: f64, progress: Option<&async_channel::Sender<f64>>) -> Result<()> {
    let mut args: Vec<String> = ["-hide_banner", "-nostdin", "-y"].map(String::from).to_vec();
    if progress.is_some() {
        args.extend(["-progress", "pipe:1", "-nostats"].map(String::from));
    }
    args.extend(arguments.iter().cloned());
    crate::log::write(format!("ffmpeg {}", args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")));

    let mut child = Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Couldn't start ffmpeg. Is it installed?")?;

    // stderr is drained on its own thread so a chatty ffmpeg can't block on a full pipe.
    let stderr = child.stderr.take().expect("piped stderr");
    let error_tail = std::thread::spawn(move || {
        let mut tail = VecDeque::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            tail.push_back(line);
            if tail.len() > 30 {
                tail.pop_front();
            }
        }
        tail.into_iter().collect::<Vec<_>>().join("\n")
    });

    let stdout = child.stdout.take().expect("piped stdout");
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        // -progress writes key=value lines; out_time_us is the position in the output.
        if let (Some(progress), Some(value)) = (progress, line.strip_prefix("out_time_us=")) {
            if let (Ok(microseconds), true) = (value.trim().parse::<f64>(), expected_duration > 0.0) {
                let _ = progress.send_blocking((microseconds / 1_000_000.0 / expected_duration).clamp(0.0, 1.0));
            }
        }
    }

    let status = child.wait()?;
    let tail = error_tail.join().unwrap_or_default();
    if !status.success() {
        crate::log::write(format!("ffmpeg failed ({status}):\n{tail}"));
        bail!("FFmpeg failed ({status}). Details are in the Cappa log.");
    }
    if let Some(progress) = progress {
        let _ = progress.send_blocking(1.0);
    }
    Ok(())
}

/// Runs ffmpeg and returns everything it writes to stdout (for `-f f32le pipe:1` and the like).
pub fn output_blocking(arguments: &[&str]) -> Result<Vec<u8>> {
    let mut child = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostdin", "-v", "error"])
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Couldn't start ffmpeg. Is it installed?")?;
    let mut stderr = child.stderr.take().expect("piped stderr");
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let mut data = Vec::new();
    child.stdout.take().expect("piped stdout").read_to_end(&mut data)?;
    let status = child.wait()?;
    if !status.success() {
        let errors = errors.join().unwrap_or_default();
        crate::log::write(format!("ffmpeg failed ({status}): {errors}"));
        bail!("FFmpeg failed ({status}).");
    }
    Ok(data)
}

/// The H.264 encoder arguments for exports: libx264 where available, otherwise OpenH264 (Fedora's
/// ffmpeg-free ships without libx264).
pub fn h264_encoder_args() -> Vec<String> {
    static HAS_X264: OnceLock<bool> = OnceLock::new();
    let has_x264 = *HAS_X264.get_or_init(|| {
        Command::new("ffmpeg")
            .args(["-hide_banner", "-encoders"])
            .stderr(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(" libx264 "))
            .unwrap_or(false)
    });
    let args: &[&str] = if has_x264 {
        &["-c:v", "libx264", "-preset", "veryfast", "-crf", "20", "-pix_fmt", "yuv420p"]
    } else {
        &["-c:v", "libopenh264", "-b:v", "12M", "-pix_fmt", "yuv420p"]
    };
    args.iter().map(|s| s.to_string()).collect()
}

#[derive(Clone, Debug)]
pub struct AudioStreamInfo {
    /// The position among the file's audio streams (for `0:a:N`).
    pub index: usize,
    pub channels: u32,
    pub title: Option<String>,
}

#[derive(Clone, Debug)]
pub struct MediaInfo {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub frame_rate: f64,
    pub audio_streams: Vec<AudioStreamInfo>,
}

/// Stream and format information for a media file.
pub async fn probe(path: &Path) -> Result<MediaInfo> {
    let path = path.to_owned();
    gio::spawn_blocking(move || {
        let output = Command::new("ffprobe")
            .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
            .arg(&path)
            .stdin(Stdio::null())
            .output()
            .context("Couldn't start ffprobe. Is FFmpeg installed?")?;
        if !output.status.success() {
            bail!("ffprobe couldn't read {}.", path.file_name().unwrap_or_default().to_string_lossy());
        }
        parse_probe(&serde_json::from_slice(&output.stdout)?)
    })
    .await
    .map_err(|_| anyhow!("ffprobe crashed"))?
}

fn parse_probe(root: &Value) -> Result<MediaInfo> {
    let duration = root["format"]["duration"].as_str().and_then(|d| d.parse().ok()).unwrap_or(0.0);
    let mut info = MediaInfo { duration, width: 0, height: 0, frame_rate: 30.0, audio_streams: Vec::new() };
    let streams = root["streams"].as_array().ok_or_else(|| anyhow!("No streams in this file."))?;
    for stream in streams {
        match stream["codec_type"].as_str() {
            Some("video") if info.width == 0 => {
                info.width = stream["width"].as_u64().unwrap_or(0) as u32;
                info.height = stream["height"].as_u64().unwrap_or(0) as u32;
                if let Some(rate) = stream["avg_frame_rate"].as_str().and_then(parse_rate) {
                    info.frame_rate = rate;
                }
            }
            Some("audio") => {
                let tags = &stream["tags"];
                // MP4 reports a track title as "handler_name" or "name"; other containers as "title".
                let title = ["title", "name", "handler_name"]
                    .iter()
                    .find_map(|key| tags[*key].as_str())
                    .filter(|t| !t.is_empty() && !t.ends_with("Handler") && *t != "SoundHandler")
                    .map(str::to_owned);
                info.audio_streams.push(AudioStreamInfo {
                    index: info.audio_streams.len(),
                    channels: stream["channels"].as_u64().unwrap_or(2) as u32,
                    title,
                });
            }
            _ => {}
        }
    }
    if info.width == 0 {
        bail!("This file has no video.");
    }
    Ok(info)
}

fn parse_rate(rate: &str) -> Option<f64> {
    let (numerator, denominator) = rate.split_once('/')?;
    let (numerator, denominator) = (numerator.parse::<f64>().ok()?, denominator.parse::<f64>().ok()?);
    (numerator > 0.0 && denominator > 0.0).then(|| numerator / denominator)
}

fn quote(argument: &str) -> String {
    if argument.contains([' ', ';']) { format!("\"{argument}\"") } else { argument.to_owned() }
}
