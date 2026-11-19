//! The laptop's speakers as the sender's sound output.
//! A second TCP port carries plain 48 kHz, 16-bit, stereo samples; they go to the sound chip
//! through ALSA. The chip has no volume control of its own, so volume is applied here and the
//! laptop's volume keys change it (see input.rs).
use std::io::Read;
use std::net::TcpListener;
use std::os::fd::AsRawFd;
use std::os::raw::{c_char, c_int, c_long, c_uint, c_void};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

pub const VOLUME_STEPS: i32 = 16;
pub static VOLUME: AtomicI32 = AtomicI32::new(9);
pub static MUTED: AtomicBool = AtomicBool::new(false);

#[link(name = "asound")]
extern "C" {
    fn snd_pcm_open(pcm: *mut *mut c_void, name: *const c_char, stream: c_int, mode: c_int) -> c_int;
    fn snd_pcm_set_params(pcm: *mut c_void, format: c_int, access: c_int, channels: c_uint, rate: c_uint, soft_resample: c_int, latency_us: c_uint) -> c_int;
    fn snd_pcm_writei(pcm: *mut c_void, buf: *const c_void, frames: usize) -> c_long;
    fn snd_pcm_recover(pcm: *mut c_void, err: c_int, silent: c_int) -> c_int;
    fn snd_pcm_close(pcm: *mut c_void) -> c_int;
}

/// The sound chip routes by a named "path": speakers, headphones, or off (amplifier powered down).
pub fn route(path: &str) {
    let _ = Command::new("amixer").args(["-q", "-c", "0", "cset", "name=Playback Path", path]).status();
}
fn headphones_in() -> bool {
    std::fs::read_to_string("/sys/class/extcon/extcon5/state").is_ok_and(|s| s.contains("HEADPHONE=1"))
}

/// Opens the sound device for the speakers or the headphones.
/// The order matters: the chip switches its amplifier on at the moment a stream is prepared,
/// according to the path selected at that moment. Selecting the path afterwards leaves the
/// amplifier off and the stream silent.
fn open_routed(headphones: bool) -> Option<*mut c_void> {
    route(if headphones { "HP" } else { "SPK" });
    let mut pcm = std::ptr::null_mut();
    if unsafe { snd_pcm_open(&mut pcm, b"plughw:0,0\0".as_ptr() as *const c_char, 0, 0) } != 0 {
        eprintln!("audio: cannot open the sound device");
        route("OFF");
        return None;
    }
    // 16-bit little endian (2), interleaved (3), stereo, 48 kHz, about 50 ms of buffer.
    if unsafe { snd_pcm_set_params(pcm, 2, 3, 2, 48000, 1, 50_000) } != 0 {
        eprintln!("audio: sound device refused 48 kHz stereo");
        unsafe { snd_pcm_close(pcm) };
        route("OFF");
        return None;
    }
    Some(pcm)
}

pub fn serve(listener: TcpListener, key: &[u8]) {
    for conn in listener.incoming() {
        let Ok(mut sock) = conn else { continue };
        if !crate::auth::accept(&mut sock, key) { continue; }
        let mut on_headphones = headphones_in();
        let Some(mut pcm) = open_routed(on_headphones) else { continue };
        eprintln!("audio: playing through {}", if on_headphones { "headphones" } else { "speakers" });

        let mut buf = [0u8; 1920]; // 10 ms
        let mut ticks = 0u32;
        while sock.read_exact(&mut buf).is_ok() {
            // The sender's clock and the sound chip's clock differ slightly. If sound piles up in
            // the network buffer the delay would grow without limit, so throw the excess away.
            let mut queued: c_int = 0;
            unsafe { libc::ioctl(sock.as_raw_fd(), libc::FIONREAD as _, &mut queued) };
            if queued > 1920 * 12 { continue; }

            let step = if MUTED.load(Ordering::Relaxed) { 0 } else { VOLUME.load(Ordering::Relaxed) };
            let gain = (step * step * 256) / (VOLUME_STEPS * VOLUME_STEPS); // squared: even-sounding steps
            for s in buf.chunks_exact_mut(2) {
                let v = (i16::from_le_bytes([s[0], s[1]]) as i32 * gain) >> 8;
                s.copy_from_slice(&(v as i16).to_le_bytes());
            }
            let r = unsafe { snd_pcm_writei(pcm, buf.as_ptr() as *const c_void, 480) };
            if r < 0 { unsafe { snd_pcm_recover(pcm, r as c_int, 1) }; }

            ticks += 1;
            if ticks % 50 == 0 && headphones_in() != on_headphones {
                on_headphones = !on_headphones;
                unsafe { snd_pcm_close(pcm) };
                match open_routed(on_headphones) { Some(p) => pcm = p, None => break }
            }
        }
        route("OFF");
        unsafe { snd_pcm_close(pcm) };
        eprintln!("audio: sender gone");
    }
}
