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

    init?(hiDPI: Bool) {
        let descriptor = CGVirtualDisplayDescriptor()
        descriptor.setDispatchQueue(DispatchQueue.main)
        descriptor.name = "Shabnam"
        descriptor.maxPixelsWide = UInt32(Self.width * 2) // room for double-precision ("Retina") drawing
        descriptor.maxPixelsHigh = UInt32(Self.height * 2)
        descriptor.sizeInMillimeters = CGSize(width: 309, height: 174) // 14-inch 16:9 panel
        descriptor.productID = 0x4E42
        descriptor.vendorID = 0x4543
        descriptor.serialNum = 1
        guard let display = CGVirtualDisplay(descriptor: descriptor) else { return nil }

        let settings = CGVirtualDisplaySettings()
        settings.hiDPI = 1 // offer each size in a double-precision variant too
        settings.modes = [1920, 1600, 1280, 1024, 960].map {
            CGVirtualDisplayMode(width: UInt($0), height: UInt($0 * 9 / 16), refreshRate: Double(Self.refreshRate))
        }
        guard display.apply(settings) else { return nil }
        self.display = display
        selectMode(hiDPI: hiDPI)
    }

    /// macOS may start a new display in any of its modes; select the one asked for.
    /// hiDPI: the desktop is drawn at 3840x2160 and fitted to the panel 2:1. Otherwise 1920x1080 as is.
    func selectMode(hiDPI: Bool) {
        let id = displayID
        let all = CGDisplayCopyAllDisplayModes(id, [kCGDisplayShowDuplicateLowResolutionModes: true] as CFDictionary) as? [CGDisplayMode] ?? []
        let w = Self.width
        let wanted = hiDPI
            ? all.first { $0.width == w && $0.pixelWidth == w * 2 } ?? all.first { $0.width == w }
            : all.first { $0.width == w && $0.pixelWidth == w } ?? all.first { $0.width == w }
        guard let mode = wanted else {
            log("no \(w)-wide display mode; offered: " + all.map { "\($0.width)pt/\($0.pixelWidth)px" }.joined(separator: " "))
            return
        }
        var config: CGDisplayConfigRef?
        CGBeginDisplayConfiguration(&config)
        CGConfigureDisplayWithDisplayMode(config, id, mode, nil)
        let r = CGCompleteDisplayConfiguration(config, .forSession)
        if r != .success { log("display mode switch failed: \(r.rawValue)") }
        if let m = CGDisplayCopyDisplayMode(id) {
            log("virtual display \(id): \(m.width)x\(m.height) points, \(m.pixelWidth)x\(m.pixelHeight) pixels")
        }
    }
}
