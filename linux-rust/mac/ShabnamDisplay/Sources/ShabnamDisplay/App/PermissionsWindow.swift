import AppKit

/// "Permissions…": shows whether the two macOS permissions are granted and leads to the right
/// panes of System Settings. The app never tries to work around these prompts.
final class PermissionsWindowController: NSWindowController, NSWindowDelegate {
    private let screenStatus = NSTextField(labelWithString: "")
    private let inputStatus = NSTextField(labelWithString: "")
    private var timer: Timer?

    init() {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 480, height: 10),
                              styleMask: [.titled, .closable], backing: .buffered, defer: true)
        window.title = L("Permissions")
        window.isReleasedWhenClosed = false
        super.init(window: window)
        window.delegate = self

        let screen = row(title: L("Screen Recording"),
                         detail: L("To capture the extra screen and the Mac's sound and send them to the laptop."),
                         status: screenStatus, action: #selector(grantScreenRecording))
        let input = row(title: L("Accessibility"),
                        detail: L("To let the laptop's keyboard and touchpad control this Mac."),
                        status: inputStatus, action: #selector(grantAccessibility))
        let note = NSTextField(wrappingLabelWithString: L("After allowing Screen Recording, macOS may ask you to quit and reopen the app. If the app was rebuilt, remove its old entry from the list and add it again."))
        note.textColor = .secondaryLabelColor
        note.font = .systemFont(ofSize: NSFont.smallSystemFontSize)

        let stack = NSStackView(views: [screen, input, note])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 16
        stack.edgeInsets = NSEdgeInsets(top: 20, left: 20, bottom: 20, right: 20)
        stack.translatesAutoresizingMaskIntoConstraints = false
        let content = NSView()
        content.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: content.topAnchor),
            stack.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            stack.widthAnchor.constraint(equalToConstant: 480),
            screen.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -40),
            input.widthAnchor.constraint(equalTo: stack.widthAnchor, constant: -40),
        ])
        window.contentView = content
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    private func row(title: String, detail: String, status: NSTextField, action: Selector) -> NSView {
        let name = NSTextField(labelWithString: title)
        name.font = .boldSystemFont(ofSize: NSFont.systemFontSize)
        let heading = NSStackView(views: [name, status])
        heading.orientation = .horizontal
        heading.spacing = 8
        let text = NSTextField(wrappingLabelWithString: detail)
        text.textColor = .secondaryLabelColor
        let left = NSStackView(views: [heading, text])
        left.orientation = .vertical
        left.alignment = .leading
        left.spacing = 3
        let button = NSButton(title: L("Open Settings…"), target: self, action: action)
        button.setContentHuggingPriority(.required, for: .horizontal)
        button.setContentCompressionResistancePriority(.required, for: .horizontal)
        let row = NSStackView(views: [left, button])
        row.orientation = .horizontal
        row.alignment = .centerY
        row.distribution = .fill
        return row
    }

    func present() {
        refresh()
        if timer == nil {
            timer = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in self?.refresh() }
        }
        window?.center()
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
    }

    func windowWillClose(_ notification: Notification) {
        timer?.invalidate()
        timer = nil
    }

    private func refresh() {
        show(Permissions.screenRecordingGranted, in: screenStatus)
        show(Permissions.accessibilityGranted, in: inputStatus)
    }

    private func show(_ granted: Bool, in label: NSTextField) {
        label.stringValue = granted ? L("Granted") : L("Not granted")
        label.textColor = granted ? .systemGreen : .systemOrange
    }

    /// The first click makes macOS show its own prompt (it only does so once); the Settings pane
    /// is opened as well so the switch can be found afterwards.
    @objc private func grantScreenRecording() {
        if !Permissions.screenRecordingGranted { Permissions.requestScreenRecording() }
        Permissions.openScreenRecordingSettings()
    }

    @objc private func grantAccessibility() {
        if !Permissions.accessibilityGranted { Permissions.requestAccessibility() }
        Permissions.openAccessibilitySettings()
    }
}
