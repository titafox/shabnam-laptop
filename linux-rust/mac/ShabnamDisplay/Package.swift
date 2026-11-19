// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "ShabnamDisplay",
    platforms: [.macOS(.v14)],
    targets: [
        // Header-only target declaring CoreGraphics' unpublished virtual display classes.
        .target(name: "CVirtualDisplay"),
        // System audio capture through a Core Audio process tap ("sound only on the laptop").
        .target(name: "ShabnamAudio"),
        .executableTarget(
            name: "ShabnamDisplay",
            dependencies: ["CVirtualDisplay", "ShabnamAudio"]
        ),
        // Diagnostic: creates the tap for a few seconds and reports what it captured.
        .executableTarget(name: "audio-tap-probe", dependencies: ["ShabnamAudio"]),
        .testTarget(name: "ShabnamDisplayTests", dependencies: ["ShabnamDisplay"]),
    ]
)
