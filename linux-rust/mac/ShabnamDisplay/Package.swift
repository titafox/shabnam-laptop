// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "ShabnamDisplay",
    platforms: [.macOS(.v14)],
    targets: [
        // Header-only target declaring CoreGraphics' unpublished virtual display classes.
        .target(name: "CVirtualDisplay"),
        .executableTarget(
            name: "ShabnamDisplay",
            dependencies: ["CVirtualDisplay"]
        ),
        .testTarget(name: "ShabnamDisplayTests", dependencies: ["ShabnamDisplay"]),
    ]
)
