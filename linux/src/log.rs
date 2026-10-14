//! A plain text log at `~/.local/state/cappa/cappa.log` (rolled over at 2 MB), so problems on a machine
//! nobody is debugging can still be diagnosed. "Open Log" in the tray menu shows it.

use std::fmt::Display;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use gtk::glib;

static GATE: Mutex<()> = Mutex::new(());

pub fn file_path() -> PathBuf {
    crate::paths::state_dir().join("cappa.log")
}

pub fn write(message: impl AsRef<str>) {
    let Ok(_guard) = GATE.lock() else { return };
    let path = file_path();
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 2_000_000) {
        let _ = std::fs::rename(&path, path.with_extension("log.old"));
    }
    let time = glib::DateTime::now_local()
        .and_then(|now| now.format("%Y-%m-%d %H:%M:%S%f"))
        .map(|s| s.to_string())
        .unwrap_or_default();
    // %f gives microseconds; milliseconds are plenty.
    let time = time.get(..time.len().saturating_sub(3)).unwrap_or(&time).to_owned();
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        // Logging must never take the app down.
        let _ = writeln!(file, "{time}  {}", message.as_ref());
    }
    if cfg!(debug_assertions) {
        eprintln!("{}", message.as_ref());
    }
}

pub fn error(context: &str, error: impl Display) {
    write(format!("ERROR {context}: {error:#}"));
}
