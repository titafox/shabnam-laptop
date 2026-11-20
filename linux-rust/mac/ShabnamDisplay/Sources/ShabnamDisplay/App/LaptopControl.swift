import Foundation

/// What the menu and the Settings window show of the laptop, kept steady while the user is
/// moving a control.
///
/// The laptop reports its state at most once a second, so a report that was already on its way
/// can arrive just after the user moved a slider. A value the user has just set is therefore held
/// for a moment: reports that disagree are not shown until the laptop confirms the value or the
/// hold runs out.
struct LaptopControlState: Equatable {
    static let holdTime: TimeInterval = 2

    /// nil while the control channel is not available (not connected, or an older receiver).
    private(set) var status: LaptopStatus?
    private var brightnessHeld: Date?
    private var volumeHeld: Date?
    private var muteHeld: Date?

    var isAvailable: Bool { status != nil }

    mutating func received(_ new: LaptopStatus, at now: Date = Date()) {
        var shown = new
        if let old = status {
            if Self.holds(&brightnessHeld, local: old.brightness, reported: new.brightness, now) { shown.brightness = old.brightness }
            if Self.holds(&volumeHeld, local: old.volume, reported: new.volume, now) { shown.volume = min(old.volume, new.volumeSteps) }
            if Self.holds(&muteHeld, local: old.muted, reported: new.muted, now) { shown.muted = old.muted }
        }
        status = shown
    }

    /// True if the local value should stay on show. Ends the hold once the laptop agrees or time is up.
    private static func holds<Value: Equatable>(_ until: inout Date?, local: Value, reported: Value, _ now: Date) -> Bool {
        guard let deadline = until else { return false }
        if reported == local || now >= deadline {
            until = nil
            return false
        }
        return true
    }

    /// The user moved the brightness control. Returns what to send, or nil if there is nothing
    /// to do (no channel, or the value is the one already shown).
    mutating func setBrightness(_ percent: Int, at now: Date = Date()) -> ControlProtocol.Command? {
        let value = max(0, min(100, percent))
        guard status != nil, status?.brightness != value else { return nil }
        status?.brightness = value
        brightnessHeld = now.addingTimeInterval(Self.holdTime)
        return .setBrightness(value)
    }

    mutating func setVolume(_ step: Int, at now: Date = Date()) -> ControlProtocol.Command? {
        guard let current = status else { return nil }
        let value = max(0, min(current.volumeSteps, step))
        guard current.volume != value else { return nil }
        status?.volume = value
        volumeHeld = now.addingTimeInterval(Self.holdTime)
        return .setVolume(value)
    }

    mutating func setMuted(_ muted: Bool, at now: Date = Date()) -> ControlProtocol.Command? {
        guard status != nil, status?.muted != muted else { return nil }
        status?.muted = muted
        muteHeld = now.addingTimeInterval(Self.holdTime)
        return .setMute(muted)
    }

    /// The channel is gone: the controls disappear.
    mutating func disconnected() {
        self = LaptopControlState()
    }
}

/// The control channel to the laptop: brightness, volume, mute, battery, "go back to your own
/// desktop". It is an extra — a laptop whose receiver does not offer it simply refuses the
/// connection, and the controls stay unavailable.
final class LaptopControl {
    /// Called on the main thread whenever what should be shown has changed.
    var onChange: ((LaptopStatus?) -> Void)?
    private(set) var state = LaptopControlState() // main thread only

    private let lock = NSLock()
    private var link: Link?     // guarded by `lock`
    private var refreshes = 0   // main thread only

    var status: LaptopStatus? { state.status }

    // MARK: user actions (main thread)

    func setBrightness(_ percent: Int) { perform(state.setBrightness(percent)) }
    func setVolume(_ step: Int) { perform(state.setVolume(step)) }
    func setMuted(_ muted: Bool) { perform(state.setMuted(muted)) }

    /// Asks the laptop to leave second-screen mode. The link then drops by itself.
    func returnToDesktop() {
        guard state.isAvailable else { return }
        log("asking the laptop to return to its own desktop")
        send(.leave)
    }

    private func perform(_ command: ControlProtocol.Command?) {
        guard let command else { return }
        send(command)
        onChange?(state.status)
        // The laptop may have settled on another value than the one asked for (it never goes
        // fully dark, for instance). Ask once the hold is over, after the last change.
        refreshes += 1
        let mine = refreshes
        DispatchQueue.main.asyncAfter(deadline: .now() + LaptopControlState.holdTime + 0.1) { [weak self] in
            guard let self, self.refreshes == mine else { return }
            self.send(.getStatus)
        }
    }

    private func send(_ command: ControlProtocol.Command) {
        lock.lock()
        let link = self.link
        lock.unlock()
        link?.send(command.encoded)
    }

    // MARK: connection (session threads)

    /// Reads the laptop's reports from an authenticated control connection until it ends.
    /// `wanted` is asked once the connection can be reached by `disconnect()`; false ends it at once.
    func serve(_ link: Link, while wanted: () -> Bool) {
        lock.lock()
        self.link = link
        lock.unlock()
        defer {
            lock.lock()
            if self.link === link { self.link = nil }
            lock.unlock()
            DispatchQueue.main.async {
                self.state.disconnected()
                self.onChange?(nil)
            }
        }
        guard wanted() else { return }
        var parser = ControlProtocol.Parser()
        while true {
            var chunk = [UInt8](repeating: 0, count: parser.wanted)
            guard chunk.withUnsafeMutableBytes({ link.read(into: $0) }) else { return }
            guard let reports = try? parser.feed(chunk) else {
                log("control: oversized message from the laptop — closing the control connection")
                return
            }
            for status in reports {
                DispatchQueue.main.async {
                    let before = self.state.status
                    self.state.received(status)
                    if self.state.status != before { self.onChange?(self.state.status) }
                }
            }
        }
    }

    /// Ends the control connection, if there is one. Safe from any thread.
    func disconnect() {
        lock.lock()
        let link = self.link
        lock.unlock()
        link?.shutdown()
    }

    /// For the UI preview: pretend a laptop reported this. Main thread.
    func showFake(_ status: LaptopStatus) {
        state.received(status)
        onChange?(state.status)
    }
}
