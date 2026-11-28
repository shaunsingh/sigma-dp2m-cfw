use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use dp2m_emu::sim::{Camera, Key, LCD_H, LCD_W};

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn build_probe_package() -> (Vec<u8>, u32, u32, u32, u32) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).expect("system clock").as_nanos();
    let out = TempDir(std::env::temp_dir().join(format!(
        "dp2m-probe-test-{}-{nonce}",
        std::process::id()
    )));
    std::fs::create_dir(&out.0).expect("create temp probe build dir");

    let res = Command::new(env!("CARGO_BIN_EXE_dp2m"))
        .current_dir(&root)
        .args(["build", "--config", "dp2m.probe.fnl", "--out"])
        .arg(&out.0)
        .env("SOURCE_DATE_EPOCH", "1767225600")
        .output()
        .expect("run dp2m builder for dp2m.probe.fnl");
    assert!(
        res.status.success(),
        "probe build failed:\n{}\n{}",
        String::from_utf8_lossy(&res.stdout),
        String::from_utf8_lossy(&res.stderr)
    );

    let pkg = std::fs::read(out.0.join("DP2MV200.bin")).expect("read probe DP2MV200.bin");
    let syms = std::fs::read_to_string(out.0.join("m6m/m6m.syms")).expect("read probe m6m.syms");
    for required in ["doom_upscale_2x2", "bench_w0_loads_arm"] {
        assert!(
            syms.lines().any(|line| line.ends_with(&format!(" {required}"))),
            "probe must retain {required}"
        );
    }
    for collected in ["emit_visplane", "zbuf", "job_ytop"] {
        assert!(
            !syms.lines().any(|line| line.ends_with(&format!(" {collected}"))),
            "unused portal renderer symbol {collected} must be garbage-collected"
        );
    }
    let sym = |name: &str| -> (u32, u32) {
        syms.lines()
            .find_map(|line| {
                let mut it = line.split_whitespace();
                let addr = u32::from_str_radix(it.next()?, 16).ok()?;
                let size = it.next()?.parse::<u32>().ok()?;
                let sname = it.next()?;
                (sname == name).then_some((addr, size))
            })
            .unwrap_or_else(|| panic!("missing symbol {name} in m6m.syms"))
    };
    let (bss_start, _) = sym("__bss_start");
    let (bss_end, _) = sym("__bss_end");
    let (pst_addr, pst_size) = sym("pst");
    (pkg, bss_start, bss_end, pst_addr, pst_size)
}

fn argb_pixel(cam: &Camera, x: usize, y: usize) -> u16 {
    let off = 0x021D_8C00 + (y * LCD_W + x) * 2;
    let s = cam.cpu().sdram();
    u16::from_be_bytes([s[off], s[off + 1]])
}

fn check_header_and_footer(cam: &Camera, label: &str) {
    // Header bar at rows 0..28 has background 0x125F and yellow text 0xFF2F
    let mut hdr_bg = 0usize;
    let mut hdr_txt = 0usize;
    for y in 0..28 {
        for x in 0..LCD_W {
            match argb_pixel(cam, x, y) {
                0x125F => hdr_bg += 1,
                0xFF2F => hdr_txt += 1,
                _ => {}
            }
        }
    }
    assert!(hdr_bg > 5_000, "{label}: expected header bar bg pixels, got {hdr_bg}");
    assert!(hdr_txt > 500, "{label}: expected legible 2x yellow header glyphs, got {hdr_txt}");

    // Footer bar at rows 438..480 has background 0x223F and white text 0xFFFF
    let mut ftr_txt = 0usize;
    for y in 438..LCD_H {
        for x in 0..LCD_W {
            if argb_pixel(cam, x, y) == 0xFFFF {
                ftr_txt += 1;
            }
        }
    }
    assert!(ftr_txt > 800, "{label}: expected legible 2x white footer glyphs, got {ftr_txt}");
}

#[test]
fn probe_applet_all_pages_actions_write_safety_and_clean_exit() {
    let (pkg, bss_start, bss_end, pst_addr, pst_size) = build_probe_package();
    assert_eq!(bss_start, 0xA1C0_0000);
    assert!(
        bss_end + 0x10000 <= 0xA1E0_0000,
        "probe .bss ({bss_end:#x}) must leave >= 64 KiB free in RAM for P6 survey"
    );

    let mut cam = Camera::from_package(&pkg).expect("boot probe firmware in simulator");
    // Hardware powers on with arbitrary payload RAM. Poison exactly the probe state after
    // the normal boot, preserving other modules' already-initialized state.
    let pst_off = (pst_addr - 0xA000_0000) as usize;
    cam.cpu_mut().sdram_mut()[pst_off..pst_off + pst_size as usize].fill(0xFF);
    assert_eq!(cam.screen_id(), 0x15);

    // Snapshot NVRAM before opening probe so we can prove zero NVRAM mutations
    let nv_before = cam.cpu().sdram()[0x0093_9684..0x0093_9684 + 0x3C4].to_vec();

    // Open Setup Page 4 -> DOOM (now PROBE)
    cam.press(Key::Menu);
    assert_eq!(cam.screen_id(), 0x1B);
    cam.press(Key::Left); // wraps from page 0 -> Setup 4 (0x17)
    for _ in 0..4 {
        cam.press(Key::Down);
    }
    cam.press(Key::Ok);
    assert!(
        cam.cpu().last_error.is_none(),
        "probe open fault (screen={}, pc={:#x}): {:?}",
        cam.screen_id(),
        cam.cpu().r[15],
        cam.cpu().last_error
    );
    assert_eq!(cam.screen_id(), 33, "Setup 4 -> DOOM must open SCREEN_DOOM (33)");

    // OSD must be cleared on entry, and header/footer/body must render cleanly on P1
    let osd = &cam.cpu().sdram()[0x0213_0000..0x0213_0000 + LCD_W * LCD_H];
    assert!(osd.iter().all(|&b| b == 0), "8bpp OSD plane must be cleared in PROBE");
    check_header_and_footer(&cam, "p1/10 initial");

    // Allowed write ranges while navigating/running read-only probes inside SCREEN_DOOM:
    //  - Simulator call stack around CALL_SP (0xA200_1000)
    //  - Probe own .bss (0xA1C0_0000..bss_end; strictly NOT beyond bss_end!)
    //  - ARGB_FB front & back (0xA21D_8C00..0xA230_4C00)
    //  - Stock display dirty rect struct (0xA07D_D754..0xA07D_D774)
    let base_allowed = [
        (0xA1FF_0000u32, 0xA200_2000u32), // simulator stack
        (bss_start, bss_end),              // own .bss only
        (0xA21D_8C00u32, 0xA230_4C00u32), // ARGB_FB front + back
        (0xA07D_D754u32, 0xA07D_D774u32), // stock display dirty rect
    ];

    let check_no_stray_writes = |log: &[(u32, u32)], label: &str| {
        let stray: Vec<u32> = log.iter().map(|&(va, _)| va).collect::<BTreeSet<_>>().into_iter().take(16).collect();
        assert!(
            stray.is_empty(),
            "{label}: unexpected writes outside allowed ranges: {:08x?}",
            stray
        );
    };

    // P1 INPUT: exercise extra keys (Up, Disp, Shutter, Ok) — must not capture or stray-write
    cam.cpu_mut().start_write_log(&base_allowed);
    let caps0 = cam.capture_count();
    let fb_p1_a = cam.render_lcd().to_vec();
    cam.press(Key::Up);
    cam.press(Key::Disp);
    cam.press(Key::Shutter);
    assert_eq!(cam.capture_count(), caps0, "Shutter in PROBE must not trigger photo capture");
    cam.press(Key::Ok);
    assert_eq!(cam.screen_id(), 33);
    assert!(cam.cpu().last_error.is_none(), "P1 keys fault: {:?}", cam.cpu().last_error);
    let fb_p1_b = cam.render_lcd().to_vec();
    assert_ne!(fb_p1_b, fb_p1_a, "P1 INPUT event log must update on key presses");
    let log = cam.cpu_mut().stop_write_log();
    check_no_stray_writes(&log, "P1 INPUT");

    // Step through pages p2..p10 with Key::Right, verifying each page renders distinctly
    let mut page_fbs = vec![fb_p1_b];

    // p2/10: P2 TIME/CPU (allow stock power-save inhibit state 0xA07D_D730..0xA07D_D744 during OK)
    cam.cpu_mut().start_write_log(&base_allowed);
    cam.press(Key::Right);
    assert!(cam.cpu().last_error.is_none(), "P2 nav fault: {:?}", cam.cpu().last_error);
    check_header_and_footer(&cam, "p2/10 idle");
    let fb_p2_idle = cam.render_lcd().to_vec();
    let log = cam.cpu_mut().stop_write_log();
    check_no_stray_writes(&log, "P2 nav");

    let mut p2_allowed = base_allowed.to_vec();
    p2_allowed.push((0xA07D_D730u32, 0xA07D_D744u32)); // stock power_save_inhibit/release state
    cam.cpu_mut().start_write_log(&p2_allowed);
    let p2_steps = cam.press(Key::Ok);
    assert!(cam.cpu().last_error.is_none(), "P2 benchmark fault: {:?}", cam.cpu().last_error);
    assert!(
        p2_steps > 42_000_000,
        "expected P2 to include direct portal-upscale work, got {p2_steps} steps"
    );
    assert_eq!(
        cam.cpu().r32_be(0xA07D_D73C),
        0,
        "power_save_release(probe mask) must restore inhibit mask to 0"
    );
    check_header_and_footer(&cam, "p2/10 results");
    let fb_p2_done = cam.render_lcd().to_vec();
    assert_ne!(fb_p2_done, fb_p2_idle, "P2 OK must display benchmark results");
    let log = cam.cpu_mut().stop_write_log();
    check_no_stray_writes(&log, "P2 benchmarks");
    page_fbs.push(fb_p2_done);

    // p3/11: P2B WRITE PATTERNS (640x424 ARGB_FB W1..W5 warm + cold D-cache, power-save inhibited)
    cam.cpu_mut().start_write_log(&base_allowed);
    cam.press(Key::Right);
    assert!(cam.cpu().last_error.is_none(), "P2B nav fault: {:?}", cam.cpu().last_error);
    check_header_and_footer(&cam, "p3/11 idle");
    let fb_p2b_idle = cam.render_lcd().to_vec();
    let log = cam.cpu_mut().stop_write_log();
    check_no_stray_writes(&log, "P2B nav");

    cam.cpu_mut().start_write_log(&p2_allowed);
    let p2b_steps = cam.press(Key::Ok);
    assert!(cam.cpu().last_error.is_none(), "P2B benchmark fault: {:?}", cam.cpu().last_error);
    // W0..W5 x 3 repetitions x 2 cache states x 8 passes exercises the new min-of-three path.
    assert!(
        p2b_steps > 40_000_000,
        "expected W0..W5 repeated warm+cold ARM loops on P2B, got {p2b_steps} steps"
    );
    assert_eq!(
        cam.cpu().r32_be(0xA07D_D73C),
        0,
        "P2B power_save_release(probe mask) must restore inhibit mask to 0"
    );
    check_header_and_footer(&cam, "p3/11 results");
    let fb_p2b_done = cam.render_lcd().to_vec();
    assert_ne!(fb_p2b_done, fb_p2b_idle, "P2B OK must display W0..W5 timing table");
    let log = cam.cpu_mut().stop_write_log();
    check_no_stray_writes(&log, "P2B write patterns");
    page_fbs.push(fb_p2b_done);

    // p4/11: P3A MEM POOLS -> run Pass 1 and Pass 2 of 248-window SDRAM scan
    cam.cpu_mut().start_write_log(&base_allowed);
    cam.press(Key::Right);
    check_header_and_footer(&cam, "p4/11 idle");
    let fb_p3_idle = cam.render_lcd().to_vec();
    cam.press(Key::Ok); // Pass 1
    assert!(cam.cpu().last_error.is_none(), "P3 pass 1 fault: {:?}", cam.cpu().last_error);
    let fb_p3_p1 = cam.render_lcd().to_vec();
    assert_ne!(fb_p3_p1, fb_p3_idle, "P3 Pass 1 must update summary");
    cam.press(Key::Ok); // Pass 2
    assert!(cam.cpu().last_error.is_none(), "P3 pass 2 fault: {:?}", cam.cpu().last_error);
    let fb_p3_p2 = cam.render_lcd().to_vec();
    assert_ne!(fb_p3_p2, fb_p3_p1, "P3 Pass 2 must increment pass counter / diff");
    page_fbs.push(fb_p3_p2);

    // p5/11..p8/11: P3B..P3E SDRAM tables
    for p in 5..=8 {
        cam.press(Key::Right);
        assert!(cam.cpu().last_error.is_none(), "p{p}/11 fault: {:?}", cam.cpu().last_error);
        check_header_and_footer(&cam, &format!("p{p}/11"));
        page_fbs.push(cam.render_lcd().to_vec());
    }

    // p9/11: P4 SD STORAGE -> press OK (graceful NO FILE in sim, zero fault, zero stray writes)
    cam.press(Key::Right);
    check_header_and_footer(&cam, "p9/11 idle");
    let fb_p4_idle = cam.render_lcd().to_vec();
    cam.press(Key::Ok);
    assert!(cam.cpu().last_error.is_none(), "P4 SD action fault: {:?}", cam.cpu().last_error);
    let fb_p4_done = cam.render_lcd().to_vec();
    assert_ne!(fb_p4_done, fb_p4_idle, "P4 OK must display OPEN result / NO FILE");
    page_fbs.push(fb_p4_done);

    // p10/11: P5 DISPLAY -> press OK refresh (strictly read-only, zero MMIO writes)
    cam.press(Key::Right);
    check_header_and_footer(&cam, "p10/11");
    cam.press(Key::Ok);
    assert!(cam.cpu().last_error.is_none(), "P5 display fault: {:?}", cam.cpu().last_error);
    page_fbs.push(cam.render_lcd().to_vec());

    // p11/11: P6 BOOT RAM -> press OK rescan (64 KiB beyond __bss_end untouched)
    cam.press(Key::Right);
    check_header_and_footer(&cam, "p11/11");
    cam.press(Key::Ok);
    assert!(cam.cpu().last_error.is_none(), "P6 boot ram fault: {:?}", cam.cpu().last_error);
    page_fbs.push(cam.render_lcd().to_vec());

    // Verify DialRight wraps p11/11 -> p1/11 and DialLeft wraps p1/11 -> p11/11
    cam.press(Key::DialRight);
    check_header_and_footer(&cam, "wrap to p1/11");
    cam.press(Key::DialLeft);
    check_header_and_footer(&cam, "wrap back to p11/11");

    let log = cam.cpu_mut().stop_write_log();
    check_no_stray_writes(&log, "P3..P6 + wrap navigation");

    // Verify all 11 pages rendered distinct framebuffers
    assert_eq!(page_fbs.len(), 11);
    for i in 0..page_fbs.len() {
        for j in (i + 1)..page_fbs.len() {
            assert_ne!(page_fbs[i], page_fbs[j], "pages p{} and p{} rendered identically", i + 1, j + 1);
        }
    }

    // Verify zero writes occurred in the 64 KiB beyond __bss_end
    let tail_off = (bss_end - 0xA000_0000) as usize;
    assert!(
        cam.cpu().sdram()[tail_off..tail_off + 0x10000].iter().all(|&b| b == 0),
        "probe must never write beyond __bss_end ({bss_end:#x})"
    );

    // Verify NVRAM is byte-identical (zero NOR/NVRAM writes)
    let nv_after = &cam.cpu().sdram()[0x0093_9684..0x0093_9684 + 0x3C4];
    assert_eq!(nv_after, &nv_before[..], "probe must never mutate NVRAM");

    // Exit via Menu -> clean return to Live View (0x15)
    cam.press(Key::Menu);
    assert_eq!(cam.screen_id(), 0x15, "Key::Menu in PROBE must exit to SCREEN_LV");
    assert!(cam.cpu().last_error.is_none(), "PROBE Menu exit fault: {:?}", cam.cpu().last_error);
    let argb_top = &cam.cpu().sdram()[0x021D_8C00..0x021D_8C00 + LCD_W * 120 * 2];
    assert!(argb_top.iter().all(|&b| b == 0), "ARGB_FB top 120 rows must be cleared on PROBE exit");
    let osd_after = &cam.cpu().sdram()[0x0213_0000..0x0213_0000 + LCD_W * LCD_H];
    let lv_osd_nz = osd_after.iter().filter(|&&b| b != 0).count();
    assert!(lv_osd_nz > 500 && lv_osd_nz < 40_000, "Live View OSD must resume cleanly: {lv_osd_nz}");

    // Re-open PROBE (Menu remembers Setup 4 tab; press Down x4 + Ok) and confirm Key::Play exits cleanly
    cam.press(Key::Menu);
    assert_eq!(cam.screen_id(), 0x1B);
    cam.press(Key::Ok); // if cursor stayed on DOOM, enters directly; otherwise navigate down
    if cam.screen_id() != 33 {
        for _ in 0..4 {
            cam.press(Key::Down);
        }
        cam.press(Key::Ok);
    }
    assert_eq!(cam.screen_id(), 33, "re-opening PROBE must enter SCREEN_DOOM (33)");
    cam.press(Key::Play);
    assert_eq!(cam.screen_id(), 0x15, "Key::Play in PROBE must exit to SCREEN_LV");
    assert!(cam.cpu().last_error.is_none(), "PROBE Play exit fault: {:?}", cam.cpu().last_error);

    println!(
        "PROBE simulator flow PASS: 11/11 pages distinct, p2_steps={p2_steps}, p2b_steps={p2b_steps}, bss={bss_start:#x}..{bss_end:#x}, zero stray writes, clean Menu/Play exit"
    );
}

#[test]
#[ignore]
fn dump_probe_previews() {
    let (pkg, ..) = build_probe_package();
    let mut cam = Camera::from_package(&pkg).expect("boot probe firmware");
    let dir = std::env::temp_dir().join("dp2m-probe-previews");
    std::fs::create_dir_all(&dir).expect("create preview dir");
    let write_ppm = |name: &str, rgba: &[u8]| {
        let mut out = format!("P6\n{LCD_W} {LCD_H}\n255\n").into_bytes();
        for p in rgba.as_chunks::<4>().0 {
            out.extend_from_slice(&p[..3]);
        }
        std::fs::write(dir.join(name), out).expect("write ppm");
    };

    cam.press(Key::Menu);
    cam.press(Key::Left);
    for _ in 0..4 {
        cam.press(Key::Down);
    }
    cam.press(Key::Ok);
    cam.press(Key::Up);
    write_ppm("p01_input.ppm", cam.render_lcd());

    cam.press(Key::Right);
    cam.press(Key::Ok);
    write_ppm("p02_time_cpu.ppm", cam.render_lcd());

    cam.press(Key::Right);
    cam.press(Key::Ok);
    write_ppm("p03_write_patterns.ppm", cam.render_lcd());

    cam.press(Key::Right);
    cam.press(Key::Ok);
    cam.press(Key::Ok);
    write_ppm("p04_mem_pools.ppm", cam.render_lcd());

    cam.press(Key::Right);
    write_ppm("p05_ram_a08.ppm", cam.render_lcd());

    for _ in 0..4 {
        cam.press(Key::Right);
    }
    cam.press(Key::Ok);
    write_ppm("p09_sd.ppm", cam.render_lcd());

    cam.press(Key::Right);
    write_ppm("p10_display.ppm", cam.render_lcd());

    cam.press(Key::Right);
    cam.press(Key::Ok);
    write_ppm("p11_boot_ram.ppm", cam.render_lcd());
}
