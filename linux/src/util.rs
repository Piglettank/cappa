//! Small desktop helpers shared by the recorder and the editor.

use std::path::Path;

use gtk::prelude::*;
use gtk::{gio, glib};

/// Shows a file selected in the file manager (Files, Dolphin, …), or opens its folder if that isn't supported.
pub fn reveal_in_file_manager(path: &Path) {
    let uri = gio::File::for_path(path).uri();
    let call = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).and_then(|bus| {
        bus.call_sync(
            Some("org.freedesktop.FileManager1"),
            "/org/freedesktop/FileManager1",
            "org.freedesktop.FileManager1",
            "ShowItems",
            Some(&(vec![uri.to_string()], "").to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        )
    });
    if let Err(e) = call {
        crate::log::error("Revealing file in the file manager", e);
        if let Some(folder) = path.parent() {
            open_path(folder);
        }
    }
}

/// Opens a file or folder in its default app.
pub fn open_path(path: &Path) {
    let uri = gio::File::for_path(path).uri();
    if let Err(e) = gio::AppInfo::launch_default_for_uri(&uri, gio::AppLaunchContext::NONE) {
        crate::log::error(&format!("Opening {}", path.display()), e);
    }
}

/// A simple message dialog. `parent` may be `None` for messages from the tray.
pub fn show_message(parent: Option<&gtk::Window>, heading: &str, body: &str) {
    let dialog = gtk::AlertDialog::builder().modal(true).message(heading).detail(body).build();
    dialog.show(parent);
}

/// Asks to confirm a destructive action; resolves to `true` when the user chose `confirm`.
pub async fn confirm(parent: &gtk::Window, heading: &str, body: &str, confirm: &str) -> bool {
    let dialog = gtk::AlertDialog::builder()
        .modal(true)
        .message(heading)
        .detail(body)
        .buttons(["Cancel", confirm])
        .cancel_button(0)
        .default_button(1)
        .build();
    matches!(dialog.choose_future(Some(parent)).await, Ok(1))
}

/// Runs a future on the UI thread's main loop.
pub fn spawn(future: impl std::future::Future<Output = ()> + 'static) {
    glib::MainContext::default().spawn_local(future);
}
