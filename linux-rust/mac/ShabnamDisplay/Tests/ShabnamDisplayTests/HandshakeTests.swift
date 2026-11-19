import CryptoKit
import Darwin
import XCTest
@testable import ShabnamDisplay

/// Runs the sender's handshake against a reference implementation of the receiver's side
/// (see docs/PROTOCOL.md) over a local socket pair. Uses throwaway keys only.
final class HandshakeTests: XCTestCase {
    private func socketPair() -> (Int32, Int32) {
        var fds: [Int32] = [0, 0]
        XCTAssertEqual(socketpair(AF_UNIX, SOCK_STREAM, 0, &fds), 0)
        return (fds[0], fds[1])
    }

    /// The receiver: reads Ns, answers Nr and its tag, then checks the sender's proof.
    private func receiver(_ fd: Int32, key: SymmetricKey, result: @escaping (Bool) -> Void) {
        Thread.detachNewThread {
            defer { close(fd) }
            guard let ns = SocketIO.readExact(fd, 16) else { return result(false) }
            let nr = Data((0..<16).map { _ in UInt8.random(in: 0...255) })
            let tag = Data(HMAC<SHA256>.authenticationCode(for: Data("rx".utf8) + ns + nr, using: key))
            guard SocketIO.writeAll(fd, nr + tag), let proof = SocketIO.readExact(fd, 32) else { return result(false) }
            result(HMAC<SHA256>.isValidAuthenticationCode(proof, authenticating: Data("tx".utf8) + nr + ns, using: key))
        }
    }

    func testBothSidesAcceptWhenKeysMatch() {
        let key = SymmetricKey(size: .bits256)
        let (a, b) = socketPair()
        let accepted = expectation(description: "receiver accepted the sender")
        receiver(b, key: key) { ok in
            XCTAssertTrue(ok)
            accepted.fulfill()
        }
        XCTAssertTrue(Handshake.perform(on: a, key: key))
        wait(for: [accepted], timeout: 5)
        close(a)
    }

    func testSenderRefusesReceiverWithDifferentKey() {
        let (a, b) = socketPair()
        let finished = expectation(description: "receiver finished")
        receiver(b, key: SymmetricKey(size: .bits256)) { ok in
            XCTAssertFalse(ok) // the sender hung up without sending its proof
            finished.fulfill()
        }
        XCTAssertFalse(Handshake.perform(on: a, key: SymmetricKey(size: .bits256)))
        close(a)
        wait(for: [finished], timeout: 5)
    }

    func testInputRecordIsLittleEndian() {
        // EV_REL (2), REL_Y (1), value -3
        let r = InputRecord([0x02, 0x00, 0x01, 0x00, 0xFD, 0xFF, 0xFF, 0xFF])
        XCTAssertEqual(r.type, 2)
        XCTAssertEqual(r.code, 1)
        XCTAssertEqual(r.value, -3)
    }
}
