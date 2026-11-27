use super::*;
use crate::{catalog::{PACKED_MAX, PACKED_NEG0}, lut::D2ML_LEN};
use dp2m_emu::fr80::{self, Emulator};

#[cfg(test)]
mod tests;

const SCRATCH_BASE: u32 = 0x8010_0000;
const COLOR_TEST_MODE: u32 = PACKED_NEG0 as u32 + 12 * 8 + 5;
const BW_TEST_MODE: u32 = PACKED_BW0 as u32 + 13;
const FULL_W: u32 = 4704;
const FULL_H: u32 = 3136;

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
const SCRATCH_LEN: usize = 0xC800;
// PREVIEW_Y, THUMB_Y/CB/CR & RAW buffers that must stay untouched
const GUARDS: [(u32, usize, u8); 3] = [(0x80FB_0580, 0x1000, 0x5A), (0x8104_6580, 0x5010, 0x6B), (0x8104_B590, 0x4000, 0xA5)];
const EMU_STEPS: u64 = 200_000_000;

/// 4:2:2 planar YCbCr (luma stride `w`, chroma stride `w / 2`)
#[derive(Clone, PartialEq)]
struct Ycc {
    w: u32,
    h: u32,
    p: [Vec<u8>; 3],
}

impl Ycc {
    /// smooth ramps, hard-edged blocks, clipped highlights & a noisy right quarter
    fn synth(w: u32, h: u32) -> Self {
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

struct EngineCall {
    ctx: [u32; 22],
    scratch: Vec<u8>,
}

struct Shot {
    out: Ycc,
    call: Option<EngineCall>,
    steps: u64,
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
fn run_driver(s2: &[u8], cm: u32, p1d: u32, img: &Ycc, sd: Option<&[u8]>, engine: bool) -> Result<Shot, String> {
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
        let ctx = std::array::from_fn(|i| em.r32(em.r[4] + i as u32 * 4));
        let scratch = em.sdram[si].data.clone();
        let result = if engine { em.run(VA_YCC_SUB, EMU_STEPS) } else { Ok(0) };
        *cc.borrow_mut() = Some(result.and_then(|_| {
            if em.r[15] != sp {
                return Err("ycc engine stack imbalance".into());
            }
            Ok(EngineCall { ctx, scratch })
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
fn run_engine(s2: &[u8], call: &EngineCall, img: &Ycc, prof: Option<&mut Vec<u64>>) -> Result<(Ycc, u64, [u64; 2]), String> {
    let (mut e, planes, si) = emu_job(s2, img);
    if prof.is_some() {
        e.prof = Some((VA_YCC_SUB, vec![0; ((VA_SPATIAL_DRV - VA_YCC_SUB) / 2) as usize]));
    }
    e.sdram[si].data.copy_from_slice(&call.scratch);
    let planes_and_dims = [IMG_VA[0], IMG_VA[1], IMG_VA[2], img.w, img.h, img.w, img.w / 2];
    for (i, v) in planes_and_dims.into_iter().chain(call.ctx[7..].iter().copied()).enumerate() {
        e.w32(EMU_CTX + 4 * i as u32, v);
    }
    e.r[4] = EMU_CTX;
    let steps = e.run(VA_YCC_SUB, EMU_STEPS)?;
    if e.r[15] != EMU_SP {
        return Err(format!("ycc engine stack imbalance (fx={})", call.ctx[7]));
    }
    if let (Some(p), Some((_, hits))) = (prof, e.prof.take()) {
        *p = hits;
    }
    Ok((emu_out(&e, planes, img)?, steps, e.mem))
}

pub fn verify_m5(s2: &[u8]) -> Result<(), String> {
    if s2.len() < CHECKSUM_END {
        return Err(format!("Firm.bin too short ({} B < {CHECKSUM_END:#x})", s2.len()));
    }
    let hdr_sum = stored_checksum(s2);
    let full_sum = m5_checksum(s2);
    let nor8_sum = byte_sum(&s2[CHECKSUM_START..CHECKSUM_END]);
    if hdr_sum != full_sum || hdr_sum != nor8_sum {
        return Err(format!("checksum mismatch: hdr={hdr_sum:#x} full={full_sum:#x} nor8={nor8_sum:#x}"));
    }
    check_stock_instructions(s2)?;

    let teal_off = va_off(VA_CMCM_TABLE + 4 * 32);
    let teal: [i16; 9] = core::array::from_fn(|k| i16::from_be_bytes(s2[teal_off + k * 2..teal_off + k * 2 + 2].try_into().unwrap()));
    let official_teal = [1092, 59, -127, 257, 392, 375, -17, -43, 1084];
    if teal != official_teal {
        return Err(format!("Teal & Orange CMCM mismatch: got {teal:?}, official DP2Q Q10 {official_teal:?}"));
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

        for cm in 1..=u32::from(PACKED_MAX) {
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
            e.set_handler(SYM_GET_JOB_CM, move |em| { em.r[4] = cm; em.rp });
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
                let got: [u8; 18] = core::array::from_fn(|i| em.r8(em.r[5] + 0x14 + i as u32));
                so.set(em.r[4] == 1 && got == sentinel);
                em.rp
            });
            e.run(VA_MCAS_HOOK, 20_000)?;
            let exp_m = if cm <= 8 || cm >= 151 {
                VA_MCAS_IDENT
            } else {
                (if cs_val == 2 { VA_MCAS_ARGB } else { VA_MCAS_SRGB }) + (cm - u32::from(FILM_MODE0)) * MCAS_STRIDE
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
    let (big, strip) = (Ycc::synth(FULL_W, FULL_H), Ycc::synth(FULL_W, 8));
    let cm_c = COLOR_TEST_MODE;
    let tint = s2[va_off(VA_MODE_META + (cm_c - 9) * 4) + 3];
    let fxs: [u32; 8] = [0, 1, 2, 3, 4, 5, 6, 7];
    let strip_res = par_map(&fxs, |&fx| -> Result<(Ycc, u64, (u64, u64)), String> {
        let shot = run_driver(s2, cm_c, (1 << 5) | (fx << 2) | 2, &big, None, false)?;
        let c = shot.call.ok_or("4704x3136: no ycc pass")?;
        if c.ctx[16] != 80 || c.ctx[17] != 53 {
            return Err(format!("4704x3136 pyramid failed: {:?}", c.ctx));
        }
        let (out, steps, mem) = run_engine(s2, &c, &strip, None)?;
        let pairs = u64::from(FULL_W / 2 * 8);
        Ok((out, shot.steps, (steps / pairs, mem[0] / pairs)))
    });
    let mut strip_out = Vec::with_capacity(8);
    let (mut pyr_steps, mut full_fx) = (0, (0, 0));
    for r in strip_res {
        let (out, steps, f) = r?;
        pyr_steps = pyr_steps.max(steps);
        full_fx = f;
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
    let mut sd = vec![0u8; D2ML_LEN];
    sd[..16].copy_from_slice(b"D2ML\x01\x1E\x00\x50\x02verify\0");
    let lo = va_off(VA_LUT3D_SRGB + 5 * LUT3D_STRIDE as u32);
    sd[0x10..0x10 + 4131].iter_mut().zip(&s2[lo..lo + 4131]).for_each(|(d, &s)| *d = s ^ 0x80);
    let jobs: Vec<(u32, u32, u32)> = [3u32, cm_c, BW_TEST_MODE, 191]
        .into_iter()
        .flat_map(|cm| [1u32, 2].into_iter().flat_map(move |cs| (0..4u32).map(move |fill| (cm, cs, fill))))
        .collect();
    let job_res = par_map(&jobs, |&(cm, cs, fill)| -> Result<usize, String> {
        let meta: [u8; 4] = match cm {
            191.. => [sd[5], sd[6], sd[7], sd[8]],
            9.. => s2[va_off(VA_MODE_META + (cm - 9) * 4)..][..4].try_into().unwrap(),
            _ => [28, 0, 64, 0],
        };
        let toner = if (151..=190).contains(&cm) { VA_BW_TONER_40 + (cm - 151) * 512 } else { 0 };
        let lut = match cm {
            19..=150 => (if cs == 2 { VA_LUT3D_ARGB } else { VA_LUT3D_SRGB }) + (cm - 19) * LUT3D_STRIDE as u32,
            191..=198 => SCRATCH_SD_LUT + 0x10,
            _ => 0,
        };
        let shots = (0..8u32)
            .map(|fx| run_driver(s2, cm, (fill << 5) | (fx << 2) | cs, &img, (cm >= 191).then_some(&sd[..]), true))
            .collect::<Result<Vec<_>, _>>()?;
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
        Ok(shots.len())
    });
    let mut runs = 0;
    for r in job_res {
        runs += r?;
    }

    println!(
        "  [VERIFY M5Slave] 198 modes x 2 CS, signed CCMatrix, IPC/EXIF, 4704x3136 pyramid ({} insns), 4704x8 fx 0..7 (fx=7 fill=1: {} insns, {} SDRAM/pair), {} fx x fill x CS stage runs, zero-collision 0x80100000: PASS",
        pyr_steps, full_fx.0, full_fx.1, runs
    );
    Ok(())
}
