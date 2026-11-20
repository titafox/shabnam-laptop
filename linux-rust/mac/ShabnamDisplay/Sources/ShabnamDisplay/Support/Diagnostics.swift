import Foundation

/// The text behind "Copy diagnostics": enough to understand a problem report, and nothing that
/// identifies the person or could unlock the link — no key material, user name, host name or
/// home path.
enum Diagnostics {
    /// Takes the user's name and home folder out of a string (system error messages sometimes
    /// contain paths).
    static func redact(_ text: String, home: String = NSHomeDirectory(), user: String = NSUserName(),
                       hostNames: [String] = [ProcessInfo.processInfo.hostName, Host.current().localizedName ?? ""]) -> String {
        var out = text
        if !home.isEmpty, home != "/" { out = out.replacingOccurrences(of: home, with: "~") }
        if user.count >= 3 { out = out.replacingOccurrences(of: user, with: "<user>") }
        for name in hostNames where name.count >= 3 { out = out.replacingOccurrences(of: name, with: "<host>") }
        return out
    }

    static func report(state: LinkState, stats: SessionStats.Snapshot, settings: Settings, cable: CableInterface?, laptop: LaptopStatus? = nil) -> String {
        let os = ProcessInfo.processInfo.operatingSystemVersion
        var lines: [String] = []
        lines.append("\(ProjectInfo.name) \(ProjectInfo.version) (\(ProjectInfo.build))")
        lines.append("macOS \(os.majorVersion).\(os.minorVersion).\(os.patchVersion)")
        lines.append("state: \(state.diagnosticName)")
        lines.append("uptime: \(stats.connectedSince.map { "\(Int(Date().timeIntervalSince($0))) s" } ?? "-")")
        lines.append(String(format: "video: %.0f fps, %.1f Mbit/s, %d frames sent, %d dropped by the encoder", stats.fps, stats.megabitsPerSecond, stats.framesSent, stats.framesDropped))
        lines.append("sound: \(stats.audioBytes) bytes sent, mode \(settings.audioMode.rawValue)")
        lines.append("last error: \(stats.lastError.map { redact($0) } ?? "none")")
        lines.append("display: \(settings.displayMode.rawValue), \(settings.frameRate) fps cap, quality \(settings.quality.rawValue) (\(settings.quality.averageBitRate / 1_000_000) Mbit/s, max QP \(settings.quality.maxQP)), cursor \(settings.showCursor ? "shown" : "hidden")")
        let input = settings.inputOptions
        lines.append(String(format: "input: %@, win/alt %@, scroll %@ x%.2f, pointer x%.2f, media keys %@",
                            settings.acceptInput ? "accepted" : "ignored", input.swapWinAlt ? "swapped" : "default",
                            input.reverseScroll ? "reversed" : "normal", input.scrollSpeed, input.pointerSpeed, input.mediaKeys ? "on" : "off"))
        lines.append("laptop: \(settings.host):\(settings.port), cable interface: \(cable.map { "\($0.name) (\($0.address))" } ?? "none")")
        if let laptop {
            let battery = laptop.battery.map { "\($0)%\(laptop.charging ? " charging" : laptop.externalPower ? " on charger" : "")" } ?? "none"
            let lid = laptop.lidClosed.map { $0 ? "closed" : "open" } ?? "unknown"
            lines.append("laptop control: receiver \(laptop.receiverVersion), protocol \(laptop.protocolVersion), brightness \(laptop.brightness)%, volume \(laptop.volume)/\(laptop.volumeSteps)\(laptop.muted ? " muted" : ""), battery \(battery), lid \(lid)")
        } else {
            lines.append("laptop control: not available")
        }
        lines.append("permissions: screen recording \(Permissions.screenRecordingGranted ? "granted" : "missing"), accessibility \(Permissions.accessibilityGranted ? "granted" : "missing")")
        lines.append("pairing key file: \(!PairingKey.exists ? "missing" : PairingKey.load() == nil ? "too short" : PairingKey.isPrivate ? "present" : "present, readable by others")")
        return redact(lines.joined(separator: "\n"))
    }
}

extension LinkState {
    /// A fixed English name, for logs and diagnostics.
    var diagnosticName: String {
        switch self {
        case .stopped: return "stopped"
        case .waiting: return "waiting"
        case .connected: return "connected"
        case .problem(let problem): return "problem (\(problem))"
        }
    }
}
