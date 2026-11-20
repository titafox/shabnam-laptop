import AppKit
import ShabnamAudio

final class AppDelegate: NSObject, NSApplicationDelegate {
    private static let showSettings = Notification.Name("org.shabnam.display.show-settings")

    private let settings = Settings.shared
    private var session: SessionController?
    private var model: AppModel!
    private var settingsWindow: SettingsWindowController!
    private var statusItem: StatusItemController?
    private var signalSources: [DispatchSourceSignal] = []

    /// `--ui-preview [pane]`: show the Settings window and do nothing else.
    private let preview: SettingsTab? = {
        guard let i = CommandLine.arguments.firstIndex(of: "--ui-preview") else { return nil }
        return CommandLine.arguments.dropFirst(i + 1).first.flatMap(SettingsTab.init) ?? .general
    }()

    func applicationDidFinishLaunching(_ notification: Notification) {
        if CommandLine.arguments.contains("--probe-display-modes") {
            VirtualDisplay.probeModes { exit(0) }
            return
        }
        if let preview {
            model = AppModel(session: nil)
            model.laptopControl.onChange = { [weak self] status in self?.model.laptop = status }
            // `--fake-laptop`: show the laptop's controls as if one were connected.
            if CommandLine.arguments.contains("--fake-laptop") {
                model.state = .connected(.init(framesPerSecond: 60, megabitsPerSecond: 42))
                model.laptopControl.showFake(LaptopStatus(brightness: 63, volume: 9, volumeSteps: 16, muted: false, battery: 82, charging: true,
                                                          externalPower: true, lidClosed: false, receiverVersion: "0.2.0", protocolVersion: 1))
            }
            settingsWindow = SettingsWindowController(model: model)
            settingsWindow.present(preview, activate: false)
            return
        }
        if handOverToRunningInstance() { exit(0) }

        // If an earlier run was killed while it held the system audio tap, clear what it left behind.
        if #available(macOS 14.2, *) { SystemAudioTap.removeLeftoverTaps() }
        log("\(ProjectInfo.name) \(ProjectInfo.version) (\(ProjectInfo.build)) started")

        let session = SessionController()
        self.session = session
        model = AppModel(session: session)
        settingsWindow = SettingsWindowController(model: model)
        let statusItem = StatusItemController(session: session, openSettings: { [weak self] tab in self?.settingsWindow.present(tab) })
        self.statusItem = statusItem

        session.onStateChange = { [weak self] new in self?.stateChanged(to: new) }
        session.laptop.onChange = { [weak self] status in
            self?.statusItem?.showLaptop(status)
            self?.model.laptop = status
        }
        observeSystem()
        installSignalHandlers()

        if settings.autoConnect { session.start() }
        // First run, or the app's signature changed: explain what is needed instead of failing quietly.
        if !Permissions.screenRecordingGranted {
            // Asking once is what makes macOS list the app under Screen Recording at all;
            // without it the user has to add the app by hand with the "+" button.
            Permissions.requestScreenRecording()
            settingsWindow.present(.permissions)
        }
    }

    func applicationWillTerminate(_ notification: Notification) {
        session?.releaseAudioNow()
        session?.stop()
        LogFile.shared.flush()
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { preview != nil }

    // MARK: single instance

    /// Two copies would fight over the laptop. If one is already running, ask it to show its
    /// Settings window and step aside.
    private func handOverToRunningInstance() -> Bool {
        guard let id = Bundle.main.bundleIdentifier else { return false }
        let others = NSRunningApplication.runningApplications(withBundleIdentifier: id).filter { $0 != NSRunningApplication.current }
        guard let first = others.first else { return false }
        DistributedNotificationCenter.default().postNotificationName(Self.showSettings, object: id, userInfo: nil, deliverImmediately: true)
        first.activate()
        return true
    }

    // MARK: reacting to the world

    private var wasConnected = false

    private func stateChanged(to new: LinkState) {
        statusItem?.show(new)
        model.refresh()
        if new.isConnected != wasConnected {
            wasConnected = new.isConnected
            if settings.notifications {
                Notifier.post(new.isConnected ? L("The laptop is connected as a second screen.") : L("The laptop disconnected."))
            }
        }
    }

    private func observeSystem() {
        let center = NotificationCenter.default
        center.addObserver(forName: Settings.didChange, object: settings, queue: .main) { [weak self] note in
            guard let self, let key = note.userInfo?["key"] as? Settings.Key else { return }
            self.session?.settingChanged(key)
            switch key {
            case .autoConnect where self.settings.autoConnect:
                self.session?.start()
                self.statusItem?.refresh()
            case .notifications where self.settings.notifications:
                Notifier.requestAuthorization()
            case .menuBarShowsDetails:
                self.statusItem?.refresh()
            default:
                break
            }
        }
        // A sleeping Mac drops the cable link anyway; end the session cleanly beforehand and
        // start looking again after waking.
        let workspace = NSWorkspace.shared.notificationCenter
        workspace.addObserver(forName: NSWorkspace.willSleepNotification, object: nil, queue: .main) { [weak self] _ in
            log("the Mac is going to sleep — ending the session")
            self?.session?.suspend()
        }
        workspace.addObserver(forName: NSWorkspace.didWakeNotification, object: nil, queue: .main) { [weak self] _ in
            log("the Mac woke up")
            self?.session?.resume()
        }
        center.addObserver(forName: NSApplication.didChangeScreenParametersNotification, object: nil, queue: .main) { [weak self] _ in
            self?.session?.displaysChanged()
        }
        DistributedNotificationCenter.default().addObserver(forName: Self.showSettings, object: Bundle.main.bundleIdentifier, queue: .main) { [weak self] _ in
            self?.settingsWindow.present()
        }
    }

    /// Being told to stop from outside (kill, logout, an installer) must not leave the Mac muted
    /// in "Shabnam only" sound mode: un-mute first, then go. The virtual display belongs to this
    /// process and disappears with it.
    private func installSignalHandlers() {
        for number in [SIGTERM, SIGINT, SIGHUP] {
            signal(number, SIG_IGN)
            let source = DispatchSource.makeSignalSource(signal: number, queue: .main)
            source.setEventHandler { [weak self] in
                self?.session?.releaseAudioNow()
                log("terminated by signal \(number)")
                LogFile.shared.flush()
                exit(0)
            }
            source.resume()
            signalSources.append(source)
        }
    }
}
