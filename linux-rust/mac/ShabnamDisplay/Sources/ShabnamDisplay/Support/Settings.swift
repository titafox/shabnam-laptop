import Combine
import Foundation

/// The size the desktop appears to have on the laptop's 1920 × 1080 panel.
enum DisplayMode: String, CaseIterable {
    /// Drawn at 3840 × 2160 and averaged down 2:1 — the sharpest text. Default.
    case hiDPI1080
    /// Drawn at exactly 1920 × 1080.
    case plain1080
    /// Looks like 1600 × 900 (drawn at 3200 × 1800): larger text.
    case hiDPI900
    /// Looks like 1280 × 720 (drawn at 2560 × 1440): much larger text.
    case hiDPI720

    var pointWidth: Int {
        switch self {
        case .hiDPI1080, .plain1080: return 1920
        case .hiDPI900: return 1600
        case .hiDPI720: return 1280
        }
    }
    var pointHeight: Int { pointWidth * 9 / 16 }
    var isHiDPI: Bool { self != .plain1080 }
}

/// How many bits the encoder may spend. The cable carries about 300 Mbit/s.
enum QualityPreset: String, CaseIterable {
    case sharpest, balanced, economy

    /// Target average bitrate, bits per second.
    var averageBitRate: Int {
        switch self {
        case .sharpest: return 120_000_000
        case .balanced: return 60_000_000
        case .economy: return 25_000_000
        }
    }
    /// Ceiling on how coarsely any frame may be quantised (lower = sharper).
    var maxQP: Int {
        switch self {
        case .sharpest: return 18
        case .balanced: return 24
        case .economy: return 30
        }
    }
}

/// How the laptop's keyboard and touchpad are replayed.
struct InputOptions: Equatable {
    /// false: Win → Command, Alt → Option. true: Win → Option, Alt → Command.
    var swapWinAlt = false
    var reverseScroll = false
    var scrollSpeed = 1.0
    var pointerSpeed = 1.0
    var mediaKeys = true
}

/// User preferences, stored in the app's defaults domain. Setters are for the main thread;
/// getters may be used from any thread.
final class Settings: ObservableObject {
    /// The app's settings. The UI preview uses a separate domain so that looking at the
    /// Settings window of a development build can never change an installed copy's settings.
    static let shared: Settings = {
        if CommandLine.arguments.contains("--ui-preview"), let preview = UserDefaults(suiteName: previewDomain) {
            return Settings(defaults: preview)
        }
        return Settings(defaults: .standard)
    }()
    static let previewDomain = "org.shabnam.display.preview"

    /// Posted on the main thread after a value changed; `userInfo["key"]` is the `Key`.
    static let didChange = Notification.Name("ShabnamSettingsDidChange")

    static let defaultHost = "169.254.1.1"
    static let defaultPort: UInt16 = 7000
    static let speedRange = 0.25...3.0

    enum Key: String {
        case autoConnect, menuBarShowsDetails, notifications
        case displayMode, frameRate, quality, showCursor
        case audioMode
        case acceptInput, swapWinAlt, reverseScroll, scrollSpeed, pointerSpeed, mediaKeys
        case host, port
        // Earlier versions; read once for migration.
        case hiDPI, sendAudio
    }

    private let defaults: UserDefaults
    private let revisionLock = NSLock()
    private var _revision = 0

    init(defaults: UserDefaults) {
        self.defaults = defaults
        defaults.register(defaults: [
            Key.autoConnect.rawValue: true,
            Key.menuBarShowsDetails.rawValue: false,
            Key.notifications.rawValue: false,
            Key.frameRate.rawValue: 60,
            Key.showCursor.rawValue: true,
            Key.acceptInput.rawValue: true,
            Key.swapWinAlt.rawValue: false,
            Key.reverseScroll.rawValue: false,
            Key.scrollSpeed.rawValue: 1.0,
            Key.pointerSpeed.rawValue: 1.0,
            Key.mediaKeys.rawValue: true,
            Key.host.rawValue: Settings.defaultHost,
            Key.port.rawValue: Int(Settings.defaultPort),
        ])
    }

    /// Goes up by one with every change; lets other threads notice cheaply that they should re-read.
    var revision: Int {
        revisionLock.lock(); defer { revisionLock.unlock() }
        return _revision
    }

    private func set(_ value: Any, _ key: Key) {
        objectWillChange.send()
        defaults.set(value, forKey: key.rawValue)
        revisionLock.lock(); _revision += 1; revisionLock.unlock()
        NotificationCenter.default.post(name: Settings.didChange, object: self, userInfo: ["key": key])
    }

    private func bool(_ key: Key) -> Bool { defaults.bool(forKey: key.rawValue) }

    // MARK: General

    /// Start looking for the laptop as soon as the app launches.
    var autoConnect: Bool {
        get { bool(.autoConnect) }
        set { set(newValue, .autoConnect) }
    }

    /// Show bitrate next to the menu bar icon while connected.
    var menuBarShowsDetails: Bool {
        get { bool(.menuBarShowsDetails) }
        set { set(newValue, .menuBarShowsDetails) }
    }

    /// Post a notification when the laptop connects or disconnects.
    var notifications: Bool {
        get { bool(.notifications) }
        set { set(newValue, .notifications) }
    }

    // MARK: Display

    var displayMode: DisplayMode {
        get {
            if let raw = defaults.string(forKey: Key.displayMode.rawValue), let mode = DisplayMode(rawValue: raw) { return mode }
            // Carry over the old HiDPI on/off switch.
            if defaults.object(forKey: Key.hiDPI.rawValue) != nil, !bool(.hiDPI) { return .plain1080 }
            return .hiDPI1080
        }
        set { set(newValue.rawValue, .displayMode) }
    }

    /// Frames per second at most: 60 or 30.
    var frameRate: Int {
        get { defaults.integer(forKey: Key.frameRate.rawValue) == 30 ? 30 : 60 }
        set { set(newValue == 30 ? 30 : 60, .frameRate) }
    }

    var quality: QualityPreset {
        get { defaults.string(forKey: Key.quality.rawValue).flatMap(QualityPreset.init) ?? .sharpest }
        set { set(newValue.rawValue, .quality) }
    }

    /// Whether the Mac's pointer is drawn into the picture sent to the laptop.
    var showCursor: Bool {
        get { bool(.showCursor) }
        set { set(newValue, .showCursor) }
    }

    // MARK: Audio

    /// Where the Mac's sound is heard while connected: both machines, the laptop only, or not sent.
    var audioMode: AudioMode {
        get {
            if let raw = defaults.string(forKey: Key.audioMode.rawValue), let mode = AudioMode(rawValue: raw) { return mode }
            // Carry over the old on/off switch.
            if defaults.object(forKey: Key.sendAudio.rawValue) != nil, !bool(.sendAudio) { return .off }
            return .both
        }
        set { set(newValue.rawValue, .audioMode) }
    }

    // MARK: Input

    /// false: the laptop's keyboard and touchpad events are read and thrown away.
    var acceptInput: Bool {
        get { bool(.acceptInput) }
        set { set(newValue, .acceptInput) }
    }

    var swapWinAlt: Bool {
        get { bool(.swapWinAlt) }
        set { set(newValue, .swapWinAlt) }
    }

    var reverseScroll: Bool {
        get { bool(.reverseScroll) }
        set { set(newValue, .reverseScroll) }
    }

    var scrollSpeed: Double {
        get { Self.clampSpeed(defaults.double(forKey: Key.scrollSpeed.rawValue)) }
        set { set(Self.clampSpeed(newValue), .scrollSpeed) }
    }

    var pointerSpeed: Double {
        get { Self.clampSpeed(defaults.double(forKey: Key.pointerSpeed.rawValue)) }
        set { set(Self.clampSpeed(newValue), .pointerSpeed) }
    }

    var mediaKeys: Bool {
        get { bool(.mediaKeys) }
        set { set(newValue, .mediaKeys) }
    }

    var inputOptions: InputOptions {
        InputOptions(swapWinAlt: swapWinAlt, reverseScroll: reverseScroll, scrollSpeed: scrollSpeed,
                     pointerSpeed: pointerSpeed, mediaKeys: mediaKeys)
    }

    static func clampSpeed(_ value: Double) -> Double {
        guard value.isFinite, value > 0 else { return 1 }
        return min(max(value, speedRange.lowerBound), speedRange.upperBound)
    }

    // MARK: Connection

    /// The laptop's address on the cable (dotted IPv4).
    var host: String {
        get {
            let stored = defaults.string(forKey: Key.host.rawValue) ?? Settings.defaultHost
            return Settings.isValidHost(stored) ? stored : Settings.defaultHost
        }
        set { if Settings.isValidHost(newValue) { set(newValue, .host) } }
    }

    /// Video and input port; sound uses the next one up.
    var port: UInt16 {
        get {
            let stored = defaults.integer(forKey: Key.port.rawValue)
            return Settings.isValidPort(stored) ? UInt16(stored) : Settings.defaultPort
        }
        set { if Settings.isValidPort(Int(newValue)) { set(Int(newValue), .port) } }
    }

    func restoreConnectionDefaults() {
        host = Settings.defaultHost
        port = Settings.defaultPort
    }

    static func isValidHost(_ text: String) -> Bool {
        var address = in_addr()
        return inet_pton(AF_INET, text, &address) == 1
    }

    /// The sound port is this plus one, so the very last port is not usable.
    static func isValidPort(_ value: Int) -> Bool { (1...65534).contains(value) }
}
