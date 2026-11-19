import XCTest
@testable import ShabnamDisplay

final class SettingsTests: XCTestCase {
    private var suite = ""
    private var defaults: UserDefaults!

    override func setUp() {
        suite = "org.shabnam.display.tests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suite)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suite)
    }

    func testDefaults() {
        let s = Settings(defaults: defaults)
        XCTAssertTrue(s.autoConnect)
        XCTAssertFalse(s.menuBarShowsDetails)
        XCTAssertFalse(s.notifications)
        XCTAssertEqual(s.displayMode, .hiDPI1080)
        XCTAssertEqual(s.frameRate, 60)
        XCTAssertEqual(s.quality, .sharpest)
        XCTAssertTrue(s.showCursor)
        XCTAssertEqual(s.audioMode, .both)
        XCTAssertTrue(s.acceptInput)
        XCTAssertEqual(s.inputOptions, InputOptions())
        XCTAssertEqual(s.host, "169.254.1.1")
        XCTAssertEqual(s.port, 7000)
    }

    func testValuesPersistAcrossInstances() {
        let a = Settings(defaults: defaults)
        a.displayMode = .hiDPI900
        a.frameRate = 30
        a.quality = .economy
        a.audioMode = .laptopOnly
        a.swapWinAlt = true
        a.scrollSpeed = 1.5
        a.host = "10.55.0.1"
        a.port = 7100

        let b = Settings(defaults: defaults)
        XCTAssertEqual(b.displayMode, .hiDPI900)
        XCTAssertEqual(b.frameRate, 30)
        XCTAssertEqual(b.quality, .economy)
        XCTAssertEqual(b.audioMode, .laptopOnly)
        XCTAssertTrue(b.inputOptions.swapWinAlt)
        XCTAssertEqual(b.inputOptions.scrollSpeed, 1.5)
        XCTAssertEqual(b.host, "10.55.0.1")
        XCTAssertEqual(b.port, 7100)
    }

    func testMigratesTheOldSwitches() {
        defaults.set(false, forKey: "hiDPI")
        defaults.set(false, forKey: "sendAudio")
        let s = Settings(defaults: defaults)
        XCTAssertEqual(s.displayMode, .plain1080)
        XCTAssertEqual(s.audioMode, .off)

        // A choice made with the new settings wins over the old switches.
        s.displayMode = .hiDPI720
        s.audioMode = .both
        XCTAssertEqual(Settings(defaults: defaults).displayMode, .hiDPI720)
        XCTAssertEqual(Settings(defaults: defaults).audioMode, .both)
    }

    func testOldSwitchesLeftOnMeanTheDefaults() {
        defaults.set(true, forKey: "hiDPI")
        defaults.set(true, forKey: "sendAudio")
        let s = Settings(defaults: defaults)
        XCTAssertEqual(s.displayMode, .hiDPI1080)
        XCTAssertEqual(s.audioMode, .both)
    }

    func testRejectsBadValues() {
        let s = Settings(defaults: defaults)
        s.host = "not an address"
        XCTAssertEqual(s.host, "169.254.1.1")
        defaults.set("999.1.1.1", forKey: "host")
        defaults.set(70000, forKey: "port")
        defaults.set(99.0, forKey: "pointerSpeed")
        defaults.set(-1.0, forKey: "scrollSpeed")
        defaults.set(45, forKey: "frameRate")
        defaults.set("ultra", forKey: "quality")
        XCTAssertEqual(s.host, "169.254.1.1")
        XCTAssertEqual(s.port, 7000)
        XCTAssertEqual(s.pointerSpeed, 3.0)
        XCTAssertEqual(s.scrollSpeed, 1.0)
        XCTAssertEqual(s.frameRate, 60)
        XCTAssertEqual(s.quality, .sharpest)
        XCTAssertFalse(Settings.isValidPort(65535)) // the sound port would not fit
    }

    func testChangesBumpTheRevisionAndNotify() {
        let s = Settings(defaults: defaults)
        let before = s.revision
        let told = expectation(forNotification: Settings.didChange, object: s) { note in
            note.userInfo?["key"] as? Settings.Key == .reverseScroll
        }
        s.reverseScroll = true
        wait(for: [told], timeout: 1)
        XCTAssertEqual(s.revision, before + 1)
    }

    func testRestoreConnectionDefaults() {
        let s = Settings(defaults: defaults)
        s.host = "10.55.0.1"
        s.port = 9000
        s.restoreConnectionDefaults()
        XCTAssertEqual(s.host, Settings.defaultHost)
        XCTAssertEqual(s.port, Settings.defaultPort)
    }

    // MARK: presets

    func testQualityPresetNumbers() {
        XCTAssertEqual(QualityPreset.sharpest.averageBitRate, 120_000_000)
        XCTAssertEqual(QualityPreset.sharpest.maxQP, 18)
        XCTAssertEqual(QualityPreset.balanced.averageBitRate, 60_000_000)
        XCTAssertEqual(QualityPreset.balanced.maxQP, 24)
        XCTAssertEqual(QualityPreset.economy.averageBitRate, 25_000_000)
        XCTAssertEqual(QualityPreset.economy.maxQP, 30)
        // Each step down spends fewer bits and allows coarser frames.
        let all = QualityPreset.allCases
        for (a, b) in zip(all, all.dropFirst()) {
            XCTAssertGreaterThan(a.averageBitRate, b.averageBitRate)
            XCTAssertLessThan(a.maxQP, b.maxQP)
        }
    }

    func testDisplayModeGeometry() {
        for mode in DisplayMode.allCases {
            XCTAssertEqual(mode.pointWidth * 9, mode.pointHeight * 16, "\(mode) is 16:9")
        }
        XCTAssertFalse(DisplayMode.plain1080.isHiDPI)
        XCTAssertEqual(DisplayMode.hiDPI900.pointWidth, 1600)
        XCTAssertEqual(DisplayMode.hiDPI720.pointHeight, 720)
    }
}
