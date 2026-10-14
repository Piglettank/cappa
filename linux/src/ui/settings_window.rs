//! Settings: starting at login, recording audio, video options, the controls, and where recordings are saved. It doubles as
//! Cappa's home window on desktops without a tray, so it also has a Record button.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use super::hotkey::{HotkeyStatus, SharedStatus};
use crate::recording::{audio_devices, is_x11, Controller};
use crate::settings::{RecordingQuality, Settings};

thread_local! {
    static SHARED: RefCell<Option<adw::PreferencesWindow>> = const { RefCell::new(None) };
}

/// Shows the settings window, creating it the first time.
pub fn show(controller: &Rc<Controller>, hotkey: &SharedStatus) {
    if let Some(window) = SHARED.with_borrow(|w| w.clone()) {
        window.present();
        return;
    }
    let window = build(controller, hotkey);
    window.connect_close_request(|_| {
        SHARED.with_borrow_mut(|w| *w = None);
        glib::Propagation::Proceed
    });
    SHARED.with_borrow_mut(|w| *w = Some(window.clone()));
    window.present();
}

fn build(controller: &Rc<Controller>, hotkey: &SharedStatus) -> adw::PreferencesWindow {
    let store = controller.settings.clone();
    let settings = store.get();
    let window = adw::PreferencesWindow::builder()
        .title("Cappa Settings")
        .default_width(560)
        .default_height(720)
        .search_enabled(false)
        .build();
    let page = adw::PreferencesPage::new();
    window.add(&page);
    // While the window updates itself from the settings, its own change handlers stay quiet.
    let updating = Rc::new(Cell::new(false));

    // MARK: Record
    let record_group = adw::PreferencesGroup::new();
    let record_row = adw::ActionRow::builder()
        .title("Record the screen")
        .subtitle(hotkey_text(&hotkey.borrow()))
        .build();
    let record_button = gtk::Button::builder().label("Record…").valign(gtk::Align::Center).build();
    record_button.add_css_class("suggested-action");
    let weak_controller = Rc::downgrade(controller);
    let weak_window = window.downgrade();
    record_button.connect_clicked(move |_| {
        // The window would otherwise be in the recording; it's easy to bring back from the tray.
        if let Some(window) = weak_window.upgrade() {
            window.close();
        }
        if let Some(controller) = weak_controller.upgrade() {
            controller.start_recording();
        }
    });
    record_row.add_suffix(&record_button);
    record_group.add(&record_row);
    page.add(&record_group);

    // MARK: General
    let general = adw::PreferencesGroup::builder().title("General").build();
    let autostart = adw::SwitchRow::builder()
        .title("Start Cappa automatically on startup")
        .active(crate::autostart::is_enabled())
        .build();
    autostart.connect_active_notify(|row| {
        crate::autostart::set_enabled(row.is_active());
        row.set_active(crate::autostart::is_enabled());
    });
    // The user may also have changed it in their desktop's settings meanwhile.
    {
        let autostart = autostart.downgrade();
        window.connect_is_active_notify(move |window| {
            if let (true, Some(row)) = (window.is_active(), autostart.upgrade()) {
                row.set_active(crate::autostart::is_enabled());
            }
        });
    }
    general.add(&autostart);
    page.add(&general);

    // MARK: Audio
    let audio = adw::PreferencesGroup::builder().title("Audio").build();
    let system_audio = adw::SwitchRow::builder()
        .title("Record system audio")
        .subtitle("Sound from all apps (whatever plays on the default output).")
        .active(settings.record_system_audio)
        .build();
    bind_switch(&system_audio, &store, &updating, |s, on| s.record_system_audio = on);
    audio.add(&system_audio);

    let microphone = adw::SwitchRow::builder()
        .title("Record microphone")
        .subtitle("Recorded on its own track, so it can be adjusted separately in the editor.")
        .active(settings.record_microphone)
        .build();
    bind_switch(&microphone, &store, &updating, |s, on| s.record_microphone = on);
    audio.add(&microphone);

    let devices = Rc::new(RefCell::new(Vec::<String>::new()));
    let device_names = gtk::StringList::new(&[] as &[&str]);
    let device_row = adw::ComboRow::builder().title("Microphone").model(&device_names).build();
    device_row.set_sensitive(settings.record_microphone);
    let refresh_devices = {
        let (devices, device_names, device_row, store, updating) =
            (devices.clone(), device_names.clone(), device_row.clone(), store.clone(), updating.clone());
        move || {
            let selected_id = store.get().microphone_id;
            let mut ids = vec![String::new()];
            let mut names = vec![match audio_devices::default_microphone_name() {
                Some(name) => format!("System Default ({name})"),
                None => "System Default".into(),
            }];
            for device in audio_devices::microphones() {
                ids.push(device.id);
                names.push(device.name);
            }
            if !ids.contains(&selected_id) {
                ids.push(selected_id.clone());
                names.push("Selected microphone (not connected)".into());
            }
            updating.set(true);
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            device_names.splice(0, device_names.n_items(), &names);
            device_row.set_selected(ids.iter().position(|id| *id == selected_id).unwrap_or(0) as u32);
            *devices.borrow_mut() = ids;
            updating.set(false);
        }
    };
    refresh_devices();
    {
        let (devices, store, updating) = (devices.clone(), store.clone(), updating.clone());
        device_row.connect_selected_notify(move |row| {
            if updating.get() {
                return;
            }
            if let Some(id) = devices.borrow().get(row.selected() as usize).cloned() {
                store.update(|s| s.microphone_id = id);
            }
        });
    }
    // Devices come and go; refresh the list whenever the window comes forward.
    window.connect_is_active_notify(move |window| {
        if window.is_active() {
            refresh_devices();
        }
    });
    audio.add(&device_row);
    page.add(&audio);

    // MARK: Video
    let video = adw::PreferencesGroup::builder().title("Video").build();
    let quality_names: Vec<&str> = RecordingQuality::ALL.iter().map(|q| q.title()).collect();
    let quality = adw::ComboRow::builder()
        .title("Quality")
        .subtitle("Lower quality makes smaller files.")
        .model(&gtk::StringList::new(&quality_names))
        .selected(RecordingQuality::ALL.iter().position(|q| *q == settings.quality).unwrap_or(2) as u32)
        .build();
    {
        let (store, updating) = (store.clone(), updating.clone());
        quality.connect_selected_notify(move |row| {
            if let (false, Some(q)) = (updating.get(), RecordingQuality::ALL.get(row.selected() as usize)) {
                store.update(|s| s.quality = *q);
            }
        });
    }
    video.add(&quality);

    let frame_rate = adw::ComboRow::builder()
        .title("Frame rate")
        .model(&gtk::StringList::new(&["30 fps", "60 fps"]))
        .selected(if settings.frame_rate == 30 { 0 } else { 1 })
        .build();
    {
        let (store, updating) = (store.clone(), updating.clone());
        frame_rate.connect_selected_notify(move |row| {
            if !updating.get() {
                let fps = if row.selected() == 0 { 30 } else { 60 };
                store.update(|s| s.frame_rate = fps);
            }
        });
    }
    video.add(&frame_rate);

    let cursor = adw::SwitchRow::builder().title("Show cursor").active(settings.show_cursor).build();
    bind_switch(&cursor, &store, &updating, |s, on| s.show_cursor = on);
    video.add(&cursor);

    let clicks_supported = is_x11();
    let clicks = adw::SwitchRow::builder()
        .title("Highlight clicks")
        .subtitle(if clicks_supported {
            "Shows a splash effect where you click."
        } else {
            "Only available in X11 sessions: Wayland doesn't let apps see clicks in other windows."
        })
        .active(settings.highlight_clicks)
        .build();
    clicks.set_sensitive(clicks_supported && settings.show_cursor);
    bind_switch(&clicks, &store, &updating, |s, on| s.highlight_clicks = on);
    video.add(&clicks);

    let controls = adw::SwitchRow::builder()
        .title("Show controls while recording")
        .subtitle(
            "A timer with Pause and Stop next to the recorded area. Linux can't hide windows from a recording, \
             so they only appear when there's room outside the area, and not on GNOME's Wayland session.",
        )
        .active(settings.show_overlay)
        .build();
    bind_switch(&controls, &store, &updating, |s, on| s.show_overlay = on);
    video.add(&controls);
    page.add(&video);

    // MARK: Recordings
    let recordings = adw::PreferencesGroup::builder().title("Recordings").build();
    let folder_row = adw::ActionRow::builder()
        .title("Save recordings to")
        .subtitle(settings.recordings_folder.display().to_string())
        .subtitle_selectable(true)
        .build();
    let choose = gtk::Button::builder().label("Choose…").valign(gtk::Align::Center).build();
    {
        let (store, window) = (store.clone(), window.downgrade());
        choose.connect_clicked(move |_| {
            let (store, window) = (store.clone(), window.clone());
            crate::util::spawn(async move {
                let dialog = gtk::FileDialog::builder()
                    .title("Choose where Cappa saves new recordings")
                    .modal(true)
                    .initial_folder(&gio::File::for_path(store.get().recordings_folder))
                    .build();
                if let Ok(folder) = dialog.select_folder_future(window.upgrade().as_ref()).await {
                    if let Some(path) = folder.path() {
                        store.update(|s| s.recordings_folder = path);
                    }
                }
            });
        });
    }
    let open = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Show in Files")
        .valign(gtk::Align::Center)
        .build();
    open.add_css_class("flat");
    let weak_controller = Rc::downgrade(controller);
    open.connect_clicked(move |_| {
        if let Some(controller) = weak_controller.upgrade() {
            controller.open_recordings_folder();
        }
    });
    let use_default = gtk::Button::builder().label("Use Default").valign(gtk::Align::Center).build();
    use_default.set_sensitive(settings.recordings_folder != crate::paths::default_recordings_dir());
    {
        let store = store.clone();
        use_default.connect_clicked(move |_| store.update(|s| s.recordings_folder = crate::paths::default_recordings_dir()));
    }
    folder_row.add_suffix(&choose);
    folder_row.add_suffix(&open);
    folder_row.add_suffix(&use_default);
    recordings.add(&folder_row);
    page.add(&recordings);

    // Follow changes made elsewhere (the tray menu) while the window is open.
    let weak_window = window.downgrade();
    let hotkey = hotkey.clone();
    store.connect_changed(move |settings: &Settings| {
        if weak_window.upgrade().is_none() {
            return;
        }
        updating.set(true);
        system_audio.set_active(settings.record_system_audio);
        microphone.set_active(settings.record_microphone);
        device_row.set_sensitive(settings.record_microphone);
        cursor.set_active(settings.show_cursor);
        clicks.set_active(settings.highlight_clicks);
        clicks.set_sensitive(clicks_supported && settings.show_cursor);
        controls.set_active(settings.show_overlay);
        folder_row.set_subtitle(&settings.recordings_folder.display().to_string());
        use_default.set_sensitive(settings.recordings_folder != crate::paths::default_recordings_dir());
        record_row.set_subtitle(&hotkey_text(&hotkey.borrow()));
        updating.set(false);
    });
    window
}

fn bind_switch(row: &adw::SwitchRow, store: &crate::settings::SettingsStore, updating: &Rc<Cell<bool>>, set: fn(&mut Settings, bool)) {
    let (store, updating) = (store.clone(), updating.clone());
    row.connect_active_notify(move |row| {
        if !updating.get() {
            let on = row.is_active();
            store.update(|s| set(s, on));
        }
    });
}

fn hotkey_text(status: &HotkeyStatus) -> String {
    match status {
        HotkeyStatus::Active(shortcut) => format!("Or press {shortcut} anywhere. Drag to record an area; click to record the whole screen."),
        HotkeyStatus::Pending => "Drag to record an area; click to record the whole screen.".into(),
        HotkeyStatus::Unavailable => {
            "Drag to record an area; click to record the whole screen. For a keyboard shortcut, bind the command \
             \"cappa --toggle\" in your system's keyboard settings."
                .into()
        }
    }
}
