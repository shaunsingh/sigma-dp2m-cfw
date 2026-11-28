//! m6m (arm926ej-s BE, `C78FRM.bin`): link the C/asm payload into reclaimed S3 regions & apply its hooks

#[cfg(test)]
mod bench;
mod codegen;

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::config::Config;
use crate::elf::{self, Elf};
use crate::pack::FilmPack;

pub const C78_S3_OFF: usize = 0x010A_4100;
pub const C78_LEN: usize = 0x0082_0000;
const S3_IN_C78: usize = 0x0002_0000;
const VA_BASE: u32 = 0xA000_0000;

/// S3 image (`C78FRM.bin[0x20000..]`) addressed by VA.
pub type S3 = [u8];

// payload regions, emitted as the linker MEMORY block; S3 runs from SDRAM so all are RAM seeded by the image
const REGIONS: &[(&str, u32, u32)] = &[
    // factory screens 33..49 (vtable slots retired to dp2m_ret0; slot 33 hosts SCREEN_DOOM when enabled)
    ("CODE", 0xA005_9D50, 0xA005_D924),
    // f64 table reachable only from orphaned 0xA0034870 / 0xA00348CA
    ("DATA", 0xA026_5670, 0xA026_8870),
    // orphaned Im_TFR driver (23 fns, no references)
    ("LOOKS", 0xA009_4658, 0xA009_688C),
    // factory service menus 0x18..0x22 + their submenus; menu 0xA3 at 0xA07EDC0C stays
    ("MENU", 0xA07E_C9CC, 0xA07E_DC0C),
    // verified zero-xref stock-orphan code runs outside 0xA0070000..0xA00B8000 for DOOM applet
    ("DOOM_A", 0xA010_AF44, 0xA010_C250),
    ("DOOM_B", 0xA010_7BD8, 0xA010_8E14),
    ("DOOM_C", 0xA013_60FC, 0xA013_6AB8),
    ("DOOM_D", 0xA014_0CB0, 0xA014_15B0),
    ("DOOM_E", 0xA002_A834, 0xA002_B5E0),
];
// unreferenced SDRAM between the uITRON task stacks (0xA1B5717C) and the OSD planes (0xA2130000)
const RAM: (u32, u32) = (0xA1C0_0000, 0xA1E0_0000);

// never touch
const FPGA: (u32, u32) = (0xA017_9000, 0xA023_D35A);
const VECTOR_SOURCE: (u32, u32) = (0xA07F_BC1C, 0xA080_0000);
const SHARED_LITERAL: (u32, u32) = (0xA004_D3D0, 0xA07E_EFCC);
const LIVE_SCREENS: (u32, u32) = (0xA005_D924, 0xA005_F1A0);
const LIVE_VTABLE: (u32, u32) = (0xA07D_EBF0, 0xA07D_EC0C);

/// Screen vtable slots (`0xA07DEB28 + 4 * id`) of factory-only screens 33..49 now holding payload.
const DEAD_SCREEN_SLOTS: (u32, u32) = (0xA07D_EBAC, 0xA07D_EBF0);

const VERSION_IMM: u32 = 0xA005_F87C;
const UPDATER_BANNERS: &[(u32, &[u8], &[u8])] = &[
    (0xA005_FC3C, b"M6M5 Both Debug Write.\0\0", b"dp2m-fw (M6M + M5)"),
    (0xA005_FC5C, b"M6M Only Debug Write.\0\0\0", b"dp2m-fw (M6M)"),
    (0xA005_FC7C, b"M5 Only Debug Write.\0\0\0\0", b"dp2m-fw (M5)"),
];

const HOOK_CALL: u32 = 1;
const HOOK_BL: u32 = 2;
const HOOK_WORD: u32 = 3;
const HOOK_HALF: u32 = 4;

fn s3_off(va: u32) -> usize {
    (va - VA_BASE) as usize
}

pub fn read(s3: &S3, va: u32, n: usize) -> &[u8] {
    &s3[s3_off(va)..s3_off(va) + n]
}

pub fn read_u32s<const N: usize>(s3: &S3, va: u32) -> [u32; N] {
    std::array::from_fn(|i| u32::from_be_bytes(read(s3, va + 4 * i as u32, 4).try_into().unwrap()))
}

fn write(s3: &mut S3, va: u32, data: &[u8]) {
    s3[s3_off(va)..s3_off(va) + data.len()].copy_from_slice(data);
}

fn be32(s3: &S3, va: u32) -> u32 {
    read_u32s::<1>(s3, va)[0]
}

fn thumb_bl(src: u32, dst: u32) -> Result<[u8; 4], String> {
    let d = dst as i64 - (src as i64 + 4);
    if !(-(1 << 22)..(1 << 22)).contains(&d) || d & 1 != 0 {
        return Err(format!("BL {src:#x} -> {dst:#x} out of range or unaligned"));
    }
    Ok((0xF000_F800 | (((d >> 12) as u32 & 0x7FF) << 16) | ((d >> 1) as u32 & 0x7FF)).to_be_bytes())
}

/// Callee of a Thumb BL/BLX pair (BLX targets are word aligned ARM).
fn thumb_call_target(src: u32, w: u32) -> Option<u32> {
    let (hi, lo) = (w >> 16, w & 0xFFFF);
    if hi & 0xF800 != 0xF000 || !matches!(lo & 0xF800, 0xF800 | 0xE800) {
        return None;
    }
    let off = (((hi & 0x7FF) << 12 | (lo & 0x7FF) << 1) as i32) << 9 >> 9;
    let t = (src + 4).wrapping_add(off as u32);
    Some(if lo & 0xF800 == 0xE800 { t & !3 } else { t })
}

struct Toolchain<'a> {
    src: &'a Path,
    work: &'a Path,
}

impl Toolchain<'_> {
    fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
        let out = cmd.output().map_err(|e| format!("{what}: {e} (needs clang & ld.lld on PATH)"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("{what} failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
        }
    }

    fn compile(&self, file: &Path) -> Result<std::path::PathBuf, String> {
        let name = file.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let opt = if matches!(name, "rt" | "hud") { "-O2" } else { "-Os" };
        let obj = self.work.join(file.file_name().unwrap()).with_extension("o");
        Self::run(
            Command::new("clang")
                .args(["--target=armeb-none-eabi", "-mcpu=arm926ej-s", "-mthumb", opt, "-std=c11"])
                .args(["-ffreestanding", "-fno-builtin", "-fno-stack-protector", "-fno-common"])
                .args(["-ffunction-sections", "-fdata-sections", "-Wall", "-Wextra", "-Werror"])
                .arg("-I")
                .arg(self.src)
                .arg("-I")
                .arg(self.work)
                .arg("-c")
                .arg(file)
                .arg("-o")
                .arg(&obj),
            &format!("clang {}", file.display()),
        )?;
        Ok(obj)
    }

    fn link(&self, objs: &[std::path::PathBuf]) -> Result<Vec<u8>, String> {
        let mut mem = String::from("MEMORY {\n");
        for (name, lo, hi) in REGIONS.iter().copied().chain([("RAM", RAM.0, RAM.1)]) {
            mem += &format!("  {name} : ORIGIN = {lo:#X}, LENGTH = {:#X}\n", hi - lo);
        }
        mem += "}\n";
        fs::write(self.work.join("regions.ld"), mem).map_err(|e| e.to_string())?;
        let elf = self.work.join("m6m.elf");
        Self::run(
            Command::new("ld.lld")
                .arg("-L")
                .arg(self.work)
                .arg("-T")
                .arg(self.src.join("m6m.ld"))
                .args(["--gc-sections", "--no-undefined", "-Map"])
                .arg(self.work.join("m6m.map"))
                .arg("-o")
                .arg(&elf)
                .args(objs),
            "ld.lld",
        )?;
        fs::read(&elf).map_err(|e| format!("read {}: {e}", elf.display()))
    }
}

pub fn build_m6m(fw_raw: &[u8], pack: &FilmPack, src: &Path, work: &Path, cfg: &Config) -> Result<Vec<u8>, String> {
    if fw_raw.len() < C78_S3_OFF + C78_LEN || &fw_raw[C78_S3_OFF..C78_S3_OFF + 8] != b"FUJITSU!" {
        return Err("DP2MV105.bin has no M6M C78FRM image at 0x10A4100".into());
    }
    let stock = &fw_raw[C78_S3_OFF..C78_S3_OFF + C78_LEN];
    let mut c78 = stock.to_vec();
    let stock_s3 = &stock[S3_IN_C78..];

    let g = codegen::generate(stock_s3, pack, cfg)?;
    fs::create_dir_all(work).map_err(|e| format!("mkdir {}: {e}", work.display()))?;
    fs::write(work.join("catalog.h"), &g.header).map_err(|e| e.to_string())?;
    fs::write(work.join("catalog.c"), &g.source).map_err(|e| e.to_string())?;

    let tc = Toolchain { src, work };
    let mut sources: Vec<_> = fs::read_dir(src)
        .map_err(|e| format!("{}: {e}", src.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("c" | "S")))
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if cfg.m6m.probe {
                // Keep the portal upscaler for direct timing; GC drops the unused renderer.
                name != "doom.c"
            } else {
                name != "probe.c"
            }
        })
        .collect();
    sources.sort();
    sources.push(work.join("catalog.c"));
    let objs = sources.iter().map(|s| tc.compile(s)).collect::<Result<Vec<_>, _>>()?;
    let image = tc.link(&objs)?;
    let elf = Elf::parse(&image)?;

    let s3 = &mut c78[S3_IN_C78..];
    let mut used = Vec::new();
    for sec in elf.sections.iter().filter(|s| s.flags & elf::SHF_ALLOC != 0 && s.size > 0) {
        let lo = sec.addr;
        let hi = lo.checked_add(sec.size).ok_or_else(|| format!("{} address range overflows", sec.name))?;
        if sec.kind == elf::SHT_NOBITS {
            if lo < RAM.0 || hi > RAM.1 {
                return Err(format!("{} {lo:#x}..{hi:#x} outside payload RAM", sec.name));
            }
            continue;
        }
        let region = REGIONS.iter().find(|r| lo >= r.1 && hi <= r.2).ok_or_else(|| format!("{} {lo:#x}..{hi:#x} outside payload regions", sec.name))?;
        if sec.flags & elf::SHF_EXECINSTR != 0 && lo < 0xA00B_8000 && hi > 0xA007_0000 {
            return Err(format!("executable section {} {lo:#x}..{hi:#x} overlaps simulator HLE stub range 0xA0070000..0xA00B8000", sec.name));
        }
        used.push((region.0, sec.name.clone(), sec.size));
        write(s3, lo, sec.data);
    }

    let ret0 = elf.symbol("dp2m_ret0").ok_or("payload lacks dp2m_ret0")?.value;
    let doom_scr = elf.symbol("doom_screen").map(|s| s.value);
    for slot in (DEAD_SCREEN_SLOTS.0..DEAD_SCREEN_SLOTS.1).step_by(4) {
        let old = be32(stock_s3, slot);
        if !(REGIONS[0].1..REGIONS[0].2).contains(&(old & !1)) {
            return Err(format!("screen vtable {slot:#x} = {old:#x} is not a reclaimed factory screen"));
        }
        let target = if slot == DEAD_SCREEN_SLOTS.0 {
            doom_scr.unwrap_or(ret0)
        } else {
            ret0
        };
        write(s3, slot, &(target | 1).to_be_bytes());
    }

    let section = elf.section(".dp2m.hooks").ok_or("payload has no .dp2m.hooks")?;
    let (hooks, partial) = section.data.as_chunks::<16>();
    if hooks.is_empty() || !partial.is_empty() {
        return Err("expected nonempty .dp2m.hooks with 16-byte records".into());
    }
    for h in hooks {
        let [kind, site, target, expect] = std::array::from_fn(|i| u32::from_be_bytes(h[4 * i..4 * i + 4].try_into().unwrap()));
        let width = if kind == HOOK_HALF { 2 } else { 4 };
        if site & 1 != 0 || site < VA_BASE || (site - VA_BASE) as usize > s3.len() - width {
            return Err(format!("hook {kind} @ {site:#010x} is unaligned or outside S3"));
        }
        let cur = if kind == HOOK_HALF {
            u16::from_be_bytes(read(s3, site, 2).try_into().unwrap()) as u32
        } else {
            be32(s3, site)
        };
        let ok = match kind {
            HOOK_CALL => thumb_call_target(site, cur) == Some(expect & !1),
            HOOK_BL | HOOK_WORD | HOOK_HALF => cur == expect,
            _ => return Err(format!("unknown hook kind {kind} @ {site:#x}")),
        };
        if !ok {
            return Err(format!("hook {kind} @ {site:#010x}: stock {cur:#010x} != expected {expect:#010x}"));
        }
        match kind {
            HOOK_CALL | HOOK_BL => write(s3, site, &thumb_bl(site, target & !1)?),
            HOOK_WORD => write(s3, site, &target.to_be_bytes()),
            _ => write(s3, site, &(target as u16).to_be_bytes()),
        }
    }

    let (major, minor) = cfg.version_pair()?;
    if read(s3, VERSION_IMM, 4) != [0x22, 0x01, 0x23, 0x05] {
        return Err("'Current version' immediates mismatch @ 0xA005F87C".into());
    }
    write(s3, VERSION_IMM, &[0x22, major, 0x23, minor]);
    for &(va, orig, repl) in UPDATER_BANNERS {
        if read(s3, va, orig.len()) != orig {
            return Err(format!("updater banner mismatch @ {va:#010x}"));
        }
        let mut slot = vec![0; orig.len()];
        slot[..repl.len()].copy_from_slice(repl);
        write(s3, va, &slot);
    }

    let syms: String = elf
        .symbols
        .iter()
        .filter(|s| !s.name.is_empty() && s.value >= VA_BASE)
        .map(|s| format!("{:08x} {:6} {}\n", s.value, s.size, s.name))
        .collect();
    fs::write(work.join("m6m.syms"), syms).map_err(|e| e.to_string())?;

    for (region, lo, hi) in REGIONS {
        let n: u32 = used.iter().filter(|u| u.0 == *region).map(|u| u.2).sum();
        print!("  [M6M {region}] {n}/{} B", hi - lo);
    }
    println!("\n  [M6M payload] {} hooks, {}", hooks.len(), g.stats);

    verify_m6m_invariants(&c78, stock)?;
    Ok(c78)
}

fn verify_m6m_invariants(c78: &[u8], stock: &[u8]) -> Result<(), String> {
    let (s3, st) = (&c78[S3_IN_C78..], &stock[S3_IN_C78..]);
    let same = |lo: u32, hi: u32| read(s3, lo, (hi - lo) as usize) == read(st, lo, (hi - lo) as usize);
    for (what, (lo, hi)) in [
        ("Spartan-6 FPGA bitstream", FPGA),
        ("live screens 50..55", LIVE_SCREENS),
        ("live screen vtable", LIVE_VTABLE),
        ("exception vector source", VECTOR_SOURCE),
    ] {
        if !same(lo, hi) {
            return Err(format!("CRITICAL: {what} modified"));
        }
    }
    if be32(s3, SHARED_LITERAL.0) != SHARED_LITERAL.1 {
        return Err("CRITICAL: shared literal 0xA004D3D0 modified".into());
    }
    if c78[..S3_IN_C78] != stock[..S3_IN_C78] {
        return Err("CRITICAL: IPL modified".into());
    }
    println!("  [VERIFY M6M] FPGA, IPL, vector source, shared literal, live screens & vtable: PASS");
    Ok(())
}
