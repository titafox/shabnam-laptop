//! Minimal bindings to Rockchip MPP (librockchip_mpp), decoder side only.
//! Constants were printed from the 1.5.0 headers with a small C program.
#![allow(non_upper_case_globals, dead_code)]
use std::os::raw::{c_int, c_void};

pub type MppCtx = *mut c_void;
pub type MppPacket = *mut c_void;
pub type MppFrame = *mut c_void;
pub type MppBuffer = *mut c_void;
pub type Ret = c_int;

pub const MPP_CTX_DEC: u32 = 0x0;
pub const CODING_AVC: u32 = 0x7;
pub const CODING_HEVC: u32 = 0x1000004;
pub const CODING_MJPEG: u32 = 0x8;
pub const DEC_SET_PARSER_SPLIT_MODE: u32 = 0x310005;
pub const DEC_SET_INFO_CHANGE_READY: u32 = 0x310003;
pub const DEC_SET_IMMEDIATE_OUT: u32 = 0x31000c;
pub const DEC_SET_ENABLE_FAST_PLAY: u32 = 0x31000e;
pub const SET_OUTPUT_TIMEOUT: u32 = 0x200007;
pub const FMT_YUV420SP: u32 = 0x0;

/// `struct MppApi_t` from rk_mpi.h. Only the decoder entries are typed.
#[repr(C)]
pub struct MppApi {
    pub size: u32,
    pub version: u32,
    pub decode: *const c_void,
    pub decode_put_packet: unsafe extern "C" fn(MppCtx, MppPacket) -> Ret,
    pub decode_get_frame: unsafe extern "C" fn(MppCtx, *mut MppFrame) -> Ret,
    pub encode: *const c_void,
    pub encode_put_frame: *const c_void,
    pub encode_get_packet: *const c_void,
    pub isp: *const c_void,
    pub isp_put_frame: *const c_void,
    pub isp_get_frame: *const c_void,
    pub poll: *const c_void,
    pub dequeue: *const c_void,
    pub enqueue: *const c_void,
    pub reset: unsafe extern "C" fn(MppCtx) -> Ret,
    pub control: unsafe extern "C" fn(MppCtx, u32, *mut c_void) -> Ret,
    pub reserv: [u32; 16],
}

#[link(name = "rockchip_mpp")]
extern "C" {
    pub fn mpp_create(ctx: *mut MppCtx, mpi: *mut *mut MppApi) -> Ret;
    pub fn mpp_init(ctx: MppCtx, kind: u32, coding: u32) -> Ret;
    pub fn mpp_destroy(ctx: MppCtx) -> Ret;
    pub fn mpp_packet_init(packet: *mut MppPacket, data: *mut c_void, size: usize) -> Ret;
    pub fn mpp_packet_deinit(packet: *mut MppPacket) -> Ret;
    pub fn mpp_frame_deinit(frame: *mut MppFrame) -> Ret;
    pub fn mpp_frame_get_width(frame: MppFrame) -> u32;
    pub fn mpp_frame_get_height(frame: MppFrame) -> u32;
    pub fn mpp_frame_get_hor_stride(frame: MppFrame) -> u32;
    pub fn mpp_frame_get_ver_stride(frame: MppFrame) -> u32;
    pub fn mpp_frame_get_fmt(frame: MppFrame) -> u32;
    pub fn mpp_frame_get_info_change(frame: MppFrame) -> u32;
    pub fn mpp_frame_get_errinfo(frame: MppFrame) -> u32;
    pub fn mpp_frame_get_discard(frame: MppFrame) -> u32;
    pub fn mpp_frame_get_buffer(frame: MppFrame) -> MppBuffer;
    pub fn mpp_buffer_get_fd_with_caller(buffer: MppBuffer, caller: *const u8) -> c_int;
}
