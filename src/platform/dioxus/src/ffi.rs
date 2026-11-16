//! Safe wrapper around the C shim in `shim/mgba_shim.c`.

use std::ffi::{c_char, c_int, c_uint, CString};
use std::path::Path;

#[repr(C)]
struct MShim {
    _private: [u8; 0],
}

extern "C" {
    fn mshim_open(rom: *const c_char, save: *const c_char) -> *mut MShim;
    fn mshim_close(s: *mut MShim);
    fn mshim_reset(s: *mut MShim);
    fn mshim_run_frame(s: *mut MShim);
    fn mshim_set_keys(s: *mut MShim, keys: u32);
    fn mshim_video_size(s: *mut MShim, w: *mut c_uint, h: *mut c_uint);
    fn mshim_copy_frame_rgba(s: *mut MShim, rgba: *mut u8);
    fn mshim_read_audio(s: *mut MShim, out: *mut i16, max_frames: usize) -> usize;
    fn mshim_sample_rate(s: *mut MShim) -> c_uint;
    fn mshim_fps(s: *mut MShim) -> f64;
    fn mshim_save_state(s: *mut MShim, path: *const c_char) -> bool;
    fn mshim_load_state(s: *mut MShim, path: *const c_char) -> bool;
    fn mshim_screenshot(s: *mut MShim, path: *const c_char) -> bool;
    fn mshim_platform(s: *mut MShim) -> c_int;
}

/// GBA key bits, as understood by `mCore::setKeys`.
pub mod keys {
    pub const A: u32 = 1 << 0;
    pub const B: u32 = 1 << 1;
    pub const SELECT: u32 = 1 << 2;
    pub const START: u32 = 1 << 3;
    pub const RIGHT: u32 = 1 << 4;
    pub const LEFT: u32 = 1 << 5;
    pub const UP: u32 = 1 << 6;
    pub const DOWN: u32 = 1 << 7;
    pub const R: u32 = 1 << 8;
    pub const L: u32 = 1 << 9;
}

/// An mGBA core with a ROM loaded. Not `Send`: it lives on the emulation thread.
pub struct Core {
    raw: *mut MShim,
}

fn cpath(p: &Path) -> CString {
    CString::new(p.to_string_lossy().as_bytes()).unwrap_or_default()
}

impl Core {
    pub fn open(rom: &Path, save: Option<&Path>) -> Option<Core> {
        let rom = cpath(rom);
        let save = save.map(cpath);
        let raw = unsafe {
            mshim_open(
                rom.as_ptr(),
                save.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()),
            )
        };
        (!raw.is_null()).then_some(Core { raw })
    }

    pub fn reset(&mut self) {
        unsafe { mshim_reset(self.raw) }
    }

    pub fn run_frame(&mut self) {
        unsafe { mshim_run_frame(self.raw) }
    }

    pub fn set_keys(&mut self, keys: u32) {
        unsafe { mshim_set_keys(self.raw, keys) }
    }

    pub fn video_size(&self) -> (u32, u32) {
        let (mut w, mut h) = (0, 0);
        unsafe { mshim_video_size(self.raw, &mut w, &mut h) };
        (w, h)
    }

    /// Copy the current frame as RGBA8888 into `buf` (resized as needed).
    pub fn frame_rgba(&self, buf: &mut Vec<u8>) -> (u32, u32) {
        let (w, h) = self.video_size();
        buf.resize((w * h * 4) as usize, 0);
        unsafe { mshim_copy_frame_rgba(self.raw, buf.as_mut_ptr()) };
        (w, h)
    }

    /// Drain interleaved stereo i16 samples into `out`; returns frames read.
    pub fn read_audio(&mut self, out: &mut [i16]) -> usize {
        unsafe { mshim_read_audio(self.raw, out.as_mut_ptr(), out.len() / 2) }
    }

    pub fn sample_rate(&self) -> u32 {
        unsafe { mshim_sample_rate(self.raw) }
    }

    pub fn fps(&self) -> f64 {
        unsafe { mshim_fps(self.raw) }
    }

    pub fn save_state(&mut self, path: &Path) -> bool {
        unsafe { mshim_save_state(self.raw, cpath(path).as_ptr()) }
    }

    pub fn load_state(&mut self, path: &Path) -> bool {
        unsafe { mshim_load_state(self.raw, cpath(path).as_ptr()) }
    }

    pub fn screenshot(&mut self, path: &Path) -> bool {
        unsafe { mshim_screenshot(self.raw, cpath(path).as_ptr()) }
    }

    #[allow(dead_code)]
    pub fn is_gba(&self) -> bool {
        unsafe { mshim_platform(self.raw) == 0 }
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        unsafe { mshim_close(self.raw) }
    }
}
