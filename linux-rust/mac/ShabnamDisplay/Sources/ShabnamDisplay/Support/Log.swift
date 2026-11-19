import Foundation
import os

private let logger = Logger(subsystem: Bundle.main.bundleIdentifier ?? "shabnam.display", category: "link")

/// Diagnostic output: standard error (visible when the binary is started from a terminal), the
/// unified log (Console.app) and a small rotating file. Never pass key material to this function.
func log(_ message: String) {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    logger.notice("\(message, privacy: .public)")
    LogFile.shared.append(message)
}

/// `~/Library/Logs/ShabnamDisplay/shabnam-display.log`, rotated at about 1 MB, three old files kept.
final class LogFile {
    static let shared = LogFile(
        directory: FileManager.default.urls(for: .libraryDirectory, in: .userDomainMask)[0].appendingPathComponent("Logs/ShabnamDisplay", isDirectory: true),
        // Not from the UI preview or the unit tests: they must leave no traces.
        enabled: !CommandLine.arguments.contains("--ui-preview") && NSClassFromString("XCTestCase") == nil)

    static let displayPath = "~/Library/Logs/ShabnamDisplay/"

    let directory: URL
    private let enabled: Bool
    private let maxBytes: Int
    private let keep: Int
    private let queue = DispatchQueue(label: "log-file")
    private var handle: FileHandle?
    private var size = 0
    private let stamp: DateFormatter = {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yyyy-MM-dd HH:mm:ss.SSS"
        return f
    }()

    init(directory: URL, enabled: Bool = true, maxBytes: Int = 1_000_000, keep: Int = 3) {
        self.directory = directory
        self.enabled = enabled
        self.maxBytes = maxBytes
        self.keep = keep
    }

    private var current: URL { directory.appendingPathComponent("shabnam-display.log") }
    private func old(_ n: Int) -> URL { directory.appendingPathComponent("shabnam-display.\(n).log") }

    func append(_ message: String) {
        guard enabled else { return }
        let now = Date()
        queue.async {
            let line = Data("\(self.stamp.string(from: now)) \(Diagnostics.redact(message))\n".utf8)
            if self.handle == nil { self.open() }
            if self.size + line.count > self.maxBytes { self.rotate() }
            try? self.handle?.write(contentsOf: line)
            self.size += line.count
        }
    }

    /// Waits until everything logged so far is on disk (tests, and just before exit).
    func flush() { queue.sync { try? handle?.synchronize() } }

    private func open() {
        let fm = FileManager.default
        try? fm.createDirectory(at: directory, withIntermediateDirectories: true)
        if !fm.fileExists(atPath: current.path) { fm.createFile(atPath: current.path, contents: nil, attributes: [.posixPermissions: 0o600]) }
        handle = try? FileHandle(forWritingTo: current)
        size = Int((try? handle?.seekToEnd()) ?? 0)
    }

    private func rotate() {
        try? handle?.close()
        handle = nil
        let fm = FileManager.default
        try? fm.removeItem(at: old(keep))
        for n in stride(from: keep - 1, through: 1, by: -1) { try? fm.moveItem(at: old(n), to: old(n + 1)) }
        try? fm.moveItem(at: current, to: old(1))
        open()
    }
}
