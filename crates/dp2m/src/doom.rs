//! Standalone SD-loaded `DOOM.BIN` (`D2AP` v1 @ `0xA1CC0000`) builder for Sigma DP2 Merrill.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::thread;

use crate::elf::{self, Elf};

pub const APP_MAGIC: u32 = 0x4432_4150; // "D2AP"
pub const APP_ABI_VER: u32 = 1;
pub const APP_LOAD_VA: u32 = 0xA1CC_0000;
pub const APP_LIMIT_VA: u32 = 0xA1E0_0000;

const DOOMGENERIC_CORE: &[&str] = &[
    "dummy.c", "am_map.c", "doomdef.c", "doomstat.c", "dstrings.c", "d_event.c",
    "d_items.c", "d_iwad.c", "d_loop.c", "d_main.c", "d_mode.c", "d_net.c",
    "f_finale.c", "f_wipe.c", "g_game.c", "hu_lib.c", "hu_stuff.c", "info.c",
    "i_cdmus.c", "i_endoom.c", "i_joystick.c", "i_scale.c", "i_sound.c",
    "i_system.c", "i_timer.c", "memio.c", "m_argv.c", "m_bbox.c", "m_cheat.c",
    "m_config.c", "m_controls.c", "m_menu.c", "m_misc.c", "m_random.c", "p_ceilng.c",
    "p_doors.c", "p_enemy.c", "p_floor.c", "p_inter.c", "p_lights.c", "p_map.c",
    "p_maputl.c", "p_mobj.c", "p_plats.c", "p_pspr.c", "p_saveg.c", "p_setup.c",
    "p_sight.c", "p_spec.c", "p_switch.c", "p_telept.c", "p_tick.c", "p_user.c",
    "r_bsp.c", "r_data.c", "r_draw.c", "r_main.c", "r_plane.c", "r_segs.c",
    "r_sky.c", "r_things.c", "sha1.c", "sounds.c", "statdump.c", "st_lib.c",
    "st_stuff.c", "s_sound.c", "tables.c", "v_video.c", "wi_stuff.c", "w_checksum.c",
    "w_file.c", "w_main.c", "w_wad.c", "z_zone.c",
];

const DP2M_DOOM_SRCS: &[&str] = &[
    "rt_dp2m.c",
    "m_fixed_dp2m.c",
    "libc_dp2m.c",
    "w_file_dp2m.c",
    "i_video_dp2m.c",
    "dp2m_doom_main.c",
];

const HOT_ARM_UNITS: &[&str] = &[
    "rt_dp2m.c",
    "m_fixed_dp2m.c",
    "i_video_dp2m.c",
    "dp2m_doom_main.c",
    "r_bsp.c",
    "r_data.c",
    "r_draw.c",
    "r_main.c",
    "r_plane.c",
    "r_segs.c",
    "r_things.c",
    "p_map.c",
    "p_maputl.c",
    "p_mobj.c",
    "p_sight.c",
    "w_wad.c",
    "z_zone.c",
];

pub fn resolve_doomgeneric_dir(root: &Path, explicit: Option<&Path>) -> Result<PathBuf, String> {
    let mut cands = Vec::new();
    if let Some(p) = explicit {
        cands.push(p.to_path_buf());
    }
    if let Ok(env_p) = env::var("DOOMGENERIC_DIR") {
        cands.push(PathBuf::from(env_p));
    }
    cands.push(root.join("vendor/doomgeneric/doomgeneric"));
    cands.push(root.join("vendor/doomgeneric"));

    for c in cands {
        if c.join("d_main.c").is_file() {
            return Ok(c);
        }
        let sub = c.join("doomgeneric");
        if sub.join("d_main.c").is_file() {
            return Ok(sub);
        }
    }
    Err("could not locate doomgeneric sources (pass --doomgeneric <dir> or set DOOMGENERIC_DIR)".into())
}

fn run_cmd(cmd: &mut Command, what: &str) -> Result<(), String> {
    let out = cmd.output().map_err(|e| format!("{what}: {e} (needs clang & ld.lld on PATH)"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{what} failed:\n{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

pub fn build_doom(root: &Path, dg_dir: &Path, out_dir: &Path, warp_e1m1: bool) -> Result<PathBuf, String> {
    let doom_src = root.join("src/doom");
    let m6m_src = root.join("src/m6m");
    let work = out_dir.join("doom");
    fs::create_dir_all(&work).map_err(|e| format!("mkdir {}: {e}", work.display()))?;

    let mut jobs: Vec<(PathBuf, PathBuf, bool)> = Vec::with_capacity(DOOMGENERIC_CORE.len() + DP2M_DOOM_SRCS.len());
    for &name in DP2M_DOOM_SRCS {
        let src = doom_src.join(name);
        if !src.is_file() {
            return Err(format!("missing DP2M DOOM source {}", src.display()));
        }
        let obj = work.join(format!("dp2m_{name}.o"));
        jobs.push((src, obj, true));
    }
    for &name in DOOMGENERIC_CORE {
        let src = dg_dir.join(name);
        if !src.is_file() {
            return Err(format!("missing doomgeneric source {}", src.display()));
        }
        let obj = work.join(format!("dg_{name}.o"));
        jobs.push((src, obj, false));
    }

    let queue = Mutex::new(jobs.iter().collect::<Vec<_>>());
    let first_err: Mutex<Option<String>> = Mutex::new(None);
    let n_threads = 4usize;

    thread::scope(|s| {
        for _ in 0..n_threads {
            s.spawn(|| loop {
                if first_err.lock().unwrap().is_some() {
                    return;
                }
                let Some((src, obj, strict)) = queue.lock().unwrap().pop() else {
                    return;
                };
                let fname = src.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let hot_arm = HOT_ARM_UNITS.contains(&fname);

                let mut cmd = Command::new("clang");
                cmd.args([
                    "--target=armeb-none-eabi",
                    "-mcpu=arm926ej-s",
                    "-mfloat-abi=soft",
                    "-mno-unaligned-access",
                    "-std=gnu99",
                    "-fsigned-char",
                    "-ffreestanding",
                    "-fno-builtin",
                    "-fno-stack-protector",
                    "-fno-common",
                    "-nostdlibinc",
                    "-ffunction-sections",
                    "-fdata-sections",
                    "-DCMAP256",
                    "-DDOOMGENERIC_RESX=320",
                    "-DDOOMGENERIC_RESY=200",
                ]);
                if hot_arm {
                    cmd.args(["-marm", "-O2"]);
                } else {
                    cmd.args(["-mthumb", "-Os"]);
                }
                if *strict {
                    cmd.args(["-Wall", "-Wextra", "-Werror"]);
                } else {
                    cmd.arg("-w");
                }
                cmd.arg("-isystem")
                    .arg(doom_src.join("include"))
                    .arg("-I")
                    .arg(&doom_src)
                    .arg("-I")
                    .arg(&m6m_src)
                    .arg("-I")
                    .arg(dg_dir)
                    .arg("-c")
                    .arg(src)
                    .arg("-o")
                    .arg(obj);

                if let Err(e) = run_cmd(&mut cmd, &format!("clang {}", src.display())) {
                    let mut slot = first_err.lock().unwrap();
                    if slot.is_none() {
                        *slot = Some(e);
                    }
                    return;
                }
            });
        }
    });

    if let Some(err) = first_err.into_inner().unwrap() {
        return Err(err);
    }

    let elf_path = work.join("doom.elf");
    let map_path = work.join("doom.map");
    let mut link = Command::new("ld.lld");
    link.arg("-T")
        .arg(doom_src.join("doom.ld"))
        .args(["--gc-sections", "--no-undefined", "-Map"])
        .arg(&map_path)
        .arg("-o")
        .arg(&elf_path);
    for (_, obj, _) in &jobs {
        link.arg(obj);
    }
    run_cmd(&mut link, "ld.lld DOOM.BIN")?;

    let elf_bytes = fs::read(&elf_path).map_err(|e| format!("read {}: {e}", elf_path.display()))?;
    let elf = Elf::parse(&elf_bytes)?;
    let image_end = elf.symbol("__image_end").ok_or("doom.elf missing __image_end")?.value;
    let bss_end = elf.symbol("__bss_end").ok_or("doom.elf missing __bss_end")?.value;
    if image_end <= APP_LOAD_VA || bss_end < image_end || bss_end > APP_LIMIT_VA {
        return Err(format!(
            "DOOM.BIN layout out of arena: load={APP_LOAD_VA:#x} image_end={image_end:#x} bss_end={bss_end:#x}"
        ));
    }

    let img_len = (image_end - APP_LOAD_VA) as usize;
    if img_len < 32 {
        return Err(format!("DOOM.BIN image too small for D2AP header: {img_len} B"));
    }
    let mut bin = vec![0u8; img_len];
    for sec in elf.sections.iter().filter(|s| s.flags & elf::SHF_ALLOC != 0 && s.kind != elf::SHT_NOBITS && s.size > 0) {
        if sec.addr < APP_LOAD_VA || sec.addr.saturating_add(sec.size) > image_end {
            return Err(format!("section {} @ {:#x} outside DOOM.BIN image", sec.name, sec.addr));
        }
        let off = (sec.addr - APP_LOAD_VA) as usize;
        bin[off..off + sec.data.len()].copy_from_slice(sec.data);
    }

    if warp_e1m1 {
        let old_flags = u32::from_be_bytes(bin[24..28].try_into().unwrap());
        bin[24..28].copy_from_slice(&(old_flags | 1).to_be_bytes());
    }

    let field = |off: usize| u32::from_be_bytes(bin[off..off + 4].try_into().unwrap());
    let magic = field(0);
    let abi_ver = field(4);
    let load_va = field(8);
    let entry_va = field(12);
    let header_image_size = field(16);
    let header_bss_end = field(20);
    if magic != APP_MAGIC {
        return Err(format!("DOOM.BIN header magic mismatch: {magic:#010x}"));
    }
    if abi_ver != APP_ABI_VER || load_va != APP_LOAD_VA {
        return Err(format!(
            "DOOM.BIN ABI/load mismatch: abi={abi_ver}, load={load_va:#010x}"
        ));
    }
    if header_image_size != img_len as u32 || header_bss_end != bss_end {
        return Err(format!(
            "DOOM.BIN layout header mismatch: image={header_image_size:#x}/{img_len:#x}, bss={header_bss_end:#010x}/{bss_end:#010x}"
        ));
    }
    let entry_base = entry_va & !1;
    if !(APP_LOAD_VA..image_end).contains(&entry_base) {
        return Err(format!("DOOM.BIN entry outside image: {entry_va:#010x}"));
    }

    fs::create_dir_all(out_dir).map_err(|e| format!("mkdir {}: {e}", out_dir.display()))?;
    let out_bin = out_dir.join("DOOM.BIN");
    fs::write(&out_bin, &bin).map_err(|e| format!("write {}: {e}", out_bin.display()))?;

    println!("Built {} ({} B):", out_bin.display(), bin.len());
    println!(
        "  Arena : {APP_LOAD_VA:#010x}..{image_end:#010x} image ({:.1} KB), .bss..{bss_end:#010x} ({:.1} KB), headroom {:.1} KB",
        bin.len() as f64 / 1024.0,
        (bss_end - image_end) as f64 / 1024.0,
        (APP_LIMIT_VA - bss_end) as f64 / 1024.0,
    );
    println!("  Entry : {entry_va:#010x}{}", if warp_e1m1 { " (flags=warp E1M1)" } else { "" });
    Ok(out_bin)
}
