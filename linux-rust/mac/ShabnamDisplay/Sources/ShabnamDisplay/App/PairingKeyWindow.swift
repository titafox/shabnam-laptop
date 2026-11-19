import AppKit

/// "Pairing key…": says where the key file is and lets the user reveal or replace it.
/// The key's contents are never shown.
final class PairingKeyWindowController: NSWindowController {
    private let statusLabel = NSTextField(wrappingLabelWithString: "")
    private let revealButton = NSButton(title: "", target: nil, action: nil)

    init() {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 460, height: 10),
                              styleMask: [.titled, .closable], backing: .buffered, defer: true)
        window.title = L("Pairing key")
        window.isReleasedWhenClosed = false
        super.init(window: window)

        let intro = NSTextField(wrappingLabelWithString: L("The Mac and the laptop prove to each other that they hold the same secret key before any picture or keystroke is exchanged. The key is stored in this file:"))
        let path = NSTextField(labelWithString: PairingKey.displayPath)
        path.font = .monospacedSystemFont(ofSize: NSFont.systemFontSize, weight: .regular)
        path.isSelectable = true
        statusLabel.textColor = .secondaryLabelColor
        let hint = NSTextField(wrappingLabelWithString: L("The laptop needs an identical copy of this file. Copy it over a channel you trust; never share it."))
        hint.textColor = .secondaryLabelColor
        hint.font = .systemFont(ofSize: NSFont.smallSystemFontSize)

        revealButton.title = L("Reveal in Finder")
        revealButton.target = self
        revealButton.action = #selector(reveal)
        let generate = NSButton(title: L("Generate new key…"), target: self, action: #selector(generate))
        let buttons = NSStackView(views: [revealButton, generate])
        buttons.orientation = .horizontal

        let stack = NSStackView(views: [intro, path, statusLabel, hint, buttons])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 10
        stack.edgeInsets = NSEdgeInsets(top: 20, left: 20, bottom: 20, right: 20)
        stack.translatesAutoresizingMaskIntoConstraints = false
        let content = NSView()
        content.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.topAnchor.constraint(equalTo: content.topAnchor),
            stack.bottomAnchor.constraint(equalTo: content.bottomAnchor),
            stack.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            stack.widthAnchor.constraint(equalToConstant: 460),
        ])
        window.contentView = content
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError("not used") }

    func present() {
        refresh()
        window?.center()
        NSApp.activate(ignoringOtherApps: true)
        window?.makeKeyAndOrderFront(nil)
    }

    private func refresh() {
        if !PairingKey.exists {
            statusLabel.stringValue = L("No key file yet.")
        } else if PairingKey.load() == nil {
            statusLabel.stringValue = L("The key file is too short to be used.")
        } else if !PairingKey.isPrivate {
            statusLabel.stringValue = L("Key file present, but other users of this Mac can read it.")
        } else {
            statusLabel.stringValue = L("Key file present.")
        }
        revealButton.isEnabled = PairingKey.exists
    }

    @objc private func reveal() {
        NSWorkspace.shared.activateFileViewerSelecting([PairingKey.url])
    }

    @objc private func generate() {
        guard let window else { return }
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = L("Generate a new pairing key?")
        alert.informativeText = L("The current key will be replaced. The laptop will refuse this Mac until the new key file has been copied to it.")
        alert.addButton(withTitle: L("Generate"))
        alert.addButton(withTitle: L("Cancel"))
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn else { return }
            do {
                try PairingKey.generate()
                log("a new pairing key was generated")
            } catch {
                let failure = NSAlert()
                failure.messageText = L("Could not write the key file")
                failure.runModal()
            }
            self?.refresh()
        }
    }
}
