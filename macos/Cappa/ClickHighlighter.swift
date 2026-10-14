import AppKit
import QuartzCore

/// Draws an expanding ring that fades from black to white wherever the mouse is clicked on one screen.
///
/// Unlike Cappa's other windows, this one is meant to be recorded: its window is passed to the
/// content filter as an exception to the app-wide exclusion. It is click-through, so it also shows
/// on screen without getting in the way.
@MainActor
final class ClickHighlighter {
    private static let startRadius: CGFloat = 6
    private static let endRadius: CGFloat = 30
    private static let lineWidth: CGFloat = 3
    private static let duration: CFTimeInterval = 0.5

    private let window: NSWindow
    private let screenFrame: CGRect
    private var monitors: [Any] = []

    /// The window server's ID for the window, to find it in `SCShareableContent`.
    var windowID: CGWindowID { CGWindowID(window.windowNumber) }

    init(screen: NSScreen) {
        screenFrame = screen.frame
        window = NSWindow(contentRect: screen.frame, styleMask: .borderless, backing: .buffered, defer: false)
        window.setFrame(screen.frame, display: false)
        window.isReleasedWhenClosed = false
        window.level = .screenSaver
        window.backgroundColor = .clear
        window.isOpaque = false
        window.hasShadow = false
        window.ignoresMouseEvents = true
        window.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .stationary]
        let view = NSView(frame: NSRect(origin: .zero, size: screen.frame.size))
        view.wantsLayer = true
        window.contentView = view
    }

    func show() {
        window.orderFrontRegardless()
        let clicks: NSEvent.EventTypeMask = [.leftMouseDown, .rightMouseDown, .otherMouseDown]
        // The global monitor sees clicks in other apps; the local one clicks in Cappa's own windows.
        if let global = NSEvent.addGlobalMonitorForEvents(matching: clicks, handler: { [weak self] _ in
            MainActor.assumeIsolated { self?.highlightClick() }
        }) {
            monitors.append(global)
        }
        if let local = NSEvent.addLocalMonitorForEvents(matching: clicks, handler: { [weak self] event in
            self?.highlightClick()
            return event
        }) {
            monitors.append(local)
        }
    }

    func close() {
        monitors.forEach(NSEvent.removeMonitor)
        monitors.removeAll()
        window.orderOut(nil)
    }

    private func highlightClick() {
        let mouse = NSEvent.mouseLocation
        guard screenFrame.contains(mouse), let root = window.contentView?.layer else { return }
        let center = CGPoint(x: mouse.x - screenFrame.minX, y: mouse.y - screenFrame.minY)

        let ring = CAShapeLayer()
        ring.fillColor = nil
        ring.lineWidth = Self.lineWidth
        // The final state, so nothing flashes back when the animations are removed.
        ring.path = Self.circle(center: center, radius: Self.endRadius)
        ring.strokeColor = NSColor.white.cgColor
        ring.opacity = 0
        root.addSublayer(ring)

        let grow = CABasicAnimation(keyPath: "path")
        grow.fromValue = Self.circle(center: center, radius: Self.startRadius)
        let color = CABasicAnimation(keyPath: "strokeColor")
        color.fromValue = NSColor.black.cgColor
        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = 1

        let group = CAAnimationGroup()
        group.animations = [grow, color, fade]
        group.duration = Self.duration
        group.timingFunction = CAMediaTimingFunction(name: .easeOut)

        CATransaction.begin()
        CATransaction.setCompletionBlock { ring.removeFromSuperlayer() }
        ring.add(group, forKey: "highlight")
        CATransaction.commit()
    }

    private static func circle(center: CGPoint, radius: CGFloat) -> CGPath {
        CGPath(ellipseIn: CGRect(x: center.x - radius, y: center.y - radius, width: radius * 2, height: radius * 2), transform: nil)
    }
}
