# Cappa

A lightweight, native screen recorder for macOS and Windows, with a built-in video editor.

Each platform gets its own native app that uses the OS's first-party capture and media stack:

| | macOS (`macos/`) | Windows (`windows/`) |
|---|---|---|
| Language / UI | Swift, SwiftUI + AppKit | C#, WPF (.NET 8) |
| Capture | ScreenCaptureKit | Windows.Graphics.Capture (via ScreenRecorderLib), WASAPI audio (NAudio) |
| Encoding / editing | AVFoundation | Media Foundation (capture), FFmpeg (merge, export, GIF) |
| Entry point | Menu bar item | Notification-area (tray) icon |
| While recording | Floating, always-on-top control panel | Always-on-top control window |

Shared behaviour and file formats are described in [`docs/`](docs/), so both apps stay in step.

## macOS

Requirements: Xcode 16 or later (built with 26.3), macOS 13 or later.

```sh
open macos/Cappa.xcodeproj        # then ⌘R
# or
xcodebuild -project macos/Cappa.xcodeproj -scheme Cappa build
```

- Cappa lives in the menu bar (no Dock icon). Choose **Record Region…** or press **⌥⇧⌘R** anywhere, then drag out the area to record. A click records the whole screen, and Esc cancels.
- **Record Screen** in the menu skips the selector and records the display under the mouse pointer.
- System audio is recorded by default. Turn on **Record Microphone** in the menu to add your voice (macOS 15+).
- Recordings are saved to `~/Movies/Cappa/` (change it in **Settings… ⌘,**, where you also pick the microphone), and the **video editor** opens when you stop: trim by dragging clip edges, **⌘B** splits at the playhead, **⌫** deletes a clip, and **Export** saves as video (⌘E), copies to the clipboard (⇧⌘C) or saves as a GIF (⇧⌘E).
- The first time, macOS asks for **Screen & System Audio Recording** permission.

**Signing:** the project currently signs ad hoc ("Sign to Run Locally"). macOS ties the Screen Recording permission to the code signature, so ad hoc builds may need to be re-approved after a rebuild. To avoid this, set your Apple Development team under *Signing & Capabilities*.

## Windows

A WPF (.NET 8) app in [`windows/`](windows/). Open `windows/Cappa.sln` in Visual Studio 2022 and press F5;
see [`windows/README.md`](windows/README.md). It was written and compiled on a Mac and hasn't been run on
Windows yet.
