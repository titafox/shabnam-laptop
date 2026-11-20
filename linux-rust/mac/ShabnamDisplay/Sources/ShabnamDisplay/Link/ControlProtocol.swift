import Foundation

/// What the laptop reports about itself on the control channel.
struct LaptopStatus: Equatable {
    /// Panel backlight, 0–100 percent.
    var brightness: Int
    /// Speaker volume, 0–`volumeSteps`.
    var volume: Int
    var volumeSteps: Int
    var muted: Bool
    /// Percent; nil when the laptop reports no battery.
    var battery: Int?
    var charging: Bool
    /// Running from the charger (charging, or already full).
    var externalPower: Bool
    /// nil until the laptop's lid sensor has reported.
    var lidClosed: Bool?
    /// The receiver program's version, e.g. "0.2.0".
    var receiverVersion: String
    var protocolVersion: Int
}

/// Wire format of the control channel (video port + 2), see docs/PROTOCOL.md §6.
/// Every message is `type u8 | length u8 | payload[length]`.
enum ControlProtocol {
    static let portOffset: UInt16 = 2
    /// A longer payload than this is a protocol violation; the connection is closed.
    static let maxPayload = 32

    static let statusType: UInt8 = 0x81
    static let statusLength = 9

    /// Everything the Mac can ask of the laptop. Numbers are clamped before they are sent (and
    /// again by the laptop).
    enum Command: Equatable {
        case setBrightness(Int)
        case setVolume(Int)
        case setMute(Bool)
        /// Leave second-screen mode: the laptop goes back to its own desktop.
        case leave
        case getStatus

        var encoded: Data {
            switch self {
            case .setBrightness(let percent): return Data([0x01, 1, UInt8(max(0, min(100, percent)))])
            case .setVolume(let step): return Data([0x02, 1, UInt8(max(0, min(255, step)))])
            case .setMute(let on): return Data([0x03, 1, on ? 1 : 0])
            case .leave: return Data([0x04, 0])
            case .getStatus: return Data([0x05, 0])
            }
        }
    }

    /// One message from the laptop. nil for anything that is not a well-formed status: unknown
    /// types and short payloads are skipped. Out-of-range numbers are clamped; payload bytes
    /// beyond the ones defined are ignored.
    static func decodeStatus(type: UInt8, payload: Data) -> LaptopStatus? {
        guard type == statusType, payload.count >= statusLength else { return nil }
        let b = [UInt8](payload)
        let steps = Int(b[3])
        guard steps >= 1 else { return nil }
        let flags = b[4]
        return LaptopStatus(
            brightness: min(100, Int(b[1])),
            volume: min(steps, Int(b[2])),
            volumeSteps: steps,
            muted: flags & 0x01 != 0,
            battery: flags & 0x02 != 0 ? min(100, Int(b[5])) : nil,
            charging: flags & 0x04 != 0,
            externalPower: flags & 0x08 != 0,
            lidClosed: flags & 0x10 != 0 ? flags & 0x20 != 0 : nil,
            receiverVersion: "\(b[6]).\(b[7]).\(b[8])",
            protocolVersion: Int(b[0]))
    }

    struct Oversize: Error, Equatable {}

    /// Cuts the incoming byte stream into messages. Holds at most one incomplete message.
    struct Parser {
        private var pending: [UInt8] = []

        /// How many more bytes complete the message in progress (2 = a fresh header).
        var wanted: Int { pending.count < 2 ? 2 - pending.count : 2 + Int(pending[1]) - pending.count }

        mutating func feed<Bytes: Sequence>(_ bytes: Bytes) throws -> [LaptopStatus] where Bytes.Element == UInt8 {
            var out: [LaptopStatus] = []
            for byte in bytes {
                pending.append(byte)
                guard pending.count >= 2 else { continue }
                let length = Int(pending[1])
                guard length <= ControlProtocol.maxPayload else { throw Oversize() }
                if pending.count == 2 + length {
                    if let status = ControlProtocol.decodeStatus(type: pending[0], payload: Data(pending[2...])) { out.append(status) }
                    pending.removeAll(keepingCapacity: true)
                }
            }
            return out
        }
    }
}
