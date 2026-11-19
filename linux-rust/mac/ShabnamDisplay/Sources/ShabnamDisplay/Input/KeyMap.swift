import CoreGraphics

/// Translation tables from the laptop's Linux input event codes to macOS key codes.
enum KeyMap {
    /// Linux key code -> macOS virtual key code (US layout positions).
    static let keys: [UInt16: CGKeyCode] = [
        30: 0, 31: 1, 32: 2, 33: 3, 35: 4, 34: 5, 44: 6, 45: 7, 46: 8, 47: 9, 48: 11, 16: 12, 17: 13, 18: 14, 19: 15,
        21: 16, 20: 17, 2: 18, 3: 19, 4: 20, 5: 21, 7: 22, 6: 23, 13: 24, 10: 25, 8: 26, 12: 27, 9: 28, 11: 29,
        27: 30, 24: 31, 22: 32, 26: 33, 23: 34, 25: 35, 28: 36, 38: 37, 36: 38, 40: 39, 37: 40, 39: 41, 43: 42,
        51: 43, 53: 44, 49: 45, 50: 46, 52: 47, 15: 48, 57: 49, 41: 50, 14: 51, 1: 53, 86: 10,
        126: 54, 125: 55, 42: 56, 58: 57, 56: 58, 29: 59, 54: 60, 100: 61, 97: 62,
        59: 122, 60: 120, 61: 99, 62: 118, 63: 96, 64: 97, 65: 98, 66: 100, 67: 101, 68: 109, 87: 103, 88: 111,
        102: 115, 104: 116, 111: 117, 107: 119, 109: 121, 105: 123, 106: 124, 108: 125, 103: 126, 110: 114,
    ]
    /// Modifier keys: Ctrl = Control, Alt = Option, the Windows/Super key = Command.
    static let modifierFlags: [UInt16: CGEventFlags] = [
        42: .maskShift, 54: .maskShift, 29: .maskControl, 97: .maskControl,
        56: .maskAlternate, 100: .maskAlternate, 125: .maskCommand, 126: .maskCommand,
    ]

    /// Linux media key -> macOS system key (play/pause, next, previous).
    // (The laptop keeps its own volume keys: they set its speaker volume.)
    static let mediaKeys: [UInt16: Int] = [164: 16, 163: 17, 165: 18]

    // Linux codes of the keys that can be swapped.
    static let leftMeta: UInt16 = 125, rightMeta: UInt16 = 126, leftAlt: UInt16 = 56, rightAlt: UInt16 = 100

    /// Applies the user's modifier layout to a Linux key code before it is looked up.
    /// Default: Win → Command, Alt → Option. Swapped: Win → Option, Alt → Command (the order
    /// of the keys on a Mac keyboard). Ctrl is always Control. Left stays left, right stays right.
    static func remap(_ code: UInt16, swapWinAlt: Bool) -> UInt16 {
        guard swapWinAlt else { return code }
        switch code {
        case leftMeta: return leftAlt
        case leftAlt: return leftMeta
        case rightMeta: return rightAlt
        case rightAlt: return rightMeta
        default: return code
        }
    }
}

/// Pointer and scroll arithmetic, kept apart from event posting so it can be tested.
enum InputMath {
    /// Gentle acceleration: slow strokes stay precise, fast strokes cross both screens.
    /// `speed` is the user's pointer speed multiplier (1 = as designed).
    static func pointerGain(dx: Int, dy: Int, speed: Double) -> Double {
        min(1.0 + hypot(Double(dx), Double(dy)) * 0.06, 3.0) * speed
    }

    /// Scroll lines to post for a wheel delta, carrying the fraction that could not be posted
    /// yet in `remainder` so slow settings do not lose movement.
    static func scrollLines(_ delta: Int, speed: Double, reverse: Bool, remainder: inout Double) -> Int32 {
        let exact = Double(delta * 2) * speed * (reverse ? -1 : 1) + remainder
        let whole = exact.rounded(.towardZero)
        remainder = exact - whole
        return Int32(whole)
    }
}
