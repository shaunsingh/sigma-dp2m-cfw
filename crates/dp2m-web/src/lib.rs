//! Zero-dependency `wasm32-unknown-unknown` C-ABI bindings for `dp2m-emu::sim`.

use std::cell::RefCell;
use std::slice;

use dp2m_emu::sim::{Camera, Key};

thread_local! {
    static CAM: RefCell<Option<Camera>> = const { RefCell::new(None) };
}

#[unsafe(no_mangle)]
pub extern "C" fn wasm_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return core::ptr::null_mut();
    }
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    core::mem::forget(buf);
    ptr
}

/// # Safety
/// `ptr` and `cap` must match a prior `wasm_alloc(cap)` call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wasm_free(ptr: *mut u8, cap: usize) {
    if !ptr.is_null() && cap > 0 {
        drop(unsafe { Vec::from_raw_parts(ptr, 0, cap) });
    }
}

/// # Safety
/// `c78_ptr..+c78_len` and `firm_ptr..+firm_len` must be valid byte slices in linear memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cam_boot_split(
    c78_ptr: *const u8,
    c78_len: usize,
    firm_ptr: *const u8,
    firm_len: usize,
) -> u32 {
    if c78_ptr.is_null() || firm_ptr.is_null() {
        return 0;
    }
    let c78 = unsafe { slice::from_raw_parts(c78_ptr, c78_len) };
    let firm = unsafe { slice::from_raw_parts(firm_ptr, firm_len) };
    match Camera::new(c78, firm) {
        Ok(cam) => {
            CAM.with(|slot| *slot.borrow_mut() = Some(cam));
            1
        }
        Err(_) => 0,
    }
}

/// # Safety
/// `pkg_ptr..+pkg_len` must be a valid byte slice in linear memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cam_boot_pkg(pkg_ptr: *const u8, pkg_len: usize) -> u32 {
    if pkg_ptr.is_null() {
        return 0;
    }
    let pkg = unsafe { slice::from_raw_parts(pkg_ptr, pkg_len) };
    match Camera::from_package(pkg) {
        Ok(cam) => {
            CAM.with(|slot| *slot.borrow_mut() = Some(cam));
            1
        }
        Err(_) => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_press(code: u8) -> u32 {
    let Some(key) = Key::from_code(code) else { return 0 };
    CAM.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .map_or(0, |c| c.press(key).min(u32::MAX as u64) as u32)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_tick() -> u32 {
    CAM.with(|slot| slot.borrow_mut().as_mut().is_some_and(Camera::tick) as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_tilt(ax: i8, ay: i8) {
    CAM.with(|slot| {
        if let Some(c) = slot.borrow_mut().as_mut() {
            c.set_tilt(ax, ay);
        }
    });
}

/// # Safety
/// `rgba_ptr..+(w * h * 4)` must be a valid RGBA8888 buffer in linear memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cam_set_scene(rgba_ptr: *const u8, w: usize, h: usize) {
    let Some(len) = w.checked_mul(h).and_then(|px| px.checked_mul(4)) else {
        return;
    };
    if rgba_ptr.is_null() || len == 0 {
        return;
    }
    let rgba = unsafe { slice::from_raw_parts(rgba_ptr, len) };
    CAM.with(|slot| {
        if let Some(c) = slot.borrow_mut().as_mut() {
            c.set_scene_rgba(rgba, w, h);
        }
    });
}

/// # Safety
/// `ptr..+len` must be a valid byte slice in linear memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cam_set_sd_lut(lut_slot: usize, ptr: *const u8, len: usize) -> u32 {
    if ptr.is_null() || len == 0 {
        return 0;
    }
    let data = unsafe { slice::from_raw_parts(ptr, len) }.to_vec();
    CAM.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .is_some_and(|c| c.set_sd_lut(lut_slot, data)) as u32
    })
}

/// # Safety
/// `name_ptr..+name_len` and `data_ptr..+data_len` must be valid byte slices in linear memory.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cam_mount_sd_file(
    name_ptr: *const u8,
    name_len: usize,
    data_ptr: *const u8,
    data_len: usize,
) -> u32 {
    if name_ptr.is_null() || name_len == 0 || data_ptr.is_null() || data_len == 0 {
        return 0;
    }
    let Ok(name) = core::str::from_utf8(unsafe { slice::from_raw_parts(name_ptr, name_len) }) else {
        return 0;
    };
    let data = unsafe { slice::from_raw_parts(data_ptr, data_len) }.to_vec();
    CAM.with(|slot| {
        if let Some(c) = slot.borrow_mut().as_mut() {
            c.mount_sd_file(name, data);
            1
        } else {
            0
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_set_cm(cm: u32) {
    CAM.with(|slot| {
        if let Some(c) = slot.borrow_mut().as_mut() {
            c.set_color_mode(cm);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_set_grid(grid: u32) {
    CAM.with(|slot| {
        if let Some(c) = slot.borrow_mut().as_mut() {
            c.set_grid_mode(grid);
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_render() -> *const u8 {
    CAM.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .map_or(core::ptr::null(), |c| c.render_lcd().as_ptr())
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_screen_id() -> u32 {
    CAM.with(|slot| slot.borrow_mut().as_mut().map_or(0, Camera::screen_id))
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_color_mode() -> u32 {
    CAM.with(|slot| slot.borrow_mut().as_mut().map_or(0, Camera::color_mode))
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_expo_mode() -> u32 {
    CAM.with(|slot| slot.borrow_mut().as_mut().map_or(0, Camera::expo_mode))
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_grid_mode() -> u32 {
    CAM.with(|slot| slot.borrow_mut().as_mut().map_or(0, Camera::grid_mode))
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_in_playback() -> u32 {
    CAM.with(|slot| slot.borrow().as_ref().is_some_and(Camera::in_playback) as u32)
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_capture_count() -> u32 {
    CAM.with(|slot| slot.borrow().as_ref().map_or(0, |c| c.capture_count() as u32))
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_last_packed_cm() -> u32 {
    CAM.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|c| c.with_last_capture(|cap| cap.packed_cm as u32))
            .unwrap_or(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_last_p1d() -> u32 {
    CAM.with(|slot| {
        slot.borrow()
            .as_ref()
            .and_then(|c| c.with_last_capture(|cap| cap.p1d as u32))
            .unwrap_or(0)
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_r8(va: u32) -> u8 {
    CAM.with(|slot| slot.borrow().as_ref().map_or(0, |c| c.cpu().r8(va)))
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_r32(va: u32) -> u32 {
    CAM.with(|slot| slot.borrow().as_ref().map_or(0, |c| c.cpu().r32_be(va)))
}

#[unsafe(no_mangle)]
pub extern "C" fn cam_w8(va: u32, val: u8) {
    CAM.with(|slot| {
        if let Some(c) = slot.borrow_mut().as_mut() {
            c.write_u8(va, val);
        }
    });
}
