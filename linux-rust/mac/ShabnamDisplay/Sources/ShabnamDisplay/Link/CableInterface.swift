import Darwin

/// The network interface that is the cable to the laptop: the one holding an address the laptop
/// handed out (10.55.0.x). Connections are pinned to it so they cannot wander onto Wi-Fi.
/// Returns nil while the cable is not plugged in.
func cableInterfaceIndex() -> UInt32? {
    var list: UnsafeMutablePointer<ifaddrs>?
    guard getifaddrs(&list) == 0 else { return nil }
    defer { freeifaddrs(list) }
    var p = list
    while let i = p {
        if let sa = i.pointee.ifa_addr, sa.pointee.sa_family == UInt8(AF_INET) {
            let ip = sa.withMemoryRebound(to: sockaddr_in.self, capacity: 1) { UInt32(bigEndian: $0.pointee.sin_addr.s_addr) }
            if ip >> 8 == 0x0A3700 { // 10.55.0.0/24
                let index = if_nametoindex(i.pointee.ifa_name)
                return index == 0 ? nil : index
            }
        }
        p = i.pointee.ifa_next
    }
    return nil
}
