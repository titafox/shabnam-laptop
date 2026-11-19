import Foundation
import os

private let logger = Logger(subsystem: Bundle.main.bundleIdentifier ?? "shabnam.display", category: "link")

/// Diagnostic output: standard error (visible when the binary is started from a terminal) and the
/// unified log (Console.app). Never pass key material to this function.
func log(_ message: String) {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    logger.notice("\(message, privacy: .public)")
}
