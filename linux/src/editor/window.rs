//! One editor window per video: preview, toolbar, and a timeline with a video lane and one lane per audio
//! track. Split/trim/delete apply to all lanes; each audio track has its own mute and volume.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use adw::prelude::*;
use anyhow::Result;
use gtk::{gdk, gio, glib};

use super::export::{self, ExportAudio};
use super::media;
use super::model::{format_precise, EditorModel};
use super::playback::Playback;
use super::timeline::{self, Timeline};
use crate::{ffmpeg, paths, util};

pub struct Editor {
    pub window: adw::ApplicationWindow,
    model: Rc<EditorModel>,
    playback: RefCell<Option<Rc<Playback>>>,
    timeline: RefCell<Option<Rc<Timeline>>>,
    work_folder: PathBuf,
    toasts: adw::ToastOverlay,
    picture: gtk::Picture,
    play_button: gtk::Button,
    time_label: gtk::Label,
    length_label: gtk::Label,
    timeline_slot: gtk::Box,
    headers: gtk::Box,
    busy_overlay: gtk::Box,
    busy_label: gtk::Label,
    busy_progress: gtk::ProgressBar,
    busy: Cell<bool>,
    close_confirmed: Cell<bool>,
    on_closed: RefCell<Option<Box<dyn FnOnce()>>>,
}

impl Editor {
    pub fn new(app: &adw::Application, path: &Path, on_closed: impl FnOnce() + 'static) -> Rc<Self> {
        let model = Rc::new(EditorModel::new(path.to_owned()));
        let title = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();

        // Video, with click-to-play and toasts on top.
        let picture = gtk::Picture::builder().content_fit(gtk::ContentFit::Contain).hexpand(true).vexpand(true).build();
        let video_frame = gtk::Box::builder().css_classes(["cappa-video"]).build();
        video_frame.append(&picture);
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&video_frame));

        // Toolbar under the video: editing on the left, playback in the middle, length on the right.
        let split = tool_button("Split", "edit-cut-symbolic", "win.split", "Split at playhead (Ctrl+B)");
        let delete = tool_button("Delete Clip", "user-trash-symbolic", "win.delete", "Delete clip (Delete)");
        let edit_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        edit_buttons.append(&split);
        edit_buttons.append(&delete);
        let play_button = gtk::Button::builder()
            .icon_name("media-playback-start-symbolic")
            .action_name("win.play")
            .tooltip_text("Play / Pause (Space)")
            .css_classes(["circular"])
            .build();
        let time_label = gtk::Label::builder().label("0:00.0").width_chars(9).xalign(0.0).css_classes(["monospace", "numeric"]).build();
        let playback_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        playback_box.append(&play_button);
        playback_box.append(&time_label);
        let length_label = gtk::Label::builder().css_classes(["dim-label", "numeric"]).build();
        let toolbar = gtk::CenterBox::builder().margin_start(16).margin_end(16).margin_top(10).margin_bottom(10).build();
        toolbar.set_start_widget(Some(&edit_buttons));
        toolbar.set_center_widget(Some(&playback_box));
        toolbar.set_end_widget(Some(&length_label));

        // Timeline: lane headers on the left, lanes on the right.
        let headers = gtk::Box::builder().orientation(gtk::Orientation::Vertical).width_request(170).margin_end(10).build();
        let timeline_slot = gtk::Box::builder().hexpand(true).build();
        let timeline_row = gtk::Box::builder().margin_start(16).margin_end(16).margin_bottom(16).build();
        timeline_row.append(&headers);
        timeline_row.append(&timeline_slot);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.append(&toasts);
        content.append(&toolbar);
        content.append(&timeline_row);

        // Loading / exporting: covers the window and swallows clicks.
        let busy_label = gtk::Label::builder().css_classes(["heading"]).xalign(0.0).build();
        let busy_progress = gtk::ProgressBar::new();
        let busy_card = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .width_request(280)
            .css_classes(["card", "cappa-busy-card"])
            .build();
        busy_card.append(&busy_label);
        busy_card.append(&busy_progress);
        let busy_overlay = gtk::Box::builder().css_classes(["cappa-busy"]).visible(false).build();
        busy_overlay.append(&busy_card);
        busy_card.set_hexpand(true);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&content));
        overlay.add_overlay(&busy_overlay);

        // Header bar: undo/redo, and the export menu.
        let header = adw::HeaderBar::new();
        header.pack_start(&icon_button("edit-undo-symbolic", "win.undo", "Undo (Ctrl+Z)"));
        header.pack_start(&icon_button("edit-redo-symbolic", "win.redo", "Redo (Ctrl+Shift+Z)"));
        let export_menu = gio::Menu::new();
        export_menu.append(Some("Save as Video…"), Some("win.save-video"));
        export_menu.append(Some("Copy to Clipboard"), Some("win.copy"));
        export_menu.append(Some("Save as GIF…"), Some("win.save-gif"));
        let export_button = gtk::MenuButton::builder()
            .label("Export")
            .menu_model(&export_menu)
            .css_classes(["suggested-action"])
            .tooltip_text("Save as Video (Ctrl+E) · Copy to Clipboard (Ctrl+Shift+C) · Save as GIF (Ctrl+Shift+E)")
            .build();
        header.pack_end(&export_button);
        header.pack_end(&gtk::Button::builder().label("Show in Files").action_name("win.show-in-files").build());
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&overlay));

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title(format!("{title} — Cappa"))
            .default_width(1100)
            .default_height(760)
            .width_request(760)
            .height_request(520)
            .content(&view)
            .build();

        let work_folder = paths::temp_dir(&paths::unique_name("Editor"));
        let editor = Rc::new(Self {
            window,
            model,
            playback: RefCell::default(),
            timeline: RefCell::default(),
            work_folder,
            toasts,
            picture,
            play_button,
            time_label,
            length_label,
            timeline_slot,
            headers,
            busy_overlay,
            busy_label,
            busy_progress,
            busy: Cell::new(false),
            close_confirmed: Cell::new(false),
            on_closed: RefCell::new(Some(Box::new(on_closed))),
        });

        editor.add_actions();
        editor.add_shortcuts();

        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&editor);
        click.connect_released(move |_, _, _, _| {
            if let Some(editor) = weak.upgrade() {
                editor.toggle_play();
            }
        });
        video_frame.add_controller(click);

        let weak = Rc::downgrade(&editor);
        editor.window.connect_close_request(move |_| match weak.upgrade() {
            Some(editor) => editor.close_request(),
            None => glib::Propagation::Proceed,
        });

        let weak = Rc::downgrade(&editor);
        editor.model.connect_changed(move || {
            if let Some(editor) = weak.upgrade() {
                editor.update_controls();
            }
        });

        editor.window.present();
        let loading = editor.clone();
        util::spawn(async move { loading.load().await });
        editor
    }

    pub fn source_path(&self) -> &Path {
        &self.model.source_path
    }

    // MARK: - Loading

    async fn load(self: Rc<Self>) {
        self.set_busy("Loading…", None);
        let path = self.model.source_path.clone();
        let opened: Result<Rc<Playback>> = async {
            let info = ffmpeg::probe(&path).await?;
            self.model.load(info);
            Playback::new(self.model.clone(), &path)
        }
        .await;
        let playback = match opened {
            Ok(playback) => playback,
            Err(e) => {
                crate::log::error("Opening video in editor", &e);
                self.clear_busy();
                util::show_message(Some(self.window.upcast_ref::<gtk::Window>()), "Cappa couldn't open this video", &format!("{e:#}"));
                self.close_confirmed.set(true);
                self.window.close();
                return;
            }
        };

        self.picture.set_paintable(Some(playback.paintable()));
        let timeline = Timeline::new(self.model.clone(), playback.clone());
        self.timeline_slot.append(timeline.widget());
        let weak = Rc::downgrade(&self);
        playback.connect_updated(move || {
            if let Some(editor) = weak.upgrade() {
                editor.update_play_state();
            }
        });
        *self.playback.borrow_mut() = Some(playback);
        *self.timeline.borrow_mut() = Some(timeline);
        self.build_lane_headers();
        self.clear_busy();
        self.update_controls();

        // Thumbnails and waveforms come in the background; the editor is usable meanwhile.
        match media::thumbnails(&path, self.model.duration(), &self.work_folder).await {
            Ok(thumbnails) => self.model.set_thumbnails(thumbnails),
            Err(e) => crate::log::error("Creating thumbnails", e),
        }
        for (index, track) in self.model.audio_tracks().iter().enumerate() {
            match media::waveform(&path, track.stream_index, self.model.duration()).await {
                Ok(levels) => self.model.set_waveform(index, levels),
                Err(e) => crate::log::error("Reading waveform", e),
            }
        }
    }

    /// The header column, aligned with the timeline's lanes.
    fn build_lane_headers(self: &Rc<Self>) {
        self.headers.append(&gtk::Box::builder().height_request((timeline::RULER_HEIGHT + timeline::LANE_SPACING) as i32).build());
        let video = gtk::Box::builder().spacing(6).height_request(timeline::VIDEO_HEIGHT as i32).build();
        video.append(&gtk::Image::from_icon_name("video-x-generic-symbolic"));
        video.append(&gtk::Label::builder().label("Video").css_classes(["dim-label", "caption"]).build());
        self.headers.append(&video);

        for (index, track) in self.model.audio_tracks().into_iter().enumerate() {
            let mute = gtk::Button::builder()
                .icon_name(track.icon_name())
                .tooltip_text("Mute / unmute")
                .valign(gtk::Align::Center)
                .css_classes(["flat", "circular"])
                .build();
            let name = gtk::Label::builder().label(track.name.as_str()).xalign(0.0).hexpand(true).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["caption"]).build();
            let level = gtk::Label::builder().label(track.volume_label()).css_classes(["dim-label", "caption", "numeric"]).build();
            let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
            scale.set_value(track.volume as f64);
            scale.set_draw_value(false);
            scale.set_focusable(false);

            let labels = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            labels.append(&name);
            labels.append(&level);
            let column = gtk::Box::builder().orientation(gtk::Orientation::Vertical).valign(gtk::Align::Center).hexpand(true).build();
            column.append(&labels);
            column.append(&scale);
            let row = gtk::Box::builder()
                .spacing(6)
                .margin_top(timeline::LANE_SPACING as i32)
                .height_request(timeline::AUDIO_HEIGHT as i32)
                .build();
            row.append(&mute);
            row.append(&column);
            self.headers.append(&row);

            // Weak references: the model's listeners hold these widgets, so strong ones would be a cycle.
            let model = Rc::downgrade(&self.model);
            let weak = model.clone();
            mute.connect_clicked(move |_| {
                if let Some(model) = weak.upgrade() {
                    model.toggle_mute(index);
                }
            });
            let weak = model.clone();
            scale.connect_value_changed(move |scale| {
                if let Some(model) = weak.upgrade() {
                    model.set_volume(index, scale.value() as f32);
                }
            });
            self.model.connect_changed(move || {
                let Some(track) = model.upgrade().and_then(|m| m.audio_tracks().into_iter().nth(index)) else { return };
                mute.set_icon_name(track.icon_name());
                level.set_label(&track.volume_label());
                if (scale.value() - track.volume as f64).abs() > f64::EPSILON {
                    scale.set_value(track.volume as f64);
                }
            });
        }
    }

    // MARK: - Actions

    fn add_actions(self: &Rc<Self>) {
        let actions: [(&str, fn(&Rc<Self>)); 9] = [
            ("undo", |e| e.model.undo()),
            ("redo", |e| e.model.redo()),
            ("split", |e| e.model.split_at(e.model.current_time())),
            ("delete", |e| e.model.delete_selected()),
            ("play", |e| e.toggle_play()),
            ("save-video", |e| e.clone().spawn_export(Self::save_video)),
            ("save-gif", |e| e.clone().spawn_export(Self::save_gif)),
            ("copy", |e| e.clone().spawn_export(Self::copy_to_clipboard)),
            ("show-in-files", |e| util::reveal_in_file_manager(&e.model.source_path)),
        ];
        for (name, run) in actions {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(editor) = weak.upgrade() {
                    // Nothing runs while loading or exporting.
                    if !editor.busy.get() {
                        run(&editor);
                    }
                }
            });
            self.window.add_action(&action);
        }
    }

    /// Shortcuts run in the capture phase, so Space plays even when a button or slider has focus.
    fn add_shortcuts(&self) {
        let controller = gtk::ShortcutController::new();
        controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        for (trigger, action) in [
            ("space", "win.play"),
            ("<Control>b", "win.split"),
            ("Delete|BackSpace", "win.delete"),
            ("<Control>z", "win.undo"),
            ("<Control><Shift>z|<Control>y", "win.redo"),
            ("<Control>e", "win.save-video"),
            ("<Control><Shift>e", "win.save-gif"),
            ("<Control><Shift>c", "win.copy"),
        ] {
            controller.add_shortcut(gtk::Shortcut::new(
                gtk::ShortcutTrigger::parse_string(trigger),
                Some(gtk::NamedAction::new(action)),
            ));
        }
        self.window.add_controller(controller);
    }

    fn action(&self, name: &str) -> Option<gio::SimpleAction> {
        self.window.lookup_action(name).and_then(|a| a.downcast::<gio::SimpleAction>().ok())
    }

    fn toggle_play(&self) {
        if let Some(playback) = self.playback.borrow().clone() {
            if !self.busy.get() {
                playback.toggle_play();
            }
        }
    }

    fn update_controls(&self) {
        let model = &self.model;
        for (name, enabled) in [
            ("undo", model.can_undo()),
            ("redo", model.can_redo()),
            ("delete", model.can_delete()),
        ] {
            if let Some(action) = self.action(name) {
                action.set_enabled(enabled);
            }
        }
        self.length_label.set_label(&format!("Length {}", format_precise(model.edited_duration())));
        self.update_play_state();
    }

    fn update_play_state(&self) {
        let playing = self.playback.borrow().as_ref().is_some_and(|p| p.is_playing());
        self.play_button.set_icon_name(if playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" });
        self.time_label.set_label(&format_precise(self.model.current_time()));
        if let Some(action) = self.action("split") {
            action.set_enabled(self.model.can_split_at(self.model.current_time()));
        }
    }

    // MARK: - Export

    fn spawn_export<F, Fut>(self: Rc<Self>, export: F)
    where
        F: FnOnce(Rc<Self>) -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        util::spawn(export(self));
    }

    async fn save_video(self: Rc<Self>) {
        let Some(path) = self.choose_destination("mp4").await else { return };
        let (source, segments, audio) = self.export_inputs();
        let target = path.clone();
        let done = self
            .run_export("Exporting video…", move |progress| async move {
                export::save_video(&source, &segments, &audio, &target, progress).await
            })
            .await;
        if done {
            self.model.mark_exported();
            util::reveal_in_file_manager(&path);
        }
    }

    async fn save_gif(self: Rc<Self>) {
        let Some(path) = self.choose_destination("gif").await else { return };
        let (source, segments, _) = self.export_inputs();
        let target = path.clone();
        let done = self
            .run_export("Creating GIF…", move |progress| async move { export::save_gif(&source, &segments, &target, progress).await })
            .await;
        if done {
            self.model.mark_exported();
            util::reveal_in_file_manager(&path);
        }
    }

    /// Puts the edited video on the clipboard as a file, ready to paste into Files, Slack, mail…
    async fn copy_to_clipboard(self: Rc<Self>) {
        let mut path = self.model.source_path.clone();
        // Videos with two audio tracks are exported (which mixes them) even without edits: many apps only play the first track.
        if self.model.has_edits() || self.model.audio_track_count() > 1 {
            let folder = paths::cache_dir().join("clipboard");
            let _ = std::fs::remove_dir_all(&folder); // only the latest copy is needed
            if let Err(e) = std::fs::create_dir_all(&folder) {
                crate::log::error("Creating the clipboard folder", e);
            }
            path = folder.join(format!("{} (edited).mp4", self.file_stem()));
            let (source, segments, audio) = self.export_inputs();
            let target = path.clone();
            let done = self
                .run_export("Preparing for clipboard…", move |progress| async move {
                    export::save_video(&source, &segments, &audio, &target, progress).await
                })
                .await;
            if !done {
                return;
            }
        }

        let file = gio::File::for_path(&path);
        let uri = file.uri();
        let providers = [
            gdk::ContentProvider::for_value(&gdk::FileList::from_array(&[file]).to_value()),
            gdk::ContentProvider::for_bytes("text/uri-list", &glib::Bytes::from_owned(format!("{uri}\r\n"))),
            gdk::ContentProvider::for_bytes("x-special/gnome-copied-files", &glib::Bytes::from_owned(format!("copy\n{uri}"))),
        ];
        match self.window.clipboard().set_content(Some(&gdk::ContentProvider::new_union(&providers))) {
            Ok(()) => self.toasts.add_toast(adw::Toast::new("Copied to clipboard")),
            Err(e) => crate::log::error("Copying to the clipboard", e),
        }
    }

    fn export_inputs(&self) -> (PathBuf, Vec<super::model::Segment>, Vec<ExportAudio>) {
        let audio = self
            .model
            .audio_tracks()
            .iter()
            .map(|t| ExportAudio { stream_index: t.stream_index, volume: t.effective_volume() })
            .collect();
        (self.model.source_path.clone(), self.model.segments(), audio)
    }

    fn file_stem(&self) -> String {
        self.model.source_path.file_stem().unwrap_or_default().to_string_lossy().into_owned()
    }

    async fn choose_destination(&self, extension: &str) -> Option<PathBuf> {
        let mut dialog = gtk::FileDialog::builder().modal(true).initial_name(format!("{} (edited).{extension}", self.file_stem()));
        if let Some(folder) = self.model.source_path.parent() {
            dialog = dialog.initial_folder(&gio::File::for_path(folder));
        }
        let path = dialog.build().save_future(Some(&self.window)).await.ok()?.path()?;

        let same = |a: &Path, b: &Path| a.canonicalize().ok().zip(b.canonicalize().ok()).is_some_and(|(a, b)| a == b) || a == b;
        if same(&path, &self.model.source_path) {
            util::show_message(
                Some(self.window.upcast_ref::<gtk::Window>()),
                "Choose a different file name",
                "Cappa keeps the original recording, so the edit can't be saved over it.",
            );
            return None;
        }
        Some(path)
    }

    /// Runs an export behind the busy overlay; returns whether it succeeded (failures are shown).
    async fn run_export<F, Fut>(&self, title: &str, export: F) -> bool
    where
        F: FnOnce(async_channel::Sender<f64>) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        if self.busy.get() {
            return false;
        }
        if let Some(playback) = self.playback.borrow().as_ref() {
            playback.pause();
        }
        self.set_busy(title, Some(0.0));
        let (sender, receiver) = async_channel::unbounded::<f64>();
        let progress = self.busy_progress.clone();
        util::spawn(async move {
            while let Ok(value) = receiver.recv().await {
                progress.set_fraction(value);
            }
        });
        let result = export(sender).await;
        self.clear_busy();
        match result {
            Ok(()) => true,
            Err(e) => {
                crate::log::error(title, &e);
                util::show_message(Some(self.window.upcast_ref::<gtk::Window>()), "Export failed", &format!("{e:#}"));
                false
            }
        }
    }

    // MARK: - Busy overlay

    fn set_busy(&self, text: &str, progress: Option<f64>) {
        self.busy.set(true);
        self.busy_label.set_label(text);
        self.busy_progress.set_visible(progress.is_some());
        self.busy_progress.set_fraction(progress.unwrap_or(0.0));
        self.busy_overlay.set_visible(true);
    }

    fn clear_busy(&self) {
        self.busy.set(false);
        self.busy_overlay.set_visible(false);
    }

    // MARK: - Closing

    fn close_request(self: &Rc<Self>) -> glib::Propagation {
        if self.busy.get() {
            return glib::Propagation::Stop;
        }
        if !self.close_confirmed.get() && self.model.has_unexported_edits() {
            let editor = self.clone();
            util::spawn(async move {
                let close = util::confirm(
                    editor.window.upcast_ref::<gtk::Window>(),
                    "Close without exporting?",
                    "Your edits haven't been exported and will be lost. The original recording is kept.",
                    "Close",
                )
                .await;
                if close {
                    editor.close_confirmed.set(true);
                    editor.window.close();
                }
            });
            return glib::Propagation::Stop;
        }

        if let Some(playback) = self.playback.borrow().as_ref() {
            playback.shutdown();
        }
        if let Err(e) = std::fs::remove_dir_all(&self.work_folder) {
            crate::log::error("Removing editor work folder", e);
        }
        if let Some(on_closed) = self.on_closed.take() {
            on_closed();
        }
        glib::Propagation::Proceed
    }
}

fn tool_button(label: &str, icon: &str, action: &str, tooltip: &str) -> gtk::Button {
    let content = adw::ButtonContent::builder().label(label).icon_name(icon).build();
    gtk::Button::builder().child(&content).action_name(action).tooltip_text(tooltip).build()
}

fn icon_button(icon: &str, action: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder().icon_name(icon).action_name(action).tooltip_text(tooltip).build()
}
