# Cappa for Windows

The Windows version of Cappa: a WPF (.NET 8) app in the notification area. It has the same features as the
macOS app (see `../docs/spec.md`).

> **Status: written and compiled on a Mac, not yet run on Windows.** The whole project compiles for x64 and
> ARM64, and the FFmpeg commands for recording, export and GIF were run for real. Everything that needs
> Windows itself (screen capture, WASAPI audio, the UI) still has to be tried on a PC. If something fails,
> send the log (tray menu → **Open Log**).

## Requirements
- Windows 10 version 2004 or later, or Windows 11 (x64 or ARM64)
- **Visual Studio 2022** with the **.NET desktop development** workload (includes the .NET 8 SDK), or just the
  [.NET 8 SDK](https://dotnet.microsoft.com/download/dotnet/8.0) for command-line builds
- An internet connection for the first build (NuGet packages and FFmpeg)

## Build and run
**Visual Studio:** open `windows/Cappa.sln`, pick **x64** (or **ARM64** on an ARM PC) in the toolbar, and press **F5**.

**Command line** (from the `windows` folder):
```powershell
dotnet build Cappa.sln -c Debug -p:Platform=x64
.\Cappa\bin\x64\Debug\net8.0-windows10.0.19041.0\Cappa.exe
```

The first build downloads FFmpeg (about 100 MB, from gyan.dev) into `windows/tools/ffmpeg/` and copies it next
to `Cappa.exe`. This only happens once, and the folder isn't committed. If the download fails, put `ffmpeg.exe`
and `ffprobe.exe` from any recent Windows build of FFmpeg into `windows/tools/ffmpeg/` and build again.

**A standalone folder you can copy to another PC:**
```powershell
dotnet publish Cappa\Cappa.csproj -c Release -p:Platform=x64 -r win-x64 --self-contained -o publish
```

## Using it
- Cappa starts in the **notification area** (click the `^` in the taskbar if the icon is hidden; you can drag
  it out to keep it visible). Left- or right-click it for the menu.
- **Record…** or **Ctrl+Alt+Shift+R**: drag the area to record (hold Shift for a square). A click records the
  whole monitor, and Esc cancels.
- While recording: a floating pill with the timer, **Pause** and **Stop**. It's draggable and never shows up in
  the video. A red dashed outline marks a recorded region. Ctrl+Alt+Shift+R also stops.
- When you stop, the recording opens in the **editor**:
  - drag clip edges to trim; **Ctrl+B** splits at the playhead; **Delete** removes the selected clip;
    **Space** plays and pauses; **Ctrl+Z** / **Ctrl+Y** undo and redo
  - each audio track (Computer Audio, Microphone) has its own lane with a mute button and a volume slider
  - **Export**: Save as Video (**Ctrl+E**), Copy to Clipboard (**Ctrl+Shift+C**), Save as GIF (**Ctrl+Shift+E**)
- **Settings…** (tray menu): start Cappa when Windows starts (default off), system audio and microphone on/off, which microphone, video quality and frame
  rate, cursor and click highlighting, and where recordings are saved (default `Videos\Cappa`).

## How it's built
| Piece | Library / API | Files |
|---|---|---|
| Tray icon, menu | WinForms `NotifyIcon` | `UI/TrayIcon.cs` |
| Hotkey | Win32 `RegisterHotKey` | `UI/GlobalHotKey.cs` |
| Region selector | One see-through WPF window per monitor | `UI/RegionSelector.cs` |
| Overlay, outline | WPF windows with `WDA_EXCLUDEFROMCAPTURE` | `UI/RecordingOverlay.xaml`, `UI/RegionFrame.cs` |
| Screen capture | [ScreenRecorderLib](https://github.com/sskodje/ScreenRecorderLib) (Windows Graphics Capture + hardware H.264) | `Recording/ScreenCapture.cs` |
| System audio / mic | [NAudio](https://github.com/naudio/NAudio) (WASAPI loopback / capture), one WAV each | `Recording/AudioTrackRecorder.cs` |
| Final recording | FFmpeg: video + two titled AAC tracks | `Recording/RecordingSession.cs`, `Recording/MuxCommand.cs` |
| Editor preview | WPF `MediaElement` (video) + NAudio mixer (per-track volume) | `Editor/EditorPlayback.cs` |
| Timeline | Custom-drawn WPF control | `Editor/TimelineControl.cs` |
| Export, GIF, thumbnails | FFmpeg | `Editor/Exporter.cs`, `Editor/MediaPreparation.cs` |
| Settings | JSON in `%APPDATA%\Cappa\settings.json` | `Services/AppSettings.cs` |
| Log | `%LOCALAPPDATA%\Cappa\cappa.log` | `Services/Log.cs` |

**Differences from the macOS app**
- WPF rather than WinUI 3: Cappa needs see-through, click-through and always-on-top windows (the region selector
  and overlay), which WPF supports directly.
- FFmpeg handles export and the audio/video merge. On macOS, AVFoundation provides this.
- The video encode during export uses libx264 (CPU). On macOS it's hardware-encoded.

## Troubleshooting
- **"Could not load file or assembly 'ScreenRecorderLib'"**: install the
  [Microsoft Visual C++ Redistributable 2015–2022](https://learn.microsoft.com/cpp/windows/latest-supported-vc-redist) (x64 or ARM64).
- **The microphone records silence**: Windows Settings → Privacy & security → Microphone → turn on
  *Let desktop apps access your microphone*.
- **Editor shows no video**: Windows "N" editions lack media playback; install the *Media Feature Pack*.
- **Ctrl+Alt+Shift+R does nothing**: another app has the shortcut. The log says so; use the tray menu.
- Anything else: tray menu → **Open Log**, and look at the last lines.
