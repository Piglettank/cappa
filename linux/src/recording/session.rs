//! One recording from start to stop. Each stretch between pauses is recorded as its own part; on stop,
//! FFmpeg joins the parts and writes the final MP4 with each audio source on its own titled AAC track,
//! so the editor can adjust them separately. Pausing therefore leaves no gap.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{Context, Result};

use super::click_tracker::ClickTracker;
use super::pipeline::{PartConfig, RecordingPart, VideoInput};
use super::portal::PortalCapture;
use super::{CaptureTarget, Rect};
use crate::settings::Settings;

/// The video source, kept open across pauses.
pub enum Source {
    X11,
    Portal(PortalCapture),
}

pub struct Session {
    target: CaptureTarget,
    settings: Settings,
    source: Source,
    clicks: Option<Arc<ClickTracker>>,
    work_dir: PathBuf,
    parts: Vec<PathBuf>,
    current: Option<RecordingPart>,
    on_ended: Rc<dyn Fn(Option<String>)>,
}

impl Session {
    pub async fn start(
        target: CaptureTarget,
        settings: Settings,
        source: Source,
        on_ended: Rc<dyn Fn(Option<String>)>,
    ) -> Result<Self> {
        crate::log::write(format!(
            "Starting recording of {} at {:?}, area {:?}, system audio {}, mic {}, quality {:?}, {} fps, cursor {}, click highlight {}",
            target.monitor.connector().unwrap_or_default(),
            target.monitor_geometry,
            target.region,
            settings.record_system_audio,
            settings.record_microphone,
            settings.quality,
            settings.frame_rate,
            settings.show_cursor,
            settings.highlight_clicks,
        ));
        // Click highlights are drawn by the pipeline from X11 pointer polling.
        let clicks = match (&source, settings.show_cursor && settings.highlight_clicks) {
            (Source::X11, true) => ClickTracker::start()
                .map_err(|e| crate::log::error("Starting click highlights", e))
                .ok(),
            _ => None,
        };
        let mut session = Self {
            target,
            settings,
            source,
            clicks,
            work_dir: crate::paths::temp_dir(&crate::paths::unique_name("recording")),
            parts: Vec::new(),
            current: None,
            on_ended,
        };
        if let Err(e) = session.start_part().await {
            session.discard().await;
            return Err(e);
        }
        Ok(session)
    }

    pub async fn pause(&mut self) {
        if let Some(part) = self.current.take() {
            part.finish().await;
        }
    }

    pub async fn resume(&mut self) -> Result<()> {
        if self.current.is_none() {
            self.start_part().await?;
        }
        Ok(())
    }

    /// Stops capture and writes the finished recording to `output`.
    pub async fn stop(mut self, output: &Path) -> Result<()> {
        if let Some(part) = self.current.take() {
            part.finish().await;
        }
        self.close_source().await;
        let result = self.write_output(output).await;
        self.remove_work_dir();
        result
    }

    /// Stops and throws everything away.
    pub async fn discard(mut self) {
        if let Some(part) = self.current.take() {
            part.finish().await;
        }
        self.close_source().await;
        self.remove_work_dir();
    }

    async fn start_part(&mut self) -> Result<()> {
        let path = self.work_dir.join(format!("part{}.mkv", self.parts.len()));
        let config = self.part_config()?;
        let on_ended = self.on_ended.clone();
        let part = RecordingPart::start(config, &path, move |error| on_ended(error)).await?;
        self.parts.push(path);
        self.current = Some(part);
        Ok(())
    }

    fn part_config(&self) -> Result<PartConfig> {
        let scale = self.target.scale_factor;
        let area = self.target.local_area();
        let video = match &self.source {
            // X11 works in physical pixels of the whole desktop.
            Source::X11 => VideoInput::X11 {
                area: even(Rect {
                    x: (self.target.monitor_geometry.x + area.x) * scale,
                    y: (self.target.monitor_geometry.y + area.y) * scale,
                    width: area.width * scale,
                    height: area.height * scale,
                }),
            },
            // Each part needs its own copy of the PipeWire connection; the element takes ownership.
            Source::Portal(capture) => VideoInput::PipeWire {
                fd: capture.fd.try_clone().context("Couldn't reuse the PipeWire connection")?,
                node_id: capture.node_id,
                crop: area,
                logical_size: capture.logical_size,
            },
        };
        let clicks = self.clicks.clone().map(|tracker| {
            let origin = match &video {
                VideoInput::X11 { area } => (area.x, area.y),
                VideoInput::PipeWire { .. } => (0, 0),
            };
            (tracker, origin, scale as f64)
        });
        let settings = &self.settings;
        Ok(PartConfig {
            video,
            frame_rate: settings.frame_rate,
            quality: settings.quality,
            show_cursor: settings.show_cursor,
            pixel_size: ((area.width * scale) as u32, (area.height * scale) as u32),
            system_audio: settings.record_system_audio,
            microphone: settings.record_microphone.then(|| settings.microphone_id.clone()),
            clicks,
        })
    }

    async fn close_source(&mut self) {
        self.clicks = None;
        if let Source::Portal(capture) = std::mem::replace(&mut self.source, Source::X11) {
            capture.close().await;
        }
    }

    async fn write_output(&self, output: &Path) -> Result<()> {
        let parts: Vec<&PathBuf> = self.parts.iter().filter(|p| p.metadata().is_ok_and(|m| m.len() > 0)).collect();
        if parts.is_empty() {
            anyhow::bail!("Nothing was recorded.");
        }
        // FFmpeg's concat demuxer joins the parts back to back.
        let list = self.work_dir.join("parts.txt");
        let entries: String = parts
            .iter()
            .map(|p| format!("file '{}'\n", p.to_string_lossy().replace('\'', "'\\''")))
            .collect();
        std::fs::write(&list, entries)?;

        let mut tracks: Vec<(&str, u32, &str)> = Vec::new();
        if self.settings.record_system_audio {
            tracks.push(("Computer Audio", 2, "160k"));
        }
        if self.settings.record_microphone {
            tracks.push(("Microphone", 1, "96k"));
        }

        let mut args: Vec<String> = vec!["-f".into(), "concat".into(), "-safe".into(), "0".into(), "-i".into(), list.to_string_lossy().into()];
        args.extend(["-map", "0:v:0"].map(String::from));
        for i in 0..tracks.len() {
            args.extend(["-map".into(), format!("0:a:{i}")]);
        }
        args.extend(["-c:v", "copy"].map(String::from));
        if !tracks.is_empty() {
            args.extend(["-c:a", "aac", "-ar", "48000"].map(String::from));
        }
        for (i, (title, channels, bitrate)) in tracks.iter().enumerate() {
            args.extend([
                format!("-ac:a:{i}"),
                channels.to_string(),
                format!("-b:a:{i}"),
                bitrate.to_string(),
                format!("-metadata:s:a:{i}"),
                format!("title={title}"),
                format!("-metadata:s:a:{i}"),
                // Shown as the track name by most players.
                format!("handler_name={title}"),
            ]);
        }
        args.extend(["-movflags".into(), "+faststart".into(), output.to_string_lossy().into()]);
        crate::ffmpeg::run(args, 0.0, None).await
    }

    fn remove_work_dir(&self) {
        if let Err(e) = std::fs::remove_dir_all(&self.work_dir) {
            crate::log::error("Removing the recording work folder", e);
        }
    }
}

/// H.264 needs even dimensions.
fn even(rect: Rect) -> Rect {
    Rect { width: (rect.width & !1).max(2), height: (rect.height & !1).max(2), ..rect }
}
