import Foundation

/// Facts about the project that appear in the UI. Change them here.
enum ProjectInfo {
    static let name = "Shabnam Display"
    static let licence = "MIT"
    static let copyright = "Copyright © 2026 Shabnam contributors"

    /// The project's web page, shown in Settings → About. nil until there is one.
    static let homepage: URL? = nil

    /// Stamped into Info.plist by scripts/bundle.sh from the VERSION file; "dev" when the binary
    /// runs outside an app bundle.
    static var version: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "dev"
    }

    static var build: String {
        Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "0"
    }
}
