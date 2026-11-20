//! The control channel: a third authenticated connection (video port + 2) on which the sender
//! can set the backlight and the volume, mute, switch the screen off and on, ask for status,
//! switch between second-screen mode and the laptop's own desktop, and power the laptop off or
//! restart it, and on which it is told the laptop's state (see docs/PROTOCOL.md §6 and proto.rs).
//!
//! A sender that knows nothing of this never connects, and nothing changes.
//! Whatever arrives can only become one of the fixed actions in `apply`, with clamped numbers;
//! no part of a message reaches a shell, a path or a program's argument as text.
//!
//! The channel is served in both of the laptop's modes. What differs between them — whose
//! volume it is, where the lid state comes from, whether there is a mode to leave — is the
//! `Backend`: `SecondScreen` here, `desktop::Desktop` for `subscreen-rx control`.
use crate::{audio, input};
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use subscreen_rx::proto::{self, Command, End, Parser, Power, RateLimit, Status};

const BACKLIGHT: &str = "/sys/class/backlight/backlight";
const POWER_SUPPLY: &str = "/sys/class/power_supply";
const FRAMEBUFFER_BLANK: &str = "/sys/class/graphics/fb0/blank";
/// Where the session script reads what to do next once the program owning the screen has ended.
pub const MODE_FILE: &str = "/run/mode";
/// A sender stuck repeating itself cannot make the screen flicker or the laptop loop through
/// mode changes: what comes sooner than this after the last one is dropped.
static SCREEN_LIMIT: Mutex<RateLimit> = Mutex::new(RateLimit::new(Duration::from_millis(500)));
static MODE_LIMIT: Mutex<RateLimit> = Mutex::new(RateLimit::new(Duration::from_secs(5)));
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

fn screen_off() -> bool { read_num(&format!("{BACKLIGHT}/bl_power")).is_some_and(|v| v != 0) }

/// Backlight power only, as `shabnam-lid close` / `open` do it. Switching on is refused while
/// the lid is closed: the lid has the last word.
fn set_screen(on: bool, lid_closed: Option<bool>) {
    if on {
        if lid_closed == Some(true) {
            eprintln!("control: screen on asked for, but the lid is closed — ignored");
            return;
        }
        let _ = std::fs::write(FRAMEBUFFER_BLANK, "0");
    }
    eprintln!("control: screen {}", if on { "on" } else { "off" });
    let _ = std::fs::write(format!("{BACKLIGHT}/bl_power"), if on { "0" } else { "4" });
}

/// Leaves one of the session script's fixed words for it.
pub fn write_mode(word: &'static str) -> bool {
    std::fs::write(MODE_FILE, format!("{word}\n")).is_ok()
}

fn version() -> (u8, u8, u8) {
    let n = |s: &str| s.parse().unwrap_or(0);
    (n(env!("CARGO_PKG_VERSION_MAJOR")), n(env!("CARGO_PKG_VERSION_MINOR")), n(env!("CARGO_PKG_VERSION_PATCH")))
}

/// The part of the laptop's state that belongs to the mode it is in. Every method takes numbers
/// that `proto::decode` has already clamped, or nothing.
pub trait Backend: Sync {
    /// Volume step 0–`proto::VOLUME_STEPS`, and whether sound is muted.
    fn volume(&self) -> (u8, bool);
    fn set_volume(&self, step: u8);
    fn set_mute(&self, on: bool);
    fn leave(&self);
    /// From the laptop's own desktop to second-screen mode.
    fn become_second_screen(&self);
    /// Power off or restart: leave the word for the session script and end what owns the screen.
    fn end_session(&self, end: End);
    fn lid_closed(&self) -> Option<bool>;
    /// True where there is no second-screen mode to leave.
    fn own_desktop(&self) -> bool;
}

/// Second-screen mode: the volume is this program's own gain (audio.rs), the lid sensor is one of
/// the input devices it reads (input.rs), and leaving ends the program.
pub struct SecondScreen;

impl Backend for SecondScreen {
    fn volume(&self) -> (u8, bool) {
        (audio::VOLUME.load(Ordering::Relaxed).clamp(0, audio::VOLUME_STEPS) as u8, audio::MUTED.load(Ordering::Relaxed))
    }
    fn set_volume(&self, step: u8) { audio::VOLUME.store((step as i32).clamp(0, audio::VOLUME_STEPS), Ordering::Relaxed) }
    fn set_mute(&self, on: bool) { audio::MUTED.store(on, Ordering::Relaxed) }
    fn leave(&self) {
        eprintln!("control: the sender asked to leave second-screen mode");
        input::leave();
    }
    /// It already is one.
    fn become_second_screen(&self) {}
    fn end_session(&self, end: End) {
        // The session script looks at the word when this program has ended, before anything else.
        if write_mode(end.word()) { input::leave(); }
        eprintln!("control: could not leave word for the session script — nothing done");
    }
    fn lid_closed(&self) -> Option<bool> { input::lid_closed() }
    fn own_desktop(&self) -> bool { false }
}

fn status(backend: &dyn Backend, power: Power) -> Status {
    let (volume, muted) = backend.volume();
    Status {
        brightness: brightness(),
        volume,
        muted,
        power,
        lid_closed: backend.lid_closed(),
        own_desktop: backend.own_desktop(),
        screen_off: screen_off(),
        version: version(),
    }
}

/// The only things a message can cause.
fn apply(backend: &dyn Backend, command: Command) {
    match command {
        Command::SetBrightness(percent) => set_brightness(percent),
        Command::SetVolume(step) => backend.set_volume(step.min(proto::VOLUME_STEPS)),
        Command::SetMute(on) => backend.set_mute(on),
        Command::Leave => backend.leave(),
        Command::GetStatus => {}
        Command::SetScreen(on) => {
            if SCREEN_LIMIT.lock().unwrap().allow(Instant::now()) { set_screen(on, backend.lid_closed()); }
        }
        Command::BecomeSecondScreen => {
            if !backend.own_desktop() { return; } // not counted against the limit: nothing happens
            if MODE_LIMIT.lock().unwrap().allow(Instant::now()) {
                eprintln!("control: the sender asked the laptop to become its second screen");
                backend.become_second_screen();
            } else {
                eprintln!("control: mode change asked for again too soon — ignored");
            }
        }
        Command::EndSession(end) => {
            if MODE_LIMIT.lock().unwrap().allow(Instant::now()) {
                eprintln!("control: the sender asked for {}", match end { End::PowerOff => "power off", End::Restart => "restart" });
                backend.end_session(end);
            } else {
                eprintln!("control: power command asked for again too soon — ignored");
            }
        }
    }
}

/// Accepts control connections until `stale` says the listener is of no use any more (it is
/// asked every two seconds; second-screen mode never gives up its listener).
pub fn serve(listener: TcpListener, key: Vec<u8>, backend: &'static dyn Backend, stale: impl Fn() -> bool) {
    loop {
        let mut pfd = libc::pollfd { fd: listener.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ready = unsafe { libc::poll(&mut pfd, 1, 2000) };
        if stale() { break; }
        if ready <= 0 { continue; }
        let Ok((mut sock, _)) = listener.accept() else { continue };
        if !crate::auth::accept(&mut sock, &key) { continue; }
        let id = CURRENT.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::spawn(move || {
            eprintln!("control: sender connected");
            session(sock, id, backend);
            eprintln!("control: sender gone");
        });
    }
    CURRENT.fetch_add(1, Ordering::SeqCst); // whoever is still connected came in through this listener
}

fn session(mut sock: TcpStream, id: u64, backend: &dyn Backend) {
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
                // A burst (a slider being dragged) is cut down to its last word first.
                for c in proto::coalesce(&commands) {
                    wanted |= c == Command::GetStatus;
                    apply(backend, c);
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted) => {}
            Err(_) => return,
        }
        if sent.is_some_and(|(at, _)| at.elapsed() < STATUS_INTERVAL) { continue; }
        if power.0.elapsed() >= BATTERY_INTERVAL { power = (Instant::now(), proto::read_power(Path::new(POWER_SUPPLY))); }
        let now = status(backend, power.1);
        if wanted || sent.map_or(true, |(_, last)| last != now) {
            if sock.write_all(&now.encode()).is_err() { return; }
            sent = Some((Instant::now(), now));
            wanted = false;
        }
    }
}
