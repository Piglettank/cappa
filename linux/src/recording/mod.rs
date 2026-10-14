//! Recording: the state machine, capture sessions and the GStreamer pipelines behind them.

pub mod audio_devices;
mod click_tracker;
pub mod controller;
mod pipeline;
mod portal;
mod session;

use gtk::gdk;
use gtk::prelude::*;

pub use controller::Controller;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingState {
    Idle,
    Selecting,
    Starting,
    Recording,
    Paused,
    Stopping,
}

impl RecordingState {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Recording | Self::Paused)
    }
}

/// A rectangle in logical (scaled) pixels, like GDK's monitor geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn from_gdk(rect: &gdk::Rectangle) -> Self {
        Self { x: rect.x(), y: rect.y(), width: rect.width(), height: rect.height() }
    }

    pub fn offset(self, dx: i32, dy: i32) -> Self {
        Self { x: self.x + dx, y: self.y + dy, ..self }
    }

    pub fn right(self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(self) -> i32 {
        self.y + self.height
    }

    pub fn intersects(self, other: Rect) -> bool {
        self.x < other.right() && other.x < self.right() && self.y < other.bottom() && other.y < self.bottom()
    }
}

/// What to record: a monitor, optionally cropped to a region.
#[derive(Clone, Debug)]
pub struct CaptureTarget {
    pub monitor: gdk::Monitor,
    /// The monitor in the desktop's logical layout.
    pub monitor_geometry: Rect,
    /// Physical pixels per logical pixel (integer, as on X11; Wayland streams are measured instead).
    pub scale_factor: i32,
    /// Relative to the monitor, in logical pixels. `None` records the whole monitor.
    pub region: Option<Rect>,
}

impl CaptureTarget {
    pub fn new(monitor: gdk::Monitor, region: Option<Rect>) -> Self {
        Self {
            monitor_geometry: Rect::from_gdk(&monitor.geometry()),
            scale_factor: monitor.scale_factor().max(1),
            monitor,
            region,
        }
    }

    /// The recorded area relative to the monitor, in logical pixels.
    pub fn local_area(&self) -> Rect {
        self.region.unwrap_or(Rect { x: 0, y: 0, ..self.monitor_geometry })
    }
}

/// Whether the session runs on X11 (otherwise Wayland).
pub fn is_x11() -> bool {
    gdk::Display::default().is_some_and(|display| display.is::<gdk_x11::X11Display>())
}
