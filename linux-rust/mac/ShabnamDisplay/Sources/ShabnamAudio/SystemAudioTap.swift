import AVFoundation
import AudioToolbox
import CoreAudio
import Foundation

/// Captures everything the Mac plays (except this process) with a Core Audio process tap and
/// hands it over as 48 kHz, 16-bit, stereo, interleaved samples.
///
/// With `muteMac` the tap is created "muted when tapped": while it runs, the tapped sound is
/// not played on the Mac's own output device, so it is heard on the receiving end only. The
/// muting lasts only while the tap is being read: stopping, or the process ending in any way,
/// ends it. (The tap object itself can outlive a process that was killed; such leftovers are
/// harmless and are removed the next time a tap is started.)
///
/// Needs the system-audio-recording permission (NSAudioCaptureUsageDescription). Without it the
/// tap delivers silence.
@available(macOS 14.2, *)
public final class SystemAudioTap {
    public enum TapError: Error, CustomStringConvertible {
        case createTap(OSStatus)
        case format(OSStatus)
        case createAggregate(OSStatus)
        case ioProc(OSStatus)
        case start(OSStatus)

        public var description: String {
            switch self {
            case .createTap(let s): return "could not create the audio tap (\(s))"
            case .format(let s): return "could not read the tap's format (\(s))"
            case .createAggregate(let s): return "could not create the capture device (\(s))"
            case .ioProc(let s): return "could not install the capture callback (\(s))"
            case .start(let s): return "could not start capturing (\(s))"
            }
        }
    }

    public static let outputSampleRate = 48000.0

    /// Every tap this code creates carries this name, which is how leftovers are recognised.
    public static let tapName = "Shabnam Display"

    private let muteMac: Bool
    private let tapOnly: Bool
    private let handler: (Data) -> Void
    private let log: (String) -> Void

    private let control = DispatchQueue(label: "system-audio-tap.control")
    private let io = DispatchQueue(label: "system-audio-tap.io")

    // control queue
    private var tapID = AudioObjectID(kAudioObjectUnknown)
    private var aggregateID = AudioObjectID(kAudioObjectUnknown)
    private var procID: AudioDeviceIOProcID?
    private var listening = false
    private var wanted = false
    private var listener: AudioObjectPropertyListenerBlock?

    // io queue
    private var converter: AVAudioConverter?
    private var inputFormat: AVAudioFormat?
    private let outputFormat = AVAudioFormat(commonFormat: .pcmFormatInt16, sampleRate: SystemAudioTap.outputSampleRate, channels: 2, interleaved: true)!

    /// The tap's own sample format while running (for diagnostics).
    public private(set) var sourceDescription = ""

    /// - handler: called on a private queue with interleaved s16 stereo at 48 kHz.
    /// - tapOnly: do not tie the capture device to the Mac's output device (diagnostics; this is
    ///   also what is tried automatically when the output device cannot be used as a clock).
    public init(muteMac: Bool, tapOnly: Bool = false, log: @escaping (String) -> Void = { _ in }, handler: @escaping (Data) -> Void) {
        self.muteMac = muteMac
        self.tapOnly = tapOnly
        self.log = log
        self.handler = handler
    }

    deinit { stop() }

    /// Starts capturing. If the Mac's default output device changes later (headphones plugged in,
    /// another device chosen), the tap is rebuilt for the new device.
    public func start() throws {
        try control.sync {
            wanted = true
            try build()
            watchDefaultOutput()
        }
    }

    /// Stops capturing and removes the tap; the Mac's own output is audible again at once.
    /// No samples are delivered after this returns.
    public func stop() {
        control.sync {
            wanted = false
            unwatchDefaultOutput()
            teardown()
        }
        io.sync {}
    }

    // MARK: build and teardown (control queue)

    private func build() throws {
        if let stale = Self.removeLeftoverTaps(), stale > 0 { log("removed \(stale) leftover audio tap(s) from an earlier run") }
        do {
            try build(withOutputDevice: !tapOnly)
        } catch where !tapOnly {
            // Some output devices cannot be part of a capture device; the tap can stand alone.
            log("audio tap: \(error) — retrying without the output device")
            try build(withOutputDevice: false)
        }
    }

    private func build(withOutputDevice: Bool) throws {
        let description = CATapDescription(stereoGlobalTapButExcludeProcesses: Self.ownProcessObject().map { [$0] } ?? [])
        description.uuid = UUID()
        description.name = Self.tapName
        // Not private, so that a later run can find and remove it if this process is killed.
        description.isPrivate = false
        description.muteBehavior = muteMac ? .mutedWhenTapped : .unmuted

        var tap = AudioObjectID(kAudioObjectUnknown)
        var status = AudioHardwareCreateProcessTap(description, &tap)
        guard status == noErr, tap != kAudioObjectUnknown else { throw TapError.createTap(status) }
        tapID = tap

        do {
            var asbd = AudioStreamBasicDescription()
            var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
            var address = Self.address(kAudioTapPropertyFormat)
            status = AudioObjectGetPropertyData(tap, &address, 0, nil, &size, &asbd)
            guard status == noErr, let format = AVAudioFormat(streamDescription: &asbd) else { throw TapError.format(status) }

            // A private aggregate device is what turns the tap into something an IOProc can read.
            // The current output device is its clock; the tap is the only input.
            var dictionary: [String: Any] = [
                kAudioAggregateDeviceNameKey: "Shabnam Display Capture",
                kAudioAggregateDeviceUIDKey: UUID().uuidString,
                kAudioAggregateDeviceIsPrivateKey: true,
                kAudioAggregateDeviceIsStackedKey: false,
                kAudioAggregateDeviceTapAutoStartKey: true,
                kAudioAggregateDeviceTapListKey: [[
                    kAudioSubTapDriftCompensationKey: true,
                    kAudioSubTapUIDKey: description.uuid.uuidString,
                ]],
            ]
            if withOutputDevice, let outputUID = Self.defaultOutputDeviceUID() {
                dictionary[kAudioAggregateDeviceMainSubDeviceKey] = outputUID
                dictionary[kAudioAggregateDeviceSubDeviceListKey] = [[kAudioSubDeviceUIDKey: outputUID]]
            }
            var aggregate = AudioObjectID(kAudioObjectUnknown)
            status = AudioHardwareCreateAggregateDevice(dictionary as CFDictionary, &aggregate)
            guard status == noErr, aggregate != kAudioObjectUnknown else { throw TapError.createAggregate(status) }
            aggregateID = aggregate

            io.sync {
                inputFormat = format
                converter = AVAudioConverter(from: format, to: outputFormat)
            }
            sourceDescription = "\(Int(asbd.mSampleRate)) Hz, \(asbd.mChannelsPerFrame) ch, \(asbd.mBitsPerChannel)-bit"
                + (dictionary[kAudioAggregateDeviceMainSubDeviceKey] == nil ? ", tap only" : ", clocked by the output device")

            var proc: AudioDeviceIOProcID?
            status = AudioDeviceCreateIOProcIDWithBlock(&proc, aggregate, io) { [weak self] _, input, _, _, _ in
                self?.process(input)
            }
            guard status == noErr, let proc else { throw TapError.ioProc(status) }
            procID = proc
            status = AudioDeviceStart(aggregate, proc)
            guard status == noErr else { throw TapError.start(status) }
        } catch {
            teardown()
            throw error
        }
    }

    private func teardown() {
        if aggregateID != kAudioObjectUnknown {
            if let procID {
                AudioDeviceStop(aggregateID, procID)
                AudioDeviceDestroyIOProcID(aggregateID, procID)
            }
            AudioHardwareDestroyAggregateDevice(aggregateID)
        }
        procID = nil
        aggregateID = AudioObjectID(kAudioObjectUnknown)
        if tapID != kAudioObjectUnknown {
            let status = AudioHardwareDestroyProcessTap(tapID)
            if status != noErr { log("could not remove the audio tap (\(status))") }
        }
        tapID = AudioObjectID(kAudioObjectUnknown)
        io.async {
            self.converter = nil
            self.inputFormat = nil
        }
    }

    private func watchDefaultOutput() {
        guard !listening else { return }
        let block: AudioObjectPropertyListenerBlock = { [weak self] _, _ in
            guard let self, self.wanted else { return }
            self.log("the Mac's output device changed — rebuilding the audio tap")
            self.teardown()
            do { try self.build() } catch { self.log("audio tap: \(error)") }
        }
        var address = Self.address(kAudioHardwarePropertyDefaultOutputDevice)
        if AudioObjectAddPropertyListenerBlock(AudioObjectID(kAudioObjectSystemObject), &address, control, block) == noErr {
            listener = block
            listening = true
        }
    }

    private func unwatchDefaultOutput() {
        guard listening, let listener else { return }
        var address = Self.address(kAudioHardwarePropertyDefaultOutputDevice)
        AudioObjectRemovePropertyListenerBlock(AudioObjectID(kAudioObjectSystemObject), &address, control, listener)
        self.listener = nil
        listening = false
    }

    // MARK: samples (io queue)

    private func process(_ input: UnsafePointer<AudioBufferList>) {
        guard let converter, let inputFormat,
              let source = AVAudioPCMBuffer(pcmFormat: inputFormat, bufferListNoCopy: input, deallocator: nil),
              source.frameLength > 0 else { return }
        let capacity = AVAudioFrameCount(Double(source.frameLength) * Self.outputSampleRate / inputFormat.sampleRate) + 64
        guard let out = AVAudioPCMBuffer(pcmFormat: outputFormat, frameCapacity: capacity) else { return }
        var given = false
        var error: NSError?
        converter.convert(to: out, error: &error) { _, status in
            if given {
                status.pointee = .noDataNow
                return nil
            }
            given = true
            status.pointee = .haveData
            return source
        }
        guard error == nil, out.frameLength > 0, let samples = out.int16ChannelData?[0] else { return }
        handler(Data(bytes: samples, count: Int(out.frameLength) * 4))
    }

    // MARK: Core Audio helpers

    private static func address(_ selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress(mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    }

    /// This process as Core Audio sees it, so its own sound can be left out of the tap.
    private static func ownProcessObject() -> AudioObjectID? {
        var pid = getpid()
        var object = AudioObjectID(kAudioObjectUnknown)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        var address = Self.address(kAudioHardwarePropertyTranslatePIDToProcessObject)
        let status = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address,
                                                UInt32(MemoryLayout<pid_t>.size), &pid, &size, &object)
        return status == noErr && object != kAudioObjectUnknown ? object : nil
    }

    public static func defaultOutputDeviceID() -> AudioObjectID? {
        var device = AudioObjectID(kAudioObjectUnknown)
        var size = UInt32(MemoryLayout<AudioObjectID>.size)
        var address = Self.address(kAudioHardwarePropertyDefaultOutputDevice)
        let status = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size, &device)
        return status == noErr && device != kAudioObjectUnknown ? device : nil
    }

    private static func defaultOutputDeviceUID() -> String? {
        guard let device = defaultOutputDeviceID() else { return nil }
        var uid: Unmanaged<CFString>?
        var size = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
        var address = Self.address(kAudioDevicePropertyDeviceUID)
        guard AudioObjectGetPropertyData(device, &address, 0, nil, &size, &uid) == noErr, let uid else { return nil }
        return uid.takeRetainedValue() as String
    }

    private static func visibleTaps() -> [AudioObjectID] {
        var size: UInt32 = 0
        var address = Self.address(kAudioHardwarePropertyTapList)
        let system = AudioObjectID(kAudioObjectSystemObject)
        guard AudioObjectGetPropertyDataSize(system, &address, 0, nil, &size) == noErr, size > 0 else { return [] }
        var ids = [AudioObjectID](repeating: 0, count: Int(size) / MemoryLayout<AudioObjectID>.size)
        guard AudioObjectGetPropertyData(system, &address, 0, nil, &size, &ids) == noErr else { return [] }
        return Array(ids.prefix(Int(size) / MemoryLayout<AudioObjectID>.size))
    }

    private static func isOurs(_ tap: AudioObjectID) -> Bool {
        var description: Unmanaged<CATapDescription>?
        var size = UInt32(MemoryLayout<Unmanaged<CATapDescription>?>.size)
        var address = Self.address(kAudioTapPropertyDescription)
        guard AudioObjectGetPropertyData(tap, &address, 0, nil, &size, &description) == noErr, let description else { return false }
        return description.takeRetainedValue().name == tapName
    }

    /// How many taps created by this code currently exist (diagnostics).
    public static func ownTapCount() -> Int { visibleTaps().filter(isOurs).count }

    /// Removes taps left behind by an earlier run of this code that was killed before it could
    /// clean up. Only taps carrying `tapName` are touched. Returns how many were removed.
    @discardableResult
    public static func removeLeftoverTaps() -> Int? {
        var removed = 0
        for tap in visibleTaps() where isOurs(tap) {
            if AudioHardwareDestroyProcessTap(tap) == noErr { removed += 1 }
        }
        return removed
    }
}
