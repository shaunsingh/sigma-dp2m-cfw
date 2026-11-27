//! Dual-CPU Sigma DP2 Merrill firmware simulator (`arm9` M6M + `fr80` M5Slave).
//!
//! The host drives selected `C78FRM.bin` (ARM9) and `Firm.bin` (FR80) routines
//! and supplies virtual peripherals, capture fixtures and playback buffers.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crate::arm9::{Cpu, SDRAM_BASE};
use crate::stock::{
    AE_AV_TO_Q16, AE_ENABLE_LV_METER, AE_MAP_ISO_IDX, AE_OUT_AV_IDX, AE_OUT_TV_IDX,
    AE_SET_AUTO_ISO_MAX, AE_SET_AUTO_ISO_MIN, AE_SET_ISO_STEP, AE_SET_MODE, AE_SOLVE_UI, AE_STATE,
    AE_SYNC_AV, AE_SYNC_EV, AE_SYNC_ISO, AE_SYNC_TV, AE_TV_TO_Q16, ARGB_FB, CLUT, DCF_DIR_COUNTS,
    FONT_SCALE, GET_AUTO_ISO_MAX_UI, GET_AUTO_ISO_MIN_UI, GET_CM, GET_COLOR_SPACE, GET_EV_IDX,
    GET_EXPO_MODE, GET_GRID, GET_ISO_IDX, IPC_CMD20, IPC_CMD20_END, LV_CONFIG, LV_MARK_CONTRAST_DIRTY,
    LV_REFRESH_DIRTY, LV_MARK_SHARP_DIRTY, LV_YCAP, LV_YCAP_END, MN_CM_END, MN_CM_SITE, NV_BLOCK,
    NV_CONTRAST, NV_CUR, NV_DEFAULTS, NV_DISPLAY_MODE, NV_EXPO, NV_FLUSH, NV_ISO_STEP, NV_LEN,
    NV_LOAD, NV_LUT_ENABLE, NV_LUT_SLOT, NV_SAT, NV_SHARP, NV_WB, OSD, PB_CM_PARSE_END,
    PB_CM_PARSE_START, PB_CUR_INDEX, PB_VBUF_DEFAULT_VA, PB_VBUFS, SET_CARD_READY, SET_CM, SET_GRID,
    UI_DISPATCH, UI_INIT_A, UI_INIT_B, UI_SCREEN_ID,
};
mod hooks;
mod isp;
mod m5;

use hooks::{clear_yuv420_buf, install_m6m_hooks};
use isp::*;
use m5::{M5_DELTA, M5_FLASH_LIMIT, M5_STD_LUT_VA};

pub const LCD_W: usize = 640;
pub const LCD_H: usize = 480;
pub const LV_H: usize = 424;

pub(crate) const PKG_MAGIC: &[u8] = b"SIGMA.CODP2M";
pub(crate) const PKG_S2_OFF: usize = 0x0082_0080;
pub(crate) const PKG_S2_LEN: usize = 0x0088_4080;
pub(crate) const PKG_C78_OFF: usize = 0x010A_4100;
pub(crate) const PKG_C78_LEN: usize = 0x0082_0000;
pub(crate) const C78_S3_OFF: usize = 0x0002_0000;
pub(crate) const S3_LEN: usize = 0x0080_0000;

pub(crate) const S3_BSS_START: usize = 0x007F_BC1C;
pub(crate) const S3_BSS_END: usize = 0x00A1_F398;

pub(crate) const STOCK_LV_LUT1024_OFF: usize = 0x0026_8A14;

// LCD planes in M6M SDRAM
pub(crate) const OSD_OFF: usize = (OSD - SDRAM_BASE) as usize;
pub(crate) const ARGB_OFF: usize = (ARGB_FB - SDRAM_BASE) as usize;
pub(crate) const CLUT_OFF: usize = (CLUT - SDRAM_BASE) as usize;
pub(crate) const LV_Y_VA: u32 = 0xA2A0_0000;
pub(crate) const LV_CB_VA: u32 = 0xA2A5_0000;
pub(crate) const LV_CR_VA: u32 = 0xA2A8_0000;
pub(crate) const LV_Y_OFF: usize = (LV_Y_VA - SDRAM_BASE) as usize;
pub(crate) const LV_CB_OFF: usize = (LV_CB_VA - SDRAM_BASE) as usize;
pub(crate) const LV_CR_OFF: usize = (LV_CR_VA - SDRAM_BASE) as usize;
pub(crate) const PB_PLANE_Y_LEN: usize = LCD_W * LCD_H;
pub(crate) const PB_PLANE_C_LEN: usize = (LCD_W / 2) * (LCD_H / 2);
pub(crate) const CALL_SP: u32 = 0xA200_1000;
pub(crate) const SCRATCH_VA: u32 = 0xA2B0_0000;
const HEAP_LO: std::ops::Range<u32> = 0xA0A2_0000..0xA1C0_0000;
const HEAP_HI: std::ops::Range<u32> = 0xA300_0000..0xA7F0_0000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Key {
    Ok = 0x01,
    Up = 0x02,
    Down = 0x03,
    Left = 0x04,
    Right = 0x05,
    Menu = 0x06,
    Disp = 0x07,
    Ael = 0x08,
    Qs = 0x09,
    Mode = 0x0A,
    Play = 0x0C,
    DialLeft = 0x20,
    DialRight = 0x40,
    Shutter = 0xFE,
}

impl Key {
    pub fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0x01 => Self::Ok,
            0x02 => Self::Up,
            0x03 => Self::Down,
            0x04 => Self::Left,
            0x05 => Self::Right,
            0x06 => Self::Menu,
            0x07 => Self::Disp,
            0x08 => Self::Ael,
            0x09 => Self::Qs,
            0x0A => Self::Mode,
            0x0C => Self::Play,
            0x20 => Self::DialLeft,
            0x40 => Self::DialRight,
            0xFE => Self::Shutter,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug)]
pub struct CaptureFrame {
    pub cm: u8,
    pub decoded_cm: u8,
    pub packed_cm: u8,
    pub p1d: u8,
    pub tag3d: String,
    pub tag59: String,
    pub iso: u16,
    pub tv_num: u32,
    pub tv_den: u32,
    pub av_num: u32,
    pub av_den: u32,
    pub ev_steps: i32,
    pub expo_prog: u8,
    pub wb: u8,
    pub cs: u8,
    pub sharp: u8,
    pub contrast: u8,
    pub sat: u8,
    pub m5_insns: u64,
    pub rgba: Vec<u8>, // 640 * 424 * 4 RGBA8888
}

pub(crate) struct HeapBlock {
    pub(crate) va: u32,
    pub(crate) cap: u32,
}

pub(crate) struct HwState {
    pub(crate) events: VecDeque<(u32, u32)>,
    pub(crate) nor: [u8; 0x800],
    pub(crate) heap_bump: u32,
    pub(crate) free_list: Vec<HeapBlock>,
    pub(crate) alloc_caps: Vec<(u32, u32)>,
    pub(crate) b2y_mat: [i32; 9],
    pub(crate) b2y_gamma: [u8; 1024],
    pub(crate) b2y_mono_en: bool,
    pub(crate) b2y_mono_cb: i8,
    pub(crate) b2y_mono_cr: i8,
    pub(crate) b2y_chroma_gain: u8,
    pub(crate) accel_x: i8,
    pub(crate) accel_y: i8,
    pub(crate) lv_active: bool,
    pub(crate) scene_ev100_q16: i32,
    pub(crate) captures: Vec<CaptureFrame>,
    pub(crate) dcf_cur: usize,
    pub(crate) pb_open_idx: Option<usize>,
    pub(crate) pb_vbuf_va: u32,
}

impl HwState {
    fn new(b2y_gamma: [u8; 1024]) -> Self {
        Self {
            events: VecDeque::with_capacity(16),
            nor: [0u8; 0x800],
            heap_bump: HEAP_LO.start,
            free_list: Vec::new(),
            alloc_caps: Vec::new(),
            b2y_mat: [128, 0, 0, 0, 128, 0, 0, 0, 128],
            b2y_gamma,
            b2y_mono_en: false,
            b2y_mono_cb: 0,
            b2y_mono_cr: 0,
            b2y_chroma_gain: 200,
            accel_x: 0,
            accel_y: 64,
            lv_active: true,
            scene_ev100_q16: 9 * 65536,
            captures: Vec::new(),
            dcf_cur: 0,
            pb_open_idx: None,
            pb_vbuf_va: PB_VBUF_DEFAULT_VA,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ExposureParams {
    pub(crate) gain_q8: i32, // 256 == 1.0x linear sensor light
    pub(crate) noise_amp: i32,
    pub(crate) av_idx: u16,
    pub(crate) tv_idx: u16,
    pub(crate) iso_speed: u16,
    pub(crate) ev_steps: i32,
    pub(crate) expo_prog: u8,
}

pub struct Camera {
    pub(crate) cpu: Cpu,
    pub(crate) hw: Rc<RefCell<HwState>>,
    pub(crate) m5_flash: Vec<u8>,
    sd_luts: [Option<Rc<Vec<u8>>>; 8],
    pub(crate) lv_inv_lut: [u16; 256],
    pub(crate) m5_inv_lut: [u16; 256],
    pub(crate) scene_srgb: Vec<[u8; 3]>, // 640 * 424
    pub(crate) ref_scene_luma: f32,
    pub(crate) scene_bv_ev: f32,
    pub(crate) last_screen_id: u32,
    pub(crate) pb_menu_parent: bool,
    pub(crate) lcd_rgba: Vec<u8>, // 640 * 480 * 4
    pub(crate) lcd_dirty: bool,
    luma_src: Vec<u8>,
}

impl Camera {
    pub fn from_package(pkg: &[u8]) -> Result<Self, &'static str> {
        if pkg.len() < PKG_C78_OFF + PKG_C78_LEN || !pkg.starts_with(PKG_MAGIC) {
            return Err("not a valid DP2MVxxx.bin firmware package");
        }
        let firm = &pkg[PKG_S2_OFF..PKG_S2_OFF + PKG_S2_LEN];
        let c78 = &pkg[PKG_C78_OFF..PKG_C78_OFF + PKG_C78_LEN];
        Self::new(c78, firm)
    }

    pub fn new(c78: &[u8], firm: &[u8]) -> Result<Self, &'static str> {
        if c78.len() < C78_S3_OFF + S3_LEN {
            return Err("C78FRM.bin truncated");
        }
        if firm.len() < M5_FLASH_LIMIT {
            return Err("Firm.bin truncated");
        }
        let s3 = &c78[C78_S3_OFF..C78_S3_OFF + S3_LEN];

        let lv_inv_lut = invert_monotonic_lut(&s3[STOCK_LV_LUT1024_OFF..STOCK_LV_LUT1024_OFF + 1024], 1023);
        let m5_std_off = (M5_STD_LUT_VA - M5_DELTA) as usize;
        let m5_inv_lut = invert_monotonic_lut(&firm[m5_std_off..m5_std_off + 4096], 4095);

        let mut b2y_gamma = [0u8; 1024];
        b2y_gamma.copy_from_slice(&s3[STOCK_LV_LUT1024_OFF..STOCK_LV_LUT1024_OFF + 1024]);

        let hw = Rc::new(RefCell::new(HwState::new(b2y_gamma)));

        let mut cpu = Cpu::new();
        cpu.sdram_mut()[..S3_LEN].copy_from_slice(s3);
        cpu.sdram_mut()[S3_BSS_START..S3_BSS_END].fill(0);
        cpu.w32(FONT_SCALE, 1);
        cpu.w32(FONT_SCALE + 4, 1);

        for &vbuf_va in &PB_VBUFS {
            clear_yuv420_buf(cpu.sdram_mut(), vbuf_va);
        }

        install_m6m_hooks(&mut cpu, &hw);

        let nv_def_off = (NV_DEFAULTS - SDRAM_BASE) as usize;
        cpu.write_bytes(NV_BLOCK, &s3[nv_def_off..nv_def_off + NV_LEN]);

        let scene_srgb = default_scene_srgb();
        let ref_scene_luma = mean_scene_luma(&scene_srgb);

        let mut cam = Self {
            cpu,
            hw,
            m5_flash: firm[..M5_FLASH_LIMIT].to_vec(),
            sd_luts: Default::default(),
            lv_inv_lut,
            m5_inv_lut,
            scene_srgb,
            ref_scene_luma,
            scene_bv_ev: 9.0,
            last_screen_id: 0x15,
            pb_menu_parent: false,
            lcd_rgba: vec![0u8; LCD_W * LCD_H * 4],
            lcd_dirty: true,
            luma_src: Vec::new(),
        };

        cam.boot_m6m();
        Ok(cam)
    }

    fn boot_m6m(&mut self) {
        // Default NV_DISP_LV = 4 (status bar + icons + grid, without unpopulated HW luma histogram)
        self.cpu.w8(NV_DISPLAY_MODE, 4);
        self.call_thumb(NV_FLUSH, &[]);
        self.call_thumb(NV_LOAD, &[NV_BLOCK]);
        self.call_thumb(UI_INIT_A, &[]);
        self.call_thumb(UI_INIT_B, &[]);
        // Enable Live View AE metering path (0xA0002CB2(1, 1)) and mark virtual SD card ready
        self.call_thumb(AE_ENABLE_LV_METER, &[1, 1]);
        self.call_thumb(SET_CARD_READY, &[2]);

        self.refresh_lv_isp_and_ycap();
        {
            let mut st = self.hw.borrow_mut();
            st.events.push_back((1, 0x15)); // open SCREEN_LV
            st.events.push_back((4, 0));    // LV enter
            st.events.push_back((4, 3));    // LV periodic widget refresh
        }
        self.drain_events();
        self.lcd_dirty = true;
    }

    fn call_thumb(&mut self, va: u32, args: &[u32]) -> u32 {
        self.cpu.call(va | 1, args, &[], CALL_SP, 25_000_000)
            .map_or(0, |(r0, _)| r0)
    }

    fn drain_events(&mut self) -> u64 {
        let mut total = 0u64;
        let mut guard = 0usize;
        while guard < 64 {
            let ev = self.hw.borrow_mut().events.pop_front();
            let Some((ty, val)) = ev else { break };
            guard += 1;
            if let Ok((_, n)) = self.cpu.call(UI_DISPATCH | 1, &[ty, val], &[], CALL_SP, 250_000_000) {
                total += n;
            }
        }
        let scr = self.call_thumb(UI_SCREEN_ID, &[]);
        if scr == 0x19 {
            self.pb_menu_parent = true;
        } else if scr != 0x1B {
            self.pb_menu_parent = false;
        }
        self.last_screen_id = scr;
        self.hw.borrow_mut().lv_active = scr != 0x19 && !(scr == 0x1B && self.pb_menu_parent);
        total
    }

    // Supply scene EV through the 0xA000273C hook and invoke the AE solver at 0xA003EABE.
    fn sync_ae_and_solve(&mut self) -> (ExposureParams, u64) {
        let mut insns = 0u64;
        let scene_ev100_q16 = (self.scene_bv_ev * 65536.0).round() as i32;
        self.hw.borrow_mut().scene_ev100_q16 = scene_ev100_q16;

        // Map UI expo_mode (`0xA09396AC`) -> active AE program (`0xA09396C0`: 1=M, 2=P, 3=A, 4=S)
        // and AE_STATE[0] (`1=M, 2=A, 3=S, 4=P`)
        let ui_mode = self.cpu.r8(NV_EXPO);
        let prog = match ui_mode {
            0 => 1u8, // M
            1 => 4u8, // S
            2 => 3u8, // A
            3 => 2u8, // P
            _ => self.cpu.r8(NV_CUR).clamp(1, 4),
        };
        self.cpu.w8(NV_CUR, prog);
        let ae_mode = match prog {
            1 => 1u32, // M
            2 => 4u32, // P (program line 0xA000E218)
            3 => 2u32, // A (user Av, solve Tv)
            _ => 3u32, // S (user Tv, solve Av)
        };

        let iso_ext = self.cpu.r8(NV_ISO_STEP) as u32;
        let min_iso_ui = self.call_thumb(GET_AUTO_ISO_MIN_UI, &[]);
        let max_iso_ui = self.call_thumb(GET_AUTO_ISO_MAX_UI, &[]);
        let min_iso_ae = self.call_thumb(AE_MAP_ISO_IDX, &[min_iso_ui]);
        let max_iso_ae = self.call_thumb(AE_MAP_ISO_IDX, &[max_iso_ui]);
        let iso_idx = self.call_thumb(GET_ISO_IDX, &[]);
        let ev_idx = self.call_thumb(GET_EV_IDX, &[]);
        for (fn_va, arg) in [
            (AE_SET_MODE, Some(ae_mode)),
            (AE_SET_ISO_STEP, Some(iso_ext)),
            (AE_SET_AUTO_ISO_MIN, Some(min_iso_ae)),
            (AE_SET_AUTO_ISO_MAX, Some(max_iso_ae)),
            (AE_SYNC_ISO, Some(iso_idx)),
            (AE_SYNC_AV, None),
            (AE_SYNC_TV, None),
            (AE_SYNC_EV, Some(ev_idx)),
            (AE_SOLVE_UI, None),
        ] {
            if let Ok((_, n)) = self.cpu.call(fn_va | 1, arg.as_slice(), &[], CALL_SP, 1_000_000) {
                insns += n;
            }
        }

        let av_idx = self.cpu.r16_be(AE_OUT_AV_IDX).min(15);
        let tv_idx = self.cpu.r16_be(AE_OUT_TV_IDX).min(48);
        let sv_q16 = self.cpu.r32_be(AE_STATE + 0x10) as i32;
        let av_q16 = self.call_thumb(AE_AV_TO_Q16, &[av_idx as u32]) as i32;
        let tv_q16 = self.call_thumb(AE_TV_TO_Q16, &[tv_idx as u32]) as i32;

        // M6M 0xA000D6B6 uncompensated APEX law: EV_scene = (scene_ev100_q16 - 5.0_Q16) + SV_Q16
        let uncomp_ev_q16 = (scene_ev100_q16 - 0x0005_0000).saturating_add(sv_q16);
        let actual_ev_q16 = av_q16.saturating_add(tv_q16);
        let delta_ev = ((uncomp_ev_q16 - actual_ev_q16) as f32 / 65536.0).clamp(-6.0, 6.0);
        let gain_q8 = if delta_ev.abs() < 0.06 {
            256
        } else {
            (256.0 * delta_ev.exp2()).round().clamp(4.0, 16384.0) as i32
        };
        let sv = (sv_q16 as f32) / 65536.0;
        let iso_speed = sv_to_iso_speed(sv);
        let noise_amp = if sv > 6.4 {
            (((sv - 6.0) * 0.65).exp2() * 5.0).round().clamp(0.0, 96.0) as i32
        } else {
            0
        };

        let params = ExposureParams {
            gain_q8,
            noise_amp,
            av_idx,
            tv_idx,
            iso_speed,
            ev_steps: (ev_idx as i32) - 9,
            expo_prog: prog,
        };
        (params, insns)
    }

    pub fn press(&mut self, key: Key) -> u64 {
        let prev_scr = self.screen_id();
        if key == Key::Shutter && prev_scr != 0x21 {
            if prev_scr == 0x19 || (prev_scr == 0x1B && self.pb_menu_parent) {
                let mut insns = 0u64;
                if prev_scr == 0x1B {
                    insns += self.press(Key::Menu);
                }
                insns += self.press(Key::Play);
                return insns;
            }
            return self.shoot();
        }

        if key == Key::Play && prev_scr == 0x15 && self.hw.borrow().captures.is_empty() {
            let vbuf = self.hw.borrow().pb_vbuf_va;
            clear_yuv420_buf(self.cpu.sdram_mut(), vbuf);
        }

        self.hw.borrow_mut().events.push_back((5, key as u32));
        let mut insns = self.drain_events();
        let new_scr = self.last_screen_id;
        self.lcd_dirty = true;
        if new_scr == 0x15 {
            insns += self.refresh_lv_isp_and_ycap();
            {
                let mut st = self.hw.borrow_mut();
                if prev_scr != 0x15 {
                    st.events.push_back((4, 0));
                }
                st.events.push_back((4, 3));
            }
            insns += self.drain_events();
        } else if matches!(new_scr, 0x16 | 0x17 | 0x1A) {
            insns += self.refresh_lv_isp_and_ycap();
        }
        insns
    }

    /// Advances animated HUD overlays (such as diagonal zebra phase) or active applet screens by one frame.
    pub fn tick(&mut self) -> bool {
        if self.last_screen_id == 0x21 {
            self.hw.borrow_mut().events.push_back((4, 3));
            self.drain_events();
            self.lcd_dirty = true;
            return true;
        }
        if self.last_screen_id != 0x15 {
            return false;
        }
        let grid = self.grid_mode();
        if grid == 13 {
            // Advance by 2 frames so grid_widget_get's (fcnt >> 1) dirty key triggers redraw
            self.step_lv_ycap();
            self.step_lv_ycap();
            self.hw.borrow_mut().events.push_back((4, 3));
            self.drain_events();
            self.lcd_dirty = true;
            return true;
        }
        false
    }

    pub fn set_tilt(&mut self, ax: i8, ay: i8) {
        {
            let mut st = self.hw.borrow_mut();
            if st.accel_x == ax && st.accel_y == ay {
                return;
            }
            st.accel_x = ax;
            st.accel_y = ay;
        }
        if self.last_screen_id == 0x15 {
            self.hw.borrow_mut().events.push_back((4, 3));
            self.drain_events();
            self.lcd_dirty = true;
        }
    }

    pub fn write_u8(&mut self, va: u32, val: u8) {
        self.cpu.w8(va, val);
        if matches!(self.last_screen_id, 0x15..=0x17 | 0x1A) {
            self.refresh_lv_pipeline();
        } else {
            self.lcd_dirty = true;
        }
    }

    pub fn set_scene_rgba(&mut self, rgba: &[u8], w: usize, h: usize) {
        let Some(min_len) = w.checked_mul(h).and_then(|px| px.checked_mul(4)) else {
            return;
        };
        if w == 0 || h == 0 || rgba.len() < min_len {
            return;
        }
        let dst_aspect = (LCD_W as f32) / (LV_H as f32);
        let src_aspect = (w as f32) / (h as f32);
        let (crop_w, crop_h, x0, y0) = if src_aspect > dst_aspect {
            let cw = (h as f32) * dst_aspect;
            (cw, h as f32, ((w as f32) - cw) * 0.5, 0.0)
        } else {
            let ch = (w as f32) / dst_aspect;
            (w as f32, ch, 0.0, ((h as f32) - ch) * 0.5)
        };

        for dy in 0..LV_H {
            let sy = y0 + ((dy as f32) + 0.5) * (crop_h / (LV_H as f32)) - 0.5;
            let iy = (sy.floor() as isize).clamp(0, (h - 1) as isize) as usize;
            let iy1 = (iy + 1).min(h - 1);
            let fy = ((sy - sy.floor()) * 256.0) as u32;
            for dx in 0..LCD_W {
                let sx = x0 + ((dx as f32) + 0.5) * (crop_w / (LCD_W as f32)) - 0.5;
                let ix = (sx.floor() as isize).clamp(0, (w - 1) as isize) as usize;
                let ix1 = (ix + 1).min(w - 1);
                let fx = ((sx - sx.floor()) * 256.0) as u32;

                let p00 = (iy * w + ix) * 4;
                let p10 = (iy * w + ix1) * 4;
                let p01 = (iy1 * w + ix) * 4;
                let p11 = (iy1 * w + ix1) * 4;

                let mut rgb = [0u8; 3];
                for c in 0..3 {
                    let top = (rgba[p00 + c] as u32) * (256 - fx) + (rgba[p10 + c] as u32) * fx;
                    let bot = (rgba[p01 + c] as u32) * (256 - fx) + (rgba[p11 + c] as u32) * fx;
                    rgb[c] = ((top * (256 - fy) + bot * fy + 32768) >> 16) as u8;
                }
                self.scene_srgb[dy * LCD_W + dx] = rgb;
            }
        }
        let luma = mean_scene_luma(&self.scene_srgb);
        self.scene_bv_ev = (9.0 + (luma / self.ref_scene_luma.max(1.0)).max(1.0 / 64.0).log2()).clamp(-2.0, 15.0);
        self.lcd_dirty = true;
        self.refresh_lv_pipeline();
    }

    pub fn set_color_mode(&mut self, cm: u32) {
        self.call_thumb(SET_CM, &[cm]);
        self.refresh_lv_pipeline();
    }

    pub fn set_grid_mode(&mut self, grid: u32) {
        self.call_thumb(SET_GRID, &[grid]);
        self.refresh_lv_pipeline();
    }

    /// Mounts a 4160-byte `D2ML` binary into `D:\LUT{slot}.BIN` (`slot = 1..=8`), sets
    /// `NV_LUT_SLOT = slot`, enables the 3D LUT engine, and selects the `SD Custom LUT` film entry if present.
    pub fn set_sd_lut(&mut self, slot: usize, data: Vec<u8>) -> bool {
        if !(1..=8).contains(&slot) || data.len() < 0x1040 || &data[0..4] != b"D2ML" {
            return false;
        }
        self.sd_luts[slot - 1] = Some(Rc::new(data));
        self.cpu.w8(NV_LUT_SLOT, slot as u8);
        self.cpu.w8(NV_LUT_ENABLE, 1); // enable 3D LUT engine
        let cmd_va = SCRATCH_VA + 0x100;
        let meta_va = SCRATCH_VA + 0x200;
        for cand in 9u8..64 {
            self.cpu.w8(meta_va + 0x1D, 0);
            self.cpu.w8(meta_va + 0x1E, cand);
            self.cpu.w32(CALL_SP + 0x14, meta_va);
            self.cpu.r[4] = cmd_va;
            self.cpu.r[1] = 0x3F80_0000;
            self.cpu.r[13] = CALL_SP;
            self.cpu.branch_exchange(IPC_CMD20 | 1);
            if self.cpu.run(IPC_CMD20_END, 50_000).is_ok() {
                let p = self.cpu.r8(cmd_va + 5);
                if (191..=198).contains(&p) {
                    self.set_color_mode(cand as u32);
                    break;
                }
            }
        }
        true
    }

    fn refresh_lv_isp_and_ycap(&mut self) -> u64 {
        let (exp, mut insns) = self.sync_ae_and_solve();
        for dirty_fn in [LV_MARK_SHARP_DIRTY, LV_REFRESH_DIRTY, LV_MARK_CONTRAST_DIRTY] {
            if let Ok((_, n)) = self.cpu.call(dirty_fn | 1, &[], &[], CALL_SP, 100_000) {
                insns += n;
            }
        }
        if let Ok((_, n)) = self.cpu.call(LV_CONFIG | 1, &[2, 2, exp.iso_speed as u32], &[], CALL_SP, 2_000_000) {
            insns += n;
        }
        self.develop_lv_planes(exp);
        insns += self.step_lv_ycap();
        insns
    }

    fn refresh_lv_pipeline(&mut self) -> u64 {
        let mut insns = self.refresh_lv_isp_and_ycap();
        if self.last_screen_id == 0x15 {
            self.hw.borrow_mut().events.push_back((4, 3));
            insns += self.drain_events();
        }
        self.lcd_dirty = true;
        insns
    }

    fn develop_lv_planes(&mut self, exp: ExposureParams) {
        let (mat, gamma, mono_en, mono_cb, mono_cr, chroma_gain) = {
            let st = self.hw.borrow();
            (
                st.b2y_mat,
                st.b2y_gamma,
                st.b2y_mono_en,
                st.b2y_mono_cb,
                st.b2y_mono_cr,
                st.b2y_chroma_gain,
            )
        };
        let sharp = self.cpu.r8(NV_SHARP).min(10) as i32;
        let contrast = self.cpu.r8(NV_CONTRAST).min(10) as i32;
        let sat = self.cpu.r8(NV_SAT).min(10) as i32;
        let contrast_q8 = 256 + (contrast - 5) * 26;
        let chroma_q8 = ((chroma_gain as i32) * (256 + (sat - 5) * 32) / 200).clamp(0, 640);

        let inv = &self.lv_inv_lut;
        let sdram = self.cpu.sdram_mut();
        for y in 0..LV_H {
            let row_px = y * LCD_W;
            let y_row = LV_Y_OFF + y * LCD_W;
            let c_row = y * (LCD_W / 2);
            for pair in 0..(LCD_W / 2) {
                let mut ys = [0i32; 2];
                let mut rs = 0i32;
                let mut gs = 0i32;
                let mut bs = 0i32;
                for (k, y_out) in ys.iter_mut().enumerate() {
                    let px_idx = row_px + pair * 2 + k;
                    let srgb = self.scene_srgb[px_idx];
                    let mut r_in = ((inv[srgb[0] as usize] as i32) * exp.gain_q8 + 128) >> 8;
                    let mut g_in = ((inv[srgb[1] as usize] as i32) * exp.gain_q8 + 128) >> 8;
                    let mut b_in = ((inv[srgb[2] as usize] as i32) * exp.gain_q8 + 128) >> 8;
                    if exp.noise_amp > 0 {
                        let [nr, ng, nb] = sensor_noise_rgb(px_idx as u32, exp.noise_amp);
                        r_in += nr >> 2;
                        g_in += ng >> 2;
                        b_in += nb >> 2;
                    }
                    r_in = r_in.clamp(0, 1023);
                    g_in = g_in.clamp(0, 1023);
                    b_in = b_in.clamp(0, 1023);

                    let r_m = ((mat[0] * r_in + mat[1] * g_in + mat[2] * b_in + 64) >> 7).clamp(0, 1023) as usize;
                    let g_m = ((mat[3] * r_in + mat[4] * g_in + mat[5] * b_in + 64) >> 7).clamp(0, 1023) as usize;
                    let b_m = ((mat[6] * r_in + mat[7] * g_in + mat[8] * b_in + 64) >> 7).clamp(0, 1023) as usize;

                    let mut r = gamma[r_m] as i32;
                    let mut g = gamma[g_m] as i32;
                    let mut b = gamma[b_m] as i32;
                    if contrast_q8 != 256 {
                        r = apply_contrast_u8(r, contrast_q8);
                        g = apply_contrast_u8(g, contrast_q8);
                        b = apply_contrast_u8(b, contrast_q8);
                    }
                    *y_out = (77 * r + 150 * g + 29 * b + 128) >> 8;
                    rs += r;
                    gs += g;
                    bs += b;
                }
                sdram[y_row + pair * 2] = ys[0].clamp(0, 255) as u8;
                sdram[y_row + pair * 2 + 1] = ys[1].clamp(0, 255) as u8;
                let (cb, cr) = if mono_en {
                    (128 + mono_cb as i32, 128 + mono_cr as i32)
                } else {
                    let r_avg = (rs + 1) >> 1;
                    let g_avg = (gs + 1) >> 1;
                    let b_avg = (bs + 1) >> 1;
                    let mut cb_d = (-43 * r_avg - 85 * g_avg + 128 * b_avg + 128) >> 8;
                    let mut cr_d = (128 * r_avg - 107 * g_avg - 21 * b_avg + 128) >> 8;
                    if chroma_q8 != 256 {
                        cb_d = (cb_d * chroma_q8 + 128) >> 8;
                        cr_d = (cr_d * chroma_q8 + 128) >> 8;
                    }
                    (cb_d + 128, cr_d + 128)
                };
                sdram[LV_CB_OFF + c_row + pair] = cb.clamp(0, 255) as u8;
                sdram[LV_CR_OFF + c_row + pair] = cr.clamp(0, 255) as u8;
            }
        }
        if sharp != 5 {
            apply_luma_sharpness(&mut sdram[LV_Y_OFF..LV_Y_OFF + LCD_W * LV_H], LCD_W, LV_H, sharp - 5, &mut self.luma_src);
        }
    }

    fn step_lv_ycap(&mut self) -> u64 {
        self.cpu.w32(SCRATCH_VA, LV_Y_VA);
        self.cpu.w32(SCRATCH_VA + 4, LV_CB_VA);
        self.cpu.w32(SCRATCH_VA + 8, LV_CR_VA);
        self.cpu.r[0] = SCRATCH_VA;
        self.cpu.r[13] = CALL_SP;
        self.cpu.branch_exchange(LV_YCAP | 1);
        self.cpu.run(LV_YCAP_END, 10_000).unwrap_or(0)
    }

    /// Runs selected M6M and M5 capture routines and presents the simulated frame in playback.
    pub fn shoot(&mut self) -> u64 {
        let mut pre_insns = 0u64;
        if self.last_screen_id == 0x1B {
            pre_insns += self.press(Key::Menu);
        } else if self.last_screen_id == 0x1A {
            pre_insns += self.press(Key::Ok);
        } else if self.last_screen_id == 0x17 {
            pre_insns += self.press(Key::Mode);
        }
        let (exp, ae_insns) = self.sync_ae_and_solve();
        let mut total_insns = pre_insns + ae_insns;
        let cm = self.color_mode() as u8;
        let cs_meta = (self.call_thumb(GET_COLOR_SPACE, &[]) as u8).clamp(1, 2);
        let wb = self.cpu.r8(NV_WB).max(1);
        let sharp = self.cpu.r8(NV_SHARP).min(10);
        let contrast = self.cpu.r8(NV_CONTRAST).min(10);
        let sat = self.cpu.r8(NV_SAT).min(10);

        let cmd_va = SCRATCH_VA + 0x100;
        let meta_va = SCRATCH_VA + 0x200;
        self.cpu.sdram_mut()[(cmd_va - SDRAM_BASE) as usize..(cmd_va - SDRAM_BASE) as usize + 64].fill(0);
        self.cpu.sdram_mut()[(meta_va - SDRAM_BASE) as usize..(meta_va - SDRAM_BASE) as usize + 64].fill(0);
        self.cpu.w8(meta_va + 0x1D, cs_meta);
        self.cpu.w8(meta_va + 0x1E, cm);
        self.cpu.w32(CALL_SP + 0x14, meta_va);
        self.cpu.r[4] = cmd_va;
        self.cpu.r[1] = 0x3F80_0000;
        self.cpu.r[13] = CALL_SP;
        self.cpu.branch_exchange(IPC_CMD20 | 1);
        if let Ok(n) = self.cpu.run(IPC_CMD20_END, 500_000) {
            total_insns += n;
        }
        let raw_packed_cm = self.cpu.r8(cmd_va + 5);
        let packed_cm = if raw_packed_cm != 0 { raw_packed_cm } else { cm };
        let raw_p1d = self.cpu.r8(cmd_va + 6);
        let p1d = if raw_p1d != 0 { raw_p1d } else { cs_meta };

        let t3d_va = SCRATCH_VA + 0x300;
        let t59_va = SCRATCH_VA + 0x340;
        let rec_va = SCRATCH_VA + 0x380;
        self.cpu.sdram_mut()[(t3d_va - SDRAM_BASE) as usize..(t3d_va - SDRAM_BASE) as usize + 0xC0].fill(0);
        let (tag3d, tag59) = if cm >= 9 {
            self.cpu.w32(CALL_SP + 0x44, t3d_va);
            self.cpu.w32(CALL_SP + 0x40, t59_va);
            self.cpu.r[3] = cm as u32;
            self.cpu.r[13] = CALL_SP;
            self.cpu.branch_exchange(MN_CM_SITE | 1);
            if let Ok(n) = self.cpu.run(MN_CM_END, 500_000) {
                total_insns += n;
            }
            (read_cstr(&self.cpu, t3d_va, 16), read_cstr(&self.cpu, t59_va, 32))
        } else {
            let s = stock_cm_name(cm);
            self.cpu.write_bytes(t3d_va, s.as_bytes());
            (s.into(), s.into())
        };

        self.cpu.r[7] = t3d_va;
        self.cpu.r[4] = rec_va;
        self.cpu.r[13] = CALL_SP;
        self.cpu.branch_exchange(PB_CM_PARSE_START | 1);
        let decoded_cm = if let Ok(n) = self.cpu.run(PB_CM_PARSE_END, 200_000) {
            total_insns += n;
            let d = self.cpu.r8(rec_va + 0x17);
            if d != 0 { d } else { cm }
        } else {
            cm
        };

        let wb_mat = wb_q7_matrix(&self.cpu);
        let (rgba, m5_insns) = self.run_m5_capture(cmd_va, packed_cm, p1d, exp, wb_mat, sharp, contrast, sat);
        total_insns += m5_insns;

        let (tv_num, tv_den) = tv_idx_rational(exp.tv_idx);
        let (av_num, av_den) = av_idx_rational(exp.av_idx);

        {
            let mut st = self.hw.borrow_mut();
            st.captures.push(CaptureFrame {
                cm,
                decoded_cm,
                packed_cm,
                p1d,
                tag3d,
                tag59,
                iso: exp.iso_speed,
                tv_num,
                tv_den,
                av_num,
                av_den,
                ev_steps: exp.ev_steps,
                expo_prog: exp.expo_prog,
                wb,
                cs: cs_meta,
                sharp,
                contrast,
                sat,
                m5_insns,
                rgba,
            });
            let len = st.captures.len();
            st.dcf_cur = len - 1;
            st.pb_open_idx = Some(len - 1);
            self.cpu.w32(DCF_DIR_COUNTS, len as u32);
        }
        // Reset PB_STATE current DCF index (+0x34) to 0 so SCREEN_PB queries PB_STEP_OR_CUR for the newest photo
        self.cpu.w32(PB_CUR_INDEX, 0);
        if self.last_screen_id == 0x15 {
            // If LV EV/dial bar is active (`0xA0020148() == 2`), first Play dismisses it, second enters SCREEN_PB
            for _ in 0..2 {
                if self.last_screen_id == 0x19 {
                    break;
                }
                self.hw.borrow_mut().events.push_back((5, Key::Play as u32));
                total_insns += self.drain_events();
            }
        } else {
            {
                let mut st = self.hw.borrow_mut();
                if self.last_screen_id != 0x19 {
                    st.events.push_back((1, 0x19));
                }
                st.events.push_back((4, 0));
            }
            total_insns += self.drain_events();
        }
        self.lcd_dirty = true;
        total_insns
    }

    /// Composites all 3 Milbeaut LCD planes (`YCbCr` video + `RGBA4444` overlay + `8bpp` CLUT OSD)
    /// into a `640×480` RGBA8888 buffer.
    pub fn render_lcd(&mut self) -> &[u8] {
        if !self.lcd_dirty {
            return &self.lcd_rgba;
        }
        self.lcd_dirty = false;

        let scr = self.screen_id();
        self.last_screen_id = scr;
        let (pb_vbuf_va, has_captures) = {
            let st = self.hw.borrow();
            (st.pb_vbuf_va, !st.captures.is_empty())
        };
        let sdram = self.cpu.sdram();

        let mut clut_rgba = [[0u8; 4]; 256];
        for (i, entry) in clut_rgba.iter_mut().enumerate() {
            let c = &sdram[CLUT_OFF + i * 4..CLUT_OFF + i * 4 + 4];
            let a = c[0].min(15);
            if a > 0 {
                let [r, g, b] = ycbcr_to_rgb(c[1] as i32, c[2] as i32, c[3] as i32);
                *entry = [r, g, b, a];
            }
        }

        let show_lv_video = matches!(scr, 0x15..=0x17 | 0x1A);
        let pb_y_off = (pb_vbuf_va.saturating_sub(SDRAM_BASE)) as usize;
        let pb_cb_off = pb_y_off + PB_PLANE_Y_LEN;
        let pb_cr_off = pb_cb_off + PB_PLANE_C_LEN;
        let show_pb_video = scr == 0x19 && has_captures && pb_vbuf_va >= SDRAM_BASE && pb_cr_off + PB_PLANE_C_LEN <= sdram.len();
        let show_argb_hud = scr == 0x15 || scr == 0x21;

        for y in 0..LCD_H {
            let row_px = y * LCD_W;
            let osd_row = &sdram[OSD_OFF + row_px..OSD_OFF + row_px + LCD_W];
            let argb_row = if show_argb_hud {
                Some(&sdram[ARGB_OFF + row_px * 2..ARGB_OFF + (row_px + LCD_W) * 2])
            } else {
                None
            };
            let video_rows = if show_lv_video && y < LV_H {
                let c_row = y * (LCD_W / 2);
                Some((
                    &sdram[LV_Y_OFF + row_px..LV_Y_OFF + row_px + LCD_W],
                    &sdram[LV_CB_OFF + c_row..LV_CB_OFF + c_row + LCD_W / 2],
                    &sdram[LV_CR_OFF + c_row..LV_CR_OFF + c_row + LCD_W / 2],
                ))
            } else if show_pb_video {
                let c_row = (y >> 1) * (LCD_W / 2);
                Some((
                    &sdram[pb_y_off + row_px..pb_y_off + row_px + LCD_W],
                    &sdram[pb_cb_off + c_row..pb_cb_off + c_row + LCD_W / 2],
                    &sdram[pb_cr_off + c_row..pb_cr_off + c_row + LCD_W / 2],
                ))
            } else {
                None
            };
            let out_row = &mut self.lcd_rgba[row_px * 4..(row_px + LCD_W) * 4];

            for x in 0..LCD_W {
                let (mut r, mut g, mut b) = if let Some((yr, cbr, crr)) = video_rows {
                    let [rr, gg, bb] = ycbcr_to_rgb(yr[x] as i32, cbr[x >> 1] as i32, crr[x >> 1] as i32);
                    (rr as u32, gg as u32, bb as u32)
                } else {
                    (0u32, 0u32, 0u32)
                };

                // Plane 1: RGBA4444 (R:12..15, G:8..11, B:4..7, A:0..3)
                if let Some(ar) = argb_row {
                    let px16 = u16::from_be_bytes([ar[x * 2], ar[x * 2 + 1]]);
                    let a4 = (px16 & 0xF) as u32;
                    if a4 > 0 {
                        let r4 = ((px16 >> 12) & 0xF) as u32 * 17;
                        let g4 = ((px16 >> 8) & 0xF) as u32 * 17;
                        let b4 = ((px16 >> 4) & 0xF) as u32 * 17;
                        if a4 >= 15 {
                            r = r4;
                            g = g4;
                            b = b4;
                        } else {
                            let inv_a = 15 - a4;
                            r = ((r * inv_a + r4 * a4 + 7) * 0x1111) >> 16;
                            g = ((g * inv_a + g4 * a4 + 7) * 0x1111) >> 16;
                            b = ((b * inv_a + b4 * a4 + 7) * 0x1111) >> 16;
                        }
                    }
                }

                let ce = clut_rgba[osd_row[x] as usize];
                let oa = ce[3] as u32;
                if oa > 0 {
                    if oa >= 15 {
                        r = ce[0] as u32;
                        g = ce[1] as u32;
                        b = ce[2] as u32;
                    } else {
                        let inv_a = 15 - oa;
                        r = ((r * inv_a + (ce[0] as u32) * oa + 7) * 0x1111) >> 16;
                        g = ((g * inv_a + (ce[1] as u32) * oa + 7) * 0x1111) >> 16;
                        b = ((b * inv_a + (ce[2] as u32) * oa + 7) * 0x1111) >> 16;
                    }
                }

                let o = x * 4;
                out_row[o] = r as u8;
                out_row[o + 1] = g as u8;
                out_row[o + 2] = b as u8;
                out_row[o + 3] = 255;
            }
        }

        &self.lcd_rgba
    }

    pub fn screen_id(&mut self) -> u32 {
        let scr = self.call_thumb(UI_SCREEN_ID, &[]);
        self.last_screen_id = scr;
        scr
    }

    pub fn color_mode(&mut self) -> u32 {
        self.call_thumb(GET_CM, &[])
    }

    pub fn expo_mode(&mut self) -> u32 {
        self.call_thumb(GET_EXPO_MODE, &[])
    }

    pub fn grid_mode(&mut self) -> u32 {
        self.call_thumb(GET_GRID, &[])
    }

    pub fn in_playback(&self) -> bool {
        self.last_screen_id == 0x19 || (self.last_screen_id == 0x1B && self.pb_menu_parent)
    }

    pub fn capture_count(&self) -> usize {
        self.hw.borrow().captures.len()
    }

    pub fn captures(&self) -> Vec<CaptureFrame> {
        self.hw.borrow().captures.clone()
    }

    pub fn last_capture(&self) -> Option<CaptureFrame> {
        self.hw.borrow().captures.last().cloned()
    }

    pub fn with_last_capture<R>(&self, f: impl FnOnce(&CaptureFrame) -> R) -> Option<R> {
        self.hw.borrow().captures.last().map(f)
    }

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    pub fn cpu_mut(&mut self) -> &mut Cpu {
        self.lcd_dirty = true;
        &mut self.cpu
    }
}
