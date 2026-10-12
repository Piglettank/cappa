import AVFoundation

/// The microphones currently connected, kept up to date as devices are plugged in or removed.
@MainActor
final class MicrophoneList: ObservableObject {
    struct Device: Identifiable, Hashable {
        let id: String // AVCaptureDevice.uniqueID
        let name: String
    }

    @Published private(set) var devices: [Device] = []
    @Published private(set) var defaultDeviceName: String?

    private var observers: [NSObjectProtocol] = []

    init() {
        refresh()
        for name in [AVCaptureDevice.wasConnectedNotification, AVCaptureDevice.wasDisconnectedNotification] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: nil, queue: .main) { [weak self] _ in
                Task { @MainActor in self?.refresh() }
            })
        }
    }

    deinit {
        observers.forEach(NotificationCenter.default.removeObserver)
    }

    func refresh() {
        let types: [AVCaptureDevice.DeviceType]
        if #available(macOS 14.0, *) {
            types = [.microphone, .external]
        } else {
            types = [.builtInMicrophone, .externalUnknown]
        }
        devices = AVCaptureDevice.DiscoverySession(deviceTypes: types, mediaType: .audio, position: .unspecified)
            .devices
            .map { Device(id: $0.uniqueID, name: $0.localizedName) }
        defaultDeviceName = AVCaptureDevice.default(for: .audio)?.localizedName
    }
}
