//! subscreen-rx: listens on TCP, feeds the incoming Annex-B H.264/H.265 byte stream to the
//! Rockchip hardware decoder and puts each decoded frame straight on a display plane.
//! No compositor, no copies: the decoder's buffer is the buffer the display scans out.
//! The laptop's keyboard and touchpad events travel back to the sender on the same connection.
//!
//! usage: subscreen-rx [--bind 169.254.1.1] [--iface usb0] [--port 7000] [--key /root/subscreen.key] [--hevc] [--connector 348] [--crtc 87] [--plane 229] [--primary 73]
mod audio;
mod auth;
mod drm;
mod input;
mod mpp;

use std::io::Read;
use std::net::TcpListener;
use std::os::raw::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
struct Dec { ctx: mpp::MppCtx, api: *mut mpp::MppApi }
unsafe impl Send for Dec {}

struct Frame(mpp::MppFrame);
unsafe impl Send for Frame {}
impl Drop for Frame {
    fn drop(&mut self) { unsafe { mpp::mpp_frame_deinit(&mut self.0); } }
}

fn arg(name: &str, default: u32) -> u32 {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn new_decoder(coding: u32) -> Dec {
    unsafe {
        let mut ctx = null_mut();
        let mut api = null_mut();
        assert_eq!(mpp::mpp_create(&mut ctx, &mut api), 0, "mpp_create");
        let ctl = (*api).control;
        // The sender's chunks do not line up with frames; let the decoder find the boundaries.
        let mut on: u32 = 1;
        ctl(ctx, mpp::DEC_SET_PARSER_SPLIT_MODE, &mut on as *mut u32 as *mut c_void);
        assert_eq!(mpp::mpp_init(ctx, mpp::MPP_CTX_DEC, coding), 0, "mpp_init");
        // Hand every frame out as soon as it is decoded instead of holding it for reordering.
        ctl(ctx, mpp::DEC_SET_IMMEDIATE_OUT, &mut on as *mut u32 as *mut c_void);
        ctl(ctx, mpp::DEC_SET_ENABLE_FAST_PLAY, &mut on as *mut u32 as *mut c_void);
        let mut wait_ms: i32 = 100;
        ctl(ctx, mpp::SET_OUTPUT_TIMEOUT, &mut wait_ms as *mut i32 as *mut c_void);
        Dec { ctx, api }
    }
}

fn main() {
    let port = arg("--port", 7000);
    let coding = if std::env::args().any(|a| a == "--hevc") { mpp::CODING_HEVC } else { mpp::CODING_AVC };
    let primary = arg("--primary", 73);
    let mut card = drm::Card::open("/dev/dri/card0", arg("--connector", 348), arg("--crtc", 87), arg("--plane", 229))
        .expect("open display");
    eprintln!("display {}x{}@{}", card.mode.hdisplay, card.mode.vdisplay, card.mode.vrefresh);

    // Listen only on the USB cable's address: anything that connects can draw on the screen and
    // feed the decoder, so the Wi-Fi side must not be able to reach this port.
    let args: Vec<String> = std::env::args().collect();
    let bind = args.iter().position(|x| x == "--bind").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| "169.254.1.1".into());
    // Binding to the address alone is not enough: Linux answers for any of its addresses on any
    // interface, so a machine on the Wi-Fi could still reach these ports by routing to them — and
    // whoever connects receives the keyboard. Tie the sockets to the USB network interface itself.
    let iface = args.iter().position(|x| x == "--iface").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| "usb0".into());
    let listen = |port: u16| {
        let l = TcpListener::bind((bind.as_str(), port)).expect("bind");
        let r = unsafe {
            use std::os::fd::AsRawFd;
            libc::setsockopt(l.as_raw_fd(), libc::SOL_SOCKET, libc::SO_BINDTODEVICE, iface.as_ptr() as *const c_void, iface.len() as u32)
        };
        assert_eq!(r, 0, "cannot restrict the listener to interface {iface}: {}", std::io::Error::last_os_error());
        l
    };
    let listener = listen(port as u16);
    let sound = listen(port as u16 + 1);
    let key_path = args.iter().position(|x| x == "--key").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| "/root/subscreen.key".into());
    let key = auth::load_key(&key_path);
    let sound_key = key.clone();
    std::thread::spawn(move || audio::serve(sound, &sound_key));
    eprintln!("listening on {bind}:{port} (video, input) and :{} (sound), {iface} only", port + 1);
    input::led("blue", false);
    let sink: input::Sink = Arc::new(Mutex::new(None));
    input::start(sink.clone());
    for conn in listener.incoming() {
        let Ok(mut sock) = conn else { continue };
        if !auth::accept(&mut sock, &key) {
            eprintln!("rejected a connection that does not hold the pairing key: {:?}", sock.peer_addr().ok());
            continue;
        }
        let _ = sock.set_nodelay(true);
        eprintln!("sender connected: {:?}", sock.peer_addr().ok());
        *sink.lock().unwrap() = sock.try_clone().ok();
        input::led("blue", true); // the blue light means "a computer is connected"
        let dec = new_decoder(coding);
        let alive = Arc::new(AtomicBool::new(true));
        let bytes = Arc::new(AtomicU64::new(0));

        // Thread 1: socket -> decoder.
        let (a1, b1) = (alive.clone(), bytes.clone());
        let feeder = std::thread::spawn(move || {
            let dec = dec;
            let mut buf = vec![0u8; 256 * 1024];
            while let Ok(n) = sock.read(&mut buf) {
                if n == 0 { break; }
                b1.fetch_add(n as u64, Ordering::Relaxed);
                unsafe {
                    let mut pkt = null_mut();
                    mpp::mpp_packet_init(&mut pkt, buf.as_mut_ptr() as *mut c_void, n);
                    while ((*dec.api).decode_put_packet)(dec.ctx, pkt) != 0 {
                        std::thread::sleep(Duration::from_micros(500)); // decoder input full
                    }
                    mpp::mpp_packet_deinit(&mut pkt);
                }
            }
            a1.store(false, Ordering::Relaxed);
        });

        // Thread 2: decoder -> "newest frame" slot. If the display falls behind, older frames
        // are dropped here rather than queued, so delay cannot build up.
        let slot: Arc<(Mutex<Option<Frame>>, Condvar)> = Arc::new((Mutex::new(None), Condvar::new()));
        let (a2, s2) = (alive.clone(), slot.clone());
        let dropped = Arc::new(AtomicU64::new(0));
        let d2 = dropped.clone();
        let getter = std::thread::spawn(move || {
            let dec = dec;
            let mut idle = 0;
            loop {
                let mut f = null_mut();
                let r = unsafe { ((*dec.api).decode_get_frame)(dec.ctx, &mut f) };
                if r != 0 || f.is_null() {
                    if !a2.load(Ordering::Relaxed) { idle += 1; if idle > 3 { break; } }
                    continue;
                }
                idle = 0;
                let frame = Frame(f);
                unsafe {
                    if mpp::mpp_frame_get_info_change(f) != 0 {
                        eprintln!("stream {}x{} (stride {}x{}) fmt {:#x}", mpp::mpp_frame_get_width(f), mpp::mpp_frame_get_height(f),
                            mpp::mpp_frame_get_hor_stride(f), mpp::mpp_frame_get_ver_stride(f), mpp::mpp_frame_get_fmt(f));
                        ((*dec.api).control)(dec.ctx, mpp::DEC_SET_INFO_CHANGE_READY, null_mut());
                        continue;
                    }
                    if mpp::mpp_frame_get_buffer(f).is_null() || mpp::mpp_frame_get_discard(f) != 0 || mpp::mpp_frame_get_errinfo(f) != 0 { continue; }
                }
                let (m, cv) = &*s2;
                if m.lock().unwrap().replace(frame).is_some() { d2.fetch_add(1, Ordering::Relaxed); }
                cv.notify_one();
            }
            s2.1.notify_one();
        });

        // This thread: newest frame -> screen. The frame on screen is kept until its successor is up.
        let mut on_screen: Option<Frame> = None;
        let (mut shown, mut t0, mut b0, mut first) = (0u64, Instant::now(), 0u64, true);
        loop {
            let next = {
                let (m, cv) = &*slot;
                let mut g = m.lock().unwrap();
                while g.is_none() && !getter.is_finished() {
                    g = cv.wait_timeout(g, Duration::from_millis(200)).unwrap().0;
                }
                g.take()
            };
            let Some(frame) = next else { break };
            let f = frame.0;
            let r = unsafe {
                let fd = mpp::mpp_buffer_get_fd_with_caller(mpp::mpp_frame_get_buffer(f), b"show\0".as_ptr());
                card.show_nv12(fd, mpp::mpp_frame_get_width(f), mpp::mpp_frame_get_height(f), mpp::mpp_frame_get_hor_stride(f), mpp::mpp_frame_get_ver_stride(f))
            };
            match r {
                Ok(()) => {
                    on_screen = Some(frame);
                    if first { first = false; if primary != 0 { if let Err(e) = card.disable_plane(primary) { eprintln!("primary off: {e}"); } } }
                    shown += 1;
                }
                Err(e) => { eprintln!("show: {e}"); }
            }
            if t0.elapsed() >= Duration::from_secs(2) {
                let b = bytes.load(Ordering::Relaxed);
                eprintln!("{:.1} fps shown, {} dropped, {:.1} Mbit/s", shown as f64 / t0.elapsed().as_secs_f64(), dropped.swap(0, Ordering::Relaxed),
                    (b - b0) as f64 * 8.0 / 1e6 / t0.elapsed().as_secs_f64());
                shown = 0; t0 = Instant::now(); b0 = b;
            }
        }
        *sink.lock().unwrap() = None;
        input::led("blue", false);
        let _ = feeder.join();
        let _ = getter.join();
        card.clear();
        drop(on_screen);
        unsafe { mpp::mpp_destroy(dec.ctx); }
        eprintln!("sender gone");
    }
}
