//! Whether Cappa starts when you log in, via an XDG autostart entry in `~/.config/autostart`. That file is
//! the source of truth (`install.sh --autostart` and desktop settings apps write it too), so it isn't
//! kept in settings.json.

use std::path::PathBuf;

use gtk::glib;

fn entry_path() -> PathBuf {
    glib::user_config_dir().join("autostart").join(format!("{}.desktop", crate::APP_ID))
}

pub fn is_enabled() -> bool {
    let Ok(entry) = std::fs::read_to_string(entry_path()) else {
        return false;
    };
    // An entry can be kept but switched off with `Hidden=true` or `X-GNOME-Autostart-enabled=false`.
    !entry.lines().any(|line| {
        let line = line.trim();
        line.eq_ignore_ascii_case("Hidden=true") || line.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
    })
}

pub fn set_enabled(enabled: bool) {
    let path = entry_path();
    let result = if enabled {
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("cappa"));
        let entry = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Cappa\n\
             Comment=Record your screen and trim the result\n\
             Exec=\"{}\"\n\
             Icon={}\n\
             Terminal=false\n\
             StartupNotify=false\n",
            exe.display(),
            crate::APP_ID,
        );
        path.parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, entry))
    } else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    };
    if let Err(e) = result {
        crate::log::error("Changing autostart", e);
    }
}
