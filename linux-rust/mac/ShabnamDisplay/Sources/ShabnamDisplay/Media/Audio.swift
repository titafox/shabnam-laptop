import CoreMedia
import CryptoKit
import Foundation

/// Sends the Mac's sound output to the laptop's speakers over its own authenticated connection
/// (video port + 1) as plain 48 kHz, 16-bit, stereo samples.
final class AudioSender {
    static let sampleRate = 48000
    static let channels = 2

    private let lock = NSLock()
    private var link: Link?

    private var current: Link? {
        lock.lock(); defer { lock.unlock() }
        return link
    }

    /// Called about once a second while the video link is up: opens the sound connection when
    /// sound is wanted, re-opens it after a drop, and closes it when sound is switched off.
    func maintain(enabled: Bool, host: String, port: UInt16, key: SymmetricKey) {
        if let l = current {
            if enabled && l.isUp { return }
            close()
            if enabled { log("sound link lost, reconnecting") }
        }
        guard enabled, case .success(let fresh) = Link.open(host: host, port: port, key: key) else { return }
        lock.lock(); link = fresh; lock.unlock()
        log("sound connected")
    }

    func close() {
        lock.lock()
        let old = link
        link = nil
        lock.unlock()
        old?.close()
    }

    /// macOS hands sound over as 32-bit floats, usually one buffer per channel; the laptop wants
    /// plain 16-bit samples with left and right alternating.
    func send(_ sb: CMSampleBuffer) {
        guard let link = current, link.isUp, sb.isValid else { return }
        try? sb.withAudioBufferList { list, _ in
            let frames = sb.numSamples
            guard frames > 0, list.count > 0 else { return }
            var out = [Int16](repeating: 0, count: frames * 2)
            let put = { (i: Int, v: Float) in out[i] = Int16(max(-1, min(1, v)) * 32767) }
            if list.count >= 2, let l = list[0].mData?.assumingMemoryBound(to: Float.self), let r = list[1].mData?.assumingMemoryBound(to: Float.self) {
                for i in 0..<frames { put(i * 2, l[i]); put(i * 2 + 1, r[i]) }
            } else if let p = list[0].mData?.assumingMemoryBound(to: Float.self) {
                let ch = Int(list[0].mNumberChannels)
                for i in 0..<frames { put(i * 2, p[i * ch]); put(i * 2 + 1, p[i * ch + (ch > 1 ? 1 : 0)]) }
            }
            link.send(out.withUnsafeBufferPointer { Data(buffer: $0) })
        }
    }
}
