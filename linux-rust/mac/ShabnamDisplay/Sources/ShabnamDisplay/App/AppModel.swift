import AppKit
import Combine

/// What the Settings window shows besides the settings themselves: link state, statistics,
/// permissions and other facts that change underneath it. Main thread only.
final class AppModel: ObservableObject {
    enum KeyFileStatus { case missing, tooShort, readableByOthers, present }

    /// nil in the UI preview, where nothing is ever connected.
    let session: SessionController?
    let settings: Settings

    @Published var state: LinkState = .stopped
    @Published var stats = SessionStats.Snapshot()
    @Published var screenRecordingGranted = false
    @Published var accessibilityGranted = false
    @Published var loginItemEnabled = false
    @Published var cable: CableInterface?
    @Published var keyFile = KeyFileStatus.missing

    private var timer: Timer?

    init(session: SessionController?, settings: Settings = .shared) {
        self.session = session
        self.settings = settings
        refresh()
    }

    /// Re-reads everything that can change without the app being told.
    func refresh() {
        if let session {
            state = session.state
            stats = session.stats.snapshot
        }
        screenRecordingGranted = Permissions.screenRecordingGranted
        accessibilityGranted = Permissions.accessibilityGranted
        loginItemEnabled = LoginItem.isEnabled
        cable = CableInterface.find()
        keyFile = !PairingKey.exists ? .missing : PairingKey.load() == nil ? .tooShort : PairingKey.isPrivate ? .present : .readableByOthers
    }

    /// Keeps the figures fresh while the Settings window is open.
    func startRefreshing() {
        refresh()
        guard timer == nil else { return }
        timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in self?.refresh() }
    }

    func stopRefreshing() {
        timer?.invalidate()
        timer = nil
    }

    func setLoginItem(_ enabled: Bool) {
        do {
            try LoginItem.setEnabled(enabled)
        } catch {
            let alert = NSAlert()
            alert.messageText = L("Could not change the login item")
            alert.informativeText = error.localizedDescription
            alert.runModal()
        }
        loginItemEnabled = LoginItem.isEnabled
    }

    var diagnostics: String {
        Diagnostics.report(state: state, stats: stats, settings: settings, cable: cable)
    }
}

extension LinkState {
    /// The status line: menu, tooltip and Settings.
    var localizedDescription: String {
        let format = "\(VirtualDisplay.height)p"
        switch self {
        case .stopped:
            return L("Disconnected")
        case .waiting:
            return L("Waiting for laptop")
        case .connected(let rate):
            guard let rate else { return L("Connected · %@", format) }
            return L("Connected · %@ · %d fps · %@ Mbit/s", format, rate.framesPerSecond, LinkState.megabits(rate.megabitsPerSecond))
        case .problem(let problem):
            switch problem {
            case .screenRecordingMissing: return L("Error: Screen Recording permission needed")
            case .noPairingKey: return L("Error: no pairing key")
            case .authenticationFailed: return L("Error: laptop has a different pairing key")
            case .displayFailed: return L("Error: could not create the display")
            case .encoderFailed: return L("Error: video encoder unavailable")
            case .captureFailed: return L("Error: screen capture failed")
            }
        }
    }

    static func megabits(_ value: Double) -> String {
        String(format: value < 10 ? "%.1f" : "%.0f", value)
    }
}
