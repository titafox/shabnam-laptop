//! Mutual authentication with a shared secret, run at the start of every connection.
//!
//! Without it, whoever reaches the port would receive the keyboard, and whoever answers at our
//! address would receive the sender's screen and could type into it. Both ends hold the same
//! 32-byte key (created once at pairing) and prove it to each other without sending it:
//!
//!   sender   -> 16 random bytes Ns
//!   receiver -> 16 random bytes Nr, HMAC-SHA256(key, "rx" | Ns | Nr)
//!   sender   -> HMAC-SHA256(key, "tx" | Nr | Ns)
//!
//! The link itself is not encrypted; it is a cable between the two machines.
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

fn tag(key: &[u8], who: &[u8], a: &[u8], b: &[u8]) -> Hmac<Sha256> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("any key length works");
    m.update(who);
    m.update(a);
    m.update(b);
    m
}

pub fn load_key(path: &str) -> Vec<u8> {
    let k = std::fs::read(path).unwrap_or_else(|e| panic!("no pairing key at {path}: {e}"));
    assert!(k.len() >= 32, "pairing key at {path} is too short");
    k
}

/// Receiver side of the handshake. True only if the peer proved it holds the key.
pub fn accept(sock: &mut TcpStream, key: &[u8]) -> bool {
    let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
    let ok = (|| -> std::io::Result<bool> {
        let mut ns = [0u8; 16];
        sock.read_exact(&mut ns)?;
        let mut nr = [0u8; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut nr)?;
        sock.write_all(&nr)?;
        sock.write_all(&tag(key, b"rx", &ns, &nr).finalize().into_bytes())?;
        let mut proof = [0u8; 32];
        sock.read_exact(&mut proof)?;
        Ok(tag(key, b"tx", &nr, &ns).verify_slice(&proof).is_ok()) // constant-time comparison
    })()
    .unwrap_or(false);
    let _ = sock.set_read_timeout(None);
    ok
}
