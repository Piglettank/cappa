import AppKit
import AVFoundation
import ScreenCaptureKit
import UniformTypeIdentifiers

/// App-level recording state shared by the menu bar menu and the overlay.
@MainActor
final class RecordingController: ObservableObject {
    enum State { case idle, selecting, starting, recording, paused, stopping }

    @Published private(set) var state: State = .idle
    @Published private(set) var elapsed: TimeInterval = 0

    @Published var captureSystemAudio: Bool {
        didSet { UserDefaults.standard.set(captureSystemAudio, forKey: "captureSystemAudio") }
    }
    @Published var captureMicrophone: Bool {
        didSet { UserDefaults.standard.set(captureMicrophone, forKey: "captureMicrophone") }
    }
    /// `AVCaptureDevice.uniqueID` of the chosen microphone; empty means the system default input.
    @Published var microphoneID: String {
        didSet { UserDefaults.standard.set(microphoneID, forKey: "microphoneID") }
    }
    @Published var recordingsFolder: URL {
        didSet { UserDefaults.standard.set(recordingsFolder.path, forKey: "recordingsFolder") }
    }
    @Published var quality: RecordingQuality {
        didSet { UserDefaults.standard.set(quality.rawValue, forKey: "quality") }
    }
    /// 30 or 60.
    @Published var frameRate: Int {
        didSet { UserDefaults.standard.set(frameRate, forKey: "frameRate") }
    }
    @Published var showCursor: Bool {
        didSet { UserDefaults.standard.set(showCursor, forKey: "showCursor") }
    }
    @Published var highlightClicks: Bool {
        didSet { UserDefaults.standard.set(highlightClicks, forKey: "highlightClicks") }
    }
    @Published var showOverlay: Bool {
        didSet {
            UserDefaults.standard.set(showOverlay, forKey: "showOverlay")
            updateOverlayVisibility()
        }
    }

    var isActive: Bool { state == .recording || state == .paused }

    private var recorder: ScreenRecorder?
    private var outputURL: URL?
    private var timer: Timer?
    private var accumulated: TimeInterval = 0
    private var segmentStart: Date?
    private lazy var overlay = OverlayPanelController(controller: self)
    private let regionSelector = RegionSelector()
    private var regionFrame: RegionFrameWindow?
    private var clickHighlighter: ClickHighlighter?

    init() {
        UserDefaults.standard.register(defaults: [
            "captureSystemAudio": true, "captureMicrophone": false, "showOverlay": true,
            "frameRate": 60, "showCursor": true, "highlightClicks": false,
        ])
        captureSystemAudio = UserDefaults.standard.bool(forKey: "captureSystemAudio")
        captureMicrophone = UserDefaults.standard.bool(forKey: "captureMicrophone")
        microphoneID = UserDefaults.standard.string(forKey: "microphoneID") ?? ""
        recordingsFolder = UserDefaults.standard.string(forKey: "recordingsFolder")
            .map { URL(fileURLWithPath: $0, isDirectory: true) } ?? Self.defaultRecordingsFolder
        quality = UserDefaults.standard.string(forKey: "quality").flatMap(RecordingQuality.init) ?? .high
        frameRate = UserDefaults.standard.integer(forKey: "frameRate") == 30 ? 30 : 60
        showCursor = UserDefaults.standard.bool(forKey: "showCursor")
        highlightClicks = UserDefaults.standard.bool(forKey: "highlightClicks")
        showOverlay = UserDefaults.standard.bool(forKey: "showOverlay")
    }

    static var defaultRecordingsFolder: URL {
        FileManager.default.urls(for: .moviesDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Cappa", isDirectory: true)
    }

    /// Hotkey behaviour: start a region recording, cancel an in-progress selection, or stop.
    func toggleRecording() async {
        switch state {
        case .idle: await startRegionRecording()
        case .selecting: regionSelector.cancel()
        case .recording, .paused: await stop()
        case .starting, .stopping: break
        }
    }

    func startRegionRecording() async {
        guard state == .idle else { return }
        guard await ensurePermissions() else { return }

        state = .selecting
        let selection = await regionSelector.select()
        state = .idle
        guard let selection else { return }

        await beginRecording(displayID: selection.displayID, sourceRect: selection.sourceRect, screenRect: selection.screenRect)
    }

    private func beginRecording(displayID: CGDirectDisplayID, sourceRect: CGRect?, screenRect: CGRect?) async {
        state = .starting
        // The click highlight must be on screen before asking for shareable content, so its window is listed.
        if showCursor, highlightClicks, let screen = NSScreen.screens.first(where: { $0.displayID == displayID }) {
            clickHighlighter = ClickHighlighter(screen: screen)
            clickHighlighter?.show()
        }
        do {
            let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
            guard let display = content.displays.first(where: { $0.displayID == displayID }) else {
                throw RecorderError.noDisplay
            }
            // Excluding our own app keeps the overlay (and any future Cappa windows) out of the video;
            // the click highlight is the one Cappa window that belongs in it.
            let ownApps = content.applications.filter { $0.bundleIdentifier == Bundle.main.bundleIdentifier }
            let recordedWindows = content.windows.filter { $0.windowID == clickHighlighter?.windowID }

            try FileManager.default.createDirectory(at: recordingsFolder, withIntermediateDirectories: true)
            let url = recordingsFolder.appendingPathComponent(Self.fileName())

            let recorder = ScreenRecorder()
            recorder.onStreamError = { [weak self] error in
                Task { @MainActor in await self?.handleStreamFailure(error) }
            }
            try await recorder.start(
                display: display,
                sourceRect: sourceRect,
                excludingApplications: ownApps,
                includingWindows: recordedWindows,
                scale: Self.backingScale(for: display),
                quality: quality,
                frameRate: frameRate,
                showsCursor: showCursor,
                captureAudio: captureSystemAudio,
                captureMicrophone: captureMicrophone,
                // A remembered microphone that isn't plugged in falls back to the default input.
                microphoneID: AVCaptureDevice(uniqueID: microphoneID) == nil ? nil : microphoneID,
                outputURL: url
            )

            self.recorder = recorder
            outputURL = url
            accumulated = 0
            segmentStart = Date()
            elapsed = 0
            state = .recording
            startTimer()
            overlay.place(displayID: displayID, region: screenRect)
            updateOverlayVisibility()
            if let screenRect {
                regionFrame = RegionFrameWindow(screenRect: screenRect)
                regionFrame?.show()
            }
        } catch {
            clickHighlighter?.close()
            clickHighlighter = nil
            state = .idle
            showError("Couldn't start recording", error)
        }
    }

    func togglePause() {
        guard let recorder else { return }
        switch state {
        case .recording:
            recorder.pause()
            accumulated += Date().timeIntervalSince(segmentStart ?? Date())
            segmentStart = nil
            state = .paused
        case .paused:
            recorder.resume()
            segmentStart = Date()
            state = .recording
        default:
            break
        }
    }

    func stop() async {
        guard isActive, let recorder else { return }
        state = .stopping
        stopTimer()
        overlay.hide()
        regionFrame?.close()
        regionFrame = nil
        clickHighlighter?.close()
        clickHighlighter = nil

        do {
            try await recorder.stop()
            if let outputURL {
                EditorWindowController.open(url: outputURL)
            }
        } catch {
            showError("Couldn't save recording", error)
        }
        self.recorder = nil
        outputURL = nil
        state = .idle
    }

    func openVideoInEditor() {
        let panel = NSOpenPanel()
        panel.allowedContentTypes = [.movie]
        panel.directoryURL = recordingsFolder
        NSApp.activate(ignoringOtherApps: true)
        guard panel.runModal() == .OK, let url = panel.url else { return }
        EditorWindowController.open(url: url)
    }

    func openRecordingsFolder() {
        try? FileManager.default.createDirectory(at: recordingsFolder, withIntermediateDirectories: true)
        NSWorkspace.shared.open(recordingsFolder)
    }

    // MARK: - Private

    private func handleStreamFailure(_ error: Error) async {
        guard isActive else { return }
        showError("Recording stopped unexpectedly", error)
        await stop()
    }

    private func updateOverlayVisibility() {
        if isActive && showOverlay {
            overlay.show()
        } else {
            overlay.hide()
        }
    }

    private func startTimer() {
        timer = Timer.scheduledTimer(withTimeInterval: 0.25, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.tick() }
        }
    }

    private func stopTimer() {
        timer?.invalidate()
        timer = nil
    }

    private func tick() {
        elapsed = accumulated + (segmentStart.map { Date().timeIntervalSince($0) } ?? 0)
    }

    /// Checked before the region selector opens, so a prompt never interrupts a selection.
    private func ensurePermissions() async -> Bool {
        guard CGPreflightScreenCaptureAccess() else {
            CGRequestScreenCaptureAccess()
            showPermissionAlert(
                "Cappa needs Screen Recording permission",
                "Turn on Cappa under Privacy & Security → Screen & System Audio Recording, then try again. You may need to restart Cappa.",
                settingsPane: "Privacy_ScreenCapture"
            )
            return false
        }

        guard captureMicrophone, ScreenRecorder.supportsMicrophone else { return true }
        switch AVCaptureDevice.authorizationStatus(for: .audio) {
        case .authorized:
            return true
        case .notDetermined:
            if await AVCaptureDevice.requestAccess(for: .audio) { return true }
        default:
            break
        }
        showPermissionAlert(
            "Cappa needs Microphone permission",
            "Turn on Cappa under Privacy & Security → Microphone, or turn off Record Microphone in the Cappa menu.",
            settingsPane: "Privacy_Microphone"
        )
        return false
    }

    private func showPermissionAlert(_ title: String, _ message: String, settingsPane: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.addButton(withTitle: "Open System Settings")
        alert.addButton(withTitle: "Cancel")
        NSApp.activate(ignoringOtherApps: true)
        if alert.runModal() == .alertFirstButtonReturn,
           let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?\(settingsPane)") {
            NSWorkspace.shared.open(url)
        }
    }

    private func showError(_ title: String, _ error: Error) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = error.localizedDescription
        alert.alertStyle = .warning
        NSApp.activate(ignoringOtherApps: true)
        alert.runModal()
    }

    private static func backingScale(for display: SCDisplay) -> CGFloat {
        NSScreen.screens.first { $0.displayID == display.displayID }?.backingScaleFactor ?? 2
    }

    private static func fileName() -> String {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd 'at' HH.mm.ss"
        return "Cappa \(formatter.string(from: Date())).mp4"
    }
}

extension NSScreen {
    var displayID: CGDirectDisplayID? {
        deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? CGDirectDisplayID
    }
}
