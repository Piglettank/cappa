//! User settings, saved as JSON in `~/.config/cappa/settings.json` whenever one changes.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde::{Deserialize, Serialize};

/// Video quality for new recordings: trades file size for sharpness.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecordingQuality {
    Low,
    Medium,
    #[default]
    High,
}

impl RecordingQuality {
    pub const ALL: [RecordingQuality; 3] = [Self::Low, Self::Medium, Self::High];

    pub fn title(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }

    /// Average bitrate in bits per second, for encoders that take one (the same formula as on macOS).
    pub fn bit_rate(self, width: u32, height: u32, frame_rate: u32) -> u64 {
        let bits_per_pixel: u64 = match self {
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 4,
        };
        // Tuned at 60 fps; fewer frames need proportionally fewer bits.
        width as u64 * height as u64 * bits_per_pixel * frame_rate as u64 / 60
    }

    /// x264's constant-quality factor (lower is better).
    pub fn x264_quantizer(self) -> u32 {
        match self {
            Self::Low => 30,
            Self::Medium => 25,
            Self::High => 20,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub record_system_audio: bool,
    pub record_microphone: bool,
    /// The PulseAudio/PipeWire source name of the chosen microphone; empty means the default input.
    pub microphone_id: String,
    pub recordings_folder: PathBuf,
    pub show_overlay: bool,
    pub quality: RecordingQuality,
    /// 30 or 60.
    pub frame_rate: u32,
    pub show_cursor: bool,
    pub highlight_clicks: bool,
    /// Lets the Wayland screen-sharing portal reuse the last choice of screens without asking again.
    pub screencast_restore_token: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            record_system_audio: true,
            record_microphone: false,
            microphone_id: String::new(),
            recordings_folder: crate::paths::default_recordings_dir(),
            show_overlay: true,
            quality: RecordingQuality::High,
            frame_rate: 60,
            show_cursor: true,
            highlight_clicks: false,
            screencast_restore_token: None,
        }
    }
}

type Listener = Box<dyn Fn(&Settings)>;

/// Shared, observable settings. UI thread only.
#[derive(Clone)]
pub struct SettingsStore(Rc<Inner>);

struct Inner {
    settings: RefCell<Settings>,
    listeners: RefCell<Vec<Listener>>,
}

impl SettingsStore {
    pub fn load() -> Self {
        let mut settings = std::fs::read_to_string(file_path())
            .ok()
            .and_then(|json| match serde_json::from_str::<Settings>(&json) {
                Ok(settings) => Some(settings),
                Err(e) => {
                    crate::log::error("Loading settings", e);
                    None
                }
            })
            .unwrap_or_default();
        if settings.recordings_folder.as_os_str().is_empty() {
            settings.recordings_folder = crate::paths::default_recordings_dir();
        }
        if settings.frame_rate != 30 {
            settings.frame_rate = 60;
        }
        Self(Rc::new(Inner { settings: RefCell::new(settings), listeners: RefCell::default() }))
    }

    /// A snapshot of the current settings.
    pub fn get(&self) -> Settings {
        self.0.settings.borrow().clone()
    }

    /// Changes settings, saves them and tells every listener (if anything changed).
    pub fn update(&self, change: impl FnOnce(&mut Settings)) {
        let snapshot = {
            let mut settings = self.0.settings.borrow_mut();
            let before = settings.clone();
            change(&mut settings);
            if *settings == before {
                return;
            }
            settings.clone()
        };
        save(&snapshot);
        for listener in self.0.listeners.borrow().iter() {
            listener(&snapshot);
        }
    }

    pub fn connect_changed(&self, listener: impl Fn(&Settings) + 'static) {
        self.0.listeners.borrow_mut().push(Box::new(listener));
    }
}

fn file_path() -> PathBuf {
    crate::paths::config_dir().join("settings.json")
}

fn save(settings: &Settings) {
    let result = serde_json::to_string_pretty(settings)
        .map_err(anyhow::Error::from)
        .and_then(|json| std::fs::write(file_path(), json).map_err(anyhow::Error::from));
    if let Err(e) = result {
        crate::log::error("Saving settings", e);
    }
}
