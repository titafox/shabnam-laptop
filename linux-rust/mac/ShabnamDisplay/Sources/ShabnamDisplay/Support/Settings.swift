import Foundation

/// User preferences, stored in the app's defaults domain.
final class Settings {
    static let shared = Settings()

    private let defaults = UserDefaults.standard
    private enum Key {
        static let autoConnect = "autoConnect"
        static let hiDPI = "hiDPI"
        static let sendAudio = "sendAudio"
        static let host = "host"
        static let port = "port"
    }

    private init() {
        defaults.register(defaults: [
            Key.autoConnect: true,
            Key.hiDPI: true,
            Key.sendAudio: true,
            Key.host: "169.254.1.1",
            Key.port: 7000,
        ])
    }

    /// Whether the app keeps looking for the laptop (the Connect/Disconnect toggle).
    var autoConnect: Bool {
        get { defaults.bool(forKey: Key.autoConnect) }
        set { defaults.set(newValue, forKey: Key.autoConnect) }
    }

    /// true: the desktop is drawn at 3840x2160 and averaged down 2:1 (sharper text).
    /// false: the desktop is drawn at exactly 1920x1080.
    var hiDPI: Bool {
        get { defaults.bool(forKey: Key.hiDPI) }
        set { defaults.set(newValue, forKey: Key.hiDPI) }
    }

    /// Whether the Mac's sound output is sent to the laptop's speakers.
    var sendAudio: Bool {
        get { defaults.bool(forKey: Key.sendAudio) }
        set { defaults.set(newValue, forKey: Key.sendAudio) }
    }

    /// The laptop's address on the cable. Not exposed in the UI; change with `defaults write`.
    var host: String { defaults.string(forKey: Key.host) ?? "169.254.1.1" }

    /// Video and input port; sound uses the next one up.
    var port: UInt16 { UInt16(clamping: defaults.integer(forKey: Key.port)) }
}
