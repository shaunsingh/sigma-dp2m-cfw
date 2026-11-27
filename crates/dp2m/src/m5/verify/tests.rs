use super::*;

fn firm() -> Vec<u8> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let fw = std::fs::read(root.join("data/DP2MV105.bin")).expect("data/DP2MV105.bin");
    let pack = FilmPack::load(&root.join("data/film_pack.bin")).unwrap();
    build_m5(&fw, &pack, &root.join("src/asm/m5"), &Config::default()).unwrap()
}

#[test]
fn pack_swar_signed_deltas() {
    let s2 = firm();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (_, labels) = fr80::assemble_file_labels(&root.join("src/asm/m5/ycc_engine.fr80"), VA_YCC_SUB, &HashMap::new()).unwrap();
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

/// ycc engine insns & SDRAM / ROM data accesses per pixel pair for every fx x fill combo; `s/L` = engine
/// seconds for a 4704x3136 capture at ~70M insns/s. `DP2M_REF_ASM=<dir>` (e.g. `git archive <rev> src/asm/m5`)
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
    let mut labels = fr80::assemble_file_labels(&root.join("src/asm/m5/ycc_engine.fr80"), VA_YCC_SUB, &HashMap::new()).unwrap().1;
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
