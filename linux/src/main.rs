//! Cappa for Linux: a tray app with no main window. Recording starts from the tray menu, the global
//! shortcut (Ctrl+Alt+Shift+R) or `cappa --toggle`; finished recordings open in the editor.
//!
//! Cappa is a single-instance GTK application: launching it again forwards the command line to the
//! running instance, which is how `--toggle` and opening files work.

mod autostart;
mod editor;
mod ffmpeg;
mod log;
mod paths;
mod recording;
mod settings;
mod ui;
mod util;

use std::cell::{OnceCell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use recording::Controller;
use settings::SettingsStore;
use ui::hotkey::SharedStatus;
use ui::tray::TrayIcon;

const APP_ID: &str = "dev.fallberg.Cappa";

/// Everything the running instance keeps alive.
struct AppState {
    controller: Rc<Controller>,
    hotkey: SharedStatus,
    tray: RefCell<Option<TrayIcon>>,
    _hold: gio::ApplicationHoldGuard,
}

thread_local! {
    static STATE: OnceCell<AppState> = const { OnceCell::new() };
}

fn main() -> glib::ExitCode {
    std::panic::set_hook(Box::new(|info| {
        log::write(format!("PANIC {info}"));
        eprintln!("{info}");
    }));

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.add_main_option("toggle", glib::Char::from(b't'), glib::OptionFlags::NONE, glib::OptionArg::None, "Start or stop a recording (bind this to a keyboard shortcut)", None);
    app.add_main_option("settings", glib::Char::from(b's'), glib::OptionFlags::NONE, glib::OptionArg::None, "Show the settings", None);
    app.add_main_option("quit", glib::Char::from(b'q'), glib::OptionFlags::NONE, glib::OptionArg::None, "Quit Cappa (saving a recording in progress)", None);
    app.connect_startup(startup);
    app.connect_command_line(command_line);
    app.run()
}

/// Runs once, in the primary instance.
fn startup(app: &adw::Application) {
    log::write(format!("Cappa {} starting", env!("CARGO_PKG_VERSION")));
    if let Err(e) = gst::init() {
        log::error("Initializing GStreamer", e);
    }
    // The editor's video sink, built into the app.
    if let Err(e) = gstgtk4::plugin_register_static() {
        log::error("Registering the GTK video sink", e);
    }
    ui::style::load();

    let settings = SettingsStore::load();
    let controller = Controller::new(app, settings);
    let is_x11 = recording::is_x11();
    log::write(if is_x11 { "Display server: X11" } else { "Display server: Wayland" });

    let weak = Rc::downgrade(&controller);
    let hotkey = ui::hotkey::register(is_x11, move || {
        if let Some(controller) = weak.upgrade() {
            controller.toggle();
        }
    });

    let state = AppState { controller, hotkey, tray: RefCell::default(), _hold: app.hold() };
    STATE.with(|s| {
        let _ = s.set(state);
    });

    // The tray is started a moment later, once the portal has had a chance to report the shortcut.
    let app = app.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || start_tray(&app));
}

fn start_tray(app: &adw::Application) {
    with_state(|state| {
        let shortcut = match &*state.hotkey.borrow() {
            ui::hotkey::HotkeyStatus::Active(shortcut) => Some(shortcut.clone()),
            _ => None,
        };
        let tray = ui::tray::start(&state.controller, shortcut, show_settings, {
            let app = app.clone();
            move || quit(&app)
        });
        if tray.is_none() {
            // No tray on this desktop: the settings window, with its Record button, is the way in.
            show_settings();
        }
        *state.tray.borrow_mut() = tray;
    });
}

/// Handles `cappa [--toggle|--settings|--quit] [video…]`, from this launch or a later one.
fn command_line(app: &adw::Application, command_line: &gio::ApplicationCommandLine) -> glib::ExitCode {
    let options = command_line.options_dict();
    let files: Vec<_> = command_line
        .arguments()
        .into_iter()
        .skip(1)
        .filter(|arg| !arg.to_string_lossy().starts_with('-'))
        .filter_map(|arg| command_line.create_file_for_arg(arg).path())
        .collect();

    if options.contains("quit") {
        quit(app);
    } else if options.contains("toggle") {
        with_state(|state| state.controller.toggle());
    } else if options.contains("settings") {
        show_settings();
    } else if !files.is_empty() {
        for file in files {
            editor::open(app, &file);
        }
    } else if command_line.is_remote() {
        // Launched again from the app menu: show something rather than nothing.
        show_settings();
    }
    glib::ExitCode::SUCCESS
}

fn show_settings() {
    with_state(|state| ui::settings_window::show(&state.controller, &state.hotkey));
}

/// Saves a recording in progress, then quits.
fn quit(app: &adw::Application) {
    let app = app.clone();
    util::spawn(async move {
        if let Some(controller) = STATE.with(|s| s.get().map(|state| state.controller.clone())) {
            controller.finish().await;
        }
        log::write("Cappa exiting");
        app.quit();
    });
}

fn with_state(action: impl FnOnce(&AppState)) {
    STATE.with(|s| {
        if let Some(state) = s.get() {
            action(state);
        }
    });
}
