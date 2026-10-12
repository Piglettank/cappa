import AppKit
import SwiftUI

/// Owns the floating recording-controls panel shown while a recording is active.
@MainActor
final class OverlayPanelController {
    private unowned let controller: RecordingController
    private var panel: NSPanel?

    init(controller: RecordingController) {
        self.controller = controller
    }

    func show() {
        let panel = self.panel ?? makePanel()
        self.panel = panel
        panel.orderFrontRegardless()
    }

    func hide() {
        panel?.orderOut(nil)
    }

    private func makePanel() -> NSPanel {
        let hosting = DraggableHostingView(rootView: OverlayView(controller: controller))
        let size = hosting.fittingSize

        let panel = OverlayPanel(
            contentRect: NSRect(origin: .zero, size: size),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: false
        )
        panel.contentView = hosting
        panel.isFloatingPanel = true
        panel.level = .floating
        panel.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        panel.hidesOnDeactivate = false
        panel.isMovableByWindowBackground = true
        panel.backgroundColor = .clear
        panel.isOpaque = false
        panel.hasShadow = true

        // Default to top-centre of the main screen; after that, AppKit remembers where it was dragged.
        if !panel.setFrameUsingName(Self.autosaveName), let screen = NSScreen.main {
            let visible = screen.visibleFrame
            panel.setFrameOrigin(NSPoint(x: visible.midX - size.width / 2, y: visible.maxY - size.height - 12))
        }
        panel.setFrameAutosaveName(Self.autosaveName)
        return panel
    }

    private static let autosaveName = "CappaRecordingOverlay"
}

private final class OverlayPanel: NSPanel {
    override var canBecomeKey: Bool { true }
}

/// Lets the panel be dragged from anywhere that isn't a button.
private final class DraggableHostingView<Content: View>: NSHostingView<Content> {
    override var mouseDownCanMoveWindow: Bool { true }
}

private struct OverlayView: View {
    @ObservedObject var controller: RecordingController

    private var isPaused: Bool { controller.state == .paused }

    var body: some View {
        HStack(spacing: 12) {
            Circle()
                .fill(isPaused ? Color.orange : Color.red)
                .frame(width: 10, height: 10)

            Text(Self.format(controller.elapsed))
                .font(.system(size: 13, weight: .medium).monospacedDigit())
                .frame(minWidth: 44, alignment: .leading)

            Button(action: controller.togglePause) {
                Image(systemName: isPaused ? "play.fill" : "pause.fill")
                    .frame(width: 22, height: 22)
                    .contentShape(Rectangle())
            }
            .help(isPaused ? "Resume" : "Pause")

            Button {
                Task { await controller.stop() }
            } label: {
                Image(systemName: "stop.fill")
                    .foregroundStyle(.red)
                    .frame(width: 22, height: 22)
                    .contentShape(Rectangle())
            }
            .help("Stop recording (⌥⇧⌘R)")
        }
        .buttonStyle(.plain)
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(.regularMaterial, in: Capsule())
        .overlay(Capsule().strokeBorder(.primary.opacity(0.1)))
        .padding(4)
    }

    private static func format(_ interval: TimeInterval) -> String {
        let total = Int(interval)
        let hours = total / 3600
        let minutes = (total % 3600) / 60
        let seconds = total % 60
        return hours > 0
            ? String(format: "%d:%02d:%02d", hours, minutes, seconds)
            : String(format: "%02d:%02d", minutes, seconds)
    }
}
