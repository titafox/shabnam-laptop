import CryptoKit
import Foundation
import Security

/// The secret shared with the laptop. It lives in a file in the user's home directory and is only
/// ever read into memory for the handshake; it is never shown, logged or copied anywhere else.
enum PairingKey {
    static let minimumLength = 32

    /// How the location is shown to the user.
    static let displayPath = "~/.config/subscreen/key"

    static var url: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent(".config/subscreen/key", isDirectory: false)
    }

    static var exists: Bool { FileManager.default.fileExists(atPath: url.path) }

    /// true if only the owner can read the file.
    static var isPrivate: Bool {
        guard let mode = (try? FileManager.default.attributesOfItem(atPath: url.path))?[.posixPermissions] as? Int else { return false }
        return mode & 0o077 == 0
    }

    /// Reads the key for one connection attempt. nil if the file is missing or too short.
    static func load() -> SymmetricKey? {
        guard let data = FileManager.default.contents(atPath: url.path), data.count >= minimumLength else { return nil }
        return SymmetricKey(data: data)
    }

    enum GenerateError: Error { case random, write, tooShort }

    /// Replaces the key file with 32 fresh random bytes, readable by the owner only. The laptop
    /// must be given the same file afterwards or it will refuse the connection.
    static func generate() throws {
        var bytes = Data(count: minimumLength)
        let status = bytes.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, minimumLength, $0.baseAddress!) }
        guard status == errSecSuccess else { throw GenerateError.random }
        try install(bytes)
    }

    /// Replaces the key file with a copy of another file (for example the laptop's key, brought
    /// over on a USB stick). The file must hold at least 32 bytes.
    static func importKey(from source: URL) throws {
        let bytes = try Data(contentsOf: source)
        guard bytes.count >= minimumLength else { throw GenerateError.tooShort }
        try install(bytes)
    }

    private static func install(_ bytes: Data) throws {
        let fm = FileManager.default
        let dir = url.deletingLastPathComponent()
        try fm.createDirectory(at: dir, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        let tmp = dir.appendingPathComponent(".key.\(ProcessInfo.processInfo.processIdentifier).tmp")
        try? fm.removeItem(at: tmp)
        guard fm.createFile(atPath: tmp.path, contents: bytes, attributes: [.posixPermissions: 0o600]) else { throw GenerateError.write }
        guard rename(tmp.path, url.path) == 0 else {
            try? fm.removeItem(at: tmp)
            throw GenerateError.write
        }
    }
}
