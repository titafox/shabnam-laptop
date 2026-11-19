import AppKit

/// The menu bar item and its menu.
final class StatusItemController: NSObject, NSMenuDelegate {
    private let session: SessionController
    private let settings = Settings.shared
    private let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)

    private let statusLine = NSMenuItem(title: "", action: nil, keyEquivalent: "")
    private let warningLine = NSMenuItem(title: "", action: #selector(openPermissions), keyEquivalent: "")
    private let connectItem = NSMenuItem(title: "", action: #selector(toggleConnection), keyEquivalent: "")
    private let hiDPIItem = NSMenuItem(title: "", action: #selector(chooseHiDPI), keyEquivalent: "")
    private let plainItem = NSMenuItem(title: "", action: #selector(choosePlain), keyEquivalent: "")
    private let audioItem = NSMenuItem(title: "", action: #selector(toggleAudio), keyEquivalent: "")
    private let loginItem = NSMenuItem(title: "", action: #selector(toggleLoginItem), keyEquivalent: "")

    private lazy var pairingWindow = PairingKeyWindowController()
    private lazy var permissionsWindow = PermissionsWindowController()

    init(session: SessionController) {
        self.session = session
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

        let resolution = NSMenuItem(title: L("Resolution"), action: nil, keyEquivalent: "")
        let resolutionMenu = NSMenu()
        hiDPIItem.title = L("1920 × 1080 HiDPI (sharper)")
        plainItem.title = L("1920 × 1080")
        resolutionMenu.addItem(hiDPIItem)
        resolutionMenu.addItem(plainItem)
        resolution.submenu = resolutionMenu
        menu.addItem(resolution)

        audioItem.title = L("Send audio")
        menu.addItem(audioItem)
        loginItem.title = L("Launch at login")
        menu.addItem(loginItem)
        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: L("Pairing key…"), action: #selector(openPairingKey), keyEquivalent: ""))
        menu.addItem(NSMenuItem(title: L("Permissions…"), action: #selector(openPermissions), keyEquivalent: ""))
        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: L("Quit Shabnam Display"), action: #selector(quit), keyEquivalent: "q"))
        for entry in menu.items + resolutionMenu.items where entry.action != nil { entry.target = self }

        item.menu = menu
        item.button?.setAccessibilityLabel("Shabnam Display")
        show(session.isRunning ? .waiting : .stopped)
    }

    // MARK: state

    /// Reflects the link state in the icon and the first menu line. Main thread.
    func show(_ state: LinkState) {
        let text = Self.describe(state)
        statusLine.title = text
        item.button?.toolTip = "Shabnam Display — " + text
        switch state {
        case .stopped: item.button?.image = StatusIcon.image(.off)
        case .waiting: item.button?.image = StatusIcon.image(.idle)
        case .connected: item.button?.image = StatusIcon.image(.active)
        case .problem: item.button?.image = StatusIcon.image(.attention)
        }
        connectItem.title = session.isRunning ? L("Disconnect") : L("Connect")
    }

    static func describe(_ state: LinkState) -> String {
        let format = "\(VirtualDisplay.height)p\(VirtualDisplay.refreshRate)"
        switch state {
        case .stopped:
            return L("Disconnected")
        case .waiting:
            return L("Waiting for laptop")
        case .connected(let mbit):
            guard let mbit else { return L("Connected · %@", format) }
            return L("Connected · %@ · %@ Mbit/s", format, String(format: mbit < 10 ? "%.1f" : "%.0f", mbit))
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

    func menuNeedsUpdate(_ menu: NSMenu) {
        connectItem.title = session.isRunning ? L("Disconnect") : L("Connect")
        hiDPIItem.state = settings.hiDPI ? .on : .off
        plainItem.state = settings.hiDPI ? .off : .on
        audioItem.state = settings.sendAudio ? .on : .off
        loginItem.state = LoginItem.isEnabled ? .on : .off

        let screen = Permissions.screenRecordingGranted, input = Permissions.accessibilityGranted
        warningLine.isHidden = screen && input
        if !screen {
            warningLine.title = L("⚠ Screen Recording permission missing")
        } else if !input {
            warningLine.title = L("⚠ Accessibility permission missing (keyboard and touchpad)")
        }
    }

    // MARK: actions

    @objc private func toggleConnection() {
        if session.isRunning {
            settings.autoConnect = false
            session.stop()
        } else {
            settings.autoConnect = true
            session.start()
            show(.waiting)
        }
    }

    @objc private func chooseHiDPI() { setHiDPI(true) }
    @objc private func choosePlain() { setHiDPI(false) }

    private func setHiDPI(_ on: Bool) {
        guard settings.hiDPI != on else { return }
        settings.hiDPI = on
        session.resolutionSettingChanged()
    }

    @objc private func toggleAudio() {
        settings.sendAudio.toggle()
        session.audioSettingChanged()
    }

    @objc private func toggleLoginItem() {
        do {
            try LoginItem.setEnabled(!LoginItem.isEnabled)
        } catch {
            let alert = NSAlert()
            alert.messageText = L("Could not change the login item")
            alert.informativeText = error.localizedDescription
            NSApp.activate(ignoringOtherApps: true)
            alert.runModal()
        }
    }

    @objc private func openPairingKey() { pairingWindow.present() }
    @objc private func openPermissions() { showPermissions() }
    func showPermissions() { permissionsWindow.present() }

    @objc private func quit() { NSApp.terminate(nil) }
}
