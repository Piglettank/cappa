import AVKit
import SwiftUI
import UniformTypeIdentifiers

struct EditorView: View {
    @ObservedObject var model: EditorModel

    var body: some View {
        VStack(spacing: 0) {
            PlayerView(player: model.player)
                .overlay(Color.clear.contentShape(Rectangle()).onTapGesture(perform: model.togglePlay))
                .overlay(alignment: .bottom) {
                    if let notice = model.notice {
                        Label(notice, systemImage: "checkmark.circle.fill")
                            .padding(.horizontal, 14)
                            .padding(.vertical, 8)
                            .background(.regularMaterial, in: Capsule())
                            .padding(.bottom, 16)
                            .transition(.opacity)
                    }
                }
                .animation(.easeInOut(duration: 0.2), value: model.notice)
                .background(Color.black)

            Divider()

            controlBar
                .padding(.horizontal, 16)
                .padding(.vertical, 10)
                .zIndex(1) // keeps hover hints above the timeline

            EditorTimeline(model: model)
                .padding(.horizontal, 16)
                .padding(.bottom, 16)
        }
        .frame(minWidth: 720, minHeight: 520)
        .disabled(!model.isLoaded || model.exportProgress != nil)
        .overlay {
            if !model.isLoaded {
                ProgressView()
            } else if let progress = model.exportProgress {
                exportingOverlay(progress)
            }
        }
    }

    private var controlBar: some View {
        HStack(spacing: 12) {
            HStack(spacing: 4) {
                toolbarButton("Undo  ⌘Z", shortcut: KeyboardShortcut("z", modifiers: .command),
                              isAvailable: model.canUndo, action: model.undo) {
                    Image(systemName: "arrow.uturn.backward").frame(width: 18, height: 18)
                }
                toolbarButton("Redo  ⇧⌘Z", shortcut: KeyboardShortcut("z", modifiers: [.command, .shift]),
                              isAvailable: model.canRedo, action: model.redo) {
                    Image(systemName: "arrow.uturn.forward").frame(width: 18, height: 18)
                }
            }

            Divider().frame(height: 18)

            toolbarButton("Split at playhead  ⌘B", shortcut: KeyboardShortcut("b", modifiers: .command),
                          isAvailable: model.canSplit, action: model.split) {
                Label("Split", systemImage: "scissors")
            }

            toolbarButton("Delete clip  ⌫", shortcut: KeyboardShortcut(.delete, modifiers: []),
                          isAvailable: model.canDelete, action: model.deleteSelected) {
                Label("Delete Clip", systemImage: "trash")
            }

            Spacer()

            Button(action: model.togglePlay) {
                Image(systemName: model.isPlaying ? "pause.fill" : "play.fill")
                    .font(.system(size: 16))
                    .frame(width: 28, height: 22)
            }
            .help("Play / Pause (Space)")
            .keyboardShortcut(.space, modifiers: [])

            Text(TimeFormat.precise(model.currentTime))
                .font(.system(.body, design: .monospaced))
                .frame(minWidth: 72, alignment: .leading)

            Spacer()

            Text("Length \(TimeFormat.precise(model.editedDuration))")
                .font(.callout.monospacedDigit())
                .foregroundStyle(.secondary)

            Button("Show in Finder") {
                NSWorkspace.shared.activateFileViewerSelecting([model.url])
            }

            Menu {
                Button("Save as Video…  ⌘E", action: saveAsVideo)
                Button("Copy to Clipboard  ⇧⌘C", action: copyToClipboard)
                Button("Save as GIF…  ⇧⌘E", action: saveAsGIF)
            } label: {
                Label("Export", systemImage: "square.and.arrow.up")
            }
            .fixedSize()
            .help("Save as Video ⌘E · Copy to Clipboard ⇧⌘C · Save as GIF ⇧⌘E")
        }
        // Menu items don't own key equivalents while the menu is closed, so the shortcuts live on hidden buttons.
        .background {
            Group {
                Button("", action: saveAsVideo).keyboardShortcut("e", modifiers: .command)
                Button("", action: copyToClipboard).keyboardShortcut("c", modifiers: [.command, .shift])
                Button("", action: saveAsGIF).keyboardShortcut("e", modifiers: [.command, .shift])
            }
            .frame(width: 0, height: 0)
            .opacity(0)
            .accessibilityHidden(true)
        }
    }

    /// These buttons stay enabled and only look disabled when unavailable (the model ignores the action then),
    /// so the hover hint with the shortcut always works.
    private func toolbarButton<Content: View>(
        _ tooltip: String,
        shortcut: KeyboardShortcut,
        isAvailable: Bool,
        action: @escaping () -> Void,
        @ViewBuilder label: () -> Content
    ) -> some View {
        Button(action: action) {
            label().foregroundStyle(isAvailable ? HierarchicalShapeStyle.primary : .tertiary)
        }
        .keyboardShortcut(shortcut)
        .modifier(ShortcutHint(text: tooltip))
    }

    private func exportingOverlay(_ progress: Double) -> some View {
        VStack(spacing: 10) {
            Text(model.exportTitle).font(.headline)
            ProgressView(value: progress)
                .frame(width: 240)
        }
        .padding(24)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
    }

    private func saveAsVideo() {
        guard let url = chooseDestination(type: .mpeg4Movie, fileExtension: "mp4") else { return }
        Task {
            do {
                try await model.saveVideo(to: url)
                NSWorkspace.shared.activateFileViewerSelecting([url])
            } catch {
                showAlert("Export failed", error.localizedDescription)
            }
        }
    }

    private func saveAsGIF() {
        guard let url = chooseDestination(type: .gif, fileExtension: "gif") else { return }
        Task {
            do {
                try await model.saveGIF(to: url)
                NSWorkspace.shared.activateFileViewerSelecting([url])
            } catch {
                showAlert("GIF export failed", error.localizedDescription)
            }
        }
    }

    private func copyToClipboard() {
        Task {
            do {
                try await model.copyToClipboard()
            } catch {
                showAlert("Couldn't copy to clipboard", error.localizedDescription)
            }
        }
    }

    private func chooseDestination(type: UTType, fileExtension: String) -> URL? {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [type]
        panel.directoryURL = model.url.deletingLastPathComponent()
        panel.nameFieldStringValue = model.url.deletingPathExtension().lastPathComponent + " (edited).\(fileExtension)"
        guard panel.runModal() == .OK, let url = panel.url else { return nil }

        guard url.standardizedFileURL != model.url.standardizedFileURL else {
            showAlert("Choose a different file name", "Cappa keeps the original recording, so the edit can't be saved over it.")
            return nil
        }
        return url
    }

    private func showAlert(_ title: String, _ message: String) {
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = message
        alert.alertStyle = .warning
        alert.runModal()
    }
}

/// A hint shown under a button shortly after the pointer rests on it. Unlike the system
/// tooltip it appears quickly and doesn't depend on the button's or window's state.
private struct ShortcutHint: ViewModifier {
    let text: String

    @State private var isHovering = false
    @State private var isShown = false

    func body(content: Content) -> some View {
        content
            .onHover { hovering in
                isHovering = hovering
                guard hovering else {
                    isShown = false
                    return
                }
                Task {
                    try? await Task.sleep(nanoseconds: 400_000_000)
                    if isHovering {
                        withAnimation(.easeOut(duration: 0.12)) { isShown = true }
                    }
                }
            }
            .overlay(alignment: .bottom) {
                if isShown {
                    Text(text)
                        .font(.caption)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 6))
                        .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.primary.opacity(0.15)))
                        .fixedSize()
                        .offset(y: 30)
                        .allowsHitTesting(false)
                        .transition(.opacity)
                }
            }
    }
}

private struct PlayerView: NSViewRepresentable {
    let player: AVPlayer

    func makeNSView(context: Context) -> AVPlayerView {
        let view = AVPlayerView()
        view.player = player
        view.controlsStyle = .none
        view.videoGravity = .resizeAspect
        return view
    }

    func updateNSView(_ view: AVPlayerView, context: Context) {}
}

enum TimeFormat {
    /// `m:ss.t`, or `h:mm:ss.t` for long videos.
    static func precise(_ seconds: Double) -> String {
        let safe = max(seconds.isFinite ? seconds : 0, 0)
        let tenths = Int((safe * 10).rounded(.down))
        let total = tenths / 10
        let fraction = tenths % 10
        let hours = total / 3600
        let minutes = (total % 3600) / 60
        let secs = total % 60
        return hours > 0
            ? String(format: "%d:%02d:%02d.%d", hours, minutes, secs, fraction)
            : String(format: "%d:%02d.%d", minutes, secs, fraction)
    }

    /// `m:ss`, for ruler labels.
    static func short(_ seconds: Double) -> String {
        let total = Int(seconds.rounded())
        return String(format: "%d:%02d", total / 60, total % 60)
    }
}
