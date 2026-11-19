// Regenerates Resources/AppIcon.icns from Resources/mark.png:
//     swift scripts/make-icon.swift
// The result is checked in, so this only needs running when the artwork changes.
import AppKit

let root = URL(fileURLWithPath: CommandLine.arguments[0]).deletingLastPathComponent().deletingLastPathComponent()
let resources = root.appendingPathComponent("Resources")
guard let mark = NSImage(contentsOf: resources.appendingPathComponent("mark.png")) else { fatalError("Resources/mark.png not found") }

func render(_ pixels: Int) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8, samplesPerPixel: 4,
                               hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    NSGraphicsContext.current?.imageInterpolation = .high
    let s = CGFloat(pixels)
    // macOS icon shape: a rounded square inset from the canvas, on a deep navy gradient.
    let plate = NSRect(x: s * 0.1, y: s * 0.1, width: s * 0.8, height: s * 0.8)
    let path = NSBezierPath(roundedRect: plate, xRadius: s * 0.18, yRadius: s * 0.18)
    NSGradient(starting: NSColor(srgbRed: 0.07, green: 0.16, blue: 0.27, alpha: 1),
               ending: NSColor(srgbRed: 0.03, green: 0.08, blue: 0.15, alpha: 1))!.draw(in: path, angle: -90)
    let h = plate.height * 0.66, w = h * mark.size.width / mark.size.height
    mark.draw(in: NSRect(x: plate.midX - w / 2, y: plate.midY - h / 2, width: w, height: h))
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

let iconset = FileManager.default.temporaryDirectory.appendingPathComponent("AppIcon.iconset")
try? FileManager.default.removeItem(at: iconset)
try FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
for size in [16, 32, 128, 256, 512] {
    try render(size).write(to: iconset.appendingPathComponent("icon_\(size)x\(size).png"))
    try render(size * 2).write(to: iconset.appendingPathComponent("icon_\(size)x\(size)@2x.png"))
}
let tool = Process()
tool.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
tool.arguments = ["-c", "icns", iconset.path, "-o", resources.appendingPathComponent("AppIcon.icns").path]
try tool.run()
tool.waitUntilExit()
try? FileManager.default.removeItem(at: iconset)
print(tool.terminationStatus == 0 ? "wrote Resources/AppIcon.icns" : "iconutil failed")
