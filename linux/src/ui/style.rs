//! App-wide CSS for Cappa's custom windows.

use gtk::gdk;

const CSS: &str = r#"
window.cappa-transparent,
window.cappa-transparent > contents {
    background: none;
    box-shadow: none;
}

.cappa-pill {
    background-color: rgba(28, 28, 30, 0.9);
    border-radius: 18px;
    padding: 4px 6px 4px 12px;
    color: white;
}

.cappa-pill button {
    min-width: 28px;
    min-height: 28px;
    padding: 0;
    border-radius: 14px;
    color: white;
    background: none;
    box-shadow: none;
}

.cappa-pill button:hover {
    background-color: rgba(255, 255, 255, 0.15);
}

.cappa-dot {
    color: #ff3b30;
    font-size: 11px;
}

.cappa-dot.paused {
    color: #ff9f0a;
}

.cappa-time {
    font-family: monospace;
    font-weight: 500;
    min-width: 48px;
}
"#;

pub fn load() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(CSS);
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    }
}
