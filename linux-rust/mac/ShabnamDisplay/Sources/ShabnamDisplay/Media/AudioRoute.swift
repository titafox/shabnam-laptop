import CryptoKit
import Foundation
import ShabnamAudio

/// Where the Mac's sound goes while the laptop is connected.
enum AudioMode: String, CaseIterable {
    /// Heard on both: the Mac keeps playing and ScreenCaptureKit listens in.
    case both
    /// Heard on the laptop only: a system audio tap takes the sound and the Mac stays silent.
    case laptopOnly
    /// Not sent.
    case off

    /// The laptop-only mode needs Core Audio process taps (macOS 14.2).
    static var laptopOnlySupported: Bool {
        if #available(macOS 14.2, *) { return true }
        return false
    }
}

/// Carries out the chosen `AudioMode` for one session: keeps the sound connection, switches
/// ScreenCaptureKit's sound capture, and runs the muting system tap.
///
/// The Mac is only ever muted while sound is really reaching the laptop: the tap starts after
/// the sound connection is up and stops the moment a send fails or the session ends.
final class AudioRoute {
    private let sender: AudioSender
    private let capture: Capture
    private let lock = NSLock()
    private var tap: AnyObject?          // SystemAudioTap; guarded by `lock`
    private var tapFailed = false        // this session could not tap; fall back to "both"
    private var finished = false

    init(sender: AudioSender, capture: Capture) {
        self.sender = sender
        self.capture = capture
    }

    /// Brings everything in line with `mode`. Called about once a second, and at once when the
    /// setting changes, from the session's sound thread.
    func apply(_ mode: AudioMode, host: String, port: UInt16, key: SymmetricKey) {
        sender.maintain(enabled: mode != .off, host: host, port: port, key: key)

        lock.lock()
        defer { lock.unlock() }
        guard !finished else { return }
        let wantsTap = mode == .laptopOnly && AudioMode.laptopOnlySupported && !tapFailed && sender.isUp
        if wantsTap {
            if tap == nil { startTap() }
        } else {
            stopTapLocked()
        }
        // While the tap runs it is the only source; otherwise ScreenCaptureKit supplies the
        // sound for "both" (and stands in if the tap could not be created).
        let fallback = mode == .laptopOnly && (tapFailed || !AudioMode.laptopOnlySupported)
        capture.setCapturesAudio(tap == nil && (mode == .both || fallback))
    }

    /// Ends the session's sound: un-mutes the Mac first, then closes the connection.
    func finish() {
        lock.lock()
        finished = true
        stopTapLocked()
        lock.unlock()
        sender.close()
    }

    /// Un-mutes the Mac right now (app is quitting or being terminated).
    func stopTap() {
        lock.lock()
        stopTapLocked()
        lock.unlock()
    }

    private func startTap() {
        guard #available(macOS 14.2, *) else { return }
        let new = SystemAudioTap(muteMac: true, log: { log($0) }) { [weak self] pcm in
            guard let self else { return }
            if !self.sender.send(pcm: pcm) {
                // The laptop is not taking the sound any more: give it back to the Mac at once.
                DispatchQueue.global(qos: .userInitiated).async { self.stopTap() }
            }
        }
        do {
            try new.start()
            tap = new
            log("sound: laptop only (Mac muted), source \(new.sourceDescription)")
        } catch {
            tapFailed = true
            log("sound: \(error) — playing on both instead")
        }
    }

    private func stopTapLocked() {
        guard let running = tap else { return }
        tap = nil
        if #available(macOS 14.2, *) { (running as? SystemAudioTap)?.stop() }
        log("sound: Mac un-muted")
    }
}
