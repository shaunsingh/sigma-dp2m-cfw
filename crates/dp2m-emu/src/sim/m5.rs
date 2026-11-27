use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::fr80::Emulator as Fr80Emu;

use super::isp::{apply_contrast_u8, apply_luma_sharpness, hue_sector_12, sensor_noise_rgb, ycbcr_to_rgb};
use super::{Camera, ExposureParams, LCD_W, LV_H};

// M5Slave (Firm.bin) layout & hooks
pub(crate) const M5_DELTA: u32 = 0x0003_BF80;
pub(crate) const M5_FLASH_LIMIT: usize = 0x007C_4080;
pub(crate) const M5_STD_LUT_VA: u32 = 0x0054_CC94;
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
const M5_SCRATCH_LEN: usize = 0xC800;
const M5_IMG_VA: [u32; 3] = [0x8200_0000, 0x8300_0000, 0x8380_0000];

impl Camera {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn run_m5_capture(
        &mut self,
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

        // M5 job slot: +0x150 is color mode, +0x14C is p1d.
        let ipc_va: u32 = 0x8008_1000;
        let slot_va: u32 = 0x8008_1200;
        for i in 0..32u32 {
            emu.w8(ipc_va + i, self.cpu.r8(cmd_va_m6m + i));
        }
        let stock_m5_cm = match packed_cm {
            1 => 1u32, // Sepia
            2 => 2u32, // Monochrome
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

        let is_stock_sepia = packed_cm == 1;
        let is_stock_mono = packed_cm == 2;
        let stock_chroma = match packed_cm {
            4 => 292, // Vivid
            5 => 228, // Neutral
            6 => 244, // Portrait
            7 => 280, // Landscape
            _ => 256,
        };
        let contrast_q8 = 256 + ((contrast as i32) - 5) * 26;
        let chroma_q8 = ((stock_chroma * (256 + ((sat as i32) - 5) * 32)) >> 8).clamp(0, 640);

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
            apply_luma_sharpness(&mut y_plane, LCD_W, LV_H, (sharp as i32) - 5, &mut self.luma_src);
        }

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

        let sd_files = self.sd_luts.clone();
        let open_slot = Rc::new(Cell::new(None));
        {
            let os = Rc::clone(&open_slot);
            let mounted = sd_files.each_ref().map(Option::is_some);
            drv.set_handler(M5_SYM_MDF_OPEN, move |e| {
                let digit = e.r8(e.r[4] + 6);
                let idx = digit.wrapping_sub(b'1') as usize;
                if idx < 8 && mounted[idx] {
                    os.set(Some(idx));
                    e.r[4] = 3;
                } else {
                    os.set(None);
                    e.r[4] = u32::MAX;
                }
                e.rp
            });
        }
        {
            let os = Rc::clone(&open_slot);
            drv.set_handler(M5_SYM_MDF_READ, move |e| {
                if let Some(idx) = os.get()
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
}
