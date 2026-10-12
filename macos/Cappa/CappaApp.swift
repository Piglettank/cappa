import SwiftUI

@main
struct CappaApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate

    var body: some Scene {
        MenuBarExtra {
            MenuContent(controller: appDelegate.controller)
        } label: {
            MenuBarLabel(controller: appDelegate.controller)
        }
    }
}

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    let controller = RecordingController()
    private var hotKey: GlobalHotKey?

    func applicationDidFinishLaunching(_ notification: Notification) {
        // ⌥⇧⌘R toggles recording from anywhere.
        hotKey = GlobalHotKey(keyCode: GlobalHotKey.keyR, modifiers: [.command, .option, .shift]) { [weak self] in
            Task { @MainActor in await self?.controller.toggleRecording() }
        }
    }
}

private struct MenuBarLabel: View {
    @ObservedObject var controller: RecordingController

    var body: some View {
        Image(systemName: controller.isActive ? "stop.circle.fill" : "record.circle")
    }
}

private struct MenuContent: View {
    @ObservedObject var controller: RecordingController

    var body: some View {
        switch controller.state {
        case .idle:
            Button("Record Region…") { Task { await controller.startRegionRecording() } }
                .keyboardShortcut("r", modifiers: [.command, .option, .shift])
            Button("Record Screen") { Task { await controller.startDisplayRecording() } }
        case .selecting:
            Button("Cancel Selection") { Task { await controller.toggleRecording() } }
        case .starting:
            Text("Starting…")
        case .recording, .paused:
            Button("Stop Recording") { Task { await controller.stop() } }
                .keyboardShortcut("r", modifiers: [.command, .option, .shift])
            Button(controller.state == .paused ? "Resume" : "Pause") { controller.togglePause() }
        case .stopping:
            Text("Saving…")
        }

        Divider()

        Toggle("Record System Audio", isOn: $controller.captureSystemAudio)
            .disabled(controller.state != .idle)
        Toggle(ScreenRecorder.supportsMicrophone ? "Record Microphone" : "Record Microphone (macOS 15+)",
               isOn: $controller.captureMicrophone)
            .disabled(controller.state != .idle || !ScreenRecorder.supportsMicrophone)
        Toggle("Show Overlay While Recording", isOn: $controller.showOverlay)

        Divider()

        Button("Settings…") { SettingsWindowController.show(controller: controller) }
            .keyboardShortcut(",")
        Button("Open Video in Editor…") { controller.openVideoInEditor() }
        Button("Open Recordings Folder") { controller.openRecordingsFolder() }
        Button("Quit Cappa") { NSApp.terminate(nil) }
            .keyboardShortcut("q")
    }
}
