//! The video editor: trim, split and delete clips, adjust each audio track, and export.

mod export;
mod media;
mod model;
mod playback;
mod timeline;
mod window;

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;

use adw::prelude::*;
use gtk::gdk;

use window::Editor;

thread_local! {
    static OPEN_EDITORS: RefCell<Vec<Rc<Editor>>> = RefCell::default();
    static STYLE_LOADED: Cell<bool> = Cell::new(false);
}

const CSS: &str = "
.cappa-video { background-color: black; }
.cappa-busy { background-color: rgba(0, 0, 0, 0.45); }
.cappa-busy-card { padding: 24px; }
";

/// Opens a video in the editor, or brings its window forward if it's already open.
pub fn open(app: &adw::Application, path: &Path) {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    let existing = OPEN_EDITORS.with_borrow(|editors| editors.iter().find(|e| e.source_path() == path).cloned());
    if let Some(editor) = existing {
        editor.window.present();
        return;
    }

    setup_once(app);
    let closed_path = path.clone();
    let editor = Editor::new(app, &path, move || {
        OPEN_EDITORS.with_borrow_mut(|editors| editors.retain(|e| e.source_path() != closed_path));
    });
    OPEN_EDITORS.with_borrow_mut(|editors| editors.push(editor));
}

/// Styles for the editor, and the export shortcuts shown next to the menu items.
fn setup_once(app: &adw::Application) {
    if STYLE_LOADED.replace(true) {
        return;
    }
    if let Some(display) = gdk::Display::default() {
        let provider = gtk::CssProvider::new();
        provider.load_from_string(CSS);
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
    app.set_accels_for_action("win.save-video", &["<Control>e"]);
    app.set_accels_for_action("win.copy", &["<Control><Shift>c"]);
    app.set_accels_for_action("win.save-gif", &["<Control><Shift>e"]);
}
