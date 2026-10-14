//! The tray icon (a StatusNotifierItem, shown by KDE, Ubuntu's GNOME, Cinnamon, XFCE… and by stock GNOME
//! with the AppIndicator extension). Its menu mirrors the macOS and Windows menus. The tray service runs
//! on its own thread, so menu actions travel back to the UI thread as commands.

use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::MenuItem;

use crate::recording::{Controller, RecordingState};

#[derive(Clone, Copy, Debug)]
enum Command {
    Record,
    CancelSelection,
    Stop,
    TogglePause,
    SetSystemAudio(bool),
    SetMicrophone(bool),
    SetShowOverlay(bool),
    Settings,
    OpenVideo,
    OpenFolder,
    OpenLog,
    Quit,
}

struct Tray {
    state: RecordingState,
    record_system_audio: bool,
    record_microphone: bool,
    show_overlay: bool,
    hotkey: Option<String>,
    icon: Vec<ksni::Icon>,
    commands: async_channel::Sender<Command>,
}

impl Tray {
    fn send(&self, command: Command) {
        let _ = self.commands.send_blocking(command);
    }

    fn item(label: &str, command: Command) -> MenuItem<Self> {
        StandardItem { label: label.into(), activate: Box::new(move |tray: &mut Self| tray.send(command)), ..Default::default() }
            .into()
    }

    fn disabled(label: &str) -> MenuItem<Self> {
        StandardItem { label: label.into(), enabled: false, ..Default::default() }.into()
    }

    fn toggle(label: &str, checked: bool, enabled: bool, command: fn(bool) -> Command) -> MenuItem<Self> {
        CheckmarkItem {
            label: label.into(),
            checked,
            enabled,
            activate: Box::new(move |tray: &mut Self| tray.send(command(!checked))),
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for Tray {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        "dev.fallberg.Cappa".into()
    }

    fn title(&self) -> String {
        if self.state.is_active() { "Cappa — recording".into() } else { "Cappa".into() }
    }

    fn icon_name(&self) -> String {
        // A red record icon while recording; otherwise the app icon (as pixels, so it works uninstalled).
        if self.state.is_active() { "media-record".into() } else { String::new() }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        if self.state.is_active() { Vec::new() } else { self.icon.clone() }
    }

    fn status(&self) -> ksni::Status {
        if self.state.is_active() { ksni::Status::NeedsAttention } else { ksni::Status::Active }
    }

    fn attention_icon_name(&self) -> String {
        "media-record".into()
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let mut menu = Vec::new();
        match self.state {
            RecordingState::Idle => {
                let label = match &self.hotkey {
                    Some(shortcut) => format!("Record…    {shortcut}"),
                    None => "Record…".into(),
                };
                menu.push(Self::item(&label, Command::Record));
            }
            RecordingState::Selecting => menu.push(Self::item("Cancel Selection", Command::CancelSelection)),
            RecordingState::Recording | RecordingState::Paused => {
                menu.push(Self::item("Stop Recording", Command::Stop));
                let pause = if self.state == RecordingState::Paused { "Resume" } else { "Pause" };
                menu.push(Self::item(pause, Command::TogglePause));
            }
            RecordingState::Starting => menu.push(Self::disabled("Starting…")),
            RecordingState::Stopping => menu.push(Self::disabled("Saving…")),
        }

        let idle = self.state == RecordingState::Idle;
        menu.push(MenuItem::Separator);
        menu.push(Self::toggle("Record System Audio", self.record_system_audio, idle, Command::SetSystemAudio));
        menu.push(Self::toggle("Record Microphone", self.record_microphone, idle, Command::SetMicrophone));
        menu.push(Self::toggle("Show Controls While Recording", self.show_overlay, true, Command::SetShowOverlay));
        menu.push(MenuItem::Separator);
        menu.push(Self::item("Settings…", Command::Settings));
        menu.push(Self::item("Open Video in Editor…", Command::OpenVideo));
        menu.push(Self::item("Open Recordings Folder", Command::OpenFolder));
        menu.push(Self::item("Open Log", Command::OpenLog));
        menu.push(MenuItem::Separator);
        menu.push(Self::item("Quit Cappa", Command::Quit));
        menu
    }
}

/// Keeps the tray icon alive.
pub struct TrayIcon {
    _handle: Rc<ksni::blocking::Handle<Tray>>,
}

/// Starts the tray icon. `None` if the desktop has no tray (stock GNOME without the AppIndicator
/// extension); Cappa then shows its settings window, with a Record button, when launched.
pub fn start(
    controller: &Rc<Controller>,
    hotkey: Option<String>,
    on_settings: impl Fn() + 'static,
    on_quit: impl Fn() + 'static,
) -> Option<TrayIcon> {
    let settings = controller.settings.get();
    let (commands, receiver) = async_channel::unbounded();
    let tray = Tray {
        state: controller.state(),
        record_system_audio: settings.record_system_audio,
        record_microphone: settings.record_microphone,
        show_overlay: settings.show_overlay,
        hotkey,
        icon: app_icon(),
        commands,
    };
    let handle = match tray.spawn() {
        Ok(handle) => Rc::new(handle),
        Err(e) => {
            crate::log::error("Starting the tray icon", e);
            return None;
        }
    };

    // Keep the menu in step with the recording state and settings.
    let weak = Rc::downgrade(&handle);
    controller.connect_state_changed(move |state| {
        if let Some(handle) = weak.upgrade() {
            handle.update(|tray: &mut Tray| tray.state = state);
        }
    });
    let weak = Rc::downgrade(&handle);
    controller.settings.connect_changed(move |settings| {
        if let Some(handle) = weak.upgrade() {
            let settings = settings.clone();
            handle.update(move |tray: &mut Tray| {
                tray.record_system_audio = settings.record_system_audio;
                tray.record_microphone = settings.record_microphone;
                tray.show_overlay = settings.show_overlay;
            });
        }
    });

    let controller = controller.clone();
    crate::util::spawn(async move {
        while let Ok(command) = receiver.recv().await {
            match command {
                Command::Record => controller.start_recording(),
                Command::CancelSelection => controller.cancel_selection(),
                Command::Stop => controller.stop(),
                Command::TogglePause => controller.toggle_pause(),
                Command::SetSystemAudio(on) => controller.settings.update(|s| s.record_system_audio = on),
                Command::SetMicrophone(on) => controller.settings.update(|s| s.record_microphone = on),
                Command::SetShowOverlay(on) => controller.settings.update(|s| s.show_overlay = on),
                Command::Settings => on_settings(),
                Command::OpenVideo => controller.open_video_in_editor(),
                Command::OpenFolder => controller.open_recordings_folder(),
                Command::OpenLog => crate::util::open_path(&crate::log::file_path()),
                Command::Quit => on_quit(),
            }
        }
    });
    Some(TrayIcon { _handle: handle })
}

/// The app icon as ARGB32 pixels in network byte order, as the tray protocol wants them.
fn app_icon() -> Vec<ksni::Icon> {
    let bytes = gtk::glib::Bytes::from_static(include_bytes!("../../data/icons/hicolor/64x64/apps/dev.fallberg.Cappa.png"));
    let Ok(texture) = gdk::Texture::from_bytes(&bytes) else { return Vec::new() };
    let (width, height) = (texture.width(), texture.height());
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    // Native-endian premultiplied ARGB32: B, G, R, A on little-endian machines.
    texture.download(&mut pixels, (width * 4) as usize);
    for pixel in pixels.chunks_exact_mut(4) {
        let [b, g, r, a] = [pixel[0], pixel[1], pixel[2], pixel[3]];
        let unpremultiply = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
        pixel.copy_from_slice(&[a, unpremultiply(r), unpremultiply(g), unpremultiply(b)]);
    }
    vec![ksni::Icon { width, height, data: pixels }]
}
