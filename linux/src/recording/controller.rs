//! App-level recording state shared by the tray menu, the hotkey and the recording controls. UI thread only.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use anyhow::Result;
use gtk::prelude::*;
use gtk::{gdk, glib};

use super::portal::{self, OpenError};
use super::session::{Session, Source};
use super::{is_x11, CaptureTarget, RecordingState};
use crate::settings::SettingsStore;
use crate::ui::overlay::RecordingOverlay;
use crate::ui::region_frame::RegionFrame;
use crate::ui::region_selector::RegionSelector;

pub struct Controller {
    app: adw::Application,
    pub settings: SettingsStore,
    state: Cell<RecordingState>,
    selector: Rc<RegionSelector>,
    session: RefCell<Option<Session>>,
    /// Time recorded before the current stretch, and when that stretch started.
    elapsed: Cell<(Duration, Option<Instant>)>,
    overlay: RefCell<Option<RecordingOverlay>>,
    frame: RefCell<Option<RegionFrame>>,
    timer: RefCell<Option<glib::SourceId>>,
    listeners: RefCell<Vec<Box<dyn Fn(RecordingState)>>>,
}

impl Controller {
    pub fn new(app: &adw::Application, settings: SettingsStore) -> Rc<Self> {
        let controller = Rc::new(Self {
            app: app.clone(),
            settings,
            state: Cell::new(RecordingState::Idle),
            selector: Rc::default(),
            session: RefCell::default(),
            elapsed: Cell::new((Duration::ZERO, None)),
            overlay: RefCell::default(),
            frame: RefCell::default(),
            timer: RefCell::default(),
            listeners: RefCell::default(),
        });
        let weak = Rc::downgrade(&controller);
        controller.settings.connect_changed(move |settings| {
            // Turning the controls off mid-recording hides them; turning them on shows them next time.
            if let Some(controller) = weak.upgrade() {
                if !settings.show_overlay {
                    if let Some(overlay) = controller.overlay.take() {
                        overlay.close();
                    }
                }
            }
        });
        controller
    }

    pub fn state(&self) -> RecordingState {
        self.state.get()
    }

    pub fn elapsed(&self) -> Duration {
        let (before, since) = self.elapsed.get();
        before + since.map(|s| s.elapsed()).unwrap_or_default()
    }

    /// Called on every state change (on the UI thread).
    pub fn connect_state_changed(&self, listener: impl Fn(RecordingState) + 'static) {
        self.listeners.borrow_mut().push(Box::new(listener));
    }

    /// Hotkey behaviour: start a recording, cancel an in-progress selection, or stop.
    pub fn toggle(self: &Rc<Self>) {
        match self.state() {
            RecordingState::Idle => self.start_recording(),
            RecordingState::Selecting => self.selector.cancel(),
            RecordingState::Recording | RecordingState::Paused => self.stop(),
            RecordingState::Starting | RecordingState::Stopping => {}
        }
    }

    pub fn cancel_selection(&self) {
        self.selector.cancel();
    }

    /// Shows the region selector, then records the chosen area (or the whole screen after a click).
    pub fn start_recording(self: &Rc<Self>) {
        if self.state() != RecordingState::Idle {
            return;
        }
        let this = self.clone();
        crate::util::spawn(async move {
            this.set_state(RecordingState::Selecting);
            let selection = this.selector.select().await;
            this.set_state(RecordingState::Idle);
            let Some(selection) = selection else { return };
            let target = CaptureTarget::new(selection.monitor, selection.region);

            this.set_state(RecordingState::Starting);
            // Give the compositor a moment to take the dimmed selector off screen before capturing.
            glib::timeout_future(Duration::from_millis(250)).await;
            match this.begin(target.clone()).await {
                Ok(session) => {
                    *this.session.borrow_mut() = Some(session);
                    this.elapsed.set((Duration::ZERO, Some(Instant::now())));
                    this.set_state(RecordingState::Recording);
                    this.show_recording_windows(&target);
                    this.start_timer();
                }
                Err(Some(e)) => {
                    crate::log::error("Starting recording", &e);
                    this.set_state(RecordingState::Idle);
                    crate::util::show_message(None, "Couldn't start recording", &format!("{e:#}"));
                }
                Err(None) => this.set_state(RecordingState::Idle), // cancelled in the portal dialog
            }
        });
    }

    /// `Err(None)` means the user cancelled.
    async fn begin(self: &Rc<Self>, target: CaptureTarget) -> std::result::Result<Session, Option<anyhow::Error>> {
        let settings = self.settings.get();
        let source = if is_x11() { Source::X11 } else { Source::Portal(self.open_portal(&target).await?) };
        let weak = Rc::downgrade(self);
        let on_ended = Rc::new(move |error: Option<String>| {
            if let Some(controller) = weak.upgrade() {
                controller.capture_ended(error);
            }
        });
        Session::start(target, settings, source, on_ended).await.map_err(Some)
    }

    /// Opens the screen-sharing portal, asking again if the remembered choice doesn't include this monitor.
    async fn open_portal(&self, target: &CaptureTarget) -> std::result::Result<portal::PortalCapture, Option<anyhow::Error>> {
        let monitor_count = gdk::Display::default().map(|d| d.monitors().n_items() as usize).unwrap_or(1);
        let show_cursor = self.settings.get().show_cursor;
        let mut token = self.settings.get().screencast_restore_token;
        loop {
            match portal::open(target.monitor_geometry, monitor_count, show_cursor, token.as_deref()).await {
                Ok((capture, new_token)) => {
                    self.settings.update(|s| s.screencast_restore_token = new_token);
                    return Ok(capture);
                }
                Err(OpenError::WrongScreen) if token.is_some() => {
                    crate::log::write("The remembered screen choice doesn't include this monitor; asking again");
                    token = None;
                }
                Err(OpenError::WrongScreen) => {
                    return Err(Some(anyhow::anyhow!(
                        "Cappa can only record screens you share. Choose this screen (or all screens) when asked."
                    )));
                }
                Err(OpenError::Cancelled) => return Err(None),
                Err(OpenError::Failed(e)) => return Err(Some(e)),
            }
        }
    }

    pub fn toggle_pause(self: &Rc<Self>) {
        let this = self.clone();
        crate::util::spawn(async move {
            match this.state() {
                RecordingState::Recording => {
                    let (before, since) = this.elapsed.get();
                    this.elapsed.set((before + since.map(|s| s.elapsed()).unwrap_or_default(), None));
                    this.set_state(RecordingState::Paused);
                    if let Some(mut session) = this.session.take() {
                        session.pause().await;
                        *this.session.borrow_mut() = Some(session);
                    }
                }
                RecordingState::Paused => {
                    let Some(mut session) = this.session.take() else { return };
                    let resumed = session.resume().await;
                    *this.session.borrow_mut() = Some(session);
                    match resumed {
                        Ok(()) => {
                            let (before, _) = this.elapsed.get();
                            this.elapsed.set((before, Some(Instant::now())));
                            this.set_state(RecordingState::Recording);
                        }
                        Err(e) => {
                            crate::log::error("Resuming recording", &e);
                            crate::util::show_message(None, "Couldn't resume recording", &format!("{e:#}"));
                        }
                    }
                }
                _ => {}
            }
            this.update_overlay();
        });
    }

    /// Stops and saves the recording, then opens it in the editor.
    pub fn stop(self: &Rc<Self>) {
        let this = self.clone();
        crate::util::spawn(async move {
            this.finish().await;
        });
    }

    /// Stops (if recording) and waits until the recording is saved. Used before quitting too.
    pub async fn finish(self: &Rc<Self>) {
        if !self.state().is_active() {
            return;
        }
        let Some(session) = self.session.take() else { return };
        self.set_state(RecordingState::Stopping);
        self.close_recording_windows();

        let folder = self.settings.get().recordings_folder;
        let output = output_path(&folder);
        let result: Result<()> = async {
            std::fs::create_dir_all(&folder)?;
            session.stop(&output).await
        }
        .await;
        self.set_state(RecordingState::Idle);
        match result {
            Ok(()) => {
                crate::log::write(format!("Saved {}", output.display()));
                crate::editor::open(&self.app, &output);
            }
            Err(e) => {
                crate::log::error("Saving recording", &e);
                crate::util::show_message(None, "Couldn't save the recording", &format!("{e:#}"));
            }
        }
    }

    /// The capture stopped on its own: an error, or screen sharing ended from the desktop's indicator.
    fn capture_ended(self: &Rc<Self>, error: Option<String>) {
        if !self.state().is_active() {
            return;
        }
        if let Some(error) = error {
            crate::util::show_message(None, "Recording stopped unexpectedly", &error);
        }
        self.stop();
    }

    pub fn open_recordings_folder(&self) {
        let folder = self.settings.get().recordings_folder;
        let _ = std::fs::create_dir_all(&folder);
        crate::util::open_path(&folder);
    }

    pub fn open_video_in_editor(&self) {
        let app = self.app.clone();
        let folder = self.settings.get().recordings_folder;
        crate::util::spawn(async move {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Videos"));
            filter.add_mime_type("video/*");
            let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::builder()
                .title("Open Video in Cappa")
                .modal(true)
                .filters(&filters)
                .initial_folder(&gtk::gio::File::for_path(&folder))
                .build();
            if let Ok(file) = dialog.open_future(gtk::Window::NONE).await {
                if let Some(path) = file.path() {
                    crate::editor::open(&app, &path);
                }
            }
        });
    }

    fn set_state(&self, state: RecordingState) {
        self.state.set(state);
        for listener in self.listeners.borrow().iter() {
            listener(state);
        }
    }

    fn show_recording_windows(self: &Rc<Self>, target: &CaptureTarget) {
        *self.frame.borrow_mut() = RegionFrame::new(target);
        if self.settings.get().show_overlay {
            let (pause, stop) = (Rc::downgrade(self), Rc::downgrade(self));
            *self.overlay.borrow_mut() = RecordingOverlay::new(
                target,
                move || with(&pause, |c| c.toggle_pause()),
                move || with(&stop, |c| c.stop()),
            );
        }
        self.update_overlay();
    }

    fn close_recording_windows(&self) {
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
        if let Some(overlay) = self.overlay.take() {
            overlay.close();
        }
        if let Some(frame) = self.frame.take() {
            frame.close();
        }
    }

    fn start_timer(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let timer = glib::timeout_add_local(Duration::from_millis(250), move || {
            let Some(controller) = weak.upgrade() else { return glib::ControlFlow::Break };
            controller.update_overlay();
            glib::ControlFlow::Continue
        });
        *self.timer.borrow_mut() = Some(timer);
    }

    fn update_overlay(&self) {
        if let Some(overlay) = self.overlay.borrow().as_ref() {
            overlay.update(self.state(), self.elapsed());
        }
    }
}

fn with(controller: &Weak<Controller>, action: impl FnOnce(&Rc<Controller>)) {
    if let Some(controller) = controller.upgrade() {
        action(&controller);
    }
}

fn output_path(folder: &std::path::Path) -> PathBuf {
    let stamp = glib::DateTime::now_local()
        .and_then(|now| now.format("%Y-%m-%d at %H.%M.%S"))
        .map(|s| s.to_string())
        .unwrap_or_else(|_| "recording".into());
    folder.join(format!("Cappa {stamp}.mp4"))
}
