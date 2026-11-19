//! The laptop's own controls.
//!
//! Keyboard, touchpad and media keys are forwarded to the computer whose screen we are showing:
//! each kernel input event becomes an 8-byte record (type u16, code u16, value i32, little endian)
//! written back down the video connection; a "sync" record (type 0) ends each batch.
//!
//! A few things are handled here on the laptop instead: the brightness keys drive the panel's
//! backlight, the volume keys set the speaker volume, closing the lid turns the backlight off,
//! Caps Lock lights its LED, and Win+Esc (or Ctrl+Alt+Esc) ends second-screen mode.
use crate::audio;
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub type Sink = Arc<Mutex<Option<TcpStream>>>;

const KEY_CAPSLOCK: u16 = 58;
const KEY_SLEEP: u16 = 142; // the lid sensor reports "sleep" on close
const KEY_WAKEUP: u16 = 143; // and "wake up" on open
const KEY_BRIGHTNESSDOWN: u16 = 224;
const KEY_BRIGHTNESSUP: u16 = 225;
const KEY_MUTE: u16 = 113;
const KEY_VOLUMEDOWN: u16 = 114;
const KEY_VOLUMEUP: u16 = 115;
const BACKLIGHT: &str = "/sys/class/backlight/backlight";

static CAPS: AtomicBool = AtomicBool::new(false);
static CTRL: AtomicBool = AtomicBool::new(false);
static ALT: AtomicBool = AtomicBool::new(false);
static META: AtomicBool = AtomicBool::new(false);

fn read_num(path: &str) -> i32 { fs::read_to_string(path).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0) }
fn write(path: &str, v: impl ToString) { let _ = fs::write(path, v.to_string()); }

/// Sets every LED whose name ends with `suffix` (LED names carry a device number that can change).
pub fn led(suffix: &str, on: bool) {
    let Ok(dir) = fs::read_dir("/sys/class/leds") else { return };
    for e in dir.flatten() {
        if e.file_name().to_string_lossy().ends_with(suffix) {
            write(&format!("{}/brightness", e.path().display()), if on { 255 } else { 0 });
        }
    }
}

fn brightness_step(up: bool) {
    let max = read_num(&format!("{BACKLIGHT}/max_brightness")).max(1);
    let step = (max / 12).max(1);
    let now = read_num(&format!("{BACKLIGHT}/brightness"));
    // Never step all the way to black: a dark screen with no way to see why is worse than a dim one.
    let next = if up { now + step } else { now - step }.clamp(step / 2 + 1, max);
    write(&format!("{BACKLIGHT}/brightness"), next);
}

/// True if the event was dealt with here and must not be forwarded.
fn handle_locally(code: u16, value: i32) -> bool {
    match code {
        29 | 97 => { CTRL.store(value != 0, Ordering::Relaxed); false }
        56 | 100 => { ALT.store(value != 0, Ordering::Relaxed); false }
        125 | 126 => { META.store(value != 0, Ordering::Relaxed); false }
        // Win+Esc leaves second-screen mode — the same key that enters it from the desktop.
        // macOS has no Command+Esc shortcut, so nothing is taken away from the Mac.
        // Ctrl+Alt+Esc does the same. The session script then brings the desktop back.
        1 if value == 1 && (META.load(Ordering::Relaxed) || (CTRL.load(Ordering::Relaxed) && ALT.load(Ordering::Relaxed))) => {
            led("blue", false);
            audio::route("OFF");
            std::process::exit(0);
        }
        KEY_BRIGHTNESSUP | KEY_BRIGHTNESSDOWN => { if value != 0 { brightness_step(code == KEY_BRIGHTNESSUP); } true }
        KEY_SLEEP => { if value == 1 { write(&format!("{BACKLIGHT}/bl_power"), 4); } true }
        KEY_WAKEUP => { if value == 1 { write(&format!("{BACKLIGHT}/bl_power"), 0); } true }
        // Sound comes out of this laptop's speakers, so its volume keys act here, not on the sender.
        KEY_VOLUMEUP | KEY_VOLUMEDOWN => {
            if value != 0 {
                let d = if code == KEY_VOLUMEUP { 1 } else { -1 };
                let _ = audio::VOLUME.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| Some((v + d).clamp(0, audio::VOLUME_STEPS)));
                audio::MUTED.store(false, Ordering::Relaxed);
            }
            true
        }
        KEY_MUTE => { if value == 1 { audio::MUTED.fetch_xor(true, Ordering::Relaxed); } true }
        KEY_CAPSLOCK => { if value == 1 { led("::capslock", !CAPS.fetch_xor(true, Ordering::Relaxed)); } false }
        _ => false,
    }
}

/// Whether the device can produce key `code`. The capability file holds 64-bit hex words, most
/// significant first.
fn has_key(event: &str, kind: &str, code: usize) -> bool {
    let Ok(s) = fs::read_to_string(format!("/sys/class/input/{event}/device/capabilities/{kind}")) else { return false };
    let words: Vec<u64> = s.split_whitespace().rev().filter_map(|w| u64::from_str_radix(w, 16).ok()).collect();
    words.get(code / 64).is_some_and(|w| w & (1 << (code % 64)) != 0)
}

/// Watches for keyboards, pointers, media keys and the lid sensor, including ones plugged into
/// the laptop's USB port later, and runs one reader thread per device until it is unplugged.
pub fn start(sink: Sink) {
    std::thread::spawn(move || {
        let open: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
        loop {
            if let Ok(dir) = fs::read_dir("/sys/class/input") {
                for entry in dir.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !name.starts_with("event") || open.lock().unwrap().contains(&name) { continue; }
                    let pointer = has_key(&name, "rel", 0) && has_key(&name, "rel", 1); // moves in X and Y
                    let keyboard = has_key(&name, "key", 30); // has the letter A
                    let media = has_key(&name, "key", KEY_VOLUMEUP as usize);
                    let lid = has_key(&name, "key", KEY_SLEEP as usize);
                    if !(pointer || keyboard || media || lid) { continue; }
                    let Ok(dev) = fs::File::open(format!("/dev/input/{name}")) else { continue };
                    eprintln!("input: forwarding {name}");
                    open.lock().unwrap().insert(name.clone());
                    let (sink, open) = (sink.clone(), open.clone());
                    std::thread::spawn(move || {
                        read_device(dev, &sink);
                        eprintln!("input: {name} removed");
                        open.lock().unwrap().remove(&name);
                    });
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}

fn read_device(mut dev: fs::File, sink: &Sink) {
    let mut ev = [0u8; 24]; // struct input_event on 64-bit: 16 bytes of time, then type, code, value
    let mut batch = Vec::with_capacity(64);
    while dev.read_exact(&mut ev).is_ok() {
        let kind = u16::from_le_bytes([ev[16], ev[17]]);
        if kind > 2 { continue; } // only sync, key/button, relative motion
        if kind == 1 && handle_locally(u16::from_le_bytes([ev[18], ev[19]]), i32::from_le_bytes([ev[20], ev[21], ev[22], ev[23]])) { continue; }
        batch.extend_from_slice(&ev[16..24]);
        if kind != 0 { continue; }
        if batch.len() > 8 {
            if let Some(s) = sink.lock().unwrap().as_mut() { let _ = s.write_all(&batch); }
        }
        batch.clear();
    }
}
