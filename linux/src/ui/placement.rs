//! Putting small windows at exact screen positions, above everything else: the recording controls and the
//! region outline. X11 allows this through the window manager; on Wayland it needs the wlr-layer-shell
//! protocol (KDE, Sway, Hyprland…; built with the `layer-shell` feature). GNOME's Wayland session has
//! neither, so there those windows aren't shown.
//!
//! Unlike macOS and Windows, Linux can't keep a window out of a screen capture, so callers only place
//! windows outside the recorded area.

use gtk::prelude::*;
use gtk::{cairo, gdk};

use crate::recording::is_x11;

pub fn supported() -> bool {
    if is_x11() {
        return true;
    }
    #[cfg(feature = "layer-shell")]
    if gtk4_layer_shell::is_supported() {
        return true;
    }
    false
}

/// Prepares `window` to show at (`x`, `y`) in logical pixels relative to `monitor`, above other windows
/// and outside the taskbar. Call before presenting it.
pub fn place(window: &gtk::Window, monitor: &gdk::Monitor, x: i32, y: i32) {
    #[cfg(feature = "layer-shell")]
    if !is_x11() && gtk4_layer_shell::is_supported() {
        use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
        window.init_layer_shell();
        window.set_namespace(Some("cappa"));
        window.set_layer(Layer::Overlay);
        window.set_monitor(Some(monitor));
        window.set_anchor(Edge::Left, true);
        window.set_anchor(Edge::Top, true);
        window.set_margin(Edge::Left, x);
        window.set_margin(Edge::Top, y);
        window.set_keyboard_mode(KeyboardMode::None);
        window.set_exclusive_zone(-1);
        return;
    }
    if is_x11() {
        let geometry = monitor.geometry();
        let scale = monitor.scale_factor();
        let position = ((geometry.x() + x) * scale, (geometry.y() + y) * scale);
        window.connect_realize(|window| x11::prepare(window));
        window.connect_map(move |window| {
            x11::move_to(window, position);
            // Some window managers place a window themselves right after it maps; move it back.
            let window = window.downgrade();
            gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(60), move || {
                if let Some(window) = window.upgrade() {
                    x11::move_to(&window, position);
                }
            });
        });
    }
}

/// Lets clicks pass through `window` to whatever is below it.
pub fn click_through(window: &gtk::Window) {
    window.connect_realize(|window| {
        if let Some(surface) = window.surface() {
            surface.set_input_region(&cairo::Region::create());
        }
    });
}

mod x11 {
    use std::sync::OnceLock;

    use gtk::prelude::*;
    use x11rb::connection::Connection;
    use x11rb::properties::WmHints;
    use x11rb::protocol::xproto::{AtomEnum, ConfigureWindowAux, ConnectionExt, PropMode};
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    fn connection() -> Option<&'static RustConnection> {
        static CONNECTION: OnceLock<Option<RustConnection>> = OnceLock::new();
        CONNECTION
            .get_or_init(|| x11rb::connect(None).map(|(connection, _)| connection).ok())
            .as_ref()
    }

    fn xid(window: &gtk::Window) -> Option<u32> {
        let surface = window.surface()?.downcast::<gdk_x11::X11Surface>().ok()?;
        Some(surface.xid() as u32)
    }

    fn atom(connection: &RustConnection, name: &str) -> Option<u32> {
        Some(connection.intern_atom(false, name.as_bytes()).ok()?.reply().ok()?.atom)
    }

    /// Before the window maps: a utility window that stays on top, on every workspace, outside the
    /// taskbar and pager, and never takes keyboard focus from the app being recorded.
    pub fn prepare(window: &gtk::Window) {
        let (Some(connection), Some(xid)) = (connection(), xid(window)) else { return };
        let result = (|| -> Option<()> {
            let window_type = atom(connection, "_NET_WM_WINDOW_TYPE")?;
            let utility = atom(connection, "_NET_WM_WINDOW_TYPE_UTILITY")?;
            let state = atom(connection, "_NET_WM_STATE")?;
            let states: Vec<u32> = ["_NET_WM_STATE_ABOVE", "_NET_WM_STATE_SKIP_TASKBAR", "_NET_WM_STATE_SKIP_PAGER", "_NET_WM_STATE_STICKY"]
                .iter()
                .filter_map(|name| atom(connection, name))
                .collect();
            connection.change_property32(PropMode::REPLACE, xid, window_type, AtomEnum::ATOM, &[utility]).ok()?;
            connection.change_property32(PropMode::REPLACE, xid, state, AtomEnum::ATOM, &states).ok()?;
            let mut hints = WmHints::new();
            hints.input = Some(false);
            hints.set(connection, xid).ok()?;
            connection.flush().ok()
        })();
        if result.is_none() {
            crate::log::write("Couldn't set X11 window properties");
        }
    }

    pub fn move_to(window: &gtk::Window, (x, y): (i32, i32)) {
        let (Some(connection), Some(xid)) = (connection(), xid(window)) else { return };
        let _ = connection.configure_window(xid, &ConfigureWindowAux::new().x(x).y(y));
        let _ = connection.flush();
    }
}
