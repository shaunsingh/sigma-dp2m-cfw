//! Dual-CPU Sigma DP2 Merrill firmware simulator (`arm9` M6M + `fr80` M5Slave).
//!
//! Runs compiled `C78FRM.bin` (ARM926EJ-S BE) for the camera UI, native AE solver, Live View
//! B2Y ISP pipeline, HUD overlays, NVRAM/NOR persistence, native `SCREEN_PB` (`0x19`) playback,
//! and IPC packing, and runs compiled `Firm.bin` (FR80 BE) for full-pipeline shutter captures
//! (`hook_ipc_exif`, `hook_matrix`, `hook_lutsel`, `hook_mcas`, `spatial_driver`, `ycc_engine`).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use arm9::{Cpu, HookAction, SDRAM_BASE};
use fr80::Emulator as Fr80Emu;

pub const LCD_W: usize = 640;
pub const LCD_H: usize = 480;
pub const LV_H: usize = 424;

const PKG_MAGIC: &[u8] = b"SIGMA.CODP2M";
const PKG_S2_OFF: usize = 0x0082_0080;
const PKG_S2_LEN: usize = 0x0088_4080;
const PKG_C78_OFF: usize = 0x010A_4100;
const PKG_C78_LEN: usize = 0x0082_0000;
const C78_S3_OFF: usize = 0x0002_0000;
const S3_LEN: usize = 0x0080_0000;

// M6M stock addresses
const S3_BSS_START: usize = 0x007F_BC1C;
const S3_BSS_END: usize = 0x00A1_F398;
const NV_BLOCK: u32 = 0xA093_9684;
const NV_DEFAULTS: u32 = 0xA07F_9E68;
const NV_LEN: usize = 0x44C;
const NV_LOAD: u32 = 0xA004_0110;
const NV_FLUSH: u32 = 0xA004_017A;
const NOR_READ: u32 = 0xA002_06CC;
const NOR_WRITE: u32 = 0xA002_06FA;
const MALLOC: u32 = 0xA003_D9C0;
const FREE: u32 = 0xA003_D9AC;
const CALLOC: u32 = 0xA003_D990;
const REALLOC: u32 = 0xA003_D9D8;
const GET_LANG: u32 = 0xA016_7DD4;
const FONT_SCALE: u32 = 0xA07D_D774;

// ARM libc routines in C78FRM.bin (ARM state)
const LIBC_MEMCPY: u32 = 0xA010_4DAC;
const LIBC_MEMSET: u32 = 0xA010_4DB8;
const LIBC_AEABI_MEMSET: u32 = 0xA010_4DD4;
const LIBC_AEABI_MEMMOVE: u32 = 0xA010_4EB8;
const LIBC_AEABI_MEMCLR: u32 = 0xA010_4F8C;
const LIBC_AEABI_MEMCLR4: u32 = 0xA010_5030;

const UI_INIT_A: u32 = 0xA004_0D74;
const UI_INIT_B: u32 = 0xA004_0D7C;
const SCREEN_POST_MBOX: u32 = 0xA004_2B4E;
const SCREEN_POST_RET: u32 = 0xA004_2B70;
const UI_DISPATCH: u32 = 0xA004_3234;
const UI_SCREEN_ID: u32 = 0xA004_333C;
const SET_CARD_READY: u32 = 0xA016_80A0;
const LV_ACTIVE: u32 = 0xA003_44F2;
const ACCEL_X: u32 = 0xA001_A38E;
const ACCEL_Y: u32 = 0xA001_A3A2;

// Native M6M AE solver & exposure parameter sync
const AE_SENSOR_EV100: u32 = 0xA000_273C;
const AE_ENABLE_LV_METER: u32 = 0xA000_2CB2;
const AE_STATE: u32 = 0xA080_08F8;
const AE_SET_MODE: u32 = 0xA000_D1A2;
const AE_SET_AUTO_ISO_MIN: u32 = 0xA000_D2AA;
const AE_SET_AUTO_ISO_MAX: u32 = 0xA000_D2B2;
const AE_AV_TO_Q16: u32 = 0xA000_DDF6;
const AE_TV_TO_Q16: u32 = 0xA000_DE26;
const AE_SYNC_ISO: u32 = 0xA003_F3A6;
const AE_MAP_ISO_IDX: u32 = 0xA003_F288;
const AE_SYNC_TV: u32 = 0xA003_EDD0;
const AE_SYNC_AV: u32 = 0xA003_EECC;
const AE_SYNC_EV: u32 = 0xA003_EC88;
const AE_SOLVE_UI: u32 = 0xA003_EABE;
const AE_OUT_AV_IDX: u32 = 0xA07D_D6E0;
const AE_OUT_TV_IDX: u32 = 0xA07D_D6E4;
const GET_AUTO_ISO_MIN_UI: u32 = 0xA016_F4FE;
const GET_AUTO_ISO_MAX_UI: u32 = 0xA016_F556;
const GET_ISO_IDX: u32 = 0xA016_F5AE;
const GET_EV_IDX: u32 = 0xA016_7BF0;

const LV_CONFIG: u32 = 0xA003_A180;
const LV_MARK_SHARP_DIRTY: u32 = 0xA003_A258;
const LV_MARK_DIRTY: u32 = 0xA003_A260;
const LV_MARK_CONTRAST_DIRTY: u32 = 0xA003_A268;
const LV_YCAP: u32 = 0xA003_3F1A;
const LV_YCAP_END: u32 = 0xA003_3F20;
const GET_LV_CC7: u32 = 0xA016_33CA;
const GET_LV_CCN: u32 = 0xA016_339A;
const B2Y_SET_MAT: u32 = 0xA009_C334;
const B2Y_SET_MONO: u32 = 0xA009_CD7C;
const B2Y_SET_CHROMA: u32 = 0xA009_CF84;
const B2Y_SET_SHARP_LUT: u32 = 0xA009_D650;
const B2Y_SET_GAMMA: u32 = 0xA009_D940;
const STOCK_LV_LUT1024_OFF: usize = 0x0026_8A14;

const GET_CM: u32 = 0xA016_7926;
const GET_COLOR_SPACE: u32 = 0xA016_797E;
const GET_EXPO_MODE: u32 = 0xA016_7892;
const GET_GRID: u32 = 0xA016_7F98;
const IPC_CMD20: u32 = 0xA003_60A2;
const IPC_CMD20_END: u32 = 0xA003_60A8;
const MN_CM_SITE: u32 = 0xA014_8CF8;
const MN_CM_END: u32 = 0xA014_8D2E;
const PB_CM_PARSE_START: u32 = 0xA014_78A6;
const PB_CM_PARSE_END: u32 = 0xA014_793C;

// Native SCREEN_PB (0x19) & virtual DCF SD card hooks
const PB_SELECT_VBUF: u32 = 0xA000_F222;
const PB_IIP_HIST: u32 = 0xA004_A7AC;
const PB_STEP_OR_CUR: u32 = 0xA005_3354;
const PB_HW_JPEG_DEC: u32 = 0xA005_35C8;
const PB_DECODE_RESAMPLE: u32 = 0xA005_3F5C;
const PB_YCBCR_TO_RGB_PLANAR: u32 = 0xA005_4AB2;
const PB_OPEN_JPEG_B: u32 = 0xA005_7D40;
const PB_OPEN_JPEG_A: u32 = 0xA005_7DE6;
const PB_DEC_INIT: u32 = 0xA005_7EA8;
const PB_OPEN_IMAGE_B: u32 = 0xA005_7EB0;
const PB_OPEN_IMAGE_A: u32 = 0xA005_7F16;
const DCF_DIR_COUNTS: u32 = 0xA096_2758;
const DCF_SYNC_CUR: u32 = 0xA011_49EC;
const DCF_GET_FILE_INFO: u32 = 0xA011_51D0;
const SD_MOUNT_STATUS: u32 = 0xA011_6A4C;
const DCF_COUNT_IMAGES: u32 = 0xA013_4234;
const DCF_GET_ENTRY: u32 = 0xA013_4286;
const DCF_READ_JPEG_EXIF: u32 = 0xA013_42EC;
const DCF_READ_X3F_EXIF: u32 = 0xA013_4326;
const DCF_SCAN_READY: u32 = 0xA014_FDE6;
const DCF_GET_CUR: u32 = 0xA015_39BE;
const DCF_GET_TOTAL: u32 = 0xA015_39DE;
const DCF_STEP_NEXT: u32 = 0xA015_3C22;
const DCF_STEP_PREV: u32 = 0xA015_3C7E;
const DCF_SET_CUR: u32 = 0xA015_3CE6;
const SD_FREE_SPACE: u32 = 0xA015_5144;

// LCD planes in M6M SDRAM
const OSD_OFF: usize = 0x0213_0000;
const ARGB_OFF: usize = 0x021D_8C00;
const CLUT_OFF: usize = 0x007D_EC54;
const LV_Y_VA: u32 = 0xA2A0_0000;
const LV_CB_VA: u32 = 0xA2A5_0000;
const LV_CR_VA: u32 = 0xA2A8_0000;
const LV_Y_OFF: usize = 0x02A0_0000;
const LV_CB_OFF: usize = 0x02A5_0000;
const LV_CR_OFF: usize = 0x02A8_0000;
const PB_VBUF_DEFAULT_VA: u32 = 0xA235_E600;
const PB_PLANE_Y_LEN: usize = LCD_W * LCD_H;
const PB_PLANE_C_LEN: usize = (LCD_W / 2) * (LCD_H / 2);
const CALL_SP: u32 = 0xA200_1000;
const SCRATCH_VA: u32 = 0xA2B0_0000;

// M5Slave (Firm.bin) layout & hooks
const M5_DELTA: u32 = 0x0003_BF80;
const M5_FLASH_LIMIT: usize = 0x007C_4080;
const M5_STD_LUT_VA: u32 = 0x0054_CC94;
const M5_MONO_LUT_VA: u32 = 0x0055_2C94;
const M5_MATRIX_HOOK: u32 = 0x005D_0000;
const M5_LUTSEL_HOOK: u32 = 0x005D_0180;
const M5_MCAS_HOOK: u32 = 0x005D_0200;
const M5_IPC_UNPK_HOOK: u32 = 0x005D_0340;
const M5_SPATIAL_DRV: u32 = 0x005D_1600;

const M5_SYM_GET_CM: u32 = 0x004D_D702;
const M5_SYM_GET_P1D: u32 = 0x004D_D72A;
const M5_SYM_ORIG_IPC: u32 = 0x004D_CC48;
const M5_SYM_ORIG_CC: u32 = 0x0053_BEF0;
const M5_SYM_ORIG_LUT: u32 = 0x0049_50E0;
const M5_SYM_COMMIT_CC: u32 = 0x004B_1F20;
const M5_SYM_ORIG_MCAS: u32 = 0x004B_3546;
const M5_SYM_MEMCPY: u32 = 0x004A_201C;
const M5_SYM_COMMIT_MCAS: u32 = 0x004B_2088;
const M5_SYM_MDF_OPEN: u32 = 0x0048_AB80;
const M5_SYM_MDF_CLOSE: u32 = 0x0048_ABAA;
const M5_SYM_MDF_READ: u32 = 0x0048_ABD0;
const M5_SYM_STRIDE_DEFAULT: u32 = 0x0049_07CA;
const M5_SYM_HW_JPEG_ENCODE: u32 = 0x004D_3E82;
const M5_SYM_GET_IMG_DIM: u32 = 0x004D_4E56;
const M5_RAM_CC_ACTIVE: u32 = 0x8000_CC44;
const M5_RAM_CS_FLAG: u32 = 0x8000_1B88;
const M5_SCRATCH_BASE: u32 = 0x8010_0000;
const M5_SCRATCH_LEN: usize = 0x5000;
const M5_IMG_VA: [u32; 3] = [0x8200_0000, 0x8300_0000, 0x8380_0000];

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

struct HeapBlock {
    va: u32,
    cap: u32,
}

struct HwState {
    events: VecDeque<(u32, u32)>,
    nor: [u8; 0x800],
    heap_bump: u32,
    free_list: Vec<HeapBlock>,
    alloc_caps: Vec<(u32, u32)>,
    b2y_mat: [i32; 9],
    b2y_gamma: [u8; 1024],
    b2y_mono_en: bool,
    b2y_mono_cb: i8,
    b2y_mono_cr: i8,
    b2y_chroma_gain: u8,
    b2y_sharp_idx: u8,
    accel_x: i8,
    accel_y: i8,
    lv_active: bool,
    scene_ev100_q16: i32,
    captures: Vec<CaptureFrame>,
    dcf_cur: usize,
    pb_open_idx: Option<usize>,
    pb_vbuf_va: u32,
}

#[derive(Clone, Copy)]
struct ExposureParams {
    gain_q8: i32, // 256 == 1.0x linear sensor light
    noise_amp: i32,
    av_idx: u16,
    tv_idx: u16,
    iso_speed: u16,
    ev_steps: i32,
    expo_prog: u8,
}

pub struct Camera {
    cpu: Cpu,
    hw: Rc<RefCell<HwState>>,
    m5_flash: Vec<u8>,
    sd_luts: [Option<Vec<u8>>; 8],
    lv_inv_lut: [u16; 256],
    m5_inv_lut: [u16; 256],
    scene_srgb: Vec<[u8; 3]>, // 640 * 424
    ref_scene_luma: f32,
    scene_bv_ev: f32,
    last_screen_id: u32,
    pb_menu_parent: bool,
    lcd_rgba: Vec<u8>, // 640 * 480 * 4
    lcd_dirty: bool,
    lv_dirty: bool,
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

        let hw = Rc::new(RefCell::new(HwState {
            events: VecDeque::with_capacity(16),
            nor: [0u8; 0x800],
            heap_bump: 0xA0A2_0000,
            free_list: Vec::new(),
            alloc_caps: Vec::new(),
            b2y_mat: [128, 0, 0, 0, 128, 0, 0, 0, 128],
            b2y_gamma,
            b2y_mono_en: false,
            b2y_mono_cb: 0,
            b2y_mono_cr: 0,
            b2y_chroma_gain: 200,
            b2y_sharp_idx: 5,
            accel_x: 0,
            accel_y: 64,
            lv_active: true,
            scene_ev100_q16: 9 * 65536,
            captures: Vec::new(),
            dcf_cur: 0,
            pb_open_idx: None,
            pb_vbuf_va: PB_VBUF_DEFAULT_VA,
        }));

        let mut cpu = Cpu::new();
        cpu.sdram_mut()[..S3_LEN].copy_from_slice(s3);
        cpu.sdram_mut()[S3_BSS_START..S3_BSS_END].fill(0);
        cpu.w32(FONT_SCALE, 1);
        cpu.w32(FONT_SCALE + 4, 1);

        // Pre-clear 4:2:0 playback video buffers to neutral black (Y=0, Cb=128, Cr=128)
        for &vbuf_va in &[0xA235_E600u32, 0xA243_F600, 0xA252_0600, 0xA260_1600, 0xA259_0E00] {
            clear_yuv420_buf(cpu.sdram_mut(), vbuf_va);
        }

        install_m6m_hooks(&mut cpu, &hw);

        // Seed NVRAM defaults and flush a checksummed NOR Bank 0 record so NV_LOAD & custom slots work natively
        let nv_def_off = (NV_DEFAULTS - SDRAM_BASE) as usize;
        let mut defaults = [0u8; NV_LEN];
        defaults.copy_from_slice(&s3[nv_def_off..nv_def_off + NV_LEN]);
        cpu.write_bytes(NV_BLOCK, &defaults);

        let scene_srgb = default_scene_srgb();
        let ref_scene_luma = mean_scene_luma(&scene_srgb);

        let mut cam = Self {
            cpu,
            hw,
            m5_flash: firm[..M5_FLASH_LIMIT].to_vec(),
            sd_luts: std::array::from_fn(|_| None),
            lv_inv_lut,
            m5_inv_lut,
            scene_srgb,
            ref_scene_luma,
            scene_bv_ev: 9.0,
            last_screen_id: 0x15,
            pb_menu_parent: false,
            lcd_rgba: vec![0u8; LCD_W * LCD_H * 4],
            lcd_dirty: true,
            lv_dirty: true,
        };

        cam.boot_m6m();
        Ok(cam)
    }

    fn boot_m6m(&mut self) {
        // Default NV_DISP_LV = 4 (status bar + icons + grid, without unpopulated HW luma histogram)
        self.cpu.w8(0xA093_9698, 4);
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
        let (r0, _) = self
            .cpu
            .call(va | 1, args, &[], CALL_SP, 25_000_000)
            .unwrap_or((0, 0));
        r0
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

    /// Synchronizes NVRAM exposure parameters into M6M's `AE_STATE` (`0xA08008F8`), feeds the
    /// scene Brightness Value (`BV_Q16`) into `0xA07AD8B8`, and executes M6M's native Thumb AE
    /// solver (`0xA003EABE` -> `0xA000D6B6` + `autoiso_a_hook`/`autoiso_p_hook`).
    fn sync_ae_and_solve(&mut self) -> (ExposureParams, u64) {
        let mut insns = 0u64;
        let scene_ev100_q16 = (self.scene_bv_ev * 65536.0).round() as i32;
        self.hw.borrow_mut().scene_ev100_q16 = scene_ev100_q16;

        // Map UI expo_mode (`0xA09396AC`) -> active AE program (`0xA09396C0`: 1=M, 2=P, 3=A, 4=S)
        // and AE_STATE[0] (`1=M, 2=A, 3=S, 4=P`)
        let ui_mode = self.cpu.r8(0xA093_96AC);
        let prog = match ui_mode {
            0 => 1u8, // M
            1 => 4u8, // S
            2 => 3u8, // A
            3 => 2u8, // P
            _ => self.cpu.r8(0xA093_96C0).clamp(1, 4),
        };
        self.cpu.w8(0xA093_96C0, prog);
        let ae_mode = match prog {
            1 => 1u32, // M
            2 => 4u32, // P (program line 0xA000E218)
            3 => 2u32, // A (user Av, solve Tv)
            _ => 3u32, // S (user Tv, solve Av)
        };

        let iso_ext = self.cpu.r8(0xA093_96DD) as u32;
        let min_iso_ui = self.call_thumb(GET_AUTO_ISO_MIN_UI, &[]);
        let max_iso_ui = self.call_thumb(GET_AUTO_ISO_MAX_UI, &[]);
        let min_iso_ae = self.call_thumb(AE_MAP_ISO_IDX, &[min_iso_ui]);
        let max_iso_ae = self.call_thumb(AE_MAP_ISO_IDX, &[max_iso_ui]);
        let iso_idx = self.call_thumb(GET_ISO_IDX, &[]);
        let ev_idx = self.call_thumb(GET_EV_IDX, &[]);
        for (fn_va, arg) in [
            (AE_SET_MODE, Some(ae_mode)),
            (0xA016_F5DE, Some(iso_ext)),
            (AE_SET_AUTO_ISO_MIN, Some(min_iso_ae)),
            (AE_SET_AUTO_ISO_MAX, Some(max_iso_ae)),
            (AE_SYNC_ISO, Some(iso_idx)),
            (AE_SYNC_AV, None),
            (AE_SYNC_TV, None),
            (AE_SYNC_EV, Some(ev_idx)),
            (AE_SOLVE_UI, None),
        ] {
            let slice: &[u32] = match arg {
                Some(ref a) => core::slice::from_ref(a),
                None => &[],
            };
            if let Ok((_, n)) = self.cpu.call(fn_va | 1, slice, &[], CALL_SP, 1_000_000) {
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
        if key == Key::Shutter {
            // Half-press / shutter in Playback exits natively back to Live View (restoring shooting expo_mode)
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

        // Entering empty Playback clears the active PB video plane so "No Image" sits on clean black
        if key == Key::Play && prev_scr == 0x15 && self.hw.borrow().captures.is_empty() {
            let vbuf = self.hw.borrow().pb_vbuf_va;
            clear_yuv420_buf(self.cpu.sdram_mut(), vbuf);
        }

        self.hw.borrow_mut().events.push_back((5, key as u32));
        let mut insns = self.drain_events();
        let new_scr = self.last_screen_id;
        self.lv_dirty = true;
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

    /// Advances animated HUD overlays (such as diagonal zebra phase) by one Live View frame.
    pub fn tick(&mut self) -> bool {
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
        if w == 0 || h == 0 || rgba.len() < w * h * 4 {
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
        self.lv_dirty = true;
        self.lcd_dirty = true;
        self.refresh_lv_pipeline();
    }

    /// Sets the active color mode via the stock M6M setter (`0xA016F86E`), keeping NVRAM,
    /// the `0xA07FBC10` cache, and Live View dirty state in sync.
    pub fn set_color_mode(&mut self, cm: u32) {
        self.call_thumb(0xA016_F86E, &[cm]);
        self.refresh_lv_pipeline();
    }

    /// Sets the active grid/HUD overlay mode via `stock_set_grid` (`0xA0167FA0`).
    pub fn set_grid_mode(&mut self, grid: u32) {
        self.call_thumb(0xA016_7FA0, &[grid]);
        self.refresh_lv_pipeline();
    }

    /// Mounts a 4160-byte `D2ML` binary into `D:\LUT{slot}.BIN` (`slot = 1..=8`), sets
    /// `NV_LUT_SLOT = slot`, enables the 3D LUT engine, and selects the `SD Custom LUT` film entry if present.
    pub fn set_sd_lut(&mut self, slot: usize, data: Vec<u8>) -> bool {
        if !(1..=8).contains(&slot) || data.len() < 0x1040 || &data[0..4] != b"D2ML" {
            return false;
        }
        self.sd_luts[slot - 1] = Some(data);
        self.cpu.w8(0xA093_9690, slot as u8);
        self.cpu.w8(0xA093_9689, 1); // enable 3D LUT engine
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
        for dirty_fn in [LV_MARK_SHARP_DIRTY, LV_MARK_DIRTY, LV_MARK_CONTRAST_DIRTY] {
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
        self.lv_dirty = false;
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
        let sharp = self.cpu.r8(0xA093_96D4).min(10) as i32;
        let contrast = self.cpu.r8(0xA093_96D5).min(10) as i32;
        let sat = self.cpu.r8(0xA093_96D6).min(10) as i32;
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
            apply_luma_sharpness(&mut sdram[LV_Y_OFF..LV_Y_OFF + LCD_W * LV_H], LCD_W, LV_H, sharp - 5);
        }
    }

    fn step_lv_ycap(&mut self) -> u64 {
        let mut insns = 0u64;
        self.cpu.w32(SCRATCH_VA, LV_Y_VA);
        self.cpu.w32(SCRATCH_VA + 4, LV_CB_VA);
        self.cpu.w32(SCRATCH_VA + 8, LV_CR_VA);
        self.cpu.r[0] = SCRATCH_VA;
        self.cpu.r[13] = CALL_SP;
        self.cpu.branch_exchange(LV_YCAP | 1);
        if let Ok(n) = self.cpu.run(LV_YCAP_END, 10_000) {
            insns += n;
        }
        insns
    }

    /// Runs a full dual-CPU shutter capture: M6M AE + `ipc_pack` + `mn_film` + `pb_cm_jpg`,
    /// followed by M5Slave `Firm.bin` FR80 hooks + `spatial_driver` + `ycc_engine`, and enters
    /// native M6M `SCREEN_PB` (`0x19`).
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
        let wb = self.cpu.r8(0xA093_96C4).max(1);
        let sharp = self.cpu.r8(0xA093_96D4).min(10);
        let contrast = self.cpu.r8(0xA093_96D5).min(10);
        let sat = self.cpu.r8(0xA093_96D6).min(10);

        // 1. Pack IPC cmd 0x20 via M6M firmware hook @ 0xA00360A2
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

        // 2. MakerNote film tag hook @ 0xA0148CF8 + native pb_cm_jpg -> pb_film_cm round-trip @ 0xA01478A6
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

        // 3. M5Slave FR80 full capture pipeline (with WB + Exposure + Picture Settings)
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
        self.cpu.w32(0xA07F_220C, 0);
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

    #[allow(clippy::too_many_arguments)]
    fn run_m5_capture(
        &self,
        cmd_va_m6m: u32,
        packed_cm: u8,
        p1d: u8,
        exp: ExposureParams,
        wb_mat: [i32; 9],
        sharp: u8,
        contrast: u8,
        sat: u8,
    ) -> (Vec<u8>, u64) {
        let mut emu = Fr80Emu::new(&self.m5_flash, M5_DELTA);
        let mut m5_insns = 0u64;
        let has_m5_hooks = emu.r16(M5_MATRIX_HOOK) != 0;

        // 1. Unpack M6M IPC cmd 0x20 into M5 job slot (`+0x150` = cm, `+0x14C` = p1d)
        let ipc_va: u32 = 0x8008_1000;
        let slot_va: u32 = 0x8008_1200;
        for i in 0..32u32 {
            emu.w8(ipc_va + i, self.cpu.r8(cmd_va_m6m + i));
        }
        let stock_m5_cm = match packed_cm {
            1 => 2u32,
            2 => 1u32,
            _ => 3u32,
        };
        emu.w32(slot_va + 0x150, stock_m5_cm);
        emu.w32(slot_va + 0x14C, (p1d & 3).max(1) as u32);
        if has_m5_hooks {
            emu.r[14] = 0x8008_0800;
            emu.r[15] = 0x8008_0400;
            emu.w32(0x8008_0800 + 8, ipc_va);
            emu.r[4] = 5;
            emu.r[5] = slot_va;
            emu.set_handler(M5_SYM_ORIG_IPC, |e| e.rp);
            if let Ok(n) = emu.run(M5_IPC_UNPK_HOOK, 2_000) {
                m5_insns += n;
            }
        }
        let job_cm = emu.r32(slot_va + 0x150);
        let job_p1d = emu.r32(slot_va + 0x14C);
        let cs_val = (job_p1d & 3).max(1);

        // 2. Run M5 CCMatrix hook (`0x005D0000`) against active White Balance matrix in M5_RAM_CC_ACTIVE
        emu.w32(M5_RAM_CS_FLAG, cs_val);
        for (k, &v) in wb_mat.iter().enumerate() {
            emu.w16(M5_RAM_CC_ACTIVE + (k as u32) * 2, (v as u16) & 0x0FFF);
        }
        let committed_cc = Rc::new(RefCell::new(wb_mat));
        if has_m5_hooks {
            emu.set_handler(M5_SYM_ORIG_CC, |e| e.rp);
            {
                let cc_c = Rc::clone(&committed_cc);
                emu.set_handler(M5_SYM_COMMIT_CC, move |e| {
                    let mut m = [0i32; 9];
                    for (k, slot) in m.iter_mut().enumerate() {
                        let raw = e.r16(e.r[4] + 2 + (k as u32) * 2) as i32;
                        *slot = if raw & 0x800 != 0 { raw - 4096 } else { raw };
                    }
                    *cc_c.borrow_mut() = m;
                    e.rp
                });
            }
            emu.r[14] = 0x8008_0200;
            emu.r[15] = 0x8008_0100;
            emu.w32(0x8008_0200 + 12, job_cm);
            if let Ok(n) = emu.run(M5_MATRIX_HOOK, 10_000) {
                m5_insns += n;
            }
        }

        // 3. Run M5 4096-byte 1D LUT selector hook (`0x005D0180`), or stock selector on unpatched FW
        let lut_va = if has_m5_hooks {
            emu.set_handler(M5_SYM_ORIG_LUT, |e| {
                e.r[4] = if e.r[5] == 2 { M5_MONO_LUT_VA } else { M5_STD_LUT_VA };
                e.rp
            });
            emu.r[5] = job_cm;
            emu.r[15] = 0x8008_0100;
            if let Ok(n) = emu.run(M5_LUTSEL_HOOK, 2_000) {
                m5_insns += n;
            }
            emu.r[4]
        } else if job_cm == 2 {
            M5_MONO_LUT_VA
        } else {
            M5_STD_LUT_VA
        };

        // 4. Run M5 12-sector MCAS hook (`0x005D0200`)
        let mcas_buf_va = Rc::new(RefCell::new(0u32));
        if has_m5_hooks {
            emu.set_handler(M5_SYM_ORIG_MCAS, |e| e.rp);
            emu.set_handler(M5_SYM_GET_CM, move |e| {
                e.r[4] = job_cm;
                e.rp
            });
            emu.set_handler(M5_SYM_MEMCPY, |e| {
                let (dst, src, n) = (e.r[4], e.r[5], e.r[6]);
                for i in 0..n {
                    let b = e.r8(src + i);
                    e.w8(dst + i, b);
                }
                e.rp
            });
            {
                let mc = Rc::clone(&mcas_buf_va);
                emu.set_handler(M5_SYM_COMMIT_MCAS, move |e| {
                    *mc.borrow_mut() = e.r[5];
                    e.rp
                });
            }
            emu.r[14] = 0x8008_0200;
            emu.r[15] = 0x8008_0180;
            emu.w32(0x8008_0200 + 16, 1);
            if let Ok(n) = emu.run(M5_MCAS_HOOK, 20_000) {
                m5_insns += n;
            }
        }

        let cc = *committed_cc.borrow();
        let mut lut4096 = [0u8; 4096];
        for (i, b) in lut4096.iter_mut().enumerate() {
            *b = emu.r8(lut_va + i as u32);
        }
        let mcas_va = *mcas_buf_va.borrow();
        let mcas_sectors = if mcas_va != 0 {
            let mut mck = [[0i32; 9]; 12];
            for (s, sec) in mck.iter_mut().enumerate() {
                let base = mcas_va + 0x46 + (s as u32) * 0x1E;
                for (k, v) in sec.iter_mut().enumerate() {
                    *v = (emu.r16(base + (k as u32) * 2) as i16) as i32;
                }
            }
            Some(mck)
        } else {
            None
        };

        // Stock M5 monochrome/sepia/chroma flags + user Picture Settings
        let is_stock_mono = packed_cm == 1;
        let is_stock_sepia = packed_cm == 2;
        let stock_chroma = match packed_cm {
            4 => 292, // Vivid
            5 => 228, // Neutral
            6 => 244, // Portrait
            7 => 280, // Landscape
            _ => 256,
        };
        let contrast_q8 = 256 + ((contrast as i32) - 5) * 26;
        let chroma_q8 = ((stock_chroma * (256 + ((sat as i32) - 5) * 32)) >> 8).clamp(0, 640);

        // 5. Develop 640x424 scene through M5 B2Y (Exposure + CCMatrix + MCAS + 4096B 1D LUT) into 4:2:2 Y/Cb/Cr
        let mut y_plane = vec![0u8; LCD_W * LV_H];
        let mut cb_plane = vec![0u8; (LCD_W / 2) * LV_H];
        let mut cr_plane = vec![0u8; (LCD_W / 2) * LV_H];
        let inv = &self.m5_inv_lut;

        for y in 0..LV_H {
            let row_px = y * LCD_W;
            let c_row = y * (LCD_W / 2);
            for pair in 0..(LCD_W / 2) {
                let mut ys = [0i32; 2];
                let mut r_sum = 0i32;
                let mut g_sum = 0i32;
                let mut b_sum = 0i32;
                for (k, y_out) in ys.iter_mut().enumerate() {
                    let px_idx = row_px + pair * 2 + k;
                    let srgb = self.scene_srgb[px_idx];
                    let mut r_in = ((inv[srgb[0] as usize] as i32) * exp.gain_q8 + 128) >> 8;
                    let mut g_in = ((inv[srgb[1] as usize] as i32) * exp.gain_q8 + 128) >> 8;
                    let mut b_in = ((inv[srgb[2] as usize] as i32) * exp.gain_q8 + 128) >> 8;
                    if exp.noise_amp > 0 {
                        let [nr, ng, nb] = sensor_noise_rgb(px_idx as u32 ^ 0x5A5A_5A5A, exp.noise_amp);
                        r_in += nr;
                        g_in += ng;
                        b_in += nb;
                    }
                    r_in = r_in.clamp(0, 4095);
                    g_in = g_in.clamp(0, 4095);
                    b_in = b_in.clamp(0, 4095);

                    let mut r_l = ((cc[0] * r_in + cc[1] * g_in + cc[2] * b_in + 64) >> 7).clamp(0, 4095);
                    let mut g_l = ((cc[3] * r_in + cc[4] * g_in + cc[5] * b_in + 64) >> 7).clamp(0, 4095);
                    let mut b_l = ((cc[6] * r_in + cc[7] * g_in + cc[8] * b_in + 64) >> 7).clamp(0, 4095);

                    if let Some(ref mck) = mcas_sectors {
                        let sec = hue_sector_12(r_l, g_l, b_l);
                        let m = &mck[sec];
                        let rm = ((m[0] * r_l + m[1] * g_l + m[2] * b_l + 512) >> 10).clamp(0, 4095);
                        let gm = ((m[3] * r_l + m[4] * g_l + m[5] * b_l + 512) >> 10).clamp(0, 4095);
                        let bm = ((m[6] * r_l + m[7] * g_l + m[8] * b_l + 512) >> 10).clamp(0, 4095);
                        r_l = rm;
                        g_l = gm;
                        b_l = bm;
                    }

                    let mut r = lut4096[r_l as usize] as i32;
                    let mut g = lut4096[g_l as usize] as i32;
                    let mut b = lut4096[b_l as usize] as i32;
                    if contrast_q8 != 256 {
                        r = apply_contrast_u8(r, contrast_q8);
                        g = apply_contrast_u8(g, contrast_q8);
                        b = apply_contrast_u8(b, contrast_q8);
                    }
                    *y_out = (77 * r + 150 * g + 29 * b + 128) >> 8;
                    r_sum += r;
                    g_sum += g;
                    b_sum += b;
                }
                y_plane[row_px + pair * 2] = ys[0].clamp(0, 255) as u8;
                y_plane[row_px + pair * 2 + 1] = ys[1].clamp(0, 255) as u8;
                let (cb, cr) = if is_stock_mono {
                    (128, 128)
                } else if is_stock_sepia {
                    (108, 148)
                } else {
                    let r_avg = (r_sum + 1) >> 1;
                    let g_avg = (g_sum + 1) >> 1;
                    let b_avg = (b_sum + 1) >> 1;
                    let mut cb_d = (-43 * r_avg - 85 * g_avg + 128 * b_avg + 128) >> 8;
                    let mut cr_d = (128 * r_avg - 107 * g_avg - 21 * b_avg + 128) >> 8;
                    if chroma_q8 != 256 {
                        cb_d = (cb_d * chroma_q8 + 128) >> 8;
                        cr_d = (cr_d * chroma_q8 + 128) >> 8;
                    }
                    (cb_d + 128, cr_d + 128)
                };
                cb_plane[c_row + pair] = cb.clamp(0, 255) as u8;
                cr_plane[c_row + pair] = cr.clamp(0, 255) as u8;
            }
        }
        if sharp != 5 {
            apply_luma_sharpness(&mut y_plane, LCD_W, LV_H, (sharp as i32) - 5);
        }

        // 6. Run FR80 spatial_driver (`0x005D1600`) + ycc_engine (`0x005D03C0`) across all 3 planes
        let mut drv = Fr80Emu::new(&self.m5_flash, M5_DELTA);
        let p_y = drv.map_sdram(M5_IMG_VA[0], y_plane.len());
        let p_cb = drv.map_sdram(M5_IMG_VA[1], cb_plane.len());
        let p_cr = drv.map_sdram(M5_IMG_VA[2], cr_plane.len());
        drv.sdram[p_y].data.copy_from_slice(&y_plane);
        drv.sdram[p_cb].data.copy_from_slice(&cb_plane);
        drv.sdram[p_cr].data.copy_from_slice(&cr_plane);
        drv.map_sdram(M5_SCRATCH_BASE, M5_SCRATCH_LEN);

        let emu_fp: u32 = 0x8008_0200;
        let emu_sp: u32 = 0x8008_0100;
        let emu_planes: u32 = 0x8009_0000;
        for (i, &va) in M5_IMG_VA.iter().enumerate() {
            drv.w32(emu_planes + (i as u32) * 4, va);
        }
        drv.r[14] = emu_fp;
        drv.r[15] = emu_sp;
        drv.w32(emu_fp + 8, 3);
        drv.w32(emu_sp, 0);
        drv.r[4..8].copy_from_slice(&[emu_planes, 0x8050_E000, 1, 1]);

        drv.set_handler(M5_SYM_GET_P1D, move |e| {
            e.r[4] = job_p1d;
            e.rp
        });
        drv.set_handler(M5_SYM_GET_CM, move |e| {
            e.r[4] = job_cm;
            e.rp
        });
        drv.set_handler(M5_SYM_GET_IMG_DIM, |e| {
            e.r[4] = if e.r[4] == 0 { LCD_W as u32 } else { LV_H as u32 };
            e.rp
        });
        drv.set_handler(M5_SYM_STRIDE_DEFAULT, |e| {
            e.r[4] = LCD_W as u32;
            e.rp
        });
        drv.set_handler(M5_SYM_HW_JPEG_ENCODE, |e| e.rp);

        // Virtual SD card handler for D:\LUT1.BIN .. D:\LUT8.BIN
        let sd_files = self.sd_luts.clone();
        let open_slot: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));
        {
            let os = Rc::clone(&open_slot);
            let files = sd_files.clone();
            drv.set_handler(M5_SYM_MDF_OPEN, move |e| {
                let digit = e.r8(e.r[4] + 6);
                let idx = digit.wrapping_sub(b'1') as usize;
                if idx < 8 && files[idx].is_some() {
                    *os.borrow_mut() = Some(idx);
                    e.r[4] = 3;
                } else {
                    *os.borrow_mut() = None;
                    e.r[4] = u32::MAX;
                }
                e.rp
            });
        }
        {
            let os = Rc::clone(&open_slot);
            drv.set_handler(M5_SYM_MDF_READ, move |e| {
                if let Some(idx) = *os.borrow()
                    && let Some(ref data) = sd_files[idx]
                {
                    let n = (e.r[6] as usize).min(data.len());
                    for (i, &b) in data[..n].iter().enumerate() {
                        e.w8(e.r[5] + i as u32, b);
                    }
                    e.w32(e.r[7], n as u32);
                    e.r[4] = 0;
                    return e.rp;
                }
                e.w32(e.r[7], 0);
                e.r[4] = u32::MAX;
                e.rp
            });
        }
        drv.set_handler(M5_SYM_MDF_CLOSE, |e| {
            e.r[4] = 0;
            e.rp
        });

        if has_m5_hooks
            && let Ok(n) = drv.run(M5_SPATIAL_DRV, 80_000_000)
        {
            m5_insns += n;
        }

        let yp = &drv.sdram[p_y].data;
        let cbp = &drv.sdram[p_cb].data;
        let crp = &drv.sdram[p_cr].data;
        let mut rgba = vec![0u8; LCD_W * LV_H * 4];
        for y in 0..LV_H {
            let y_row = y * LCD_W;
            let c_row = y * (LCD_W / 2);
            for pair in 0..(LCD_W / 2) {
                let cb = cbp[c_row + pair] as i32;
                let cr = crp[c_row + pair] as i32;
                let [r0, g0, b0] = ycbcr_to_rgb(yp[y_row + pair * 2] as i32, cb, cr);
                let [r1, g1, b1] = ycbcr_to_rgb(yp[y_row + pair * 2 + 1] as i32, cb, cr);
                let p = (y_row + pair * 2) * 4;
                rgba[p] = r0;
                rgba[p + 1] = g0;
                rgba[p + 2] = b0;
                rgba[p + 3] = 255;
                rgba[p + 4] = r1;
                rgba[p + 5] = g1;
                rgba[p + 6] = b1;
                rgba[p + 7] = 255;
            }
        }
        (rgba, m5_insns)
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

        // Predecode 256-entry OSD CLUT ([A, Y, Cb, Cr] -> [R, G, B, A_0_15])
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
        let show_pb_video = scr == 0x19 && has_captures && pb_vbuf_va >= SDRAM_BASE;
        let pb_y_off = (pb_vbuf_va.saturating_sub(SDRAM_BASE)) as usize;
        let pb_cb_off = pb_y_off + PB_PLANE_Y_LEN;
        let pb_cr_off = pb_cb_off + PB_PLANE_C_LEN;
        let show_argb_hud = scr == 0x15;

        for y in 0..LCD_H {
            let row_px = y * LCD_W;
            let osd_row = &sdram[OSD_OFF + row_px..OSD_OFF + row_px + LCD_W];
            let argb_row = &sdram[ARGB_OFF + row_px * 2..ARGB_OFF + (row_px + LCD_W) * 2];

            for x in 0..LCD_W {
                let (mut r, mut g, mut b) = if show_lv_video && y < LV_H {
                    let yv = sdram[LV_Y_OFF + row_px + x] as i32;
                    let cb = sdram[LV_CB_OFF + y * (LCD_W / 2) + (x >> 1)] as i32;
                    let cr = sdram[LV_CR_OFF + y * (LCD_W / 2) + (x >> 1)] as i32;
                    let [rr, gg, bb] = ycbcr_to_rgb(yv, cb, cr);
                    (rr as u32, gg as u32, bb as u32)
                } else if show_pb_video && pb_cr_off + PB_PLANE_C_LEN <= sdram.len() {
                    let yv = sdram[pb_y_off + row_px + x] as i32;
                    let c_idx = (y >> 1) * (LCD_W / 2) + (x >> 1);
                    let cb = sdram[pb_cb_off + c_idx] as i32;
                    let cr = sdram[pb_cr_off + c_idx] as i32;
                    let [rr, gg, bb] = ycbcr_to_rgb(yv, cb, cr);
                    (rr as u32, gg as u32, bb as u32)
                } else {
                    (0u32, 0u32, 0u32)
                };

                // Plane 1: RGBA4444 (R:12..15, G:8..11, B:4..7, A:0..3)
                if show_argb_hud {
                    let px16 = u16::from_be_bytes([argb_row[x * 2], argb_row[x * 2 + 1]]);
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
                            r = (r * inv_a + r4 * a4 + 7) / 15;
                            g = (g * inv_a + g4 * a4 + 7) / 15;
                            b = (b * inv_a + b4 * a4 + 7) / 15;
                        }
                    }
                }

                // Plane 2: 8bpp CLUT OSD
                let idx = osd_row[x] as usize;
                let ce = clut_rgba[idx];
                let oa = ce[3] as u32;
                if oa > 0 {
                    if oa >= 15 {
                        r = ce[0] as u32;
                        g = ce[1] as u32;
                        b = ce[2] as u32;
                    } else {
                        let inv_a = 15 - oa;
                        r = (r * inv_a + (ce[0] as u32) * oa + 7) / 15;
                        g = (g * inv_a + (ce[1] as u32) * oa + 7) / 15;
                        b = (b * inv_a + (ce[2] as u32) * oa + 7) / 15;
                    }
                }

                let o = (row_px + x) * 4;
                self.lcd_rgba[o] = r as u8;
                self.lcd_rgba[o + 1] = g as u8;
                self.lcd_rgba[o + 2] = b as u8;
                self.lcd_rgba[o + 3] = 255;
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

    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    pub fn cpu_mut(&mut self) -> &mut Cpu {
        &mut self.cpu
    }
}

const DMA_2D_COPY_MODE: u32 = 0xA00B_125C;
const DMA_2D_COPY: u32 = 0xA00B_14A0;
const DMA_RECT_FILL: u32 = 0xA00B_1870;
const DMA_MEM_COPY: u32 = 0xA00B_23A8;

fn install_m6m_hooks(cpu: &mut Cpu, hw: &Rc<RefCell<HwState>>) {
    // uITRON kernel + Milbeaut driver range stub; stops below ICON_TBL getters at 0xA00BD7FC..0xA00C0594
    cpu.stub_range(0xA007_0000, 0xA00B_8000, Some(0));
    // uITRON 4.0 memory pool, task-ID, and mailbox/dataqueue out-pointer syscalls
    for &va in &[0xA007_1830, 0xA007_1970, 0xA007_1C54] {
        let st = Rc::clone(hw);
        cpu.add_hook(va, move |c| {
            let p_blk = c.r[1];
            let ptr = heap_alloc(&mut st.borrow_mut(), 256);
            if ptr == 0 {
                return HookAction::Return(Some((-33i32) as u32));
            }
            let off = (ptr - SDRAM_BASE) as usize;
            c.sdram_mut()[off..off + 256].fill(0);
            if p_blk >= SDRAM_BASE {
                c.w32(p_blk, ptr);
            }
            HookAction::Return(Some(0))
        });
    }
    for &va in &[0xA007_1F30, 0xA007_212C, 0xA007_22A0] {
        let st = Rc::clone(hw);
        cpu.add_hook(va, move |c| {
            let sz = ((c.r[1] + 15) & !15).max(16);
            let p_blk = c.r[2];
            let ptr = heap_alloc(&mut st.borrow_mut(), sz);
            if ptr == 0 {
                return HookAction::Return(Some((-33i32) as u32));
            }
            let off = (ptr - SDRAM_BASE) as usize;
            c.sdram_mut()[off..off + sz as usize].fill(0);
            if p_blk >= SDRAM_BASE {
                c.w32(p_blk, ptr);
            }
            HookAction::Return(Some(0))
        });
    }
    for &va in &[0xA007_1B00, 0xA007_2388] {
        let st = Rc::clone(hw);
        cpu.add_hook(va, move |c| {
            let ptr = c.r[1];
            if (0xA0A2_0000..0xA1F0_0000).contains(&ptr) {
                let mut s = st.borrow_mut();
                if let Some(pos) = s.alloc_caps.iter().position(|&(va, _)| va == ptr) {
                    let (va, cap) = s.alloc_caps.swap_remove(pos);
                    s.free_list.push(HeapBlock { va, cap });
                }
            }
            HookAction::Return(Some(0))
        });
    }
    cpu.add_hook(0xA007_3B70, |c| {
        let p_tskid = c.r[0];
        if p_tskid >= SDRAM_BASE {
            c.w16(p_tskid, 0);
        }
        HookAction::Return(Some(0))
    });
    // uITRON ref_tsk(tskid, pk_rtsk): background slideshow/movie/audio tasks are TTS_DMT (0x10)
    cpu.add_hook(0xA007_52D4, |c| {
        let pk_rtsk = c.r[1];
        if pk_rtsk >= SDRAM_BASE {
            let off = (pk_rtsk - SDRAM_BASE) as usize;
            if off + 0x24 <= c.sdram().len() {
                c.sdram_mut()[off..off + 0x24].fill(0);
                c.w16(pk_rtsk, 0x10);
            }
        }
        HookAction::Return(Some(0))
    });
    cpu.add_hook(0xA007_2C74, |c| {
        let pk_rmbx = c.r[1];
        if pk_rmbx >= SDRAM_BASE {
            c.w32(pk_rmbx, 0);
            c.w32(pk_rmbx.wrapping_add(4), 0);
        }
        HookAction::Return(Some(0))
    });
    for &va in &[0xA007_29B8, 0xA007_3240, 0xA007_4808, 0xA007_4AC0, 0xA007_4C50] {
        cpu.stub(va, Some((-50i32) as u32));
    }
    cpu.stub(0xA010_EF48, Some(0));
    cpu.stub(0xA010_EF28, Some(0));
    cpu.stub(GET_LANG, Some(0));

    // ARM libc memory intrinsics (bit-exact with compiled ARM routines)
    for &(va, ret_r0) in &[(LIBC_MEMCPY, true), (LIBC_AEABI_MEMMOVE, false)] {
        cpu.add_hook(va, move |c| {
            let dst = c.r[0];
            let src = c.r[1];
            let len = c.r[2] as usize;
            if dst >= SDRAM_BASE && src >= SDRAM_BASE && len > 0 {
                let d = (dst - SDRAM_BASE) as usize;
                let s = (src - SDRAM_BASE) as usize;
                let sdram = c.sdram_mut();
                if d + len <= sdram.len() && s + len <= sdram.len() {
                    sdram.copy_within(s..s + len, d);
                }
            }
            HookAction::Return(if ret_r0 { Some(dst) } else { None })
        });
    }
    cpu.add_hook(LIBC_MEMSET, |c| {
        let dst = c.r[0];
        let val = c.r[1] as u8;
        let len = c.r[2] as usize;
        if dst >= SDRAM_BASE && len > 0 {
            let d = (dst - SDRAM_BASE) as usize;
            let sdram = c.sdram_mut();
            if d + len <= sdram.len() {
                sdram[d..d + len].fill(val);
            }
        }
        HookAction::Return(Some(dst))
    });
    cpu.add_hook(LIBC_AEABI_MEMSET, |c| {
        let dst = c.r[0];
        let len = c.r[1] as usize;
        let val = c.r[2] as u8;
        if dst >= SDRAM_BASE && len > 0 {
            let d = (dst - SDRAM_BASE) as usize;
            let sdram = c.sdram_mut();
            if d + len <= sdram.len() {
                sdram[d..d + len].fill(val);
            }
        }
        HookAction::Return(None)
    });
    for &va in &[LIBC_AEABI_MEMCLR, LIBC_AEABI_MEMCLR4] {
        cpu.add_hook(va, |c| {
            let dst = c.r[0];
            let len = c.r[1] as usize;
            if dst >= SDRAM_BASE && len > 0 {
                let d = (dst - SDRAM_BASE) as usize;
                let sdram = c.sdram_mut();
                if d + len <= sdram.len() {
                    sdram[d..d + len].fill(0);
                }
            }
            HookAction::Return(None)
        });
    }

    // Milbeaut 2D DMA rectangular fill: (mode=r0, fill=r1, stride=r2, dst=r3, [sp]=w_bytes, [sp+4]=rows)
    cpu.add_hook(DMA_RECT_FILL, |c| {
        let fill = c.r[1] as u8;
        let stride = c.r[2] as usize;
        let dst = c.r[3];
        let sp = c.r[13];
        let w_bytes = c.r32_be(sp) as usize;
        let rows = c.r32_be(sp.wrapping_add(4)) as usize;
        if dst >= SDRAM_BASE && w_bytes > 0 && rows > 0 && rows <= 4096 && stride <= 16384 {
            let base = (dst - SDRAM_BASE) as usize;
            let sdram = c.sdram_mut();
            for r in 0..rows {
                let off = base.saturating_add(r.saturating_mul(stride));
                if off + w_bytes <= sdram.len() {
                    sdram[off..off + w_bytes].fill(fill);
                }
            }
        }
        HookAction::Return(Some(0))
    });

    // Milbeaut 2D DMA copy with mode arg: (mode=r0, src=r1, src_stride=r2, dst_stride=r3, [sp]=dst, [sp+4]=w_bytes, [sp+8]=rows)
    cpu.add_hook(DMA_2D_COPY_MODE, |c| {
        let src = c.r[1];
        let src_stride = c.r[2] as usize;
        let dst_stride = c.r[3] as usize;
        let sp = c.r[13];
        let dst = c.r32_be(sp);
        let w_bytes = c.r32_be(sp.wrapping_add(4)) as usize;
        let rows = c.r32_be(sp.wrapping_add(8)) as usize;
        dma_copy_2d(c.sdram_mut(), src, src_stride, dst, dst_stride, w_bytes, rows);
        HookAction::Return(Some(0))
    });

    // Milbeaut 2D DMA block copy: (src=r0, src_stride=r1, dst_stride=r2, dst=r3, [sp]=w_bytes, [sp+4]=rows)
    cpu.add_hook(DMA_2D_COPY, |c| {
        let src = c.r[0];
        let src_stride = c.r[1] as usize;
        let dst_stride = c.r[2] as usize;
        let dst = c.r[3];
        let sp = c.r[13];
        let w_bytes = c.r32_be(sp) as usize;
        let rows = c.r32_be(sp.wrapping_add(4)) as usize;
        dma_copy_2d(c.sdram_mut(), src, src_stride, dst, dst_stride, w_bytes, rows);
        HookAction::Return(Some(0))
    });

    // Milbeaut 1D DMA copy: (dst=r0, src=r1, len=r2)
    cpu.add_hook(DMA_MEM_COPY, |c| {
        let dst = c.r[0];
        let src = c.r[1];
        let len = c.r[2] as usize;
        if dst >= SDRAM_BASE && src >= SDRAM_BASE && len > 0 {
            let d = (dst - SDRAM_BASE) as usize;
            let s = (src - SDRAM_BASE) as usize;
            let sdram = c.sdram_mut();
            if d + len <= sdram.len() && s + len <= sdram.len() {
                sdram.copy_within(s..s + len, d);
            }
        }
        HookAction::Return(Some(0))
    });

    // NOR record counter & sync helpers
    cpu.stub(0xA001_B020, None);
    cpu.stub(0xA001_B026, Some(0));
    cpu.stub(0xA014_3336, None);
    cpu.stub(0xA014_5650, Some(0));

    {
        let st = Rc::clone(hw);
        cpu.add_hook(NOR_READ, move |c| {
            let off = c.r[0] as usize;
            let n = c.r[1] as usize;
            let buf = c.r[2];
            let s = st.borrow();
            if off < s.nor.len() {
                let end = (off + n).min(s.nor.len());
                c.write_bytes(buf, &s.nor[off..end]);
            }
            HookAction::Return(Some(0))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(NOR_WRITE, move |c| {
            let off = c.r[0] as usize;
            let n = c.r[1] as usize;
            let buf = c.r[2];
            let mut s = st.borrow_mut();
            if off < s.nor.len() {
                let end = (off + n).min(s.nor.len());
                c.read_bytes(buf, &mut s.nor[off..end]);
            }
            HookAction::Return(Some(0))
        });
    }

    // Recycling freelist + bump allocator in [0xA0A20000, 0xA1F00000)
    {
        let st = Rc::clone(hw);
        cpu.add_hook(MALLOC, move |c| {
            let sz = ((c.r[0] + 15) & !15).max(16);
            HookAction::Return(Some(heap_alloc(&mut st.borrow_mut(), sz)))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(CALLOC, move |c| {
            let total = c.r[0].saturating_mul(c.r[1]);
            let sz = ((total + 15) & !15).max(16);
            let ptr = heap_alloc(&mut st.borrow_mut(), sz);
            if ptr != 0 {
                let off = (ptr - SDRAM_BASE) as usize;
                c.sdram_mut()[off..off + sz as usize].fill(0);
            }
            HookAction::Return(Some(ptr))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(FREE, move |c| {
            let ptr = c.r[0];
            if (0xA0A2_0000..0xA1F0_0000).contains(&ptr) {
                let mut s = st.borrow_mut();
                if let Some(pos) = s.alloc_caps.iter().position(|&(va, _)| va == ptr) {
                    let (va, cap) = s.alloc_caps.swap_remove(pos);
                    s.free_list.push(HeapBlock { va, cap });
                }
            }
            HookAction::Return(Some(0))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(REALLOC, move |c| {
            let ptr = c.r[0];
            let new_sz = c.r[1];
            if ptr == 0 {
                let sz = ((new_sz + 15) & !15).max(16);
                return HookAction::Return(Some(heap_alloc(&mut st.borrow_mut(), sz)));
            }
            if new_sz == 0 {
                let mut s = st.borrow_mut();
                if let Some(pos) = s.alloc_caps.iter().position(|&(va, _)| va == ptr) {
                    let (va, cap) = s.alloc_caps.swap_remove(pos);
                    s.free_list.push(HeapBlock { va, cap });
                }
                return HookAction::Return(Some(0));
            }
            let aligned = ((new_sz + 15) & !15).max(16);
            let mut s = st.borrow_mut();
            if let Some(pos) = s.alloc_caps.iter().position(|&(va, _)| va == ptr) {
                let old_cap = s.alloc_caps[pos].1;
                if aligned <= old_cap {
                    return HookAction::Return(Some(ptr));
                }
                let new_ptr = heap_alloc(&mut s, aligned);
                if new_ptr != 0 {
                    let src_off = (ptr - SDRAM_BASE) as usize;
                    let dst_off = (new_ptr - SDRAM_BASE) as usize;
                    c.sdram_mut().copy_within(src_off..src_off + old_cap as usize, dst_off);
                    if let Some(p2) = s.alloc_caps.iter().position(|&(va, _)| va == ptr) {
                        let (va, cap) = s.alloc_caps.swap_remove(p2);
                        s.free_list.push(HeapBlock { va, cap });
                    }
                }
                return HookAction::Return(Some(new_ptr));
            }
            HookAction::Return(Some(heap_alloc(&mut s, aligned)))
        });
    }

    // Intercept SCREEN_POST mailbox send after stock deduplication/guards
    {
        let st = Rc::clone(hw);
        cpu.add_hook(SCREEN_POST_MBOX, move |c| {
            st.borrow_mut().events.push_back((c.r[6], c.r[5]));
            c.r[7] = 0;
            HookAction::Jump(SCREEN_POST_RET | 1)
        });
    }

    {
        let st = Rc::clone(hw);
        cpu.add_hook(AE_SENSOR_EV100, move |_| {
            HookAction::Return(Some(st.borrow().scene_ev100_q16 as u32))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(LV_ACTIVE, move |_| HookAction::Return(Some(st.borrow().lv_active as u32)));
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(ACCEL_X, move |_| HookAction::Return(Some((st.borrow().accel_x as u8) as u32)));
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(ACCEL_Y, move |_| HookAction::Return(Some((st.borrow().accel_y as u8) as u32)));
    }

    // Sensor AWB/Preset CCMatrix getters: write active Q7 3x3 WB matrix at buf + 10..28
    for &va in &[GET_LV_CC7, GET_LV_CCN] {
        cpu.add_hook(va, |c| {
            let dst = c.r[0];
            let wb = wb_q7_matrix(c);
            let mut buf = [0u8; 28];
            for (k, &v) in wb.iter().enumerate() {
                let raw = ((v as i16) as u16) & 0x0FFF;
                buf[10 + k * 2..12 + k * 2].copy_from_slice(&raw.to_be_bytes());
            }
            c.write_bytes(dst, &buf);
            HookAction::Return(None)
        });
    }

    // Capture live B2Y ISP registers programmed by M6M firmware
    {
        let st = Rc::clone(hw);
        cpu.add_hook(B2Y_SET_MAT, move |c| {
            let ptr = c.r[0];
            let mut m = [0i32; 9];
            for (i, slot) in m.iter_mut().enumerate() {
                let raw = c.r16_be(ptr + (i as u32) * 2);
                *slot = if raw & 0x800 != 0 {
                    ((raw | 0xF000) as i16) as i32
                } else {
                    (raw & 0x7FF) as i32
                };
            }
            st.borrow_mut().b2y_mat = m;
            HookAction::Return(Some(0))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(B2Y_SET_MONO, move |c| {
            let ptr = c.r[0];
            let mut s = st.borrow_mut();
            s.b2y_mono_en = c.r8(ptr + 1) != 0;
            s.b2y_mono_cb = c.r8(ptr + 6) as i8;
            s.b2y_mono_cr = c.r8(ptr + 7) as i8;
            HookAction::Return(Some(0))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(B2Y_SET_CHROMA, move |c| {
            let ptr = c.r[0];
            st.borrow_mut().b2y_chroma_gain = c.r8(ptr + 0x11);
            HookAction::Return(Some(0))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(B2Y_SET_SHARP_LUT, move |c| {
            st.borrow_mut().b2y_sharp_idx = (c.r[4] & 0xFF) as u8;
            HookAction::Return(Some(0))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(B2Y_SET_GAMMA, move |c| {
            let lut_ptr = c.r[1];
            let sz = c.r[2] as usize;
            if sz >= 1024 {
                c.read_bytes(lut_ptr, &mut st.borrow_mut().b2y_gamma);
            }
            HookAction::Return(Some(0))
        });
    }

    // Native SCREEN_PB (0x19) video buffer selector + virtual DCF SD card & JPEG/EXIF pipeline
    {
        let st = Rc::clone(hw);
        cpu.add_hook(PB_SELECT_VBUF, move |c| {
            let va = match c.r[0] {
                1 => 0xA235_E600,
                2 => 0xA243_F600,
                4 => 0xA252_0600,
                8 => 0xA260_1600,
                _ => PB_VBUF_DEFAULT_VA,
            };
            st.borrow_mut().pb_vbuf_va = va;
            HookAction::Return(Some(0))
        });
    }
    cpu.stub(PB_DEC_INIT, Some(1));
    cpu.stub(PB_HW_JPEG_DEC, Some(0));
    cpu.stub(SD_MOUNT_STATUS, Some(3));
    cpu.stub(DCF_SCAN_READY, Some(1));
    cpu.stub(DCF_SYNC_CUR, Some(0));
    cpu.add_hook(SD_FREE_SPACE, |c| {
        if c.r[0] >= SDRAM_BASE {
            c.w32(c.r[0], 999);
        }
        if c.r[1] >= SDRAM_BASE {
            c.w32(c.r[1], 999);
        }
        HookAction::Return(Some(0))
    });
    for &va in &[DCF_COUNT_IMAGES, DCF_GET_TOTAL] {
        let st = Rc::clone(hw);
        cpu.add_hook(va, move |c| {
            let n = st.borrow().captures.len() as u32;
            c.w32(DCF_DIR_COUNTS, n);
            HookAction::Return(Some(n))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(PB_STEP_OR_CUR, move |c| {
            let mode = c.r[0];
            let mut s = st.borrow_mut();
            if s.captures.is_empty() {
                return HookAction::Return(Some(0));
            }
            let len = s.captures.len();
            let cur = s.dcf_cur.min(len - 1);
            s.dcf_cur = match mode {
                1 => (cur + 1) % len,
                2 => if cur == 0 { len - 1 } else { cur - 1 },
                _ => cur,
            };
            HookAction::Return(Some((s.dcf_cur + 1) as u32))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(DCF_GET_FILE_INFO, move |c| {
            let idx = c.r[0] as usize;
            let p_dir = c.r[1];
            let p_fnum = c.r[2];
            let p_ftype = c.r[3];
            let sp = c.r[13];
            let p_a1 = c.r32_be(sp);
            let p_a2 = c.r32_be(sp.wrapping_add(4));
            let s = st.borrow();
            if idx < 1 || idx > s.captures.len() {
                return HookAction::Return(Some((-1i32) as u32));
            }
            if p_dir >= SDRAM_BASE {
                c.w32(p_dir, 100);
            }
            if p_fnum >= SDRAM_BASE {
                c.w32(p_fnum, idx as u32);
            }
            if p_ftype >= SDRAM_BASE {
                c.w16(p_ftype, 1); // JPEG
            }
            if p_a1 >= SDRAM_BASE {
                c.w32(p_a1, 0);
            }
            if p_a2 >= SDRAM_BASE {
                c.w32(p_a2, 0);
            }
            HookAction::Return(Some(0))
        });
    }
    for &va in &[DCF_READ_JPEG_EXIF, DCF_READ_X3F_EXIF] {
        let st = Rc::clone(hw);
        cpu.add_hook(va, move |c| {
            let idx = c.r[0] as usize;
            let exif_main = c.r[1];
            let exif_ext = c.r[2];
            let cap = {
                let s = st.borrow();
                let Some(cap) = idx.checked_sub(1).and_then(|i| s.captures.get(i)) else {
                    return HookAction::Return(Some((-1i32) as u32));
                };
                cap.clone()
            };
            if exif_main >= SDRAM_BASE {
                populate_exif_main(c, exif_main, idx as u16, &cap);
            }
            if exif_ext >= SDRAM_BASE {
                populate_exif_ext(c, exif_ext, idx);
            }
            HookAction::Return(Some(0))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(DCF_GET_CUR, move |_| {
            let s = st.borrow();
            let cur = if s.captures.is_empty() {
                0
            } else {
                (s.dcf_cur.min(s.captures.len() - 1) + 1) as u32
            };
            HookAction::Return(Some(cur))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(DCF_SET_CUR, move |c| {
            let req = c.r[0] as usize;
            let mut s = st.borrow_mut();
            if req >= 1 && req <= s.captures.len() {
                s.dcf_cur = req - 1;
                HookAction::Return(Some(1))
            } else {
                HookAction::Return(Some(0))
            }
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(DCF_STEP_NEXT, move |_| {
            let mut s = st.borrow_mut();
            if s.captures.is_empty() {
                return HookAction::Return(Some(0));
            }
            s.dcf_cur = (s.dcf_cur + 1) % s.captures.len();
            HookAction::Return(Some((s.dcf_cur + 1) as u32))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(DCF_STEP_PREV, move |_| {
            let mut s = st.borrow_mut();
            if s.captures.is_empty() {
                return HookAction::Return(Some(0));
            }
            s.dcf_cur = if s.dcf_cur == 0 { s.captures.len() - 1 } else { s.dcf_cur - 1 };
            HookAction::Return(Some((s.dcf_cur + 1) as u32))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(DCF_GET_ENTRY, move |c| {
            let idx = c.r[0] as usize;
            let out = c.r[1];
            if out >= SDRAM_BASE {
                let d = (out - SDRAM_BASE) as usize;
                c.sdram_mut()[d..d + 0x20].fill(0);
                let s = st.borrow();
                if idx >= 1 && idx <= s.captures.len() {
                    c.w16(out + 2, 4704);
                    c.w16(out + 4, 3136);
                    c.w8(out + 0x10, 3); // FINE JPEG
                    return HookAction::Return(Some(1));
                }
            }
            HookAction::Return(Some(0))
        });
    }

    for &va in &[PB_OPEN_JPEG_A, PB_OPEN_JPEG_B, PB_OPEN_IMAGE_A, PB_OPEN_IMAGE_B] {
        let st = Rc::clone(hw);
        cpu.add_hook(va, move |c| {
            let idx = c.r[0] as usize;
            let raw_buf = c.r[1];
            let yuv_buf = c.r[2];
            let sp = c.r[13];
            let dec_ctx = c.r32_be(sp);
            let exif_main = c.r32_be(sp.wrapping_add(4));
            let exif_ext = c.r32_be(sp.wrapping_add(8));

            let cap = {
                let mut s = st.borrow_mut();
                if idx < 1 || idx > s.captures.len() {
                    return HookAction::Return(Some(0));
                }
                s.dcf_cur = idx - 1;
                s.pb_open_idx = Some(idx - 1);
                s.captures[idx - 1].clone()
            };

            if dec_ctx >= SDRAM_BASE {
                let d = (dec_ctx - SDRAM_BASE) as usize;
                c.sdram_mut()[d..d + 0x3C].fill(0);
                c.w16(dec_ctx, LCD_W as u16);
                c.w16(dec_ctx + 2, LCD_H as u16);
                c.w16(dec_ctx + 4, LCD_W as u16);
                c.w16(dec_ctx + 6, LCD_H as u16);
                c.w32(dec_ctx + 8, 2); // JPEG ready
                c.w32(dec_ctx + 0x14, 4096);
                let yuv = if yuv_buf >= SDRAM_BASE { yuv_buf } else { SCRATCH_VA };
                c.w32(dec_ctx + 0x18, yuv);
                c.w32(dec_ctx + 0x1C, yuv + PB_PLANE_Y_LEN as u32);
                c.w32(dec_ctx + 0x20, yuv + (PB_PLANE_Y_LEN + PB_PLANE_C_LEN) as u32);
                c.w32(dec_ctx + 0x24, raw_buf);
                c.w8(dec_ctx + 0x29, 1);
            }
            if exif_main >= SDRAM_BASE {
                populate_exif_main(c, exif_main, idx as u16, &cap);
            }
            if exif_ext >= SDRAM_BASE {
                populate_exif_ext(c, exif_ext, idx);
            }
            HookAction::Return(Some(1))
        });
    }

    {
        let st = Rc::clone(hw);
        cpu.add_hook(PB_DECODE_RESAMPLE, move |c| {
            let dst_va = c.r[1];
            let dst_w = c.r[2] as usize;
            let dst_h = c.r[3] as usize;
            let sp = c.r[13];
            let stride = c.r32_be(sp) as usize;
            let out_desc = c.r32_be(sp.wrapping_add(4));

            let rgba = {
                let s = st.borrow();
                let idx = s.pb_open_idx.unwrap_or(s.dcf_cur);
                s.captures.get(idx).map(|cap| cap.rgba.clone())
            };
            let Some(rgba) = rgba else {
                return HookAction::Return(Some(1));
            };
            if dst_va < SDRAM_BASE || dst_w == 0 || dst_h == 0 || stride < dst_w {
                return HookAction::Return(Some(1));
            }

            // Match yuv_desc_init (0xA0127F9E) 4:2:0 plane spacing & 3:2 vertical centering inside 4:3
            let h_plane = dst_h.max(LCD_H);
            let y_base = (dst_va - SDRAM_BASE) as usize;
            let cb_base = y_base + stride * h_plane;
            let cr_base = cb_base + (stride * h_plane) / 4;
            let c_stride = stride / 2;

            let (img_h, y_pad) = if dst_w == LCD_W && dst_h == LCD_H {
                (LV_H, (LCD_H - LV_H) / 2)
            } else {
                (dst_h & !1, 0)
            };

            let sdram = c.sdram_mut();
            if cr_base + (stride * h_plane) / 4 > sdram.len() {
                return HookAction::Return(Some(1));
            }
            sdram[y_base..y_base + stride * dst_h].fill(0);
            sdram[cb_base..cb_base + c_stride * (dst_h / 2)].fill(128);
            sdram[cr_base..cr_base + c_stride * (dst_h / 2)].fill(128);

            for py in (0..img_h).step_by(2) {
                let dy0 = y_pad + py;
                let dy1 = dy0 + 1;
                let sy0 = (py * LV_H / img_h).min(LV_H - 1);
                let sy1 = ((py + 1) * LV_H / img_h).min(LV_H - 1);
                let c_row = cb_base + (dy0 / 2) * c_stride;
                let cr_row = cr_base + (dy0 / 2) * c_stride;

                for px in (0..dst_w).step_by(2) {
                    let sx0 = (px * LCD_W / dst_w).min(LCD_W - 1);
                    let sx1 = ((px + 1) * LCD_W / dst_w).min(LCD_W - 1);
                    let coords = [(sx0, sy0), (sx1, sy0), (sx0, sy1), (sx1, sy1)];
                    let mut rs = 0i32;
                    let mut gs = 0i32;
                    let mut bs = 0i32;
                    let mut ys = [0u8; 4];
                    for (k, &(sx, sy)) in coords.iter().enumerate() {
                        let p = (sy * LCD_W + sx) * 4;
                        let r = rgba[p] as i32;
                        let g = rgba[p + 1] as i32;
                        let b = rgba[p + 2] as i32;
                        ys[k] = ((77 * r + 150 * g + 29 * b + 128) >> 8).clamp(0, 255) as u8;
                        rs += r;
                        gs += g;
                        bs += b;
                    }
                    sdram[y_base + dy0 * stride + px] = ys[0];
                    if px + 1 < dst_w {
                        sdram[y_base + dy0 * stride + px + 1] = ys[1];
                    }
                    if dy1 < dst_h {
                        sdram[y_base + dy1 * stride + px] = ys[2];
                        if px + 1 < dst_w {
                            sdram[y_base + dy1 * stride + px + 1] = ys[3];
                        }
                    }
                    let r_avg = (rs + 2) >> 2;
                    let g_avg = (gs + 2) >> 2;
                    let b_avg = (bs + 2) >> 2;
                    let cb = (((-43 * r_avg - 85 * g_avg + 128 * b_avg + 128) >> 8) + 128).clamp(0, 255) as u8;
                    let cr = (((128 * r_avg - 107 * g_avg - 21 * b_avg + 128) >> 8) + 128).clamp(0, 255) as u8;
                    sdram[c_row + px / 2] = cb;
                    sdram[cr_row + px / 2] = cr;
                }
            }

            if out_desc >= SDRAM_BASE {
                c.w16(out_desc, dst_w as u16);
                c.w16(out_desc + 2, dst_h as u16);
                c.w16(out_desc + 4, stride as u16);
                c.w16(out_desc + 6, ((dst_w + 15) & !15) as u16);
                c.w32(out_desc + 8, dst_va);
                c.w32(out_desc + 0x0C, SDRAM_BASE + cb_base as u32);
                c.w32(out_desc + 0x10, SDRAM_BASE + cr_base as u32);
            }
            HookAction::Return(Some(0))
        });
    }

    // Milbeaut IIP YCbCr420 -> planar RGB888 converter (`0xA0054AB2`) for SCREEN_PB R/G/B histograms
    cpu.add_hook(PB_YCBCR_TO_RGB_PLANAR, |c| {
        let y_va = c.r[0];
        let cb_va = c.r[1];
        let cr_va = c.r[2];
        let w = c.r[3] as usize;
        let sp = c.r[13];
        let h = c.r32_be(sp) as usize;
        let plane_step = c.r32_be(sp.wrapping_add(4)) as usize;
        let rgb_va = c.r32_be(sp.wrapping_add(8));
        if y_va < SDRAM_BASE || cb_va < SDRAM_BASE || cr_va < SDRAM_BASE || rgb_va < SDRAM_BASE || w == 0 || h == 0 {
            return HookAction::Return(Some(0));
        }
        let y_off = (y_va - SDRAM_BASE) as usize;
        let cb_off = (cb_va - SDRAM_BASE) as usize;
        let cr_off = (cr_va - SDRAM_BASE) as usize;
        let r_off = (rgb_va - SDRAM_BASE) as usize;
        let g_off = r_off.saturating_add(plane_step);
        let b_off = g_off.saturating_add(plane_step);
        let cw = w / 2;
        let sdram = c.sdram_mut();
        if y_off + w * h > sdram.len()
            || cb_off + cw * (h / 2) > sdram.len()
            || cr_off + cw * (h / 2) > sdram.len()
            || b_off + w * h > sdram.len()
        {
            return HookAction::Return(Some(0));
        }
        for ry in 0..h {
            let cy = ry / 2;
            for rx in 0..w {
                let idx = ry * w + rx;
                let cidx = cy * cw + rx / 2;
                let [r, g, b] = ycbcr_to_rgb(
                    sdram[y_off + idx] as i32,
                    sdram[cb_off + cidx] as i32,
                    sdram[cr_off + cidx] as i32,
                );
                sdram[r_off + idx] = r;
                sdram[g_off + idx] = g;
                sdram[b_off + idx] = b;
            }
        }
        HookAction::Return(Some(0))
    });

    // Milbeaut IIP 256-bin byte-plane histogram engine (`0xA004A7AC` -> `0xA00B1D60`)
    cpu.add_hook(PB_IIP_HIST, |c| {
        let src_va = c.r[0];
        let w = c.r[1] as usize;
        let h = c.r[2] as usize;
        let hist_va = c.r[3];
        let out_flag_va = c.r32_be(c.r[13]);
        if src_va < SDRAM_BASE || hist_va < SDRAM_BASE || w == 0 || h == 0 {
            return HookAction::Return(Some(0));
        }
        let s_off = (src_va - SDRAM_BASE) as usize;
        let len = w.saturating_mul(h);
        let mut bins = [0u32; 256];
        {
            let sdram = c.sdram();
            if s_off + len > sdram.len() {
                return HookAction::Return(Some(0));
            }
            for &px in &sdram[s_off..s_off + len] {
                bins[px as usize] += 1;
            }
        }
        let h_off = (hist_va - SDRAM_BASE) as usize;
        if h_off + 1024 <= c.sdram().len() {
            for (i, &cnt) in bins.iter().enumerate() {
                c.w32(hist_va + (i as u32) * 4, cnt);
            }
        }
        if out_flag_va >= SDRAM_BASE {
            c.w8(out_flag_va, 0);
        }
        HookAction::Return(Some(0))
    });
}

fn populate_exif_ext(c: &mut Cpu, exif_ext: u32, idx: usize) {
    let e = (exif_ext - SDRAM_BASE) as usize;
    c.sdram_mut()[e..e + 0x44].fill(0);
    c.w32(exif_ext + 8, 2);
    c.w8(exif_ext + 0x1C, 1);
    c.w8(exif_ext + 0x39, 4);
    c.w8(exif_ext + 0x3A, 18);
    c.w8(exif_ext + 0x3B, 12);
    c.w8(exif_ext + 0x3C, (idx / 60) as u8);
    c.w8(exif_ext + 0x3D, (idx % 60) as u8);
    c.w16(exif_ext + 0x3E, 2026);
}

fn populate_exif_main(c: &mut Cpu, base: u32, fnum: u16, cap: &CaptureFrame) {
    let off = (base - SDRAM_BASE) as usize;
    c.sdram_mut()[off..off + 0x188].fill(0);
    c.w16(base + 0x02, 100); // DCF directory number (100SIGMA -> 100-xxxx)
    c.w16(base + 0x04, fnum);
    c.w8(base + 0x10, 0); // Unprotected
    c.w8(base + 0x1C, 1); // Orientation: horizontal normal
    c.w16(base + 0x1E, 4704);
    c.w16(base + 0x20, 3136);
    c.w16(base + 0x22, cap.iso);
    c.w16(base + 0x24, 5); // Evaluative metering
    c.w32(base + 0x28, cap.tv_num);
    c.w32(base + 0x2C, cap.tv_den);
    c.w32(base + 0x30, cap.av_num);
    c.w32(base + 0x34, cap.av_den);
    c.w16(base + 0x38, 1); // rational EV comp at +0x170/+0x174
    c.w16(base + 0x3A, cap.expo_prog as u16);
    c.w16(base + 0x3E, 0); // Flash off
    c.w16(base + 0x48, 2026);
    c.w8(base + 0x4A, 4);
    c.w8(base + 0x4B, 18);
    c.w8(base + 0x4C, 12);
    c.w8(base + 0x4D, (fnum / 60) as u8);
    c.w8(base + 0x4E, (fnum % 60) as u8);
    c.write_bytes(base + 0x5C, b"SIGMA\0");
    c.write_bytes(base + 0x68, b"SIGMA DP2 Merrill\0");
    c.w32(base + 0x90, cap.ev_steps as u32);
    c.w32(base + 0x94, 3);
    c.w8(base + 0x112, cap.wb);
    c.w8(base + 0x113, 1); // AF
    c.write_bytes(base + 0x114, b"30\0");
    c.w8(base + 0x124, 3); // FINE
    write_f64_be(c, base + 0x128, ((cap.sharp as f64) - 5.0) * 0.2);
    write_f64_be(c, base + 0x130, ((cap.contrast as f64) - 5.0) * 0.2);
    write_f64_be(c, base + 0x138, ((cap.sat as f64) - 5.0) * 0.2);
    c.w8(base + 0x140, 1); // JPEG
    c.w8(base + 0x143, 3); // Single drive
    c.w8(base + 0x156, cap.cs.saturating_sub(1));
    c.w8(base + 0x157, cap.decoded_cm);
    write_f64_be(c, base + 0x160, 30.0); // 30mm focal length
    c.w32(base + 0x170, cap.ev_steps as u32);
    c.w32(base + 0x174, 3);
    // Center AF box on thumbnail (scaled inside 640x424)
    c.w16(base + 0x17C, 292);
    c.w16(base + 0x17E, 193);
    c.w16(base + 0x180, 56);
    c.w16(base + 0x182, 38);
    c.w32(base + 0x184, 4_820_000);
}

fn write_f64_be(c: &mut Cpu, va: u32, val: f64) {
    let bits = val.to_bits();
    c.w32(va, (bits >> 32) as u32);
    c.w32(va + 4, bits as u32);
}

fn clear_yuv420_buf(sdram: &mut [u8], vbuf_va: u32) {
    if vbuf_va < SDRAM_BASE {
        return;
    }
    let y_off = (vbuf_va - SDRAM_BASE) as usize;
    let cb_off = y_off + PB_PLANE_Y_LEN;
    let end = cb_off + PB_PLANE_C_LEN * 2;
    if end <= sdram.len() {
        sdram[y_off..cb_off].fill(0);
        sdram[cb_off..end].fill(128);
    }
}

fn dma_copy_2d(
    sdram: &mut [u8],
    src: u32,
    src_stride: usize,
    dst: u32,
    dst_stride: usize,
    w_bytes: usize,
    rows: usize,
) {
    if src < SDRAM_BASE || dst < SDRAM_BASE || w_bytes == 0 || rows == 0 || rows > 4096 {
        return;
    }
    let s_base = (src - SDRAM_BASE) as usize;
    let d_base = (dst - SDRAM_BASE) as usize;
    for r in 0..rows {
        let s_off = s_base.saturating_add(r.saturating_mul(src_stride));
        let d_off = d_base.saturating_add(r.saturating_mul(dst_stride));
        if s_off + w_bytes <= sdram.len() && d_off + w_bytes <= sdram.len() {
            sdram.copy_within(s_off..s_off + w_bytes, d_off);
        }
    }
}

fn heap_alloc(st: &mut HwState, sz: u32) -> u32 {
    if let Some(pos) = st.free_list.iter().position(|b| b.cap >= sz && b.cap <= sz * 2) {
        let blk = st.free_list.swap_remove(pos);
        st.alloc_caps.push((blk.va, blk.cap));
        return blk.va;
    }
    if st.heap_bump + sz > 0xA1F0_0000 {
        return 0;
    }
    let va = st.heap_bump;
    st.heap_bump += sz;
    st.alloc_caps.push((va, sz));
    va
}

/// Returns a signed 12-bit Q7 (`128 == 1.0`) 3x3 white-balance matrix from M6M NVRAM WB settings.
/// WB == 1 (Auto) returns exact identity `[128, 0, 0, 0, 128, 0, 0, 0, 128]`.
fn wb_q7_matrix(c: &Cpu) -> [i32; 9] {
    let wb = c.r8(0xA093_96C4);
    let (rg, gg, bg) = match wb {
        2 => (134, 128, 122), // Daylight (~5400K)
        3 => (154, 128, 100), // Shade (~7500K)
        4 => (144, 128, 110), // Overcast (~6500K)
        5 => (90, 128, 176),  // Incandescent (~3000K)
        6 => (114, 124, 148), // Fluorescent (~4200K + magenta comp)
        7 => (138, 128, 116), // Flash (~5800K)
        8..=10 => (130, 128, 126),
        11 => {
            // Color Temp: 0xA09396DC steps (~2500K..10000K)
            let step = c.r8(0xA093_96DC).min(75) as i32;
            let r = (86 + step * 80 / 75).clamp(72, 180);
            let b = (184 - step * 92 / 75).clamp(80, 196);
            (r, 128, b)
        }
        _ => (128, 128, 128), // Auto (1)
    };
    [rg, 0, 0, 0, gg, 0, 0, 0, bg]
}

fn sv_to_iso_speed(sv: f32) -> u16 {
    const ISOS: [u16; 19] = [
        100, 125, 160, 200, 250, 320, 400, 500, 640, 800, 1000, 1250, 1600, 2000, 2500, 3200,
        4000, 5000, 6400,
    ];
    let step = ((sv - 5.0) * 3.0).round().clamp(0.0, (ISOS.len() - 1) as f32) as usize;
    ISOS[step]
}

fn tv_idx_rational(tv_idx: u16) -> (u32, u32) {
    const TABLE: [(u32, u32); 49] = [
        (30, 1), (25, 1), (20, 1), (15, 1), (13, 1), (10, 1), (8, 1), (6, 1), (5, 1),
        (4, 1), (32, 10), (25, 10), (2, 1), (16, 10), (13, 10), (1, 1), (8, 10), (6, 10),
        (1, 2), (4, 10), (3, 10), (1, 4), (1, 5), (1, 6), (1, 8), (1, 10), (1, 13),
        (1, 15), (1, 20), (1, 25), (1, 30), (1, 40), (1, 50), (1, 60), (1, 80), (1, 100),
        (1, 125), (1, 160), (1, 200), (1, 250), (1, 320), (1, 400), (1, 500), (1, 640),
        (1, 800), (1, 1000), (1, 1250), (1, 1600), (1, 2000),
    ];
    TABLE[tv_idx.min(48) as usize]
}

fn av_idx_rational(av_idx: u16) -> (u32, u32) {
    const F10: [u32; 16] = [28, 32, 35, 40, 45, 50, 56, 63, 71, 80, 90, 100, 110, 130, 140, 160];
    (F10[av_idx.min(15) as usize], 10)
}

#[inline]
fn apply_contrast_u8(v: i32, slope_q8: i32) -> i32 {
    (((v - 128) * slope_q8 + 128) >> 8).clamp(-128, 127) + 128
}

fn apply_luma_sharpness(y_plane: &mut [u8], w: usize, h: usize, delta_steps: i32) {
    if delta_steps == 0 || y_plane.len() < w * h {
        return;
    }
    let src = y_plane[..w * h].to_vec();
    let amt = delta_steps * 28; // Q8 unsharp / soften weight
    for y in 1..h - 1 {
        let row = y * w;
        for x in 1..w - 1 {
            let i = row + x;
            let c = src[i] as i32;
            let blur = (src[i - w] as i32 + src[i + w] as i32 + src[i - 1] as i32 + src[i + 1] as i32 + 2) >> 2;
            y_plane[i] = (c + (((c - blur) * amt + 128) >> 8)).clamp(0, 255) as u8;
        }
    }
}

#[inline]
fn sensor_noise_rgb(seed: u32, amp: i32) -> [i32; 3] {
    let mut h = seed.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    h ^= h >> 16;
    h = h.wrapping_mul(0xC2B2_AE35);
    let n0 = ((h & 0xFF) as i32) - 128;
    let n1 = (((h >> 8) & 0xFF) as i32) - 128;
    let n2 = (((h >> 16) & 0xFF) as i32) - 128;
    [(n0 * amp) >> 7, (n1 * amp) >> 7, (n2 * amp) >> 7]
}

fn mean_scene_luma(scene: &[[u8; 3]]) -> f32 {
    if scene.is_empty() {
        return 128.0;
    }
    let mut sum = 0u64;
    for &[r, g, b] in scene {
        sum += (77 * (r as u64) + 150 * (g as u64) + 29 * (b as u64)) >> 8;
    }
    (sum as f32) / (scene.len() as f32)
}

#[inline]
fn ycbcr_to_rgb(y: i32, cb: i32, cr: i32) -> [u8; 3] {
    let dc = cb - 128;
    let dr = cr - 128;
    let r = y + ((359 * dr + 128) >> 8);
    let g = y - ((88 * dc + 183 * dr + 128) >> 8);
    let b = y + ((454 * dc + 128) >> 8);
    [r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8]
}

fn invert_monotonic_lut(lut: &[u8], max_domain: u16) -> [u16; 256] {
    let mut inv = [0u16; 256];
    let mut k = 0usize;
    for (target, slot) in inv.iter_mut().enumerate() {
        while k + 1 < lut.len() && (lut[k] as usize) < target {
            k += 1;
        }
        *slot = (k as u16).min(max_domain);
    }
    inv
}

/// Maps linear `(r, g, b)` to the 12-sector MCAS hue wheel (`0..11`, 30° per sector).
#[inline]
fn hue_sector_12(r: i32, g: i32, b: i32) -> usize {
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let d = mx - mn;
    if d <= 8 {
        return 0;
    }
    let h2 = if mx == r {
        let mut t = 2 * (g - b) / d;
        if t < 0 {
            t += 12;
        }
        t
    } else if mx == g {
        4 + 2 * (b - r) / d
    } else {
        8 + 2 * (r - g) / d
    };
    (h2.rem_euclid(12)) as usize
}

fn read_cstr(cpu: &Cpu, va: u32, max_len: usize) -> String {
    let mut bytes = Vec::with_capacity(max_len);
    for i in 0..max_len {
        let b = cpu.r8(va + i as u32);
        if b == 0 {
            break;
        }
        bytes.push(b);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn stock_cm_name(cm: u8) -> &'static str {
    match cm {
        1 => "Monochrome",
        2 => "Sepia",
        3 => "Standard",
        4 => "Vivid",
        5 => "Neutral",
        6 => "Portrait",
        7 => "Landscape",
        8 => "FOV Classic Blue",
        _ => "Standard",
    }
}

/// Built-in daylight photographic test scene (sky gradient, foliage, warm brick/portrait patches,
/// specular highlight bulb for halation/zebras, and crisp Siemens star edges for focus peaking).
fn default_scene_srgb() -> Vec<[u8; 3]> {
    const PATCHES: [[u8; 3]; 12] = [
        [115, 82, 68],   // dark skin
        [194, 150, 130], // light skin
        [98, 122, 157],  // blue sky
        [87, 108, 67],   // foliage
        [193, 90, 99],   // moderate red
        [224, 124, 44],  // orange
        [70, 91, 166],   // purplish blue
        [175, 54, 60],   // crimson
        [231, 199, 31],  // yellow
        [187, 86, 149],  // magenta
        [8, 133, 161],   // cyan
        [243, 243, 242], // white
    ];

    let mut out = vec![[0u8; 3]; LCD_W * LV_H];
    for y in 0..LV_H {
        for x in 0..LCD_W {
            let rgb = if y < 190 {
                let t = (y as u32 * 255) / 190;
                let mut r = (54 + (t * 95) / 255) as i32;
                let mut g = (108 + (t * 88) / 255) as i32;
                let mut b = (186 + (t * 48) / 255) as i32;
                let dx = x as i32 - 490;
                let dy = y as i32 - 74;
                let d2 = dx * dx + dy * dy;
                if d2 < 26 * 26 {
                    r = 254;
                    g = 250;
                    b = 236;
                } else if d2 < 64 * 64 {
                    let glow = ((64 * 64 - d2) * 80) / (64 * 64);
                    r = (r + glow).min(255);
                    g = (g + (glow * 3) / 4).min(255);
                    b = (b + glow / 3).min(255);
                }
                [r as u8, g as u8, b as u8]
            } else if y < 310 {
                let dx = x as i32 - 160;
                let dy = y as i32 - 250;
                let d2 = dx * dx + dy * dy;
                if (16..=48 * 48).contains(&d2) {
                    let oct = ((dx.abs() > dy.abs()) as u8)
                        ^ ((dx > 0) as u8)
                        ^ ((dy > 0) as u8)
                        ^ (((dx.abs() * 5) > (dy.abs() * 12)) as u8)
                        ^ (((dy.abs() * 5) > (dx.abs() * 12)) as u8);
                    if oct & 1 != 0 {
                        [238, 236, 230]
                    } else {
                        [26, 28, 32]
                    }
                } else {
                    let wave = ((x * 7 + y * 13) % 29) as u8;
                    [58 + wave / 2, 96 + wave, 52 + wave / 3]
                }
            } else {
                let idx = (x * 12 / LCD_W).min(11);
                let border = (x % (LCD_W / 12) < 3) || !(314..=418).contains(&y);
                if border {
                    [28, 28, 30]
                } else {
                    PATCHES[idx]
                }
            };
            out[y * LCD_W + x] = rgb;
        }
    }
    out
}
