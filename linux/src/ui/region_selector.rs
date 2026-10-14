//! Dims every monitor and lets the user drag out the area to record. A click without dragging picks the
//! whole monitor; holding Shift keeps the area square; Esc cancels. The selection stays on the monitor
//! where the drag started.

use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{cairo, gdk, glib};

use crate::recording::Rect;

/// In logical pixels; smaller drags count as a click.
const MINIMUM_SIZE: f64 = 8.0;
const HINT: [&str; 2] = [
    "Drag to select the area to record · Hold Shift for a square",
    "Click to record the whole screen · Esc to cancel",
];

pub struct Selection {
    pub monitor: gdk::Monitor,
    /// Relative to the monitor; `None` for the whole monitor.
    pub region: Option<Rect>,
}

#[derive(Default)]
pub struct RegionSelector {
    views: RefCell<Vec<Rc<SelectionView>>>,
    result: RefCell<Option<async_channel::Sender<Option<Selection>>>>,
}

impl RegionSelector {
    pub fn is_active(&self) -> bool {
        self.result.borrow().is_some()
    }

    /// Shows the selector on every monitor and waits for a selection (`None` when cancelled).
    pub async fn select(self: &Rc<Self>) -> Option<Selection> {
        if self.is_active() {
            return None;
        }
        let (sender, receiver) = async_channel::bounded(1);
        *self.result.borrow_mut() = Some(sender);

        let display = gdk::Display::default()?;
        let monitors = display.monitors();
        for monitor in (0..monitors.n_items()).filter_map(|i| monitors.item(i).and_downcast::<gdk::Monitor>()) {
            let view = SelectionView::new(self, monitor);
            self.views.borrow_mut().push(view);
        }
        receiver.recv().await.ok().flatten()
    }

    pub fn cancel(&self) {
        self.finish(None);
    }

    fn finish(&self, selection: Option<Selection>) {
        let Some(sender) = self.result.borrow_mut().take() else { return };
        for view in self.views.borrow_mut().drain(..) {
            view.window.destroy();
        }
        let _ = sender.try_send(selection);
    }

    /// Only one selection at a time, across all monitors.
    fn began(&self, active: &SelectionView) {
        for view in self.views.borrow().iter().filter(|v| !std::ptr::eq(v.as_ref(), active)) {
            view.clear();
        }
    }
}

/// One full-screen, see-through window per monitor.
struct SelectionView {
    window: gtk::Window,
    area: gtk::DrawingArea,
    monitor: gdk::Monitor,
    drag_start: Cell<Option<(f64, f64)>>,
    drag_point: Cell<Option<(f64, f64)>>,
    shift: Cell<bool>,
    /// x, y, width, height in logical pixels.
    selection: Cell<Option<(f64, f64, f64, f64)>>,
}

impl SelectionView {
    fn new(selector: &Rc<RegionSelector>, monitor: gdk::Monitor) -> Rc<Self> {
        let window = gtk::Window::builder().decorated(false).title("Cappa — select an area").build();
        window.add_css_class("cappa-transparent");
        let area = gtk::DrawingArea::new();
        area.set_cursor_from_name(Some("crosshair"));
        area.set_focusable(true);
        window.set_child(Some(&area));

        let view = Rc::new(Self {
            window,
            area,
            monitor,
            drag_start: Cell::new(None),
            drag_point: Cell::new(None),
            shift: Cell::new(false),
            selection: Cell::new(None),
        });

        let weak = Rc::downgrade(&view);
        view.area.set_draw_func(move |_, cr, width, height| {
            if let Some(view) = weak.upgrade() {
                view.draw(cr, width as f64, height as f64);
            }
        });

        let drag = gtk::GestureDrag::new();
        let (weak, selector_weak) = (Rc::downgrade(&view), Rc::downgrade(selector));
        drag.connect_drag_begin(move |_, x, y| {
            let (Some(view), Some(selector)) = (weak.upgrade(), selector_weak.upgrade()) else { return };
            selector.began(&view);
            view.drag_start.set(Some(view.clamp(x, y)));
            view.drag_point.set(None);
            view.selection.set(None);
            view.area.grab_focus();
            view.area.queue_draw();
        });
        let weak = Rc::downgrade(&view);
        drag.connect_drag_update(move |gesture, dx, dy| {
            let Some(view) = weak.upgrade() else { return };
            let Some((x, y)) = view.drag_start.get() else { return };
            view.shift.set(gesture.current_event_state().contains(gdk::ModifierType::SHIFT_MASK));
            view.drag_point.set(Some(view.clamp(x + dx, y + dy)));
            view.update_selection();
        });
        let (weak, selector_weak) = (Rc::downgrade(&view), Rc::downgrade(selector));
        drag.connect_drag_end(move |_, _, _| {
            let (Some(view), Some(selector)) = (weak.upgrade(), selector_weak.upgrade()) else { return };
            if view.drag_start.take().is_none() {
                return;
            }
            let region = view
                .selection
                .get()
                .filter(|&(_, _, w, h)| w >= MINIMUM_SIZE && h >= MINIMUM_SIZE)
                .map(|(x, y, w, h)| Rect { x: x.round() as i32, y: y.round() as i32, width: w.round() as i32, height: h.round() as i32 });
            let monitor = view.monitor.clone();
            // Closing the windows from inside their own event handler is left to the main loop.
            glib::idle_add_local_once(move || selector.finish(Some(Selection { monitor, region })));
        });
        view.area.add_controller(drag);

        let keys = gtk::EventControllerKey::new();
        let (weak, selector_weak) = (Rc::downgrade(&view), Rc::downgrade(selector));
        keys.connect_key_pressed(move |_, key, _, _| {
            let (Some(view), Some(selector)) = (weak.upgrade(), selector_weak.upgrade()) else {
                return glib::Propagation::Proceed;
            };
            match key {
                gdk::Key::Escape => {
                    glib::idle_add_local_once(move || selector.cancel());
                }
                gdk::Key::Shift_L | gdk::Key::Shift_R => {
                    // Pressing or releasing Shift mid-drag reshapes the selection without waiting for the mouse to move.
                    view.shift.set(true);
                    view.update_selection();
                }
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(&view);
        keys.connect_key_released(move |_, key, _, _| {
            if let (Some(view), gdk::Key::Shift_L | gdk::Key::Shift_R) = (weak.upgrade(), key) {
                view.shift.set(false);
                view.update_selection();
            }
        });
        view.window.add_controller(keys);

        view.window.fullscreen_on_monitor(&view.monitor);
        view.window.present();
        view
    }

    fn clear(&self) {
        self.drag_start.set(None);
        self.drag_point.set(None);
        self.selection.set(None);
        self.area.queue_draw();
    }

    fn clamp(&self, x: f64, y: f64) -> (f64, f64) {
        let (width, height) = (self.area.width() as f64, self.area.height() as f64);
        (x.clamp(0.0, width).round(), y.clamp(0.0, height).round())
    }

    fn update_selection(&self) {
        let (Some((sx, sy)), Some((px, py))) = (self.drag_start.get(), self.drag_point.get()) else { return };
        let (mut width, mut height) = ((px - sx).abs(), (py - sy).abs());
        let (grows_right, grows_down) = (px >= sx, py >= sy);
        if self.shift.get() {
            // Follow the longer side, but stop at the screen edge in the direction of the drag.
            let room_x = if grows_right { self.area.width() as f64 - sx } else { sx };
            let room_y = if grows_down { self.area.height() as f64 - sy } else { sy };
            let side = width.max(height).min(room_x).min(room_y);
            (width, height) = (side, side);
        }
        let x = if grows_right { sx } else { sx - width };
        let y = if grows_down { sy } else { sy - height };
        self.selection.set(Some((x, y, width, height)));
        self.area.queue_draw();
    }

    fn draw(&self, cr: &cairo::Context, width: f64, height: f64) {
        cr.set_fill_rule(cairo::FillRule::EvenOdd);
        cr.rectangle(0.0, 0.0, width, height);
        if let Some((x, y, w, h)) = self.selection.get() {
            cr.rectangle(x, y, w, h);
        }
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.4);
        let _ = cr.fill();

        if let Some((x, y, w, h)) = self.selection.get() {
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(1.0);
            cr.rectangle(x - 0.5, y - 0.5, w + 1.0, h + 1.0);
            let _ = cr.stroke();
            let scale = self.monitor.scale_factor() as f64;
            let label = format!("{} × {}", (w * scale).round(), (h * scale).round());
            // Below the selection, or inside it when there's no room.
            let below = y + h + 6.0;
            let top = if below + 24.0 > height { y + h - 30.0 } else { below };
            draw_label(cr, &label, 12.0, x.min(width - 120.0).max(0.0), top, false);
        } else if self.drag_start.get().is_none() {
            draw_hint(cr, width, height);
        }
    }
}

/// A rounded dark box with white text; `top` is the box's top edge.
fn draw_label(cr: &cairo::Context, text: &str, size: f64, left: f64, top: f64, centered: bool) {
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(size);
    let Ok(extents) = cr.text_extents(text) else { return };
    let (padding_x, padding_y) = (8.0, 5.0);
    let (box_width, box_height) = (extents.x_advance() + padding_x * 2.0, size + padding_y * 2.0);
    let left = if centered { left - box_width / 2.0 } else { left };
    rounded_rectangle(cr, left, top, box_width, box_height, 5.0);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.7);
    let _ = cr.fill();
    cr.set_source_rgb(1.0, 1.0, 1.0);
    cr.move_to(left + padding_x, top + padding_y + size * 0.85);
    let _ = cr.show_text(text);
}

fn draw_hint(cr: &cairo::Context, width: f64, height: f64) {
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(15.0);
    let widest = HINT.iter().filter_map(|line| cr.text_extents(line).ok()).map(|e| e.x_advance()).fold(0.0, f64::max);
    let (line_height, padding_x, padding_y) = (22.0, 20.0, 14.0);
    let (box_width, box_height) = (widest + padding_x * 2.0, line_height * HINT.len() as f64 + padding_y * 2.0 - 4.0);
    let (left, top) = (width / 2.0 - box_width / 2.0, height / 2.0 - box_height / 2.0);
    rounded_rectangle(cr, left, top, box_width, box_height, 10.0);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.6);
    let _ = cr.fill();
    cr.set_source_rgb(1.0, 1.0, 1.0);
    for (i, line) in HINT.iter().enumerate() {
        let Ok(extents) = cr.text_extents(line) else { continue };
        cr.move_to(width / 2.0 - extents.x_advance() / 2.0, top + padding_y + 14.0 + i as f64 * line_height);
        let _ = cr.show_text(line);
    }
}

fn rounded_rectangle(cr: &cairo::Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    cr.new_sub_path();
    cr.arc(x + width - radius, y + radius, radius, -PI / 2.0, 0.0);
    cr.arc(x + width - radius, y + height - radius, radius, 0.0, PI / 2.0);
    cr.arc(x + radius, y + height - radius, radius, PI / 2.0, PI);
    cr.arc(x + radius, y + radius, radius, PI, 3.0 * PI / 2.0);
    cr.close_path();
}
