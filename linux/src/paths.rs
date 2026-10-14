//! Where Cappa keeps its files, following the XDG base directories.

use std::path::PathBuf;

use gtk::glib;

/// `~/.config/cappa`: settings.
pub fn config_dir() -> PathBuf {
    create(glib::user_config_dir().join("cappa"))
}

/// `~/.local/state/cappa`: the log.
pub fn state_dir() -> PathBuf {
    create(glib::user_state_dir().join("cappa"))
}

/// `~/.cache/cappa`: temporary recording parts, editor caches and clipboard copies.
pub fn cache_dir() -> PathBuf {
    create(glib::user_cache_dir().join("cappa"))
}

/// A fresh folder under the cache for one recording or editor session.
pub fn temp_dir(name: &str) -> PathBuf {
    create(cache_dir().join("temp").join(name))
}

/// A short random name for temporary folders.
pub fn unique_name(prefix: &str) -> String {
    format!("{prefix}-{}", glib::uuid_string_random())
}

/// `~/Videos/Cappa` (or the localized Videos folder).
pub fn default_recordings_dir() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Videos)
        .unwrap_or_else(|| glib::home_dir().join("Videos"))
        .join("Cappa")
}

fn create(path: PathBuf) -> PathBuf {
    let _ = std::fs::create_dir_all(&path);
    path
}
