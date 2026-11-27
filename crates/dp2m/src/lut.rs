//! Zero-dep Adobe/Resolve `.cube` 3D LUT -> DP2M 4,160 B D2ML SD-card binary (`LUT1.BIN`..`LUT8.BIN`).
//! Fits M5Slave's 17x9x9x3 Q1 YCbCr tetrahedral residual LUT via matrix-free Jacobi-PCG on (A^T W A + 0.05 L^T L).

use std::fs;
use std::path::Path;

const NY: usize = 17;
const NCB: usize = 9;
const NCR: usize = 9;
const N_VERT: usize = NY * NCB * NCR;
pub const D2ML_LEN: usize = 0x1040;

fn rgb_to_ycbcr(rgb: [f64; 3]) -> [f64; 3] {
    let (r, g, b) = (rgb[0] * 255.0, rgb[1] * 255.0, rgb[2] * 255.0);
    [
        (0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 255.0),
        (-0.168_736 * r - 0.331_264 * g + 0.500 * b + 128.0).clamp(0.0, 255.0),
        (0.500 * r - 0.418_688 * g - 0.081_312 * b + 128.0).clamp(0.0, 255.0),
    ]
}

pub struct CubeLut {
    pub size: usize,
    pub title: Option<String>,
    pub domain_min: [f64; 3],
    pub domain_max: [f64; 3],
    /// Indexed as `((r * size + g) * size + b)`.
    pub data: Vec<[f64; 3]>,
}

fn parse_vec3(s: &str) -> Option<[f64; 3]> {
    let mut it = s.split_whitespace();
    let (Some(a), Some(b), Some(c), None) = (it.next(), it.next(), it.next(), it.next()) else {
        return None;
    };
    Some([a.parse().ok()?, b.parse().ok()?, c.parse().ok()?])
}

impl CubeLut {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut size = None;
        let mut title = None;
        let mut domain_min = [0.0; 3];
        let mut domain_max = [1.0; 3];
        let mut rows: Vec<[f64; 3]> = Vec::new();
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("TITLE") {
                let t = rest.trim().trim_matches('"').trim();
                if !t.is_empty() {
                    title = Some(t.to_owned());
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("DOMAIN_MIN") {
                domain_min = parse_vec3(rest).ok_or("bad DOMAIN_MIN in .cube")?;
                continue;
            }
            if let Some(rest) = line.strip_prefix("DOMAIN_MAX") {
                domain_max = parse_vec3(rest).ok_or("bad DOMAIN_MAX in .cube")?;
                continue;
            }
            if let Some(rest) = line.strip_prefix("LUT_3D_SIZE") {
                size = Some(rest.trim().parse::<usize>().map_err(|_| "bad LUT_3D_SIZE")?);
                continue;
            }
            if line.starts_with("LUT_1D_") {
                return Err("1D .cube LUTs are not supported; export a 3D .cube LUT".into());
            }
            if let Some(rgb) = parse_vec3(line) {
                rows.push(rgb);
            }
        }
        let s = size.ok_or("missing LUT_3D_SIZE in .cube")?;
        if s < 2 || rows.len() != s * s * s {
            return Err(format!("invalid .cube LUT: size={s}, rows={}", rows.len()));
        }
        // Standard .cube order varies R fastest (b, g, r); transpose to (r, g, b)
        let mut data = vec![[0.0; 3]; s * s * s];
        for b in 0..s {
            for g in 0..s {
                for r in 0..s {
                    data[(r * s + g) * s + b] = rows[(b * s + g) * s + r];
                }
            }
        }
        Ok(Self { size: s, title, domain_min, domain_max, data })
    }

    pub fn eval(&self, rgb: [f64; 3]) -> [f64; 3] {
        let s = self.size;
        let sm1 = (s - 1) as f64;
        let mut p = [0.0; 3];
        for c in 0..3 {
            let span = (self.domain_max[c] - self.domain_min[c]).max(1e-9);
            let norm = ((rgb[c] - self.domain_min[c]) / span).clamp(0.0, 1.0);
            p[c] = norm * sm1;
        }
        let i0 = p.map(|v| (v.floor() as usize).min(s - 2));
        let f = [p[0] - i0[0] as f64, p[1] - i0[1] as f64, p[2] - i0[2] as f64];
        let at = |r: usize, g: usize, b: usize| self.data[(r * s + g) * s + b];
        let lerp = |a: [f64; 3], b: [f64; 3], t: f64| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
        let c00 = lerp(at(i0[0], i0[1], i0[2]), at(i0[0] + 1, i0[1], i0[2]), f[0]);
        let c01 = lerp(at(i0[0], i0[1], i0[2] + 1), at(i0[0] + 1, i0[1], i0[2] + 1), f[0]);
        let c10 = lerp(at(i0[0], i0[1] + 1, i0[2]), at(i0[0] + 1, i0[1] + 1, i0[2]), f[0]);
        let c11 = lerp(at(i0[0], i0[1] + 1, i0[2] + 1), at(i0[0] + 1, i0[1] + 1, i0[2] + 1), f[0]);
        let c0 = lerp(c00, c10, f[1]);
        let c1 = lerp(c01, c11, f[1]);
        lerp(c0, c1, f[2]).map(|v| v.clamp(0.0, 1.0))
    }
}

/// Sakamoto 6-simplex tetrahedral weights matching M5Slave `ycc_engine.fr80`.
fn tetra_weights(ycc: [f64; 3]) -> ([usize; 4], [f64; 4]) {
    let y = ycc[0].clamp(0.0, 255.0);
    let cb = ycc[1].clamp(0.0, 255.0);
    let cr = ycc[2].clamp(0.0, 255.0);
    let iy = ((y / 16.0).floor() as usize).min(NY - 2);
    let icb = ((cb / 32.0).floor() as usize).min(NCB - 2);
    let icr = ((cr / 32.0).floor() as usize).min(NCR - 2);
    let fx = (y - iy as f64 * 16.0) / 16.0;
    let fy = (cb - icb as f64 * 32.0) / 32.0;
    let fz = (cr - icr as f64 * 32.0) / 32.0;
    let (sy, scb, scr) = (NCB * NCR, NCR, 1usize);
    let p0 = iy * sy + icb * scb + icr * scr;
    let p3 = p0 + sy + scb + scr;
    let (d1, d2, a, b, c) = if fx >= fy && fy >= fz {
        (sy, sy + scb, fx, fy, fz)
    } else if fx >= fz && fz > fy {
        (sy, sy + scr, fx, fz, fy)
    } else if fz > fx && fx >= fy {
        (scr, sy + scr, fz, fx, fy)
    } else if fz >= fy && fy > fx {
        (scr, scb + scr, fz, fy, fx)
    } else if fy > fz && fz >= fx {
        (scb, scb + scr, fy, fz, fx)
    } else {
        (scb, sy + scb, fy, fx, fz)
    };
    ([p0, p0 + d1, p0 + d2, p3], [1.0 - a, a - b, b - c, c])
}

/// Matrix-free weighted SPD system `(A^T W A + 0.05 L^T L)` with exact Jacobi preconditioner.
struct System {
    rows: Vec<([usize; 4], [f64; 4], f64)>,
    lap_nbrs: Vec<Vec<usize>>,
    diag: Vec<f64>,
}

impl System {
    fn new(samples: &[([f64; 3], f64)]) -> Self {
        let rows: Vec<_> = samples
            .iter()
            .map(|&(ycc, sw)| {
                let (idx, w) = tetra_weights(ycc);
                (idx, w, sw)
            })
            .collect();
        let mut lap_nbrs = vec![Vec::new(); N_VERT];
        for iy in 0..NY {
            for icb in 0..NCB {
                for icr in 0..NCR {
                    let u = (iy * NCB + icb) * NCR + icr;
                    for (dy, dcb, dcr) in [(-1isize, 0, 0), (1, 0, 0), (0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1)] {
                        let (ny, ncb, ncr) = (iy as isize + dy, icb as isize + dcb, icr as isize + dcr);
                        if ny >= 0 && ny < NY as isize && ncb >= 0 && ncb < NCB as isize && ncr >= 0 && ncr < NCR as isize {
                            lap_nbrs[u].push((ny as usize * NCB + ncb as usize) * NCR + ncr as usize);
                        }
                    }
                }
            }
        }
        let mut diag = vec![0.0f64; N_VERT];
        for &(idx, w, sw) in &rows {
            for k in 0..4 {
                diag[idx[k]] += sw * w[k] * w[k];
            }
        }
        for u in 0..N_VERT {
            let deg = lap_nbrs[u].len() as f64;
            diag[u] += 0.05 * (deg * deg + deg);
        }
        Self { rows, lap_nbrs, diag }
    }

    fn apply(&self, x: &[f64], lx: &mut [f64], out: &mut [f64]) {
        out.fill(0.0);
        for &(idx, w, sw) in &self.rows {
            let ax = sw * (w[0] * x[idx[0]] + w[1] * x[idx[1]] + w[2] * x[idx[2]] + w[3] * x[idx[3]]);
            for k in 0..4 {
                out[idx[k]] += w[k] * ax;
            }
        }
        for u in 0..N_VERT {
            let deg = self.lap_nbrs[u].len() as f64;
            let mut s = deg * x[u];
            for &v in &self.lap_nbrs[u] {
                s -= x[v];
            }
            lx[u] = s;
        }
        for u in 0..N_VERT {
            let deg = self.lap_nbrs[u].len() as f64;
            let mut s = deg * lx[u];
            for &v in &self.lap_nbrs[u] {
                s -= lx[v];
            }
            out[u] += 0.05 * s;
        }
    }

    fn solve_pcg(&self, rhs: &[f64]) -> Vec<f64> {
        let mut x = vec![0.0f64; N_VERT];
        let mut r = rhs.to_vec();
        let mut z: Vec<f64> = (0..N_VERT).map(|i| r[i] / self.diag[i]).collect();
        let mut p = z.clone();
        let mut ap = vec![0.0f64; N_VERT];
        let mut lx = vec![0.0f64; N_VERT];
        let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        for _ in 0..400 {
            if rz < 1e-18 {
                break;
            }
            self.apply(&p, &mut lx, &mut ap);
            let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
            if pap <= 0.0 {
                break;
            }
            let alpha = rz / pap;
            for i in 0..N_VERT {
                x[i] += alpha * p[i];
                r[i] -= alpha * ap[i];
                z[i] = r[i] / self.diag[i];
            }
            let rz_new: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
            let beta = rz_new / rz;
            for i in 0..N_VERT {
                p[i] = z[i] + beta * p[i];
            }
            rz = rz_new;
        }
        x
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FitStats {
    pub mae: f64,
    pub p95: f64,
    pub max_err: f64,
    pub gray_mae: f64,
}

pub fn compile_cube_to_d2ml(
    cube: &CubeLut,
    grain_amp: u8,
    grain_profile: u8,
    halo_gain: u8,
    halo_tint: u8,
    tag: &str,
) -> (Vec<u8>, FitStats) {
    // 17^3 RGB lattice matching the 17-slice Y grid + 65 weighted neutral-gray anchors
    const N_GRID: usize = 17;
    const N_GRAY: usize = 65;
    let n_cube = N_GRID * N_GRID * N_GRID;
    let mut ycc_in = Vec::with_capacity(n_cube + N_GRAY);
    let mut ycc_out = Vec::with_capacity(n_cube + N_GRAY);
    for ir in 0..N_GRID {
        for ig in 0..N_GRID {
            for ib in 0..N_GRID {
                let rgb = [ir as f64 / (N_GRID - 1) as f64, ig as f64 / (N_GRID - 1) as f64, ib as f64 / (N_GRID - 1) as f64];
                ycc_in.push((rgb_to_ycbcr(rgb), 1.0));
                ycc_out.push(rgb_to_ycbcr(cube.eval(rgb)));
            }
        }
    }
    for k in 0..N_GRAY {
        let g = k as f64 / (N_GRAY - 1) as f64;
        let rgb = [g, g, g];
        ycc_in.push((rgb_to_ycbcr(rgb), 2.5));
        ycc_out.push(rgb_to_ycbcr(cube.eval(rgb)));
    }

    let sys = System::new(&ycc_in);
    let mut lut_i8 = vec![0i8; N_VERT * 3];
    for ch in 0..3 {
        let mut rhs = vec![0.0f64; N_VERT];
        for (row_i, &(idx, w, sw)) in sys.rows.iter().enumerate() {
            let d = sw * (ycc_out[row_i][ch] - ycc_in[row_i].0[ch]);
            for k in 0..4 {
                rhs[idx[k]] += w[k] * d;
            }
        }
        let sol = sys.solve_pcg(&rhs);
        for (u, &v) in sol.iter().enumerate() {
            lut_i8[u * 3 + ch] = (v * 2.0).round_ties_even().clamp(-126.0, 126.0) as i8;
        }
    }

    let mut errs = Vec::with_capacity(n_cube * 3);
    let mut gray_sum = 0.0;
    for (row_i, &(idx, w, _)) in sys.rows.iter().enumerate() {
        for ch in 0..3 {
            let delta = (0..4).map(|k| w[k] * (lut_i8[idx[k] * 3 + ch] as f64) * 0.5).sum::<f64>();
            let pred = (ycc_in[row_i].0[ch] + delta).clamp(0.0, 255.0);
            let e = (pred - ycc_out[row_i][ch]).abs();
            if row_i < n_cube {
                errs.push(e);
            } else {
                gray_sum += e;
            }
        }
    }
    errs.sort_by(|a, b| a.total_cmp(b));
    let mae = errs.iter().sum::<f64>() / errs.len() as f64;
    let p95 = errs[(errs.len() * 95) / 100];
    let max_err = *errs.last().unwrap_or(&0.0);
    let gray_mae = gray_sum / (N_GRAY * 3) as f64;

    let mut blob = vec![0u8; D2ML_LEN];
    blob[0..4].copy_from_slice(b"D2ML");
    blob[4] = 1;
    blob[5] = grain_amp;
    blob[6] = grain_profile;
    blob[7] = halo_gain;
    blob[8] = halo_tint;
    for (i, b) in tag.bytes().filter(|b| b.is_ascii_graphic() || *b == b' ').take(7).enumerate() {
        blob[9 + i] = b;
    }
    for (i, &v) in lut_i8.iter().enumerate() {
        blob[0x10 + i] = v as u8;
    }
    (blob, FitStats { mae, p95, max_err, gray_mae })
}

pub fn compile_file(
    input: &Path,
    output: &Path,
    grain_amp: u8,
    grain_profile: u8,
    halo_gain: u8,
    halo_tint: u8,
    tag_override: Option<&str>,
) -> Result<(), String> {
    let text = fs::read_to_string(input).map_err(|e| format!("read {}: {e}", input.display()))?;
    let cube = CubeLut::parse(&text)?;
    let fallback = input.file_stem().and_then(|s| s.to_str()).unwrap_or("CUSTOM");
    let tag = tag_override.or(cube.title.as_deref()).unwrap_or(fallback);
    let (blob, st) = compile_cube_to_d2ml(&cube, grain_amp, grain_profile, halo_gain, halo_tint, tag);
    fs::write(output, &blob).map_err(|e| format!("write {}: {e}", output.display()))?;
    println!(
        "Wrote {} ({} B, cube {}^3): YCbCr MAE={:.2} P95={:.2} max={:.2} gray_MAE={:.2} LSB",
        output.display(),
        blob.len(),
        cube.size,
        st.mae,
        st.p95,
        st.max_err,
        st.gray_mae
    );
    Ok(())
}

pub fn inspect_d2ml(path: &Path) -> Result<(), String> {
    let b = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if b.len() != D2ML_LEN || &b[0..4] != b"D2ML" {
        return Err(format!("not a valid 4160 B D2ML file: {} ({} B)", path.display(), b.len()));
    }
    let ver = b[4];
    let grain = b[5];
    let gprof = match b[6] { 0 => "negative", 1 => "slide", 2 => "bw", _ => "unknown" };
    let halo = b[7];
    let htint = match b[8] { 0 => "crimson", 1 => "neutral", 2 => "amber", _ => "unknown" };
    let tag_end = b[9..16].iter().position(|&c| c == 0).unwrap_or(7);
    let tag = String::from_utf8_lossy(&b[9..9 + tag_end]);
    println!("D2ML v{ver} tag={tag:?} grain={grain} ({gprof}) halo={halo} ({htint})");
    for (ch, name) in [(0, "Y "), (1, "Cb"), (2, "Cr")] {
        let mut mn = 127i8;
        let mut mx = -128i8;
        let mut sq = 0.0f64;
        for u in 0..N_VERT {
            let v = b[0x10 + u * 3 + ch] as i8;
            mn = mn.min(v);
            mx = mx.max(v);
            let lsb = v as f64 * 0.5;
            sq += lsb * lsb;
        }
        println!("  {name} delta: [{:+.1}, {:+.1}] LSB  RMS={:.2} LSB", mn as f64 * 0.5, mx as f64 * 0.5, (sq / N_VERT as f64).sqrt());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_cube(n: usize, dom_min: [f64; 3], dom_max: [f64; 3], f: impl Fn([f64; 3]) -> [f64; 3]) -> CubeLut {
        let mut s = format!(
            "TITLE \"TESTLUT\"\nDOMAIN_MIN {} {} {}\nDOMAIN_MAX {} {} {}\nLUT_3D_SIZE {n}\n",
            dom_min[0], dom_min[1], dom_min[2], dom_max[0], dom_max[1], dom_max[2]
        );
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    let t = [r as f64 / (n - 1) as f64, g as f64 / (n - 1) as f64, b as f64 / (n - 1) as f64];
                    let o = f(t);
                    s.push_str(&format!("{:.5} {:.5} {:.5}\n", o[0], o[1], o[2]));
                }
            }
        }
        CubeLut::parse(&s).unwrap()
    }

    #[test]
    fn identity_cube_compiles_to_zero_residual_d2ml() {
        let cube = synth_cube(5, [0.0; 3], [1.0; 3], |rgb| rgb);
        assert_eq!(cube.title.as_deref(), Some("TESTLUT"));
        let (blob, st) = compile_cube_to_d2ml(&cube, 28, 0, 64, 0, "IDENT");
        assert_eq!(blob.len(), D2ML_LEN);
        assert_eq!(&blob[0..4], b"D2ML");
        assert!(st.mae < 1e-6 && st.gray_mae < 1e-6, "identity stats {st:?}");
        assert!(blob[0x10..0x10 + N_VERT * 3].iter().all(|&b| b == 0));
    }

    #[test]
    fn domain_and_filmic_grade_fit_tightly() {
        let grade = |[r, g, b]: [f64; 3]| {
            let s = |x: f64| (x * x * (3.0 - 2.0 * x)).clamp(0.0, 1.0);
            [(s(r) * 1.04).min(1.0), s(g), (s(b) * 0.94).min(1.0)]
        };
        let cube_dom = synth_cube(17, [0.05; 3], [0.95; 3], grade);
        let cube_unit = synth_cube(17, [0.0; 3], [1.0; 3], grade);
        assert!(
            (cube_dom.eval([0.05, 0.05, 0.05])[1]).abs() < 1e-6 && cube_unit.eval([0.05, 0.05, 0.05])[1] > 0.005,
            "DOMAIN_MIN should remap 0.05 to black"
        );
        let (blob, st) = compile_cube_to_d2ml(&cube_dom, 24, 0, 48, 0, "FILMIC");
        assert_eq!(&blob[9..15], b"FILMIC");
        assert!(st.mae < 1.2 && st.gray_mae < 0.75, "filmic fit too loose: {st:?}");
    }
}
