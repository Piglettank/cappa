//! The timeline maps the whole source at a fixed scale. Kept clips are bright, cut-away parts stay visible
//! but dimmed, so an edge can be dragged back to restore them. Audio tracks get lanes below the video; they
//! follow the video's clips. Lane headers (mute/volume) live in the editor window, aligned to these rows.

use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{cairo, gdk};

use super::model::{format_short, EditorModel, Segment, TrimEdge};
use super::playback::Playback;

pub const RULER_HEIGHT: f64 = 22.0;
pub const VIDEO_HEIGHT: f64 = 72.0;
pub const AUDIO_HEIGHT: f64 = 38.0;
pub const LANE_SPACING: f64 = 6.0;
const HANDLE_WIDTH: f64 = 10.0;
const CORNER: f64 = 6.0;

type Rgb = (f64, f64, f64);
const SELECTED: Rgb = (1.0, 0.8, 0.0);
const PLAYHEAD: Rgb = (1.0, 0.231, 0.188);
const RULER_TEXT: Rgb = (0.5, 0.5, 0.5);
const COMPUTER_AUDIO: Rgb = (0.188, 0.69, 0.78);
const MICROPHONE: Rgb = (1.0, 0.584, 0.0);
const MUTED: Rgb = (0.557, 0.557, 0.576);

pub fn total_height(audio_track_count: usize) -> f64 {
    RULER_HEIGHT + LANE_SPACING + VIDEO_HEIGHT + audio_track_count as f64 * (LANE_SPACING + AUDIO_HEIGHT)
}

const VIDEO_TOP: f64 = RULER_HEIGHT + LANE_SPACING;

fn audio_top(index: usize) -> f64 {
    VIDEO_TOP + VIDEO_HEIGHT + LANE_SPACING + index as f64 * (AUDIO_HEIGHT + LANE_SPACING)
}

#[derive(Clone, Copy)]
struct Trim {
    id: u64,
    edge: TrimEdge,
    grab_offset: f64,
}

pub struct Timeline {
    area: gtk::DrawingArea,
    model: Rc<EditorModel>,
    playback: Rc<Playback>,
    /// Thumbnails as cairo surfaces, converted once.
    tiles: RefCell<Vec<cairo::ImageSurface>>,
    trim: Cell<Option<Trim>>,
    scrubbing: Cell<bool>,
    scrub_selects_clip: Cell<bool>,
}

impl Timeline {
    pub fn new(model: Rc<EditorModel>, playback: Rc<Playback>) -> Rc<Self> {
        let area = gtk::DrawingArea::builder().hexpand(true).valign(gtk::Align::Start).build();
        let timeline = Rc::new(Self {
            area,
            model: model.clone(),
            playback: playback.clone(),
            tiles: RefCell::default(),
            trim: Cell::new(None),
            scrubbing: Cell::new(false),
            scrub_selects_clip: Cell::new(false),
        });

        let weak = Rc::downgrade(&timeline);
        timeline.area.set_draw_func(move |_, cr, width, height| {
            if let Some(timeline) = weak.upgrade() {
                timeline.draw(cr, width as f64, height as f64);
            }
        });

        let drag = gtk::GestureDrag::new();
        let weak = Rc::downgrade(&timeline);
        drag.connect_drag_begin(move |_, x, y| {
            if let Some(timeline) = weak.upgrade() {
                timeline.drag_begin(x, y);
            }
        });
        let weak = Rc::downgrade(&timeline);
        drag.connect_drag_update(move |gesture, dx, _| {
            if let (Some(timeline), Some((x, _))) = (weak.upgrade(), gesture.start_point()) {
                timeline.drag_update(x + dx);
            }
        });
        let weak = Rc::downgrade(&timeline);
        drag.connect_drag_end(move |_, _, _| {
            if let Some(timeline) = weak.upgrade() {
                timeline.drag_end();
            }
        });
        timeline.area.add_controller(drag);

        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(&timeline);
        motion.connect_motion(move |_, x, y| {
            if let Some(timeline) = weak.upgrade() {
                if timeline.trim.get().is_none() {
                    let cursor = timeline.hit_handle(x, y).map(|_| "ew-resize");
                    timeline.area.set_cursor_from_name(cursor);
                }
            }
        });
        timeline.area.add_controller(motion);

        let weak = Rc::downgrade(&timeline);
        model.connect_changed(move || {
            if let Some(timeline) = weak.upgrade() {
                timeline.refresh();
            }
        });
        let weak = Rc::downgrade(&timeline);
        playback.connect_updated(move || {
            if let Some(timeline) = weak.upgrade() {
                timeline.area.queue_draw();
            }
        });
        timeline.refresh();
        timeline
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    fn refresh(&self) {
        let thumbnails = self.model.thumbnails();
        if self.tiles.borrow().len() != thumbnails.len() {
            *self.tiles.borrow_mut() = thumbnails.iter().filter_map(surface_from_texture).collect();
        }
        self.area.set_content_height(total_height(self.model.audio_track_count()) as i32);
        self.area.queue_draw();
    }

    // MARK: - Geometry

    fn width(&self) -> f64 {
        self.area.width() as f64
    }

    fn x(&self, time: f64) -> f64 {
        let duration = self.model.duration();
        if duration > 0.0 { time / duration * self.width() } else { 0.0 }
    }

    fn time_at(&self, x: f64) -> f64 {
        let (duration, width) = (self.model.duration(), self.width());
        if duration > 0.0 && width > 0.0 { (x / width).clamp(0.0, 1.0) * duration } else { 0.0 }
    }

    fn clip_rect(&self, segment: &Segment) -> (f64, f64, f64, f64) {
        let x = self.x(segment.start);
        (x, VIDEO_TOP, (self.x(segment.end) - x).max(2.0), VIDEO_HEIGHT)
    }

    // MARK: - Drawing

    fn draw(&self, cr: &cairo::Context, width: f64, height: f64) {
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        self.draw_ruler(cr, width);
        self.draw_video_lane(cr, width);
        for (index, track) in self.model.audio_tracks().iter().enumerate() {
            self.draw_audio_lane(cr, width, index, track);
        }
        self.draw_playhead(cr, height);
    }

    fn draw_ruler(&self, cr: &cairo::Context, width: f64) {
        let duration = self.model.duration();
        if duration <= 0.0 {
            return;
        }
        const STEPS: [f64; 12] = [0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0];
        let step = STEPS.into_iter().find(|s| s / duration * width >= 64.0).unwrap_or(3600.0);
        set_rgb(cr, RULER_TEXT, 1.0);
        cr.set_line_width(1.0);
        cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Normal);
        cr.set_font_size(10.0);
        let mut t = 0.0;
        while t <= duration {
            let x = self.x(t).round() + 0.5;
            cr.move_to(x, RULER_HEIGHT - 6.0);
            cr.line_to(x, RULER_HEIGHT);
            let _ = cr.stroke();
            cr.move_to(x + 3.0, RULER_HEIGHT - 9.0);
            let _ = cr.show_text(&format_short(t));
            t += step;
        }
    }

    fn draw_video_lane(&self, cr: &cairo::Context, width: f64) {
        let lane = (0.0, VIDEO_TOP, width, VIDEO_HEIGHT);

        // The whole source, dimmed: what's been cut away.
        let _ = cr.save();
        rounded_rect(cr, lane, CORNER);
        cr.clip();
        cr.set_source_rgba(0.5, 0.5, 0.5, 0.08);
        cr.paint().ok();
        self.draw_filmstrip(cr, lane, 0.25);
        let _ = cr.restore();

        let selected_id = self.model.selected_id();
        for segment in self.model.segments() {
            let clip = self.clip_rect(&segment);
            let selected = Some(segment.id) == selected_id;

            let _ = cr.save();
            rounded_rect(cr, clip, CORNER);
            cr.clip();
            self.draw_filmstrip(cr, lane, 1.0);
            let handle = HANDLE_WIDTH.min(clip.2 / 2.0);
            if selected { set_rgb(cr, SELECTED, 1.0) } else { cr.set_source_rgba(1.0, 1.0, 1.0, 0.9) }
            cr.rectangle(clip.0, clip.1, handle, clip.3);
            cr.rectangle(clip.0 + clip.2 - handle, clip.1, handle, clip.3);
            let _ = cr.fill();
            let _ = cr.restore();

            let line = if selected { 3.0 } else { 1.5 };
            if selected { set_rgb(cr, SELECTED, 1.0) } else { cr.set_source_rgba(1.0, 1.0, 1.0, 0.9) }
            cr.set_line_width(line);
            rounded_rect(cr, inset(clip, line / 2.0), CORNER);
            let _ = cr.stroke();
        }
    }

    fn draw_filmstrip(&self, cr: &cairo::Context, lane: (f64, f64, f64, f64), alpha: f64) {
        let tiles = self.tiles.borrow();
        if tiles.is_empty() {
            return;
        }
        let tile_width = lane.2 / tiles.len() as f64;
        for (i, tile) in tiles.iter().enumerate() {
            let (x, y, w, h) = (lane.0 + i as f64 * tile_width, lane.1, tile_width + 0.5, lane.3);
            let (image_width, image_height) = (tile.width() as f64, tile.height() as f64);
            if image_width <= 0.0 || image_height <= 0.0 {
                continue;
            }
            // Fill the tile, cropping the overflow (like UniformToFill).
            let scale = (w / image_width).max(h / image_height);
            let _ = cr.save();
            cr.rectangle(x, y, w, h);
            cr.clip();
            cr.translate(x + (w - image_width * scale) / 2.0, y + (h - image_height * scale) / 2.0);
            cr.scale(scale, scale);
            if cr.set_source_surface(tile, 0.0, 0.0).is_ok() {
                let _ = cr.paint_with_alpha(alpha);
            }
            let _ = cr.restore();
        }
    }

    fn draw_audio_lane(&self, cr: &cairo::Context, width: f64, index: usize, track: &super::model::AudioTrack) {
        let top = audio_top(index);
        let color = if track.muted { MUTED } else if track.is_microphone { MICROPHONE } else { COMPUTER_AUDIO };

        cr.set_source_rgba(0.5, 0.5, 0.5, 0.08);
        rounded_rect(cr, (0.0, top, width, AUDIO_HEIGHT), 5.0);
        let _ = cr.fill();

        let selected_id = self.model.selected_id();
        for segment in self.model.segments() {
            let x = self.x(segment.start);
            let clip = (x, top, (self.x(segment.end) - x).max(2.0), AUDIO_HEIGHT);
            set_rgb(cr, color, 0.18);
            rounded_rect(cr, clip, 5.0);
            let _ = cr.fill_preserve();
            if Some(segment.id) == selected_id {
                set_rgb(cr, SELECTED, 1.0);
                cr.set_line_width(2.0);
            } else {
                set_rgb(cr, color, 0.25);
                cr.set_line_width(1.0);
            }
            let _ = cr.stroke();
        }

        let levels = &track.waveform;
        let duration = self.model.duration();
        if levels.is_empty() || duration <= 0.0 {
            return;
        }
        let gain = if track.muted { 1.0 } else { track.volume as f64 };
        let middle = top + AUDIO_HEIGHT / 2.0;
        let mut x = 1.0;
        while x < width - 1.0 {
            let fraction = x / width;
            let level = levels[((fraction * levels.len() as f64) as usize).min(levels.len() - 1)] as f64;
            let height = (level * gain * (AUDIO_HEIGHT - 8.0)).max(1.0);
            let kept = self.model.segment_at(fraction * duration).is_some();
            set_rgb(cr, color, if kept { 1.0 } else { 0.25 });
            cr.rectangle(x, middle - height / 2.0, 1.5, height);
            let _ = cr.fill();
            x += 2.5;
        }
    }

    fn draw_playhead(&self, cr: &cairo::Context, height: f64) {
        let x = self.x(self.model.current_time());
        set_rgb(cr, PLAYHEAD, 1.0);
        cr.rectangle(x - 1.0, 2.0, 2.0, height - 2.0);
        let _ = cr.fill();
        cr.move_to(x - 6.0, 0.0);
        cr.line_to(x + 6.0, 0.0);
        cr.line_to(x, 9.0);
        cr.close_path();
        let _ = cr.fill();
    }

    // MARK: - Mouse

    fn drag_begin(&self, x: f64, y: f64) {
        self.playback.pause();
        if let Some((segment, edge)) = self.hit_handle(x, y) {
            let edge_x = self.x(if edge == TrimEdge::Start { segment.start } else { segment.end });
            self.trim.set(Some(Trim { id: segment.id, edge, grab_offset: x - edge_x }));
            self.model.begin_trim(segment.id);
        } else {
            // Clicking a clip (in any lane) selects it; the ruler and cut-away parts deselect.
            self.scrubbing.set(true);
            self.scrub_selects_clip.set(y >= VIDEO_TOP);
            self.scrub_to(x);
        }
    }

    fn drag_update(&self, x: f64) {
        if let Some(trim) = self.trim.get() {
            let time = self.model.trim(trim.id, trim.edge, self.time_at(x - trim.grab_offset));
            self.playback.seek(time);
        } else if self.scrubbing.get() {
            self.scrub_to(x);
        }
    }

    fn drag_end(&self) {
        if self.trim.take().is_some() {
            self.model.end_trim();
        }
        self.scrubbing.set(false);
    }

    fn scrub_to(&self, x: f64) {
        let time = self.time_at(x);
        let selection = if self.scrub_selects_clip.get() { self.model.segment_at(time).map(|s| s.id) } else { None };
        self.model.select(selection);
        self.playback.seek(time);
    }

    fn hit_handle(&self, x: f64, y: f64) -> Option<(Segment, TrimEdge)> {
        if !(VIDEO_TOP..=VIDEO_TOP + VIDEO_HEIGHT).contains(&y) {
            return None;
        }
        self.model.segments().into_iter().find_map(|segment| {
            let (left, _, width, _) = self.clip_rect(&segment);
            let handle = HANDLE_WIDTH.min(width / 2.0);
            if x >= left - 2.0 && x <= left + handle {
                Some((segment, TrimEdge::Start))
            } else if x >= left + width - handle && x <= left + width + 2.0 {
                Some((segment, TrimEdge::End))
            } else {
                None
            }
        })
    }
}

fn set_rgb(cr: &cairo::Context, (r, g, b): Rgb, alpha: f64) {
    cr.set_source_rgba(r, g, b, alpha);
}

fn inset((x, y, w, h): (f64, f64, f64, f64), by: f64) -> (f64, f64, f64, f64) {
    (x + by, y + by, (w - 2.0 * by).max(0.0), (h - 2.0 * by).max(0.0))
}

fn rounded_rect(cr: &cairo::Context, (x, y, w, h): (f64, f64, f64, f64), radius: f64) {
    let r = radius.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

/// Copies a texture's pixels into a cairo surface. `download` writes premultiplied BGRA on little-endian
/// machines, which is exactly cairo's ARGB32.
fn surface_from_texture(texture: &gdk::Texture) -> Option<cairo::ImageSurface> {
    let (width, height) = (texture.width(), texture.height());
    let stride = cairo::Format::ARgb32.stride_for_width(width as u32).ok()?;
    let mut data = vec![0u8; stride as usize * height as usize];
    texture.download(&mut data, stride as usize);
    cairo::ImageSurface::create_for_data(data, cairo::Format::ARgb32, width, height, stride).ok()
}
