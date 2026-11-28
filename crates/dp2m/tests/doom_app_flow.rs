mod common;

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use std::cell::Cell;
use std::rc::Rc;

use dp2m_emu::sim::{Camera, Key};

const APP_MAGIC: u32 = 0x4432_4150; // "D2AP"
const APP_ABI_VER: u32 = 1;
const APP_LOAD_VA: u32 = 0xA1CC_0000;
const SCREEN_LV: u32 = 0x15;
const SCREEN_DOOM: u32 = 0x21;

fn pkg_bytes() -> &'static [u8] {
    static PACKAGE: OnceLock<Vec<u8>> = OnceLock::new();
    PACKAGE.get_or_init(|| {
        common::with_fresh_build(|out, _| {
            std::fs::read(out.join("DP2MV200.bin")).expect("read freshly built package")
        })
    })
}


/// Encode `KEY0` (`0xA07FAA1C`) 2-bit held field for key index `k` (`0..12`).
const fn key0_bit(k: u32) -> u32 {
    3u32 << (30 - 2 * k)
}

const K0_OK: u32 = key0_bit(0);
const K0_UP: u32 = key0_bit(1);
const K0_DOWN: u32 = key0_bit(2);
const K0_AEL: u32 = key0_bit(7);
const K0_QS: u32 = key0_bit(8);
const K0_MODE: u32 = key0_bit(9);
const K0_PLAY: u32 = key0_bit(11);

/// Simulator call stack: every `UI_DISPATCH` call starts at `CALL_SP` and grows down into
/// SDRAM that neither stock firmware, the payload `.bss`, nor the `D2AP` arena references.
const SIM_CALL_SP_OFF: usize = 0x0200_1000;
const STACK_PROBE_LEN: usize = 0x0010_0000;
const STACK_PROBE_FILL: u8 = 0xC5;

fn stack_probe_arm(cam: &mut Camera) {
    cam.cpu_mut().sdram_mut()[SIM_CALL_SP_OFF - STACK_PROBE_LEN..SIM_CALL_SP_OFF].fill(STACK_PROBE_FILL);
}

/// Deepest stack use in bytes below the screen dispatcher's entry SP since `stack_probe_arm`.
fn stack_probe_high_water(cam: &mut Camera) -> usize {
    let probe = &cam.cpu_mut().sdram_mut()[SIM_CALL_SP_OFF - STACK_PROBE_LEN..SIM_CALL_SP_OFF];
    let untouched = probe.iter().take_while(|&&b| b == STACK_PROBE_FILL).count();
    STACK_PROBE_LEN - untouched
}

const APP_STACK_LEN: usize = 8192;
const APP_STACK_GUARD: u32 = 0x4432_5347;
// Offsets into `struct dp2m_doom_telemetry` (`src/doom/dp2m_doom.h`); `inp` starts at 248.
const TELEMETRY_HUD_MODE_OFF: u32 = 52;
const TELEMETRY_TILT_STRAFE_OFF: u32 = 56;
const TELEMETRY_PEEK_IDX_OFF: u32 = 60;
const TELEMETRY_ENTRY_SP_OFF: u32 = 76;
const TELEMETRY_RAW_KEY0_OFF: u32 = 248 + 16;
const TELEMETRY_RAW_DIAL_OFF: u32 = 248 + 24;
const TELEMETRY_RAW_RLS_STATE_OFF: u32 = 276;
const TELEMETRY_RAW_RLS_PORT_OFF: u32 = 280;

/// Stages of the autonomous `DOOM.BIN` input script, named by the effect each one waits for.
const AUTO_SCRIPT: [&str; 15] = [
    "applet polls the inherited OK+MODE hold",
    "applet polls OK released with MODE still held",
    "fresh OK opens the menu",
    "OK release polled",
    "half-press polled from the release state while the lines still read closed (New Game)",
    "half-press release and idle release lines polled",
    "full press polled from the release lines (episode)",
    "full-press release polled",
    "OK confirms the skill and the level starts",
    "UP and a dial detent polled after the melt",
    "QS+AEL turns tilt-strafe on",
    "MODE selects HUD mode 1",
    "MODE release polled",
    "MODE selects HUD mode 2 (explorer)",
    "QS+DOWN steps the explorer region",
];

/// `SW28` (`0xA07FAA28`) as the camera reports it at idle. It carries no shutter contact; DOOM only
/// resynchronises its edge snapshot on exit.
const SW28_IDLE: u32 = 0x4000_0000;
const SW28_CHANGED: u32 = SW28_IDLE | 0x8400_0000;

/// Release switches: the stock handlers keep `RLS_STATE[0]` (S1) and `[1]` (S2) at 1 while the switch is
/// closed, and `RLS_PORT` bits 3 (S1) and 2 (S2) read 0 while closed.
const RLS_STATE: u32 = 0xA07A_F398;
const RLS_PORT: u32 = 0x9000_1018;
const RLS_PORT_IDLE: u32 = 0xC;
const RLS_PORT_FULL: u32 = 0x0;
const RLS_STATE_HALF: u32 = 0x0100;

fn linked_map_symbol(map: &str, name: &str) -> u32 {
    for line in map.lines() {
        let mut fields = line.split_whitespace();
        let Some(addr) = fields.next() else { continue };
        if fields.any(|field| field == name) {
            return u32::from_str_radix(addr, 16)
                .unwrap_or_else(|_| panic!("invalid address for {name}: {addr}"));
        }
    }
    panic!("linked map is missing {name}");
}

/// Arm the real applet stack between event-driven calls, when no applet frame is active.
fn app_stack_probe_arm(cam: &mut Camera, stack_va: u32) {
    let off = (stack_va - 0xA000_0000) as usize;
    let stack = &mut cam.cpu_mut().sdram_mut()[off..off + APP_STACK_LEN];
    stack.fill(STACK_PROBE_FILL);
    stack[..4].copy_from_slice(&APP_STACK_GUARD.to_be_bytes());
}

fn app_stack_probe_high_water(cam: &mut Camera, stack_va: u32) -> usize {
    let off = (stack_va - 0xA000_0000) as usize;
    // Skip the permanently non-fill guard word at the bottom of the stack.
    let probe = &cam.cpu_mut().sdram_mut()[off + 4..off + APP_STACK_LEN];
    let untouched = probe.iter().take_while(|&&b| b == STACK_PROBE_FILL).count();
    probe.len() - untouched
}

/// Assemble a self-contained big-endian ARM `D2AP` v1 applet that:
/// - verifies `.bss` (`0xA1CC0080..0xA1CC0100`) was zeroed by `doom_try_load_app`,
/// - allocates a 256-byte heap block on `INIT` (`cmd == 0`) and stores the pointer in `.bss`,
/// - paints solid green RGBA4444 (`0x0F0F`) to `api->argb_fb[0..2]`, calls `api->present_argb()`,
///   calls `api->sleep_ms(5)` on `STEP` (`cmd == 1`) for 2 steps then returns `1` to request exit,
/// - frees its heap allocation on `SHUTDOWN` (`cmd == 2`) so zero blocks leak.
fn build_synthetic_d2ap_applet() -> Vec<u8> {
    let entry_va = APP_LOAD_VA + 0x20;
    let prog: &[u32] = &[
        0xE92D_4070, // 0: push {r4, r5, r6, lr}
        0xE1A0_4000, // 1: mov r4, r0
        0xE3A0_54A1, // 2: mov r5, #0xA1000000
        0xE385_5733, // 3: orr r5, r5, #0x00CC0000
        0xE385_5C01, // 4: orr r5, r5, #0x0100      (bss @ 0xA1CC0100)
        0xE351_0000, // 5: cmp r1, #0
        0x1A00_0008, // 6: bne idx 16 (6 + 2 + 8 = 16)
        // INIT (7..15):
        0xE595_0000, // 7: ldr r0, [r5, #0]
        0xE350_0000, // 8: cmp r0, #0
        0x13E0_0000, // 9: movne r0, #-1
        0x18BD_8070, // 10: popne {r4, r5, r6, pc}
        0xE3A0_0C01, // 11: mov r0, #256
        0xE594_3020, // 12: ldr r3, [r4, #0x20]      (api->malloc_fn)
        0xE12F_FF33, // 13: blx r3
        0xE585_0000, // 14: str r0, [r5, #0]
        0xEA00_0017, // 15: b idx 40 (15 + 2 + 23 = 40)
        // STEP (16..33):
        0xE351_0001, // 16: cmp r1, #1
        0x1A00_000F, // 17: bne idx 34 (17 + 2 + 15 = 34)
        0xE594_0008, // 18: ldr r0, [r4, #0x08]      (api->argb_fb)
        0xE3A0_1C0F, // 19: mov r1, #0x0F00
        0xE381_100F, // 20: orr r1, r1, #0x0F        (0x0F0F green RGBA4444)
        0xE1C0_10B0, // 21: strh r1, [r0, #0]
        0xE594_300C, // 22: ldr r3, [r4, #0x0C]      (api->present_argb)
        0xE12F_FF33, // 23: blx r3
        0xE3A0_0005, // 24: mov r0, #5
        0xE594_3018, // 25: ldr r3, [r4, #0x18]      (api->sleep_ms)
        0xE12F_FF33, // 26: blx r3
        0xE595_6004, // 27: ldr r6, [r5, #4]         (step_count)
        0xE286_6001, // 28: add r6, r6, #1
        0xE585_6004, // 29: str r6, [r5, #4]
        0xE356_0002, // 30: cmp r6, #2
        0x23A0_0001, // 31: movhs r0, #1
        0x28BD_8070, // 32: pophs {r4, r5, r6, pc}
        0xEA00_0005, // 33: b idx 40 (33 + 2 + 5 = 40)
        // SHUTDOWN (34..39):
        0xE595_0000, // 34: ldr r0, [r5, #0]
        0xE350_0000, // 35: cmp r0, #0
        0x1594_302C, // 36: ldrne r3, [r4, #0x2C]    (api->free_fn)
        0x112F_FF33, // 37: blxne r3
        0xE3A0_0000, // 38: mov r0, #0
        0xE585_0000, // 39: str r0, [r5, #0]
        // ret_zero (40..41):
        0xE3A0_0000, // 40: mov r0, #0
        0xE8BD_8070, // 41: pop {r4, r5, r6, pc}
    ];
    assert_eq!(prog.len(), 42);

    let img_size = 0x100u32;
    let mut bin = vec![0u8; img_size as usize];
    let hdr = [
        APP_MAGIC,
        APP_ABI_VER,
        APP_LOAD_VA,
        entry_va,
        img_size,
        APP_LOAD_VA + 0x180,
        0,
        0,
    ];
    for (i, w) in hdr.iter().enumerate() {
        bin[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    for (i, w) in prog.iter().enumerate() {
        let off = 0x20 + i * 4;
        bin[off..off + 4].copy_from_slice(&w.to_be_bytes());
    }
    bin
}

#[test]
fn autonomous_missing_applet_notice_controls_psave_mbx_and_input_sync() {
    let mut cam = Camera::from_package(pkg_bytes()).expect("boot simulator");
    cam.cpu_mut().set_write_watch(Some((0xA000_0000, 0xA017_9000)));

    // The notice still owns the UI task: drain UI packets, inhibit power saving, and resync input on exit.
    cam.enqueue_mbx_packet(0xA080_2000);
    cam.enqueue_mbx_packet(0xA080_2040);
    cam.enqueue_mbx_packet(0xA080_2080);
    assert_eq!(cam.pending_mbx_packets(), 3);
    cam.cpu_mut().w32(0xA07F_AB20, 0);
    cam.cpu_mut().w32(0xA07F_AA28, SW28_IDLE);
    cam.cpu_mut().w8(0xA07A_F38C, 1);

    let saw_psave = Rc::new(Cell::new(false));
    let saw_psave_cb = Rc::clone(&saw_psave);
    cam.run_autonomous_open(SCREEN_DOOM, 7, 10_000_000, move |cpu, step| {
        if cpu.r32_be(0xA07D_D73C) & 0x4000_0000 != 0 {
            saw_psave_cb.set(true);
        }
        if step == 0 {
            cpu.w32(0xA07F_AA1C, K0_PLAY);
            cpu.w32(0xA07F_AA28, SW28_CHANGED);
            cpu.w32(RLS_PORT, RLS_PORT_FULL);
        }
    })
    .expect("autonomous missing-applet notice");

    assert!(saw_psave.get(), "PSAVE_DOOM_MASK must be set while the notice owns the UI task");
    assert!(cam.cpu().watch_hits.is_empty(), ".text write watch hits: {:?}", cam.cpu().watch_hits);
    assert!(cam.cpu().last_error.is_none(), "CPU error: {:?}", cam.cpu().last_error);
    assert_eq!(cam.screen_id(), SCREEN_LV, "PLAY must dismiss the notice back to SCREEN_LV");
    assert_eq!(cam.capture_count(), 0, "shutter input on the notice must not capture a photo");
    assert_eq!(cam.pending_mbx_packets(), 0, "notice/exit must drain normal MBX 0x11 packets");
    assert_eq!(cam.cpu().r32_be(0xA07D_D73C) & 0x4000_0000, 0, "PSAVE_DOOM_MASK must be released on exit");
    assert_eq!(cam.cpu().r32_be(0xA07F_AB20), SW28_CHANGED, "SW28 edge snapshot must sync on exit");
    assert_eq!(cam.cpu().r32_be(0xA07F_AB24), K0_PLAY, "key edge snapshot must sync on exit");

    // Clear held registers and verify normal camera Shutter capture works again in Live View.
    cam.cpu_mut().w32(0xA07F_AA1C, 0);
    cam.cpu_mut().w32(0xA07F_AA28, 0);
    cam.cpu_mut().w32(RLS_PORT, RLS_PORT_IDLE);
    cam.press(Key::Shutter);
    assert_eq!(cam.capture_count(), 1, "Live View shutter capture must work normally after DOOM exit");

    // A system power-off packet breaks before the first notice sleep and is re-queued for the UI task.
    let sys_pkt = 0xA080_2100;
    cam.cpu_mut().fill_bytes(sys_pkt, 0, 16);
    cam.cpu_mut().w8(sys_pkt + 8, 6);
    cam.enqueue_mbx_packet(sys_pkt);
    let sleep_cnt = Rc::new(Cell::new(0usize));
    let sleep_cnt_cb = Rc::clone(&sleep_cnt);
    cam.run_autonomous_open(SCREEN_DOOM, 7, 10_000_000, move |_cpu, _step| {
        sleep_cnt_cb.set(sleep_cnt_cb.get() + 1);
    })
    .expect("power-off packet autonomous break");
    assert_eq!(sleep_cnt.get(), 0, "type == 6 power-off packet must break before first notice sleep");
    assert_eq!(cam.pending_mbx_packets(), 1, "power-off packet must be re-posted to MBX 0x11");
    assert_eq!(cam.screen_id(), SCREEN_DOOM, "power-off exit must not post SCREEN_LV");
}
#[test]
fn synthetic_d2ap_applet_and_bad_header_notice_zero_leak() {
    let mut cam = Camera::from_package(pkg_bytes()).expect("boot simulator");
    // Warm stock SCREEN_MENU once so its one-time persistent UI/font allocations settle
    cam.press(Key::Menu);
    cam.press(Key::Menu);
    let heap_base = cam.live_heap_allocations();

    // 1. Corrupt header must be rejected with the notice and without a heap leak
    let mut bad = build_synthetic_d2ap_applet();
    bad[0..4].copy_from_slice(b"NOPE");
    cam.mount_sd_file("DOOM.BIN", bad);

    cam.press(Key::Menu);
    cam.press(Key::Left);
    for _ in 0..4 {
        cam.press(Key::Down);
    }
    cam.press(Key::Ok);
    assert_eq!(cam.screen_id(), SCREEN_DOOM);
    assert_eq!(cam.live_heap_allocations(), heap_base, "rejected header must not leak heap");
    assert_eq!(cam.cpu().r16_be(0xA21D_8C00), 0x111F, "bad header must show the notice");
    cam.press(Key::Play);
    assert_eq!(cam.screen_id(), SCREEN_LV);

    // 2. Valid synthetic D2AP v1 applet in autonomous mode: poison .bss at 0xA1CC0100 first,
    //    verify loader zeroes .bss, runs 2 steps, paints ARGB_FB[0] = 0x0F0F, and frees heap on SHUTDOWN
    cam.cpu_mut().sdram_mut()[0x01CC_0100..0x01CC_0180].fill(0xDE);
    cam.mount_sd_file("DOOM.BIN", build_synthetic_d2ap_applet());

    let saw_green = Rc::new(Cell::new(false));
    let saw_green_cb = Rc::clone(&saw_green);
    cam.run_autonomous_open(SCREEN_DOOM, 5, 50_000_000, move |cpu, _step| {
        if cpu.r16_be(0xA21D_8C00) == 0x0F0F {
            saw_green_cb.set(true);
        }
    })
    .expect("synthetic D2AP autonomous run");

    assert!(saw_green.get(), "synthetic D2AP applet must paint 0x0F0F to ARGB_FB");
    assert_eq!(cam.screen_id(), SCREEN_LV);
    assert_eq!(
        cam.live_heap_allocations(),
        heap_base,
        "SHUTDOWN must release all applet heap allocations"
    );
    assert_eq!(cam.cpu().r32_be(0xA1CC_0104), 2, "synthetic applet must execute exactly 2 steps");
}

#[test]
fn standalone_doom_bin_and_shareware_wad_full_demo_flow() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut wad_cands = Vec::new();
    if let Ok(p) = std::env::var("DP2M_WAD") {
        wad_cands.push(PathBuf::from(p));
    }
    wad_cands.push(root.join("out/DOOM1.WAD"));

    let mut dg_cands = Vec::new();
    if let Ok(p) = std::env::var("DOOMGENERIC_DIR") {
        dg_cands.push(PathBuf::from(p));
    }
    dg_cands.push(root.join("vendor/doomgeneric/doomgeneric"));
    let Some(wad_path) = wad_cands.iter().find(|p| p.is_file()) else {
        eprintln!("skipping standalone DOOM1.WAD test (set DP2M_WAD or place out/DOOM1.WAD)");
        return;
    };
    let Some(dg_dir) = dg_cands.iter().find(|p| p.join("d_main.c").is_file()) else {
        eprintln!("skipping standalone DOOM.BIN build test (set DOOMGENERIC_DIR or clone into vendor/doomgeneric)");
        return;
    };

    let tmp_out = std::env::temp_dir().join(format!("dp2m-doom-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_out);
    std::fs::create_dir_all(&tmp_out).expect("mkdir tmp_out");

    // Build both standard (title screen) and --warp (E1M1) DOOM.BIN via matching dp2m CLI
    let warp_dir = tmp_out.join("warp");
    let std_dir = tmp_out.join("std");
    for (dir, warp) in [(&warp_dir, true), (&std_dir, false)] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_dp2m"));
        cmd.current_dir(&root)
            .args(["build-doom", "--doomgeneric"])
            .arg(dg_dir)
            .arg("--out")
            .arg(dir);
        if warp {
            cmd.arg("--warp");
        }
        let out = cmd.output().expect("run dp2m build-doom");
        assert!(
            out.status.success(),
            "dp2m build-doom failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    let doom_warp = std::fs::read(warp_dir.join("DOOM.BIN")).expect("read warp DOOM.BIN");
    let doom_std = std::fs::read(std_dir.join("DOOM.BIN")).expect("read std DOOM.BIN");
    let warp_map = std::fs::read_to_string(warp_dir.join("doom/doom.map")).expect("read warp map");
    let std_map = std::fs::read_to_string(std_dir.join("doom/doom.map")).expect("read std map");
    let warp_stack = linked_map_symbol(&warp_map, "g_app_stack");
    let warp_telemetry = linked_map_symbol(&warp_map, "g_doom_tel");
    let std_stack = linked_map_symbol(&std_map, "g_app_stack");
    let std_telemetry = linked_map_symbol(&std_map, "g_doom_tel");
    let std_menuactive = linked_map_symbol(&std_map, "menuactive");
    let std_gamestate = linked_map_symbol(&std_map, "gamestate");
    let wad_bytes = std::fs::read(wad_path).expect("read DOOM1.WAD");
    let _ = std::fs::remove_dir_all(&tmp_out);

    let mut cam = Camera::from_package(pkg_bytes()).expect("boot simulator");
    cam.cpu_mut().set_write_watch(Some((0xA000_0000, 0xA017_9000)));
    // Warm stock SCREEN_MENU once so its one-time persistent UI/font allocations settle
    cam.press(Key::Menu);
    cam.press(Key::Menu);
    let heap_base = cam.live_heap_allocations();

    // A) DOOM.BIN mounted WITHOUT DOOM1.WAD must fail INIT cleanly with zero leaked heap
    //    and show the missing/invalid-applet notice.
    cam.mount_sd_file("DOOM.BIN", doom_std.clone());
    cam.press(Key::Menu);
    cam.press(Key::Left);
    for _ in 0..4 {
        cam.press(Key::Down);
    }
    cam.press(Key::Ok);
    assert_eq!(cam.screen_id(), SCREEN_DOOM);
    assert_eq!(
        cam.live_heap_allocations(),
        heap_base,
        "missing WAD notice must leave zero leaked heap allocations"
    );
    assert_eq!(cam.open_sd_fds(), 0, "missing WAD notice must leave zero open SD fds");
    cam.press(Key::Play);
    assert_eq!(cam.screen_id(), SCREEN_LV);

    // B) Event-driven simulator flow with --warp DOOM.BIN + DOOM1.WAD:
    //    verify E1M1 3D render, firing, Automap (Disp), Mode 0/1/2 cycling, and clean Play exit
    cam.mount_sd_file("DOOM.BIN", doom_warp.clone());
    cam.mount_sd_file("DOOM1.WAD", wad_bytes.clone());

    // Re-open DOOM (SCREEN_MENU remembers Setup Page 4 from step A)
    stack_probe_arm(&mut cam);
    cam.press(Key::Menu);
    let mut boot_insns = cam.press(Key::Ok);
    if cam.screen_id() != SCREEN_DOOM {
        for _ in 0..4 {
            cam.press(Key::Down);
        }
        boot_insns = cam.press(Key::Ok);
    }
    assert_eq!(cam.screen_id(), SCREEN_DOOM);
    assert!(
        cam.live_heap_allocations() > heap_base + 5,
        "DOOM.BIN + preloaded DOOM1.WAD must allocate zone + SDRAM WAD on stock heap"
    );
    let app_entry_sp = cam.cpu().r32_be(warp_telemetry + TELEMETRY_ENTRY_SP_OFF);
    assert!(
        (warp_stack..warp_stack + APP_STACK_LEN as u32).contains(&app_entry_sp),
        "applet entry SP {app_entry_sp:#010x} must be inside private stack {warp_stack:#010x}"
    );
    app_stack_probe_arm(&mut cam, warp_stack);

    // Advance past screen wipe into live E1M1 Hangar
    for _ in 0..38 {
        assert!(cam.tick());
    }
    let fb_mode0 = cam.render_lcd().to_vec();

    // Mode 1: Fullscreen 640x480 4:3 CRT stretch
    cam.press(Key::Mode);
    let fb_mode1 = cam.render_lcd().to_vec();
    assert_ne!(fb_mode1, fb_mode0, "Key::Mode must switch to Mode 1 (640x480 4:3 CRT)");

    // Mode 2: Semi-transparent Live Hardware & Memory Explorer
    cam.press(Key::Mode);
    let fb_mode2 = cam.render_lcd().to_vec();
    assert_ne!(fb_mode2, fb_mode1, "Key::Mode must switch to Mode 2 (HW Explorer)");

    // Back to Mode 0 Dashboard, toggle Automap with Key::Disp, fire with Key::Shutter
    cam.press(Key::Mode);
    cam.press(Key::Disp);
    let fb_automap = cam.render_lcd().to_vec();
    assert_ne!(fb_automap, fb_mode0, "Key::Disp must open DOOM Automap immediately");
    cam.press(Key::Disp);

    let caps_before = cam.capture_count();
    let step_insns = cam.press(Key::Shutter);
    assert_eq!(cam.capture_count(), caps_before, "Shutter inside DOOM.BIN must not capture photo");
    assert!(
        step_insns < 3_000_000,
        "DOOM.BIN E1M1 frame must stay well under 216 MHz 35 FPS budget: {step_insns} insns"
    );

    // Exit with Play -> verify zero heap and SD fd leaks!
    assert_eq!(cam.open_sd_fds(), 0, "preloaded DOOM1.WAD closes raw MDF fd after SDRAM preload");
    cam.press(Key::Play);
    assert_eq!(cam.screen_id(), SCREEN_LV);
    assert_eq!(cam.open_sd_fds(), 0);
    assert_eq!(
        cam.live_heap_allocations(),
        heap_base,
        "Play exit from DOOM.BIN must free 100% of heap allocations"
    );
    let stack_evt = stack_probe_high_water(&mut cam);
    let app_stack_evt = app_stack_probe_high_water(&mut cam, warp_stack);
    assert!(app_stack_evt > 0, "private-stack probe must observe real engine frames");
    assert_eq!(cam.cpu().r32_be(warp_stack), APP_STACK_GUARD, "private stack guard overwritten");

    // C) Autonomous hardware UI-task flow (`tid = 7`) with standard Title-Screen DOOM.BIN + DOOM1.WAD
    //    plus re-entry check: inherited holds (OK, MODE and release lines that read closed), shutter
    //    half-press from the release state, full press from the release lines and OK through
    //    Menu -> New Game, dial polling, QS+AEL Tilt-Strafe, HUD modes, Explorer region stepping
    //    (`QS+Down`), MBX drain, PSAVE bit release, and zero heap leak.
    cam.mount_sd_file("DOOM.BIN", doom_std.clone());
    cam.set_tilt(28, 64);
    cam.enqueue_mbx_packet(0xA080_3000);
    cam.cpu_mut().w32(0xA07F_AA28, SW28_IDLE);
    cam.cpu_mut().w8(0xA07A_F38C, 1);
    stack_probe_arm(&mut cam);

    // Active while DOOM opens: OK, MODE and the release lines must all stay inert until they go idle.
    cam.cpu_mut().w32(0xA07F_AA1C, K0_OK | K0_MODE);
    cam.cpu_mut().w32(RLS_PORT, RLS_PORT_FULL);
    let stage = Rc::new(Cell::new(0usize));
    let stage_cb = Rc::clone(&stage);

    // Each stage drives its inputs until the applet shows the stated effect, so the script does not
    // depend on how many `dly_tsk` calls a frame makes (the level-start melt sleeps without polling).
    let auto_insns = cam
        .run_autonomous_open(SCREEN_DOOM, 7, 450_000_000, move |cpu, step| {
            let tel = |off: u32| cpu.r32_be(std_telemetry + off);
            let polled = |key0: u32, rls_state: u32, rls_port: u32| {
                tel(TELEMETRY_RAW_KEY0_OFF) == key0
                    && tel(TELEMETRY_RAW_RLS_STATE_OFF) == rls_state
                    && tel(TELEMETRY_RAW_RLS_PORT_OFF) == rls_port
            };
            let menu = cpu.r32_be(std_menuactive);
            let hud = tel(TELEMETRY_HUD_MODE_OFF);
            let mut s = stage_cb.get();
            let done = match s {
                0 => polled(K0_OK | K0_MODE, 0, RLS_PORT_FULL),
                1 => polled(K0_MODE, 0, RLS_PORT_FULL),
                2 => menu != 0,
                3 => polled(0, 0, RLS_PORT_FULL),
                4 => polled(0, RLS_STATE_HALF, RLS_PORT_FULL),
                5 => polled(0, 0, RLS_PORT_IDLE),
                6 => polled(0, 0, RLS_PORT_FULL),
                7 => polled(0, 0, RLS_PORT_IDLE),
                8 => cpu.r32_be(std_gamestate) == 0 && menu == 0,
                9 => polled(K0_UP, 0, RLS_PORT_IDLE) && tel(TELEMETRY_RAW_DIAL_OFF) == 2,
                10 => tel(TELEMETRY_TILT_STRAFE_OFF) == 1,
                11 => hud == 1,
                12 => polled(0, 0, RLS_PORT_IDLE),
                13 => hud == 2,
                14 => tel(TELEMETRY_PEEK_IDX_OFF) == 1,
                _ => false,
            };
            if done {
                match s {
                    1 => {
                        assert_eq!(menu, 0, "inherited OK or release lines opened the DOOM menu");
                        assert_eq!(hud, 0, "inherited MODE cycled the HUD");
                        let sp = tel(TELEMETRY_ENTRY_SP_OFF);
                        assert!((std_stack..std_stack + APP_STACK_LEN as u32).contains(&sp));
                    }
                    2 => assert_eq!(hud, 0, "MODE acted although its inherited hold was never released"),
                    _ => {}
                }
                s += 1;
                stage_cb.set(s);
            }
            assert!(
                step < 200,
                "DOOM input script stalled at stage {s}: {}",
                AUTO_SCRIPT.get(s).copied().unwrap_or("PLAY exit")
            );
            let (key0, s1_state, rls_port, dial) = match s {
                0 => (K0_OK | K0_MODE, 0, RLS_PORT_FULL, 1),
                1 => (K0_MODE, 0, RLS_PORT_FULL, 1),
                2 => (K0_OK | K0_MODE, 0, RLS_PORT_FULL, 1),
                3 => (0, 0, RLS_PORT_FULL, 1),
                4 => (0, 1, RLS_PORT_FULL, 1),
                5 | 7 => (0, 0, RLS_PORT_IDLE, 1),
                6 => (0, 0, RLS_PORT_FULL, 1),
                8 => (K0_OK, 0, RLS_PORT_IDLE, 1),
                9 => (K0_UP, 0, RLS_PORT_IDLE, 2),
                10 => (K0_QS | K0_AEL, 0, RLS_PORT_IDLE, 2),
                11 | 13 => (K0_MODE, 0, RLS_PORT_IDLE, 2),
                12 => (0, 0, RLS_PORT_IDLE, 2),
                14 => (K0_QS | K0_DOWN, 0, RLS_PORT_IDLE, 2),
                _ => (K0_PLAY, 0, RLS_PORT_IDLE, 2),
            };
            cpu.w32(0xA07F_AA1C, key0);
            cpu.w8(RLS_STATE, s1_state);
            cpu.w32(RLS_PORT, rls_port);
            cpu.w8(0xA07A_F38C, dial);
        })
        .expect("autonomous DOOM.BIN + DOOM1.WAD run");
    assert_eq!(stage.get(), AUTO_SCRIPT.len(), "autonomous input script did not run to completion");
    let stack_auto = stack_probe_high_water(&mut cam);
    assert_eq!(cam.cpu().r32_be(std_stack), APP_STACK_GUARD, "autonomous stack guard overwritten");
    println!(
        "DOOM.BIN verified: boot_insns={boot_insns}, e1m1_step_insns={step_insns}, auto_insns={auto_insns}, \
         stock_stack_high_water: event={stack_evt} B, autonomous={stack_auto} B; \
         app_stack_high_water: event={app_stack_evt} B"
    );
    assert!(cam.cpu().watch_hits.is_empty(), ".text write watch hits: {:?}", cam.cpu().watch_hits);
    assert!(cam.cpu().last_error.is_none(), "CPU error: {:?}", cam.cpu().last_error);
    assert_eq!(cam.screen_id(), SCREEN_LV);
    assert_eq!(cam.pending_mbx_packets(), 0);
    assert_eq!(cam.open_sd_fds(), 0);
    assert_eq!(cam.cpu().r32_be(0xA07D_D73C) & 0x4000_0000, 0);
    assert_eq!(
        cam.live_heap_allocations(),
        heap_base,
        "autonomous DOOM.BIN exit must leave zero leaked heap allocations"
    );

    // D) Streaming >6 MB WAD (> WAD_PRELOAD_MAX) exercises 16 KB MDF sector cache and raw fd tracking
    cam.cpu_mut().w32(0xA07F_AA1C, 0);
    cam.cpu_mut().w32(0xA07F_AA28, 0);
    let mut big_wad = wad_bytes.clone();
    big_wad.resize(6 * 1024 * 1024 + 4096, 0);
    cam.mount_sd_file("DOOM.BIN", doom_warp.clone());
    cam.mount_sd_file("DOOM1.WAD", big_wad);
    cam.press(Key::Menu);
    cam.press(Key::Ok);
    if cam.screen_id() != SCREEN_DOOM {
        for _ in 0..4 {
            cam.press(Key::Down);
        }
        cam.press(Key::Ok);
    }
    assert_eq!(cam.screen_id(), SCREEN_DOOM);
    assert_eq!(
        cam.open_sd_fds(),
        1,
        ">6 MB WAD must stay open via 16 KB MDF streaming cache while running"
    );
    assert!(cam.tick());
    cam.press(Key::Play);
    assert_eq!(cam.screen_id(), SCREEN_LV);
    assert_eq!(
        cam.open_sd_fds(),
        0,
        "DP2M_APP_SHUTDOWN must close streaming WAD MDF descriptor"
    );
    assert_eq!(
        cam.live_heap_allocations(),
        heap_base,
        "streaming WAD exit must free 100% of heap allocations"
    );

    // E) A directory entry beyond EOF must fail INIT instead of parsing recycled memory.
    let mut bad_wad = wad_bytes;
    let dir_ofs = u32::from_le_bytes(bad_wad[8..12].try_into().unwrap()) as usize;
    let bad_pos = bad_wad.len() as u32 - 4;
    bad_wad[dir_ofs..dir_ofs + 4].copy_from_slice(&bad_pos.to_le_bytes());
    bad_wad[dir_ofs + 4..dir_ofs + 8].copy_from_slice(&0x100u32.to_le_bytes());
    cam.mount_sd_file("DOOM.BIN", doom_warp);
    cam.mount_sd_file("DOOM1.WAD", bad_wad);
    cam.press(Key::Menu);
    cam.press(Key::Ok);
    if cam.screen_id() != SCREEN_DOOM {
        for _ in 0..4 {
            cam.press(Key::Down);
        }
        cam.press(Key::Ok);
    }
    assert_eq!(cam.screen_id(), SCREEN_DOOM, "bad WAD must leave the notice open");
    assert_eq!(cam.cpu().r16_be(0xA21D_8C00), 0x111F, "bad WAD must show the notice");
    assert_eq!(cam.open_sd_fds(), 0, "bad WAD must close all SD descriptors");
    assert_eq!(
        cam.live_heap_allocations(),
        heap_base,
        "bad WAD INIT failure must free all applet heap allocations"
    );
    cam.press(Key::Play);
    assert_eq!(cam.screen_id(), SCREEN_LV);
}
