import Foundation

/// Live counters for the Settings window and the diagnostics text. Safe from any thread.
final class SessionStats {
    struct Snapshot: Equatable {
        var connectedSince: Date?
        var fps = 0.0
        var megabitsPerSecond = 0.0
        var framesSent = 0
        var framesDropped = 0
        var audioBytes = 0
        var lastError: String?
    }

    private let lock = NSLock()
    private var current = Snapshot()
    private var framesAtLastRate = 0

    var snapshot: Snapshot {
        lock.lock(); defer { lock.unlock() }
        return current
    }

    func sessionStarted() {
        lock.lock(); defer { lock.unlock() }
        current = Snapshot(connectedSince: Date(), lastError: current.lastError)
        framesAtLastRate = 0
    }

    func sessionEnded() {
        lock.lock(); defer { lock.unlock() }
        current.connectedSince = nil
        current.fps = 0
        current.megabitsPerSecond = 0
    }

    func frameSent() { lock.lock(); current.framesSent += 1; lock.unlock() }
    func frameDropped() { lock.lock(); current.framesDropped += 1; lock.unlock() }
    func audioSent(_ bytes: Int) { lock.lock(); current.audioBytes += bytes; lock.unlock() }

    func note(error: String) {
        lock.lock(); current.lastError = error; lock.unlock()
    }

    /// Called once per measuring interval with the bytes that went out during it.
    func measured(videoBytes: Int, over seconds: Double) -> (fps: Double, megabitsPerSecond: Double) {
        lock.lock(); defer { lock.unlock() }
        current.fps = Double(current.framesSent - framesAtLastRate) / seconds
        current.megabitsPerSecond = Double(videoBytes) * 8 / (seconds * 1e6)
        framesAtLastRate = current.framesSent
        return (current.fps, current.megabitsPerSecond)
    }
}
