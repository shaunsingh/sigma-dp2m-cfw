use super::*;

fn firm() -> Vec<u8> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fw = std::fs::read(root.join("data/DP2MV105.bin")).expect("data/DP2MV105.bin");
    let pack = FilmPack::load(&root.join("data/film_pack.bin")).unwrap();
    build_m5(&fw, &pack, &root.join("firmware/m5"), &Config::default()).unwrap()
}

#[test]
fn pack_swar_signed_deltas() {
    let s2 = firm();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (_, labels) = fr80::assemble_file_labels(&root.join("firmware/m5/ycc_engine.fr80"), VA_YCC_SUB, &HashMap::new()).unwrap();
    let entry = labels.iter().find(|(_, name)| name == "pack_swar").unwrap().0;
    for delta in -127i32..=127 {
        let mut e = Emulator::new(&s2, DELTA);
        e.r[4] = delta as u32;
        for reg in [3, 5, 7, 8, 9, 10] {
            e.r[reg] = 0x1234_0000 + reg as u32;
        }
        let steps = e.run(entry, 100).unwrap();
        let expected = (delta.unsigned_abs() * 0x0101_0101) | if delta < 0 { 0x8000_0000 } else { 0 };
        assert_eq!(e.r[4], expected, "delta={delta}");
        assert_eq!(steps, 10);
        for reg in [3, 5, 7, 8, 9, 10] {
            assert_eq!(e.r[reg], 0x1234_0000 + reg as u32);
        }
    }
}

#[test]
fn tile_3d_grain_saturates_odd_pixels_without_cross_lane_borrow_or_carry() {
    let s2 = firm();
    // 64x32 aligned frame: even Y=128 (passes 4x2 coherence), odd Y in {0, 128, 255} (unchecked by check_4x2_coherence).
    // Test dY in {0, +31, +32, -32, -33} (covers SWAR range boundaries [-32..=31] and clamp fallback):
    // odd=0 must saturate to 0 without wrapping or borrowing -1 from the adjacent even pixel,
    // and odd=255 must saturate to 255 without wrapping or carrying +1 into the adjacent even pixel.
    for dy_lsb in [0i8, 31, 32, -32, -33] {
        let mut sd = vec![0u8; D2ML_LEN];
        sd[..16].copy_from_slice(b"D2ML\x01\x30\x00\x00\x00oddtest"); // grain_amp=48, profile=0 (neg), halo=0
        let dy_q1 = (dy_lsb * 2) as u8;
        for v in 0..(17 * 9 * 9) {
            sd[0x10 + v * 3] = dy_q1;
        }
        for odd_y in [0u8, 128u8, 255u8] {
            let mut img = Ycc { w: 64, h: 32, p: [vec![128; 64 * 32], vec![128; 32 * 32], vec![128; 32 * 32]] };
            for r in 0..32usize {
                for x in (1..64usize).step_by(2) {
                    img.p[0][r * 64 + x] = odd_y;
                }
            }
            let shot = run_driver(&s2, 191, (3 << 2) | 1, &img, Some(&sd), true).unwrap();
            let call = shot.call.as_ref().expect("ycc pass ran");
            let y00_post = (128 + i32::from(dy_lsb)).clamp(0, 255) as u32;
            let env = s2[va_off(call.ctx[11] + y00_post)];
            let scale = ((u32::from(env) * call.ctx[12]) >> 8) as i32;
            assert!(scale > 10, "expected active grain scale, got {scale}");

            // Exact scalar replay of ftg_row's xorshift32 (seed 0x1357_9BDF) with per-pixel saturation.
            let mut prng = 0x1357_9BDFu32;
            let mut expected_y = img.p[0].clone();
            for by in (0..32usize).step_by(2) {
                for bx in (0..64usize).step_by(4) {
                    for dy in 0..2usize {
                        prng ^= prng << 13;
                        prng ^= prng >> 17;
                        prng ^= prng << 5;
                        for lane in 0..4usize {
                            let byte = (prng >> (24 - lane * 8)) & 0xFF;
                            let tri = ((byte & 0xF) + (byte >> 4)) as i32 - 15;
                            let g = (scale * tri).div_euclid(32);
                            let idx = (by + dy) * 64 + bx + lane;
                            expected_y[idx] = (i32::from(img.p[0][idx]) + i32::from(dy_lsb) + g).clamp(0, 255) as u8;
                        }
                    }
                }
            }
            assert_eq!(shot.out.p[0], expected_y, "dy={dy_lsb} odd_y={odd_y}: odd-pixel grain wrapped or contaminated adjacent even pixel");
        }
    }
}

#[test]
fn sd_lut_clamps_grain_amp_and_rejects_bad_profile_atomically() {
    let s2 = firm();
    let img = Ycc { w: 64, h: 32, p: [vec![128; 64 * 32], vec![128; 32 * 32], vec![128; 32 * 32]] };

    // Valid baseline at max documented grain_amp = 64.
    let mut sd_max = vec![0u8; D2ML_LEN];
    sd_max[..16].copy_from_slice(b"D2ML\x01\x40\x00\x00\x00maxamp\0");
    let baseline = run_driver(&s2, 191, (3 << 2) | 1, &img, Some(&sd_max), true).unwrap();

    // Out-of-range grain_amp (70 crosses 1024 - 15*scale < 0; 255 is max u8) must clamp to 64 at load time.
    for bad_amp in [70u8, 255u8] {
        let mut sd_bad = sd_max.clone();
        sd_bad[5] = bad_amp;
        let shot = run_driver(&s2, 191, (3 << 2) | 1, &img, Some(&sd_bad), true).unwrap();
        let ctx = shot.call.as_ref().expect("ycc pass ran").ctx;
        assert_eq!(ctx[12], 64, "grain_amp={bad_amp} was not clamped to 64");
        assert_eq!(shot.out.p, baseline.out.p, "grain_amp={bad_amp} output diverged from clamped 64");
    }

    // Invalid grain_profile (> 2) must reject the SD header without leaving ctx+48 (grain_amp) clobbered.
    let mut sd_bad_prof = sd_max.clone();
    sd_bad_prof[5] = 55;
    sd_bad_prof[6] = 9;
    let shot = run_driver(&s2, 191, (2 << 2) | 1, &img, Some(&sd_bad_prof), false).unwrap();
    let ctx = shot.call.as_ref().expect("ycc pass ran").ctx;
    let default_amp = u32::from(Config::default().m5.sd_lut_defaults.meta()[0]);
    assert_eq!(ctx[12], default_amp, "invalid grain_profile clobbered default grain_amp");
}

#[test]
fn ycc_engine_zero_or_subtile_dims_do_not_write_outside_buffer() {
    let s2 = firm();
    let sample = Ycc::synth(64, 32);
    // Exercise all four ycc_engine dispatch paths: Path A (3D), Path T (B&W toner), Path F (fill), Path B (general).
    let cases = [
        ("path_a_3d", COLOR_TEST_MODE, (1 << 2) | 1),
        ("path_t_toner", BW_TEST_MODE, 1),
        ("path_f_fill", COLOR_TEST_MODE, (1 << 5) | 1),
        ("path_b_gen", COLOR_TEST_MODE, (1 << 5) | (7 << 2) | 1),
    ];
    for (name, cm, p1d) in cases {
        let call = run_driver(&s2, cm, p1d, &sample, None, false).unwrap().call.expect("captured ctx");
        for (w, h) in [(0u32, 0u32), (0, 8), (8, 0), (2, 8), (8, 1)] {
            let sentinel = Ycc { w: 16, h: 8, p: [vec![0x5A; 16 * 8], vec![0x6B; 8 * 8], vec![0x7C; 8 * 8]] };
            let (mut e, planes, si) = emu_job(&s2, &sentinel);
            e.sdram[si].data.copy_from_slice(&call.scratch);
            // Set V=true on entry to verify entry guard does not rely on stale V after `and`.
            e.flags = (false, false, true, false);
            let hdr = [IMG_VA[0], IMG_VA[1], IMG_VA[2], w, h, 16, 8];
            for (i, v) in hdr.into_iter().chain(call.ctx[7..].iter().copied()).enumerate() {
                e.w32(EMU_CTX + 4 * i as u32, v);
            }
            e.r[4] = EMU_CTX;
            e.run(VA_YCC_SUB, 100_000).unwrap();
            assert_eq!(e.r[15], EMU_SP, "{name} ({w}x{h}): stack imbalance");
            let out = emu_out(&e, planes, &sentinel).unwrap();
            assert_eq!(out.p, sentinel.p, "{name} ({w}x{h}): wrote outside zero/sub-tile buffer");
        }
    }
}

/// ycc engine insns & SDRAM / ROM data accesses per pixel pair for every fx x fill combo; `s/L` = engine
/// seconds for a 4704x3136 capture at ~70M insns/s. `DP2M_REF_ASM=<dir>` (e.g. `git archive <rev> firmware/m5`)
/// also runs that engine & driver on the same ctx / input and diffs them; `DP2M_PROF=1` prints insns/pair per
/// engine block. `cargo test --release --offline ycc_bench -- --ignored --nocapture`
/// `DP2M_BENCH_STRESS=1` adds constant boundary frames and full-range noise;
/// `DP2M_REF_EXACT=1` makes any output difference against `DP2M_REF_ASM` fail.
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
    let exact = std::env::var_os("DP2M_REF_EXACT").is_some();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut labels = fr80::assemble_file_labels(&root.join("firmware/m5/ycc_engine.fr80"), VA_YCC_SUB, &HashMap::new()).unwrap().1;
    labels.insert(0, (VA_YCC_SUB, "<entry>".into()));
    let groups = [("color", COLOR_TEST_MODE, 0), ("color fill=1", COLOR_TEST_MODE, 1), ("color fill=2", COLOR_TEST_MODE, 2), ("color fill=3", COLOR_TEST_MODE, 3), ("B&W toner", BW_TEST_MODE, 0), ("B&W fill=2", BW_TEST_MODE, 2)];
    let mut sizes = vec![("mixed".to_string(), Ycc::synth(256, 128), Ycc::synth(256, 128)), ("strip".to_string(), Ycc::synth(FULL_W, 8), Ycc::synth(FULL_W, FULL_H))];
    if std::env::var_os("DP2M_BENCH_STRESS").is_some() {
        for level in [0, 1, 127, 128, 254, 255] {
            let mut img = Ycc::synth(64, 32);
            for plane in &mut img.p { plane.fill(level); }
            sizes.push((format!("constant-{level}"), img.clone(), img));
        }
        let mut img = Ycc::synth(256, 32);
        let mut seed = 0x1234_5678u32;
        for plane in &mut img.p {
            for pixel in plane {
                seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5;
                *pixel = seed as u8;
            }
        }
        sizes.push(("noise".to_string(), img.clone(), img));
    }
    if let Some(dir) = std::env::var_os("DP2M_BENCH_YCC") {
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ycc"))
            .collect();
        entries.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
        for path in entries {
            let raw = std::fs::read(&path).unwrap();
            let w = u32::from_le_bytes(raw[0..4].try_into().unwrap());
            let h = u32::from_le_bytes(raw[4..8].try_into().unwrap());
            let y_len = (w * h) as usize;
            let c_len = (w / 2 * h) as usize;
            assert_eq!(raw.len(), 8 + y_len + 2 * c_len, "bad ycc size: {}", path.display());
            let ycc = Ycc {
                w,
                h,
                p: [
                    raw[8..8 + y_len].to_vec(),
                    raw[8 + y_len..8 + y_len + c_len].to_vec(),
                    raw[8 + y_len + c_len..].to_vec(),
                ],
            };
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            sizes.push((stem, ycc.clone(), ycc));
        }
    }
    let l_pairs = f64::from(FULL_W * FULL_H) / 2.0;
    for (fixture, img, cap) in &sizes {
        let pairs = (img.w / 2 * img.h) as f64;
        println!("\n{} {}x{} ({} pairs)  insns/pair  sdram/pair  rom/pair   s/L  [ref insns  ref sdram  max|d|  mean|d|  (diff px)]", fixture, img.w, img.h, pairs);
        for (label, cm, fill) in groups {
            for fx in 0..8u32 {
                let p1d = (fill << 5) | (fx << 2) | 1;
                let shot = run_driver(&s2, cm, p1d, cap, None, false).unwrap();
                let Some(c) = shot.call else {
                    if let Some(r) = &refs {
                        assert!(run_driver(r, cm, p1d, cap, None, false).unwrap().call.is_none(),
                                "{label} fx={fx}: reference ran YCC while candidate did not");
                    }
                    println!("  {:<13} fx={}  stock HW JPEG", label, fx);
                    continue;
                };
                let mut hits = vec![];
                let (out, steps, mem) = run_engine(&s2, &c, img, prof.then_some(&mut hits)).unwrap();
                let per = |v: u64| v as f64 / pairs;
                let mut line = format!(
                    "  {:<13} fx={}  {:>9.1}  {:>9.2}  {:>8.2}  {:>5.1}",
                    label, fx, per(steps), per(mem[0]), per(mem[1]), per(steps) * l_pairs / 70e6
                );
                if let Some(r) = &refs {
                    let rc = run_driver(r, cm, p1d, cap, None, false).unwrap().call.unwrap();
                    assert!(rc.ctx[..21] == c.ctx[..21], "{} fx={}: driver ctx differ from ref", label, fx);
                    if c.ctx[19] != 0 {
                        assert!(rc.scratch[..0x1100] == c.scratch[..0x1100], "{} fx={}: halo grid differs", label, fx);
                    }
                    if c.ctx[20] != 0 {
                        assert!(rc.ctx[21] == c.ctx[21] && rc.scratch[0x2200..0x3300] == c.scratch[0x2200..0x3300], "{} fx={}: fill grid differs", label, fx);
                    }
                    let (rout, rsteps, rmem) = run_engine(r, &c, img, None).unwrap();
                    let diffs = out.p.iter().zip(&rout.p).flat_map(|(a, b)| a.iter().zip(b));
                    let (mut maxd, mut ndiff, mut sum) = (0, 0, 0u64);
                    for (&a, &b) in diffs {
                        let d = a.abs_diff(b);
                        maxd = maxd.max(d);
                        ndiff += usize::from(d != 0);
                        sum += u64::from(d);
                    }
                    let mean = sum as f64 / out.p.iter().map(Vec::len).sum::<usize>() as f64;
                    if exact {
                        assert_eq!(ndiff, 0, "{label} fx={fx}: output differs from reference");
                    }
                    line += &format!("  {:>9.1}  {:>9.2}  {:>6}  {:>7.4}  ({})", per(rsteps), per(rmem[0]), maxd, mean, ndiff);
                }
                println!("{}", line);
                if prof {
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
