import XCTest
@testable import ShabnamDisplay

final class DiagnosticsTests: XCTestCase {
    func testRedactRemovesUserHomeAndHostNames() {
        let text = "could not open /Users/sample/Library/x for sample on Samples-MacBook.local"
        let out = Diagnostics.redact(text, home: "/Users/sample", user: "sample", hostNames: ["Samples-MacBook.local"])
        XCTAssertEqual(out, "could not open ~/Library/x for <user> on <host>")
    }

    func testReportHasNoPersonalDetails() {
        let suite = "org.shabnam.display.tests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        var stats = SessionStats.Snapshot()
        stats.lastError = "capture failed: \(NSHomeDirectory())/somewhere"
        let report = Diagnostics.report(state: .waiting, stats: stats, settings: Settings(defaults: defaults), cable: nil)
        XCTAssertFalse(report.contains(NSHomeDirectory()))
        if NSUserName().count >= 3 { XCTAssertFalse(report.contains(NSUserName())) }
        XCTAssertTrue(report.contains("state: waiting"))
        XCTAssertTrue(report.contains("120 Mbit/s, max QP 18"))
    }

    func testLogFileRotates() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("shabnam-log-test-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: dir) }
        let file = LogFile(directory: dir, maxBytes: 400, keep: 2)
        for i in 0..<40 { file.append("line \(i) " + String(repeating: "x", count: 30)) }
        file.flush()
        let names = try FileManager.default.contentsOfDirectory(atPath: dir.path).sorted()
        XCTAssertEqual(names, ["shabnam-display.1.log", "shabnam-display.2.log", "shabnam-display.log"])
        for name in names {
            let size = try FileManager.default.attributesOfItem(atPath: dir.appendingPathComponent(name).path)[.size] as? Int ?? 0
            XCTAssertLessThanOrEqual(size, 400)
        }
    }

    func testStatsRates() {
        let stats = SessionStats()
        stats.sessionStarted()
        for _ in 0..<60 { stats.frameSent() }
        stats.frameDropped()
        stats.audioSent(1920)
        let rate = stats.measured(videoBytes: 1_250_000, over: 1)
        XCTAssertEqual(rate.fps, 60)
        XCTAssertEqual(rate.megabitsPerSecond, 10)
        let snapshot = stats.snapshot
        XCTAssertEqual(snapshot.framesDropped, 1)
        XCTAssertEqual(snapshot.audioBytes, 1920)
        XCTAssertNotNil(snapshot.connectedSince)
        stats.sessionEnded()
        XCTAssertNil(stats.snapshot.connectedSince)
    }
}
