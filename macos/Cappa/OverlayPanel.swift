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

    /// Puts the panel next to what's being recorded: centred below a region in the top half of the screen,
    /// above one in the bottom half, and near the top centre of the screen for a full-screen recording.
    func place(displayID: CGDirectDisplayID, region: CGRect?) {
        let panel = self.panel ?? makePanel()
        self.panel = panel
        guard let screen = NSScreen.screens.first(where: { $0.displayID == displayID }) ?? NSScreen.main else { return }

        let visible = screen.visibleFrame
        let size = panel.frame.size
        var origin: NSPoint
        if let region {
            let isInTopHalf = region.midY > screen.frame.midY // AppKit's y axis points up
            origin = NSPoint(
                x: region.midX - size.width / 2,
                y: isInTopHalf ? region.minY - Self.regionGap - size.height : region.maxY + Self.regionGap
            )
        } else {
            origin = NSPoint(x: visible.midX - size.width / 2, y: visible.maxY - size.height - Self.screenTopMargin)
        }
        // A region filling most of the screen leaves no room outside it; the panel then overlaps the region,
        // which is fine since it never shows up in the video.
        origin.x = min(max(origin.x, visible.minX + Self.edgeMargin), visible.maxX - size.width - Self.edgeMargin)
        origin.y = min(max(origin.y, visible.minY + Self.edgeMargin), visible.maxY - size.height - Self.edgeMargin)
        panel.setFrameOrigin(origin)
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
        return panel
    }

    /// Space between the panel and the recorded region (whose red outline sits 3 pt outside it).
    private static let regionGap: CGFloat = 12
    private static let screenTopMargin: CGFloat = 40
    private static let edgeMargin: CGFloat = 8
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
