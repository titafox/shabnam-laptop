// Turns the laptop's keyboard and touchpad events (Linux event codes, sent back over the video
// connection) into macOS input. Needs the Accessibility permission; without it macOS silently
// discards what we post.
import AppKit
import ApplicationServices
import CoreGraphics
import Foundation

/// Linux key code -> macOS virtual key code (US layout positions).
let keyMap: [UInt16: CGKeyCode] = [
    30: 0, 31: 1, 32: 2, 33: 3, 35: 4, 34: 5, 44: 6, 45: 7, 46: 8, 47: 9, 48: 11, 16: 12, 17: 13, 18: 14, 19: 15,
    21: 16, 20: 17, 2: 18, 3: 19, 4: 20, 5: 21, 7: 22, 6: 23, 13: 24, 10: 25, 8: 26, 12: 27, 9: 28, 11: 29,
    27: 30, 24: 31, 22: 32, 26: 33, 23: 34, 25: 35, 28: 36, 38: 37, 36: 38, 40: 39, 37: 40, 39: 41, 43: 42,
    51: 43, 53: 44, 49: 45, 50: 46, 52: 47, 15: 48, 57: 49, 41: 50, 14: 51, 1: 53, 86: 10,
    126: 54, 125: 55, 42: 56, 58: 57, 56: 58, 29: 59, 54: 60, 100: 61, 97: 62,
    59: 122, 60: 120, 61: 99, 62: 118, 63: 96, 64: 97, 65: 98, 66: 100, 67: 101, 68: 109, 87: 103, 88: 111,
    102: 115, 104: 116, 111: 117, 107: 119, 109: 121, 105: 123, 106: 124, 108: 125, 103: 126, 110: 114,
]
/// Modifier keys: Ctrl = Control, Alt = Option, the Windows/Super key = Command.
let modifierFlag: [UInt16: CGEventFlags] = [
    42: .maskShift, 54: .maskShift, 29: .maskControl, 97: .maskControl,
    56: .maskAlternate, 100: .maskAlternate, 125: .maskCommand, 126: .maskCommand,
]

/// Linux media key -> macOS system key (play/pause, next, previous).
// (The laptop keeps its own volume keys: they set its speaker volume.)
let mediaMap: [UInt16: Int] = [164: 16, 163: 17, 165: 18]

final class Injector {
    private let src = CGEventSource(stateID: .hidSystemState)
    private var flags: CGEventFlags = []
    private var held: [UInt16: Int] = [:]   // modifier key -> down count (left and right share a flag)
    private var caps = false
    private var pos = CGPoint.zero
    private var lastMove = Date.distantPast
    private var buttons: Set<UInt16> = []
    private var dx = 0, dy = 0, wheel = 0, hwheel = 0
    private var lastClick = (at: Date.distantPast, where_: CGPoint.zero, button: UInt16(0), count: 0)
    private var warned = false

    static func checkPermission() -> Bool {
        AXIsProcessTrustedWithOptions(["AXTrustedCheckOptionPrompt": true] as CFDictionary)
    }

    /// One record from the laptop. Motion is collected until the sync record that ends the batch.
    func handle(type: UInt16, code: UInt16, value: Int32) {
        if !warned, !AXIsProcessTrusted() {
            warned = true
            log("input arrived but the Accessibility permission is missing: System Settings > Privacy & Security > Accessibility")
        }
        switch type {
        case 1: code >= 0x110 && code <= 0x117 ? button(code, value != 0) : key(code, value)
        case 2:
            switch code {
            case 0: dx += Int(value)
            case 1: dy += Int(value)
            case 8: wheel += Int(value)
            case 6: hwheel += Int(value)
            default: break
            }
        case 0: flush()
        default: break
        }
    }

    /// Volume and playback keys are not ordinary keys on a Mac; they travel as "system" events.
    private func media(_ key: Int, _ down: Bool) {
        let state = down ? 0xA : 0xB
        let e = NSEvent.otherEvent(with: .systemDefined, location: .zero, modifierFlags: NSEvent.ModifierFlags(rawValue: UInt(state << 8)),
                                   timestamp: 0, windowNumber: 0, context: nil, subtype: 8, data1: (key << 16) | (state << 8), data2: -1)
        e?.cgEvent?.post(tap: .cghidEventTap)
    }

    private func key(_ code: UInt16, _ value: Int32) {
        if let m = mediaMap[code] { media(m, value != 0); return }
        guard let vk = keyMap[code] else { return }
        if let f = modifierFlag[code] {
            if value == 2 { return }
            held[code] = Int(value)
            let down = modifierFlag.contains { $0.value == f && (held[$0.key] ?? 0) != 0 }
            if down { flags.insert(f) } else { flags.remove(f) }
            post(vk, value != 0, type: .flagsChanged)
        } else if code == 58 {
            if value != 1 { return }
            caps.toggle()
            if caps { flags.insert(.maskAlphaShift) } else { flags.remove(.maskAlphaShift) }
            post(vk, caps, type: .flagsChanged)
        } else {
            post(vk, value != 0, repeating: value == 2)
        }
    }

    private func post(_ vk: CGKeyCode, _ down: Bool, type: CGEventType? = nil, repeating: Bool = false) {
        guard let e = CGEvent(keyboardEventSource: src, virtualKey: vk, keyDown: down) else { return }
        if let type { e.type = type }
        e.flags = flags
        if repeating { e.setIntegerValueField(.keyboardEventAutorepeat, value: 1) }
        e.post(tap: .cghidEventTap)
    }

    /// Where the pointer is. Re-read from the system after a pause so the Mac's own trackpad and
    /// the laptop's can be used alternately without the pointer jumping back.
    private func syncPosition() {
        if Date().timeIntervalSince(lastMove) > 0.3, let now = CGEvent(source: nil)?.location { pos = now }
        lastMove = Date()
    }

    private func mouse(_ type: CGEventType, _ button: CGMouseButton, dx: Int = 0, dy: Int = 0, clicks: Int = 0) {
        guard let e = CGEvent(mouseEventSource: src, mouseType: type, mouseCursorPosition: pos, mouseButton: button) else { return }
        e.flags = flags
        e.setIntegerValueField(.mouseEventDeltaX, value: Int64(dx))
        e.setIntegerValueField(.mouseEventDeltaY, value: Int64(dy))
        if clicks > 0 { e.setIntegerValueField(.mouseEventClickState, value: Int64(clicks)) }
        e.post(tap: .cghidEventTap)
    }

    private func button(_ code: UInt16, _ down: Bool) {
        flush()
        syncPosition()
        let (b, downType, upType): (CGMouseButton, CGEventType, CGEventType) =
            code == 0x110 ? (.left, .leftMouseDown, .leftMouseUp) : code == 0x111 ? (.right, .rightMouseDown, .rightMouseUp) : (.center, .otherMouseDown, .otherMouseUp)
        if down {
            let again = lastClick.button == code && Date().timeIntervalSince(lastClick.at) < 0.4 && hypot(pos.x - lastClick.where_.x, pos.y - lastClick.where_.y) < 5
            lastClick = (Date(), pos, code, again ? lastClick.count + 1 : 1)
            buttons.insert(code)
        } else {
            buttons.remove(code)
        }
        mouse(down ? downType : upType, b, clicks: lastClick.count)
    }

    private func flush() {
        if dx != 0 || dy != 0 {
            syncPosition()
            // Gentle acceleration: slow strokes stay precise, fast strokes cross both screens.
            let speed = hypot(Double(dx), Double(dy))
            let gain = min(1.0 + speed * 0.06, 3.0)
            var p = CGPoint(x: pos.x + Double(dx) * gain, y: pos.y + Double(dy) * gain)
            // Keep the pointer on some display: if the full move leaves all of them, try each axis alone.
            var n: UInt32 = 0
            let onScreen = { (q: CGPoint) -> Bool in CGGetDisplaysWithPoint(q, 0, nil, &n); return n > 0 }
            if !onScreen(p) {
                if onScreen(CGPoint(x: p.x, y: pos.y)) { p.y = pos.y } else if onScreen(CGPoint(x: pos.x, y: p.y)) { p.x = pos.x } else { p = pos }
            }
            pos = p
            let (type, b): (CGEventType, CGMouseButton) = buttons.contains(0x110) ? (.leftMouseDragged, .left) : buttons.contains(0x111) ? (.rightMouseDragged, .right) : buttons.contains(0x112) ? (.otherMouseDragged, .center) : (.mouseMoved, .left)
            mouse(type, b, dx: Int(Double(dx) * gain), dy: Int(Double(dy) * gain))
            dx = 0; dy = 0
        }
        if wheel != 0 || hwheel != 0 {
            if let e = CGEvent(scrollWheelEvent2Source: src, units: .line, wheelCount: 2, wheel1: Int32(wheel * 2), wheel2: Int32(hwheel * 2), wheel3: 0) {
                e.flags = flags
                e.post(tap: .cghidEventTap)
            }
            wheel = 0; hwheel = 0
        }
    }

    /// Called when the link drops: let go of anything still held so nothing stays stuck down.
    func releaseAll() {
        for (code, v) in held where v != 0 { key(code, 0) }
        for code in buttons { button(code, false) }
    }
}
