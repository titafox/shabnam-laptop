//! Just enough KMS, by raw ioctl, to light one CRTC and put a decoded NV12 buffer on a plane.
use std::collections::HashMap;
use std::io;
use std::os::raw::{c_int, c_ulong, c_void};

const fn ioc(dir: c_ulong, nr: c_ulong, size: usize) -> c_ulong {
    (dir << 30) | ((size as c_ulong) << 16) | (0x64 << 8) | nr
}
const W: c_ulong = 1;
const RW: c_ulong = 3;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct ModeInfo {
    pub clock: u32,
    pub hdisplay: u16, pub hsync_start: u16, pub hsync_end: u16, pub htotal: u16, pub hskew: u16,
    pub vdisplay: u16, pub vsync_start: u16, pub vsync_end: u16, pub vtotal: u16, pub vscan: u16,
    pub vrefresh: u32, pub flags: u32, pub kind: u32,
    pub name: [u8; 32],
}
#[repr(C)]
#[derive(Default)]
struct GetConnector {
    encoders_ptr: u64, modes_ptr: u64, props_ptr: u64, prop_values_ptr: u64,
    count_modes: u32, count_props: u32, count_encoders: u32,
    encoder_id: u32, connector_id: u32, connector_type: u32, connector_type_id: u32,
    connection: u32, mm_width: u32, mm_height: u32, subpixel: u32, pad: u32,
}
#[repr(C)]
#[derive(Default)]
struct Crtc {
    set_connectors_ptr: u64, count_connectors: u32,
    crtc_id: u32, fb_id: u32, x: u32, y: u32, gamma_size: u32, mode_valid: u32,
    mode: ModeInfo,
}
#[repr(C)]
#[derive(Default)]
struct CreateDumb { height: u32, width: u32, bpp: u32, flags: u32, handle: u32, pitch: u32, size: u64 }
#[repr(C)]
#[derive(Default)]
struct FbCmd2 {
    fb_id: u32, width: u32, height: u32, pixel_format: u32, flags: u32,
    handles: [u32; 4], pitches: [u32; 4], offsets: [u32; 4], modifier: [u64; 4],
}
#[repr(C)]
#[derive(Default)]
struct SetPlane {
    plane_id: u32, crtc_id: u32, fb_id: u32, flags: u32,
    crtc_x: i32, crtc_y: i32, crtc_w: u32, crtc_h: u32,
    src_x: u32, src_y: u32, src_h: u32, src_w: u32,
}
#[repr(C)]
#[derive(Default)]
struct PrimeHandle { handle: u32, flags: u32, fd: i32 }
#[repr(C)]
#[derive(Default)]
struct ClientCap { capability: u64, value: u64 }
#[repr(C)]
#[derive(Default)]
struct GemClose { handle: u32, pad: u32 }

pub const FOURCC_XRGB8888: u32 = u32::from_le_bytes(*b"XR24");
pub const FOURCC_NV12: u32 = u32::from_le_bytes(*b"NV12");

pub struct Card {
    fd: c_int,
    pub crtc: u32,
    pub plane: u32,
    pub mode: ModeInfo,
    /// decoder buffer fd -> (framebuffer id, gem handle)
    fbs: HashMap<i32, (u32, u32)>,
}

impl Card {
    fn call<T>(&self, req: c_ulong, arg: &mut T) -> io::Result<()> {
        loop {
            let r = unsafe { libc::ioctl(self.fd, req as _, arg as *mut T as *mut c_void) };
            if r == 0 { return Ok(()); }
            let e = io::Error::last_os_error();
            match e.raw_os_error() { Some(libc::EINTR) | Some(libc::EAGAIN) => continue, _ => return Err(e) }
        }
    }

    /// Opens the card, takes the connector's preferred (first) mode and shows a black screen.
    pub fn open(path: &str, connector: u32, crtc: u32, plane: u32) -> io::Result<Card> {
        let c = std::ffi::CString::new(path).unwrap();
        let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
        if fd < 0 { return Err(io::Error::last_os_error()); }
        let mut card = Card { fd, crtc, plane, mode: ModeInfo::default(), fbs: HashMap::new() };
        unsafe { libc::ioctl(fd, ioc(0, 0x1e, 0) as _) }; // SET_MASTER; fails harmlessly if we already are
        card.call(ioc(W, 0x0d, 16), &mut ClientCap { capability: 2, value: 1 })?; // universal planes

        // Two-step GETCONNECTOR: sizes first, then the mode list.
        let mut gc = GetConnector { connector_id: connector, ..Default::default() };
        card.call(ioc(RW, 0xA7, std::mem::size_of::<GetConnector>()), &mut gc)?;
        if gc.count_modes == 0 { return Err(io::Error::new(io::ErrorKind::Other, "connector has no modes")); }
        let mut modes = vec![ModeInfo::default(); gc.count_modes as usize];
        let mut gc2 = GetConnector { connector_id: connector, count_modes: gc.count_modes, modes_ptr: modes.as_mut_ptr() as u64, ..Default::default() };
        card.call(ioc(RW, 0xA7, std::mem::size_of::<GetConnector>()), &mut gc2)?;
        card.mode = modes[0];

        // Black XRGB background on the CRTC's primary plane; the video goes on an overlay above it.
        let (w, h) = (card.mode.hdisplay as u32, card.mode.vdisplay as u32);
        let mut dumb = CreateDumb { width: w, height: h, bpp: 32, ..Default::default() };
        card.call(ioc(RW, 0xB2, std::mem::size_of::<CreateDumb>()), &mut dumb)?;
        let mut fb = FbCmd2 { width: w, height: h, pixel_format: FOURCC_XRGB8888, ..Default::default() };
        fb.handles[0] = dumb.handle;
        fb.pitches[0] = dumb.pitch;
        card.call(ioc(RW, 0xB8, std::mem::size_of::<FbCmd2>()), &mut fb)?;
        let mut conns = [connector];
        let mut sc = Crtc { set_connectors_ptr: conns.as_mut_ptr() as u64, count_connectors: 1, crtc_id: crtc, fb_id: fb.fb_id, mode_valid: 1, mode: card.mode, ..Default::default() };
        card.call(ioc(RW, 0xA2, std::mem::size_of::<Crtc>()), &mut sc)?;
        Ok(card)
    }

    /// Shows a decoder buffer (NV12, given strides) scaled to the whole screen. Returns after the
    /// kernel has taken the update, which paces us to the display refresh.
    pub fn show_nv12(&mut self, dmabuf: i32, w: u32, h: u32, hstride: u32, vstride: u32) -> io::Result<()> {
        let fb_id = match self.fbs.get(&dmabuf) {
            Some(&(fb, _)) => fb,
            None => {
                let mut ph = PrimeHandle { fd: dmabuf, ..Default::default() };
                self.call(ioc(RW, 0x2e, std::mem::size_of::<PrimeHandle>()), &mut ph)?;
                let mut fb = FbCmd2 { width: w, height: h, pixel_format: FOURCC_NV12, ..Default::default() };
                fb.handles = [ph.handle, ph.handle, 0, 0];
                fb.pitches = [hstride, hstride, 0, 0];
                fb.offsets = [0, hstride * vstride, 0, 0];
                self.call(ioc(RW, 0xB8, std::mem::size_of::<FbCmd2>()), &mut fb)?;
                self.fbs.insert(dmabuf, (fb.fb_id, ph.handle));
                fb.fb_id
            }
        };
        let mut sp = SetPlane {
            plane_id: self.plane, crtc_id: self.crtc, fb_id,
            crtc_w: self.mode.hdisplay as u32, crtc_h: self.mode.vdisplay as u32,
            src_w: w << 16, src_h: h << 16, ..Default::default()
        };
        self.call(ioc(RW, 0xB7, std::mem::size_of::<SetPlane>()), &mut sp)
    }

    /// Turns a plane off. Used on the primary plane once video is up, so nothing can sit above it.
    pub fn disable_plane(&self, plane: u32) -> io::Result<()> {
        self.call(ioc(RW, 0xB7, std::mem::size_of::<SetPlane>()), &mut SetPlane { plane_id: plane, crtc_id: self.crtc, ..Default::default() })
    }

    /// Hides the video plane and forgets every imported buffer (stream ended or changed size).
    pub fn clear(&mut self) {
        let mut sp = SetPlane { plane_id: self.plane, crtc_id: self.crtc, ..Default::default() };
        let _ = self.call(ioc(RW, 0xB7, std::mem::size_of::<SetPlane>()), &mut sp);
        for (_, (fb, handle)) in std::mem::take(&mut self.fbs) {
            let mut id = fb;
            let _ = self.call(ioc(RW, 0xAF, 4), &mut id);
            let _ = self.call(ioc(W, 0x09, 8), &mut GemClose { handle, pad: 0 });
        }
    }
}
