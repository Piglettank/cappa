import AppKit
import SwiftUI

/// One editor window per video. While it's open Cappa shows in the Dock (see `DockPresence`).
@MainActor
final class EditorWindowController: NSWindowController, NSWindowDelegate {
    private static var openEditors: [EditorWindowController] = []

    private let model: EditorModel

    static func open(url: URL) {
        if let existing = openEditors.first(where: { $0.model.url == url }) {
            existing.window?.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return
        }

        let editor = EditorWindowController(url: url)
        openEditors.append(editor)
        DockPresence.windowOpened()
        editor.window?.center()
        editor.showWindow(nil)
        NSApp.activate(ignoringOtherApps: true)
    }

    private init(url: URL) {
        model = EditorModel(url: url)

        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1100, height: 760),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = url.deletingPathExtension().lastPathComponent
        window.representedURL = url
        window.minSize = NSSize(width: 720, height: 520)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(rootView: EditorView(model: model))

        super.init(window: window)
        window.delegate = self

        Task { await model.load() }
    }

    required init?(coder: NSCoder) {
        fatalError("init(coder:) is not supported")
    }

    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard model.hasUnexportedEdits else { return true }

        let alert = NSAlert()
        alert.messageText = "Close without exporting?"
        alert.informativeText = "Your edits haven't been exported and will be lost. The original recording is kept."
        alert.addButton(withTitle: "Close")
        alert.addButton(withTitle: "Cancel")
        return alert.runModal() == .alertFirstButtonReturn
    }

    func windowWillClose(_ notification: Notification) {
        model.close()
        Self.openEditors.removeAll { $0 === self }
        DockPresence.windowClosed()
    }
}
