import CoreGraphics
import XCTest
@testable import ShabnamDisplay

final class InputTests: XCTestCase {
    private func flag(_ linuxCode: UInt16, swapped: Bool) -> CGEventFlags? {
        KeyMap.modifierFlags[KeyMap.remap(linuxCode, swapWinAlt: swapped)]
    }

    private func macKey(_ linuxCode: UInt16, swapped: Bool) -> CGKeyCode? {
        KeyMap.keys[KeyMap.remap(linuxCode, swapWinAlt: swapped)]
    }

    func testDefaultModifierLayout() {
        XCTAssertEqual(flag(KeyMap.leftMeta, swapped: false), .maskCommand)
        XCTAssertEqual(flag(KeyMap.rightMeta, swapped: false), .maskCommand)
        XCTAssertEqual(flag(KeyMap.leftAlt, swapped: false), .maskAlternate)
        XCTAssertEqual(flag(KeyMap.rightAlt, swapped: false), .maskAlternate)
        XCTAssertEqual(flag(29, swapped: false), .maskControl) // left Ctrl
        XCTAssertEqual(macKey(KeyMap.leftMeta, swapped: false), 55) // left Command
        XCTAssertEqual(macKey(KeyMap.leftAlt, swapped: false), 58) // left Option
    }

    func testSwappedModifierLayout() {
        XCTAssertEqual(flag(KeyMap.leftMeta, swapped: true), .maskAlternate)
        XCTAssertEqual(flag(KeyMap.rightMeta, swapped: true), .maskAlternate)
        XCTAssertEqual(flag(KeyMap.leftAlt, swapped: true), .maskCommand)
        XCTAssertEqual(flag(KeyMap.rightAlt, swapped: true), .maskCommand)
        // Left stays left, right stays right.
        XCTAssertEqual(macKey(KeyMap.leftMeta, swapped: true), 58)  // left Option
        XCTAssertEqual(macKey(KeyMap.rightMeta, swapped: true), 61) // right Option
        XCTAssertEqual(macKey(KeyMap.leftAlt, swapped: true), 55)   // left Command
        XCTAssertEqual(macKey(KeyMap.rightAlt, swapped: true), 54)  // right Command
    }

    func testSwapLeavesEverythingElseAlone() {
        for code in KeyMap.keys.keys where ![KeyMap.leftMeta, KeyMap.rightMeta, KeyMap.leftAlt, KeyMap.rightAlt].contains(code) {
            XCTAssertEqual(KeyMap.remap(code, swapWinAlt: true), code)
        }
        XCTAssertEqual(flag(29, swapped: true), .maskControl)
        XCTAssertEqual(flag(42, swapped: true), .maskShift)
    }

    func testSwapIsItsOwnInverse() {
        for code in [KeyMap.leftMeta, KeyMap.rightMeta, KeyMap.leftAlt, KeyMap.rightAlt] {
            XCTAssertEqual(KeyMap.remap(KeyMap.remap(code, swapWinAlt: true), swapWinAlt: true), code)
        }
    }

    func testScrollAtDefaultSpeedMatchesTheOriginalBehaviour() {
        var remainder = 0.0
        XCTAssertEqual(InputMath.scrollLines(1, speed: 1, reverse: false, remainder: &remainder), 2)
        XCTAssertEqual(InputMath.scrollLines(-3, speed: 1, reverse: false, remainder: &remainder), -6)
        XCTAssertEqual(remainder, 0)
    }

    func testScrollReverseAndSpeed() {
        var remainder = 0.0
        XCTAssertEqual(InputMath.scrollLines(1, speed: 1, reverse: true, remainder: &remainder), -2)
        XCTAssertEqual(InputMath.scrollLines(1, speed: 2, reverse: false, remainder: &remainder), 4)
    }

    func testSlowScrollCarriesFractionsInsteadOfLosingThem() {
        var remainder = 0.0
        var total: Int32 = 0
        for _ in 0..<8 { total += InputMath.scrollLines(1, speed: 0.25, reverse: false, remainder: &remainder) }
        XCTAssertEqual(total, 4) // 8 notches × 2 lines × 0.25
    }

    func testPointerGain() {
        XCTAssertEqual(InputMath.pointerGain(dx: 0, dy: 0, speed: 1), 1)
        XCTAssertEqual(InputMath.pointerGain(dx: 3, dy: 4, speed: 1), 1.3, accuracy: 1e-9)
        XCTAssertEqual(InputMath.pointerGain(dx: 1000, dy: 0, speed: 1), 3) // capped
        XCTAssertEqual(InputMath.pointerGain(dx: 3, dy: 4, speed: 2), 2.6, accuracy: 1e-9)
    }
}
