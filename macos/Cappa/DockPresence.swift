import AppKit

/// Cappa lives in the menu bar, but while one of its regular windows (an editor, Settings) is open
/// it shows in the Dock and ⌘-Tab like a normal app, and goes back to the menu bar when the last one closes.
@MainActor
enum DockPresence {
    private static var openWindows = 0

    static func windowOpened() {
        openWindows += 1
        NSApp.setActivationPolicy(.regular)
    }

    static func windowClosed() {
        openWindows = max(openWindows - 1, 0)
        if openWindows == 0 {
            NSApp.setActivationPolicy(.accessory)
        }
    }
}
