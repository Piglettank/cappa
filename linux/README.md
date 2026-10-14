# Cappa for Linux

The Linux version of Cappa: a Rust app with a GTK 4 / libadwaita UI and a tray icon. It has the same features as
the macOS and Windows apps (see `../docs/spec.md`), with the differences listed below.

> **Status: written on a Mac, never compiled or run.** Expect a round of compile fixes on the first build. If
> something fails at runtime, check the log (tray menu → **Open Log**, or `~/.local/state/cappa/cappa.log`).

## Requirements
- A recent distribution: **Ubuntu 24.04+**, **Fedora 40+**, Debian 13, Arch… (GTK 4.12+, libadwaita 1.4+,
  GStreamer 1.22+)
- X11 or Wayland (GNOME, KDE Plasma, Sway, Hyprland…)
- Rust 1.85 or later, from [rustup](https://rustup.rs) (distribution packages are often older)
- FFmpeg (`ffmpeg` and `ffprobe` on `PATH`)

**Ubuntu / Debian:**
```sh
sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
    libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
    gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly \
    gstreamer1.0-pipewire gstreamer1.0-pulseaudio gstreamer1.0-x gstreamer1.0-gl ffmpeg
```

**Fedora:**
```sh
sudo dnf install gcc pkgconf-pkg-config gtk4-devel libadwaita-devel \
    gstreamer1-devel gstreamer1-plugins-base-devel \
    gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-plugin-openh264 \
    pipewire-gstreamer ffmpeg-free
```
For x264 (better quality per file size than OpenH264), enable RPM Fusion and install `gstreamer1-plugins-ugly`.

## Build and run
```sh
cd linux
cargo run                        # debug build; logs also go to the terminal
./install.sh                     # release build, installed to ~/.local/bin with a menu entry
./install.sh --autostart         # …and start Cappa when you log in (also a switch in Settings)
./install.sh --uninstall
```

To place the recording controls and region outline on KDE Plasma, Sway, Hyprland and other wlroots-style
Wayland compositors, install `gtk4-layer-shell` (development package) and build with
`cargo build --release --features layer-shell`.

## Using it
- Cappa runs in the **tray**. Stock GNOME has no tray; install the *AppIndicator and KStatusNotifierItem
  Support* extension (Ubuntu ships it), or use Cappa without one: launching Cappa from the app menu opens its
  settings, which has a **Record…** button.
- **Record…**, **Ctrl+Alt+Shift+R** or `cappa --toggle`: drag the area to record (hold Shift for a square). A
  click records the whole monitor, and Esc cancels. The same shortcut or command stops the recording.
- **Wayland:** the first recording asks which screens to share (choose all of them, or the one you record).
  Cappa remembers the choice, so later recordings start straight away. The shortcut is registered through the
  desktop (KDE, GNOME 48+, Hyprland), which may ask you to confirm it; where that isn't supported, bind
  `cappa --toggle` to a shortcut in your keyboard settings.
- While recording: a pill with the timer, **Pause** and **Stop** sits next to the recorded area, with a red
  dashed outline around it (see below for when they're shown). Pause and Stop are also in the tray menu, and
  GNOME's own screen-sharing indicator stops the recording too.
- The video editor opens when you stop: trim by dragging clip edges, **Ctrl+B** splits at the playhead,
  **Delete** removes a clip, and **Export** saves as video (Ctrl+E), copies to the clipboard (Ctrl+Shift+C) or
  saves as a GIF (Ctrl+Shift+E).
- Command line: `cappa [--toggle | --settings | --quit] [video…]`. Videos open in the editor.

## How it's built
| Piece | Library / API | Files |
|---|---|---|
| App, single instance, command line | `adw::Application` | `src/main.rs` |
| Tray icon, menu | StatusNotifierItem via [ksni](https://crates.io/crates/ksni) | `src/ui/tray.rs` |
| Hotkey | X11 `XGrabKey` ([x11rb](https://crates.io/crates/x11rb)); Wayland GlobalShortcuts portal ([ashpd](https://crates.io/crates/ashpd)) | `src/ui/hotkey.rs` |
| Region selector | One full-screen, see-through GTK window per monitor | `src/ui/region_selector.rs` |
| Controls, outline | GTK windows placed via X11 or wlr-layer-shell | `src/ui/overlay.rs`, `src/ui/region_frame.rs`, `src/ui/placement.rs` |
| Screen capture | GStreamer: `ximagesrc` on X11, ScreenCast portal + `pipewiresrc` on Wayland | `src/recording/pipeline.rs`, `src/recording/portal.rs` |
| Encoding | GStreamer `x264enc`, VA-API `vah264enc` or `openh264enc`, whichever is installed | `src/recording/pipeline.rs` |
| System audio / mic | GStreamer `pulsesrc` (PulseAudio or PipeWire): the default output's monitor, and the chosen source | `src/recording/pipeline.rs`, `src/recording/audio_devices.rs` |
| Click highlights (X11) | Pointer polling + `cairooverlay` drawing into the video | `src/recording/click_tracker.rs` |
| Pause / final recording | One Matroska part per stretch between pauses; FFmpeg joins them into the MP4 with titled AAC tracks | `src/recording/session.rs` |
| Editor preview | One GStreamer pipeline: `decodebin` → GTK paintable sink, and a `volume` per audio track into `audiomixer` | `src/editor/playback.rs` |
| Timeline | Custom-drawn GTK `DrawingArea` (cairo) | `src/editor/timeline.rs` |
| Export, GIF, thumbnails, waveforms | FFmpeg | `src/editor/export.rs`, `src/editor/media.rs` |
| Settings | JSON in `~/.config/cappa/settings.json` | `src/settings.rs` |
| Log | `~/.local/state/cappa/cappa.log` | `src/log.rs` |

**Differences from the macOS and Windows apps**
- Linux can't keep a window out of a screen capture. So the recording controls and the red outline only appear
  where they fit **outside** the recorded area, and only where windows can be placed precisely: always on X11,
  and on Wayland with the `layer-shell` build on compositors that support it. They're never shown for
  full-screen recordings or on GNOME's Wayland session. Use the tray, the shortcut or GNOME's screen-sharing
  indicator instead.
- **Highlight clicks** only works on X11: Wayland doesn't let apps see clicks in other windows. The rings are
  drawn into the video rather than on screen.
- On Wayland the desktop asks once which screens Cappa may record (the ScreenCast portal).
- Export uses libx264 if the installed FFmpeg has it, otherwise OpenH264 (Fedora's `ffmpeg-free`).

## Troubleshooting
- **"The GStreamer element … is missing"**: install the package the message names (see Requirements).
- **No tray icon on GNOME**: install the AppIndicator extension, or launch Cappa from the app menu to get the
  settings window with its Record button.
- **Wayland: "Cappa can only record screens you share"**: the region is on a screen that wasn't shared. Cappa
  asks again; choose that screen (or all screens).
- **The microphone records silence**: check the input in your sound settings, and pick it under Settings →
  Microphone.
- Anything else: tray menu → **Open Log**, and look at the last lines.
