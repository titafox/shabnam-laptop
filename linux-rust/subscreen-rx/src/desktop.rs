//! `subscreen-rx control`: the control channel alone, for the time the laptop is on its own
//! desktop. The session script starts it with the desktop and stops it before second-screen
//! mode, which serves the same port itself.
//!
//! Nothing but the control port is opened: no display, no decoder, no input device, no sound
//! device. Brightness and battery are the same sysfs files as in second-screen mode (control.rs).
//! Volume and mute are the desktop's own — the PipeWire output the volume keys and the quick
//! panel change — reached through `wpctl`, started directly (never through a shell) with fixed
//! arguments; the only varying one is a percentage printed from a clamped integer.
//!
//! Becoming the second screen, powering off and restarting go the way the desktop's own key
//! and buttons go: one of three fixed words is left in /run/mode and niri is asked to quit; the
//! session script then acts on the word (and stops this program).
use crate::control::{self, Backend};
use std::ffi::CString;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use subscreen_rx::proto;

const SINK: &str = "@DEFAULT_AUDIO_SINK@";
/// The kernel's listing of GPIO lines; the lid sensor is one of them (see device/shabnam-lid).
const GPIO_LISTING: &str = "/sys/kernel/debug/gpio";
/// The desktop's volume is asked for at most this often, and only while a sender is connected
/// (a status is put together at most once a second per connection).
const SINK_INTERVAL: Duration = Duration::from_millis(900);
const WPCTL_TIMEOUT: Duration = Duration::from_millis(1500);
const RETRY: Duration = Duration::from_secs(2);

struct Sink {
    read_at: Option<Instant>,
    step: u8,
    muted: bool,
}

static LAST: Mutex<Sink> = Mutex::new(Sink { read_at: None, step: 0, muted: false });

/// Runs `wpctl` with exactly these arguments and returns what it printed. None if it cannot be
/// started, fails, or does not finish in time (sound is not running).
fn wpctl(args: &[&str]) -> Option<String> {
    let mut command = Command::new("wpctl");
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    if std::env::var_os("XDG_RUNTIME_DIR").is_none() { command.env("XDG_RUNTIME_DIR", "/run/user0"); }
    let mut child = command.spawn().ok()?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < WPCTL_TIMEOUT => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let mut out = String::new();
    child.stdout.take()?.read_to_string(&mut out).ok()?; // a line at most: it fits in the pipe
    status.success().then_some(out)
}

/// The laptop on its own desktop.
pub struct Desktop;

impl Backend for Desktop {
    fn volume(&self) -> (u8, bool) {
        let mut last = LAST.lock().unwrap();
        if last.read_at.map_or(true, |at| at.elapsed() >= SINK_INTERVAL) {
            last.read_at = Some(Instant::now());
            // While sound is not running the last known values stay on show.
            if let Some((percent, muted)) = wpctl(&["get-volume", SINK]).as_deref().and_then(proto::parse_sink_volume) {
                (last.step, last.muted) = (proto::volume_step(percent), muted);
            }
        }
        (last.step, last.muted)
    }

    fn set_volume(&self, step: u8) {
        let step = step.min(proto::VOLUME_STEPS);
        let percent = format!("{}%", proto::volume_percent(step));
        let mut last = LAST.lock().unwrap();
        if wpctl(&["set-volume", "-l", "1.0", SINK, &percent]).is_some() {
            (last.step, last.read_at) = (step, Some(Instant::now()));
        }
    }

    fn set_mute(&self, on: bool) {
        let mut last = LAST.lock().unwrap();
        if wpctl(&["set-mute", SINK, if on { "1" } else { "0" }]).is_some() {
            (last.muted, last.read_at) = (on, Some(Instant::now()));
        }
    }

    /// There is no second-screen mode to leave.
    fn leave(&self) {}

    fn become_second_screen(&self) { end_desktop("mac") }

    fn end_session(&self, end: proto::End) { end_desktop(end.word()) }

    /// Read from the sensor's line, without opening the input device the desktop is using.
    fn lid_closed(&self) -> Option<bool> {
        proto::parse_lid(&std::fs::read_to_string(GPIO_LISTING).ok()?)
    }

    fn own_desktop(&self) -> bool { true }
}

/// What Win+Esc and the quick panel's power buttons do: leave the word, ask niri to quit.
/// Only from a running desktop; if niri cannot be asked, the word is taken back so that it
/// cannot take effect at some later, unrelated end of the desktop.
fn end_desktop(word: &'static str) {
    let mode = std::fs::read_to_string(control::MODE_FILE).unwrap_or_default();
    if mode.trim() != "desktop" {
        eprintln!("control: the laptop is not on its desktop (mode {:?}) — nothing done", mode.trim());
        return;
    }
    let Some(socket) = niri_socket() else {
        eprintln!("control: niri is not running — nothing done");
        return;
    };
    if !control::write_mode(word) { return; }
    // niri closes its socket as it quits, often before it has answered, so the command reports
    // a failure for a request that worked. What counts is whether niri is still there.
    let _ = Command::new("niri").args(["msg", "action", "quit", "--skip-confirmation"])
        .env("NIRI_SOCKET", &socket).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .status();
    std::thread::sleep(std::time::Duration::from_millis(300));
    if std::os::unix::net::UnixStream::connect(&socket).is_ok() {
        eprintln!("control: niri did not quit — staying on the desktop");
        control::write_mode("desktop");
    }
}

/// The running compositor's control socket: the newest niri.*.sock in the session's runtime
/// directory (this program is started before niri, so it cannot have it in its environment).
fn niri_socket() -> Option<std::path::PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| "/run/user0".into());
    std::fs::read_dir(dir).ok()?.flatten()
        .filter(|e| { let n = e.file_name(); let n = n.to_string_lossy(); n.starts_with("niri.") && n.ends_with(".sock") })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .max_by_key(|(at, _)| *at)
        .map(|(_, path)| path)
}

/// The kernel's number for the interface; 0 while it does not exist.
fn interface_index(name: &str) -> u32 {
    CString::new(name).map_or(0, |n| unsafe { libc::if_nametoindex(n.as_ptr()) })
}

/// Serves the control port for as long as the program lives.
///
/// The cable's network interface appears a few seconds after start-up and is created anew
/// whenever the USB link is re-bound; a listener tied to the old one would then wait for ever
/// without hearing anything. So: failing to listen is not an error, it is tried again; and a
/// listener whose interface has been replaced is dropped and set up afresh.
pub fn run(bind: &str, iface: &str, port: u16, key_path: &str) -> ! {
    let key = crate::auth::load_key(key_path);
    eprintln!("control only (the laptop is on its own desktop): {bind}:{port}, {iface} only");
    let mut explained = false;
    loop {
        let index = interface_index(iface);
        match crate::listen_on_cable(bind, iface, port) {
            Ok(listener) if index != 0 => {
                eprintln!("listening on {bind}:{port} (control), {iface} only");
                explained = false;
                control::serve(listener, key.clone(), &Desktop, || interface_index(iface) != index);
                eprintln!("the cable's network interface went away or was replaced — listening again");
            }
            Ok(_) => {} // the interface vanished in the same instant
            Err(e) => {
                if !explained { eprintln!("cannot listen on {bind}:{port} yet ({e}) — trying again every {} s", RETRY.as_secs()); }
                explained = true;
            }
        }
        std::thread::sleep(RETRY);
    }
}
