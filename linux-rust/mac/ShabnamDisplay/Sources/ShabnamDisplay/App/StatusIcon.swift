import AppKit

/// The menu bar icon: a water droplet ("shabnam" is dew), drawn in code as a template image so
/// macOS tints it for light and dark menu bars.
enum StatusIcon {
    enum Style {
        /// Link switched off: faint outline.
        case off
        /// Waiting for the laptop: outline.
        case idle
        /// Streaming: filled.
        case active
        /// Something needs attention: outline with a mark inside.
        case attention
    }

    static func image(_ style: Style) -> NSImage {
        let image = NSImage(size: NSSize(width: 18, height: 18), flipped: false) { _ in
            let drop = NSBezierPath()
            drop.move(to: NSPoint(x: 9, y: 16.8))
            drop.curve(to: NSPoint(x: 14.3, y: 6.8), controlPoint1: NSPoint(x: 9.9, y: 13.8), controlPoint2: NSPoint(x: 14.3, y: 10.6))
            drop.curve(to: NSPoint(x: 9, y: 1.5), controlPoint1: NSPoint(x: 14.3, y: 3.87), controlPoint2: NSPoint(x: 11.93, y: 1.5))
            drop.curve(to: NSPoint(x: 3.7, y: 6.8), controlPoint1: NSPoint(x: 6.07, y: 1.5), controlPoint2: NSPoint(x: 3.7, y: 3.87))
            drop.curve(to: NSPoint(x: 9, y: 16.8), controlPoint1: NSPoint(x: 3.7, y: 10.6), controlPoint2: NSPoint(x: 8.1, y: 13.8))
            drop.close()
            drop.lineWidth = 1.4
            drop.lineJoinStyle = .round

            switch style {
            case .active:
                NSColor.black.setFill()
                drop.fill()
            case .idle:
                NSColor.black.setStroke()
                drop.stroke()
            case .off:
                NSColor.black.withAlphaComponent(0.45).setStroke()
                drop.stroke()
            case .attention:
                NSColor.black.setStroke()
                drop.stroke()
                NSColor.black.setFill()
                NSBezierPath(roundedRect: NSRect(x: 8.3, y: 6.6, width: 1.4, height: 4.4), xRadius: 0.7, yRadius: 0.7).fill()
                NSBezierPath(ovalIn: NSRect(x: 8.2, y: 3.9, width: 1.6, height: 1.6)).fill()
            }
            return true
        }
        image.isTemplate = true
        return image
    }
}
