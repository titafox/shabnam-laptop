import CryptoKit
import Darwin
import Foundation
import Security

/// Mutual authentication, run at the start of every connection before anything else is sent or
/// accepted. Both ends prove they hold the pairing key without revealing it:
///
///     sender   -> 16 random bytes Ns
///     receiver -> 16 random bytes Nr, HMAC-SHA256(key, "rx" | Ns | Nr)
///     sender   -> HMAC-SHA256(key, "tx" | Nr | Ns)
enum Handshake {
    static let nonceLength = 16
    static let tagLength = 32

    /// Proves to the laptop that we hold the key, and checks that it does too.
    static func perform(on fd: Int32, key: SymmetricKey) -> Bool {
        var tv = timeval(tv_sec: 5, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        defer {
            tv.tv_sec = 0
            setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        }
        var ns = Data(count: nonceLength)
        guard ns.withUnsafeMutableBytes({ SecRandomCopyBytes(kSecRandomDefault, nonceLength, $0.baseAddress!) }) == errSecSuccess,
              SocketIO.writeAll(fd, ns),
              let reply = SocketIO.readExact(fd, nonceLength + tagLength) else { return false }
        let nr = reply.prefix(nonceLength), theirs = reply.suffix(tagLength)
        // Constant-time comparison.
        guard HMAC<SHA256>.isValidAuthenticationCode(theirs, authenticating: Data("rx".utf8) + ns + nr, using: key) else { return false }
        return SocketIO.writeAll(fd, Data(HMAC<SHA256>.authenticationCode(for: Data("tx".utf8) + nr + ns, using: key)))
    }
}

/// Blocking whole-buffer reads and writes on a socket.
enum SocketIO {
    static func readExact(_ fd: Int32, _ count: Int) -> Data? {
        var data = Data(count: count)
        let ok = data.withUnsafeMutableBytes { readExact(fd, into: $0) }
        return ok ? data : nil
    }

    static func readExact(_ fd: Int32, into buffer: UnsafeMutableRawBufferPointer) -> Bool {
        var got = 0
        while got < buffer.count {
            let n = read(fd, buffer.baseAddress! + got, buffer.count - got)
            if n < 0 && errno == EINTR { continue }
            if n <= 0 { return false }
            got += n
        }
        return true
    }

    static func writeAll(_ fd: Int32, _ data: Data) -> Bool {
        data.withUnsafeBytes { p in
            var off = 0
            while off < p.count {
                let n = write(fd, p.baseAddress! + off, p.count - off)
                if n < 0 && errno == EINTR { continue }
                if n <= 0 { return false }
                off += n
            }
            return true
        }
    }
}
