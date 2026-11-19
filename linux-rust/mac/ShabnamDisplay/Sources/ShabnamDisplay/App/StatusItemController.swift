import AppKit

/// The menu bar item and its (deliberately short) menu. Everything else is in Settings.
final class StatusItemController: NSObject, NSMenuDelegate {
    private let session: SessionController
    private let settings = Settings.shared
    private let openSettings: (SettingsTab?) -> Void
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    private var state: LinkState = .stopped

    private let statusLine = NSMenuItem(title: "", action: nil, keyEquivalent: "")
    private let warningLine = NSMenuItem(title: "", action: #selector(openPermissions), keyEquivalent: "")
    private let connectItem = NSMenuItem(title: "", action: #selector(toggleConnection), keyEquivalent: "")
    private var modeItems: [(DisplayMode, NSMenuItem)] = []
    private var audioItems: [(AudioMode, NSMenuItem)] = []

    init(session: SessionController, openSettings: @escaping (SettingsTab?) -> Void) {
        self.session = session
        self.openSettings = openSettings
        super.init()

        let menu = NSMenu()
        menu.delegate = self
        menu.autoenablesItems = false
        statusLine.isEnabled = false
        menu.addItem(statusLine)
        menu.addItem(warningLine)
        menu.addItem(.separator())
        menu.addItem(connectItem)
        menu.addItem(.separator())

        let sound = NSMenuItem(title: L("Sound"), action: nil, keyEquivalent: "")
        let soundMenu = NSMenu()
        soundMenu.autoenablesItems = false
        for (mode, title) in [(AudioMode.both, L("Sound: Mac and Shabnam")), (.laptopOnly, L("Sound: Shabnam only (Mac muted)")), (.off, L("Sound: off"))] {
            let entry = NSMenuItem(title: title, action: #selector(chooseAudio(_:)), keyEquivalent: "")
            entry.representedObject = mode.rawValue
            if mode == .laptopOnly { entry.isEnabled = AudioMode.laptopOnlySupported } // needs macOS 14.2
            soundMenu.addItem(entry)
            audioItems.append((mode, entry))
        }
        sound.submenu = soundMenu
        menu.addItem(sound)

        let resolution = NSMenuItem(title: L("Resolution"), action: nil, keyEquivalent: "")
        let resolutionMenu = NSMenu()
        for mode in DisplayMode.allCases {
            let entry = NSMenuItem(title: mode.title, action: #selector(chooseMode(_:)), keyEquivalent: "")
            entry.representedObject = mode.rawValue
            resolutionMenu.addItem(entry)
            modeItems.append((mode, entry))
        }
        resolution.submenu = resolutionMenu
        menu.addItem(resolution)

        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: L("Settings…"), action: #selector(showSettings), keyEquivalent: ","))
        menu.addItem(NSMenuItem(title: L("Quit Shabnam Display"), action: #selector(quit), keyEquivalent: "q"))
        for entry in menu.items + resolutionMenu.items + soundMenu.items where entry.action != nil { entry.target = self }

        item.menu = menu
        item.button?.imagePosition = .imageLeading
        item.button?.setAccessibilityLabel(ProjectInfo.name)
        show(session.isRunning ? .waiting : .stopped)
    }

    // MARK: state

    /// Reflects the link state in the icon, the optional title and the first menu line. Main thread.
    func show(_ state: LinkState) {
        self.state = state
        refresh()
    }

    /// Redraws from the current state and settings.
    func refresh() {
        let text = state.localizedDescription
        statusLine.title = text
        item.button?.toolTip = ProjectInfo.name + " — " + text
        switch state {
        case .stopped: item.button?.image = StatusIcon.image(.off)
        case .waiting: item.button?.image = StatusIcon.image(.idle)
        case .connected: item.button?.image = StatusIcon.image(.active)
        case .problem: item.button?.image = StatusIcon.image(.attention)
        }
        var title = ""
        if settings.menuBarShowsDetails, case .connected(let rate?) = state {
            title = " " + L("%@ Mbit/s", LinkState.megabits(rate.megabitsPerSecond))
        }
        item.button?.title = title
        item.button?.font = .monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize, weight: .regular)
        connectItem.title = session.isRunning ? L("Disconnect") : L("Connect")
    }

    func menuNeedsUpdate(_ menu: NSMenu) {
        connectItem.title = session.isRunning ? L("Disconnect") : L("Connect")
        let mode = settings.displayMode, audio = settings.audioMode
        for (candidate, entry) in modeItems { entry.state = candidate == mode ? .on : .off }
        for (candidate, entry) in audioItems { entry.state = candidate == audio ? .on : .off }

        let screen = Permissions.screenRecordingGranted
        let input = Permissions.accessibilityGranted || !settings.acceptInput
        warningLine.isHidden = screen && input
        if !screen {
            warningLine.title = L("⚠ Screen Recording permission missing")
        } else if !input {
            warningLine.title = L("⚠ Accessibility permission missing (keyboard and touchpad)")
        }
    }

    // MARK: actions

    /// Connect and Disconnect act on this run only; whether the app starts connecting by itself
    /// is the "Connect automatically" setting.
    @objc private func toggleConnection() {
        if session.isRunning {
            session.stop()
        } else {
            session.start()
            show(.waiting)
        }
    }

    @objc private func chooseMode(_ sender: NSMenuItem) {
        guard let raw = sender.representedObject as? String, let mode = DisplayMode(rawValue: raw), settings.displayMode != mode else { return }
        settings.displayMode = mode // applied to the live display by the settings observer
    }

    @objc private func chooseAudio(_ sender: NSMenuItem) {
        guard let raw = sender.representedObject as? String, let mode = AudioMode(rawValue: raw), settings.audioMode != mode else { return }
        settings.audioMode = mode // takes effect at once; the picture is not interrupted
    }

    @objc private func showSettings() { openSettings(nil) }
    @objc private func openPermissions() { openSettings(.permissions) }
    @objc private func quit() { NSApp.terminate(nil) }
}
