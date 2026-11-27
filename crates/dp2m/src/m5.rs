//! m5slave (fujitsu fr80 BE, `Firm.bin`) firmware builder

use std::collections::HashMap;
use std::path::Path;

mod verify;
pub use verify::verify_m5;

use dp2m_emu::fr80::{assemble_file, assemble_sections};

use crate::catalog::{FILM_MODE0, MONO_FILTER_Q7, PACKED_BW0, PACKED_SD0};
use crate::config::Config;
use crate::pack::{build_argb_std_lut, build_mcas_struct, calibrate_lut_dp2m, rint_even, FilmPack};

pub const S2_OFF: usize = 0x0082_0080;
pub const S2_LEN: usize = 0x0088_4080;
pub const DELTA: u32 = 0x0003_BF80;

pub const NOR_END_OFF: usize = va_off(0x0080_0000);
const CHECKSUM_START: usize = 0x80;
const CHECKSUM_END: usize = CHECKSUM_START + 0x80_0000;
const CHECKSUM_FIELD: std::ops::Range<usize> = 0x4C..0x50;
const MCAS_STRIDE: u32 = 0x200;
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

fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let n = std::thread::available_parallelism().map_or(1, |p| p.get()).min(items.len().max(1));
    if n <= 1 {
        return items.iter().map(f).collect();
    }
    let chunk = items.len().div_ceil(n);
    std::thread::scope(|s| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|ch| s.spawn(|| ch.iter().map(&f).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    })
}
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

const fn va_off(va: u32) -> usize {
    (va - DELTA) as usize
}

fn ldi32_r12(dst: u32) -> [u8; 6] {
    let b = dst.to_be_bytes();
    [0x9F, 0x8C, b[0], b[1], b[2], b[3]]
}

fn byte_sum(buf: &[u8]) -> u32 {
    buf.iter().map(|&b| b as u32).fold(0, u32::wrapping_add)
}

pub fn m5_checksum(buf: &[u8]) -> u32 {
    byte_sum(&buf[CHECKSUM_START..])
}

pub fn stored_checksum(buf: &[u8]) -> u32 {
    u32::from_be_bytes(buf[CHECKSUM_FIELD].try_into().unwrap())
}

fn check_stock_instructions(buf: &[u8]) -> Result<(), String> {
    for (va, expected) in [
        (0x004D_3EC8, [0x9F, 0x85, 0x3F, 0x33, 0x33, 0x33]), // 0.70f JPEG budget (PREVIEW_Y at 0x80FB0580)
        (0x004D_7398, [0x9F, 0x8C, 0x00, 0x4D, 0x7F, 0x9E]), // stock call to 0x4D7F9E
    ] {
        if buf[va_off(va)..][..expected.len()] != expected {
            return Err(format!("stock instruction mismatch @ {va:#08x}"));
        }
    }
    Ok(())
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

    check_stock_instructions(&s2)?;

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
        ("cs_set", VA_CS_SET_HOOK, VA_CS_EXF_HOOK, misc.get("cs_set").ok_or("hook_ipc_exif.fr80: missing cs_set section")?),
        ("cs_exf", VA_CS_EXF_HOOK, VA_IPC_UNPK_HOOK, misc.get("cs_exf").ok_or("hook_ipc_exif.fr80: missing cs_exf section")?),
        ("ipc_unpack", VA_IPC_UNPK_HOOK, VA_EXIF_CS_HOOK, misc.get("ipc_unpack").ok_or("hook_ipc_exif.fr80: missing ipc_unpack section")?),
        ("exif_cs", VA_EXIF_CS_HOOK, VA_YCC_SUB, misc.get("exif_cs").ok_or("hook_ipc_exif.fr80: missing exif_cs section")?),
        ("ycc_engine", VA_YCC_SUB, VA_SPATIAL_DRV, &ycc_code),
        ("spatial_driver", VA_SPATIAL_DRV, VA_HALO_OPT_LUT, &drv_code),
    ];
    for &(name, va, limit, code) in placements {
        if code.len() > (limit - va) as usize {
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
    clamp_tbl[512..].fill(255);
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

    let write_mcas = |s2: &mut [u8], idx: usize, s: &[u8; 0x1AE], a: &[u8; 0x1AE]| {
        for (base, record) in [(VA_MCAS_SRGB, s), (VA_MCAS_ARGB, a)] {
            s2[va_off(base + idx as u32 * MCAS_STRIDE)..][..record.len()].copy_from_slice(record);
        }
    };

    struct Mode<'a> {
        cmcm: [i16; 9],
        lut_s: &'a [u8],
        lut_a: &'a [u8],
        meta: [u8; 4],
    }
    let write_mode = |s2: &mut [u8], idx: usize, m: &Mode| {
        let co = va_off(VA_CMCM_TABLE + (idx as u32) * 32);
        for (k, v) in m.cmcm.iter().enumerate() {
            s2[co + k * 2..co + k * 2 + 2].copy_from_slice(&v.to_be_bytes());
        }
        let ls = va_off(VA_LUTS_SRGB + (idx as u32) * 4096);
        let la = va_off(VA_LUTS_ARGB + (idx as u32) * 4096);
        s2[ls..ls + 4096].copy_from_slice(&m.lut_s[..4096]);
        s2[la..la + 4096].copy_from_slice(&m.lut_a[..4096]);
        let mo = va_off(VA_MODE_META + (idx as u32) * 4);
        s2[mo..mo + 4].copy_from_slice(&scale_meta(m.meta, cfg));
    };

    let color = |cmcm, meta| Mode { cmcm, lut_s: stock_std_lut, lut_a: &argb_std_lut, meta };
    let mono = |meta| Mode { cmcm: eye_q10, lut_s: stock_bw_lut, lut_a: stock_bw_lut, meta };
    let quattro = [
        color(pack.dp2q_cmcm_4[0], [24, 0, 48, 0]),
        color(pack.dp2q_cmcm_4[1], [30, 0, 72, 0]),
        color(pack.dp2q_cmcm_4[2], [26, 0, 88, 2]),
        color(pack.dp2q_cmcm_4[3], [26, 0, 48, 0]),
        color(eye_q10, [32, 0, 80, 0]),
        mono([38, 2, 56, 1]),
        mono([38, 2, 56, 1]),
        mono([40, 2, 60, 1]),
        mono([42, 2, 64, 1]),
        mono([36, 2, 52, 1]),
    ];
    let teal_orange_mcas = build_mcas_struct(&pack.dp2q_to_mcas, stock_mcb, stock_mcid);
    for (idx, m) in quattro.iter().enumerate() {
        write_mode(&mut s2, idx, m);
        let record = if idx == 4 { &teal_orange_mcas } else { &ident_rec };
        write_mcas(&mut s2, idx, record, record);
    }

    let color_cal = par_map(&pack.color_recipes, |r| {
        (
            calibrate_lut_dp2m(&r.lut_srgb_raw, stock_std_lut),
            calibrate_lut_dp2m(&r.lut_argb_raw, stock_std_lut),
            build_mcas_struct(&r.mck_srgb_q10, stock_mcb, stock_mcid),
            build_mcas_struct(&r.mck_argb_q10, stock_mcb, stock_mcid),
        )
    });
    for ((c_i, r), (ls, la, ms, ma)) in pack.color_recipes.iter().enumerate().zip(&color_cal) {
        let idx = quattro.len() + c_i;
        write_mode(&mut s2, idx, &Mode { cmcm: r.cmcm_q10, lut_s: ls, lut_a: la, meta: r.meta });
        write_mcas(&mut s2, idx, ms, ma);
        let so = va_off(VA_LUT3D_SRGB + (c_i * LUT3D_STRIDE) as u32);
        let ao = va_off(VA_LUT3D_ARGB + (c_i * LUT3D_STRIDE) as u32);
        // the engine reads 3d lut bytes biased (s8 ^ 0x80) to skip sign extension
        for (o, lut) in [(so, &r.lut3d_srgb_q1), (ao, &r.lut3d_argb_q1)] {
            s2[o..o + lut.len()].iter_mut().zip(lut).for_each(|(d, &s)| *d = s ^ 0x80);
        }
    }

    let bw_cal = par_map(&pack.bw_recipes, |r| {
        (calibrate_lut_dp2m(&r.lut_srgb_raw, stock_std_lut), calibrate_lut_dp2m(&r.lut_argb_raw, stock_std_lut))
    });
    for ((bw_i, r), (ls, la)) in pack.bw_recipes.iter().enumerate().zip(&bw_cal) {
        write_mode(&mut s2, (PACKED_BW0 - FILM_MODE0) as usize + bw_i, &Mode { cmcm: r.cmcm_q10, lut_s: ls, lut_a: la, meta: r.meta });
        let to = va_off(VA_BW_TONER_40 + (bw_i * 512) as u32);
        s2[to..to + 512].copy_from_slice(&r.toner_cb_cr);
    }

    let sd = Mode { cmcm: eye_q10, lut_s: stock_std_lut, lut_a: &argb_std_lut, meta: cfg.m5.sd_lut_defaults.meta() };
    for s_i in 0..8 {
        write_mode(&mut s2, (PACKED_SD0 - FILM_MODE0) as usize + s_i, &sd);
    }

    if s2[NOR_END_OFF..] != s2_stock[NOR_END_OFF..] {
        return Err("CRITICAL: modified bytes at VA >= 0x800000 (exceeds 8 MB NOR flash)".into());
    }

    let csum = m5_checksum(&s2);
    s2[CHECKSUM_FIELD].copy_from_slice(&csum.to_be_bytes());
    let nor8_sum = byte_sum(&s2[CHECKSUM_START..CHECKSUM_END]);
    if csum != nor8_sum {
        return Err(format!("CRITICAL: 8 MB checksum mismatch ({:#010x} != {:#010x})", csum, nor8_sum));
    }

    Ok(s2)
}
