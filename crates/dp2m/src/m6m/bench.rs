//! Emulated cost of M6M live-view overlays, labels and screens.
//! `cargo test --release --offline -p dp2m m6m_bench -- --ignored --nocapture` (needs `dp2m build` first).

use std::path::PathBuf;
use std::time::Instant;

use dp2m_emu::image::load_syms;
use dp2m_emu::sim::{Camera, Key, LCD_W, LV_H};

const UI_DISPATCH: u32 = 0xA004_3234;
const CALL_SP: u32 = 0xA200_1000;
const MAX_STEPS: u64 = 500_000_000;

fn boot() -> (Camera, std::collections::HashMap<String, dp2m_emu::image::Sym>) {
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../out");
    let pkg = std::fs::read_dir(&out)
        .expect("out/ (run dp2m build)")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .find(|p| p.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with("DP2MV") && s.ends_with(".bin")))
        .expect("out/DP2MVxxx.bin");
    let syms = load_syms(&out.join("m6m/m6m.syms")).expect("out/m6m/m6m.syms (run dp2m build)");
    (Camera::from_package(&std::fs::read(pkg).unwrap()).expect("boot"), syms)
}

fn refresh(cam: &mut Camera) -> u64 {
    cam.cpu_mut().call(UI_DISPATCH | 1, &[4, 3], &[], CALL_SP, MAX_STEPS).expect("LV refresh").1
}

fn frame(cam: &mut Camera, fcnt: u32) -> u64 {
    let v = cam.cpu().r32_be(fcnt).wrapping_add(2);
    cam.cpu_mut().w32(fcnt, v);
    refresh(cam)
}

fn scene(f: impl Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
    let mut v = Vec::with_capacity(LCD_W * LV_H * 4);
    for y in 0..LV_H {
        for x in 0..LCD_W {
            let [r, g, b] = f(x, y);
            v.extend_from_slice(&[r, g, b, 255]);
        }
    }
    v
}

#[test]
#[ignore]
fn m6m_bench() {
    let t0 = Instant::now();
    let (mut cam, syms) = boot();
    let boot_ms = t0.elapsed().as_secs_f64() * 1e3;
    let va = |name: &str| syms.get(name).unwrap_or_else(|| panic!("payload symbol {name}")).va();
    let fcnt = va("lv_planes") + 12;
    let draw = va("grid_draw_hook") | 1;
    let scenes: [(&str, Option<Vec<u8>>); 4] = [
        ("default", None),
        ("checker", Some(scene(|x, y| if ((x >> 1) ^ (y >> 1)) & 1 == 0 { [255; 3] } else { [0; 3] }))),
        ("white", Some(scene(|_, _| [255; 3]))),
        ("black", Some(scene(|_, _| [0; 3]))),
    ];
    println!("boot: {boot_ms:.1} ms");
    println!("overlay cost in emulated M6M instructions (host DMA fills/flushes excluded)");
    println!("{:<8} {:>4} {:>12} {:>12} {:>12}", "scene", "grid", "idle", "new frame", "draw only");
    let t_ov = Instant::now();
    let mut ov_insns = 0u64;
    for (name, rgba) in &scenes {
        if let Some(rgba) = rgba {
            cam.set_scene_rgba(rgba, LCD_W, LV_H);
        }
        for grid in [0u32, 1, 7, 8, 10, 11, 12, 13, 14, 17] {
            cam.set_grid_mode(grid);
            assert_eq!(cam.grid_mode(), grid);
            let idle = refresh(&mut cam);
            let per_frame = frame(&mut cam, fcnt);
            let only = if grid >= 7 { cam.cpu_mut().call(draw, &[0], &[], CALL_SP, MAX_STEPS).expect("grid_draw_hook").1 } else { 0 };
            assert!(cam.cpu().last_error.is_none(), "{:?}", cam.cpu().last_error);
            ov_insns += idle + per_frame + only;
            println!("{name:<8} {grid:>4} {idle:>12} {per_frame:>12} {only:>12}");
        }
        let y = cam.cpu().r32_be(va("lv_planes"));
        let dual = cam.cpu_mut().call(va("draw_zebras_dual_arm"), &[y, 0, 242], &[], CALL_SP, MAX_STEPS).expect("dual zebras").1;
        println!("{name:<8} dual draw only {dual:>12}");
    }
    let ov_s = t_ov.elapsed().as_secs_f64();
    println!("overlay host rate: {:.1} MIPS ({:.1} ms)", ov_insns as f64 / ov_s / 1e6, ov_s * 1e3);

    let (label, magic) = (va("label_icon") | 1, va("cache"));
    println!("\nlabel icon cost (cold render / cached lookup), string 0 and last");
    let strings = cam.cpu().r32_be(va("dp2m_string_count"));
    let t_lb = Instant::now();
    let mut lb_insns = 0u64;
    for cls in 1..=6u32 {
        for idx in [0, strings - 1] {
            cam.cpu_mut().w32(magic, 0);
            let cold = cam.cpu_mut().call(label, &[cls << 10 | idx], &[], CALL_SP, MAX_STEPS).expect("label_icon").1;
            let warm = cam.cpu_mut().call(label, &[cls << 10 | idx], &[], CALL_SP, MAX_STEPS).expect("label_icon").1;
            lb_insns += cold + warm;
            println!("class {cls} string {idx:>3}: {cold:>10} / {warm}");
        }
    }
    let lb_s = t_lb.elapsed().as_secs_f64();
    println!("label host rate: {:.1} MIPS ({:.1} ms)", lb_insns as f64 / lb_s / 1e6, lb_s * 1e3);

    println!("\nscreen cost through the stock key path");
    for pass in ["cold", "warm"] {
        if pass == "cold" {
            cam.cpu_mut().w32(magic, 0);
        }
        let qs = cam.press(Key::Qs);
        let qs_close = cam.press(Key::Ok);
        let menu = cam.press(Key::Menu);
        let page = cam.press(Key::Right) + cam.press(Key::Right);
        let menu_close = cam.press(Key::Menu);
        println!("{pass}: QS open {qs}, QS close {qs_close}, menu open {menu}, to Film page {page}, menu close {menu_close}");
    }
    println!("total m6m_bench wall time: {:.2} s", t0.elapsed().as_secs_f64());
}
