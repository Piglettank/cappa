# Cappa feature spec

All platform apps implement the same behaviour. Where they differ, the difference is called out.

## App shape
- A background app with no main window.
- **macOS:** a menu bar item (`LSUIElement`, no Dock icon).
- **Windows:** a notification-area (tray) icon on the taskbar. Optional: a Jump List "Record" task when the app is pinned to the taskbar.
- **Linux:** a tray icon (StatusNotifierItem). Without a tray (stock GNOME), launching Cappa opens its settings window, which has a **Record…** button. `cappa --toggle` starts or stops a recording from anywhere.

## Menu (menu bar / tray)
While idle:
- Record…: opens the region selector (below), same as the global hotkey

While selecting:
- Cancel Selection

While recording:
- Stop Recording
- Pause / Resume

Always:
- [x] Record System Audio (persisted, default on)
- [ ] Record Microphone (persisted, default off; macOS 15+)
- Settings… (⌘,)
- [x] Show Overlay While Recording (persisted, default on)
- Open Recordings Folder
- Quit

## Settings (⌘,)
A small window (shown in the Dock while it's open):
- **General:** **Open at login** (macOS), **Start Cappa when Windows starts** (Windows), **Start Cappa automatically on startup** (Linux); default off. The system's own login items are the source of truth, so changes made there show up here: macOS `SMAppService` (if macOS asks, the user approves it in System Settings → Login Items), Windows the per-user `Run` registry key (also in Task Manager → Startup apps), Linux an XDG entry in `~/.config/autostart/`.
- **Audio:** Record system audio, Record microphone (the same switches as in the menu), and **Microphone**: *System Default* or a specific input device. The list updates as devices are plugged in or removed. A chosen mic that isn't connected when recording starts falls back to the system default.
- **Video:** **Quality** Low / Medium / *High* (macOS: bitrate width × height × 1 / 2 / 4 bits at 60 fps, scaled by frame rate; Windows: encoder quality 50 / 65 / 80), **Frame rate** 30 / *60* fps, **Show cursor** (default on), and **Highlight clicks** (default off; needs the cursor shown): each click shows a ring that expands (6 → 30 pt) while fading from black to white and out over 0.5 s. It's drawn by a click-through Cappa window over the recorded screen that, unlike Cappa's other windows, is deliberately captured, so the ring also shows on screen while recording. On Linux the ring is drawn into the video by the capture pipeline (not on screen), and only on X11, since Wayland doesn't let apps see clicks in other windows. Applies to new recordings only; editor exports are unaffected.
- **Recordings:** where new recordings are saved (default `~/Movies/Cappa/`), with **Choose…**, **Show in Finder** and **Use Default**. Exports default to the same folder as the recording.

## Global hotkey
- **⌥⇧⌘R** (macOS) / **Ctrl+Alt+Shift+R** (Windows, Linux): when idle, opens the region selector; while selecting, cancels it; while recording, stops.

## Region selector
- Every screen is dimmed and the cursor becomes a crosshair, with a hint in the middle of each screen.
- Drag to select an area, which shows a live `W × H` label (in points). Holding Shift keeps the area square (1:1); pressing or releasing it mid-drag updates the selection right away. Releasing the mouse starts recording that area at native pixel resolution.
- A click without dragging (less than 8 pt) records that whole screen.
- Esc cancels. The selection is limited to the screen where the drag started.
- While recording, a dashed red outline sits just *outside* the recorded area. Like the overlay, it never appears in the video.
- Implementation: macOS uses `SCStreamConfiguration.sourceRect`; Windows crops the monitor capture (`CopySubresourceRegion` before encoding); Linux sets the `ximagesrc` area on X11 and crops the portal's PipeWire stream with `videocrop` on Wayland.

## Recording overlay
- A small pill-shaped panel that floats above all windows (and over full-screen apps and every Space on macOS).
- Contents: a status dot (red = recording, orange = paused), the elapsed time, Pause/Resume, and Stop.
- It starts next to what's being recorded: centred just below a region in the top half of the screen, centred just above one in the bottom half, and centred near the top of the screen (a bit in from the edge) for a full-screen recording. It stays on screen, overlapping the region if there's no room outside it.
- You can drag it anywhere during a recording; the next recording places it again.
- It must **never** appear in the recording:
  - macOS: the whole Cappa app is excluded through the `SCContentFilter`.
  - Windows: `SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE)`.
  - Linux has no way to exclude a window, so the overlay (and the region outline) only appear where they fit entirely outside the recorded region, and only where windows can be positioned: X11, or Wayland compositors with wlr-layer-shell. No overlay for full-screen recordings or on GNOME Wayland.
- It can be hidden from the menu. Stop then stays available from the menu and the hotkey.

## Output
- Location: chosen in Settings; default `~/Movies/Cappa/` (macOS), `%USERPROFILE%\Videos\Cappa\` (Windows), `~/Videos/Cappa/` (Linux).
- File name: `Cappa yyyy-MM-dd at HH.mm.ss.mp4`.
- Container: MP4. Video: H.264 (HEVC when the display is wider than 4096 px), native resolution, up to 30 or 60 fps and at the quality chosen in Settings, hardware-encoded.
- Audio: system audio is AAC, 48 kHz stereo; the microphone is AAC, 48 kHz mono, on its **own track** so the two stay separate in the raw recording. Exports and clipboard copies mix the tracks into one stereo track at the volumes set in the editor (muted tracks are left out), because many players only play the first track.
- Microphone permission is requested before the region selector opens, so a prompt never interrupts a selection.
- Pausing leaves no gap: paused time is removed from the timeline.
- When a recording stops, it opens in the **video editor** automatically.

## Video editor
- One window per video, with a preview on top and the timeline below. While an editor is open, Cappa shows in the Dock and ⌘-Tab.
- The timeline shows the **whole original recording** at a fixed scale, with a time ruler and thumbnails.
  - **Clips** (the parts that are kept) are drawn bright, with a handle on each edge.
  - Parts that are **cut away** stay visible but dimmed, so you can drag an edge back to restore them.
- **Audio lanes:** each audio track has its own lane under the video: *Computer Audio* (teal) and *Microphone* (orange). A video with one stereo track shows a single *Audio* lane.
  - Lanes are **linked** to the video: split, trim and delete always apply to video and all audio together.
  - Each lane shows its waveform, bright inside kept clips and dimmed in cut-away parts. Bar heights follow the volume.
  - The lane header has a **mute** button (the icon) and a **volume** slider (0–100%). Changes are heard immediately during playback.
  - Tracks are told apart by channel count: Cappa records computer audio in stereo and the mic in mono.
- **Trim:** drag a clip's left or right edge. The preview follows the edge. Clips can't overlap or get shorter than 0.1 s.
- **Split (⌘B):** splits the clip under the playhead in two. To cut out a middle part, split twice and delete the middle clip, or split once and drag the two new edges apart.
- **Delete (⌫):** removes the selected clip (the last remaining clip can't be deleted).
- **Select / scrub:** click or drag on a clip to select it and move the playhead. Clicking the ruler or a cut-away area deselects.
- **Play / pause:** Space, or click the video. Playback skips the cut-away parts.
- **Undo / redo:** ⌘Z / ⇧⌘Z.
- **Export** (dropdown; the original recording is never modified):
  - **Save as Video… (⌘E):** joins the clips back to back into a new, frame-accurate MP4 (default name `… (edited).mp4`, next to the original). It re-encodes as H.264, or HEVC if the source is HEVC.
  - **Copy to Clipboard (⇧⌘C):** puts the edited video on the clipboard as an MP4 file, ready to paste into Finder, Slack, Messages or Mail. Unedited videos copy the original file. The edited copy lives in `~/Library/Caches/dev.fallberg.cappa/Clipboard/`, and only the latest one is kept.
  - **Save as GIF… (⇧⌘E):** a looping GIF at 20 fps, scaled to fit 800×800, without audio.
- Closing with edits that haven't been exported asks for confirmation.
- Any video can be opened from the menu with **Open Video in Editor…**.

## Roadmap
1. ~~Menu bar entry + hotkey~~ · ~~display recording~~ · ~~overlay~~ (macOS done)
2. ~~Region recording~~ (macOS done)
3. ~~Microphone capture and device picker~~ (macOS done)
4. A countdown before recording starts
5. ~~Editor: trim, split, delete, export~~ (macOS done)
6. Editor: timeline zoom, saved projects, annotations, cropping the frame (see `project-format.md`, to be written)
