import AppKit
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
    private var screenHeld: Date?

    var isAvailable: Bool { status != nil }
    /// The mode switch to offer: back to its own desktop while it shows the Mac, the other way
    /// round while it is on its own desktop. nil without the channel.
    var modeSwitch: LaptopModeSwitch? {
        guard let status else { return nil }
        return status.ownDesktop ? .becomeSecondScreen : .returnToDesktop
    }

    /// Screen off/on, power off and restart exist from revision 2 of the channel.
    var offersScreenAndPower: Bool { status?.hasModeAndPowerControls ?? false }

    /// What to send for the mode switch on offer; nil if `wanted` is not the one on offer (the
    /// laptop changed mode while the menu was open).
    func modeCommand(_ wanted: LaptopModeSwitch) -> ControlProtocol.Command? {
        guard modeSwitch == wanted else { return nil }
        return wanted == .becomeSecondScreen ? .becomeSecondScreen : .leave
    }

    /// Power off and restart are sent only after the user has said yes, and only to a receiver
    /// that knows them.
    func powerCommand(_ action: LaptopPowerAction, confirmed: Bool) -> ControlProtocol.Command? {
        guard confirmed, offersScreenAndPower else { return nil }
        return .endSession(action)
    }

    mutating func received(_ new: LaptopStatus, at now: Date = Date()) {
        var shown = new
        if let old = status {
            if Self.holds(&brightnessHeld, local: old.brightness, reported: new.brightness, now) { shown.brightness = old.brightness }
            if Self.holds(&volumeHeld, local: old.volume, reported: new.volume, now) { shown.volume = min(old.volume, new.volumeSteps) }
            if Self.holds(&muteHeld, local: old.muted, reported: new.muted, now) { shown.muted = old.muted }
            if Self.holds(&screenHeld, local: old.screenOff, reported: new.screenOff, now) { shown.screenOff = old.screenOff }
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

    mutating func setScreenOff(_ off: Bool, at now: Date = Date()) -> ControlProtocol.Command? {
        guard offersScreenAndPower, status?.screenOff != off else { return nil }
        status?.screenOff = off
        screenHeld = now.addingTimeInterval(Self.holdTime)
        return .setScreen(on: !off)
    }

    /// The channel is gone: the controls disappear.
    mutating func disconnected() {
        self = LaptopControlState()
    }
}

/// When the app looks for the laptop's control channel on its own, outside a second-screen
/// session — so that battery, brightness and volume are there while the laptop is on its own
/// desktop too.
enum StandaloneControl {
    /// Seconds between attempts. Calm: an attempt is one refused connection on the cable.
    static let retryInterval: TimeInterval = 4

    /// A second-screen session brings its own control connection, and there is only ever one.
    /// Nothing is tried while the Mac sleeps, without the cable, or without a pairing key.
    static func shouldConnect(sessionLinkUp: Bool, asleep: Bool, cablePresent: Bool, keyPresent: Bool) -> Bool {
        !sessionLinkUp && !asleep && cablePresent && keyPresent
    }
}

/// The control channel to the laptop: brightness, volume, mute, battery, "go back to your own
/// desktop". It is an extra — a laptop whose receiver does not offer it simply refuses the
/// connection, and the controls stay unavailable.
///
/// One connection at a time, and the newest wins, as on the laptop: a second-screen session's
/// connection replaces the one the app opened by itself while there was no session.
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

    func setScreenOff(_ off: Bool) { perform(state.setScreenOff(off)) }

    /// Called on the main thread when the laptop has been asked to become the second screen, so
    /// that the app starts looking for it if the link was switched off.
    var onBecomeSecondScreen: (() -> Void)?

    /// Switches the laptop between its own desktop and second-screen mode — whichever of the
    /// two `wanted` names, if that is still the one on offer. Either way the control connection
    /// then drops by itself: the laptop changes the program that serves it.
    func switchMode(_ wanted: LaptopModeSwitch) {
        guard let command = state.modeCommand(wanted) else { return }
        switch wanted {
        case .returnToDesktop:
            log("asking the laptop to return to its own desktop")
        case .becomeSecondScreen:
            log("asking the laptop to become the second screen")
            onBecomeSecondScreen?()
        }
        send(command)
    }

    /// Powers the laptop off or restarts it, if `confirm` (a dialog) says yes.
    func endSession(_ action: LaptopPowerAction, confirm: (LaptopPowerAction) -> Bool) {
        guard state.offersScreenAndPower else { return }
        guard let command = state.powerCommand(action, confirmed: confirm(action)) else { return }
        log("asking the laptop to \(action == .powerOff ? "power off" : "restart")")
        send(command)
    }

    /// The question asked before power off or restart.
    static func confirmWithAlert(_ action: LaptopPowerAction) -> Bool {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = action == .powerOff ? L("Power off Shabnam?") : L("Restart Shabnam?")
        alert.informativeText = L("Anything not saved on the laptop will be lost.")
        alert.addButton(withTitle: action == .powerOff ? L("Power Off") : L("Restart"))
        alert.addButton(withTitle: L("Cancel"))
        NSApp.activate(ignoringOtherApps: true)
        return alert.runModal() == .alertFirstButtonReturn
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
        guard wanted() else { lock.unlock(); return }
        let replaced = self.link
        self.link = link
        lock.unlock()
        replaced?.shutdown() // its reader ends; what it showed stays until this one reports
        defer {
            lock.lock()
            let wasCurrent = self.link === link
            if wasCurrent { self.link = nil }
            lock.unlock()
            // A connection that was replaced leaves the display to its successor.
            if wasCurrent {
                DispatchQueue.main.async {
                    guard !self.isConnected else { return } // a new one came in the meantime
                    self.state.disconnected()
                    self.onChange?(nil)
                }
            }
        }
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
                    guard self.isCurrent(link) else { return } // a late report from a replaced connection
                    let before = self.state.status
                    self.state.received(status)
                    if self.state.status != before { self.onChange?(self.state.status) }
                }
            }
        }
    }

    private func isCurrent(_ link: Link) -> Bool {
        lock.lock(); defer { lock.unlock() }
        return self.link === link
    }

    private var isConnected: Bool {
        lock.lock(); defer { lock.unlock() }
        return link != nil
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
