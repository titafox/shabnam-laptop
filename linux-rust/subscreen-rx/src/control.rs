//! The control channel: a third authenticated connection (video port + 2) on which the sender
//! can set the backlight and the volume, mute, ask for status and end second-screen mode, and
//! on which it is told the laptop's state (see docs/PROTOCOL.md §6 and proto.rs).
//!
//! A sender that knows nothing of this never connects, and nothing changes.
//! Whatever arrives can only become one of the fixed actions in `apply`, with clamped numbers;
//! no part of a message reaches a shell, a path or a program argument.
use crate::{audio, input};
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use subscreen_rx::proto::{self, Command, Parser, Power, Status};

const BACKLIGHT: &str = "/sys/class/backlight/backlight";
const POWER_SUPPLY: &str = "/sys/class/power_supply";
/// State is looked at this often; a status message goes out at most once a second.
const TICK: Duration = Duration::from_millis(250);
const STATUS_INTERVAL: Duration = Duration::from_secs(1);
const BATTERY_INTERVAL: Duration = Duration::from_secs(5);

/// Bumped for every new connection: the newest one wins, so a connection whose other end
/// vanished without a word (cable pulled) cannot keep the next one out.
static CURRENT: AtomicU64 = AtomicU64::new(0);

fn read_num(path: &str) -> Option<i64> { std::fs::read_to_string(path).ok()?.trim().parse().ok() }

fn brightness() -> u8 {
    let max = read_num(&format!("{BACKLIGHT}/max_brightness")).unwrap_or(0);
    proto::brightness_percent(read_num(&format!("{BACKLIGHT}/brightness")).unwrap_or(0), max)
}

/// Writes the backlight level, as `shabnam-brightness set N` does. It does not switch a dark
/// panel back on: a closed lid stays dark.
fn set_brightness(percent: u8) {
    let Some(max) = read_num(&format!("{BACKLIGHT}/max_brightness")) else { return };
    let _ = std::fs::write(format!("{BACKLIGHT}/brightness"), proto::brightness_value(percent, max).to_string());
}

fn version() -> (u8, u8, u8) {
    let n = |s: &str| s.parse().unwrap_or(0);
    (n(env!("CARGO_PKG_VERSION_MAJOR")), n(env!("CARGO_PKG_VERSION_MINOR")), n(env!("CARGO_PKG_VERSION_PATCH")))
}

fn status(power: Power) -> Status {
    Status {
        brightness: brightness(),
        volume: audio::VOLUME.load(Ordering::Relaxed).clamp(0, audio::VOLUME_STEPS) as u8,
        muted: audio::MUTED.load(Ordering::Relaxed),
        power,
        lid_closed: input::lid_closed(),
        version: version(),
    }
}

/// The only things a message can cause.
fn apply(command: Command) {
    match command {
        Command::SetBrightness(percent) => set_brightness(percent),
        Command::SetVolume(step) => audio::VOLUME.store((step as i32).clamp(0, audio::VOLUME_STEPS), Ordering::Relaxed),
        Command::SetMute(on) => audio::MUTED.store(on, Ordering::Relaxed),
        Command::Leave => {
            eprintln!("control: the sender asked to leave second-screen mode");
            input::leave();
        }
        Command::GetStatus => {}
    }
}

pub fn serve(listener: TcpListener, key: Vec<u8>) {
    for conn in listener.incoming() {
        let Ok(mut sock) = conn else { continue };
        if !crate::auth::accept(&mut sock, &key) { continue; }
        let id = CURRENT.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::spawn(move || {
            eprintln!("control: sender connected");
            session(sock, id);
            eprintln!("control: sender gone");
        });
    }
}

fn session(mut sock: TcpStream, id: u64) {
    let _ = sock.set_nodelay(true);
    let _ = sock.set_read_timeout(Some(TICK));
    let _ = sock.set_write_timeout(Some(Duration::from_secs(2)));
    let mut parser = Parser::default();
    let mut buf = [0u8; 256];
    let mut sent: Option<(Instant, Status)> = None;
    let mut wanted = true; // the first status goes out unasked
    let mut power = (Instant::now(), proto::read_power(Path::new(POWER_SUPPLY)));
    while CURRENT.load(Ordering::SeqCst) == id {
        match sock.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => {
                let Ok(commands) = parser.feed(&buf[..n]) else {
                    eprintln!("control: oversized message — closing the connection");
                    return;
                };
                for c in commands {
                    wanted |= c == Command::GetStatus;
                    apply(c);
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
            Err(_) => return,
        }
        if sent.is_some_and(|(at, _)| at.elapsed() < STATUS_INTERVAL) { continue; }
        if power.0.elapsed() >= BATTERY_INTERVAL { power = (Instant::now(), proto::read_power(Path::new(POWER_SUPPLY))); }
        let now = status(power.1);
        if wanted || sent.map_or(true, |(_, last)| last != now) {
            if sock.write_all(&now.encode()).is_err() { return; }
            sent = Some((Instant::now(), now));
            wanted = false;
        }
    }
}
