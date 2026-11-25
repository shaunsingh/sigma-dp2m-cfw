//! zero-dep reader for `data/film_pack.bin` (D2MP v1) + dp2m lut calibration, lv 18-knot packing, mcas structs

use std::fs;
use std::path::Path;

pub struct ColorRecipe {
    pub cmcm_q10: [i16; 9],
    pub mck_srgb_q10: [i16; 108],
    pub mck_argb_q10: [i16; 108],
    pub lut_srgb_raw: [u8; 4096],
    pub lut_argb_raw: [u8; 4096],
    pub lut3d_srgb_q1: [u8; 4131],
    pub lut3d_argb_q1: [u8; 4131],
    pub meta: [u8; 4], // [grain_amp, grain_profile, halo_gain, halo_tint]
}

pub struct BwRecipe {
    pub cmcm_q10: [i16; 9],
    pub lut_srgb_raw: [u8; 4096],
    pub lut_argb_raw: [u8; 4096],
    pub toner_cb_cr: [u8; 512],
    pub meta: [u8; 4],
}

pub struct FilmPack {
    pub dp2q_cmcm_4: [[i16; 9]; 4],
    pub dp2q_to_mcas: [i16; 108],
    pub neg_default_paper: [u8; 16],
    pub rom_halo_lut: [u8; 256],
    pub rom_grain_envs: [u8; 768],
    pub color_recipes: Vec<ColorRecipe>,   // 132
    pub lv_color_curves: Vec<[u8; 4096]>,  // 20
    pub lv_color_cmcm: Vec<[i16; 9]>,      // 20
    pub bw_recipes: Vec<BwRecipe>,         // 40
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.pos + n > self.buf.len() {
            return Err(format!("unexpected EOF in film_pack at offset {}", self.pos));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn read_u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.read_bytes(4)?.try_into().unwrap()))
    }

    fn read_i16_array<const N: usize>(&mut self) -> Result<[i16; N], String> {
        let raw = self.read_bytes(N * 2)?;
        let mut out = [0i16; N];
        for i in 0..N {
            out[i] = i16::from_be_bytes([raw[i * 2], raw[i * 2 + 1]]);
        }
        Ok(out)
    }

    fn read_u8_array<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let raw = self.read_bytes(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(raw);
        Ok(out)
    }
}

impl FilmPack {
    pub fn load(path: &Path) -> Result<Self, String> {
        let buf = fs::read(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
        let mut c = Cursor { buf: &buf, pos: 0 };
        if c.read_bytes(4)? != b"D2MP" {
            return Err("bad D2MP magic in film_pack.bin".into());
        }
        let version = c.read_u32()?;
        let n_color = c.read_u32()? as usize;
        let n_bw = c.read_u32()? as usize;
        let n_neg = c.read_u32()? as usize;
        let n_rev = c.read_u32()? as usize;
        let n_bw_stocks = c.read_u32()? as usize;
        let _reserved = c.read_u32()?;
        if version != 1 || n_color != 132 || n_bw != 40 || n_neg != 16 || n_rev != 4 || n_bw_stocks != 5 {
            return Err(format!(
                "unsupported D2MP header: v={} color={} bw={}",
                version, n_color, n_bw
            ));
        }

        let dp2q_cmcm_4 = [
            c.read_i16_array::<9>()?,
            c.read_i16_array::<9>()?,
            c.read_i16_array::<9>()?,
            c.read_i16_array::<9>()?,
        ];
        let dp2q_to_mcas = c.read_i16_array::<108>()?;
        let neg_default_paper = c.read_u8_array::<16>()?;
        let rom_halo_lut = c.read_u8_array::<256>()?;
        let rom_grain_envs = c.read_u8_array::<768>()?;

        let mut color_recipes = Vec::with_capacity(n_color);
        for _ in 0..n_color {
            color_recipes.push(ColorRecipe {
                cmcm_q10: c.read_i16_array::<9>()?,
                mck_srgb_q10: c.read_i16_array::<108>()?,
                mck_argb_q10: c.read_i16_array::<108>()?,
                lut_srgb_raw: c.read_u8_array::<4096>()?,
                lut_argb_raw: c.read_u8_array::<4096>()?,
                lut3d_srgb_q1: c.read_u8_array::<4131>()?,
                lut3d_argb_q1: c.read_u8_array::<4131>()?,
                meta: c.read_u8_array::<4>()?,
            });
        }

        let mut lv_color_curves = Vec::with_capacity(20);
        for _ in 0..20 {
            lv_color_curves.push(c.read_u8_array::<4096>()?);
        }
        let mut lv_color_cmcm = Vec::with_capacity(20);
        for _ in 0..20 {
            lv_color_cmcm.push(c.read_i16_array::<9>()?);
        }

        let mut bw_recipes = Vec::with_capacity(n_bw);
        for _ in 0..n_bw {
            bw_recipes.push(BwRecipe {
                cmcm_q10: c.read_i16_array::<9>()?,
                lut_srgb_raw: c.read_u8_array::<4096>()?,
                lut_argb_raw: c.read_u8_array::<4096>()?,
                toner_cb_cr: c.read_u8_array::<512>()?,
                meta: c.read_u8_array::<4>()?,
            });
        }

        if c.pos != buf.len() {
            return Err(format!("trailing {} bytes in film_pack.bin", buf.len() - c.pos));
        }

        Ok(Self {
            dp2q_cmcm_4,
            dp2q_to_mcas,
            neg_default_paper,
            rom_halo_lut,
            rom_grain_envs,
            color_recipes,
            lv_color_curves,
            lv_color_cmcm,
            bw_recipes,
        })
    }
}

// round half-to-even for parity w/ numpy rint
#[inline]
pub fn rint_even(x: f64) -> f64 {
    x.round_ties_even()
}

pub fn calibrate_lut_dp2m(lut4096: &[u8; 4096], stock_std_lut: &[u8]) -> [u8; 4096] {
    let mut x_lin = [0.0f64; 4096];
    let mut sat_idx = 0usize;
    let mut found_sat = false;
    for i in 0..4096 {
        let f = (stock_std_lut[i] as f64) / 255.0;
        if !found_sat && f >= 0.999 {
            sat_idx = i;
            found_sat = true;
        }
        x_lin[i] = if f <= 0.04045 {
            f / 12.92
        } else {
            ((f + 0.055) / 1.055).powf(2.4)
        };
    }
    if found_sat && sat_idx > 10 {
        let slope = (x_lin[sat_idx] - x_lin[sat_idx - 64]) / 64.0;
        for k in (sat_idx + 1)..4096 {
            x_lin[k] = x_lin[sat_idx] + slope * ((k - sat_idx) as f64);
        }
    }

    let mut out = [0u8; 4096];
    for i in 0..4096 {
        let v = interp_4096(x_lin[i], lut4096);
        out[i] = rint_even(v).clamp(0.0, 255.0) as u8;
    }
    out
}

pub fn build_argb_std_lut(stock_std_lut: &[u8]) -> [u8; 4096] {
    let mut out = [0u8; 4096];
    let inv_gamma = 256.0 / 563.0;
    for i in 0..4096 {
        let f = (stock_std_lut[i] as f64) / 255.0;
        let x = if f <= 0.04045 {
            f / 12.92
        } else {
            ((f + 0.055) / 1.055).powf(2.4)
        };
        let y = x.clamp(0.0, 1.0).powf(inv_gamma) * 255.0;
        out[i] = rint_even(y).clamp(0.0, 255.0) as u8;
    }
    out
}

// normalize q10 cmcm so mean row sum == 1024 (preserves 10-bit lv snr); lut compensates at x_lin * gain
pub fn norm_lv_cmcm(mat: &[i16; 9]) -> ([i16; 9], f64) {
    let r0 = (mat[0] as f64) + (mat[1] as f64) + (mat[2] as f64);
    let r1 = (mat[3] as f64) + (mat[4] as f64) + (mat[5] as f64);
    let r2 = (mat[6] as f64) + (mat[7] as f64) + (mat[8] as f64);
    let mut gain = ((r0 + r1 + r2) / 3.0) / 1024.0;
    if !(0.25..=4.0).contains(&gain) {
        gain = 1.0;
    }
    let mut out = [0i16; 9];
    for i in 0..9 {
        out[i] = rint_even((mat[i] as f64) / gain).clamp(-2047.0, 2047.0) as i16;
    }
    (out, gain)
}

pub fn pack_lv_18_knots(lut4096: &[u8; 4096], stock_lv_lut1024: &[u8], gain: f64) -> [u8; 18] {
    let mut x_lin = [0.0f64; 1024];
    let mut sat_idx = 0usize;
    let mut found_sat = false;
    for i in 0..1024 {
        let f = (stock_lv_lut1024[i] as f64) / 255.0;
        if !found_sat && f >= 0.999 {
            sat_idx = i;
            found_sat = true;
        }
        x_lin[i] = if f <= 0.04045 {
            f / 12.92
        } else {
            ((f + 0.055) / 1.055).powf(2.4)
        };
    }
    if found_sat && sat_idx > 16 {
        let slope = (x_lin[sat_idx] - x_lin[sat_idx - 16]) / 16.0;
        for k in (sat_idx + 1)..1024 {
            x_lin[k] = x_lin[sat_idx] + slope * ((k - sat_idx) as f64);
        }
    }

    const KNOT_XS: [usize; 18] = [
        0, 16, 32, 48, 64, 80, 96, 112,
        128, 192, 256, 320,
        384, 512, 640, 768, 896, 1023,
    ];
    let mut knots = [0u8; 18];
    for (k, &x_idx) in KNOT_XS.iter().enumerate() {
        let v = interp_4096(x_lin[x_idx] * gain, lut4096);
        knots[k] = rint_even(v).clamp(0.0, 255.0) as u8;
    }
    knots
}

fn interp_4096(x: f64, lut: &[u8; 4096]) -> f64 {
    let pos = x * 2048.0;
    if pos <= 0.0 {
        return lut[0] as f64;
    }
    if pos >= 4095.0 {
        return lut[4095] as f64;
    }
    let i0 = pos.floor() as usize;
    let frac = pos - (i0 as f64);
    (lut[i0] as f64) * (1.0 - frac) + (lut[i0 + 1] as f64) * frac
}

pub fn build_mcas_struct(mck_q10: &[i16; 108], stock_mcb: &[u8], stock_mcid: &[u8]) -> [u8; 0x1AE] {
    let mut buf = [0u8; 0x1AE];
    buf[0x000..0x014].copy_from_slice(&stock_mcid[0..20]);
    buf[0x026..0x046].copy_from_slice(&stock_mcb[0..32]);
    for s in 0..12 {
        let off = 0x046 + s * 0x1E;
        for k in 0..9 {
            let v = mck_q10[s * 9 + k].to_be_bytes();
            buf[off + k * 2] = v[0];
            buf[off + k * 2 + 1] = v[1];
        }
    }
    buf
}
