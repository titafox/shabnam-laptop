import CoreGraphics
import CVirtualDisplay
import Foundation

/// An extra screen that exists only in software. As far as macOS is concerned it is a real
/// 14-inch monitor called "Shabnam"; it disappears when this object is released.
///
/// Built on `CGVirtualDisplay`, a CoreGraphics class Apple ships but does not document.
/// Create, use and release on the main thread.
final class VirtualDisplay {
    /// The laptop panel's size; pixels map 1:1, nothing is scaled on the way.
    static let width = 1920
    static let height = 1080
    static let refreshRate = 60

    private let display: CGVirtualDisplay
    var displayID: CGDirectDisplayID { display.displayID }

    /// - name, serial: only the mode probe uses other values, so that it cannot be mistaken
    ///   for the real display by macOS (which remembers arrangements per display identity).
    init?(mode: DisplayMode, name: String = "Shabnam", serial: UInt32 = 1) {
        let descriptor = CGVirtualDisplayDescriptor()
        descriptor.setDispatchQueue(DispatchQueue.main)
        descriptor.name = name
        descriptor.maxPixelsWide = UInt32(Self.width * 2) // room for double-precision ("Retina") drawing
        descriptor.maxPixelsHigh = UInt32(Self.height * 2)
        descriptor.sizeInMillimeters = CGSize(width: 309, height: 174) // 14-inch 16:9 panel
        descriptor.productID = 0x4E42
        descriptor.vendorID = 0x4543
        descriptor.serialNum = serial
        guard let display = CGVirtualDisplay(descriptor: descriptor) else { return nil }

        let settings = CGVirtualDisplaySettings()
        settings.hiDPI = 1 // offer each size in a double-precision variant too
        settings.modes = [1920, 1600, 1280, 1024, 960].map {
            CGVirtualDisplayMode(width: UInt($0), height: UInt($0 * 9 / 16), refreshRate: Double(Self.refreshRate))
        }
        guard display.apply(settings) else { return nil }
        self.display = display
        select(mode)
    }

    /// macOS may start a new display in any of its modes; select the one asked for.
    ///
    /// Right after creation the display's mode list can still be empty, so a selection that
    /// finds nothing is tried again for a few seconds. Main thread.
    func select(_ mode: DisplayMode) {
        wanted = mode
        attempt(mode, triesLeft: 20)
    }

    private var wanted: DisplayMode?

    private func attempt(_ mode: DisplayMode, triesLeft: Int) {
        guard wanted == mode else { return } // superseded by a newer choice
        let id = displayID
        let all = CGDisplayCopyAllDisplayModes(id, [kCGDisplayShowDuplicateLowResolutionModes: true] as CFDictionary) as? [CGDisplayMode] ?? []
        let w = mode.pointWidth
        let scale = mode.isHiDPI ? 2 : 1
        // The exact variant if offered; otherwise the same apparent size at the other precision.
        guard let match = all.first(where: { $0.width == w && $0.pixelWidth == w * scale }) ?? all.first(where: { $0.width == w }) else {
            if all.isEmpty, triesLeft > 0 {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { [weak self] in self?.attempt(mode, triesLeft: triesLeft - 1) }
            } else {
                log("no \(w)-wide display mode; offered: " + all.map { "\($0.width)pt/\($0.pixelWidth)px" }.joined(separator: " "))
            }
            return
        }
        var config: CGDisplayConfigRef?
        CGBeginDisplayConfiguration(&config)
        CGConfigureDisplayWithDisplayMode(config, id, match, nil)
        let r = CGCompleteDisplayConfiguration(config, .forSession)
        if r != .success { log("display mode switch failed: \(r.rawValue)") }
        if let now = current { log("virtual display \(id): \(now.points.width)x\(now.points.height) points, \(now.pixels.width)x\(now.pixels.height) pixels") }
    }

    /// What the display is set to right now.
    var current: (points: (width: Int, height: Int), pixels: (width: Int, height: Int))? {
        CGDisplayCopyDisplayMode(displayID).map { (($0.width, $0.height), ($0.pixelWidth, $0.pixelHeight)) }
    }

    /// Developer check (`--probe-display-modes`): creates a short-lived display under another
    /// identity and reports, for every `DisplayMode`, what macOS really sets it to. Runs inside
    /// the started app, like a real session; calls `done` when finished.
    static func probeModes(done: @escaping () -> Void) {
        guard let probe = VirtualDisplay(mode: .hiDPI1080, name: "Shabnam Probe", serial: 0xFFFE) else {
            print("could not create a virtual display")
            return done()
        }
        var remaining = DisplayMode.allCases[...]
        func next() {
            guard let mode = remaining.popFirst() else { return done() }
            probe.select(mode)
            DispatchQueue.main.asyncAfter(deadline: .now() + 1) {
                let got = probe.current
                let ok = got.map { $0.points.width == mode.pointWidth && $0.pixels.width == mode.pointWidth * (mode.isHiDPI ? 2 : 1) } ?? false
                print("\(mode.rawValue): \(ok ? "OK" : "NOT AVAILABLE") — \(got.map { "\($0.points.width)x\($0.points.height) points, \($0.pixels.width)x\($0.pixels.height) pixels" } ?? "no mode")")
                next()
            }
        }
        next()
    }
}
