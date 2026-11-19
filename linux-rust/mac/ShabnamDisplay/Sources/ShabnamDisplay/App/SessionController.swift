import CoreMedia
import CoreVideo
import CryptoKit
import Foundation

/// What the menu bar shows.
enum LinkState: Equatable {
    /// The user switched the link off.
    case stopped
    /// Looking for the laptop on the cable.
    case waiting
    /// Streaming. The bitrate is nil until the first measurement.
    case connected(megabitsPerSecond: Double?)
    /// Something needs attention; the app keeps retrying in the background.
    case problem(Problem)

    enum Problem: Equatable {
        case screenRecordingMissing
        case noPairingKey
        case authenticationFailed
        case displayFailed
        case encoderFailed
        case captureFailed
    }
}

/// Keeps the second screen available: waits for the laptop on the cable, runs a session until the
/// link drops, removes the extra screen, and waits again.
///
/// The virtual display exists only while the laptop is really there — otherwise windows end up on
/// a screen nobody can see. So the order is always: connect and authenticate first, create the
/// display after, and tear the display down as soon as the video link is gone.
final class SessionController {
    /// Called on the main thread whenever the state changes.
    var onStateChange: ((LinkState) -> Void)?
    private(set) var state: LinkState = .stopped // main thread only

    private let settings = Settings.shared
    private let condition = NSCondition()
    private var generation = 0          // guarded by `condition`; bumped by every start/stop
    private var running = false         // guarded by `condition`
    private var activeLink: Link?       // guarded by `condition`
    private var activeCapture: Capture? // guarded by `condition`
    private var display: VirtualDisplay? // main thread only
    private var askedForAccessibility = false

    var isRunning: Bool {
        condition.lock(); defer { condition.unlock() }
        return running
    }

    // MARK: control (main thread)

    func start() {
        condition.lock()
        guard !running else { condition.unlock(); return }
        running = true
        generation += 1
        let gen = generation
        condition.unlock()
        let thread = Thread { [weak self] in self?.run(gen) }
        thread.name = "session"
        thread.start()
    }

    func stop() {
        condition.lock()
        running = false
        generation += 1
        let link = activeLink
        condition.broadcast()
        condition.unlock()
        link?.shutdown() // wakes the session thread, which removes the display
        publish(.stopped)
    }

    /// Applies a changed resolution setting to the display, if there is one right now.
    func resolutionSettingChanged() {
        display?.selectMode(hiDPI: settings.hiDPI)
    }

    /// Applies a changed "Send audio" setting to the running session, if any.
    func audioSettingChanged() {
        condition.lock()
        let capture = activeCapture
        condition.broadcast()
        condition.unlock()
        capture?.setCapturesAudio(settings.sendAudio)
    }

    // MARK: session thread

    private func isCurrent(_ gen: Int) -> Bool {
        condition.lock(); defer { condition.unlock() }
        return generation == gen
    }

    /// Sleeps, but returns early when the user stops or restarts the link.
    private func pause(_ seconds: TimeInterval, _ gen: Int) {
        let deadline = Date().addingTimeInterval(seconds)
        condition.lock()
        while generation == gen, condition.wait(until: deadline) {}
        condition.unlock()
    }

    private func publish(_ new: LinkState, ifCurrent gen: Int? = nil) {
        DispatchQueue.main.async {
            if let gen, !self.isCurrent(gen) { return }
            guard self.state != new else { return }
            if case .connected = new, case .connected = self.state {} else { log("state: \(new)") }
            self.state = new
            self.onStateChange?(new)
        }
    }

    private func run(_ gen: Int) {
        while isCurrent(gen) {
            // Without this permission the capture cannot start; do not bring a display up just to fail.
            guard Permissions.screenRecordingGranted else {
                publish(.problem(.screenRecordingMissing), ifCurrent: gen)
                pause(2, gen)
                continue
            }
            // Read afresh for every attempt so a newly generated key is picked up without a restart.
            guard let key = PairingKey.load() else {
                publish(.problem(.noPairingKey), ifCurrent: gen)
                pause(2, gen)
                continue
            }
            switch Link.open(host: settings.host, port: settings.port, key: key) {
            case .failure(.noCable), .failure(.unreachable):
                publish(.waiting, ifCurrent: gen)
                pause(1, gen)
            case .failure(.authenticationFailed):
                log("peer does not hold the pairing key — refusing to send it anything")
                publish(.problem(.authenticationFailed), ifCurrent: gen)
                pause(1, gen)
            case .success(let link):
                log("video connected")
                if let problem = session(link, key: key, gen: gen) {
                    publish(.problem(problem), ifCurrent: gen)
                    pause(10, gen) // do not make the screen flicker in and out while something is broken
                } else {
                    log("laptop disconnected — extra screen removed")
                    publish(.waiting, ifCurrent: gen)
                    pause(1, gen)
                }
            }
        }
    }

    /// One connection, from authenticated link to teardown. Returns what went wrong, or nil if the
    /// link simply ended (cable pulled, laptop left second-screen mode, user chose Disconnect).
    private func session(_ link: Link, key: SymmetricKey, gen: Int) -> LinkState.Problem? {
        defer { link.close() }

        condition.lock()
        guard generation == gen else { condition.unlock(); return nil }
        activeLink = link
        condition.unlock()
        defer {
            condition.lock()
            activeLink = nil
            activeCapture = nil
            condition.unlock()
        }

        // The extra screen appears now, and is guaranteed to go when this function returns.
        let hiDPI = settings.hiDPI
        var display: VirtualDisplay? = DispatchQueue.main.sync {
            let created = VirtualDisplay(hiDPI: hiDPI)
            self.display = created
            return created
        }
        defer {
            DispatchQueue.main.sync {
                // A newer session may already have put its own display here; leave that one alone.
                if self.display === display { self.display = nil }
                display = nil
            }
        }
        let displayID = display?.displayID
        guard let displayID else {
            log("could not create the virtual display")
            return .displayFailed
        }

        guard let encoder = Encoder(width: VirtualDisplay.width, height: VirtualDisplay.height, output: { link.send($0) }) else {
            return .encoderFailed
        }
        defer { encoder.invalidate() }

        let audio = AudioSender()
        defer { audio.close() }

        var captureFailed = false // written on the capture queue before link.shutdown(), read after the read loop ends
        let capture = Capture(
            displayID: displayID, width: VirtualDisplay.width, height: VirtualDisplay.height, capturesAudio: settings.sendAudio,
            onFrame: { pixelBuffer, pts in
                guard link.isUp else { return }
                encoder.encode(pixelBuffer, at: pts, forceKeyframe: link.takeKeyframeRequest())
            },
            onAudio: { audio.send($0) },
            onFailure: { _ in
                captureFailed = true
                link.shutdown()
            })
        condition.lock()
        activeCapture = capture
        condition.unlock()
        defer { capture.stop() } // runs before the encoder and the display go away
        capture.start()

        publish(.connected(megabitsPerSecond: nil), ifCurrent: gen)

        if !Permissions.accessibilityGranted {
            log("accessibility permission (needed for the laptop's keyboard and touchpad): NOT granted yet")
            if !askedForAccessibility {
                askedForAccessibility = true
                DispatchQueue.main.async { Permissions.requestAccessibility() }
            }
        }

        // The sound connection comes and goes independently of the picture.
        let soundPort = settings.port &+ 1
        let host = settings.host
        let soundThread = Thread { [settings, condition] in
            while link.isUp {
                audio.maintain(enabled: settings.sendAudio, host: host, port: soundPort, key: key)
                condition.lock()
                if link.isUp { condition.wait(until: Date().addingTimeInterval(1)) }
                condition.unlock()
            }
        }
        soundThread.name = "sound-link"
        soundThread.start()

        let interval = 2.0
        let stats = DispatchSource.makeTimerSource(queue: .global(qos: .utility))
        stats.schedule(deadline: .now() + interval, repeating: interval)
        stats.setEventHandler { [weak self] in
            guard link.isUp else { return }
            let mbit = Double(link.takeSentBytes()) * 8 / (interval * 1e6)
            self?.publish(.connected(megabitsPerSecond: mbit), ifCurrent: gen)
        }
        stats.resume()
        defer { stats.cancel() }

        // The laptop sends its key and pointer events back on the same connection, 8 bytes each.
        // This loop is also how we notice the link has gone.
        let injector = InputInjector()
        var record = [UInt8](repeating: 0, count: InputRecord.size)
        while record.withUnsafeMutableBytes({ link.read(into: $0) }) {
            let r = InputRecord(record)
            injector.handle(type: r.type, code: r.code, value: r.value)
        }
        injector.releaseAll() // let go of anything still held so nothing stays stuck down

        condition.lock()
        condition.broadcast() // wake the sound thread so it sees the link is down
        condition.unlock()
        return captureFailed ? .captureFailed : nil
    }
}
