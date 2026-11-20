import AppKit
import SwiftUI

enum SettingsTab: String, CaseIterable {
    case general, display, audio, input, connection, permissions, about

    var title: String {
        switch self {
        case .general: return L("General")
        case .display: return L("Display")
        case .audio: return L("Sound")
        case .input: return L("Input")
        case .connection: return L("Connection")
        case .permissions: return L("Permissions")
        case .about: return L("About")
        }
    }

    var symbol: String {
        switch self {
        case .general: return "gearshape"
        case .display: return "display"
        case .audio: return "speaker.wave.2"
        case .input: return "keyboard"
        case .connection: return "cable.connector"
        case .permissions: return "lock.shield"
        case .about: return "info.circle"
        }
    }

    var height: CGFloat {
        switch self {
        case .general: return 330
        case .display: return 665
        case .audio: return 330
        case .input: return 520
        case .connection: return 680
        case .permissions: return 330
        case .about: return 300
        }
    }

    static let width: CGFloat = 580
}

/// The Settings window: a toolbar of panes, in the manner of macOS preference windows.
final class SettingsWindowController: NSWindowController, NSWindowDelegate {
    private let model: AppModel
    private let tabs = NSTabViewController()

    init(model: AppModel) {
        self.model = model
        tabs.tabStyle = .toolbar
        for tab in SettingsTab.allCases {
            let host = NSHostingController(rootView: SettingsPane(tab: tab).environmentObject(model).environmentObject(model.settings))
            host.title = tab.title
            host.preferredContentSize = NSSize(width: SettingsTab.width, height: tab.height)
            let item = NSTabViewItem(viewController: host)
            item.label = tab.title
            item.image = NSImage(systemSymbolName: tab.symbol, accessibilityDescription: tab.title)
            tabs.addTabViewItem(item)
        }
        let window = NSWindow(contentViewController: tabs)
        window.styleMask = [.titled, .closable, .miniaturizable]
        window.isReleasedWhenClosed = false
        window.title = L("Shabnam Display Settings")
        super.init(window: window)
        window.delegate = self
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    /// - activate: false leaves the frontmost app alone (used by the UI preview).
    func present(_ tab: SettingsTab? = nil, activate: Bool = true) {
        if let tab, let index = SettingsTab.allCases.firstIndex(of: tab) { tabs.selectedTabViewItemIndex = index }
        model.startRefreshing()
        guard let window else { return }
        if !window.isVisible { window.center() }
        if activate {
            NSApp.activate(ignoringOtherApps: true)
            window.makeKeyAndOrderFront(nil)
        } else {
            window.orderFrontRegardless()
        }
    }

    func windowWillClose(_ notification: Notification) {
        model.stopRefreshing()
    }
}

/// One pane of the Settings window.
struct SettingsPane: View {
    let tab: SettingsTab

    var body: some View {
        Group {
            switch tab {
            case .general: GeneralPane()
            case .display: DisplayPane()
            case .audio: AudioPane()
            case .input: InputPane()
            case .connection: ConnectionPane()
            case .permissions: PermissionsPane()
            case .about: AboutPane()
            }
        }
        .frame(width: SettingsTab.width, height: tab.height)
    }
}

/// Small grey explanatory text under a control.
struct Note: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text)
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// A coloured dot with a word next to it.
struct StatusLight: View {
    enum Level { case good, warning, unknown }
    let level: Level
    let text: String

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(level == .good ? Color.green : level == .warning ? Color.orange : Color.gray)
                .frame(width: 9, height: 9)
            Text(text).foregroundStyle(.secondary)
        }
    }
}
