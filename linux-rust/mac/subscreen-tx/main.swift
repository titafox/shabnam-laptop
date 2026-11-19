// subscreen-tx — run on the Mac. Creates a 1920x1080 virtual display (a real extra screen as far
// as macOS is concerned), captures it, encodes with the hardware H.264 encoder and sends the
// stream to subscreen-rx on the laptop over the USB cable. Pixels map 1:1, nothing is scaled.
// The Mac's sound output is captured too and sent to the laptop's speakers.
// The laptop's keyboard and touchpad events come back on the same connection and are replayed here.
// Quit with Ctrl-C; the virtual display disappears with the process.
//
// usage: subscreen-tx [host=169.254.1.1] [port=7000] [looks-like-width=1920]
// looks-like-width: how wide the desktop appears, in points. Every size is drawn by macOS at
// double precision and then fitted to the panel's 1920x1080; 1920 (the default, chosen by the
// user as the sharpest-looking) is therefore 3840x2160 averaged 2:1. 1280 = 1.5x larger UI,
// 960 = 2x larger. Also switchable live in System Settings > Displays.
import CoreGraphics
import CoreMedia
import CoreVideo
import CryptoKit
import Foundation
import ScreenCaptureKit
import VideoToolbox

let host = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "169.254.1.1"
let port = CommandLine.arguments.count > 2 ? UInt16(CommandLine.arguments[2]) ?? 7000 : 7000
let W = 1920, H = 1080
let looksLike = CommandLine.arguments.count > 3 ? Int(CommandLine.arguments[3]) ?? 1920 : 1920
func log(_ s: String) { FileHandle.standardError.write((s + "\n").data(using: .utf8)!) }

// The pairing key shared with the laptop. Nothing is sent to, or accepted from, a peer that
// cannot prove it holds the same key (see auth.rs on the laptop side for the exchange).
let keyPath = NSString(string: "~/.config/subscreen/key").expandingTildeInPath
guard let keyData = FileManager.default.contents(atPath: keyPath), keyData.count >= 32 else {
    log("no pairing key at \(keyPath)"); exit(1)
}
let pairingKey = SymmetricKey(data: keyData)

func readExact(_ s: Int32, _ n: Int) -> Data? {
    var d = Data(count: n), got = 0
    while got < n {
        let r = d.withUnsafeMutableBytes { read(s, $0.baseAddress! + got, n - got) }
        if r <= 0 { return nil }
        got += r
    }
    return d
}
func writeAll(_ s: Int32, _ d: Data) -> Bool {
    d.withUnsafeBytes { p in
        var off = 0
        while off < p.count { let n = write(s, p.baseAddress! + off, p.count - off); if n <= 0 { return false }; off += n }
        return true
    }
}
/// Proves to the laptop that we hold the key, and checks that it does too.
func handshake(_ s: Int32) -> Bool {
    var tv = timeval(tv_sec: 5, tv_usec: 0)
    setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
    defer { tv.tv_sec = 0; setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size)) }
    var ns = Data(count: 16)
    guard ns.withUnsafeMutableBytes({ SecRandomCopyBytes(kSecRandomDefault, 16, $0.baseAddress!) }) == errSecSuccess,
          writeAll(s, ns), let reply = readExact(s, 48) else { return false }
    let nr = reply.prefix(16), theirs = reply.suffix(32)
    guard HMAC<SHA256>.isValidAuthenticationCode(theirs, authenticating: Data("rx".utf8) + ns + nr, using: pairingKey) else { return false }
    return writeAll(s, Data(HMAC<SHA256>.authenticationCode(for: Data("tx".utf8) + nr + ns, using: pairingKey)))
}

/// The network interface that is the cable to the laptop: the one holding an address the laptop
/// handed out (10.55.0.x). Connections are pinned to it so they cannot wander onto Wi-Fi.
func cableInterface() -> UInt32 {
    var list: UnsafeMutablePointer<ifaddrs>?
    guard getifaddrs(&list) == 0 else { return 0 }
    defer { freeifaddrs(list) }
    var p = list
    while let i = p {
        if let sa = i.pointee.ifa_addr, sa.pointee.sa_family == UInt8(AF_INET) {
            let ip = sa.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { UInt32(bigEndian: $0.pointee.sin_addr.s_addr) }
            if ip >> 8 == 0x0A3700 { return if_nametoindex(i.pointee.ifa_name) }
        }
        p = i.pointee.ifa_next
    }
    return 0
}

// MARK: TCP link to the laptop. Reconnects forever; frames are simply not encoded while down.
final class Link {
    private var fd: Int32 = -1
    private let lock = NSLock()
    private(set) var wantKeyframe = true
    var bytes = 0
    let port: UInt16
    let name: String
    let carriesInput: Bool
    init(port: UInt16, name: String, carriesInput: Bool) { self.port = port; self.name = name; self.carriesInput = carriesInput }

    var up: Bool { lock.lock(); defer { lock.unlock() }; return fd >= 0 }
    func takeKeyframeRequest() -> Bool { lock.lock(); defer { lock.unlock() }; let k = wantKeyframe; wantKeyframe = false; return k }

    func keepConnected() {
        Thread.detachNewThread {
            while true {
                if !self.up, let s = Link.open(self.port) {
                    self.lock.lock(); self.fd = s; self.wantKeyframe = true; self.lock.unlock()
                    log("\(self.name) connected to \(host):\(self.port)")
                    if self.carriesInput { Thread.detachNewThread { Link.readInput(s) } }
                }
                Thread.sleep(forTimeInterval: 1)
            }
        }
    }

    /// The laptop sends its key and pointer events back on the same connection, 8 bytes each.
    private static func readInput(_ s: Int32) {
        let inj = Injector()
        var rec = [UInt8](repeating: 0, count: 8)
        while true {
            var got = 0
            while got < 8 {
                let n = rec.withUnsafeMutableBytes { read(s, $0.baseAddress! + got, 8 - got) }
                if n <= 0 { inj.releaseAll(); log("laptop disconnected — removing the extra screen"); exit(0) }
                got += n
            }
            inj.handle(type: UInt16(rec[0]) | UInt16(rec[1]) << 8, code: UInt16(rec[2]) | UInt16(rec[3]) << 8,
                       value: Int32(bitPattern: UInt32(rec[4]) | UInt32(rec[5]) << 8 | UInt32(rec[6]) << 16 | UInt32(rec[7]) << 24))
        }
    }

    private static func open(_ port: UInt16) -> Int32? {
        let s = socket(AF_INET, SOCK_STREAM, 0)
        guard s >= 0 else { return nil }
        var one: Int32 = 1
        setsockopt(s, IPPROTO_TCP, TCP_NODELAY, &one, 4)
        setsockopt(s, SOL_SOCKET, SO_NOSIGPIPE, &one, 4)
        var a = sockaddr_in()
        a.sin_family = sa_family_t(AF_INET)
        a.sin_port = port.bigEndian
        inet_pton(AF_INET, host, &a.sin_addr)
        var idx = cableInterface()
        guard idx != 0 else { close(s); return nil } // cable not plugged in: do not try other networks
        setsockopt(s, IPPROTO_IP, IP_BOUND_IF, &idx, 4)
        let r = withUnsafePointer(to: &a) { $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(s, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) } }
        if r != 0 { close(s); return nil }
        guard handshake(s) else {
            close(s)
            log("peer at \(host):\(port) does not hold the pairing key — refusing to send it anything")
            return nil
        }
        return s
    }

    func send(_ d: Data) {
        lock.lock(); defer { lock.unlock() }
        guard fd >= 0 else { return }
        let ok = d.withUnsafeBytes { (p: UnsafeRawBufferPointer) -> Bool in
            var off = 0
            while off < p.count {
                let n = write(fd, p.baseAddress! + off, p.count - off)
                if n <= 0 { return false }
                off += n
            }
            return true
        }
        if ok { bytes += d.count; return }
        close(fd); fd = -1
        if carriesInput { log("laptop disconnected — removing the extra screen"); exit(0) }
        log("\(name) link lost, reconnecting")
    }
}
let link = Link(port: port, name: "video", carriesInput: true)
let soundLink = Link(port: port + 1, name: "sound", carriesInput: false)

// MARK: hardware encoder
var session: VTCompressionSession!
do {
    var s: VTCompressionSession?
    let r = VTCompressionSessionCreate(allocator: nil, width: Int32(W), height: Int32(H), codecType: kCMVideoCodecType_H264,
                                       encoderSpecification: nil, imageBufferAttributes: nil, compressedDataAllocator: nil,
                                       outputCallback: nil, refcon: nil, compressionSessionOut: &s)
    guard r == 0, let s else { log("encoder create failed: \(r)"); exit(1) }
    VTSessionSetProperty(s, key: kVTCompressionPropertyKey_RealTime, value: kCFBooleanTrue)
    VTSessionSetProperty(s, key: kVTCompressionPropertyKey_ProfileLevel, value: kVTProfileLevel_H264_High_AutoLevel)
    VTSessionSetProperty(s, key: kVTCompressionPropertyKey_AllowFrameReordering, value: kCFBooleanFalse) // no B-frames: no added delay
    VTSessionSetProperty(s, key: kVTCompressionPropertyKey_MaxKeyFrameInterval, value: 3600 as CFNumber)
    // The cable carries about 300 Mbit/s, so spend bits freely: a high rate target, and a ceiling
    // on how coarsely any frame may be quantised (lower = sharper) so text stays crisp in motion.
    VTSessionSetProperty(s, key: kVTCompressionPropertyKey_AverageBitRate, value: 120_000_000 as CFNumber)
    let qp = VTSessionSetProperty(s, key: kVTCompressionPropertyKey_MaxAllowedFrameQP, value: 18 as CFNumber)
    log("quality ceiling (max QP 18): \(qp == 0 ? "on" : "not supported, status \(qp)")")
    VTSessionSetProperty(s, key: kVTCompressionPropertyKey_ExpectedFrameRate, value: 60 as CFNumber)
    VTCompressionSessionPrepareToEncodeFrames(s)
    session = s
}

let startCode: [UInt8] = [0, 0, 0, 1]
/// Turns the encoder's output (length-prefixed units) into the start-code byte stream the laptop expects.
func emit(_ sb: CMSampleBuffer) {
    var out = Data()
    let att = CMSampleBufferGetSampleAttachmentsArray(sb, createIfNecessary: false) as? [[CFString: Any]]
    let isKey = !((att?.first?[kCMSampleAttachmentKey_NotSync] as? Bool) ?? false)
    if isKey, let fmt = CMSampleBufferGetFormatDescription(sb) {
        var count = 0
        CMVideoFormatDescriptionGetH264ParameterSetAtIndex(fmt, parameterSetIndex: 0, parameterSetPointerOut: nil, parameterSetSizeOut: nil, parameterSetCountOut: &count, nalUnitHeaderLengthOut: nil)
        for i in 0..<count {
            var p: UnsafePointer<UInt8>?
            var n = 0
            if CMVideoFormatDescriptionGetH264ParameterSetAtIndex(fmt, parameterSetIndex: i, parameterSetPointerOut: &p, parameterSetSizeOut: &n, parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil) == 0, let p {
                out.append(contentsOf: startCode); out.append(p, count: n)
            }
        }
    }
    guard let bb = CMSampleBufferGetDataBuffer(sb) else { return }
    let total = CMBlockBufferGetDataLength(bb)
    var raw = [UInt8](repeating: 0, count: total)
    guard CMBlockBufferCopyDataBytes(bb, atOffset: 0, dataLength: total, destination: &raw) == 0 else { return }
    var off = 0
    while off + 4 <= total {
        let n = Int(raw[off]) << 24 | Int(raw[off + 1]) << 16 | Int(raw[off + 2]) << 8 | Int(raw[off + 3])
        off += 4
        guard n > 0, off + n <= total else { break }
        out.append(contentsOf: startCode); out.append(contentsOf: raw[off..<off + n])
        off += n
    }
    // An "access unit delimiter" after the frame tells the receiver's parser the frame is complete,
    // so it is decoded now instead of when the next frame arrives.
    out.append(contentsOf: startCode + [0x09, 0xF0])
    link.send(out)
}

func encode(_ pb: CVPixelBuffer, _ pts: CMTime) {
    guard link.up else { return }
    let props = link.takeKeyframeRequest() ? [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary : nil
    VTCompressionSessionEncodeFrame(session, imageBuffer: pb, presentationTimeStamp: pts, duration: .invalid, frameProperties: props, infoFlagsOut: nil) { status, _, sb in
        if status == 0, let sb { emit(sb) }
    }
}

// MARK: wait for the laptop
// The extra screen must exist only while the laptop is really there, otherwise windows end up on
// a screen nobody can see. So: connect first, create the display after, and when the video link
// drops the whole program exits (taking the display with it); run.sh starts it again to wait.
link.keepConnected()
soundLink.keepConnected()
log("waiting for the laptop on the cable…")
while !link.up { Thread.sleep(forTimeInterval: 0.5) }

// MARK: virtual display
let desc = CGVirtualDisplayDescriptor()
desc.setDispatchQueue(DispatchQueue.main)
desc.name = "Shabnam"
desc.maxPixelsWide = UInt32(W * 2) // room for double-precision ("Retina") drawing
desc.maxPixelsHigh = UInt32(H * 2)
desc.sizeInMillimeters = CGSize(width: 309, height: 174) // 14-inch 16:9 panel
desc.productID = 0x4E42
desc.vendorID = 0x4543
desc.serialNum = 1
let display = CGVirtualDisplay(descriptor: desc)!
let settings = CGVirtualDisplaySettings()
settings.hiDPI = 1 // offer each size in a double-precision variant too
settings.modes = [1920, 1600, 1280, 1024, 960].map { CGVirtualDisplayMode(width: UInt($0), height: UInt($0 * 9 / 16), refreshRate: 60) }
guard display.apply(settings) else { log("could not apply display mode"); exit(1) }
let displayID = display.displayID
// macOS may start a new display in any of its modes; select the one asked for.
func selectNativeMode() {
    let all = CGDisplayCopyAllDisplayModes(displayID, [kCGDisplayShowDuplicateLowResolutionModes: true] as CFDictionary) as? [CGDisplayMode] ?? []
    // Prefer the double-precision variant of the requested size; fall back to the plain one.
    guard let m = all.first(where: { $0.width == looksLike && $0.pixelWidth == looksLike * 2 }) ?? all.first(where: { $0.width == looksLike }) else {
        log("no mode that looks like \(looksLike) wide; offered: " + all.map { "\($0.width)pt/\($0.pixelWidth)px" }.joined(separator: " ")); return
    }
    var cfg: CGDisplayConfigRef?
    CGBeginDisplayConfiguration(&cfg)
    CGConfigureDisplayWithDisplayMode(cfg, displayID, m, nil)
    let r = CGCompleteDisplayConfiguration(cfg, .forSession)
    if r != .success { log("mode switch failed: \(r.rawValue)") }
}
selectNativeMode()
if let m = CGDisplayCopyDisplayMode(displayID) {
    log("virtual display \(displayID): \(m.width)x\(m.height) points, \(m.pixelWidth)x\(m.pixelHeight) pixels")
}

// MARK: capture
final class Capture: NSObject, SCStreamOutput, SCStreamDelegate {
    let q = DispatchQueue(label: "capture")
    var stream: SCStream?
    var last: CVPixelBuffer?
    var lastAt = Date.distantPast
    var refines = 0
    var frames = 0

    func start(_ d: SCDisplay) {
        let c = SCStreamConfiguration()
        c.width = W; c.height = H
        c.minimumFrameInterval = CMTime(value: 1, timescale: 60)
        c.pixelFormat = kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
        c.colorMatrix = CGDisplayStream.yCbCrMatrix_ITU_R_709_2
        c.showsCursor = true
        c.queueDepth = 3
        // Also take the Mac's sound output, to play on the laptop's speakers.
        c.capturesAudio = true
        c.sampleRate = 48000
        c.channelCount = 2
        c.excludesCurrentProcessAudio = true
        let s = SCStream(filter: SCContentFilter(display: d, excludingWindows: []), configuration: c, delegate: self)
        do {
            try s.addStreamOutput(self, type: .screen, sampleHandlerQueue: q)
            try s.addStreamOutput(self, type: .audio, sampleHandlerQueue: DispatchQueue(label: "sound"))
        } catch { log("add output: \(error)"); exit(1) }
        s.startCapture { e in
            if let e { log("capture failed: \(e.localizedDescription) — the terminal needs the Screen Recording permission"); exit(1) }
            log("capturing")
        }
        stream = s
        // macOS only delivers a frame when the picture changes. After motion stops, re-send the
        // still picture a few times so the encoder can sharpen it to full quality.
        let t = DispatchSource.makeTimerSource(queue: q)
        t.schedule(deadline: .now() + 1, repeating: .milliseconds(33))
        t.setEventHandler { [unowned self] in
            guard let pb = last, refines < 10, Date().timeIntervalSince(lastAt) > 0.04 else { return }
            refines += 1
            encode(pb, CMClockGetTime(CMClockGetHostTimeClock()))
        }
        t.resume()
        timer = t
        let st = DispatchSource.makeTimerSource(queue: q)
        st.schedule(deadline: .now() + 5, repeating: .seconds(5))
        st.setEventHandler { [unowned self] in
            log(String(format: "%.0f fps captured, %.1f Mbit/s", Double(frames) / 5, Double(link.bytes) * 8 / 5e6))
            frames = 0; link.bytes = 0
        }
        st.resume()
        stats = st
    }
    var timer: DispatchSourceTimer?
    var stats: DispatchSourceTimer?

    func stream(_ stream: SCStream, didOutputSampleBuffer sb: CMSampleBuffer, of type: SCStreamOutputType) {
        if type == .audio { sound(sb); return }
        guard type == .screen, sb.isValid,
              let info = CMSampleBufferGetSampleAttachmentsArray(sb, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let raw = info.first?[.status] as? Int, SCFrameStatus(rawValue: raw) == .complete,
              let pb = sb.imageBuffer else { return }
        last = pb; lastAt = Date(); refines = 0; frames += 1
        encode(pb, sb.presentationTimeStamp)
    }
    /// macOS hands sound over as 32-bit floats, usually one buffer per channel; the laptop wants
    /// plain 16-bit samples with left and right alternating.
    func sound(_ sb: CMSampleBuffer) {
        guard soundLink.up, sb.isValid else { return }
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
            soundLink.send(out.withUnsafeBufferPointer { Data(buffer: $0) })
        }
    }
    func stream(_ stream: SCStream, didStopWithError error: Error) { log("capture stopped: \(error.localizedDescription)"); exit(1) }
}
let capture = Capture()

func findDisplay(_ tries: Int) {
    SCShareableContent.getExcludingDesktopWindows(false, onScreenWindowsOnly: false) { content, err in
        if let d = content?.displays.first(where: { $0.displayID == displayID }) { capture.start(d); return }
        if tries == 0 { log("virtual display not visible to screen capture: \(err?.localizedDescription ?? "not listed")"); exit(1) }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { findDisplay(tries - 1) }
    }
}
log("accessibility permission (needed for the laptop's keyboard and touchpad): \(Injector.checkPermission() ? "granted" : "NOT granted yet")")
findDisplay(20)
signal(SIGINT) { _ in exit(0) }
signal(SIGTERM) { _ in exit(0) }
RunLoop.main.run()
