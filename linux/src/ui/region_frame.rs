//! A dashed red outline just outside the recorded region while recording. It lets clicks through and
//! sits entirely outside the region, so it never shows up in the video.

use gtk::prelude::*;

use super::placement;
use crate::recording::CaptureTarget;

const OUTLINE_GAP: i32 = 3;

pub struct RegionFrame {
    window: gtk::Window,
}

impl RegionFrame {
    /// `None` for full-screen recordings, or where windows can't be placed (GNOME on Wayland).
    pub fn new(target: &CaptureTarget) -> Option<Self> {
        let region = target.region?;
        if !placement::supported() {
            return None;
        }
        let (width, height) = (region.width + OUTLINE_GAP * 2, region.height + OUTLINE_GAP * 2);

        let area = gtk::DrawingArea::new();
        area.set_content_width(width);
        area.set_content_height(height);
        area.set_draw_func(|_, cr, width, height| {
            cr.set_source_rgba(1.0, 0.23, 0.19, 0.85);
            cr.set_line_width(2.0);
            cr.set_dash(&[6.0, 4.0], 0.0);
            cr.rectangle(1.0, 1.0, width as f64 - 2.0, height as f64 - 2.0);
            let _ = cr.stroke();
        });

        let window = gtk::Window::builder()
            .decorated(false)
            .resizable(false)
            .default_width(width)
            .default_height(height)
            .title("Cappa")
            .build();
        window.add_css_class("cappa-transparent");
        window.set_can_focus(false);
        window.set_child(Some(&area));
        placement::click_through(&window);
        placement::place(&window, &target.monitor, region.x - OUTLINE_GAP, region.y - OUTLINE_GAP);
        window.present();
        Some(Self { window })
    }

    pub fn close(&self) {
        self.window.destroy();
    }
}
