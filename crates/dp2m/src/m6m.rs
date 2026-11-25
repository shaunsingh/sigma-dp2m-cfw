//! m6m (arm926ej-s BE, `C78FRM.bin`) fw patcher & binary invariant verifier

use std::collections::HashMap;
use std::path::Path;

use crate::arm_elf::{assemble_thumb_sections, compile_thumb_c, thumb_bl, thumb_blx_arm};
use crate::config::Config;
use crate::pack::{norm_lv_cmcm, pack_lv_18_knots, FilmPack};

pub const C78_S3_OFF: usize = 0x010A_4100;
pub const C78_LEN: usize = 0x0082_0000;
const S3_IN_C78: usize = 0x0002_0000;
const VA_BASE: u32 = 0xA000_0000;

// protected ranges (never touch)
const FPGA_START: u32 = 0xA017_9000;
const FPGA_END: u32 = 0xA023_D35A;
const BSS_START: u32 = 0xA07F_BC1C;

// zero-ref factory caves
const CAVE1_START: u32 = 0xA005_A400;
const CAVE1_END: u32 = 0xA005_BE38;
const CAVE2S_START: u32 = 0xA005_BE38;
const CAVE2S_END: u32 = 0xA005_CDF4;
const FACMENU_START: u32 = 0xA07E_C9CC;
const FACMENU_END: u32 = 0xA07E_D82C;

const VA_SAFE_RET0: u32 = 0xA005_A400;
const VA_ISO50_HOOK_1: u32 = 0xA005_A408;
const VA_ISO50_HOOK_2: u32 = 0xA005_A41C;
const VA_STR_HOOK: u32 = 0xA005_A430;
const VA_LV_MONO_HOOK: u32 = 0xA005_A464;
const VA_OSD_ICON_HOOK: u32 = 0xA005_A488;
const VA_LV_CMCM_TRAMP: u32 = 0xA005_A4B4;
const VA_LV_GAMMA_WRAP: u32 = 0xA005_A4D4;
const VA_GET_MENU_HOOK: u32 = 0xA005_A4E8;
const VA_LV_YCAP_HOOK: u32 = 0xA005_A5A4;
const VA_IPC_CMD_HOOK: u32 = 0xA005_A5D0;
const VA_GET_PAPER: u32 = 0xA005_A670;
const VA_SET_PAPER: u32 = 0xA005_A67C;
const VA_GET_FX: u32 = 0xA005_A694;
const VA_SET_FX: u32 = 0xA005_A6A0;
const VA_GET_FILL: u32 = 0xA005_A6AC;
const VA_SET_FILL: u32 = 0xA005_A6B8;
const VA_GET_MINSH: u32 = 0xA005_A6C4;
const VA_SET_MINSH: u32 = 0xA005_A6D0;
const VA_GET_PEAKC: u32 = 0xA005_A6DC;
const VA_SET_PEAKC: u32 = 0xA005_A6E8;
const VA_BKT5_HOOK: u32 = 0xA005_A6F4;
const VA_AUTOISO_A_HOOK: u32 = 0xA005_A768;
const VA_AUTOISO_P_HOOK: u32 = 0xA005_A790;
const VA_LV_CMCM_C: u32 = 0xA005_A7C0;
const VA_LV_GAMMA_C: u32 = 0xA005_A900;
const VA_GRID_WGET_C: u32 = 0xA005_AA60;
const VA_GRID_DRAW_TRAMP: u32 = 0xA005_ABA0;
const VA_GRID_DRAW_C: u32 = 0xA005_ABD0;
const VA_AUTO_PAPER_TBL: u32 = 0xA005_B7D0;
const VA_MONO_Q7_TBL: u32 = 0xA005_B7E0;
const VA_TV_MIN_TBL: u32 = 0xA005_B844;
const VA_STR_PTRS_83: u32 = 0xA005_B860;
const VA_STR_TEXT_1: u32 = 0xA005_B9B0;

// Cave 2S placements
const VA_CMCM_PACKED_30: u32 = 0xA005_BE40;
const VA_LV_LUTS_20: u32 = 0xA005_C060;
const VA_LV_LUTS_BW_5: u32 = 0xA005_C4A0;
const VA_COLOR_MENU_44: u32 = 0xA005_C500;
const VA_GRID_MENU_17: u32 = 0xA005_CAA0;
const VA_STR_TEXT_2: u32 = 0xA005_CCE0;

// Factory service menu cave placements
const VA_ISO_MENU_2: u32 = 0xA07E_C9CC;
const VA_ISO_MENU_1: u32 = 0xA07E_CC8C;
const VA_PAPER_MENU: u32 = 0xA07E_CDCC;
const VA_MINSHUTTER_MENU: u32 = 0xA07E_CF0C;
const VA_SHOOT_P3_NEW: u32 = 0xA07E_D00C;
const VA_SETUP_P2_V5: u32 = 0xA07E_D0EC;
const VA_BKTORD_MENU_6: u32 = 0xA07E_D1CC;
const VA_FILMFX_MENU: u32 = 0xA07E_D2AC;
const VA_SETUP_P5_NEW: u32 = 0xA07E_D36C;
const VA_PEAKCOL_MENU: u32 = 0xA07E_D40C;
const VA_X3FILL_MENU: u32 = 0xA07E_D4AC;
const VA_ISO_AUTO_3: u32 = 0xA07E_D54C;

// Stock S3 addresses
const SYM_ROM_NVRAM_DEF: u32 = 0xA07F_9E68;
const SYM_UI_STR_HOOKPT: u32 = 0xA004_4AAE;
const SYM_COLOR_MENU: u32 = 0xA07E_BFEC;
const SYM_ISO_MENU_1: u32 = 0xA07E_AC8C;
const SYM_ISO_MENU_2: u32 = 0xA07E_ADAC;
const SYM_ISO_MENU_RF: u32 = 0xA004_D3DC;
const SYM_ISO_CLAMP_1: u32 = 0xA003_F2CC;
const SYM_ISO_CLAMP_2: u32 = 0xA003_F30A;
const SYM_GET_CM: u32 = 0xA016_7926;
const SYM_LV_GAMMA_CALL: u32 = 0xA003_9890;
const SYM_B2Y_SET_GAMMA: u32 = 0xA009_D940;
const SYM_LV_CMCM_CALL: u32 = 0xA003_A20A;
const SYM_LV_CMCM_FN: u32 = 0xA003_9CE2;
const SYM_LV_MONO_CALL: u32 = 0xA003_A1E6;
const SYM_LV_MONO_FN: u32 = 0xA003_989C;
const SYM_OSD_ICON_CALL: u32 = 0xA016_BC4E;
const SYM_GRID_MENU: u32 = 0xA07E_EFCC;
const SYM_GRID_MENU_REF: u32 = 0xA004_D3D0;
const SYM_GET_GRID: u32 = 0xA016_7F98;
const SYM_GRID_DRAW_FN: u32 = 0xA004_C3EC;
const SYM_GRID_DRAW_C1: u32 = 0xA016_C708;
const SYM_GRID_DRAW_C2: u32 = 0xA004_C5AE;
const SYM_WIDGET14_GET: u32 = 0xA07F_A4A4;
const SYM_GET_MENU_TBL: u32 = 0xA004_CD08;
const SYM_SHOOT_P3: u32 = 0xA07E_A6EC;
const SYM_SETUP_P2: u32 = 0xA07E_AA0C;
const SYM_SETUP_P5: u32 = 0xA07E_AC2C;
const SYM_ISO_AUTO_MENU: u32 = 0xA07F_188C;
const SYM_BKTORD_MENU: u32 = 0xA07E_DF8C;
const SYM_LV_YCAP_FN: u32 = 0xA003_3F1A;
const SYM_GET_BKT_EV_Q16: u32 = 0xA000_2FE4;
const SYM_AE_AUTOISO_A: u32 = 0xA000_D83E;
const SYM_AE_AUTOISO_P1: u32 = 0xA000_D9A2;
const SYM_AE_AUTOISO_P2: u32 = 0xA000_D9E8;

// (mode_id, ui_string_id, label, is_mono, fennel_slug)
const COLOR_NEW: &[(u32, u32, &str, bool, &str)] = &[
    (9,  672, "FOV Classic Yellow", false, "fov-classic-yellow"),
    (10, 673, "Cinema",             false, "cinema"),
    (11, 674, "Sunset Red",         false, "sunset-red"),
    (12, 675, "Forest Green",       false, "forest-green"),
    (13, 676, "Teal & Orange",      false, "teal-and-orange"),
    (14, 677, "True Mono",          true,  "true-mono"),
    (15, 678, "Mono Yellow #8",     true,  "mono-yellow-8"),
    (16, 679, "Mono Orange #21",    true,  "mono-orange-21"),
    (17, 680, "Mono Red #25",       true,  "mono-red-25"),
    (18, 681, "Mono Green #11",     true,  "mono-green-11"),
    (19, 682, "Velvia 100",         false, "velvia-100"),
    (20, 683, "Provia 100F",        false, "provia-100f"),
    (21, 684, "Ektachrome 100",     false, "ektachrome-100"),
    (22, 685, "Kodachrome 64",      false, "kodachrome-64"),
    (23, 686, "Portra 160",         false, "portra-160"),
    (24, 687, "Portra 400",         false, "portra-400"),
    (25, 688, "Portra 800",         false, "portra-800"),
    (26, 689, "Portra 800 +1",      false, "portra-800-p1"),
    (27, 690, "Portra 800 +2",      false, "portra-800-p2"),
    (28, 691, "Gold 200",           false, "gold-200"),
    (29, 692, "UltraMax 400",       false, "ultramax-400"),
    (30, 693, "Ektar 100",          false, "ektar-100"),
    (31, 694, "Verita 200D",        false, "verita-200d"),
    (32, 695, "Vision3 50D",        false, "vision3-50d"),
    (33, 696, "Vision3 250D",       false, "vision3-250d"),
    (34, 697, "Vision3 200T",       false, "vision3-200t"),
    (35, 698, "Vision3 500T",       false, "vision3-500t"),
    (36, 699, "Pro 400H",           false, "pro-400h"),
    (37, 700, "Superia X-TRA",      false, "superia-xtra"),
    (38, 701, "Fujicolor C200",     false, "fujicolor-c200"),
    (39, 726, "Kodak Tri-X 400",    true,  "kodak-trix-400"),
    (40, 727, "Ilford HP5 Plus",    true,  "ilford-hp5-plus"),
    (41, 728, "Fuji Acros 100",     true,  "fuji-acros-100"),
    (42, 729, "Kodak T-Max 100",    true,  "kodak-tmax-100"),
    (43, 730, "Rollei Retro 80S",   true,  "rollei-retro-80s"),
    (44, 731, "SD Custom LUT",      false, "sd-custom-lut"),
];

// (grid_id, ui_string_id, label, fennel_slug)
const GRID_NEW: &[(u32, u32, &str, &str)] = &[
    (7,  702, "Electronic Level",     "level"),
    (8,  703, "21:9 XPan + Level",    "xpan-21x9"),
    (9,  704, "16:9 Cinema + Level",  "cinema-16x9"),
    (10, 705, "1:1 Square + Level",   "square-1x1"),
    (11, 706, "Focus Peaking",        "peaking"),
    (12, 707, "Peaking + Level",      "peaking-level"),
    (13, 708, "Zebras (95% IRE)",     "zebras"),
    (14, 725, "RGB Hist + Level",     "rgb-hist"),
    (15, 737, "7:6 + Level",          "frame-7x6"),
    (16, 738, "4:3 + Level",          "frame-4x3"),
];

const PAPER_META: &[(u32, u32, &str)] = &[
    (0, 709, "Auto (Authentic)"),
    (1, 710, "Endura Premier"),
    (2, 711, "Ektacolor Edge"),
    (3, 712, "Supra Endura"),
    (4, 713, "Portra Endura"),
    (5, 714, "Crystal Archive"),
    (6, 715, "Kodak 2383"),
    (7, 716, "Kodak 2393"),
    (8, 717, "Ultra Endura"),
];

const FX_META: &[(u32, u32, &str)] = &[
    (0, 718, "Off (HW Fast)"),
    (1, 719, "3D Spectral LUT"),
    (2, 720, "3D + Film Grain"),
    (3, 721, "3D + Halation"),
    (4, 722, "Full Analog"),
];

const X3FILL_META: &[(u32, u32, &str)] = &[
    (0, 732, "Off"),
    (1, 733, "Fill +0.5 (Subtle)"),
    (2, 734, "Fill +1.0 (HDR)"),
    (3, 735, "Fill -0.5 (Contrast)"),
];

const BKT5_META: &[(u32, u32, &str)] = &[
    (4, 739, "5-Shot: 0 / - / +"),
    (6, 740, "5-Shot: - / 0 / +"),
    (7, 741, "5-Shot: + / 0 / -"),
];

const MINSHUTTER_META: &[(u32, u32, &str)] = &[
    (0, 742, "Auto (1/50s)"),
    (1, 743, "1/15s (Slow)"),
    (2, 744, "1/30s"),
    (3, 745, "1/60s"),
    (4, 746, "1/125s"),
    (5, 747, "1/250s"),
    (6, 748, "1/500s (Fast)"),
];

const PEAKCOL_META: &[(u32, u32, &str)] = &[
    (0, 749, "Red"),
    (1, 750, "Yellow"),
    (2, 751, "White"),
    (3, 752, "Black"),
];

fn s3_off(va: u32) -> usize {
    S3_IN_C78 + (va - VA_BASE) as usize
}

fn read_s3(c78: &[u8], va: u32, n: usize) -> &[u8] {
    let o = s3_off(va);
    &c78[o..o + n]
}

fn patch_s3(c78: &mut [u8], va: u32, data: &[u8]) {
    let o = s3_off(va);
    c78[o..o + data.len()].copy_from_slice(data);
}

fn pack_u32_be(words: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(words.len() * 4);
    for &w in words {
        out.extend_from_slice(&w.to_be_bytes());
    }
    out
}

fn unpack_u32_be(b: &[u8]) -> Vec<u32> {
    b.chunks_exact(4)
        .map(|c| u32::from_be_bytes(c.try_into().unwrap()))
        .collect()
}

pub fn build_m6m(
    fw_raw: &[u8],
    pack: &FilmPack,
    asm_dir: &Path,
    c_dir: &Path,
    cfg: &Config,
) -> Result<Vec<u8>, String> {
    if fw_raw.len() < C78_S3_OFF + C78_LEN {
        return Err("DP2MV105.bin too small for M6M C78FRM section".into());
    }
    let c78_stock = &fw_raw[C78_S3_OFF..C78_S3_OFF + C78_LEN];
    if &c78_stock[0..8] != b"FUJITSU!" {
        return Err("unexpected C78FRM header magic".into());
    }
    let mut c78 = c78_stock.to_vec();

    if read_s3(&c78, SYM_ROM_NVRAM_DEF + 4, 16) != [0u8; 16] {
        return Err("NVRAM reserved header bytes are not zero".into());
    }

    let mut hooks: HashMap<String, Vec<u8>> = HashMap::new();
    for asm_name in ["iso50.S", "menus_strings.S", "exposure.S", "ipc_capture.S", "liveview_tramps.S"] {
        let secs = assemble_thumb_sections(&asm_dir.join(asm_name), c_dir)?;
        for (k, v) in secs {
            if k.starts_with(".text.") {
                hooks.insert(k, v);
            }
        }
    }
    let hud_defs = vec![
        format!("PEAKING_THRESH={}", cfg.peaking_threshold),
        format!("ZEBRA_THRESH={}u", cfg.zebra_threshold),
    ];
    let cmcm_c = compile_thumb_c(&c_dir.join("lv_cmcm.c"), c_dir, &[])?;
    let gamma_c = compile_thumb_c(&c_dir.join("lv_gamma.c"), c_dir, &[])?;
    let wget_c = compile_thumb_c(&c_dir.join("grid_widget.c"), c_dir, &[])?;
    let draw_c = compile_thumb_c(&c_dir.join("grid_draw.c"), c_dir, &hud_defs)?;

    // zero factory caves & stub service-screen vtable [33..46]
    for &(lo, hi) in &[
        (CAVE1_START, CAVE1_END),
        (CAVE2S_START, CAVE2S_END),
        (FACMENU_START, FACMENU_END),
    ] {
        patch_s3(&mut c78, lo, &vec![0u8; (hi - lo) as usize]);
    }
    for &(vt_start, count, lo, hi) in &[
        (0xA07D_EBB0u32, 10u32, CAVE1_START, CAVE1_END),
        (0xA07D_EBD8u32, 4u32, CAVE2S_START, CAVE2S_END),
    ] {
        for i in 0..count {
            let va = vt_start + i * 4;
            let old = u32::from_be_bytes(read_s3(&c78, va, 4).try_into().unwrap());
            if !(lo..hi).contains(&old) {
                return Err(format!("unexpected vtable entry @ {:#010x}: {:#010x}", va, old));
            }
            patch_s3(&mut c78, va, &(VA_SAFE_RET0 | 1).to_be_bytes());
        }
    }

    let hook_slots: &[(&str, u32, u32)] = &[
        (".text.safe_ret0", VA_SAFE_RET0, VA_ISO50_HOOK_1),
        (".text.iso50_hook_1", VA_ISO50_HOOK_1, VA_ISO50_HOOK_2),
        (".text.iso50_hook_2", VA_ISO50_HOOK_2, VA_STR_HOOK),
        (".text.str_hook", VA_STR_HOOK, VA_LV_MONO_HOOK),
        (".text.lv_mono_hook", VA_LV_MONO_HOOK, VA_OSD_ICON_HOOK),
        (".text.osd_icon_hook", VA_OSD_ICON_HOOK, VA_LV_CMCM_TRAMP),
        (".text.lv_cmcm_tramp", VA_LV_CMCM_TRAMP, VA_LV_GAMMA_WRAP),
        (".text.lv_gamma_wrap", VA_LV_GAMMA_WRAP, VA_GET_MENU_HOOK),
        (".text.get_menu_hook", VA_GET_MENU_HOOK, VA_LV_YCAP_HOOK),
        (".text.lv_ycap_hook", VA_LV_YCAP_HOOK, VA_IPC_CMD_HOOK),
        (".text.ipc_cmd_hook", VA_IPC_CMD_HOOK, VA_GET_PAPER),
        (".text.get_paper", VA_GET_PAPER, VA_SET_PAPER),
        (".text.set_paper", VA_SET_PAPER, VA_GET_FX),
        (".text.get_fx", VA_GET_FX, VA_SET_FX),
        (".text.set_fx", VA_SET_FX, VA_GET_FILL),
        (".text.get_fill", VA_GET_FILL, VA_SET_FILL),
        (".text.set_fill", VA_SET_FILL, VA_GET_MINSH),
        (".text.get_minsh", VA_GET_MINSH, VA_SET_MINSH),
        (".text.set_minsh", VA_SET_MINSH, VA_GET_PEAKC),
        (".text.get_peakc", VA_GET_PEAKC, VA_SET_PEAKC),
        (".text.set_peakc", VA_SET_PEAKC, VA_BKT5_HOOK),
        (".text.bkt5_hook", VA_BKT5_HOOK, VA_AUTOISO_A_HOOK),
        (".text.autoiso_a_hook", VA_AUTOISO_A_HOOK, VA_AUTOISO_P_HOOK),
        (".text.autoiso_p_hook", VA_AUTOISO_P_HOOK, VA_LV_CMCM_C),
        (".text.grid_draw_tramp", VA_GRID_DRAW_TRAMP, VA_GRID_DRAW_C),
    ];
    for &(sec, va, lim) in hook_slots {
        let code = hooks
            .get(sec)
            .ok_or_else(|| format!("missing section {} in hooks.S", sec))?;
        if va + (code.len() as u32) > lim {
            return Err(format!("{} ({} B) exceeds slot {:#x}..{:#x}", sec, code.len(), va, lim));
        }
        patch_s3(&mut c78, va, code);
    }

    for &(name, va, lim, code) in &[
        ("lv_cmcm.c", VA_LV_CMCM_C, VA_LV_GAMMA_C, cmcm_c.as_slice()),
        ("lv_gamma.c", VA_LV_GAMMA_C, VA_GRID_WGET_C, gamma_c.as_slice()),
        ("grid_widget.c", VA_GRID_WGET_C, VA_GRID_DRAW_TRAMP, wget_c.as_slice()),
        ("grid_draw.c", VA_GRID_DRAW_C, VA_AUTO_PAPER_TBL, draw_c.as_slice()),
    ] {
        if va + (code.len() as u32) > lim {
            return Err(format!("{} ({} B) exceeds slot {:#x}..{:#x}", name, code.len(), va, lim));
        }
        patch_s3(&mut c78, va, code);
    }
    println!(
        "  [M6M ARM/Thumb] lv_cmcm={}B, lv_gamma={}B, grid_widget={}B, grid_draw={}B",
        cmcm_c.len(), gamma_c.len(), wget_c.len(), draw_c.len()
    );

    if cfg.iso50 {
        let m1_stock = read_s3(&c78, SYM_ISO_MENU_1, 9 * 0x20).to_vec();
        let mut iso50_row = m1_stock[0x20..0x40].to_vec();
        iso50_row[0x0C..0x0E].copy_from_slice(&53u16.to_be_bytes());
        iso50_row[0x10..0x14].copy_from_slice(&1u32.to_be_bytes());
        let mut m1_new = Vec::with_capacity(10 * 0x20);
        m1_new.extend_from_slice(&m1_stock[0..0x20]);
        m1_new.extend_from_slice(&iso50_row);
        m1_new.extend_from_slice(&m1_stock[0x20..]);
        patch_s3(&mut c78, VA_ISO_MENU_1, &m1_new);

        let m2_stock = read_s3(&c78, SYM_ISO_MENU_2, 21 * 0x20).to_vec();
        let mut m2_new = Vec::with_capacity(22 * 0x20);
        m2_new.extend_from_slice(&m2_stock[0..0x20]);
        m2_new.extend_from_slice(&iso50_row);
        m2_new.extend_from_slice(&m2_stock[0x20..]);
        patch_s3(&mut c78, VA_ISO_MENU_2, &m2_new);

        if read_s3(&c78, SYM_ISO_MENU_RF, 8) != pack_u32_be(&[SYM_ISO_MENU_1, SYM_ISO_MENU_2]) {
            return Err("ISO menu literal pool mismatch".into());
        }
        patch_s3(&mut c78, SYM_ISO_MENU_RF, &pack_u32_be(&[VA_ISO_MENU_1, VA_ISO_MENU_2]));

        for &(orig_va, hook_va) in &[
            (SYM_ISO_CLAMP_1, VA_ISO50_HOOK_1),
            (SYM_ISO_CLAMP_2, VA_ISO50_HOOK_2),
        ] {
            if read_s3(&c78, orig_va, 6) != [0x00, 0x01, 0xB5, 0x00, 0x20, 0x05] {
                return Err(format!("ISO clamp mismatch @ {:#010x}", orig_va));
            }
            let mut p = vec![0xB5, 0x00];
            p.extend_from_slice(&thumb_bl(orig_va + 2, hook_va));
            patch_s3(&mut c78, orig_va, &p);
        }
    }

    let mut all_strings: Vec<(u32, &str)> = Vec::with_capacity(83);
    for &(_, sid, lbl, _, _) in &COLOR_NEW[0..30] { all_strings.push((sid, lbl)); }
    for &(_, sid, lbl, _) in &GRID_NEW[0..7] { all_strings.push((sid, lbl)); }
    for &(_, sid, lbl) in PAPER_META { all_strings.push((sid, lbl)); }
    for &(_, sid, lbl) in FX_META { all_strings.push((sid, lbl)); }
    all_strings.push((723, "Print Paper"));
    all_strings.push((724, "Film FX"));
    all_strings.push((725, "RGB Hist + Level"));
    for &(_, sid, lbl, _, _) in &COLOR_NEW[30..] { all_strings.push((sid, lbl)); }
    for &(_, sid, lbl) in X3FILL_META { all_strings.push((sid, lbl)); }
    all_strings.push((736, "X3 Fill Light"));
    for &(_, sid, lbl, _) in &GRID_NEW[8..] { all_strings.push((sid, lbl)); }
    for &(_, sid, lbl) in BKT5_META { all_strings.push((sid, lbl)); }
    for &(_, sid, lbl) in MINSHUTTER_META { all_strings.push((sid, lbl)); }
    for &(_, sid, lbl) in PEAKCOL_META { all_strings.push((sid, lbl)); }
    all_strings.push((753, "Peaking Color"));
    all_strings.push((754, "Min Shutter"));
    assert_eq!(all_strings.len(), 83);

    let mut str_cur = VA_STR_TEXT_1;
    let mut in_bank2 = false;
    for (idx, &(sid, label)) in all_strings.iter().enumerate() {
        assert_eq!(sid, 672 + idx as u32);
        let mut raw = label.as_bytes().to_vec();
        raw.push(0);
        if !in_bank2 && str_cur + raw.len() as u32 > CAVE1_END {
            str_cur = VA_STR_TEXT_2;
            in_bank2 = true;
        }
        let lim = if in_bank2 { CAVE2S_END } else { CAVE1_END };
        if str_cur + raw.len() as u32 > lim {
            return Err(format!("string pool overflow at sid {}", sid));
        }
        patch_s3(&mut c78, str_cur, &raw);
        patch_s3(&mut c78, VA_STR_PTRS_83 + (idx as u32) * 4, &str_cur.to_be_bytes());
        str_cur += raw.len() as u32;
    }
    if read_s3(&c78, SYM_UI_STR_HOOKPT, 4) != [0x00, 0x05, 0x00, 0x2C] {
        return Err("UI string hook point mismatch".into());
    }
    patch_s3(&mut c78, SYM_UI_STR_HOOKPT, &thumb_bl(SYM_UI_STR_HOOKPT, VA_STR_HOOK));

    let stock_cm = read_s3(&c78, SYM_COLOR_MENU, 8 * 0x20).to_vec();
    let viv_w = unpack_u32_be(&stock_cm[0x20..0x40]);
    let bw_w = unpack_u32_be(&stock_cm[0xC0..0xE0]);
    let mut cm_menu = stock_cm;
    let mut enabled_modes = 0usize;
    for &(mid, sid, _, is_mono, slug) in COLOR_NEW {
        if !cfg.mode_enabled(slug) {
            continue;
        }
        enabled_modes += 1;
        let t = if is_mono { &bw_w } else { &viv_w };
        cm_menu.extend_from_slice(&pack_u32_be(&[t[0], t[1], t[2], sid << 16, mid, t[5], t[6], 0]));
    }
    cm_menu.extend_from_slice(&[0u8; 0x20]);
    patch_s3(&mut c78, VA_COLOR_MENU_44, &cm_menu);

    let stock_grid = read_s3(&c78, SYM_GRID_MENU, 7 * 0x20).to_vec();
    let gtpl = unpack_u32_be(&stock_grid[0x20..0x40]);
    let mut gm = stock_grid;
    let mut enabled_grids = 0usize;
    for &(gid, sid, _, slug) in GRID_NEW {
        if !cfg.grid_enabled(slug) {
            continue;
        }
        enabled_grids += 1;
        gm.extend_from_slice(&pack_u32_be(&[gtpl[0], gtpl[1], gtpl[2], sid << 16, gid, gtpl[5], gtpl[6], 0]));
    }
    gm.extend_from_slice(&[0u8; 0x20]);
    patch_s3(&mut c78, VA_GRID_MENU_17, &gm);

    let mut sp3 = read_s3(&c78, SYM_SHOOT_P3, 5 * 0x20).to_vec();
    sp3.extend_from_slice(&pack_u32_be(&[0xF0, 0x0100_0000, 0, 723 << 16, 0, 0, 0, 0]));
    sp3.extend_from_slice(&[0u8; 0x20]);
    patch_s3(&mut c78, VA_SHOOT_P3_NEW, &sp3);

    let mut set2 = read_s3(&c78, SYM_SETUP_P2, 4 * 0x20).to_vec();
    set2.extend_from_slice(&pack_u32_be(&[0xF1, 0x0100_0000, 0, 724 << 16, 0, 0, 0, 0]));
    set2.extend_from_slice(&pack_u32_be(&[0xF2, 0x0100_0000, 0, 736 << 16, 0, 0, 0, 0]));
    set2.extend_from_slice(&[0u8; 0x20]);
    patch_s3(&mut c78, VA_SETUP_P2_V5, &set2);

    let mut set5 = read_s3(&c78, SYM_SETUP_P5, 2 * 0x20).to_vec();
    set5.extend_from_slice(&pack_u32_be(&[0xF4, 0x0100_0000, 0, 753 << 16, 0, 0, 0, 0]));
    set5.extend_from_slice(&pack_u32_be(&[0xA3, 0x0100_0000, 0, 463 << 16, 0, 0, 0, 0]));
    set5.extend_from_slice(&[0u8; 0x20]);
    patch_s3(&mut c78, VA_SETUP_P5_NEW, &set5);

    let mut isoa = read_s3(&c78, SYM_ISO_AUTO_MENU, 2 * 0x20).to_vec();
    isoa.extend_from_slice(&pack_u32_be(&[0xF3, 0x0100_0000, 0, 754 << 16, 0, 0, 0, 0]));
    isoa.extend_from_slice(&[0u8; 0x20]);
    patch_s3(&mut c78, VA_ISO_AUTO_3, &isoa);

    let stock_bkt = read_s3(&c78, SYM_BKTORD_MENU, 3 * 0x20).to_vec();
    let bkt_get = u32::from_be_bytes(stock_bkt[0x14..0x18].try_into().unwrap());
    let bkt_set = u32::from_be_bytes(stock_bkt[0x18..0x1C].try_into().unwrap());
    let mut bm = stock_bkt;
    if cfg.bkt5 {
        for &(bval, sid, _) in BKT5_META {
            bm.extend_from_slice(&pack_u32_be(&[gtpl[0], gtpl[1], gtpl[2], sid << 16, bval, bkt_get, bkt_set, 0]));
        }
    }
    bm.extend_from_slice(&[0u8; 0x20]);
    patch_s3(&mut c78, VA_BKTORD_MENU_6, &bm);

    for &(tbl_va, meta, g_va, s_va) in &[
        (VA_PAPER_MENU, PAPER_META, VA_GET_PAPER, VA_SET_PAPER),
        (VA_FILMFX_MENU, FX_META, VA_GET_FX, VA_SET_FX),
        (VA_X3FILL_MENU, X3FILL_META, VA_GET_FILL, VA_SET_FILL),
        (VA_MINSHUTTER_MENU, MINSHUTTER_META, VA_GET_MINSH, VA_SET_MINSH),
        (VA_PEAKCOL_MENU, PEAKCOL_META, VA_GET_PEAKC, VA_SET_PEAKC),
    ] {
        let mut mb = Vec::new();
        for &(val_id, sid, _) in meta {
            mb.extend_from_slice(&pack_u32_be(&[gtpl[0], gtpl[1], gtpl[2], sid << 16, val_id, g_va | 1, s_va | 1, 0]));
        }
        mb.extend_from_slice(&[0u8; 0x20]);
        patch_s3(&mut c78, tbl_va, &mb);
    }

    if read_s3(&c78, SYM_GET_MENU_TBL, 8) != [0xB5, 0x10, 0x28, 0x6B, 0xD0, 0x3E, 0xDC, 0x39] {
        return Err("Get_Menu_Table prologue mismatch".into());
    }
    patch_s3(&mut c78, SYM_GET_MENU_TBL + 2, &thumb_bl(SYM_GET_MENU_TBL + 2, VA_GET_MENU_HOOK));

    if cfg.bkt5 {
        if read_s3(&c78, SYM_GET_BKT_EV_Q16, 6) != [0xB5, 0x70, 0x00, 0x05, 0x24, 0x00] {
            return Err("Get_Bkt_EV_Q16 prologue mismatch".into());
        }
        patch_s3(&mut c78, SYM_GET_BKT_EV_Q16 + 2, &thumb_bl(SYM_GET_BKT_EV_Q16 + 2, VA_BKT5_HOOK));
    }

    if read_s3(&c78, SYM_AE_AUTOISO_A, 4) != [0x48, 0xEF, 0xE0, 0x04]
        || read_s3(&c78, SYM_AE_AUTOISO_P1, 4) != [0x20, 0x09, 0x04, 0x00]
        || read_s3(&c78, SYM_AE_AUTOISO_P2, 4) != [0x20, 0x09, 0x04, 0x00]
    {
        return Err("Auto-ISO min shutter hook site mismatch".into());
    }
    patch_s3(&mut c78, VA_TV_MIN_TBL, &pack_u32_be(&cfg.min_shutter_q16));
    patch_s3(&mut c78, SYM_AE_AUTOISO_A, &thumb_bl(SYM_AE_AUTOISO_A, VA_AUTOISO_A_HOOK));
    patch_s3(&mut c78, SYM_AE_AUTOISO_P1, &thumb_bl(SYM_AE_AUTOISO_P1, VA_AUTOISO_P_HOOK));
    patch_s3(&mut c78, SYM_AE_AUTOISO_P2, &thumb_bl(SYM_AE_AUTOISO_P2, VA_AUTOISO_P_HOOK));

    let mut auto_papers = pack.neg_default_paper;
    for neg_i in 0..16 {
        let slug = COLOR_NEW[14 + neg_i].4;
        auto_papers[neg_i] = cfg.mode_paper(slug, pack.neg_default_paper[neg_i]);
    }
    patch_s3(&mut c78, VA_AUTO_PAPER_TBL, &auto_papers);
    const MONO_W: [(i16, i16, i16); 5] = [
        (26, 51, 51), (46, 58, 24), (60, 52, 16), (72, 46, 10), (22, 82, 24),
    ];
    for (i, &(wr, wg, wb)) in MONO_W.iter().enumerate() {
        let mut rec = [0u8; 18];
        let vals = [wr, wg, wb, wr, wg, wb, wr, wg, wb];
        for (k, &v) in vals.iter().enumerate() {
            rec[k * 2..k * 2 + 2].copy_from_slice(&v.to_be_bytes());
        }
        patch_s3(&mut c78, VA_MONO_Q7_TBL + (i as u32) * 20, &rec);
    }

    if read_s3(&c78, SYM_OSD_ICON_CALL, 4) != thumb_bl(SYM_OSD_ICON_CALL, SYM_GET_CM) {
        return Err("OSD icon call mismatch".into());
    }
    patch_s3(&mut c78, SYM_OSD_ICON_CALL, &thumb_bl(SYM_OSD_ICON_CALL, VA_OSD_ICON_HOOK));

    if read_s3(&c78, 0xA003_60A2, 6) != [0x20, 0x00, 0x71, 0x60, 0x71, 0xA0] {
        return Err("IPC cmd 0x20 site mismatch @ 0xA00360A2".into());
    }
    let mut ipc_call = thumb_bl(0xA003_60A2, VA_IPC_CMD_HOOK).to_vec();
    ipc_call.extend_from_slice(&[0x46, 0xC0]);
    patch_s3(&mut c78, 0xA003_60A2, &ipc_call);

    // if a neg's :paper is overridden in cfg, pull that recipe's cmcm & tone curve for lv parity
    let eye_q10: [i16; 9] = [1024, 0, 0, 0, 1024, 0, 0, 0, 1024];
    let mut cmcm_30 = Vec::<[i16; 9]>::with_capacity(30);
    cmcm_30.extend_from_slice(&pack.dp2q_cmcm_4);
    cmcm_30.push(eye_q10);

    let mut film_gains = Vec::with_capacity(20);
    let mut lv_curves: Vec<&[u8; 4096]> = Vec::with_capacity(20);
    for rev_i in 0..4 {
        let (mn, g) = norm_lv_cmcm(&pack.lv_color_cmcm[rev_i]);
        cmcm_30.push(mn);
        film_gains.push(g);
        lv_curves.push(&pack.lv_color_curves[rev_i]);
    }
    for neg_i in 0..16 {
        let p_sel = auto_papers[neg_i];
        let p_def = pack.neg_default_paper[neg_i];
        if p_sel == p_def {
            let (mn, g) = norm_lv_cmcm(&pack.lv_color_cmcm[4 + neg_i]);
            cmcm_30.push(mn);
            film_gains.push(g);
            lv_curves.push(&pack.lv_color_curves[4 + neg_i]);
        } else {
            let rec = &pack.color_recipes[4 + neg_i * 8 + (p_sel as usize)];
            let (mn, g) = norm_lv_cmcm(&rec.cmcm_q10);
            cmcm_30.push(mn);
            film_gains.push(g);
            lv_curves.push(&rec.lut_srgb_raw);
        }
    }
    let mut bw_gains = Vec::with_capacity(5);
    for s_i in 0..5 {
        let (mn, g) = norm_lv_cmcm(&pack.bw_recipes[s_i * 8].cmcm_q10);
        cmcm_30.push(mn);
        bw_gains.push(g);
    }
    let mut cmcm_blob = Vec::with_capacity(30 * 18);
    for m in &cmcm_30 {
        for &v in m {
            cmcm_blob.extend_from_slice(&v.to_be_bytes());
        }
    }
    patch_s3(&mut c78, VA_CMCM_PACKED_30, &cmcm_blob);

    let stock_lv_lut = read_s3(&c78, 0xA026_8A14, 1024).to_vec();
    for (i, (curve, &g)) in lv_curves.iter().zip(film_gains.iter()).enumerate() {
        let knots = pack_lv_18_knots(curve, &stock_lv_lut, g);
        patch_s3(&mut c78, VA_LV_LUTS_20 + (i as u32) * 18, &knots);
    }
    for (i, &g) in bw_gains.iter().enumerate() {
        let knots = pack_lv_18_knots(&pack.bw_recipes[i * 8].lut_srgb_raw, &stock_lv_lut, g);
        patch_s3(&mut c78, VA_LV_LUTS_BW_5 + (i as u32) * 18, &knots);
    }
    println!(
        "  [M6M Config] enabled_modes={}/36, enabled_grids={}/10, iso50={}, bkt5={}",
        enabled_modes, enabled_grids, cfg.iso50, cfg.bkt5
    );

    if read_s3(&c78, SYM_LV_MONO_CALL, 4) != thumb_bl(SYM_LV_MONO_CALL, SYM_LV_MONO_FN) {
        return Err("LV mono call mismatch".into());
    }
    patch_s3(&mut c78, SYM_LV_MONO_CALL, &thumb_bl(SYM_LV_MONO_CALL, VA_LV_MONO_HOOK));

    if read_s3(&c78, SYM_LV_CMCM_CALL, 4) != thumb_bl(SYM_LV_CMCM_CALL, SYM_LV_CMCM_FN) {
        return Err("LV CMCM call mismatch".into());
    }
    patch_s3(&mut c78, SYM_LV_CMCM_CALL, &thumb_bl(SYM_LV_CMCM_CALL, VA_LV_CMCM_TRAMP));

    if read_s3(&c78, SYM_LV_GAMMA_CALL, 4) != thumb_blx_arm(SYM_LV_GAMMA_CALL, SYM_B2Y_SET_GAMMA) {
        return Err("LV gamma call mismatch".into());
    }
    patch_s3(&mut c78, SYM_LV_GAMMA_CALL, &thumb_bl(SYM_LV_GAMMA_CALL, VA_LV_GAMMA_WRAP));

    if read_s3(&c78, SYM_LV_YCAP_FN, 6) != [0xB5, 0xF0, 0x4D, 0x73, 0x00, 0x07] {
        return Err("LV YCAP prologue mismatch".into());
    }
    patch_s3(&mut c78, SYM_LV_YCAP_FN + 2, &thumb_bl(SYM_LV_YCAP_FN + 2, VA_LV_YCAP_HOOK));

    if read_s3(&c78, SYM_WIDGET14_GET, 4) != (SYM_GET_GRID | 1).to_be_bytes() {
        return Err("Widget 0x14 getter pointer mismatch".into());
    }
    patch_s3(&mut c78, SYM_WIDGET14_GET, &(VA_GRID_WGET_C | 1).to_be_bytes());

    for &call_va in &[SYM_GRID_DRAW_C1, SYM_GRID_DRAW_C2] {
        if read_s3(&c78, call_va, 4) != thumb_bl(call_va, SYM_GRID_DRAW_FN) {
            return Err(format!("Grid draw call mismatch @ {:#010x}", call_va));
        }
        patch_s3(&mut c78, call_va, &thumb_bl(call_va, VA_GRID_DRAW_TRAMP));
    }

    // replace stock updater banners in-place (24B NUL-padded slots)
    let banners: &[(u32, &[u8], &[u8])] = &[
        (0xA005_FC3C, b"M6M5 Both Debug Write.\0\0", b"dp2m-fw (M6M + M5)"),
        (0xA005_FC5C, b"M6M Only Debug Write.\0\0\0", b"dp2m-fw (M6M)"),
        (0xA005_FC7C, b"M5 Only Debug Write.\0\0\0\0", b"dp2m-fw (M5)"),
    ];
    for &(va, orig, repl) in banners {
        if read_s3(&c78, va, orig.len()) != orig {
            return Err(format!("updater banner mismatch @ {:#010x}", va));
        }
        let mut slot = vec![0u8; orig.len()];
        slot[..repl.len()].copy_from_slice(repl);
        patch_s3(&mut c78, va, &slot);
    }

    verify_m6m_invariants(&c78, c78_stock)?;
    Ok(c78)
}

pub fn verify_m6m_invariants(c78: &[u8], c78_stock: &[u8]) -> Result<(), String> {
    if read_s3(c78, FPGA_START, (FPGA_END - FPGA_START) as usize)
        != read_s3(c78_stock, FPGA_START, (FPGA_END - FPGA_START) as usize)
    {
        return Err("CRITICAL: Spartan-6 FPGA bitstream modified".into());
    }
    if c78[s3_off(BSS_START)..] != c78_stock[s3_off(BSS_START)..] {
        return Err("CRITICAL: .bss tail modified".into());
    }
    if read_s3(c78, SYM_GRID_MENU_REF, 4) != SYM_GRID_MENU.to_be_bytes() {
        return Err("CRITICAL: shared literal 0xA004D3D0 modified".into());
    }
    if read_s3(c78, 0xA005_CDF4, (0xA005_F1A0u32 - 0xA005_CDF4u32) as usize)
        != read_s3(c78_stock, 0xA005_CDF4, (0xA005_F1A0u32 - 0xA005_CDF4u32) as usize)
    {
        return Err("CRITICAL: user-reachable code 0xA005CDF4..0xA005F1A0 modified".into());
    }
    if read_s3(c78, 0xA07D_EBE8, 0x20) != read_s3(c78_stock, 0xA07D_EBE8, 0x20) {
        return Err("CRITICAL: vtable entries 47..54 modified".into());
    }
    for &(name, lo, hi) in &[("Cave1", CAVE1_START, CAVE1_END), ("Cave2S", CAVE2S_START, CAVE2S_END)] {
        let slice = read_s3(c78, lo, (hi - lo) as usize);
        if slice.windows(4).any(|w| w == [0xF7, 0xFF, 0xFF, 0xFE]) {
            return Err(format!("CRITICAL: unresolved Thumb BL relocation in {}", name));
        }
    }
    for &(base_va, cnt) in &[(VA_LV_LUTS_20, 20u32), (VA_LV_LUTS_BW_5, 5u32)] {
        for i in 0..cnt {
            let kn = read_s3(c78, base_va + i * 18, 18);
            if !kn.windows(2).all(|w| w[0] <= w[1]) {
                return Err(format!("non-monotonic LV gamma knots @ {:#010x}", base_va + i * 18));
            }
        }
    }
    println!("  [VERIFY M6M] FPGA, .bss, shared literal, Screen47..Cave3, zero relocs, LV gamma monotonicity: PASS");
    Ok(())
}
