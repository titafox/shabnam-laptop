// audio-tap-probe — a diagnostic for the "sound only on the laptop" mode. It creates the same
// system audio tap the app uses, plays a short system sound, and reports what the tap captured.
//
//   audio-tap-probe                 mute-when-tapped tap, one test sound, peak level
//   audio-tap-probe --volume        also repeats at a low Mac volume and with the Mac muted
//                                   (the previous volume and mute state are restored)
//   audio-tap-probe --tap-only      do not tie the capture device to the Mac's output device
//   audio-tap-probe --listen        keep the tap open for 6 s and then close it: play something
//                                   meanwhile — the Mac should go silent, then come back
//   audio-tap-probe --hold          keep the tap open until killed (to test cleanup on exit)
//   audio-tap-probe --list          only print how many of this project's taps exist
//   audio-tap-probe --purge         remove taps left behind by a killed run
//
// While the tap is open the Mac's own speakers are silent; this lasts a few seconds.
import Foundation
import ShabnamAudio


let arguments = Set(CommandLine.arguments.dropFirst())

func shell(_ path: String, _ args: [String]) -> String {
    let p = Process()
    p.executableURL = URL(fileURLWithPath: path)
    p.arguments = args
    let pipe = Pipe()
    p.standardOutput = pipe
    p.standardError = FileHandle.nullDevice
    do { try p.run() } catch { return "" }
    p.waitUntilExit()
    return String(decoding: pipe.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
}

func playTestSound() { _ = shell("/usr/bin/afplay", ["-v", "0.3", "/System/Library/Sounds/Glass.aiff"]) }

@available(macOS 14.2, *)
func run() {
    if arguments.contains("--list") {
        print("taps: \(SystemAudioTap.ownTapCount())")
        exit(0)
    }
    if arguments.contains("--purge") {
        print("removed: \(SystemAudioTap.removeLeftoverTaps() ?? 0)")
        exit(0)
    }

    let lock = NSLock()
    var peak: Int16 = 0
    var bytes = 0
    let tap = SystemAudioTap(muteMac: !arguments.contains("--unmuted"), tapOnly: arguments.contains("--tap-only"), log: { print($0) }) { data in
        data.withUnsafeBytes { raw in
            let samples = raw.bindMemory(to: Int16.self)
            var p: Int16 = 0
            for s in samples { p = max(p, s == .min ? .max : abs(s)) }
            lock.lock()
            peak = max(peak, p)
            bytes += data.count
            lock.unlock()
        }
    }

    func measure(_ label: String) {
        lock.lock(); peak = 0; bytes = 0; lock.unlock()
        let started = Date()
        playTestSound()
        Thread.sleep(forTimeInterval: 0.2)
        let seconds = Date().timeIntervalSince(started)
        lock.lock(); let (p, b) = (peak, bytes); lock.unlock()
        print(String(format: "%@: peak %d / 32767 (%.1f dBFS), %.0f frames/s delivered", label, Int(p),
                     p > 0 ? 20 * log10(Double(p) / 32767) : -Double.infinity, Double(b) / 4 / seconds))
    }

    print("taps before: \(SystemAudioTap.ownTapCount())")
    do { try tap.start() } catch {
        print("FAILED: \(error)")
        exit(1)
    }
    print("tap running, source format: \(tap.sourceDescription); taps: \(SystemAudioTap.ownTapCount())")

    if arguments.contains("--listen") {
        print("the Mac's own output should be silent for the next 6 seconds…")
        Thread.sleep(forTimeInterval: 6)
        tap.stop()
        print("tap stopped — the Mac's sound should be back")
        exit(0)
    }
    if arguments.contains("--hold") {
        print("holding the tap open (pid \(getpid())); kill this process to test cleanup")
        while true { Thread.sleep(forTimeInterval: 60) }
    }

    Thread.sleep(forTimeInterval: 0.3)
    measure("Mac volume as found")

    if arguments.contains("--volume") {
        let volume = shell("/usr/bin/osascript", ["-e", "output volume of (get volume settings)"])
        let muted = shell("/usr/bin/osascript", ["-e", "output muted of (get volume settings)"])
        print("Mac volume was \(volume), muted: \(muted)")
        if Int(volume) != nil {
            _ = shell("/usr/bin/osascript", ["-e", "set volume output volume 5 without output muted"])
            measure("Mac volume 5 %")
            _ = shell("/usr/bin/osascript", ["-e", "set volume with output muted"])
            measure("Mac muted")
            _ = shell("/usr/bin/osascript", ["-e", "set volume output volume \(volume) \(muted == "true" ? "with" : "without") output muted"])
            print("restored: volume \(shell("/usr/bin/osascript", ["-e", "output volume of (get volume settings)"])), muted: \(shell("/usr/bin/osascript", ["-e", "output muted of (get volume settings)"]))")
        } else {
            print("this output device has no volume setting; skipped")
        }
    }

    tap.stop()
    print("tap stopped; taps after: \(SystemAudioTap.ownTapCount())")
    Thread.sleep(forTimeInterval: 0.5)
    print("half a second later: \(SystemAudioTap.ownTapCount())")
}

if #available(macOS 14.2, *) {
    run()
} else {
    print("needs macOS 14.2 or later")
    exit(2)
}
