//! Click highlights for X11: a thread watches the mouse buttons and the capture pipeline draws an
//! expanding ring that fades from black to white at each click, straight into the video.
//!
//! Wayland doesn't let apps observe clicks in other windows, so there the option is unavailable.

use std::f64::consts::TAU;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, KeyButMask};

const DURATION: Duration = Duration::from_millis(500);
// In logical pixels; multiplied by the screen's scale.
const START_RADIUS: f64 = 6.0;
const END_RADIUS: f64 = 30.0;
const LINE_WIDTH: f64 = 3.0;

#[derive(Clone, Copy)]
struct Click {
    at: Instant,
    /// Root-window pixels.
    x: i32,
    y: i32,
}

pub struct ClickTracker {
    clicks: Arc<Mutex<Vec<Click>>>,
    stop: Arc<AtomicBool>,
}

impl ClickTracker {
    /// Starts watching the buttons. Polling (rather than grabbing the pointer) never interferes with
    /// other apps.
    pub fn start() -> anyhow::Result<Arc<Self>> {
        let (connection, screen) = x11rb::connect(None)?;
        let root = connection.setup().roots[screen].root;
        let tracker = Arc::new(Self { clicks: Arc::default(), stop: Arc::default() });
        let (clicks, stop) = (tracker.clicks.clone(), tracker.stop.clone());
        let buttons = KeyButMask::BUTTON1 | KeyButMask::BUTTON2 | KeyButMask::BUTTON3;

        std::thread::Builder::new().name("cappa-clicks".into()).spawn(move || {
            let mut was_down = KeyButMask::from(0u16);
            while !stop.load(Ordering::Relaxed) {
                if let Ok(Ok(pointer)) = connection.query_pointer(root).map(|cookie| cookie.reply()) {
                    let down = pointer.mask & buttons;
                    // A button that wasn't down before is a new click.
                    if u16::from(down) & !u16::from(was_down) != 0 {
                        let mut clicks = clicks.lock().unwrap();
                        clicks.retain(|c| c.at.elapsed() < DURATION);
                        clicks.push(Click { at: Instant::now(), x: pointer.root_x.into(), y: pointer.root_y.into() });
                    }
                    was_down = down;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        })?;
        Ok(tracker)
    }

    /// Draws the current rings onto a frame whose top-left corner is `origin` in root pixels.
    pub fn draw(&self, cr: &gtk::cairo::Context, origin: (i32, i32), scale: f64) {
        let clicks = self.clicks.lock().unwrap().clone();
        for click in clicks {
            let progress = click.at.elapsed().as_secs_f64() / DURATION.as_secs_f64();
            if progress >= 1.0 {
                continue;
            }
            let eased = 1.0 - (1.0 - progress).powi(2); // ease out
            let radius = (START_RADIUS + (END_RADIUS - START_RADIUS) * eased) * scale;
            let shade = eased; // black → white
            cr.set_source_rgba(shade, shade, shade, 1.0 - eased);
            cr.set_line_width(LINE_WIDTH * scale);
            cr.new_path();
            cr.arc((click.x - origin.0) as f64, (click.y - origin.1) as f64, radius, 0.0, TAU);
            let _ = cr.stroke();
        }
    }
}

impl Drop for ClickTracker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
