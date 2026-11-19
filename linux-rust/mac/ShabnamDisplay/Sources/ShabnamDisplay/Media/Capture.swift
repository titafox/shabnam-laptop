import CoreGraphics
import CoreMedia
import CoreVideo
import Foundation
import ScreenCaptureKit

/// Captures one display (and optionally the system's sound output) with ScreenCaptureKit.
final class Capture: NSObject, SCStreamOutput, SCStreamDelegate {
    private let displayID: CGDirectDisplayID
    private let width: Int
    private let height: Int
    private var capturesAudio: Bool
    private let onFrame: (CVPixelBuffer, CMTime) -> Void
    private let onAudio: (CMSampleBuffer) -> Void
    private let onFailure: (String) -> Void

    private let queue = DispatchQueue(label: "capture")
    private let audioQueue = DispatchQueue(label: "sound")
    private var stream: SCStream?
    private var stopped = false
    private var last: CVPixelBuffer?
    private var lastAt = Date.distantPast
    private var refines = 0
    private var frames = 0
    private var refineTimer: DispatchSourceTimer?

    /// - onFrame: a picture to encode (capture queue).
    /// - onAudio: a buffer of system sound (sound queue).
    /// - onFailure: capture could not start or stopped by itself; called at most once.
    init(displayID: CGDirectDisplayID, width: Int, height: Int, capturesAudio: Bool,
         onFrame: @escaping (CVPixelBuffer, CMTime) -> Void,
         onAudio: @escaping (CMSampleBuffer) -> Void,
         onFailure: @escaping (String) -> Void) {
        self.displayID = displayID
        self.width = width
        self.height = height
        self.capturesAudio = capturesAudio
        self.onFrame = onFrame
        self.onAudio = onAudio
        self.onFailure = onFailure
    }

    /// A newly created display takes a moment to show up in the capture system; look a few times.
    func start() { findDisplay(triesLeft: 20) }

    private func findDisplay(triesLeft: Int) {
        SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: false) { [weak self] content, error in
            guard let self else { return }
            self.queue.async {
                guard !self.stopped else { return }
                if let display = content?.displays.first(where: { $0.displayID == self.displayID }) {
                    self.begin(display)
                } else if triesLeft == 0 {
                    self.fail("virtual display not visible to screen capture: \(error?.localizedDescription ?? "not listed")")
                } else {
                    self.queue.asyncAfter(deadline: .now() + 0.5) { self.findDisplay(triesLeft: triesLeft - 1) }
                }
            }
        }
    }

    private func configuration() -> SCStreamConfiguration {
        let c = SCStreamConfiguration()
        c.width = width
        c.height = height
        c.minimumFrameInterval = CMTime(value: 1, timescale: 60)
        c.pixelFormat = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        c.colorMatrix = CGDisplayStream.yCbCrMatrix_ITU_R_709_2
        c.showsCursor = true
        c.queueDepth = 3
        // Also take the Mac's sound output, to play on the laptop's speakers.
        c.capturesAudio = capturesAudio
        c.sampleRate = AudioSender.sampleRate
        c.channelCount = AudioSender.channels
        c.excludesCurrentProcessAudio = true
        return c
    }

    private func begin(_ display: SCDisplay) {
        let s = SCStream(filter: SCContentFilter(display: display, excludingWindows: []), configuration: configuration(), delegate: self)
        do {
            try s.addStreamOutput(self, type: .screen, sampleHandlerQueue: queue)
            try s.addStreamOutput(self, type: .audio, sampleHandlerQueue: audioQueue)
        } catch {
            fail("add output: \(error.localizedDescription)")
            return
        }
        stream = s
        s.startCapture { [weak self] error in
            guard let self else { return }
            if let error {
                self.queue.async { self.fail("capture failed: \(error.localizedDescription)") }
            } else {
                log("capturing")
            }
        }
        // macOS only delivers a frame when the picture changes. After motion stops, re-send the
        // still picture a few times so the encoder can sharpen it to full quality.
        let t = DispatchSource.makeTimerSource(queue: queue)
        t.schedule(deadline: .now() + 1, repeating: .milliseconds(33))
        t.setEventHandler { [weak self] in
            guard let self, !self.stopped, let pb = self.last, self.refines < 10, Date().timeIntervalSince(self.lastAt) > 0.04 else { return }
            self.refines += 1
            self.onFrame(pb, CMClockGetTime(CMClockGetHostTimeClock()))
        }
        t.resume()
        refineTimer = t
    }

    /// Switches sound capture on or off without restarting the picture.
    func setCapturesAudio(_ enabled: Bool) {
        queue.async {
            guard !self.stopped, self.capturesAudio != enabled else { return }
            self.capturesAudio = enabled
            self.stream?.updateConfiguration(self.configuration()) { error in
                if let error { log("could not change sound capture: \(error.localizedDescription)") }
            }
        }
    }

    /// Frames captured since the last call.
    func takeFrameCount() -> Int {
        queue.sync {
            let n = frames
            frames = 0
            return n
        }
    }

    /// Stops capturing. No callbacks are made after this returns.
    func stop() {
        var s: SCStream?
        queue.sync {
            stopped = true
            refineTimer?.cancel()
            refineTimer = nil
            last = nil
            s = stream
            stream = nil
        }
        audioQueue.sync {}
        guard let s else { return }
        let done = DispatchSemaphore(value: 0)
        s.stopCapture { _ in done.signal() }
        _ = done.wait(timeout: .now() + 2)
    }

    /// Capture queue only.
    private func fail(_ message: String) {
        guard !stopped else { return }
        stopped = true
        refineTimer?.cancel()
        refineTimer = nil
        last = nil
        log(message)
        onFailure(message)
    }

    // MARK: SCStreamOutput, SCStreamDelegate

    func stream(_ stream: SCStream, didOutputSampleBuffer sb: CMSampleBuffer, of type: SCStreamOutputType) {
        if type == .audio {
            onAudio(sb)
            return
        }
        guard type == .screen, !stopped, sb.isValid,
              let info = CMSampleBufferGetSampleAttachmentsArray(sb, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let raw = info.first?[.status] as? Int, SCFrameStatus(rawValue: raw) == .complete,
              let pb = sb.imageBuffer else { return }
        last = pb
        lastAt = Date()
        refines = 0
        frames += 1
        onFrame(pb, sb.presentationTimeStamp)
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        queue.async { self.fail("capture stopped: \(error.localizedDescription)") }
    }
}
