import XCTest
@testable import ShabnamDisplay

/// The control channel's wire format (docs/PROTOCOL.md §6) and what the UI shows of it.
/// The byte vectors are the same ones the receiver's tests use (subscreen-rx, src/proto.rs).
final class ControlTests: XCTestCase {
    private let sample = LaptopStatus(brightness: 63, volume: 9, volumeSteps: 16, muted: true, battery: 82, charging: true,
                                      externalPower: true, lidClosed: false, receiverVersion: "0.2.0", protocolVersion: 1)
    private let sampleBytes: [UInt8] = [0x81, 9, 1, 63, 9, 16, 0b01_1111, 82, 0, 2, 0]

    // MARK: codec

    func testCommandsEncode() {
        XCTAssertEqual([UInt8](ControlProtocol.Command.setBrightness(40).encoded), [0x01, 1, 40])
        XCTAssertEqual([UInt8](ControlProtocol.Command.setVolume(7).encoded), [0x02, 1, 7])
        XCTAssertEqual([UInt8](ControlProtocol.Command.setMute(true).encoded), [0x03, 1, 1])
        XCTAssertEqual([UInt8](ControlProtocol.Command.setMute(false).encoded), [0x03, 1, 0])
        XCTAssertEqual([UInt8](ControlProtocol.Command.leave.encoded), [0x04, 0])
        XCTAssertEqual([UInt8](ControlProtocol.Command.getStatus.encoded), [0x05, 0])
    }

    func testCommandsAreClampedBeforeSending() {
        XCTAssertEqual([UInt8](ControlProtocol.Command.setBrightness(250).encoded), [0x01, 1, 100])
        XCTAssertEqual([UInt8](ControlProtocol.Command.setBrightness(-3).encoded), [0x01, 1, 0])
        XCTAssertEqual([UInt8](ControlProtocol.Command.setVolume(-1).encoded), [0x02, 1, 0])
        XCTAssertEqual([UInt8](ControlProtocol.Command.setVolume(100_000).encoded), [0x02, 1, 255])
    }

    func testStatusDecodes() {
        XCTAssertEqual(ControlProtocol.decodeStatus(type: 0x81, payload: Data(sampleBytes.dropFirst(2))), sample)
    }

    func testStatusFlags() {
        var bytes = Array(sampleBytes.dropFirst(2))
        bytes[4] = 0 // no battery, not muted, lid unknown
        let bare = ControlProtocol.decodeStatus(type: 0x81, payload: Data(bytes))
        XCTAssertNil(bare?.battery)
        XCTAssertNil(bare?.lidClosed)
        XCTAssertEqual(bare?.muted, false)
        XCTAssertEqual(bare?.charging, false)
        bytes[4] = 0b11_1010 // battery on charger but full, lid closed
        let closed = ControlProtocol.decodeStatus(type: 0x81, payload: Data(bytes))
        XCTAssertEqual(closed?.battery, 82)
        XCTAssertEqual(closed?.charging, false)
        XCTAssertEqual(closed?.externalPower, true)
        XCTAssertEqual(closed?.lidClosed, true)
    }

    func testStatusNumbersAreClamped() {
        let wild = ControlProtocol.decodeStatus(type: 0x81, payload: Data([1, 255, 200, 16, 0x02, 250, 9, 9, 9]))
        XCTAssertEqual(wild?.brightness, 100)
        XCTAssertEqual(wild?.volume, 16)
        XCTAssertEqual(wild?.battery, 100)
    }

    func testMalformedStatusIsIgnored() {
        XCTAssertNil(ControlProtocol.decodeStatus(type: 0x81, payload: Data(sampleBytes.dropFirst(2).dropLast()))) // too short
        XCTAssertNil(ControlProtocol.decodeStatus(type: 0x81, payload: Data()))
        XCTAssertNil(ControlProtocol.decodeStatus(type: 0x80, payload: Data(sampleBytes.dropFirst(2)))) // unknown type
        XCTAssertNil(ControlProtocol.decodeStatus(type: 0x01, payload: Data(sampleBytes.dropFirst(2))))
        XCTAssertNil(ControlProtocol.decodeStatus(type: 0x81, payload: Data([1, 50, 0, 0, 0, 0, 0, 0, 0]))) // zero volume steps
    }

    func testLongerStatusFromANewerReceiverStillDecodes() {
        XCTAssertEqual(ControlProtocol.decodeStatus(type: 0x81, payload: Data(sampleBytes.dropFirst(2) + [7, 7, 7])), sample)
    }

    func testParserHandlesSplitAndJoinedMessages() throws {
        var parser = ControlProtocol.Parser()
        XCTAssertEqual(parser.wanted, 2)
        XCTAssertEqual(try parser.feed(sampleBytes.prefix(1)), [])
        XCTAssertEqual(parser.wanted, 1)
        XCTAssertEqual(try parser.feed(sampleBytes[1..<5]), [])
        XCTAssertEqual(parser.wanted, 6)
        XCTAssertEqual(try parser.feed(sampleBytes[5...] + sampleBytes + sampleBytes.prefix(3)), [sample, sample])
        XCTAssertEqual(try parser.feed(sampleBytes.dropFirst(3)), [sample])
        XCTAssertEqual(parser.wanted, 2)
    }

    func testParserSkipsUnknownTypesAndStaysInStep() throws {
        var parser = ControlProtocol.Parser()
        let stream: [UInt8] = [0x90, 3, 1, 2, 3] + sampleBytes + [0x91, 0] + [0x81, 2, 1, 1] + sampleBytes
        XCTAssertEqual(try parser.feed(stream), [sample, sample])
    }

    func testParserRefusesOversizedMessages() {
        var parser = ControlProtocol.Parser()
        XCTAssertNoThrow(try parser.feed([0x90, 32]))
        parser = ControlProtocol.Parser()
        XCTAssertThrowsError(try parser.feed([0x81, 33]))
        parser = ControlProtocol.Parser()
        XCTAssertThrowsError(try parser.feed([0xEE, 255, 1, 2, 3]))
    }

    // MARK: state

    private func connected() -> LaptopControlState {
        var state = LaptopControlState()
        state.received(sample, at: Date(timeIntervalSince1970: 0))
        return state
    }

    func testNothingIsSentWithoutTheChannel() {
        var state = LaptopControlState()
        XCTAssertFalse(state.isAvailable)
        XCTAssertNil(state.setBrightness(50))
        XCTAssertNil(state.setVolume(3))
        XCTAssertNil(state.setMuted(true))
        XCTAssertNil(state.status)
    }

    func testReportsAreShown() {
        var state = connected()
        XCTAssertEqual(state.status, sample)
        var next = sample
        next.brightness = 20 // the laptop's own brightness key
        next.volume = 10
        next.battery = 81
        state.received(next, at: Date(timeIntervalSince1970: 1))
        XCTAssertEqual(state.status, next)
    }

    func testSettingAValueShowsItAtOnceAndSendsIt() {
        var state = connected()
        XCTAssertEqual(state.setBrightness(40), .setBrightness(40))
        XCTAssertEqual(state.status?.brightness, 40)
        XCTAssertEqual(state.setVolume(12), .setVolume(12))
        XCTAssertEqual(state.status?.volume, 12)
        XCTAssertEqual(state.setMuted(false), .setMute(false))
        XCTAssertEqual(state.status?.muted, false)
    }

    func testUnchangedValuesAreNotSentAgain() {
        var state = connected()
        XCTAssertNil(state.setBrightness(63))
        XCTAssertNil(state.setVolume(9))
        XCTAssertNil(state.setMuted(true))
    }

    func testValuesAreClampedToWhatTheLaptopOffers() {
        var state = connected()
        XCTAssertEqual(state.setBrightness(400), .setBrightness(100))
        XCTAssertEqual(state.setBrightness(-5), .setBrightness(0))
        XCTAssertEqual(state.setVolume(99), .setVolume(16))
        XCTAssertEqual(state.setVolume(-1), .setVolume(0))
    }

    func testAStaleReportDoesNotMoveTheSliderBack() {
        var state = connected()
        let t = Date(timeIntervalSince1970: 100)
        _ = state.setBrightness(40, at: t)
        var stale = sample // still says 63, and carries fresh news about the battery
        stale.battery = 80
        state.received(stale, at: t.addingTimeInterval(0.2))
        XCTAssertEqual(state.status?.brightness, 40)
        XCTAssertEqual(state.status?.battery, 80)
        XCTAssertEqual(state.status?.volume, 9) // fields that were not touched follow the laptop
    }

    func testConfirmationEndsTheHold() {
        var state = connected()
        let t = Date(timeIntervalSince1970: 100)
        _ = state.setBrightness(40, at: t)
        var report = sample
        report.brightness = 40
        state.received(report, at: t.addingTimeInterval(0.5))
        report.brightness = 47 // then the laptop's own key is pressed
        state.received(report, at: t.addingTimeInterval(0.8))
        XCTAssertEqual(state.status?.brightness, 47)
    }

    func testAfterTheHoldTheLaptopsValueWins() {
        var state = connected()
        let t = Date(timeIntervalSince1970: 100)
        _ = state.setBrightness(0, at: t)
        _ = state.setMuted(false, at: t)
        var report = sample
        report.brightness = 4 // the laptop never goes fully dark
        state.received(report, at: t.addingTimeInterval(1))
        XCTAssertEqual(state.status?.brightness, 0)
        XCTAssertEqual(state.status?.muted, false)
        state.received(report, at: t.addingTimeInterval(LaptopControlState.holdTime + 0.1))
        XCTAssertEqual(state.status?.brightness, 4)
        XCTAssertEqual(state.status?.muted, true)
    }

    func testDisconnectingClearsEverything() {
        var state = connected()
        _ = state.setVolume(3)
        state.disconnected()
        XCTAssertNil(state.status)
        XCTAssertFalse(state.isAvailable)
        // A new connection starts from the laptop's report, not from an old hold.
        state.received(sample)
        XCTAssertEqual(state.status?.volume, 9)
    }

    func testControlObjectAppliesChangesLocallyAndReportsThem() {
        let control = LaptopControl()
        var seen: [LaptopStatus?] = []
        control.onChange = { seen.append($0) }
        control.setBrightness(10) // no channel: nothing happens
        XCTAssertTrue(seen.isEmpty)
        control.showFake(sample)
        control.setBrightness(10)
        control.setMuted(false)
        XCTAssertEqual(seen.count, 3)
        XCTAssertEqual(control.status?.brightness, 10)
        XCTAssertEqual(control.status?.muted, false)
    }

    // MARK: wording

    func testBatteryWording() {
        XCTAssertEqual(sample.batteryText, "82% · charging")
        XCTAssertEqual(sample.batteryDescription, "Shabnam 82% · charging")
        var s = sample
        s.charging = false
        XCTAssertEqual(s.batteryText, "82% · plugged in")
        s.externalPower = false
        XCTAssertEqual(s.batteryText, "82%")
        s.battery = nil
        XCTAssertNil(s.batteryText)
        XCTAssertNil(s.batteryDescription)
    }

    func testDiagnosticsMentionTheLaptop() {
        let suite = "org.shabnam.display.tests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let settings = Settings(defaults: defaults)
        let with = Diagnostics.report(state: .waiting, stats: .init(), settings: settings, cable: nil, laptop: sample)
        XCTAssertTrue(with.contains("laptop control: receiver 0.2.0, protocol 1, brightness 63%, volume 9/16 muted, battery 82% charging, lid open"))
        XCTAssertTrue(Diagnostics.report(state: .waiting, stats: .init(), settings: settings, cable: nil).contains("laptop control: not available"))
    }
}
