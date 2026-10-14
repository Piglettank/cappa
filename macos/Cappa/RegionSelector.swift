import AppKit
import Carbon.HIToolbox

struct RegionSelection {
    let displayID: CGDirectDisplayID
    /// Display-local rect in points with a top-left origin, as ScreenCaptureKit expects. `nil` means the whole display.
    let sourceRect: CGRect?
    /// The same area in global AppKit screen coordinates, for drawing on screen. `nil` means the whole display.
    let screenRect: CGRect?
}

/// Dims every screen and lets the user drag out the area to record.
/// Click without dragging picks the whole screen; holding Shift keeps the area square; Esc cancels.
@MainActor
final class RegionSelector {
    private var windows: [SelectionWindow] = []
    private var continuation: CheckedContinuation<RegionSelection?, Never>?
    private var previousApp: NSRunningApplication?

    var isActive: Bool { continuation != nil }

    func select() async -> RegionSelection? {
        guard continuation == nil else { return nil }
        return await withCheckedContinuation { continuation in
            self.continuation = continuation
            present()
        }
    }

    func cancel() {
        finish(nil)
    }

    private func present() {
        // Cappa is a menu bar app, so it must activate to receive Esc; hand focus back afterwards.
        let frontmost = NSWorkspace.shared.frontmostApplication
        previousApp = frontmost?.bundleIdentifier == Bundle.main.bundleIdentifier ? nil : frontmost
        NSApp.activate(ignoringOtherApps: true)

        let mouse = NSEvent.mouseLocation
        for screen in NSScreen.screens {
            let window = SelectionWindow(screen: screen)
            let view = window.selectionView
            view.onBegin = { [weak self, weak view] in
                // Only one selection at a time, across all screens.
                self?.windows.filter { $0.selectionView !== view }.forEach { $0.selectionView.clearSelection() }
            }
            view.onComplete = { [weak self] rect in self?.complete(screen: screen, rect: rect) }
            view.onCancel = { [weak self] in self?.cancel() }
            windows.append(window)

            if NSMouseInRect(mouse, screen.frame, false) {
                window.makeKeyAndOrderFront(nil)
            } else {
                window.orderFrontRegardless()
            }
        }
        NSCursor.crosshair.push()
    }

    private func complete(screen: NSScreen, rect: CGRect?) {
        guard let displayID = screen.displayID else {
            finish(nil)
            return
        }
        guard let rect else {
            finish(RegionSelection(displayID: displayID, sourceRect: nil, screenRect: nil))
            return
        }
        // View coordinates are screen-local with a bottom-left origin.
        let sourceRect = CGRect(x: rect.minX, y: screen.frame.height - rect.maxY, width: rect.width, height: rect.height)
        let screenRect = rect.offsetBy(dx: screen.frame.minX, dy: screen.frame.minY)
        finish(RegionSelection(displayID: displayID, sourceRect: sourceRect, screenRect: screenRect))
    }

    private func finish(_ result: RegionSelection?) {
        guard let continuation else { return }
        self.continuation = nil

        NSCursor.pop()
        windows.forEach { $0.orderOut(nil) }
        windows.removeAll()
        previousApp?.activate(options: [])
        previousApp = nil

        continuation.resume(returning: result)
    }
}

// MARK: - Selection window

private final class SelectionWindow: NSWindow {
    let selectionView: RegionSelectionView

    init(screen: NSScreen) {
        selectionView = RegionSelectionView(frame: NSRect(origin: .zero, size: screen.frame.size))
        super.init(contentRect: screen.frame, styleMask: .borderless, backing: .buffered, defer: false)
        setFrame(screen.frame, display: false)
        isReleasedWhenClosed = false
        level = .screenSaver
        backgroundColor = .clear
        isOpaque = false
        hasShadow = false
        collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        contentView = selectionView
        makeFirstResponder(selectionView)
    }

    override var canBecomeKey: Bool { true }
}

private final class RegionSelectionView: NSView {
    var onBegin: (() -> Void)?
    /// Called with the selected rect in view coordinates, or `nil` for "whole screen".
    var onComplete: ((CGRect?) -> Void)?
    var onCancel: (() -> Void)?

    private static let minimumSize: CGFloat = 8

    private var dragStart: NSPoint?
    private var dragPoint: NSPoint?
    private var selection: NSRect? {
        didSet { needsDisplay = true }
    }

    override var acceptsFirstResponder: Bool { true }
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    func clearSelection() {
        dragStart = nil
        dragPoint = nil
        selection = nil
    }

    override func mouseDown(with event: NSEvent) {
        window?.makeKey()
        window?.makeFirstResponder(self)
        onBegin?()
        dragStart = clamped(convert(event.locationInWindow, from: nil))
        dragPoint = nil
        selection = nil
    }

    override func mouseDragged(with event: NSEvent) {
        guard dragStart != nil else { return }
        dragPoint = clamped(convert(event.locationInWindow, from: nil))
        updateSelection(square: event.modifierFlags.contains(.shift))
    }

    override func flagsChanged(with event: NSEvent) {
        // Pressing or releasing Shift mid-drag reshapes the selection without waiting for the mouse to move.
        guard dragPoint != nil else { return super.flagsChanged(with: event) }
        updateSelection(square: event.modifierFlags.contains(.shift))
    }

    private func updateSelection(square: Bool) {
        guard let dragStart, let dragPoint else { return }
        var width = abs(dragPoint.x - dragStart.x)
        var height = abs(dragPoint.y - dragStart.y)
        let growsRight = dragPoint.x >= dragStart.x
        let growsUp = dragPoint.y >= dragStart.y
        if square {
            // Follow the longer side, but stop at the screen edge in the direction of the drag.
            let roomX = growsRight ? bounds.maxX - dragStart.x : dragStart.x - bounds.minX
            let roomY = growsUp ? bounds.maxY - dragStart.y : dragStart.y - bounds.minY
            let side = min(max(width, height), roomX, roomY)
            width = side
            height = side
        }
        selection = NSRect(
            x: growsRight ? dragStart.x : dragStart.x - width,
            y: growsUp ? dragStart.y : dragStart.y - height,
            width: width,
            height: height
        )
    }

    override func mouseUp(with event: NSEvent) {
        guard dragStart != nil else { return }
        dragStart = nil
        dragPoint = nil
        if let selection, selection.width >= Self.minimumSize, selection.height >= Self.minimumSize {
            onComplete?(selection.integral)
        } else {
            onComplete?(nil)
        }
    }

    override func keyDown(with event: NSEvent) {
        if Int(event.keyCode) == kVK_Escape {
            onCancel?()
        } else {
            super.keyDown(with: event)
        }
    }

    override func draw(_ dirtyRect: NSRect) {
        let dim = NSBezierPath(rect: bounds)
        if let selection {
            dim.append(NSBezierPath(rect: selection))
            dim.windingRule = .evenOdd
        }
        NSColor.black.withAlphaComponent(0.4).setFill()
        dim.fill()

        if let selection {
            let border = NSBezierPath(rect: selection.insetBy(dx: -0.5, dy: -0.5))
            border.lineWidth = 1
            NSColor.white.setStroke()
            border.stroke()
            drawSizeLabel(for: selection)
        } else if dragStart == nil {
            drawHint()
        }
    }

    private func drawSizeLabel(for selection: NSRect) {
        let text = "\(Int(selection.width)) × \(Int(selection.height))" as NSString
        let attributes: [NSAttributedString.Key: Any] = [
            .font: NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .medium),
            .foregroundColor: NSColor.white,
        ]
        let size = text.size(withAttributes: attributes)
        let padding = NSSize(width: 8, height: 4)
        var origin = NSPoint(x: selection.minX, y: selection.minY - size.height - padding.height * 2 - 6)
        if origin.y < bounds.minY { origin.y = selection.minY + 6 } // no room below: put it inside
        origin.x = min(origin.x, bounds.maxX - size.width - padding.width * 2)

        let background = NSRect(origin: origin, size: NSSize(width: size.width + padding.width * 2, height: size.height + padding.height * 2))
        NSColor.black.withAlphaComponent(0.7).setFill()
        NSBezierPath(roundedRect: background, xRadius: 5, yRadius: 5).fill()
        text.draw(at: NSPoint(x: origin.x + padding.width, y: origin.y + padding.height), withAttributes: attributes)
    }

    private func drawHint() {
        let paragraph = NSMutableParagraphStyle()
        paragraph.alignment = .center
        paragraph.lineSpacing = 4
        let text = NSAttributedString(
            string: "Drag to select the area to record · Hold Shift for a square\nClick to record the whole screen · Esc to cancel",
            attributes: [
                .font: NSFont.systemFont(ofSize: 15, weight: .medium),
                .foregroundColor: NSColor.white,
                .paragraphStyle: paragraph,
            ]
        )
        let size = text.size()
        let padding = NSSize(width: 20, height: 14)
        let background = NSRect(
            x: bounds.midX - size.width / 2 - padding.width,
            y: bounds.midY - size.height / 2 - padding.height,
            width: size.width + padding.width * 2,
            height: size.height + padding.height * 2
        )
        NSColor.black.withAlphaComponent(0.6).setFill()
        NSBezierPath(roundedRect: background, xRadius: 10, yRadius: 10).fill()
        text.draw(in: background.insetBy(dx: padding.width, dy: padding.height))
    }

    /// Whole points, so a square selection stays square after `integral`.
    private func clamped(_ point: NSPoint) -> NSPoint {
        NSPoint(x: min(max(point.x.rounded(), bounds.minX), bounds.maxX), y: min(max(point.y.rounded(), bounds.minY), bounds.maxY))
    }
}

// MARK: - Recording frame

/// A dashed outline drawn just outside the recorded region while recording.
/// It never shows up in the video: the whole Cappa app is excluded from capture.
@MainActor
final class RegionFrameWindow {
    private let window: NSWindow

    init(screenRect: CGRect) {
        let frame = screenRect.insetBy(dx: -3, dy: -3)
        window = NSWindow(contentRect: frame, styleMask: .borderless, backing: .buffered, defer: false)
        window.setFrame(frame, display: false)
        window.isReleasedWhenClosed = false
        window.level = .floating
        window.backgroundColor = .clear
        window.isOpaque = false
        window.hasShadow = false
        window.ignoresMouseEvents = true
        window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        window.contentView = FrameBorderView(frame: NSRect(origin: .zero, size: frame.size))
    }

    func show() {
        window.orderFrontRegardless()
    }

    func close() {
        window.orderOut(nil)
    }
}

private final class FrameBorderView: NSView {
    override func draw(_ dirtyRect: NSRect) {
        let path = NSBezierPath(rect: bounds.insetBy(dx: 1, dy: 1))
        path.lineWidth = 2
        path.setLineDash([6, 4], count: 2, phase: 0)
        NSColor.systemRed.withAlphaComponent(0.85).setStroke()
        path.stroke()
    }
}
