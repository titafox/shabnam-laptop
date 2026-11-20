import CoreGraphics
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
    /// Streaming. The rate is nil until the first measurement.
    case connected(Rate?)
    /// Something needs attention; the app keeps retrying in the background.
    case problem(Problem)

    struct Rate: Equatable {
        var framesPerSecond: Int
        var megabitsPerSecond: Double
    }

    enum Problem: Equatable {
        case screenRecordingMissing
        case noPairingKey
        case authenticationFailed
        case displayFailed
        case encoderFailed
        case captureFailed
    }

    var isConnected: Bool {
        if case .connected = self { return true }
        return false
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
    let stats = SessionStats()
    /// Brightness, volume and battery of the laptop, where its receiver offers them.
    let laptop = LaptopControl()

    private let settings: Settings
    private let condition = NSCondition()
    private var generation = 0            // guarded by `condition`; bumped by every start/stop
    private var running = false           // guarded by `condition`
    private var suspended = false         // guarded by `condition`; true while the Mac sleeps
    private var activeLink: Link?         // guarded by `condition`
    private var activeAudio: AudioRoute?  // guarded by `condition`
    private var activeCapture: Capture?   // guarded by `condition`
    private var activeEncoder: Encoder?   // guarded by `condition`
    private var display: VirtualDisplay?  // main thread only
    private var askedForAccessibility = false

    init(settings: Settings = .shared) {
        self.settings = settings
    }

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

    /// Applies one changed setting to the running session, if any, without dropping the link.
    func settingChanged(_ key: Settings.Key) {
        condition.lock()
        let capture = activeCapture, encoder = activeEncoder
        condition.broadcast() // the session's own threads re-read sound and input settings
        condition.unlock()
        switch key {
        case .displayMode:
            display?.select(settings.displayMode)
        case .frameRate, .showCursor, .quality:
            capture?.update(frameRate: settings.frameRate, showsCursor: settings.showCursor)
            encoder?.apply(quality: settings.quality, frameRate: settings.frameRate)
        default:
            break
        }
    }

    /// Gives the Mac its own sound back right now. For when the app is about to end: nothing
    /// may be left muted, whatever happens to the rest of the teardown.
    func releaseAudioNow() {
        condition.lock()
        let audio = activeAudio
        condition.unlock()
        audio?.stopTap()
    }

    /// The Mac is going to sleep: end the session cleanly now (display and audio tap removed)
    /// instead of letting it die half-way. Nothing is attempted until `resume()`.
    func suspend() {
        condition.lock()
        suspended = true
        let link = activeLink
        condition.unlock()
        link?.shutdown()
    }

    /// The Mac woke up: look for the laptop again.
    func resume() {
        condition.lock()
        suspended = false
        condition.broadcast()
        condition.unlock()
    }

    /// The set of displays changed. If macOS has dropped our virtual display (it can after a
    /// display reconfiguration), start the session over rather than stream a screen that is gone.
    func displaysChanged() {
        guard let display, CGDisplayIsOnline(display.displayID) == 0 else { return }
        log("the virtual display is gone after a display change — reconnecting")
        condition.lock()
        let link = activeLink
        condition.unlock()
        link?.shutdown()
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

    private func waitWhileSuspended(_ gen: Int) {
        condition.lock()
        while generation == gen, suspended { condition.wait() }
        condition.unlock()
    }

    private func publish(_ new: LinkState, ifCurrent gen: Int? = nil) {
        if case .problem(let problem) = new { stats.note(error: "\(problem)") }
        DispatchQueue.main.async {
            if let gen, !self.isCurrent(gen) { return }
            guard self.state != new else { return }
            if !(new.isConnected && self.state.isConnected) { log("state: \(new.diagnosticName)") }
            self.state = new
            self.onStateChange?(new)
        }
    }

    private func run(_ gen: Int) {
        while isCurrent(gen) {
            waitWhileSuspended(gen)
            guard isCurrent(gen) else { break }
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
                stats.sessionStarted()
                let problem = session(link, key: key, gen: gen)
                stats.sessionEnded()
                if let problem {
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
        guard generation == gen, !suspended else { condition.unlock(); return nil }
        activeLink = link
        condition.unlock()
        defer {
            condition.lock()
            activeLink = nil
            activeAudio = nil
            activeCapture = nil
            activeEncoder = nil
            condition.unlock()
        }

        // The extra screen appears now, and is guaranteed to go when this function returns.
        let mode = settings.displayMode
        var display: VirtualDisplay? = DispatchQueue.main.sync {
            let created = VirtualDisplay(mode: mode)
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

        let stats = self.stats
        guard let encoder = Encoder(width: VirtualDisplay.width, height: VirtualDisplay.height,
                                    quality: settings.quality, frameRate: settings.frameRate,
                                    output: { if link.send($0) { stats.frameSent() } },
                                    onDrop: { stats.frameDropped() }) else {
            return .encoderFailed
        }
        defer { encoder.invalidate() }

        let audio = AudioSender()
        audio.onSent = { stats.audioSent($0) }

        var captureFailed = false // written on the capture queue before link.shutdown(), read after the read loop ends
        let capture = Capture(
            displayID: displayID, width: VirtualDisplay.width, height: VirtualDisplay.height,
            frameRate: settings.frameRate, showsCursor: settings.showCursor, capturesAudio: settings.audioMode == .both,
            onFrame: { pixelBuffer, pts in
                guard link.isUp else { return }
                encoder.encode(pixelBuffer, at: pts, forceKeyframe: link.takeKeyframeRequest())
            },
            onAudio: { audio.send($0) },
            onFailure: { message in
                stats.note(error: message)
                captureFailed = true
                link.shutdown()
            })
        defer { capture.stop() } // runs before the encoder and the display go away
        let route = AudioRoute(sender: audio, capture: capture)
        condition.lock()
        activeAudio = route
        activeCapture = capture
        activeEncoder = encoder
        condition.unlock()
        defer { route.finish() } // normally already done below; this covers every other way out
        capture.start()

        publish(.connected(nil), ifCurrent: gen)

        if !Permissions.accessibilityGranted {
            log("accessibility permission (needed for the laptop's keyboard and touchpad): NOT granted yet")
            if !askedForAccessibility, settings.acceptInput {
                askedForAccessibility = true
                DispatchQueue.main.async { Permissions.requestAccessibility() }
            }
        }

        // The sound connection comes and goes independently of the picture.
        let soundPort = settings.port &+ 1
        let host = settings.host
        let soundThread = Thread { [settings, condition] in
            while link.isUp {
                route.apply(settings.audioMode, host: host, port: soundPort, key: key)
                condition.lock()
                if link.isUp { condition.wait(until: Date().addingTimeInterval(1)) }
                condition.unlock()
            }
        }
        soundThread.name = "sound-link"
        soundThread.start()

        // So does the control connection (brightness, volume, battery). A receiver without it
        // refuses the connection; that is not an error, and it is tried again now and then.
        let controlPort = settings.port &+ ControlProtocol.portOffset
        let controlThread = Thread { [laptop, condition] in
            var explained = false
            while link.isUp {
                switch Link.open(host: host, port: controlPort, key: key) {
                case .success(let control):
                    log("laptop control connected")
                    explained = false
                    laptop.serve(control, while: { link.isUp })
                    control.close()
                case .failure:
                    if !explained { log("laptop control not available (the receiver may be an older version)") }
                    explained = true
                }
                condition.lock()
                if link.isUp { condition.wait(until: Date().addingTimeInterval(5)) }
                condition.unlock()
            }
        }
        controlThread.name = "control-link"
        controlThread.start()

        let interval = 1.0
        let timer = DispatchSource.makeTimerSource(queue: .global(qos: .utility))
        timer.schedule(deadline: .now() + interval, repeating: interval)
        timer.setEventHandler { [weak self] in
            guard link.isUp else { return }
            let rate = stats.measured(videoBytes: link.takeSentBytes(), over: interval)
            self?.publish(.connected(.init(framesPerSecond: Int(rate.fps.rounded()), megabitsPerSecond: rate.megabitsPerSecond)), ifCurrent: gen)
        }
        timer.resume()
        defer { timer.cancel() }

        // The laptop sends its key and pointer events back on the same connection, 8 bytes each.
        // This loop is also how we notice the link has gone.
        let injector = InputInjector()
        var seenRevision = -1
        var accept = true
        var record = [UInt8](repeating: 0, count: InputRecord.size)
        while record.withUnsafeMutableBytes({ link.read(into: $0) }) {
            let revision = settings.revision
            if revision != seenRevision {
                seenRevision = revision
                injector.options = settings.inputOptions
                let now = settings.acceptInput
                if accept, !now { injector.releaseAll() } // switched off mid-press: leave nothing held down
                accept = now
            }
            guard accept else { continue } // read and thrown away
            let r = InputRecord(record)
            injector.handle(type: r.type, code: r.code, value: r.value)
        }
        route.finish() // first of all: if the Mac was muted for the laptop, its own sound comes back now
        injector.releaseAll() // let go of anything still held so nothing stays stuck down
        laptop.disconnect() // the video link is down, so the control thread will not open another

        condition.lock()
        condition.broadcast() // wake the sound thread so it sees the link is down
        condition.unlock()
        return captureFailed ? .captureFailed : nil
    }
}
