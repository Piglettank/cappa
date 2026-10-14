//! One recorded stretch of a recording (from start or resume until pause or stop) as a GStreamer pipeline:
//! screen → H.264, plus raw PCM for system audio and the microphone, all in one Matroska file. Matroska
//! stays readable even if the pipeline dies before finishing, and the parts are joined and converted to
//! the final MP4 by FFmpeg when the recording stops.

use std::cell::Cell;
use std::os::fd::{IntoRawFd, OwnedFd};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use gst::prelude::*;
use gtk::glib;

use super::click_tracker::ClickTracker;
use super::Rect;
use crate::settings::RecordingQuality;

/// Where the video comes from.
pub enum VideoInput {
    /// `ximagesrc`: an area of the root window in physical pixels.
    X11 { area: Rect },
    /// `pipewiresrc` on a portal stream, cropped to `crop` (logical pixels within a stream whose logical
    /// size is `logical_size`; the stream's real pixel size is only known once it starts).
    PipeWire { fd: OwnedFd, node_id: u32, crop: Rect, logical_size: (i32, i32) },
}

pub struct PartConfig {
    pub video: VideoInput,
    pub frame_rate: u32,
    pub quality: RecordingQuality,
    pub show_cursor: bool,
    /// Size estimate in physical pixels, for encoders that need a bitrate.
    pub pixel_size: (u32, u32),
    pub system_audio: bool,
    /// `Some("")` records the default microphone.
    pub microphone: Option<String>,
    /// X11 only: draws click highlights with the frame's top-left corner at this root-window position.
    pub clicks: Option<(Arc<ClickTracker>, (i32, i32), f64)>,
}

/// A running part. `finish` ends it cleanly.
pub struct RecordingPart {
    pipeline: gst::Pipeline,
    _bus_watch: gst::bus::BusWatchGuard,
    events: async_channel::Receiver<PartEvent>,
    /// Set when the pipeline failed; then there's nothing left to finish.
    failed: Rc<Cell<bool>>,
}

/// Posted by the video source's end-of-stream probe.
const SOURCE_ENDED: &str = "cappa-source-ended";

enum PartEvent {
    Playing,
    Eos,
    Error(String),
}

impl RecordingPart {
    /// Builds the pipeline and waits until it's capturing. Once running, `on_ended` is called (on the UI
    /// thread) with the error if the pipeline fails, or `None` if the screen stops being shared (from the
    /// desktop's own screen-sharing indicator, for example).
    pub async fn start(config: PartConfig, path: &Path, on_ended: impl Fn(Option<String>) + 'static) -> Result<Self> {
        let pipeline = build(config, path)?;
        let (sender, events) = async_channel::unbounded();
        let started = Rc::new(Cell::new(false));
        let failed = Rc::new(Cell::new(false));
        let reported = Cell::new(false);
        let (watch_started, watch_failed) = (started.clone(), failed.clone());
        let weak_pipeline = pipeline.downgrade();

        let bus = pipeline.bus().context("The pipeline has no bus")?;
        let bus_watch = bus.add_watch_local(move |_, message| {
            use gst::MessageView;
            let event = match message.view() {
                MessageView::Error(error) => {
                    let source = message.src().map(|s| s.path_string().to_string()).unwrap_or_default();
                    crate::log::write(format!("Recording pipeline error from {source}: {} ({:?})", error.error(), error.debug()));
                    Some(PartEvent::Error(error.error().to_string()))
                }
                MessageView::Warning(warning) => {
                    crate::log::write(format!("Recording pipeline warning: {} ({:?})", warning.error(), warning.debug()));
                    None
                }
                MessageView::Eos(_) => Some(PartEvent::Eos),
                MessageView::StateChanged(change)
                    if change.current() == gst::State::Playing
                        && weak_pipeline
                            .upgrade()
                            .is_some_and(|p| message.src() == Some(p.upcast_ref::<gst::Object>())) =>
                {
                    Some(PartEvent::Playing)
                }
                MessageView::Application(application)
                    if application.structure().is_some_and(|s| s.name() == SOURCE_ENDED) =>
                {
                    crate::log::write("The screen stopped being shared");
                    if watch_started.get() && !reported.replace(true) {
                        on_ended(None);
                    }
                    None
                }
                _ => None,
            };
            if let Some(event) = event {
                // Once running, failures are reported through the callback; before, `start` sees them.
                if let PartEvent::Error(message) = &event {
                    watch_failed.set(true);
                    if watch_started.get() && !reported.replace(true) {
                        on_ended(Some(message.clone()));
                    }
                }
                let _ = sender.send_blocking(event);
            }
            glib::ControlFlow::Continue
        })?;

        pipeline.set_state(gst::State::Playing).context("Couldn't start the recording pipeline")?;
        let part = Self { pipeline, _bus_watch: bus_watch, events, failed };
        let result = part.wait_for(Duration::from_secs(15), |event| matches!(event, PartEvent::Playing)).await;
        if let Err(e) = result {
            let _ = part.pipeline.set_state(gst::State::Null);
            return Err(e);
        }
        started.set(true);
        Ok(part)
    }

    /// Ends the part: sends end-of-stream so the file is finished properly, then shuts the pipeline down.
    pub async fn finish(self) {
        if self.failed.get() {
            // A failed pipeline won't finish; what it wrote so far stays readable.
            let _ = self.pipeline.set_state(gst::State::Null);
            return;
        }
        self.pipeline.send_event(gst::event::Eos::new());
        if let Err(e) = self.wait_for(Duration::from_secs(10), |event| matches!(event, PartEvent::Eos)).await {
            crate::log::error("Finishing a recording part", e);
        }
        let _ = self.pipeline.set_state(gst::State::Null);
    }

    async fn wait_for(&self, timeout: Duration, wanted: impl Fn(&PartEvent) -> bool) -> Result<()> {
        let deadline = glib::timeout_future(timeout);
        futures_util::pin_mut!(deadline);
        loop {
            let next = self.events.recv();
            futures_util::pin_mut!(next);
            match futures_util::future::select(next, &mut deadline).await {
                futures_util::future::Either::Left((Ok(event), _)) => match event {
                    PartEvent::Error(message) => bail!("{message}"),
                    event if wanted(&event) => return Ok(()),
                    _ => {}
                },
                futures_util::future::Either::Left((Err(_), _)) => bail!("The recording pipeline stopped."),
                futures_util::future::Either::Right(_) => bail!("The recording pipeline didn't respond in time."),
            }
        }
    }
}

fn build(config: PartConfig, path: &Path) -> Result<gst::Pipeline> {
    let pipeline = gst::Pipeline::with_name("cappa-recording");
    let mux = make("matroskamux")?;
    let sink = make("filesink")?;
    sink.set_property("location", path.to_string_lossy().as_ref());
    pipeline.add_many([&mux, &sink])?;
    mux.link(&sink)?;

    add_video(&pipeline, &mux, &config)?;
    if config.system_audio {
        // PulseAudio's special name for the default output's monitor: everything that plays.
        add_audio(&pipeline, &mux, "@DEFAULT_MONITOR@", 2)?;
    }
    if let Some(microphone) = &config.microphone {
        add_audio(&pipeline, &mux, microphone, 1)?;
    }
    Ok(pipeline)
}

fn add_video(pipeline: &gst::Pipeline, mux: &gst::Element, config: &PartConfig) -> Result<()> {
    let rate = gst::Fraction::new(config.frame_rate as i32, 1);
    let mut chain: Vec<gst::Element> = Vec::new();

    match &config.video {
        &VideoInput::X11 { area } => {
            let source = make("ximagesrc")?;
            source.set_property("use-damage", false);
            source.set_property("show-pointer", config.show_cursor);
            source.set_property("startx", area.x.max(0) as u32);
            source.set_property("starty", area.y.max(0) as u32);
            // The end coordinates are inclusive.
            source.set_property("endx", (area.right() - 1).max(0) as u32);
            source.set_property("endy", (area.bottom() - 1).max(0) as u32);
            chain.push(source);
            chain.push(caps_filter(gst::Caps::builder("video/x-raw").field("framerate", rate).build())?);
            chain.push(make("queue")?);
            chain.push(make("videoconvert")?);
        }
        &VideoInput::PipeWire { ref fd, node_id, crop, logical_size } => {
            let source = make("pipewiresrc")?;
            // The element owns its own copy of the connection; ours closes with the config.
            source.set_property("fd", fd.try_clone().context("Couldn't reuse the PipeWire connection")?.into_raw_fd());
            source.set_property("path", node_id.to_string());
            source.set_property("do-timestamp", true);
            set_if_exists(&source, "always-copy", "true");
            // Repeats the last frame while nothing changes on screen, so the video has no gaps.
            set_if_exists(&source, "keepalive-time", "500");
            report_end_of_stream(&source);
            chain.push(source);
            chain.push(make("queue")?);
            chain.push(make("videoconvert")?);
            let cropper = make("videocrop")?;
            crop_when_size_known(&cropper, crop, logical_size);
            chain.push(cropper);
            chain.push(make("videorate")?);
            chain.push(caps_filter(gst::Caps::builder("video/x-raw").field("framerate", rate).build())?);
        }
    }

    if let Some((tracker, origin, scale)) = &config.clicks {
        let overlay = make("cairooverlay")?;
        let (tracker, origin, scale) = (tracker.clone(), *origin, *scale);
        overlay.connect("draw", false, move |values| {
            if let Ok(cr) = values[1].get::<gtk::cairo::Context>() {
                tracker.draw(&cr, origin, scale);
            }
            None
        });
        chain.push(overlay);
        chain.push(make("videoconvert")?);
    }

    chain.push(caps_filter(gst::Caps::builder("video/x-raw").field("format", "I420").build())?);
    chain.push(encoder(config)?);
    chain.push(make("h264parse")?);
    chain.push(make("queue")?);

    pipeline.add_many(&chain)?;
    gst::Element::link_many(&chain)?;
    chain.last().unwrap().link(mux)?;
    Ok(())
}

/// When the screen stops being shared, only the video source ends, so the pipeline as a whole never
/// reaches end-of-stream. Tell the UI thread, which then finishes the recording.
fn report_end_of_stream(source: &gst::Element) {
    let weak = source.downgrade();
    let pad = source.static_pad("src").expect("pipewiresrc has a src pad");
    pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
        if let Some(gst::PadProbeData::Event(event)) = &info.data {
            if event.type_() == gst::EventType::Eos {
                if let Some(source) = weak.upgrade() {
                    let _ = source.post_message(gst::message::Application::new(gst::Structure::new_empty(SOURCE_ENDED)));
                }
            }
        }
        gst::PadProbeReturn::Ok
    });
}

/// The portal's stream may be in physical pixels (scaled monitors), which is only known once it starts.
/// So the crop is set when the first caps arrive, scaled from logical to real pixels and rounded to
/// even sizes (H.264 needs them).
fn crop_when_size_known(cropper: &gst::Element, crop: Rect, logical_size: (i32, i32)) {
    let weak = cropper.downgrade();
    let pad = cropper.static_pad("sink").expect("videocrop has a sink pad");
    pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
        let Some(gst::PadProbeData::Event(event)) = &info.data else { return gst::PadProbeReturn::Ok };
        let gst::EventView::Caps(caps) = event.view() else { return gst::PadProbeReturn::Ok };
        let Some(structure) = caps.caps().structure(0) else { return gst::PadProbeReturn::Ok };
        let (Ok(width), Ok(height), Some(cropper)) =
            (structure.get::<i32>("width"), structure.get::<i32>("height"), weak.upgrade())
        else {
            return gst::PadProbeReturn::Ok;
        };
        let scale_x = width as f64 / logical_size.0.max(1) as f64;
        let scale_y = height as f64 / logical_size.1.max(1) as f64;
        let left = ((crop.x as f64 * scale_x).round() as i32).clamp(0, width - 2);
        let top = ((crop.y as f64 * scale_y).round() as i32).clamp(0, height - 2);
        let crop_width = ((crop.width as f64 * scale_x).round() as i32).min(width - left) & !1;
        let crop_height = ((crop.height as f64 * scale_y).round() as i32).min(height - top) & !1;
        let (right, bottom) = (width - left - crop_width.max(2), height - top - crop_height.max(2));
        crate::log::write(format!("Stream {width}×{height}; cropping left {left} top {top} right {right} bottom {bottom}"));
        cropper.set_property("left", left);
        cropper.set_property("top", top);
        cropper.set_property("right", right);
        cropper.set_property("bottom", bottom);
        gst::PadProbeReturn::Ok
    });
}

/// The first available H.264 encoder: x264 (best quality per bit), VA-API hardware, then OpenH264
/// (what Fedora ships without RPM Fusion).
fn encoder(config: &PartConfig) -> Result<gst::Element> {
    let (width, height) = config.pixel_size;
    let bit_rate = config.quality.bit_rate(width, height, config.frame_rate);
    let keyframes = (config.frame_rate * 2).to_string();

    if let Ok(x264) = make("x264enc") {
        set_if_exists(&x264, "speed-preset", "veryfast");
        set_if_exists(&x264, "pass", "qual");
        set_if_exists(&x264, "quantizer", &config.quality.x264_quantizer().to_string());
        set_if_exists(&x264, "key-int-max", &keyframes);
        return Ok(x264);
    }
    if let Ok(va) = make("vah264enc") {
        set_if_exists(&va, "rate-control", "vbr");
        set_if_exists(&va, "bitrate", &(bit_rate / 1000).to_string()); // kbit/s
        set_if_exists(&va, "key-int-max", &keyframes);
        return Ok(va);
    }
    if let Ok(openh264) = make("openh264enc") {
        set_if_exists(&openh264, "rate-control", "bitrate");
        set_if_exists(&openh264, "bitrate", &bit_rate.min(u32::MAX as u64).to_string());
        set_if_exists(&openh264, "gop-size", &keyframes);
        return Ok(openh264);
    }
    bail!(
        "No H.264 encoder found. Install the GStreamer x264 plugin (gstreamer1.0-plugins-ugly) \
         or OpenH264 (gstreamer1-plugin-openh264)."
    )
}

fn add_audio(pipeline: &gst::Pipeline, mux: &gst::Element, device: &str, channels: i32) -> Result<()> {
    let source = make("pulsesrc")?;
    if !device.is_empty() {
        source.set_property("device", device);
    }
    let caps = gst::Caps::builder("audio/x-raw")
        .field("format", "S16LE")
        .field("rate", 48_000i32)
        .field("channels", channels)
        .build();
    let chain = [source, make("queue")?, make("audioconvert")?, make("audioresample")?, caps_filter(caps)?, make("queue")?];
    pipeline.add_many(&chain)?;
    gst::Element::link_many(&chain)?;
    chain.last().unwrap().link(mux)?;
    Ok(())
}

fn make(factory: &str) -> Result<gst::Element> {
    gst::ElementFactory::make(factory).build().map_err(|_| anyhow!(missing_plugin_message(factory)))
}

fn caps_filter(caps: gst::Caps) -> Result<gst::Element> {
    let filter = make("capsfilter")?;
    filter.set_property("caps", caps);
    Ok(filter)
}

/// Sets a property from its string form, if the element has it (they differ between plugin versions).
fn set_if_exists(element: &gst::Element, name: &str, value: &str) {
    if element.find_property(name).is_some() {
        element.set_property_from_str(name, value);
    }
}

fn missing_plugin_message(factory: &str) -> String {
    let package = match factory {
        "ximagesrc" | "pulsesrc" | "matroskamux" | "videocrop" | "cairooverlay" => "gstreamer1.0-plugins-good",
        "pipewiresrc" => "gstreamer1.0-pipewire",
        "h264parse" => "gstreamer1.0-plugins-bad",
        _ => "gstreamer1.0-plugins-base",
    };
    format!("The GStreamer element \"{factory}\" is missing. Install {package}.")
}
