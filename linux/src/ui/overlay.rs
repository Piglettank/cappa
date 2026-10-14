//! The floating recording controls: status dot, elapsed time, pause/resume and stop. Centred just below
//! a region in the top half of the screen and just above one in the bottom half.
//!
//! Linux can't keep a window out of a screen capture, so the controls only appear when there's room for
//! them outside the recorded region, and never during full-screen recordings (the tray menu, the hotkey
//! and the desktop's own screen-sharing indicator still work then).

use std::time::Duration;

use gtk::prelude::*;

use super::placement;
use crate::recording::{CaptureTarget, Rect, RecordingState};

/// Space between the controls and the region (whose red outline sits 3 px outside it).
const REGION_GAP: i32 = 12;
const EDGE_MARGIN: i32 = 8;

pub struct RecordingOverlay {
    window: gtk::Window,
    dot: gtk::Label,
    time: gtk::Label,
    pause: gtk::Button,
}

impl RecordingOverlay {
    /// `None` when the controls can't be shown without ending up in the video.
    pub fn new(target: &CaptureTarget, on_pause: impl Fn() + 'static, on_stop: impl Fn() + 'static) -> Option<Self> {
        let region = target.region?;
        if !placement::supported() {
            return None;
        }

        let dot = gtk::Label::new(Some("●"));
        dot.add_css_class("cappa-dot");
        let time = gtk::Label::new(Some("00:00"));
        time.add_css_class("cappa-time");
        time.set_xalign(0.0);
        let pause = gtk::Button::from_icon_name("media-playback-pause-symbolic");
        pause.set_tooltip_text(Some("Pause"));
        pause.connect_clicked(move |_| on_pause());
        let stop = gtk::Button::from_icon_name("media-playback-stop-symbolic");
        stop.set_tooltip_text(Some("Stop Recording"));
        stop.connect_clicked(move |_| on_stop());

        let pill = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        pill.add_css_class("cappa-pill");
        pill.append(&dot);
        pill.append(&time);
        pill.append(&pause);
        pill.append(&stop);

        // Draggable through the window manager where that's possible (X11).
        let handle = gtk::WindowHandle::new();
        handle.set_child(Some(&pill));

        let window = gtk::Window::builder().decorated(false).resizable(false).title("Cappa").build();
        window.add_css_class("cappa-transparent");
        window.set_child(Some(&handle));

        let (_, width, _, _) = handle.measure(gtk::Orientation::Horizontal, -1);
        let (_, height, _, _) = handle.measure(gtk::Orientation::Vertical, width);
        let (x, y) = position(target.monitor_geometry, region, width, height)?;
        placement::place(&window, &target.monitor, x, y);
        window.present();
        Some(Self { window, dot, time, pause })
    }

    pub fn update(&self, state: RecordingState, elapsed: Duration) {
        let paused = state == RecordingState::Paused;
        if paused {
            self.dot.add_css_class("paused");
        } else {
            self.dot.remove_css_class("paused");
        }
        self.pause.set_icon_name(if paused { "media-playback-start-symbolic" } else { "media-playback-pause-symbolic" });
        self.pause.set_tooltip_text(Some(if paused { "Resume" } else { "Pause" }));
        self.time.set_text(&format_elapsed(elapsed));
    }

    pub fn close(&self) {
        self.window.destroy();
    }
}

/// Where the controls go relative to the monitor, or `None` if they'd overlap the region.
fn position(monitor: Rect, region: Rect, width: i32, height: i32) -> Option<(i32, i32)> {
    let in_top_half = region.y + region.height / 2 < monitor.height / 2;
    let x = region.x + (region.width - width) / 2;
    let y = if in_top_half { region.bottom() + REGION_GAP } else { region.y - REGION_GAP - height };
    let x = x.clamp(EDGE_MARGIN, (monitor.width - width - EDGE_MARGIN).max(EDGE_MARGIN));
    let y = y.clamp(EDGE_MARGIN, (monitor.height - height - EDGE_MARGIN).max(EDGE_MARGIN));
    let controls = Rect { x, y, width, height };
    let outlined = Rect { x: region.x - 3, y: region.y - 3, width: region.width + 6, height: region.height + 6 };
    (!controls.intersects(outlined)).then_some((x, y))
}

fn format_elapsed(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds >= 3600 {
        format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
    } else {
        format!("{:02}:{:02}", seconds / 60, seconds % 60)
    }
}
