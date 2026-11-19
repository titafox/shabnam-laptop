//! shabnam-splash: puts one full-screen picture on the panel and holds it until killed.
//! Shown from the moment the machine hands over the screen until the desktop is ready.
//!
//! usage: shabnam-splash <picture.bgra> [connector=348] [crtc=87]
//! The picture is raw pixels, 4 bytes each (blue, green, red, unused), exactly the panel's size.
use std::io::Read;
use std::os::raw::{c_int, c_ulong, c_void};

const fn ioc(dir: c_ulong, nr: c_ulong, size: usize) -> c_ulong { (dir << 30) | ((size as c_ulong) << 16) | (0x64 << 8) | nr }
const RW: c_ulong = 3;

#[repr(C)] #[derive(Clone, Copy, Default)]
struct Mode { clock: u32, h: [u16; 5], v: [u16; 5], vrefresh: u32, flags: u32, kind: u32, name: [u8; 32] }
#[repr(C)] #[derive(Default)]
struct GetConnector { encoders: u64, modes: u64, props: u64, values: u64, n_modes: u32, n_props: u32, n_enc: u32, encoder: u32, id: u32, kind: u32, kind_id: u32, connection: u32, mm_w: u32, mm_h: u32, subpixel: u32, pad: u32 }
#[repr(C)] #[derive(Default)]
struct Crtc { connectors: u64, n: u32, id: u32, fb: u32, x: u32, y: u32, gamma: u32, mode_valid: u32, mode: Mode }
#[repr(C)] #[derive(Default)]
struct CreateDumb { height: u32, width: u32, bpp: u32, flags: u32, handle: u32, pitch: u32, size: u64 }
#[repr(C)] #[derive(Default)]
struct MapDumb { handle: u32, pad: u32, offset: u64 }
#[repr(C)] #[derive(Default)]
struct FbCmd2 { id: u32, width: u32, height: u32, format: u32, flags: u32, handles: [u32; 4], pitches: [u32; 4], offsets: [u32; 4], modifier: [u64; 4] }

fn call<T>(fd: c_int, nr: c_ulong, arg: &mut T, what: &str) {
    let r = unsafe { libc::ioctl(fd, ioc(RW, nr, std::mem::size_of::<T>()) as _, arg as *mut T as *mut c_void) };
    if r != 0 { eprintln!("splash: {what}: {}", std::io::Error::last_os_error()); std::process::exit(1); }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let Some(path) = a.get(1) else { eprintln!("usage: shabnam-splash <picture.bgra> [connector] [crtc]"); std::process::exit(2) };
    let connector: u32 = a.get(2).and_then(|v| v.parse().ok()).unwrap_or(348);
    let crtc: u32 = a.get(3).and_then(|v| v.parse().ok()).unwrap_or(87);
    let mut picture = Vec::new();
    std::fs::File::open(path).and_then(|mut f| f.read_to_end(&mut picture)).expect("read picture");

    let fd = unsafe { libc::open(b"/dev/dri/card0\0".as_ptr() as *const _, libc::O_RDWR | libc::O_CLOEXEC) };
    assert!(fd >= 0, "open display");
    let mut gc = GetConnector { id: connector, ..Default::default() };
    call(fd, 0xA7, &mut gc, "read connector");
    let mut modes = vec![Mode::default(); gc.n_modes.max(1) as usize];
    let mut gc = GetConnector { id: connector, n_modes: gc.n_modes, modes: modes.as_mut_ptr() as u64, ..Default::default() };
    call(fd, 0xA7, &mut gc, "read modes");
    let mode = modes[0];
    let (w, h) = (mode.h[0] as u32, mode.v[0] as u32);
    if picture.len() != (w * h * 4) as usize { eprintln!("splash: picture is not {w}x{h}"); std::process::exit(1); }

    let mut dumb = CreateDumb { width: w, height: h, bpp: 32, ..Default::default() };
    call(fd, 0xB2, &mut dumb, "create buffer");
    let mut map = MapDumb { handle: dumb.handle, ..Default::default() };
    call(fd, 0xB3, &mut map, "map buffer");
    let mem = unsafe { libc::mmap(std::ptr::null_mut(), dumb.size as usize, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, map.offset as libc::off_t) };
    assert!(mem != libc::MAP_FAILED, "mmap");
    for y in 0..h as usize {
        let row = &picture[y * w as usize * 4..(y + 1) * w as usize * 4];
        unsafe { std::ptr::copy_nonoverlapping(row.as_ptr(), (mem as *mut u8).add(y * dumb.pitch as usize), row.len()) };
    }
    let mut fb = FbCmd2 { width: w, height: h, format: u32::from_le_bytes(*b"XR24"), ..Default::default() };
    fb.handles[0] = dumb.handle;
    fb.pitches[0] = dumb.pitch;
    call(fd, 0xB8, &mut fb, "make framebuffer");
    let mut conns = [connector];
    let mut set = Crtc { connectors: conns.as_mut_ptr() as u64, n: 1, id: crtc, fb: fb.id, mode_valid: 1, mode, ..Default::default() };
    call(fd, 0xA2, &mut set, "show picture");
    loop { std::thread::sleep(std::time::Duration::from_secs(3600)); }
}
