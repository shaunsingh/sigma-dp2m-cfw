//! m5slave (fujitsu fr80 BE, `Firm.bin`) fw patcher & fr80 verifier

use std::collections::HashMap;
use std::path::Path;

use fr80::{assemble_file, assemble_sections, Emulator};

use crate::catalog::MONO_FILTER_Q7;
use crate::config::Config;
use crate::pack::{build_argb_std_lut, build_mcas_struct, calibrate_lut_dp2m, rint_even, FilmPack};

pub const S2_OFF: usize = 0x0082_0080;
pub const S2_LEN: usize = 0x0088_4080;
pub const DELTA: u32 = 0x0003_BF80;

const MAX_PACKED_MODE: usize = 198;
const LUT3D_STRIDE: usize = 0x1028;

// flash map (< 0x00800000 = 8 MB NOR limit)
const VA_MATRIX_HOOK: u32 = 0x005D_0000;
const VA_LUTSEL_HOOK: u32 = 0x005D_0180;
const VA_MCAS_HOOK: u32 = 0x005D_0200;
const VA_CS_SET_HOOK: u32 = 0x005D_0300;
const VA_CS_EXF_HOOK: u32 = 0x005D_0320;
const VA_IPC_UNPK_HOOK: u32 = 0x005D_0340;
const VA_EXIF_CS_HOOK: u32 = 0x005D_0380;
const VA_YCC_SUB: u32 = 0x005D_03C0;
const VA_SPATIAL_DRV: u32 = 0x005D_1600;
const VA_HALO_OPT_LUT: u32 = 0x005D_2000;
const VA_GRAIN_ENVS: u32 = 0x005D_2100;
const VA_MODE_META: u32 = 0x005D_2400;
const VA_MONO_TABLE: u32 = 0x005D_2800;
const VA_MCAS_IDENT: u32 = 0x005D_2900;
const VA_SD_PATH_TMPL: u32 = 0x005D_2B00;
const VA_CLAMP_LUT: u32 = 0x005D_2C00;
const VA_CMCM_TABLE: u32 = 0x005D_3000;
const VA_MCAS_SRGB: u32 = 0x005D_5000;
const VA_MCAS_ARGB: u32 = 0x005E_7000;
const VA_LUTS_SRGB: u32 = 0x005F_9000;
const VA_LUTS_ARGB: u32 = 0x006B_7000;
const VA_BW_TONER_40: u32 = 0x0077_5000;
const VA_LUT3D_SRGB: u32 = 0x0077_A000;
const VA_LUT3D_ARGB: u32 = 0x0038_0000;

const SCRATCH_BASE: u32 = 0x8010_0000;

fn va_off(va: u32) -> usize {
    (va - DELTA) as usize
}

fn ldi32_r12(dst: u32) -> [u8; 6] {
    let b = dst.to_be_bytes();
    [0x9F, 0x8C, b[0], b[1], b[2], b[3]]
}

pub fn m5_checksum(buf: &[u8]) -> u32 {
    buf[0x80..].iter().fold(0u32, |acc, &b| acc.wrapping_add(b as u32))
}

fn scale_meta(mut meta: [u8; 4], cfg: &Config) -> [u8; 4] {
    let (g, h) = (cfg.m5.grain_scale, cfg.m5.halo_scale);
    if (g - 1.0).abs() > 1e-9 {
        meta[0] = rint_even((meta[0] as f64) * g).clamp(0.0, 64.0) as u8;
    }
    if (h - 1.0).abs() > 1e-9 {
        meta[2] = rint_even((meta[2] as f64) * h).clamp(0.0, 255.0) as u8;
    }
    meta
}

pub fn build_m5(fw_raw: &[u8], pack: &FilmPack, asm_dir: &Path, cfg: &Config) -> Result<Vec<u8>, String> {
    if fw_raw.len() < S2_OFF + S2_LEN {
        return Err("DP2MV105.bin too small for M5Slave section".into());
    }
    let s2_stock = &fw_raw[S2_OFF..S2_OFF + S2_LEN];
    if &s2_stock[0..16] != b"SIGMA.CODSLRSD15" {
        return Err("unexpected M5Slave header magic".into());
    }
    let mut s2 = s2_stock.to_vec();

    if s2[va_off(0x005D_0000)..va_off(0x0080_0000)].iter().any(|&b| b != 0) {
        return Err("M5Slave Bank A free flash (0x5D0000..0x800000) is not zero".into());
    }
    if s2[va_off(0x0038_0000)..va_off(0x0041_0000)].iter().any(|&b| b != 0) {
        return Err("M5Slave Bank B free flash (0x380000..0x410000) is not zero".into());
    }

    // must keep stock: 0x4D3EC8 (0.70f jpeg budget < PREVIEW_Y) & 0x4D7398 (stock call 0x4D7F9E)
    if s2[va_off(0x004D_3EC8)..va_off(0x004D_3EC8) + 6] != [0x9F, 0x85, 0x3F, 0x33, 0x33, 0x33] {
        return Err("stock 0x4D3EC8 (0.70f) mismatch".into());
    }
    if s2[va_off(0x004D_7398)..va_off(0x004D_7398) + 6] != [0x9F, 0x8C, 0x00, 0x4D, 0x7F, 0x9E] {
        return Err("stock 0x4D7398 (call 0x4D7F9E) mismatch".into());
    }

    let sites: &[(u32, [u8; 6], u32)] = &[
        (0x004D_3782, [0x9F, 0x8C, 0x00, 0x53, 0xBE, 0xF0], VA_MATRIX_HOOK),
        (0x004E_6414, [0x9F, 0x8C, 0x00, 0x49, 0x50, 0xE0], VA_LUTSEL_HOOK),
        (0x004E_6424, [0x9F, 0x8C, 0x00, 0x4B, 0x35, 0x46], VA_MCAS_HOOK),
        (0x004D_C8F6, [0x9F, 0x8C, 0x00, 0x49, 0x76, 0x74], VA_CS_SET_HOOK),
        (0x004D_76E2, [0x9F, 0x8C, 0x00, 0x4D, 0x4F, 0xF4], VA_CS_EXF_HOOK),
        (0x004E_2862, [0x9F, 0x8C, 0x00, 0x4D, 0xCC, 0x48], VA_IPC_UNPK_HOOK),
        (0x0051_47C4, [0x9F, 0x8C, 0x00, 0x51, 0x2E, 0x30], VA_EXIF_CS_HOOK),
        (0x004D_738E, [0x9F, 0x8C, 0x00, 0x4D, 0x3E, 0x82], VA_SPATIAL_DRV),
    ];
    for &(va, expected, dst) in sites {
        let o = va_off(va);
        if s2[o..o + 6] != expected {
            return Err(format!("hook site mismatch @ 0x{:06X}", va));
        }
        s2[o..o + 6].copy_from_slice(&ldi32_r12(dst));
    }

    // inline x3f exif/makernote cs mask @ 0x514680
    let x3f_orig: [u8; 22] = [
        0xa8, 0x10, 0xe2, 0x09, 0x20, 0x50, 0xc9, 0x4d, 0x00, 0x00, 0xa8,
        0x20, 0xe2, 0x04, 0x20, 0x50, 0xc0, 0x11, 0xc9, 0x4d, 0x10, 0x01,
    ];
    let x3f_patch: [u8; 22] = [
        0xc0, 0x31, 0x82, 0x10, 0xa8, 0x20, 0xe2, 0x01, 0xc0, 0x10, 0x20,
        0x51, 0xc9, 0x4d, 0x10, 0x10, 0x9f, 0xa0, 0x9f, 0xa0, 0x9f, 0xa0,
    ];
    let xo = va_off(0x0051_4680);
    if s2[xo..xo + 22] != x3f_orig {
        return Err("X3F CS mask site mismatch @ 0x514680".into());
    }
    s2[xo..xo + 22].copy_from_slice(&x3f_patch);

    let empty = HashMap::new();
    let mat_code = assemble_file(&asm_dir.join("hook_matrix.fr80"), VA_MATRIX_HOOK, &empty)
        .map_err(|e| format!("hook_matrix.fr80: {}", e))?;
    let lut_code = assemble_file(&asm_dir.join("hook_lutsel.fr80"), VA_LUTSEL_HOOK, &empty)
        .map_err(|e| format!("hook_lutsel.fr80: {}", e))?;
    let mcas_code = assemble_file(&asm_dir.join("hook_mcas.fr80"), VA_MCAS_HOOK, &empty)
        .map_err(|e| format!("hook_mcas.fr80: {}", e))?;
    let misc_vas = HashMap::from([
        ("cs_set".into(), VA_CS_SET_HOOK),
        ("cs_exf".into(), VA_CS_EXF_HOOK),
        ("ipc_unpack".into(), VA_IPC_UNPK_HOOK),
        ("exif_cs".into(), VA_EXIF_CS_HOOK),
    ]);
    let misc = assemble_sections(&asm_dir.join("hook_ipc_exif.fr80"), &misc_vas, &empty)
        .map_err(|e| format!("hook_ipc_exif.fr80: {}", e))?;
    let ycc_code = assemble_file(&asm_dir.join("ycc_engine.fr80"), VA_YCC_SUB, &empty)
        .map_err(|e| format!("ycc_engine.fr80: {}", e))?;
    let drv_code = assemble_file(&asm_dir.join("spatial_driver.fr80"), VA_SPATIAL_DRV, &empty)
        .map_err(|e| format!("spatial_driver.fr80: {}", e))?;

    let placements: &[(&str, u32, u32, &[u8])] = &[
        ("hook_matrix", VA_MATRIX_HOOK, VA_LUTSEL_HOOK, &mat_code),
        ("hook_lutsel", VA_LUTSEL_HOOK, VA_MCAS_HOOK, &lut_code),
        ("hook_mcas", VA_MCAS_HOOK, VA_CS_SET_HOOK, &mcas_code),
        ("cs_set", VA_CS_SET_HOOK, VA_CS_EXF_HOOK, &misc["cs_set"]),
        ("cs_exf", VA_CS_EXF_HOOK, VA_IPC_UNPK_HOOK, &misc["cs_exf"]),
        ("ipc_unpack", VA_IPC_UNPK_HOOK, VA_EXIF_CS_HOOK, &misc["ipc_unpack"]),
        ("exif_cs", VA_EXIF_CS_HOOK, VA_YCC_SUB, &misc["exif_cs"]),
        ("ycc_engine", VA_YCC_SUB, VA_SPATIAL_DRV, &ycc_code),
        ("spatial_driver", VA_SPATIAL_DRV, VA_HALO_OPT_LUT, &drv_code),
    ];
    for &(name, va, limit, code) in placements {
        if va + (code.len() as u32) > limit {
            return Err(format!("M5Slave {} ({} B) overflows slot 0x{:06X}..0x{:06X}", name, code.len(), va, limit));
        }
        let o = va_off(va);
        s2[o..o + code.len()].copy_from_slice(code);
    }
    println!(
        "  [M5Slave FR80] matrix={}B, lutsel={}B, mcas={}B, ipc/exif={}B, ycc_engine={}B, spatial_driver={}B",
        mat_code.len(),
        lut_code.len(),
        mcas_code.len(),
        misc.values().map(|v| v.len()).sum::<usize>(),
        ycc_code.len(),
        drv_code.len()
    );

    let mut clamp_tbl = [0u8; 1024];
    for i in 0..256 {
        clamp_tbl[256 + i] = i as u8;
    }
    for b in &mut clamp_tbl[512..1024] {
        *b = 255;
    }
    s2[va_off(VA_CLAMP_LUT)..va_off(VA_CLAMP_LUT) + 1024].copy_from_slice(&clamp_tbl);
    s2[va_off(VA_HALO_OPT_LUT)..va_off(VA_HALO_OPT_LUT) + 256].copy_from_slice(&pack.rom_halo_lut);
    s2[va_off(VA_GRAIN_ENVS)..va_off(VA_GRAIN_ENVS) + 768].copy_from_slice(&pack.rom_grain_envs);
    s2[va_off(VA_SD_PATH_TMPL)..va_off(VA_SD_PATH_TMPL) + 12].copy_from_slice(b"D:\\LUT1.BIN\0");

    for (i, &(wr, wg, wb)) in MONO_FILTER_Q7.iter().enumerate() {
        let o = va_off(VA_MONO_TABLE + (i as u32) * 20);
        s2[o] = 0x02;
        s2[o + 1] = 0x00;
        let row = [wr, wg, wb, wr, wg, wb, wr, wg, wb];
        for (k, &v) in row.iter().enumerate() {
            s2[o + 2 + k * 2..o + 4 + k * 2].copy_from_slice(&v.to_be_bytes());
        }
    }

    let stock_std_lut = &s2_stock[va_off(0x0054_CC94)..va_off(0x0054_CC94) + 4096];
    let stock_bw_lut = &s2_stock[va_off(0x0055_2C94)..va_off(0x0055_2C94) + 4096];
    let stock_mcb = &s2_stock[va_off(0x0057_1368)..va_off(0x0057_1368) + 32];
    let stock_mcid = &s2_stock[va_off(0x0057_139C)..va_off(0x0057_139C) + 20];
    let argb_std_lut = build_argb_std_lut(stock_std_lut);

    let eye_q10: [i16; 9] = [1024, 0, 0, 0, 1024, 0, 0, 0, 1024];
    let mut ident_mck = [0i16; 108];
    for s in 0..12 {
        ident_mck[s * 9..s * 9 + 9].copy_from_slice(&eye_q10);
    }
    let ident_rec = build_mcas_struct(&ident_mck, stock_mcb, stock_mcid);
    s2[va_off(VA_MCAS_IDENT)..va_off(VA_MCAS_IDENT) + 0x1AE].copy_from_slice(&ident_rec);

    fn write_mode(
        s2: &mut [u8],
        stock_mcb: &[u8],
        stock_mcid: &[u8],
        idx: usize,
        cmcm: &[i16; 9],
        lut_s: &[u8],
        lut_a: &[u8],
        meta: [u8; 4],
        mck_s: Option<&[i16; 108]>,
        mck_a: Option<&[i16; 108]>,
    ) {
        let co = va_off(VA_CMCM_TABLE + (idx as u32) * 32);
        for k in 0..9 {
            s2[co + k * 2..co + k * 2 + 2].copy_from_slice(&cmcm[k].to_be_bytes());
        }
        let ls = va_off(VA_LUTS_SRGB + (idx as u32) * 4096);
        let la = va_off(VA_LUTS_ARGB + (idx as u32) * 4096);
        s2[ls..ls + 4096].copy_from_slice(&lut_s[0..4096]);
        s2[la..la + 4096].copy_from_slice(&lut_a[0..4096]);
        let mo = va_off(VA_MODE_META + (idx as u32) * 4);
        s2[mo..mo + 4].copy_from_slice(&meta);
        if idx < 142 {
            if let (Some(ms), Some(ma)) = (mck_s, mck_a) {
                let bs = build_mcas_struct(ms, stock_mcb, stock_mcid);
                let ba = build_mcas_struct(ma, stock_mcb, stock_mcid);
                let mso = va_off(VA_MCAS_SRGB + (idx as u32) * 0x200);
                let mao = va_off(VA_MCAS_ARGB + (idx as u32) * 0x200);
                s2[mso..mso + 0x1AE].copy_from_slice(&bs);
                s2[mao..mao + 0x1AE].copy_from_slice(&ba);
            }
        }
    }

    // modes 9..13 (dp2q/sdqh) + 14..18 (optical mono)
    let q_meta: [([i16; 9], &[i16; 108], &[u8], &[u8], [u8; 4]); 10] = [
        (pack.dp2q_cmcm_4[0], &ident_mck, stock_std_lut, &argb_std_lut, [24, 0, 48, 0]),
        (pack.dp2q_cmcm_4[1], &ident_mck, stock_std_lut, &argb_std_lut, [30, 0, 72, 0]),
        (pack.dp2q_cmcm_4[2], &ident_mck, stock_std_lut, &argb_std_lut, [26, 0, 88, 2]),
        (pack.dp2q_cmcm_4[3], &ident_mck, stock_std_lut, &argb_std_lut, [26, 0, 48, 0]),
        (eye_q10, &pack.dp2q_to_mcas, stock_std_lut, &argb_std_lut, [32, 0, 80, 0]),
        (eye_q10, &ident_mck, stock_bw_lut, stock_bw_lut, [38, 2, 56, 1]),
        (eye_q10, &ident_mck, stock_bw_lut, stock_bw_lut, [38, 2, 56, 1]),
        (eye_q10, &ident_mck, stock_bw_lut, stock_bw_lut, [40, 2, 60, 1]),
        (eye_q10, &ident_mck, stock_bw_lut, stock_bw_lut, [42, 2, 64, 1]),
        (eye_q10, &ident_mck, stock_bw_lut, stock_bw_lut, [36, 2, 52, 1]),
    ];
    for (idx, (cm, mk, ls, la, meta)) in q_meta.iter().enumerate() {
        write_mode(&mut s2, stock_mcb, stock_mcid, idx, cm, ls, la, scale_meta(*meta, cfg), Some(*mk), Some(*mk));
    }

    // modes 19..150 (132 color recipes)
    for (c_i, r) in pack.color_recipes.iter().enumerate() {
        let ls = calibrate_lut_dp2m(&r.lut_srgb_raw, stock_std_lut);
        let la = calibrate_lut_dp2m(&r.lut_argb_raw, stock_std_lut);
        write_mode(&mut s2, stock_mcb, stock_mcid, 10 + c_i, &r.cmcm_q10, &ls, &la, scale_meta(r.meta, cfg), Some(&r.mck_srgb_q10), Some(&r.mck_argb_q10));
        let so = va_off(VA_LUT3D_SRGB + (c_i * LUT3D_STRIDE) as u32);
        let ao = va_off(VA_LUT3D_ARGB + (c_i * LUT3D_STRIDE) as u32);
        s2[so..so + 4131].copy_from_slice(&r.lut3d_srgb_q1);
        s2[ao..ao + 4131].copy_from_slice(&r.lut3d_argb_q1);
    }

    // modes 151..190 (40 bw recipes)
    for (bw_i, r) in pack.bw_recipes.iter().enumerate() {
        let ls = calibrate_lut_dp2m(&r.lut_srgb_raw, stock_std_lut);
        let la = calibrate_lut_dp2m(&r.lut_argb_raw, stock_std_lut);
        write_mode(&mut s2, stock_mcb, stock_mcid, 142 + bw_i, &r.cmcm_q10, &ls, &la, scale_meta(r.meta, cfg), None, None);
        let to = va_off(VA_BW_TONER_40 + (bw_i * 512) as u32);
        s2[to..to + 512].copy_from_slice(&r.toner_cb_cr);
    }

    // modes 191..198 (sd lut slots)
    let sd_meta = scale_meta(cfg.m5.sd_lut_defaults.meta(), cfg);
    for s_i in 0..8 {
        write_mode(&mut s2, stock_mcb, stock_mcid, 182 + s_i, &eye_q10, stock_std_lut, &argb_std_lut, sd_meta, None, None);
    }

    if s2[va_off(0x0080_0000)..] != s2_stock[va_off(0x0080_0000)..] {
        return Err("CRITICAL: modified bytes at VA >= 0x800000 (exceeds 8 MB NOR flash)".into());
    }

    let csum = m5_checksum(&s2);
    s2[0x4C..0x50].copy_from_slice(&csum.to_be_bytes());
    let nor8_sum = s2[0x80..0x80_0080].iter().fold(0u32, |acc, &b| acc.wrapping_add(b as u32));
    if csum != nor8_sum {
        return Err(format!("CRITICAL: 8 MB checksum mismatch ({:#010x} != {:#010x})", csum, nor8_sum));
    }

    Ok(s2)
}

const SYM_MDF_OPEN: u32 = 0x0048_AB80;
const SYM_MDF_CLOSE: u32 = 0x0048_ABAA;
const SYM_MDF_READ: u32 = 0x0048_ABD0;
const SYM_STRIDE_DEFAULT: u32 = 0x0049_07CA;
const SYM_HW_JPEG_ENCODE: u32 = 0x004D_3E82;
const SYM_GET_IMG_DIM: u32 = 0x004D_4E56;
const SYM_GET_JOB_CM: u32 = 0x004D_D702;
const SYM_GET_JOB_1D: u32 = 0x004D_D72A;
const SCRATCH_SD_LUT: u32 = 0x8010_3500;

// emulated capture job: caller frame (job id @ +8), stack, {Y, Cb, Cr} table, planes, ctx for direct engine runs
const EMU_FP: u32 = 0x8008_0200;
const EMU_SP: u32 = 0x8008_0100;
const EMU_PLANES: u32 = 0x8009_0000;
const EMU_CTX: u32 = 0x8008_5000;
const IMG_VA: [u32; 3] = [0x8200_0000, 0x8300_0000, 0x8380_0000];
const SCRATCH_LEN: usize = 0x5000;
// PREVIEW_Y, THUMB_Y/CB/CR & RAW buffers that must stay untouched
const GUARDS: [(u32, usize, u8); 3] = [(0x80FB_0580, 0x1000, 0x5A), (0x8104_6580, 0x5010, 0x6B), (0x8104_B590, 0x4000, 0xA5)];
const EMU_STEPS: u64 = 200_000_000;

/// 4:2:2 planar YCbCr (luma stride `w`, chroma stride `w / 2`)
#[derive(Clone, PartialEq)]
pub(crate) struct Ycc {
    pub w: u32,
    pub h: u32,
    pub p: [Vec<u8>; 3],
}

impl Ycc {
    /// smooth ramps, hard-edged blocks, clipped highlights & a noisy right quarter
    pub fn synth(w: u32, h: u32) -> Self {
        let mut s = 0x2545_F491u32;
        let mut p = [vec![0u8; (w * h) as usize], vec![0u8; (w / 2 * h) as usize], vec![0u8; (w / 2 * h) as usize]];
        for r in 0..h {
            for x in 0..w {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                let block = (x / 48 + r / 24) % 5 == 0;
                let noise = if x >= w * 3 / 4 { (s >> 27) as i32 - 16 } else { 0 };
                let y = 24 + (x * 160 / w + r * 48 / h) as i32 + if block { 64 } else { 0 } + noise;
                p[0][(r * w + x) as usize] = if x % 640 < 48 && r % 96 < 20 { 255 } else { y.clamp(0, 255) as u8 };
                if x % 2 == 0 {
                    let (c, e) = ((r * (w / 2) + x / 2) as usize, if block { 24 } else { 0 });
                    p[1][c] = (88 + (x * 80 / w) as i32 + e + noise / 2).clamp(0, 255) as u8;
                    p[2][c] = (168 - (r * 80 / h) as i32 - e + noise / 2).clamp(0, 255) as u8;
                }
            }
        }
        Self { w, h, p }
    }
}

pub(crate) struct EngineCall {
    pub ctx: Vec<u32>,
    pub scratch: Vec<u8>,
    pub steps: u64,
    pub mem: [u64; 2],
}

pub(crate) struct Shot {
    pub out: Ycc,
    pub call: Option<EngineCall>,
    pub steps: u64,
}

fn emu_job<'a>(s2: &'a [u8], img: &Ycc) -> (Emulator<'a>, [usize; 3], usize) {
    let mut e = Emulator::new(s2, DELTA);
    let planes = core::array::from_fn(|i| {
        let k = e.map_sdram(IMG_VA[i], img.p[i].len());
        e.sdram[k].data.copy_from_slice(&img.p[i]);
        k
    });
    let si = e.map_sdram(SCRATCH_BASE, SCRATCH_LEN);
    for (va, len, fill) in GUARDS {
        let k = e.map_sdram(va, len);
        e.sdram[k].data.fill(fill);
    }
    e.r[14] = EMU_FP;
    e.r[15] = EMU_SP;
    (e, planes, si)
}

fn emu_out(e: &Emulator, planes: [usize; 3], img: &Ycc) -> Result<Ycc, String> {
    if GUARDS.iter().any(|&(va, _, fill)| e.sdram.iter().find(|r| r.base == va).unwrap().data.iter().any(|&b| b != fill)) {
        return Err("CRITICAL: M5Slave scratch touched PREVIEW, THUMB_Y, or RAW/RGB SDRAM!".into());
    }
    Ok(Ycc { w: img.w, h: img.h, p: planes.map(|k| e.sdram[k].data.clone()) })
}

/// runs the 0x4D738E hook as the stock caller does; `sd` serves `D:\LUTn.BIN`, `engine = false` skips the ycc pass
pub(crate) fn run_driver(s2: &[u8], cm: u32, p1d: u32, img: &Ycc, sd: Option<&[u8]>, engine: bool) -> Result<Shot, String> {
    use std::{cell::RefCell, rc::Rc};
    let (mut e, planes, si) = emu_job(s2, img);
    for (i, va) in IMG_VA.into_iter().enumerate() {
        e.w32(EMU_PLANES + 4 * i as u32, va);
    }
    e.w32(EMU_FP + 8, 3);
    e.w32(EMU_SP, 0);
    let args = [EMU_PLANES, 0x8050_E000, 1, 1];
    e.r[4..8].copy_from_slice(&args);
    let (w, h) = (img.w, img.h);
    e.set_handler(SYM_GET_JOB_1D, move |em| { em.r[4] = p1d; em.rp });
    e.set_handler(SYM_GET_JOB_CM, move |em| { em.r[4] = cm; em.rp });
    e.set_handler(SYM_GET_IMG_DIM, move |em| { em.r[4] = if em.r[4] == 0 { w } else { h }; em.rp });
    e.set_handler(SYM_STRIDE_DEFAULT, move |em| { em.r[4] = w; em.rp });
    if let Some(file) = sd {
        let (file, name) = (file.to_vec(), format!("D:\\LUT{}.BIN\0", cm.wrapping_sub(190)));
        e.set_handler(SYM_MDF_OPEN, move |em| {
            let ok = name.bytes().enumerate().all(|(i, b)| em.r8(em.r[4] + i as u32) == b);
            em.r[4] = if ok { 3 } else { u32::MAX };
            em.rp
        });
        e.set_handler(SYM_MDF_READ, move |em| {
            let n = (em.r[6] as usize).min(file.len());
            for (i, &b) in file[..n].iter().enumerate() {
                em.w8(em.r[5] + i as u32, b);
            }
            em.w32(em.r[7], n as u32);
            em.r[4] = 0;
            em.rp
        });
        e.set_handler(SYM_MDF_CLOSE, |em| { em.r[4] = 0; em.rp });
    }
    let call = Rc::new(RefCell::new(None::<Result<EngineCall, String>>));
    let cc = call.clone();
    e.set_handler(VA_YCC_SUB, move |em| {
        let (ret, sp) = (em.rp, em.r[15]);
        let ctx = (0..22).map(|i| em.r32(em.r[4] + i * 4)).collect();
        let scratch = em.sdram[si].data.clone();
        let m0 = em.mem;
        let steps = if engine { em.run(VA_YCC_SUB, EMU_STEPS) } else { Ok(0) };
        let mem = [em.mem[0] - m0[0], em.mem[1] - m0[1]];
        *cc.borrow_mut() = Some(steps.and_then(|steps| {
            if em.r[15] != sp {
                return Err("ycc engine stack imbalance".into());
            }
            Ok(EngineCall { ctx, scratch, steps, mem })
        }));
        ret
    });
    let tail = Rc::new(std::cell::Cell::new(None));
    let tc = tail.clone();
    e.set_handler(SYM_HW_JPEG_ENCODE, move |em| {
        tc.set(Some(([em.r[4], em.r[5], em.r[6], em.r[7]], em.r[15], em.rp)));
        em.rp
    });
    let steps = e.run(VA_SPATIAL_DRV, EMU_STEPS)?;
    if tail.get() != Some((args, EMU_SP, fr80::RETURN)) {
        return Err(format!("cm={} p1d={:#x}: no balanced tail call into stock HW JPEG", cm, p1d));
    }
    let call = call.borrow_mut().take().transpose()?;
    Ok(Shot { out: emu_out(&e, planes, img)?, call, steps })
}

/// runs the ycc engine on `img` with a captured driver ctx & scratch (planes, dims & strides rebound to `img`);
/// `prof` receives per-halfword hits over `VA_YCC_SUB..VA_SPATIAL_DRV`
pub(crate) fn run_engine(s2: &[u8], ctx: &[u32], scratch: &[u8], img: &Ycc, prof: Option<&mut Vec<u64>>) -> Result<(Ycc, EngineCall), String> {
    let (mut e, planes, si) = emu_job(s2, img);
    if prof.is_some() {
        e.prof = Some((VA_YCC_SUB, vec![0; ((VA_SPATIAL_DRV - VA_YCC_SUB) / 2) as usize]));
    }
    e.sdram[si].data.copy_from_slice(scratch);
    let mut c = ctx.to_vec();
    c[..7].copy_from_slice(&[IMG_VA[0], IMG_VA[1], IMG_VA[2], img.w, img.h, img.w, img.w / 2]);
    for (i, &v) in c.iter().enumerate() {
        e.w32(EMU_CTX + 4 * i as u32, v);
    }
    e.r[4] = EMU_CTX;
    let steps = e.run(VA_YCC_SUB, EMU_STEPS)?;
    if e.r[15] != EMU_SP {
        return Err(format!("ycc engine stack imbalance (fx={})", c[7]));
    }
    if let (Some(p), Some((_, hits))) = (prof, e.prof.take()) {
        *p = hits;
    }
    let call = EngineCall { ctx: c, scratch: e.sdram[si].data.clone(), steps, mem: e.mem };
    Ok((emu_out(&e, planes, img)?, call))
}

pub fn verify_m5(s2: &[u8]) -> Result<(), String> {
    let hdr_sum = u32::from_be_bytes(s2[0x4C..0x50].try_into().unwrap());
    let full_sum = m5_checksum(s2);
    let nor8_sum = s2[0x80..0x80_0080].iter().fold(0u32, |acc, &b| acc.wrapping_add(b as u32));
    if hdr_sum != full_sum || hdr_sum != nor8_sum {
        return Err(format!("checksum mismatch: hdr={:#x} full={:#x} nor8={:#x}", hdr_sum, full_sum, nor8_sum));
    }
    if s2[va_off(0x004D_3EC8)..va_off(0x004D_3EC8) + 6] != [0x9F, 0x85, 0x3F, 0x33, 0x33, 0x33] {
        return Err("0x4D3EC8 is not stock 0.70f".into());
    }
    if s2[va_off(0x004D_7398)..va_off(0x004D_7398) + 6] != [0x9F, 0x8C, 0x00, 0x4D, 0x7F, 0x9E] {
        return Err("0x4D7398 is not stock call 0x4D7F9E".into());
    }

    {
        let mut e = Emulator::new(s2, DELTA);
        e.r[14] = 0x8008_0200;
        e.r[15] = 0x8008_0100;
        e.w32(0x8008_0200 + 12, 116);
        let cc_real: [i16; 9] = [320, -154, -38, -64, 243, -51, -26, -102, 256];
        for (k, &v) in cc_real.iter().enumerate() {
            e.w16(0x8000_CC44 + (k as u32) * 2, (v as u16) & 0x0FFF);
        }
        let got = std::rc::Rc::new(std::cell::RefCell::new(Vec::<i32>::new()));
        let got_c = got.clone();
        e.set_handler(0x004B_1F20, move |em| {
            let mut row = Vec::with_capacity(9);
            for k in 0..9 {
                let raw = em.r16(em.r[4] + 2 + (k as u32) * 2) as i32;
                row.push(if raw & 0x800 != 0 { raw - 4096 } else { raw });
            }
            *got_c.borrow_mut() = row;
            em.rp
        });
        e.run(VA_MATRIX_HOOK, 10_000)?;
        let m = got.borrow();
        if m.len() != 9 || *m.iter().min().unwrap() >= -50 || *m.iter().max().unwrap() >= 400 {
            return Err(format!("signed CCMatrix ASR failed: {:?}", *m));
        }
    }

    for cs_val in [1u32, 2u32] {
        for &(va_cs, orig) in &[(VA_CS_SET_HOOK, 0x0049_7674), (VA_CS_EXF_HOOK, 0x004D_4FF4)] {
            let mut e = Emulator::new(s2, DELTA);
            e.r[4] = (3 << 5) | (4 << 2) | cs_val;
            let got = std::rc::Rc::new(std::cell::Cell::new(0u32));
            let gc = got.clone();
            e.set_handler(orig, move |em| {
                gc.set(em.r[4]);
                em.rp
            });
            e.run(va_cs, 200)?;
            if got.get() != cs_val {
                return Err(format!("CS mask @ {:#x} failed: {} != {}", va_cs, got.get(), cs_val));
            }
        }

        for cm in 1..=MAX_PACKED_MODE as u32 {
            let mut e = Emulator::new(s2, DELTA);
            e.r[14] = 0x8008_0200;
            e.r[15] = 0x8008_0100;
            e.w32(0x8008_0200 + 12, cm);
            for k in 0..9 {
                e.w16(0x8000_CC44 + k * 2, if k == 0 || k == 4 || k == 8 { 0x0080 } else { 0 });
            }
            let tag = std::rc::Rc::new(std::cell::Cell::new(0u32));
            let t1 = tag.clone();
            let t2 = tag.clone();
            e.set_handler(0x0053_BEF0, move |em| { t1.set(1); em.rp });
            e.set_handler(0x004B_1F20, move |em| { t2.set(em.r16(em.r[4]) as u32); em.rp });
            e.run(VA_MATRIX_HOOK, 10_000)?;
            let exp = if cm <= 8 || cm >= 191 { 1 } else { 0x0200 };
            if e.r[15] != 0x8008_0100 || tag.get() != exp {
                return Err(format!("matrix mode {} failed (got {:#x})", cm, tag.get()));
            }

            let mut e = Emulator::new(s2, DELTA);
            e.w32(0x8000_1B88, cs_val);
            e.r[5] = cm;
            let hit_stock = std::rc::Rc::new(std::cell::Cell::new(false));
            let hs = hit_stock.clone();
            e.set_handler(0x0049_50E0, move |em| { hs.set(true); em.rp });
            e.run(VA_LUTSEL_HOOK, 500)?;
            if cm <= 8 {
                if !hit_stock.get() {
                    return Err(format!("lutsel mode {} did not hit stock", cm));
                }
            } else {
                let base = if cs_val == 2 { VA_LUTS_ARGB } else { VA_LUTS_SRGB };
                let exp_va = base + (cm - 9) * 4096;
                if e.r[4] != exp_va {
                    return Err(format!("lutsel({}, cs={}): {:#x} != {:#x}", cm, cs_val, e.r[4], exp_va));
                }
            }

            let mut e = Emulator::new(s2, DELTA);
            e.r[14] = 0x8008_0200;
            e.r[15] = 0x8008_0180;
            e.w32(0x8008_0200 + 16, 1);
            e.w32(0x8000_1B88, cs_val);
            let sentinel: [u8; 18] = core::array::from_fn(|i| (0x11 + i) as u8);
            for (i, &b) in sentinel.iter().enumerate() {
                e.w8(0x8001_2E46 + i as u32, b);
            }
            let mcas_src = std::rc::Rc::new(std::cell::Cell::new(0u32));
            let shadow_ok = std::rc::Rc::new(std::cell::Cell::new(false));
            let ms = mcas_src.clone();
            let so = shadow_ok.clone();
            e.set_handler(0x004B_3546, |em| em.rp);
            e.set_handler(0x004D_D702, move |em| { em.r[4] = cm; em.rp });
            e.set_handler(0x004A_201C, move |em| {
                let (dst, src, n) = (em.r[4], em.r[5], em.r[6]);
                if n == 0x1AE {
                    ms.set(src);
                }
                for i in 0..n {
                    let b = em.r8(src + i);
                    em.w8(dst + i, b);
                }
                em.rp
            });
            e.set_handler(0x004B_2088, move |em| {
                let mut got = [0u8; 18];
                for i in 0..18 {
                    got[i] = em.r8(em.r[5] + 0x14 + i as u32);
                }
                so.set(em.r[4] == 1 && got == sentinel);
                em.rp
            });
            e.run(VA_MCAS_HOOK, 20_000)?;
            let exp_m = if cm <= 8 || cm >= 151 {
                VA_MCAS_IDENT
            } else {
                (if cs_val == 2 { VA_MCAS_ARGB } else { VA_MCAS_SRGB }) + (cm - 9) * 0x200
            };
            if e.r[15] != 0x8008_0180 || !shadow_ok.get() || mcas_src.get() != exp_m {
                return Err(format!("MCAS mode {} cs={} failed", cm, cs_val));
            }
        }
    }

    for &(cm_v, fx_v, fill_v, cs_v) in &[
        (1u32, 0u32, 0u32, 1u32), (8, 0, 0, 2), (22, 7, 2, 2), (124, 1, 0, 2), (198, 6, 3, 2),
    ] {
        let mut e = Emulator::new(s2, DELTA);
        let cmd_va = 0x8008_1000;
        let slot_va = 0x8008_1200;
        let p1d = (fill_v << 5) | (fx_v << 2) | cs_v;
        e.w8(cmd_va + 5, cm_v as u8);
        e.w8(cmd_va + 6, p1d as u8);
        e.w32(slot_va + 0x150, 3);
        e.w32(slot_va + 0x14C, 1);
        e.r[14] = 0x8008_0800;
        e.r[15] = 0x8008_0400;
        e.w32(0x8008_0800 + 8, cmd_va);
        e.r[4] = 5;
        e.r[5] = slot_va;
        e.set_handler(0x004D_CC48, |em| em.rp);
        e.run(VA_IPC_UNPK_HOOK, 500)?;
        let exp_cm = if cm_v < 9 { 3 } else { cm_v };
        if e.r32(slot_va + 0x150) != exp_cm || e.r32(slot_va + 0x14C) != p1d {
            return Err(format!("IPC unpack failed for cm={} p1d={:#x}", cm_v, p1d));
        }

        let mut e2 = Emulator::new(s2, DELTA);
        e2.w32(0x8008_1200 + 0x94, p1d);
        e2.w32(0x8008_1300 + 0x94, p1d);
        e2.r[14] = 0x8008_0800;
        e2.r[15] = 0x8008_0400;
        e2.w32(0x8008_0800 + 20, 0x8008_1300);
        e2.r[4] = 0x8008_1200;
        e2.set_handler(0x0051_2E30, |em| em.rp);
        e2.run(VA_EXIF_CS_HOOK, 500)?;
        if e2.r32(0x8008_1294) != cs_v || e2.r32(0x8008_1394) != cs_v {
            return Err("EXIF CS mask failed".into());
        }
    }

    // full-width capture (80x53 pyramid grids), then every fx combo over a 4704x8 strip
    let (big, strip) = (Ycc::synth(4704, 3136), Ycc::synth(4704, 8));
    let cm_c = 23 + 12 * 8 + 5;
    let tint = s2[va_off(VA_MODE_META + (cm_c - 9) * 4) + 3];
    let mut strip_out = Vec::with_capacity(8);
    let (mut pyr_steps, mut full_fx) = (0, (0, 0));
    for fx in 0..8u32 {
        let shot = run_driver(s2, cm_c, (1 << 5) | (fx << 2) | 2, &big, None, false)?;
        let c = shot.call.ok_or("4704x3136: no ycc pass")?;
        if c.ctx[16] != 80 || c.ctx[17] != 53 {
            return Err(format!("4704x3136 pyramid failed: {:?}", c.ctx));
        }
        pyr_steps = pyr_steps.max(shot.steps);
        let (out, run) = run_engine(s2, &c.ctx, &c.scratch, &strip, None)?;
        full_fx = (run.steps / (4704 / 2 * 8), run.mem[0] / (4704 / 2 * 8));
        strip_out.push(out);
    }
    for fx in [4, 5] {
        let bloom = strip_out[fx].p[2].iter().zip(&strip_out[fx ^ 4].p[2]).map(|(&a, &b)| a as i32 - b as i32).max();
        if tint != 1 && bloom < Some(8) {
            return Err(format!("4704x8 fx={} halation did not bloom Cr", fx));
        }
    }

    // stage matrix: fx (b0 3d lut, b1 grain, b2 halation) x fill x cs on stock / color / B&W / SD-LUT modes
    let img = Ycc::synth(64, 32);
    let mut sd = vec![0u8; 0x1040];
    sd[..16].copy_from_slice(b"D2ML\x01\x1E\x00\x50\x02verify\0");
    let lo = va_off(VA_LUT3D_SRGB + 5 * LUT3D_STRIDE as u32);
    sd[0x10..0x10 + 4131].copy_from_slice(&s2[lo..lo + 4131]);
    let mut runs = 0;
    for cm in [3u32, cm_c, 151 + 13, 191] {
        let meta = match cm {
            191.. => [sd[5], sd[6], sd[7], sd[8]],
            9.. => s2[va_off(VA_MODE_META + (cm - 9) * 4)..][..4].try_into().unwrap(),
            _ => [28, 0, 64, 0],
        };
        let toner = if (151..=190).contains(&cm) { VA_BW_TONER_40 + (cm - 151) * 512 } else { 0 };
        for cs in [1u32, 2] {
            let lut = match cm {
                19..=150 => (if cs == 2 { VA_LUT3D_ARGB } else { VA_LUT3D_SRGB }) + (cm - 19) * LUT3D_STRIDE as u32,
                191..=198 => SCRATCH_SD_LUT + 0x10,
                _ => 0,
            };
            for fill in 0..4u32 {
                let shots = (0..8u32)
                    .map(|fx| run_driver(s2, cm, (fill << 5) | (fx << 2) | cs, &img, (cm >= 191).then_some(&sd[..]), true))
                    .collect::<Result<Vec<_>, _>>()?;
                runs += shots.len();
                for (fx, shot) in (0u32..).zip(&shots) {
                    let tag = format!("cm={} cs={} fill={} fx={}", cm, cs, fill, fx);
                    let (lut_on, grain_on, halo_on) = (fx & 1 != 0 && lut != 0, fx & 2 != 0, fx & 4 != 0 && meta[2] != 0);
                    let work = lut_on || grain_on || halo_on || fill != 0 || toner != 0;
                    match &shot.call {
                        Some(c) if work => {
                            let got = (c.ctx[7], c.ctx[8], c.ctx[9], c.ctx[10], grain_on.then_some(c.ctx[12]), c.ctx[19] != 0, c.ctx[20] != 0);
                            let exp = (fx, fill, if lut_on { lut } else { 0 }, toner, grain_on.then_some(meta[0] as u32), halo_on, fill != 0);
                            if got != exp {
                                return Err(format!("{}: ctx {:?} != {:?}", tag, got, exp));
                            }
                        }
                        None if !work => {}
                        _ => return Err(format!("{}: ycc pass {} != expected {}", tag, shot.call.is_some(), work)),
                    }
                    for (bit, on) in [(1, lut != 0), (2, meta[0] != 0), (4, meta[2] != 0)] {
                        if fx & bit == 0 && (shots[(fx | bit) as usize].out != shot.out) != on {
                            return Err(format!("{}: fx bit {} {} the output", tag, bit, if on { "did not change" } else { "changed" }));
                        }
                    }
                }
            }
        }
    }

    println!(
        "  [VERIFY M5Slave] 198 modes x 2 CS, signed CCMatrix, IPC/EXIF, 4704x3136 pyramid ({} insns), 4704x8 fx 0..7 (fx=7 fill=1: {} insns, {} SDRAM/pair), {} fx x fill x CS stage runs, zero-collision 0x80100000: PASS",
        pyr_steps, full_fx.0, full_fx.1, runs
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn firm() -> Vec<u8> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let fw = std::fs::read(root.join("data/DP2MV105.bin")).expect("data/DP2MV105.bin");
        let pack = FilmPack::load(&root.join("data/film_pack.bin")).unwrap();
        build_m5(&fw, &pack, &root.join("src/asm/m5"), &Config::default()).unwrap()
    }

    /// ycc engine insns & SDRAM / ROM data accesses per pixel pair for every fx x fill combo; `s/L` = engine
    /// seconds for a 4704x3136 capture at ~70M insns/s. `DP2M_REF_ASM=<dir>` (e.g. `git archive <rev> src/asm/m5`)
    /// also runs that engine & driver on the same ctx / input and diffs them; `DP2M_PROF=1` prints insns/pair per
    /// engine block. `cargo test --release --offline ycc_bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn ycc_bench() {
        let s2 = firm();
        let refs = std::env::var_os("DP2M_REF_ASM").map(|d| {
            let mut r = s2.clone();
            for (f, va) in [("ycc_engine.fr80", VA_YCC_SUB), ("spatial_driver.fr80", VA_SPATIAL_DRV)] {
                let code = assemble_file(&Path::new(&d).join(f), va, &HashMap::new()).unwrap();
                r[va_off(va)..][..code.len()].copy_from_slice(&code);
            }
            r
        });
        let prof = std::env::var_os("DP2M_PROF").is_some();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut labels = fr80::assemble_file_labels(&root.join("src/asm/m5/ycc_engine.fr80"), VA_YCC_SUB, &HashMap::new()).unwrap().1;
        labels.insert(0, (VA_YCC_SUB, "<entry>".into()));
        let cm_c = 23 + 12 * 8 + 5;
        let groups = [("color", cm_c, 0), ("color fill=1", cm_c, 1), ("color fill=2", cm_c, 2), ("color fill=3", cm_c, 3), ("B&W toner", 151 + 13, 0), ("B&W fill=2", 151 + 13, 2)];
        let sizes = [(Ycc::synth(256, 128), Ycc::synth(256, 128)), (Ycc::synth(4704, 8), Ycc::synth(4704, 3136))];
        let l_pairs = 4704.0 * 3136.0 / 2.0;
        for (img, cap) in &sizes {
            let pairs = (img.w / 2 * img.h) as f64;
            println!("\n{}x{} ({} pairs)  insns/pair  sdram/pair  rom/pair   s/L  [ref insns  ref sdram  max|d|  mean|d|  (diff px)]", img.w, img.h, pairs);
            for (label, cm, fill) in groups {
                for fx in 0..8u32 {
                    let p1d = (fill << 5) | (fx << 2) | 1;
                    let shot = run_driver(&s2, cm, p1d, cap, None, false).unwrap();
                    let Some(c) = shot.call else {
                        println!("  {:<13} fx={}  stock HW JPEG", label, fx);
                        continue;
                    };
                    let mut hits = vec![];
                    let (out, run) = run_engine(&s2, &c.ctx, &c.scratch, img, prof.then_some(&mut hits)).unwrap();
                    let per = |v: u64| v as f64 / pairs;
                    let mut line = format!(
                        "  {:<13} fx={}  {:>9.1}  {:>9.2}  {:>8.2}  {:>5.1}",
                        label, fx, per(run.steps), per(run.mem[0]), per(run.mem[1]), per(run.steps) * l_pairs / 70e6
                    );
                    if let Some(r) = &refs {
                        let rc = run_driver(r, cm, p1d, cap, None, false).unwrap().call.unwrap();
                        assert!(rc.ctx == c.ctx && rc.scratch == c.scratch, "{} fx={}: driver ctx/scratch differ from ref", label, fx);
                        let (rout, rrun) = run_engine(r, &c.ctx, &c.scratch, img, None).unwrap();
                        let d: Vec<u32> = (0..3).flat_map(|k| out.p[k].iter().zip(&rout.p[k])).map(|(&a, &b)| a.abs_diff(b) as u32).collect();
                        let (maxd, ndiff) = (d.iter().max().unwrap(), d.iter().filter(|&&v| v != 0).count());
                        let mean = d.iter().sum::<u32>() as f64 / d.len() as f64;
                        line += &format!("  {:>9.1}  {:>9.2}  {:>6}  {:>7.4}  ({})", per(rrun.steps), per(rrun.mem[0]), maxd, mean, ndiff);
                    }
                    println!("{}", line);
                    if prof {
                        // insns/pair per labelled block (a block runs to the next label)
                        for (k, (va, name)) in labels.iter().enumerate() {
                            let end = labels.get(k + 1).map_or(VA_SPATIAL_DRV, |l| l.0);
                            let n: u64 = hits[((va - VA_YCC_SUB) / 2) as usize..((end - VA_YCC_SUB) / 2) as usize].iter().sum();
                            if per(n) >= 0.5 {
                                println!("      {:>7.2}  0x{:06x}  {}", per(n), va, name);
                            }
                        }
                    }
                }
            }
        }
    }
}
