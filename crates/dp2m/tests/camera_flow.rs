mod common;

use std::sync::OnceLock;

use dp2m_emu::sim::{Camera, Key, LCD_H, LCD_W};

fn pkg_bytes() -> &'static [u8] {
    static PACKAGE: OnceLock<Vec<u8>> = OnceLock::new();
    PACKAGE.get_or_init(|| {
        common::with_fresh_build(|out, _| {
            std::fs::read(out.join("DP2MV200.bin")).expect("read freshly built camera-flow package")
        })
    })
}

#[test]
fn full_dual_cpu_camera_flow() {
    let pkg = pkg_bytes();
    let mut cam = Camera::from_package(pkg).expect("boot DP2M simulator");

    // 1. Boot into Live View with status bar OSD
    assert_eq!(cam.screen_id(), 0x15);
    assert_eq!(cam.color_mode(), 3); // Standard
    let fb0 = cam.render_lcd().to_vec();
    assert_eq!(fb0.len(), LCD_W * LCD_H * 4);
    let nonblack = fb0.as_chunks::<4>().0.iter().filter(|p| p[0] > 0 || p[1] > 0 || p[2] > 0).count();
    assert!(nonblack > 200_000, "expected live view + OSD pixels, got {nonblack}");

    // 2. Exposure Mode strip (10 modes including F1 = 9, F2 = 10)
    cam.press(Key::Mode);
    assert_eq!(cam.screen_id(), 0x17);
    let mut seen_f1 = false;
    for _ in 0..12 {
        cam.press(Key::DialRight);
        if cam.expo_mode() == 9 {
            seen_f1 = true;
            break;
        }
    }
    assert!(seen_f1, "MODE strip must step into custom slot F1 (expo_mode == 9)");
    cam.press(Key::Mode);
    assert_eq!(cam.screen_id(), 0x15);

    // 3. Quick Set (QS) menu + custom BP4 label icons
    cam.press(Key::Qs);
    assert_eq!(cam.screen_id(), 0x1A);
    let qs_fb = cam.render_lcd().to_vec();
    assert_ne!(qs_fb, fb0, "QS screen must composite distinct OSD over Live View");
    cam.press(Key::Qs); // toggle QS page 2
    assert_eq!(cam.screen_id(), 0x1A);
    cam.press(Key::Ok); // confirm & close QS back to LV
    assert_eq!(cam.screen_id(), 0x15);

    // 4. Main Menu -> Film page -> select film & enable 3D LUT + Grain + Halation + X3 Fill
    cam.press(Key::Menu);
    assert_eq!(cam.screen_id(), 0x1B);
    let menu_fb = cam.render_lcd().to_vec();
    assert_ne!(menu_fb, fb0, "Main Menu must render full-screen UI");
    // Step right to Film page (page 2)
    cam.press(Key::Right);
    cam.press(Key::Right);
    let film_tab_fb = cam.render_lcd().to_vec();
    assert_ne!(film_tab_fb, menu_fb, "Film page must render distinct rows");
    cam.press(Key::Menu);
    assert_eq!(cam.screen_id(), 0x15);

    // 5. Select a reversal film (cm = 19 -> Velvia 100) + fx=7 + fill=1 and test LV + HUD overlays
    cam.cpu_mut().w8(0xA093_9689, 1);  // 3D spectral engine ON
    cam.cpu_mut().w8(0xA093_968D, 1);  // grain ON
    cam.cpu_mut().w8(0xA093_968E, 1);  // halation ON
    cam.cpu_mut().w8(0xA093_968A, 1);  // X3 Fill +0.3
    cam.set_color_mode(19);

    // Exercise custom HUD overlays (7=Level, 8=XPan+Level, 11=Peaking, 13=Zebras, 14=RGB Histogram, 17=False Color)
    for grid in [7u32, 8, 11, 13, 14, 17] {
        cam.cpu_mut().sdram_mut()[0x021D_8C00..0x021D_8C00 + LCD_W * LCD_H * 2].fill(0);
        cam.set_grid_mode(grid);
        assert_eq!(cam.grid_mode(), grid);
        cam.set_tilt(16, 64);
        let argb = &cam.cpu().sdram()[0x021D_8C00..0x021D_8C00 + LCD_W * LCD_H * 2];
        let nonzero = argb.iter().filter(|&&b| b != 0).count();
        assert!(nonzero > 0, "RGBA4444 HUD plane must be non-zero for grid {grid}");
        if grid == 17 {
            assert!(nonzero > 50_000, "False Color (grid 17) must paint IRE heatmap + legend, got {nonzero}");
            let swatch_off = (152 * LCD_W + 624) * 2;
            let swatch = u16::from_be_bytes([argb[swatch_off], argb[swatch_off + 1]]);
            assert_eq!(swatch, 0xFE1F, "False Color legend 88..94% warning swatch at (624,152) must be 0xFE1F");
        }
        let _ = cam.render_lcd();
    }
    // Dual Hi+Lo Zebras (NV_ZEBRALVL = 3) must render highlight and/or shadow stripes cleanly
    cam.cpu_mut().w8(0xA093_9692, 3);
    cam.set_color_mode(13);
    cam.set_grid_mode(13);
    let zdual = cam.cpu().sdram()[0x021D_8C00..0x021D_8C00 + LCD_W * LCD_H * 2].to_vec();
    assert!(zdual.iter().any(|&b| b != 0), "Dual Hi+Lo zebras must render active overlay pixels");
    cam.cpu_mut().w8(0xA093_9692, 0);
    // Teal & Orange (cm=13) preserves stock linear highlight headroom so the sun disc clips >= 95% IRE
    cam.set_color_mode(13);
    cam.set_grid_mode(13);
    let z0 = cam.cpu().sdram()[0x021D_8C00..0x021D_8C00 + LCD_W * LCD_H * 2].to_vec();
    assert!(z0.iter().any(|&b| b != 0), "sun disc must trigger 95% IRE zebras");
    assert!(cam.tick(), "tick() must animate diagonal zebras when grid == 13");
    let z1 = &cam.cpu().sdram()[0x021D_8C00..0x021D_8C00 + LCD_W * LCD_H * 2];
    assert_ne!(&z0[..], z1, "zebra phase must advance on tick()");
    // Transitioning Zebras (13) -> Electronic Level (7) -> Off (0) must leave zero stale overlay pixels
    cam.set_grid_mode(7);
    let lvl_argb = &cam.cpu().sdram()[0x021D_8C00..0x021D_8C00 + LCD_W * 60 * 2];
    assert!(lvl_argb.iter().all(|&b| b == 0), "top 60 rows must be wiped clean on Zebras -> Level");
    cam.set_grid_mode(0);
    let off_argb = &cam.cpu().sdram()[0x021D_8C00..0x021D_8C00 + LCD_W * 424 * 2];
    // Level horizon bars (x=211, y=211 and x=407, y=211) and outer frame must be wiped clean,
    // while the stock center AF bracket around (292, 193) remains intact on ARGB_FB
    let px = |x: usize, y: usize| u16::from_be_bytes([off_argb[(y * LCD_W + x) * 2], off_argb[(y * LCD_W + x) * 2 + 1]]);
    assert!(off_argb[..LCD_W * 180 * 2].iter().all(|&b| b == 0), "upper 180 rows must be wiped clean on Level -> Off");
    assert!(off_argb[LCD_W * 244 * 2..].iter().all(|&b| b == 0), "lower rows 244..424 must be wiped clean on Level -> Off");
    assert_eq!((px(212, 211), px(410, 211)), (0, 0), "Electronic Level horizon bars must be wiped clean on Level -> Off");
    assert_ne!(px(292, 193), 0, "stock center AF bracket at (292, 193) must remain intact");
    cam.set_color_mode(19);

    // 6. Full Dual-CPU Shutter Capture (M6M ipc_pack + mn_film -> M5Slave FR80 spatial_driver + ycc_engine)
    let insns = cam.press(Key::Shutter);
    assert!(insns > 500_000, "expected real FR80 pyramid + YCC engine execution, got {insns}");
    assert!(cam.in_playback());
    let cap = cam.last_capture().expect("capture recorded").clone();
    assert!(cap.packed_cm >= 9, "expected packed film mode, got {}", cap.packed_cm);
    assert_eq!(cap.p1d, (1 << 5) | (7 << 2) | 1); // fill=1, fx=7, cs=1 (sRGB)
    assert_eq!(cap.tag3d, "Film 10");
    assert_eq!(cap.tag59, "Velvia 100");
    assert!(cap.m5_insns > 500_000);

    // Exit playback via Play key
    cam.press(Key::Play);
    assert!(!cam.in_playback());

    // 7. Mount a custom D2ML binary into D:\LUT1.BIN and capture with SD LUT slot 1
    let mut d2ml = vec![0u8; 0x1040];
    d2ml[0..4].copy_from_slice(b"D2ML");
    d2ml[4] = 1;  // version
    d2ml[5] = 24; // grain_amp
    d2ml[6] = 1;  // slide grain
    d2ml[7] = 80; // halo_gain
    d2ml[8] = 0;  // crimson tint
    for b in &mut d2ml[0x10..0x10 + 4131] {
        *b = 8u8; // +4 LSB warm shift
    }
    assert!(cam.set_sd_lut(1, d2ml));
    cam.press(Key::Shutter);
    let sd_cap = cam.last_capture().expect("SD LUT capture");
    assert_eq!(sd_cap.packed_cm, 191);
    assert_eq!(sd_cap.tag59, "SD Custom LUT");
    assert!(sd_cap.m5_insns > 500_000);
    assert_ne!(sd_cap.rgba, cap.rgba, "SD D2ML capture must produce distinct pixels");
}

#[test]
fn unicorn_bit_exact_film_menu_osd() {
    let pkg = pkg_bytes();
    let mut cam = Camera::from_package(pkg).expect("boot DP2M simulator");
    cam.press(Key::Menu);
    cam.press(Key::Right);
    cam.press(Key::Right);
    let osd = &cam.cpu().sdram()[0x0213_0000..0x0213_0000 + LCD_W * LCD_H];
    let mut fnv = 0xcbf2_9ce4_8422_2325u64;
    for &b in osd {
        fnv ^= b as u64;
        fnv = fnv.wrapping_mul(0x0000_0100_0000_01b3);
    }
    println!("arm9 OSD FNV1a64 = {fnv:#018x}");
    assert_eq!(fnv, 0x16a0_6c94_433a_64d3, "arm9 OSD must be 100% bit-exact with Unicorn");

    // Closing Menu via 2D DMA rect fill must clear the menu backdrop back to Live View OSD
    let menu_nz = osd.iter().filter(|&&b| b != 0).count();
    cam.press(Key::Menu);
    assert_eq!(cam.screen_id(), 0x15);
    let lv_nz = cam.cpu().sdram()[0x0213_0000..0x0213_0000 + LCD_W * LCD_H]
        .iter()
        .filter(|&&b| b != 0)
        .count();
    assert!(menu_nz > 200_000 && lv_nz < 40_000, "menu_nz={menu_nz}, lv_nz={lv_nz}");
}

fn mean_lv_rgb(fb: &[u8]) -> [f64; 3] {
    // Sample central Live View / Playback image region away from top/bottom OSD bars
    let mut sum = [0u64; 3];
    let mut n = 0u64;
    for y in 64..360 {
        for x in 48..592 {
            let i = (y * LCD_W + x) * 4;
            sum[0] += fb[i] as u64;
            sum[1] += fb[i + 1] as u64;
            sum[2] += fb[i + 2] as u64;
            n += 1;
        }
    }
    [
        sum[0] as f64 / n as f64,
        sum[1] as f64 / n as f64,
        sum[2] as f64 / n as f64,
    ]
}

#[test]
fn menus_exposure_wb_and_native_playback_views() {
    let pkg = pkg_bytes();
    let mut cam = Camera::from_package(pkg).expect("boot DP2M simulator");

    // 0. Empty-card Playback ("No Image") enters native SCREEN_PB (0x19) and exits cleanly
    cam.press(Key::Play);
    assert_eq!(cam.screen_id(), 0x19);
    assert!(cam.in_playback());
    assert!(cam.cpu().last_error.is_none(), "{:?}", cam.cpu().last_error);
    let empty_pb_fb = cam.render_lcd().to_vec();
    cam.press(Key::Play);
    assert_eq!(cam.screen_id(), 0x15);
    assert!(!cam.in_playback());

    // 1. Main Menu Tab 0 scrolling, Tab 2 Color Mode (32-item popup) + Print Paper (9 options), Tab 3 row 5
    cam.press(Key::Menu);
    assert_eq!(cam.screen_id(), 0x1B);
    // Scroll down Tab 0 past row 6 to verify realloc-backed list + bottom Cancel/Select bar stay intact
    for _ in 0..7 {
        cam.press(Key::Down);
    }
    assert!(cam.cpu().last_error.is_none(), "Tab 0 scroll hit error: {:?}", cam.cpu().last_error);
    let osd_tab0 = &cam.cpu().sdram()[0x0213_0000..0x0213_0000 + LCD_W * LCD_H];
    let footer_nz = osd_tab0[444 * LCD_W..476 * LCD_W].iter().filter(|&&b| b != 0).count();
    assert!(footer_nz > 1_500, "Tab 0 bottom Cancel/Select bar missing: {footer_nz}");

    // Navigate via DialRight to Tab 2 (Film page) -> focus row 0 -> open Color Mode 32-item popup -> select Color Neg film
    cam.press(Key::DialRight);
    cam.press(Key::DialRight);
    cam.press(Key::Down); // focus row 0 (Color Mode)
    cam.press(Key::Right); // open Color Mode 32-item submenu
    assert!(cam.cpu().last_error.is_none(), "Color Mode popup hit step limit: {:?}", cam.cpu().last_error);
    for _ in 0..20 {
        cam.press(Key::Down);
    }
    cam.press(Key::Ok); // commit Color Negative film
    assert!(cam.color_mode() >= 23, "expected Color Negative film selected, got {}", cam.color_mode());

    // Open Print Paper (row 1): paper_cls() == 1 must expose all 9 paper choices (Auto + 8 stocks)
    cam.press(Key::Down);
    cam.press(Key::Right);
    for _ in 0..5 {
        cam.press(Key::Down);
    }
    cam.press(Key::Ok);
    assert_eq!(cam.cpu().r8(0xA093_9688), 5, "Print Paper submenu must update NV_COLOR_PAPER to 5");

    // Step via DialRight to Tab 3 (Shoot page 3 = 0x10): verify 5th row (Peaking Color) renders in y=340..395
    cam.press(Key::DialRight);
    assert!(cam.cpu().last_error.is_none(), "Tab 3 hit error: {:?}", cam.cpu().last_error);
    let osd_tab3 = &cam.cpu().sdram()[0x0213_0000..0x0213_0000 + LCD_W * LCD_H];
    let row5_text_nz = osd_tab3[348 * LCD_W..392 * LCD_W]
        .chunks(LCD_W)
        .map(|row| row[60..580].iter().filter(|&&b| b != 0 && b != osd_tab3[348 * LCD_W + 20]).count())
        .sum::<usize>();
    assert!(row5_text_nz > 400, "Tab 3 row 5 (Peaking Color) missing: {row5_text_nz}");
    cam.press(Key::Menu);
    assert_eq!(cam.screen_id(), 0x15);

    // 2. Print Paper & Film Mode visibly change Live View B2Y output
    let lv_paper5 = cam.render_lcd().to_vec();
    cam.write_u8(0xA093_9688, 1); // Paper 1
    let lv_paper1 = cam.render_lcd().to_vec();
    assert_ne!(mean_lv_rgb(&lv_paper1), mean_lv_rgb(&lv_paper5), "Print Paper must alter Live View look");

    // 3. Native AE Exposure Compensation in P mode (DP2M Left/Right arrows adjust +/- EV in P/A/S)
    for _ in 0..16 {
        if cam.expo_mode() == 3 {
            break;
        }
        cam.press(Key::Mode);
        cam.press(Key::DialRight);
        cam.press(Key::Mode);
    }
    assert_eq!(cam.expo_mode(), 3, "MODE dial must reach P mode (3)");
    let base_rgb = mean_lv_rgb(cam.render_lcd());
    for _ in 0..4 {
        cam.press(Key::Right); // +1.3 EV in P mode
    }
    let plus_rgb = mean_lv_rgb(cam.render_lcd());
    for _ in 0..8 {
        cam.press(Key::Left); // -1.3 EV in P mode
    }
    let minus_rgb = mean_lv_rgb(cam.render_lcd());
    for _ in 0..4 {
        cam.press(Key::Right); // back to 0.0 EV
    }
    assert!(
        plus_rgb[1] > base_rgb[1] + 8.0 && minus_rgb[1] + 8.0 < base_rgb[1],
        "Native AE EV comp must brighten/darken Live View: minus={minus_rgb:?} base={base_rgb:?} plus={plus_rgb:?}"
    );

    // 4. Auto-ISO + Custom Min Shutter Speed hook (`autoiso_p_hook` @ 0xA093968B)
    let indoor_scene = vec![110u8; LCD_W * 424 * 4];
    cam.set_scene_rgba(&indoor_scene, LCD_W, 424);
    cam.write_u8(0xA093_96DC, 14); // Auto-ISO Max = ISO 1600
    cam.write_u8(0xA093_968B, 0); // stock min shutter (1/60s knee)
    cam.write_u8(0xA093_96DA, 0); // Auto-ISO
    let sv_stock_min = cam.cpu().r32_be(0xA080_0908) as i32;
    let tv_stock_min = cam.cpu().r16_be(0xA07D_D6E4);
    cam.write_u8(0xA093_968B, 6); // 1/500s fast min shutter -> forces faster Tv and higher Auto-ISO
    let sv_fast_min = cam.cpu().r32_be(0xA080_0908) as i32;
    let tv_fast_min = cam.cpu().r16_be(0xA07D_D6E4);
    assert!(
        tv_fast_min > tv_stock_min && sv_fast_min > sv_stock_min,
        "Min Shutter hook must speed up Tv and raise Auto-ISO: tv {tv_stock_min}->{tv_fast_min}, sv {sv_stock_min:#x}->{sv_fast_min:#x}"
    );

    // Restore bright scene & ISO 100
    let mut colorful_scene = vec![0u8; LCD_W * 424 * 4];
    for y in 0..424 {
        for x in 0..LCD_W {
            let i = (y * LCD_W + x) * 4;
            colorful_scene[i] = ((x * 220) / LCD_W + 20) as u8;
            colorful_scene[i + 1] = ((y * 200) / 424 + 30) as u8;
            colorful_scene[i + 2] = (180 - (x * 120) / LCD_W) as u8;
            colorful_scene[i + 3] = 255;
        }
    }
    cam.write_u8(0xA093_96DA, 2); // ISO 100
    cam.set_scene_rgba(&colorful_scene, LCD_W, 424);

    // 5. White Balance shifts R/B balance in both Live View B2Y CCMatrix and M5 Capture
    cam.write_u8(0xA093_96C4, 3); // Shade (warm)
    let shade_lv = mean_lv_rgb(cam.render_lcd());
    cam.press(Key::Shutter); // Capture #1 (Shade WB, Portra)
    assert_eq!(cam.screen_id(), 0x19);
    let cap1 = cam.last_capture().unwrap().clone();
    cam.press(Key::Play); // return to LV
    assert_eq!(cam.screen_id(), 0x15);

    cam.write_u8(0xA093_96C4, 6); // Incandescent (cool)
    let incand_lv = mean_lv_rgb(cam.render_lcd());
    assert!(
        shade_lv[0] - shade_lv[2] > incand_lv[0] - incand_lv[2] + 10.0,
        "WB Shade vs Incandescent must shift Live View R-B balance: shade={shade_lv:?}, incand={incand_lv:?}"
    );

    // Switch to B&W film (Kodak Tri-X 400 = cm 39) & Contrast +5, take Capture #2
    cam.set_color_mode(39);
    cam.write_u8(0xA093_96D5, 9); // high contrast
    let bw_lv = mean_lv_rgb(cam.render_lcd());
    assert!(
        (bw_lv[0] - bw_lv[1]).abs() < 3.0 && (bw_lv[1] - bw_lv[2]).abs() < 3.0,
        "B&W film must render neutral monochrome in Live View, got {bw_lv:?}"
    );
    cam.press(Key::Shutter); // Capture #2 -> enters native SCREEN_PB (0x19)
    assert_eq!(cam.screen_id(), 0x19);
    assert!(cam.in_playback());
    assert_eq!(cam.capture_count(), 2);
    let cap2 = cam.last_capture().unwrap().clone();
    assert_ne!(cap1.rgba, cap2.rgba);

    // 6. Native SCREEN_PB (0x19): opens newest capture (#2), updates DCF dir count, and DISP cycles all 4 views
    assert_eq!(cam.cpu().r32_be(0xA07F_220C), 2, "Post-capture PB must open newest capture #2");
    assert_eq!(cam.cpu().r32_be(0xA096_2758), 2, "DCF directory 100 image count must equal 2");
    let mut pb_views = Vec::new();
    let mut pb_osd_counts = Vec::new();
    let mut detail_idx = None;
    for step in 0..4 {
        let fb = cam.render_lcd().to_vec();
        assert!(fb != empty_pb_fb, "PB view {step} must not equal empty PB screen");
        let osd_nz = cam.cpu().sdram()[0x0213_0000..0x0213_0000 + LCD_W * LCD_H]
            .iter()
            .filter(|&&b| b != 0)
            .count();
        if osd_nz > 25_000 && detail_idx.is_none_or(|prev| osd_nz > pb_osd_counts[prev]) {
            detail_idx = Some(step);
        }
        pb_osd_counts.push(osd_nz);
        pb_views.push(fb);
        cam.press(Key::Disp);
        assert_eq!(cam.screen_id(), 0x19);
        assert!(cam.cpu().last_error.is_none(), "PB DISP hit error: {:?}", cam.cpu().last_error);
    }
    assert!(pb_osd_counts.contains(&0), "PB DISP cycle must include clean full-frame view (0 OSD px): {pb_osd_counts:?}");
    let detail_fb = &pb_views[detail_idx.expect("PB DISP cycle must include layout=5 detail view")];
    // Verify Y histogram box (x=420..610, y=58..98) has a genuine curve (both bright bar pixels and dark sky above bars)
    let mut hist_bright = 0usize;
    let mut hist_dark = 0usize;
    for y in 58..98 {
        for x in 420..610 {
            let p = (y * LCD_W + x) * 4;
            if detail_fb[p] > 180 && detail_fb[p + 1] > 180 && detail_fb[p + 2] > 180 {
                hist_bright += 1;
            } else if detail_fb[p] < 40 && detail_fb[p + 1] < 40 && detail_fb[p + 2] < 40 {
                hist_dark += 1;
            }
        }
    }
    assert!(
        hist_bright > 300 && hist_dark > 1_000,
        "Y histogram must render real distribution (not solid NaN clamp): bright={hist_bright}, dark={hist_dark}"
    );
    for i in 0..4 {
        let next = (i + 1) % 4;
        assert!(pb_views[i] != pb_views[next], "PB DISP view {i} and {next} must differ");
    }
    // After 4 DISP presses, view wraps back to view 0
    assert!(cam.render_lcd() == &pb_views[0][..], "4 DISP presses must wrap around all 4 PB layouts");

    // Step between DCF images with Left/Right
    let img2_fb = cam.render_lcd().to_vec();
    cam.press(Key::Left);
    let img1_fb = cam.render_lcd().to_vec();
    assert!(img1_fb != img2_fb, "Key::Left in Playback must step to previous captured photo");
    cam.press(Key::Right);
    assert!(cam.render_lcd() == &img2_fb[..], "Key::Right must step forward back to photo #2");

    // DialRight zooms in, DialLeft zooms back out
    cam.press(Key::DialRight);
    let zoomed_fb = cam.render_lcd().to_vec();
    assert!(zoomed_fb != img2_fb, "Key::DialRight in Playback must zoom into photo");
    cam.press(Key::DialLeft);

    // Menu in Playback opens Playback Menu (0x1B) and Menu returns to Playback (0x19)
    cam.press(Key::Menu);
    assert_eq!(cam.screen_id(), 0x1B);
    cam.press(Key::Menu);
    assert_eq!(cam.screen_id(), 0x19);

    // Half-press Shutter in Playback returns cleanly to Live View (0x15)
    cam.press(Key::Shutter);
    assert_eq!(cam.screen_id(), 0x15);
    assert!(!cam.in_playback());
}

#[test]
fn doom_e1m1_full_gameplay_and_clean_exit() {
    let pkg = pkg_bytes();
    let mut cam = Camera::from_package(pkg).expect("boot DP2M simulator");
    assert_eq!(cam.screen_id(), 0x15);

    // 1. Navigate Menu -> Setup Page 4 (page 10, wrap Left from page 0) -> Row 3 (DOOM) -> Ok
    cam.press(Key::Menu);
    assert_eq!(cam.screen_id(), 0x1B);
    cam.press(Key::Left); // wraps from page 0 -> page 10 (Setup 4 = 0x17)
    // Rows on Setup 4: 0 Firmware Update, 1 Camera Reset, 2 Imager Temp, 3 DOOM
    for _ in 0..4 {
        cam.press(Key::Down);
    }
    let open_insns = cam.press(Key::Ok);
    assert_eq!(cam.screen_id(), 33, "Setup 4 -> DOOM must open SCREEN_DOOM (33 = 0x21)");
    assert!(cam.cpu().last_error.is_none(), "DOOM open hit CPU error: {:?}", cam.cpu().last_error);

    // 2. Verify OSD was cleared and 640x480 RGBA4444 ARGB_FB has rich non-black Doom rendering
    let osd = &cam.cpu().sdram()[0x0213_0000..0x0213_0000 + LCD_W * LCD_H];
    assert!(osd.iter().all(|&b| b == 0), "8bpp OSD plane must be cleared during DOOM");

    let fb0 = cam.render_lcd().to_vec();
    assert_eq!(fb0.len(), LCD_W * LCD_H * 4);
    let mut unique_rgb = std::collections::HashSet::new();
    let mut nonblack = 0usize;
    for p in fb0.as_chunks::<4>().0 {
        if p[0] > 0 || p[1] > 0 || p[2] > 0 {
            nonblack += 1;
        }
        unique_rgb.insert([p[0] >> 4, p[1] >> 4, p[2] >> 4]);
    }
    assert!(nonblack > 250_000, "expected full 640x480 DOOM framebuffer, got {nonblack} non-black px");
    assert!(unique_rgb.len() >= 20, "expected rich DOOM palette (>20 colors), got {}", unique_rgb.len());

    // 3. Measure steady-state frame cost on real ARM926EJ-S instructions and exercise gameplay controls
    let move_insns = cam.press(Key::Up);
    assert_eq!(cam.screen_id(), 33);
    assert!(cam.cpu().last_error.is_none(), "DOOM Up error: {:?}", cam.cpu().last_error);
    let fb_moved = cam.render_lcd().to_vec();
    assert_ne!(fb_moved, fb0, "Key::Up must advance player and render new perspective");

    let turn_insns = cam.press(Key::Left);
    let fb_turned = cam.render_lcd().to_vec();
    assert_ne!(fb_turned, fb_moved, "Key::Left must rotate camera view");
    cam.press(Key::Right);

    // Cycle weapon to Shotgun (DialRight) and fire with Shutter (0xFE) — must NOT capture a camera photo!
    let caps_before = cam.capture_count();
    cam.press(Key::DialRight);
    let fire_insns = cam.press(Key::Shutter);
    assert_eq!(cam.screen_id(), 33, "Shutter inside DOOM must fire weapon, not open Playback");
    assert_eq!(cam.capture_count(), caps_before, "Shutter inside DOOM must not trigger photo capture");
    let fb_fire = cam.render_lcd().to_vec();
    assert_ne!(fb_fire, fb_moved, "Firing Shotgun must render muzzle flash & recoil");

    // Periodic timer tick (UI_DISPATCH(4, 3)) must advance simulation cleanly
    let steps_before = cam.cpu().total_steps;
    assert!(cam.tick(), "Camera::tick() must drive SCREEN_DOOM animation");
    let tick_insns = cam.cpu().total_steps - steps_before;
    assert!(cam.cpu().last_error.is_none(), "DOOM tick error: {:?}", cam.cpu().last_error);

    // Toggle Doom Automap with Key::Disp and toggle back
    cam.press(Key::Disp);
    let fb_map = cam.render_lcd().to_vec();
    assert_ne!(fb_map, fb_fire, "Key::Disp must toggle Doom Automap overlay");
    cam.press(Key::Disp);

    // Exercise Use (AEL) and Restart (QS)
    cam.press(Key::Ael);
    cam.press(Key::Qs);
    assert_eq!(cam.screen_id(), 33);

    println!(
        "DOOM ARM9 insns: open={open_insns}, move={move_insns}, turn={turn_insns}, fire={fire_insns}, tick={tick_insns}, unique_colors={}",
        unique_rgb.len()
    );
    assert!(
        tick_insns < 2_500_000,
        "DOOM frame exceeded 2.5M ARM9 instruction budget: {tick_insns}"
    );

    // 4. Press Menu to exit cleanly back to Live View (SCREEN_LV = 0x15)
    cam.press(Key::Menu);
    assert_eq!(cam.screen_id(), 0x15, "Key::Menu in DOOM must exit cleanly back to SCREEN_LV");
    assert!(cam.cpu().last_error.is_none(), "DOOM exit error: {:?}", cam.cpu().last_error);

    // Verify ARGB_FB was wiped clean (outside stock LV center AF bracket if any) and Live View resumed
    let argb_after = &cam.cpu().sdram()[0x021D_8C00..0x021D_8C00 + LCD_W * 120 * 2];
    assert!(argb_after.iter().all(|&b| b == 0), "ARGB_FB top 120 rows must be wiped clean after DOOM exit");
    let osd_after = &cam.cpu().sdram()[0x0213_0000..0x0213_0000 + LCD_W * LCD_H];
    let lv_osd_nz = osd_after.iter().filter(|&&b| b != 0).count();
    assert!(lv_osd_nz > 500 && lv_osd_nz < 40_000, "Live View OSD must resume cleanly: lv_osd_nz={lv_osd_nz}");
}

#[test]
#[ignore]
fn dump_doom_previews() {
    let pkg = pkg_bytes();
    let mut cam = Camera::from_package(pkg).expect("boot DP2M simulator");

    let dir = std::env::temp_dir().join("dp2m-doom-previews");
    std::fs::create_dir_all(&dir).expect("create preview dir");
    let write_ppm = |name: &str, rgba: &[u8]| {
        let mut out = format!("P6\n{LCD_W} {LCD_H}\n255\n").into_bytes();
        for p in rgba.as_chunks::<4>().0 {
            out.extend_from_slice(&p[..3]);
        }
        let p = dir.join(name);
        std::fs::write(&p, out).expect("write ppm");
        println!("wrote {}", p.display());
    };

    // Navigate Menu -> Setup 4 -> DOOM
    cam.press(Key::Menu);
    cam.press(Key::Left);
    for _ in 0..4 {
        cam.press(Key::Down);
    }
    cam.press(Key::Ok);
    assert_eq!(cam.screen_id(), 33);

    // 1. Opening Hangar view (Blue carpet, foyer, Zombieman, Pistol, Status Bar)
    let fb_start = cam.render_lcd().to_vec();
    write_ppm("doom_frame.ppm", &fb_start);

    // 2. Walk forward into Foyer (cross S0 -> S1 portal at x=256), switch to Shotgun, fire!
    for _ in 0..10 {
        cam.press(Key::Up);
    }
    cam.press(Key::DialRight);
    cam.press(Key::Shutter);
    write_ppm("doom_fire.ppm", cam.render_lcd());

    // 3. Turn North toward Courtyard (open sky + mountains + Imp + Green Armor!)
    for _ in 0..8 {
        cam.press(Key::Left);
    }
    cam.press(Key::Up);
    cam.press(Key::Up);
    write_ppm("doom_sky.ppm", cam.render_lcd());

    // 4. Automap overlay
    cam.press(Key::Disp);
    write_ppm("doom_map.ppm", cam.render_lcd());
}
