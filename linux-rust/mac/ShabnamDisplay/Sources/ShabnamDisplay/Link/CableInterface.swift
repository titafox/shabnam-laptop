import Darwin

/// The network interface that is the cable to the laptop.
struct CableInterface: Equatable {
    let index: UInt32
    /// BSD name, e.g. "en7".
    let name: String
    /// The Mac's own address on the cable, e.g. "10.55.0.2".
    let address: String

    /// The interface holding an address the laptop handed out (10.55.0.x). Connections are
    /// pinned to it so they cannot wander onto Wi-Fi. nil while the cable is not plugged in.
    static func find() -> CableInterface? {
        var list: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&list) == 0 else { return nil }
        defer { freeifaddrs(list) }
        var p = list
        while let i = p {
            if let sa = i.pointee.ifa_addr, sa.pointee.sa_family == UInt8(AF_INET) {
                let raw = sa.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { $0.pointee.sin_addr }
                let ip = UInt32(bigEndian: raw.s_addr)
                if isCableAddress(ip) {
                    let index = if_nametoindex(i.pointee.ifa_name)
                    guard index != 0 else { return nil }
                    return CableInterface(index: index, name: String(cString: i.pointee.ifa_name),
                                          address: "\(ip >> 24).\((ip >> 16) & 255).\((ip >> 8) & 255).\(ip & 255)")
                }
            }
            p = i.pointee.ifa_next
        }
        return nil
    }

    /// 10.55.0.0/24, host byte order.
    static func isCableAddress(_ ip: UInt32) -> Bool { ip >> 8 == 0x0A3700 }
}

/// The index of the cable interface, for `IP_BOUND_IF`.
func cableInterfaceIndex() -> UInt32? { CableInterface.find()?.index }
