import CryptoKit
import Darwin
import Foundation

/// One authenticated TCP connection to the laptop, pinned to the cable.
///
/// A `Link` only exists after the handshake has succeeded, so nothing can be sent to, or read
/// from, a peer that has not proved it holds the pairing key.
final class Link {
    enum OpenError: Error {
        /// No interface holds a cable address: the laptop is not plugged in.
        case noCable
        /// Nothing is listening (the laptop is not in second-screen mode).
        case unreachable
        /// Something answered, but could not prove it holds the pairing key.
        case authenticationFailed
    }

    private let fd: Int32
    private let sendLock = NSLock()
    private let stateLock = NSLock()
    private var alive = true
    private var closed = false
    private var keyframeWanted = true
    private var sentBytes = 0

    private init(fd: Int32) { self.fd = fd }

    deinit { close() }

    /// Connects over the cable interface and runs the handshake.
    static func open(host: String, port: UInt16, key: SymmetricKey, timeout: TimeInterval = 3) -> Result<Link, OpenError> {
        // Cable not plugged in: do not try other networks.
        guard var index = cableInterfaceIndex() else { return .failure(.noCable) }

        let s = socket(AF_INET, SOCK_STREAM, 0)
        guard s >= 0 else { return .failure(.unreachable) }
        var one: Int32 = 1
        setsockopt(s, IPPROTO_TCP, TCP_NODELAY, &one, 4)
        setsockopt(s, SOL_SOCKET, SO_NOSIGPIPE, &one, 4)
        guard setsockopt(s, IPPROTO_IP, IP_BOUND_IF, &index, 4) == 0 else {
            Darwin.close(s)
            return .failure(.noCable)
        }

        var address = sockaddr_in()
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = port.bigEndian
        guard inet_pton(AF_INET, host, &address.sin_addr) == 1 else {
            Darwin.close(s)
            return .failure(.unreachable)
        }

        // Connect without blocking so an unresponsive peer cannot hold the app for a minute.
        let flags = fcntl(s, F_GETFL, 0)
        _ = fcntl(s, F_SETFL, flags | O_NONBLOCK)
        let r = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { connect(s, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) }
        }
        if r != 0 {
            guard errno == EINPROGRESS else { Darwin.close(s); return .failure(.unreachable) }
            var pfd = pollfd(fd: s, events: Int16(POLLOUT), revents: 0)
            guard poll(&pfd, 1, Int32(timeout * 1000)) == 1 else { Darwin.close(s); return .failure(.unreachable) }
            var err: Int32 = 0
            var len = socklen_t(MemoryLayout<Int32>.size)
            guard getsockopt(s, SOL_SOCKET, SO_ERROR, &err, &len) == 0, err == 0 else { Darwin.close(s); return .failure(.unreachable) }
        }
        _ = fcntl(s, F_SETFL, flags)

        guard Handshake.perform(on: s, key: key) else {
            Darwin.close(s)
            return .failure(.authenticationFailed)
        }
        // A receiver that stops reading must not be able to stall the capture pipeline forever.
        var tv = timeval(tv_sec: 5, tv_usec: 0)
        setsockopt(s, SOL_SOCKET, SO_SNDTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        return .success(Link(fd: s))
    }

    var isUp: Bool {
        stateLock.lock(); defer { stateLock.unlock() }
        return alive
    }

    /// The first frame on a new connection must be a keyframe. True once, then false.
    func takeKeyframeRequest() -> Bool {
        stateLock.lock(); defer { stateLock.unlock() }
        let wanted = keyframeWanted
        keyframeWanted = false
        return wanted
    }

    /// Bytes sent since the last call; used for the bitrate display.
    func takeSentBytes() -> Int {
        stateLock.lock(); defer { stateLock.unlock() }
        let n = sentBytes
        sentBytes = 0
        return n
    }

    /// Sends the whole buffer. A failure marks the link as down and wakes any blocked reader.
    @discardableResult
    func send(_ data: Data) -> Bool {
        sendLock.lock(); defer { sendLock.unlock() }
        guard isUp else { return false }
        guard SocketIO.writeAll(fd, data) else {
            shutdown()
            return false
        }
        stateLock.lock(); sentBytes += data.count; stateLock.unlock()
        return true
    }

    /// Blocks until the buffer is full. false when the peer has gone or `shutdown()` was called.
    func read(into buffer: UnsafeMutableRawBufferPointer) -> Bool {
        guard isUp else { return false }
        guard SocketIO.readExact(fd, into: buffer) else {
            shutdown()
            return false
        }
        return true
    }

    /// Marks the link as down and unblocks readers and writers. Safe from any thread, any number
    /// of times. The descriptor stays open until `close()`, so it cannot be reused under a reader.
    func shutdown() {
        stateLock.lock(); defer { stateLock.unlock() }
        guard alive else { return }
        alive = false
        if !closed { Darwin.shutdown(fd, SHUT_RDWR) }
    }

    /// Releases the socket. Call once no thread is reading from the link any more.
    func close() {
        shutdown()
        sendLock.lock(); defer { sendLock.unlock() } // let a writer that is mid-send finish first
        stateLock.lock(); defer { stateLock.unlock() }
        guard !closed else { return }
        closed = true
        Darwin.close(fd)
    }
}
