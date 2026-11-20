import AppKit
import SwiftUI
import UniformTypeIdentifiers

struct ConnectionPane: View {
    @EnvironmentObject private var model: AppModel
    @EnvironmentObject private var settings: Settings
    @State private var host = ""
    @State private var port = ""
    @State private var copied = false

    var body: some View {
        Form {
            Section(L("Laptop")) {
                TextField(L("Address"), text: $host)
                    .onSubmit(commitAddress)
                TextField(L("Port"), text: $port)
                    .onSubmit(commitAddress)
                LabeledContent(L("Sound port"), value: "\(Int(settings.port) + 1)")
                LabeledContent(L("Cable interface"), value: model.cable.map { "\($0.name) · \($0.address)" } ?? L("Cable not connected"))
                HStack {
                    if !addressIsValid {
                        Text(L("Enter an IPv4 address and a port between 1 and 65534.")).font(.callout).foregroundStyle(.orange)
                    } else if addressIsDirty {
                        Button(L("Apply"), action: commitAddress)
                    }
                    Spacer()
                    Button(L("Restore Defaults")) {
                        settings.restoreConnectionDefaults()
                        loadAddress()
                    }
                    .disabled(settings.host == Settings.defaultHost && settings.port == Settings.defaultPort)
                }
                Note(L("Used for the next connection. Connections are only ever made through the cable interface."))
            }
            Section(L("Pairing key")) {
                LabeledContent(L("File"), value: PairingKey.displayPath)
                LabeledContent(L("Status"), value: keyStatus)
                HStack {
                    Button(L("Reveal in Finder")) { NSWorkspace.shared.activateFileViewerSelecting([PairingKey.url]) }
                        .disabled(model.keyFile == .missing)
                    Button(L("Import from File…"), action: importKey)
                    Button(L("Generate new key…"), action: generateKey)
                }
                Note(L("The laptop needs an identical copy of this file (the one given to subscreen-rx with --key). Copy it over a channel you trust, for example with scp across the cable; never share it. Its contents are never shown here."))
            }
            Section(L("Statistics")) {
                LabeledContent(L("State"), value: model.state.description(laptop: model.laptop))
                LabeledContent(L("Connected for"), value: uptime)
                LabeledContent(L("Video"), value: L("%d fps · %@ Mbit/s", Int(model.stats.fps.rounded()), LinkState.megabits(model.stats.megabitsPerSecond)))
                LabeledContent(L("Frames dropped by the encoder"), value: "\(model.stats.framesDropped)")
                LabeledContent(L("Sound sent"), value: ByteCountFormatter.string(fromByteCount: Int64(model.stats.audioBytes), countStyle: .binary))
                LabeledContent(L("Last error"), value: model.stats.lastError.map { Diagnostics.redact($0) } ?? L("None"))
                HStack {
                    Button(L("Copy Diagnostics")) {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(model.diagnostics, forType: .string)
                        copied = true
                    }
                    if copied { Text(L("Copied")).foregroundStyle(.secondary) }
                    Spacer()
                    Button(L("Show Log Files")) { NSWorkspace.shared.open(LogFile.shared.directory) }
                }
                Note(L("The diagnostics text contains no key material, user name, computer name or home folder path."))
            }
        }
        .formStyle(.grouped)
        .onAppear(perform: loadAddress)
    }

    private var addressIsValid: Bool { Settings.isValidHost(host) && (Int(port).map(Settings.isValidPort) ?? false) }
    private var addressIsDirty: Bool { host != settings.host || port != String(settings.port) }

    private func loadAddress() {
        host = settings.host
        port = String(settings.port)
    }

    private func commitAddress() {
        guard addressIsValid, let number = UInt16(port) else { return }
        settings.host = host
        settings.port = number
    }

    private var keyStatus: String {
        switch model.keyFile {
        case .missing: return L("No key file yet.")
        case .tooShort: return L("The key file is too short to be used.")
        case .readableByOthers: return L("Key file present, but other users of this Mac can read it.")
        case .present: return L("Key file present.")
        }
    }

    private var uptime: String {
        guard let since = model.stats.connectedSince else { return "—" }
        let seconds = Int(Date().timeIntervalSince(since))
        return String(format: "%d:%02d:%02d", seconds / 3600, seconds / 60 % 60, seconds % 60)
    }

    private func confirmReplacingKey() -> Bool {
        guard model.keyFile != .missing else { return true }
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = L("Replace the pairing key?")
        alert.informativeText = L("The current key will be replaced. The laptop will refuse this Mac until both hold the same key file.")
        alert.addButton(withTitle: L("Replace"))
        alert.addButton(withTitle: L("Cancel"))
        return alert.runModal() == .alertFirstButtonReturn
    }

    private func generateKey() {
        guard confirmReplacingKey() else { return }
        do {
            try PairingKey.generate()
            log("a new pairing key was generated")
        } catch {
            failed()
        }
        model.refresh()
    }

    private func importKey() {
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.showsHiddenFiles = true
        panel.message = L("Choose the key file to use (at least 32 bytes).")
        guard panel.runModal() == .OK, let url = panel.url, confirmReplacingKey() else { return }
        do {
            try PairingKey.importKey(from: url)
            log("a pairing key was imported")
        } catch PairingKey.GenerateError.tooShort {
            failed(L("The key file is too short to be used."))
        } catch {
            failed()
        }
        model.refresh()
    }

    private func failed(_ detail: String = "") {
        let alert = NSAlert()
        alert.messageText = L("Could not write the key file")
        alert.informativeText = detail
        alert.runModal()
    }
}
