//! Plays the edit with one GStreamer pipeline: the video into a GTK paintable, and every audio track through
//! its own volume element into a mixer, so each lane's level is heard immediately. Cut-away parts are skipped.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::{Rc, Weak};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use gst::prelude::*;
use gtk::{gdk, glib};

use super::model::EditorModel;

/// Right after a jump the reported position can still be the old one for a moment.
const JUMP_SETTLE: Duration = Duration::from_millis(500);

pub struct Playback {
    pipeline: gst::Pipeline,
    model: Rc<EditorModel>,
    paintable: gdk::Paintable,
    /// One volume element per audio track, in stream order (filled in as decodebin exposes the streams).
    volumes: Arc<Mutex<Vec<gst::Element>>>,
    playing: Cell<bool>,
    jump: Cell<Option<(f64, Instant)>>,
    timer: RefCell<Option<glib::SourceId>>,
    bus_watch: RefCell<Option<gst::bus::BusWatchGuard>>,
    listeners: RefCell<Vec<Box<dyn Fn()>>>,
}

impl Playback {
    pub fn new(model: Rc<EditorModel>, path: &Path) -> Result<Rc<Self>> {
        let pipeline = gst::Pipeline::new();
        let source = gst::ElementFactory::make("filesrc").property("location", &*path.to_string_lossy()).build()?;
        let decode = gst::ElementFactory::make("decodebin").build()?;

        let video_queue = gst::ElementFactory::make("queue").build()?;
        let video_convert = gst::ElementFactory::make("videoconvert").build()?;
        let video_sink = gst::ElementFactory::make("gtk4paintablesink")
            .build()
            .context("The GTK video sink is missing")?;
        let paintable = video_sink.property::<gdk::Paintable>("paintable");
        pipeline.add_many([&source, &decode, &video_queue, &video_convert, &video_sink])?;
        source.link(&decode)?;
        gst::Element::link_many([&video_queue, &video_convert, &video_sink])?;

        // Without audio there's no mixer: an audio sink that never receives data would never preroll.
        let mixer = if model.audio_track_count() > 0 {
            let mixer = gst::ElementFactory::make("audiomixer").build()?;
            let convert = gst::ElementFactory::make("audioconvert").build()?;
            let sink = gst::ElementFactory::make("autoaudiosink").build()?;
            pipeline.add_many([&mixer, &convert, &sink])?;
            gst::Element::link_many([&mixer, &convert, &sink])?;
            Some(mixer)
        } else {
            None
        };

        let volumes: Arc<Mutex<Vec<gst::Element>>> = Arc::default();
        let initial_volumes: Vec<f64> = model.audio_tracks().iter().map(|t| t.effective_volume() as f64).collect();
        let video_target = video_queue.static_pad("sink").expect("queue has a sink pad");
        let pipeline_weak = pipeline.downgrade();
        let volumes_for_pads = volumes.clone();
        // Runs on a streaming thread: only GStreamer work here.
        decode.connect_pad_added(move |_, pad| {
            let Some(pipeline) = pipeline_weak.upgrade() else { return };
            let Some(caps) = pad.current_caps() else { return };
            let Some(structure) = caps.structure(0) else { return };
            let name = structure.name();
            if name.starts_with("video/") {
                if !video_target.is_linked() {
                    if let Err(e) = pad.link(&video_target) {
                        crate::log::error("Linking the editor's video", format!("{e:?}"));
                    }
                }
            } else if name.starts_with("audio/") {
                if let Some(mixer) = &mixer {
                    let mut volumes = volumes_for_pads.lock().unwrap();
                    let level = initial_volumes.get(volumes.len()).copied().unwrap_or(1.0);
                    match link_audio(&pipeline, pad, mixer, level) {
                        Ok(volume) => volumes.push(volume),
                        Err(e) => crate::log::error("Linking an editor audio track", e),
                    }
                }
            }
        });

        let playback = Rc::new(Self {
            pipeline,
            model: model.clone(),
            paintable,
            volumes,
            playing: Cell::new(false),
            jump: Cell::new(None),
            timer: RefCell::default(),
            bus_watch: RefCell::default(),
            listeners: RefCell::default(),
        });

        let weak = Rc::downgrade(&playback);
        let watch = playback.pipeline.bus().expect("pipelines have a bus").add_watch_local(move |_, message| {
            if let Some(playback) = weak.upgrade() {
                match message.view() {
                    gst::MessageView::Eos(_) => playback.pause(),
                    gst::MessageView::Error(error) => {
                        crate::log::error("Editor playback", format!("{} ({:?})", error.error(), error.debug()));
                        playback.pause();
                    }
                    _ => {}
                }
            }
            glib::ControlFlow::Continue
        })?;
        *playback.bus_watch.borrow_mut() = Some(watch);

        let weak: Weak<Self> = Rc::downgrade(&playback);
        model.connect_changed(move || {
            if let Some(playback) = weak.upgrade() {
                playback.update_volumes();
            }
        });

        // Paused shows the first frame.
        playback.pipeline.set_state(gst::State::Paused)?;
        Ok(playback)
    }

    pub fn paintable(&self) -> &gdk::Paintable {
        &self.paintable
    }

    pub fn is_playing(&self) -> bool {
        self.playing.get()
    }

    /// Called when the playhead moves or playback starts/stops.
    pub fn connect_updated(&self, listener: impl Fn() + 'static) {
        self.listeners.borrow_mut().push(Box::new(listener));
    }

    pub fn toggle_play(self: &Rc<Self>) {
        if self.is_playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn play(self: &Rc<Self>) {
        let segments = self.model.segments();
        let Some(first) = segments.first() else { return };
        // Starting in a cut part (or at the very end) jumps to the next kept part, wrapping to the start.
        let time = self.model.current_time();
        if self.model.segment_at(time).is_none() {
            let next = self.model.next_segment_after(time).unwrap_or(*first);
            self.seek(next.start);
        }
        self.playing.set(true);
        let _ = self.pipeline.set_state(gst::State::Playing);

        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local(Duration::from_millis(15), move || match weak.upgrade() {
            Some(playback) => {
                playback.tick();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        if let Some(previous) = self.timer.replace(Some(timer)) {
            previous.remove();
        }
        self.notify();
    }

    pub fn pause(&self) {
        if !self.playing.replace(false) {
            return;
        }
        let _ = self.pipeline.set_state(gst::State::Paused);
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
        if let Some(time) = self.position() {
            self.model.set_current_time(time);
        }
        self.jump.set(None);
        self.notify();
    }

    pub fn seek(&self, time: f64) {
        let time = time.clamp(0.0, self.model.duration());
        self.model.set_current_time(time);
        self.seek_pipeline(time);
        self.notify();
    }

    /// Stops playback for good; the window is closing.
    pub fn shutdown(&self) {
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
        self.bus_watch.take();
        let _ = self.pipeline.set_state(gst::State::Null);
    }

    fn tick(&self) {
        if !self.playing.get() {
            return;
        }
        let Some(time) = self.position() else { return };

        // Ignore stale positions until a jump lands (or a short timeout), otherwise it would repeat forever.
        if let Some((target, started)) = self.jump.get() {
            if time < target - 0.05 && started.elapsed() < JUMP_SETTLE {
                return;
            }
            self.jump.set(None);
        }

        self.model.set_current_time(time);
        if self.model.segment_at(time).is_none() {
            if let Some(next) = self.model.next_segment_after(time) {
                self.jump.set(Some((next.start, Instant::now())));
                self.model.set_current_time(next.start);
                self.seek_pipeline(next.start);
            } else {
                let end = self.model.segments().last().map_or(time, |s| s.end);
                self.pause();
                self.seek(end);
                return;
            }
        }
        self.notify();
    }

    fn position(&self) -> Option<f64> {
        self.pipeline.query_position::<gst::ClockTime>().map(|t| t.seconds_f64())
    }

    fn seek_pipeline(&self, time: f64) {
        let flags = gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE;
        // Fails until the pipeline has prerolled; the next seek or play catches up.
        let _ = self.pipeline.seek_simple(flags, gst::ClockTime::from_seconds_f64(time));
    }

    fn update_volumes(&self) {
        let tracks = self.model.audio_tracks();
        for (volume, track) in self.volumes.lock().unwrap().iter().zip(&tracks) {
            volume.set_property("volume", track.effective_volume() as f64);
        }
    }

    fn notify(&self) {
        for listener in self.listeners.borrow().iter() {
            listener();
        }
    }
}

/// decodebin pad → queue → audioconvert → audioresample → volume → audiomixer.
fn link_audio(pipeline: &gst::Pipeline, pad: &gst::Pad, mixer: &gst::Element, level: f64) -> Result<gst::Element> {
    let queue = gst::ElementFactory::make("queue").build()?;
    let convert = gst::ElementFactory::make("audioconvert").build()?;
    let resample = gst::ElementFactory::make("audioresample").build()?;
    let volume = gst::ElementFactory::make("volume").property("volume", level).build()?;
    let elements = [&queue, &convert, &resample, &volume];
    pipeline.add_many(elements)?;
    gst::Element::link_many(elements)?;
    let mixer_pad = mixer.request_pad_simple("sink_%u").context("audiomixer gave no input")?;
    volume.static_pad("src").expect("volume has a src pad").link(&mixer_pad)?;
    for element in elements {
        element.sync_state_with_parent()?;
    }
    pad.link(&queue.static_pad("sink").expect("queue has a sink pad"))?;
    Ok(volume)
}
