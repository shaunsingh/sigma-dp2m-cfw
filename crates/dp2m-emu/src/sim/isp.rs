use crate::arm9::Cpu;

use super::{LCD_W, LV_H};

/// Returns a signed 12-bit Q7 (`128 == 1.0`) 3x3 white-balance matrix from M6M NVRAM WB settings.
/// WB == 1 (Auto) returns exact identity `[128, 0, 0, 0, 128, 0, 0, 0, 128]`.
pub(crate) fn wb_q7_matrix(c: &Cpu) -> [i32; 9] {
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

pub(crate) fn sv_to_iso_speed(sv: f32) -> u16 {
    const ISOS: [u16; 19] = [
        100, 125, 160, 200, 250, 320, 400, 500, 640, 800, 1000, 1250, 1600, 2000, 2500, 3200,
        4000, 5000, 6400,
    ];
    let step = ((sv - 5.0) * 3.0).round().clamp(0.0, (ISOS.len() - 1) as f32) as usize;
    ISOS[step]
}

pub(crate) fn tv_idx_rational(tv_idx: u16) -> (u32, u32) {
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

pub(crate) fn av_idx_rational(av_idx: u16) -> (u32, u32) {
    const F10: [u32; 16] = [28, 32, 35, 40, 45, 50, 56, 63, 71, 80, 90, 100, 110, 130, 140, 160];
    (F10[av_idx.min(15) as usize], 10)
}

#[inline]
pub(crate) fn apply_contrast_u8(v: i32, slope_q8: i32) -> i32 {
    (((v - 128) * slope_q8 + 128) >> 8).clamp(-128, 127) + 128
}

pub(crate) fn apply_luma_sharpness(y_plane: &mut [u8], w: usize, h: usize, delta_steps: i32) {
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
pub(crate) fn sensor_noise_rgb(seed: u32, amp: i32) -> [i32; 3] {
    let mut h = seed.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    h ^= h >> 16;
    h = h.wrapping_mul(0xC2B2_AE35);
    let n0 = ((h & 0xFF) as i32) - 128;
    let n1 = (((h >> 8) & 0xFF) as i32) - 128;
    let n2 = (((h >> 16) & 0xFF) as i32) - 128;
    [(n0 * amp) >> 7, (n1 * amp) >> 7, (n2 * amp) >> 7]
}

pub(crate) fn mean_scene_luma(scene: &[[u8; 3]]) -> f32 {
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
pub(crate) fn ycbcr_to_rgb(y: i32, cb: i32, cr: i32) -> [u8; 3] {
    let dc = cb - 128;
    let dr = cr - 128;
    let r = y + ((359 * dr + 128) >> 8);
    let g = y - ((88 * dc + 183 * dr + 128) >> 8);
    let b = y + ((454 * dc + 128) >> 8);
    [r.clamp(0, 255) as u8, g.clamp(0, 255) as u8, b.clamp(0, 255) as u8]
}

pub(crate) fn invert_monotonic_lut(lut: &[u8], max_domain: u16) -> [u16; 256] {
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
pub(crate) fn hue_sector_12(r: i32, g: i32, b: i32) -> usize {
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

pub(crate) fn read_cstr(cpu: &Cpu, va: u32, max_len: usize) -> String {
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

pub(crate) fn stock_cm_name(cm: u8) -> &'static str {
    match cm {
        1 => "Sepia",
        2 => "Monochrome",
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
pub(crate) fn default_scene_srgb() -> Vec<[u8; 3]> {
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
