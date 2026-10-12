import AppKit
import SwiftUI

/// The Settings window (⌘, from the menu bar menu).
@MainActor
final class SettingsWindowController: NSWindowController, NSWindowDelegate {
    private static var shared: SettingsWindowController?

    static func show(controller: RecordingController) {
        if let shared {
            shared.window?.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return
        }

        let settings = SettingsWindowController(controller: controller)
        shared = settings
        DockPresence.windowOpened()
        settings.window?.center()
        settings.showWindow(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    private init(controller: RecordingController) {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 480, height: 400),
            styleMask: [.titled, .closable],
            backing: .buffered,
            defer: false
        )
        window.title = "Cappa Settings"
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: SettingsView(controller: controller))
        super.init(window: window)
        window.delegate = self
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    func windowWillClose(_ notification: Notification) {
        Self.shared = nil
        DockPresence.windowClosed()
    }
}

struct SettingsView: View {
    @ObservedObject var controller: RecordingController
    @StateObject private var microphones = MicrophoneList()

    var body: some View {
        Form {
            Section("Audio") {
                Toggle(isOn: $controller.captureSystemAudio) {
                    Text("Record system audio")
                    Text("Sound from all apps on your Mac, except Cappa.")
                }

                Toggle(isOn: $controller.captureMicrophone) {
                    Text("Record microphone")
                    Text(ScreenRecorder.supportsMicrophone
                         ? "Recorded on its own track, so it can be adjusted separately in the editor."
                         : "Requires macOS 15 or later.")
                }
                .disabled(!ScreenRecorder.supportsMicrophone)

                Picker("Microphone", selection: $controller.microphoneID) {
                    Text(microphones.defaultDeviceName.map { "System Default (\($0))" } ?? "System Default")
                        .tag("")
                    if !controller.microphoneID.isEmpty,
                       !microphones.devices.contains(where: { $0.id == controller.microphoneID }) {
                        Text("Selected microphone (not connected)")
                            .tag(controller.microphoneID)
                    }
                    Divider()
                    ForEach(microphones.devices) { device in
                        Text(device.name).tag(device.id)
                    }
                }
                .disabled(!controller.captureMicrophone || !ScreenRecorder.supportsMicrophone)
            }

            Section("Recordings") {
                LabeledContent("Save recordings to") {
                    HStack(spacing: 6) {
                        Image(nsImage: NSWorkspace.shared.icon(forFile: controller.recordingsFolder.path))
                            .resizable()
                            .frame(width: 16, height: 16)
                        Text(controller.recordingsFolder.lastPathComponent)
                    }
                }
                Text(Self.abbreviated(controller.recordingsFolder))
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)

                HStack {
                    Button("Choose…", action: chooseFolder)
                    Button("Show in Finder") { controller.openRecordingsFolder() }
                    Spacer()
                    Button("Use Default") {
                        controller.recordingsFolder = RecordingController.defaultRecordingsFolder
                    }
                    .disabled(controller.recordingsFolder.standardizedFileURL
                              == RecordingController.defaultRecordingsFolder.standardizedFileURL)
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 480)
        .fixedSize(horizontal: false, vertical: true)
    }

    private func chooseFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "Choose"
        panel.message = "Choose where Cappa saves new recordings."
        panel.directoryURL = controller.recordingsFolder
        if panel.runModal() == .OK, let url = panel.url {
            controller.recordingsFolder = url
        }
    }

    /// `/Users/name/Movies/Cappa` → `~/Movies/Cappa`
    private static func abbreviated(_ url: URL) -> String {
        (url.path as NSString).abbreviatingWithTildeInPath
    }
}
