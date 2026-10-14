import Foundation
import ServiceManagement

/// Whether Cappa starts when you log in. The system's login items are the source of truth (the user can
/// also change it in System Settings → General → Login Items), so nothing is stored in UserDefaults.
@MainActor
final class LaunchAtLogin: ObservableObject {
    @Published private(set) var isEnabled = false
    /// Turned on, but waiting for the user to allow it in System Settings.
    @Published private(set) var needsApproval = false

    init() {
        refresh()
    }

    func refresh() {
        let status = SMAppService.mainApp.status
        isEnabled = status == .enabled || status == .requiresApproval
        needsApproval = status == .requiresApproval
    }

    func setEnabled(_ enabled: Bool) {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            NSLog("Cappa: couldn't \(enabled ? "enable" : "disable") launch at login: \(error)")
        }
        refresh()
    }

    func openLoginItemsSettings() {
        SMAppService.openSystemSettingsLoginItems()
    }
}
