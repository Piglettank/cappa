//! Editing state for one video. The edit is a list of kept segments in source order; everything between
//! them is cut. The source file is never modified, and every export writes a new file.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use gtk::gdk;

use crate::ffmpeg::MediaInfo;

pub const MINIMUM_SEGMENT_DURATION: f64 = 0.1;
/// How far before a clip's start a reported playback time still counts as inside it.
const BOUNDARY_TOLERANCE: f64 = 0.005;

/// A kept part of the source video, in source-time seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub id: u64,
    pub start: f64,
    pub end: f64,
}

impl Segment {
    pub fn duration(&self) -> f64 {
        self.end - self.start
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrimEdge {
    Start,
    End,
}

/// One audio track of the source. It always follows the video's clips; only its level is separate.
#[derive(Clone, Debug)]
pub struct AudioTrack {
    /// The position among the source's audio streams (for `0:a:N`).
    pub stream_index: usize,
    pub name: String,
    pub is_microphone: bool,
    /// 0…1, set with the lane header's slider.
    pub volume: f32,
    pub muted: bool,
    /// Peak levels (0…1) evenly spaced over the whole source; empty until loaded.
    pub waveform: Vec<f32>,
}

impl AudioTrack {
    pub fn effective_volume(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume }
    }

    pub fn volume_label(&self) -> String {
        if self.muted { "Muted".into() } else { format!("{}%", (self.volume * 100.0).round()) }
    }

    pub fn icon_name(&self) -> &'static str {
        if self.muted {
            "audio-volume-muted-symbolic"
        } else if self.is_microphone {
            "audio-input-microphone-symbolic"
        } else {
            "audio-volume-high-symbolic"
        }
    }
}

#[derive(Default)]
struct State {
    segments: Vec<Segment>,
    undo: Vec<Vec<Segment>>,
    redo: Vec<Vec<Segment>>,
    exported_state: Option<String>,
    audio_tracks: Vec<AudioTrack>,
    thumbnails: Vec<gdk::Texture>,
    selected: Option<u64>,
}

pub struct EditorModel {
    pub source_path: PathBuf,
    info: RefCell<Option<MediaInfo>>,
    duration: Cell<f64>,
    current_time: Cell<f64>,
    next_id: Cell<u64>,
    state: RefCell<State>,
    listeners: RefCell<Vec<Box<dyn Fn()>>>,
}

impl EditorModel {
    pub fn new(source_path: PathBuf) -> Self {
        Self {
            source_path,
            info: RefCell::default(),
            duration: Cell::new(0.0),
            current_time: Cell::new(0.0),
            next_id: Cell::new(1),
            state: RefCell::default(),
            listeners: RefCell::default(),
        }
    }

    pub fn load(&self, info: MediaInfo) {
        self.duration.set(info.duration);
        let first = self.segment(0.0, info.duration);
        {
            let mut state = self.state.borrow_mut();
            state.segments = vec![first];
            let count = info.audio_streams.len();
            state.audio_tracks = info
                .audio_streams
                .iter()
                .map(|stream| {
                    // Cappa titles its tracks; for other videos fall back to channel count (the mic is recorded in mono).
                    let is_microphone = match &stream.title {
                        Some(title) => title.to_lowercase().contains("microphone"),
                        None => stream.channels == 1,
                    };
                    let name = stream.title.clone().unwrap_or_else(|| {
                        if is_microphone {
                            "Microphone".into()
                        } else if count > 1 {
                            "Computer Audio".into()
                        } else {
                            "Audio".into()
                        }
                    });
                    AudioTrack { stream_index: stream.index, name, is_microphone, volume: 1.0, muted: false, waveform: Vec::new() }
                })
                .collect();
        }
        *self.info.borrow_mut() = Some(info);
        self.raise_changed();
    }

    // MARK: - Reading

    pub fn duration(&self) -> f64 {
        self.duration.get()
    }

    pub fn current_time(&self) -> f64 {
        self.current_time.get()
    }

    pub fn set_current_time(&self, time: f64) {
        self.current_time.set(time);
    }

    pub fn segments(&self) -> Vec<Segment> {
        self.state.borrow().segments.clone()
    }

    pub fn audio_tracks(&self) -> Vec<AudioTrack> {
        self.state.borrow().audio_tracks.clone()
    }

    pub fn audio_track_count(&self) -> usize {
        self.state.borrow().audio_tracks.len()
    }

    pub fn thumbnails(&self) -> Vec<gdk::Texture> {
        self.state.borrow().thumbnails.clone()
    }

    pub fn selected_id(&self) -> Option<u64> {
        self.state.borrow().selected
    }

    pub fn edited_duration(&self) -> f64 {
        self.state.borrow().segments.iter().map(Segment::duration).sum()
    }

    pub fn can_undo(&self) -> bool {
        !self.state.borrow().undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.state.borrow().redo.is_empty()
    }

    pub fn can_delete(&self) -> bool {
        let state = self.state.borrow();
        state.segments.len() > 1 && state.selected.is_some_and(|id| state.segments.iter().any(|s| s.id == id))
    }

    pub fn has_edits(&self) -> bool {
        let state = self.state.borrow();
        let untouched = state.segments.len() == 1
            && state.segments[0].start == 0.0
            && state.segments[0].end == self.duration();
        !untouched || state.audio_tracks.iter().any(|t| t.effective_volume() != 1.0)
    }

    pub fn has_unexported_edits(&self) -> bool {
        self.has_edits() && self.state.borrow().exported_state.as_deref() != Some(self.state_key().as_str())
    }

    pub fn mark_exported(&self) {
        let key = self.state_key();
        self.state.borrow_mut().exported_state = Some(key);
    }

    /// The clip at `time`, allowing for playback reporting a time just before a clip's start.
    pub fn segment_at(&self, time: f64) -> Option<Segment> {
        self.state.borrow().segments.iter().copied().find(|s| time >= s.start - BOUNDARY_TOLERANCE && time < s.end)
    }

    /// The first clip starting at or after `time`.
    pub fn next_segment_after(&self, time: f64) -> Option<Segment> {
        self.state.borrow().segments.iter().copied().find(|s| s.start >= time - BOUNDARY_TOLERANCE)
    }

    pub fn can_split_at(&self, time: f64) -> bool {
        self.state
            .borrow()
            .segments
            .iter()
            .find(|s| s.start <= time && time < s.end)
            .is_some_and(|s| time - s.start >= MINIMUM_SEGMENT_DURATION && s.end - time >= MINIMUM_SEGMENT_DURATION)
    }

    // MARK: - Editing

    pub fn split_at(&self, time: f64) {
        let time = snap(time);
        if !self.can_split_at(time) {
            return;
        }
        self.record_undo();
        let second_id = self.next_id();
        {
            let mut state = self.state.borrow_mut();
            let index = state.segments.iter().position(|s| s.start <= time && time < s.end).expect("checked by can_split_at");
            let original = state.segments[index];
            state.segments[index].end = time;
            state.segments.insert(index + 1, Segment { id: second_id, start: time, end: original.end });
            state.selected = Some(second_id);
        }
        self.raise_changed();
    }

    pub fn delete_selected(&self) {
        if !self.can_delete() {
            return;
        }
        self.record_undo();
        {
            let mut state = self.state.borrow_mut();
            let selected = state.selected.take();
            state.segments.retain(|s| Some(s.id) != selected);
        }
        self.raise_changed();
    }

    pub fn select(&self, id: Option<u64>) {
        if self.state.borrow().selected == id {
            return;
        }
        self.state.borrow_mut().selected = id;
        self.raise_changed();
    }

    pub fn begin_trim(&self, id: u64) {
        self.state.borrow_mut().selected = Some(id);
        self.record_undo();
    }

    /// Moves one edge of a clip; clips can't overlap their neighbours or get shorter than the minimum.
    /// Returns where the edge ended up.
    pub fn trim(&self, id: u64, edge: TrimEdge, time: f64) -> f64 {
        let time = snap(time);
        let result = {
            let mut state = self.state.borrow_mut();
            let Some(index) = state.segments.iter().position(|s| s.id == id) else { return time };
            let segment = state.segments[index];
            match edge {
                TrimEdge::Start => {
                    let lower = if index > 0 { state.segments[index - 1].end } else { 0.0 };
                    let start = time.clamp(lower, segment.end - MINIMUM_SEGMENT_DURATION);
                    state.segments[index].start = start;
                    start
                }
                TrimEdge::End => {
                    let upper = state.segments.get(index + 1).map_or(self.duration(), |s| s.start);
                    let end = time.clamp(segment.start + MINIMUM_SEGMENT_DURATION, upper);
                    state.segments[index].end = end;
                    end
                }
            }
        };
        self.raise_changed();
        result
    }

    pub fn end_trim(&self) {
        {
            // Drop the undo step if the drag didn't change anything.
            let mut state = self.state.borrow_mut();
            if state.undo.last() == Some(&state.segments) {
                state.undo.pop();
            }
        }
        self.raise_changed();
    }

    pub fn undo(&self) {
        {
            let mut state = self.state.borrow_mut();
            let Some(previous) = state.undo.pop() else { return };
            let current = std::mem::replace(&mut state.segments, previous);
            state.redo.push(current);
            clear_stale_selection(&mut state);
        }
        self.raise_changed();
    }

    pub fn redo(&self) {
        {
            let mut state = self.state.borrow_mut();
            let Some(next) = state.redo.pop() else { return };
            let current = std::mem::replace(&mut state.segments, next);
            state.undo.push(current);
            clear_stale_selection(&mut state);
        }
        self.raise_changed();
    }

    // MARK: - Audio and media

    pub fn set_volume(&self, track: usize, volume: f32) {
        {
            let mut state = self.state.borrow_mut();
            let Some(track) = state.audio_tracks.get_mut(track) else { return };
            let volume = volume.clamp(0.0, 1.0);
            if track.volume == volume {
                return;
            }
            track.volume = volume;
        }
        self.raise_changed();
    }

    pub fn toggle_mute(&self, track: usize) {
        {
            let mut state = self.state.borrow_mut();
            let Some(track) = state.audio_tracks.get_mut(track) else { return };
            track.muted = !track.muted;
        }
        self.raise_changed();
    }

    pub fn set_waveform(&self, track: usize, waveform: Vec<f32>) {
        if let Some(track) = self.state.borrow_mut().audio_tracks.get_mut(track) {
            track.waveform = waveform;
        }
        self.raise_changed();
    }

    pub fn set_thumbnails(&self, thumbnails: Vec<gdk::Texture>) {
        self.state.borrow_mut().thumbnails = thumbnails;
        self.raise_changed();
    }

    // MARK: - Change notification

    /// Called whenever clips, selection, audio levels or media change, so views can redraw.
    pub fn connect_changed(&self, listener: impl Fn() + 'static) {
        self.listeners.borrow_mut().push(Box::new(listener));
    }

    pub fn raise_changed(&self) {
        for listener in self.listeners.borrow().iter() {
            listener();
        }
    }

    // MARK: - Private

    fn segment(&self, start: f64, end: f64) -> Segment {
        Segment { id: self.next_id(), start, end }
    }

    fn next_id(&self) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        id
    }

    fn record_undo(&self) {
        let mut state = self.state.borrow_mut();
        let snapshot = state.segments.clone();
        state.undo.push(snapshot);
        state.redo.clear();
    }

    fn state_key(&self) -> String {
        let state = self.state.borrow();
        let segments: Vec<String> = state.segments.iter().map(|s| format!("{:.3}-{:.3}", s.start, s.end)).collect();
        let volumes: Vec<String> = state.audio_tracks.iter().map(|t| format!("{:.2}", t.effective_volume())).collect();
        format!("{}|{}", segments.join(";"), volumes.join(";"))
    }
}

fn clear_stale_selection(state: &mut State) {
    if let Some(id) = state.selected {
        if !state.segments.iter().any(|s| s.id == id) {
            state.selected = None;
        }
    }
}

/// Clip edges are kept on a millisecond grid so seeking to a clip's start lands exactly on it.
fn snap(time: f64) -> f64 {
    (time * 1000.0).round() / 1000.0
}

/// m:ss.t, or h:mm:ss.t for long videos.
pub fn format_precise(seconds: f64) -> String {
    let tenths = (seconds.max(0.0) * 10.0).floor() as u64;
    let (whole, fraction) = (tenths / 10, tenths % 10);
    let (hours, minutes, secs) = (whole / 3600, whole / 60 % 60, whole % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}.{fraction}")
    } else {
        format!("{minutes}:{secs:02}.{fraction}")
    }
}

/// m:ss, for ruler labels.
pub fn format_short(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", total / 60, total % 60)
}
