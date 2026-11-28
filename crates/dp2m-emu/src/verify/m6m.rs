//! m6m suite (308 checks): invariants, hooks, menus, strings, NVRAM settings, bracketing,
//! auto-ISO min shutter, IPC packing, makernotes, Live View B2Y, Y-cap, HUD overlays, updater & package.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::rc::Rc;
use std::cell::RefCell;

use crate::image::{BSS, END, FPGA, IPL_LEN, elf_section};
use crate::machine::{Machine, Nor, SCRATCH, STACK, outside};
use crate::stock;

use super::{Build, FilmEntry, Report, menu_tables, read_rows_img, read_rows_mach};

const FILM_MODE0: u32 = 9;
const NV_RESERVED: (u32, u32) = (stock::NV_BLOCK + 0x04, stock::NV_BLOCK + 0x14);
const LV_DIRTY: (u32, u32) = (0xA07B_1398, 0xA07B_139C);
const LV_CMCM: u32 = 1;
const LV_MONO_Q7: u32 = 2;
const LV_KNOTS: u32 = 4;
const PAGE_RESET: u32 = 0xA005_8A0A;
const PAGE_LIST: u32 = 0xA07F_230E;
const SID0: u32 = 672;
const MN_SWITCH_SETUP: u32 = 0xA014_8CE4;
const MN_SWITCH_DONE: u32 = 0xA014_8D2E;
const MN_TAG_3D: u32 = 0xA07F_7D78;
const MN_TAG_59: u32 = 0xA07F_8320;

fn paper_nv(cls: u32) -> &'static str {
    match cls {
        1 => "color_paper",
        2 => "bw_paper",
        3 => "lut_slot",
        _ => "",
    }
}

fn qs_slot_of(getter_va: u32) -> Option<(u32, u32)> {
    match getter_va {
        0xA016_84FE => Some((0, 0)),
        0xA016_8504 => Some((0, 1)),
        0xA016_850A => Some((0, 2)),
        0xA016_8510 => Some((0, 3)),
        0xA016_8516 => Some((1, 0)),
        0xA016_851C => Some((1, 1)),
        0xA016_8522 => Some((1, 2)),
        0xA016_8528 => Some((1, 3)),
        _ => None,
    }
}

fn menu_map(b: &Build) -> BTreeMap<u32, u32> {
    let mut out = BTreeMap::new();
    for name in b.catalog.menus.keys() {
        if let Some(hex) = name.strip_prefix("m_")
            && !hex.is_empty()
            && hex.bytes().all(|c| c.is_ascii_hexdigit())
            && let Ok(id) = u32::from_str_radix(hex, 16)
        {
            out.insert(id, b.sym(name));
        }
    }
    out
}

fn packed_of(f: &FilmEntry, nv: &HashMap<&str, u32>) -> u32 {
    if f.paper_cls == 0 {
        return f.packed;
    }
    let p = nv.get(paper_nv(f.paper_cls)).copied().unwrap_or(0);
    f.packed + if (1..=8).contains(&p) { p - 1 } else { f.auto_paper }
}

fn expand_knots(k: &[u8; 18]) -> Vec<u8> {
    let mut out = Vec::with_capacity(1024);
    let mut i = 0;
    for (sh, cnt) in [(4u32, 8usize), (6, 4), (7, 5)] {
        for _ in 0..cnt {
            let y0 = k[i] as i32;
            let dy = k[i + 1] as i32 - y0;
            for s in 0..(1i32 << sh) {
                let v = y0 + ((dy * s + (1 << (sh - 1))) >> sh);
                out.push((v & 0xFF) as u8);
            }
            i += 1;
        }
    }
    out
}

fn lv_matrix(cmcm: &[i32; 9], flags: u32, cc: &[i32; 9]) -> [u16; 14] {
    let mut m = [0u16; 14];
    m[9] = 0x004D;
    m[10] = 0x0096;
    m[11] = 0x001D;
    m[12] = 0x0200;
    for i in 0..3 {
        for j in 0..3 {
            let v = if flags & LV_MONO_Q7 != 0 {
                cmcm[i * 3 + j]
            } else {
                let s = (cmcm[i * 3] * cc[j] + cmcm[i * 3 + 1] * cc[3 + j] + cmcm[i * 3 + 2] * cc[6 + j] + 512) >> 10;
                s.clamp(-2047, 2047)
            };
            m[i * 3 + j] = (v as u16) & 0x0FFF;
        }
    }
    m
}

fn check_invariants(b: &Build, r: &mut Report) {
    r.check(b.img.c78[..IPL_LEN] == b.stock.c78[..IPL_LEN], "IPL is stock");
    r.check(
        b.img.rd(FPGA.0, (FPGA.1 - FPGA.0) as usize) == b.stock.rd(FPGA.0, (FPGA.1 - FPGA.0) as usize),
        "FPGA bitstream is stock",
    );
    r.check(
        b.img.rd(BSS, (END - BSS) as usize) == b.stock.rd(BSS, (END - BSS) as usize),
        ".bss tail is stock",
    );
    r.eq(b.img.u32(0xA004_D3D0), 0xA07E_EFCC, "shared literal 0xA004D3D0 intact");
    r.check(
        b.img.rd(0xA005_D924, 0xA005_F1A0 - 0xA005_D924) == b.stock.rd(0xA005_D924, 0xA005_F1A0 - 0xA005_D924),
        "live screens 50..55 intact",
    );
    let bss_end = b.sym("__bss_end");
    r.check(
        (0xA1C0_0000..=0xA1CC_0000).contains(&bss_end),
        format!("payload __bss_end ({bss_end:#x}) stays below D2AP applet arena 0xA1CC0000"),
    );
    r.eq(
        (b.img.u32(0xA017_2170), b.img.u32(0xA017_2190)),
        (0xE3A0_000C, 0xE380_0C06),
        "boot MPU Region 2 enables I-cache (c2=0xC) and Priv/User execute AP (c5.i R2=6)",
    );
}

fn check_hooks(b: &Build, r: &mut Report) -> Result<(), String> {
    let elf_raw = fs::read(b.dir.join("m6m/m6m.elf")).map_err(|e| format!("read m6m.elf: {e}"))?;
    let sec = elf_section(&elf_raw, b".dp2m.hooks")?;
    let n = sec.len() / 16;
    r.check(n > 0 && sec.len() == n * 16, format!(".dp2m.hooks has {n} records"));
    let (mut bad_stock, mut bad_img) = (Vec::new(), Vec::new());
    for chunk in sec.as_chunks::<16>().0 {
        let kind = u32::from_be_bytes(chunk[0..4].try_into().unwrap());
        let site = u32::from_be_bytes(chunk[4..8].try_into().unwrap());
        let target = u32::from_be_bytes(chunk[8..12].try_into().unwrap());
        let expect = u32::from_be_bytes(chunk[12..16].try_into().unwrap());
        let st_ok = match kind {
            1 => b.stock.bl_target(site) == Some(expect & !1),
            2 | 3 => b.stock.u32(site) == expect,
            4 => b.stock.u16(site) as u32 == expect,
            _ => false,
        };
        let im_ok = match kind {
            1 | 2 => b.img.bl_target(site) == Some(target & !1) && (b.img.u16(site + 2) & 0xF800) == 0xF800,
            3 => b.img.u32(site) == target,
            4 => b.img.u16(site) == (target as u16),
            _ => false,
        };
        if !st_ok {
            bad_stock.push((kind, site));
        }
        if !im_ok {
            bad_img.push((kind, site));
        }
    }
    r.eq(bad_stock, vec![], format!(".dp2m.hooks expect values match stock across all {n} sites"));
    r.eq(bad_img, vec![], format!(".dp2m.hooks patches applied in C78FRM.bin across all {n} sites"));

    let ret0 = b.sym("dp2m_ret0") | 1;
    let doom_scr = b.syms.get("doom_screen").map(|s| s.va() | 1);
    let dead_ok = (0xA07D_EBAC..0xA07D_EBF0).step_by(4).all(|s| {
        let st = b.stock.u32(s) & !1;
        let exp = if s == 0xA07D_EBAC { doom_scr.unwrap_or(ret0) } else { ret0 };
        (0xA005_9D50..0xA005_D924).contains(&st) && b.img.u32(s) == exp
    });
    r.check(dead_ok, "retired factory screen vtable slots 33..49 -> doom_screen / dp2m_ret0");
    r.check(
        b.img.rd(0xA07D_EB28, 0x84) == b.stock.rd(0xA07D_EB28, 0x84)
            && b.img.rd(0xA07D_EBF0, 0x1C) == b.stock.rd(0xA07D_EBF0, 0x1C),
        "live screen vtable slots 0..32 & 50..55 intact",
    );
    let mut m = Machine::new(&b.img, 0);
    r.eq(m.call(b.sym("dp2m_ret0"), &[])?, 0, "dp2m_ret0 returns 0");

    // Verify shutter_guard_hook @ 0xA01690B6 blocks S1/S2 on SCREEN_DOOM (0x21) and delegates on LV (0x15)
    let sg = b.sym("shutter_guard_hook");
    r.eq(b.img.bl_target(0xA016_90B6), Some(sg), "BL @ 0xA01690B6 -> shutter_guard_hook");
    let cur_scr = Rc::new(std::cell::Cell::new(0x21u32));
    {
        let cs = cur_scr.clone();
        m.stub_on(b.sym("stock_ui_screen_id"), move |_| Some(cs.get()));
    }
    m.stub_on(b.sym("stock_busy_check"), |_| Some(0));
    let sg_doom = m.call(sg, &[])?;
    cur_scr.set(0x15);
    let sg_lv = m.call(sg, &[])?;
    r.eq((sg_doom, sg_lv), (1, 0), "shutter_guard_hook returns busy (1) on SCREEN_DOOM and delegates (0) on SCREEN_LV");
    Ok(())
}

fn check_menus(b: &Build, r: &mut Report) -> Result<(), String> {
    let mut m = Machine::new(&b.img, 0);
    let mut ms = Machine::new(&b.stock, 0);
    m.load_nv_defaults();
    ms.load_nv_defaults();

    let gmap = menu_map(b);
    let (film_page, paper, paper0, qs) = (
        b.cat("MENU_FILM_PAGE"),
        b.cat("MENU_PAPER"),
        b.cat("MENU_PAPER_CLS0"),
        b.cat("MENU_QS_COPY"),
    );
    r.check(gmap.contains_key(&film_page), format!("Film page {film_page:#x} in the generated menu map"));
    let names: HashMap<u32, &str> = b.syms.iter().map(|(n, s)| (s.va(), n.as_str())).collect();
    for (&mid, &rows) in &gmap {
        let label = names.get(&rows).map_or_else(|| format!("{rows:#x}"), |s| (*s).to_owned());
        r.eq(m.menu_table(mid)?, rows, format!("Get_Menu_Table({mid:#x}) -> {label}"));
    }
    for (name, rows) in &b.catalog.menus {
        let got: Vec<[u32; 8]> = read_rows_img(&b.img, b.sym(name), 128).into_iter().map(|x| x.w).collect();
        let exp: Vec<[u32; 8]> = rows
            .iter()
            .map(|row| core::array::from_fn(|k| b.value(&row[k])))
            .collect();
        let bad = (0..got.len().max(exp.len())).find(|&k| got.get(k) != exp.get(k));
        let msg = match bad {
            None => format!("{name}: {} rows = catalog.c", exp.len()),
            Some(k) => format!("{name}: {} rows = catalog.c (row {k} differs)", exp.len()),
        };
        r.check(bad.is_none(), msg);
    }
    let fp_rows = read_rows_mach(&m, gmap[&film_page], 128);
    let mut subs = Vec::new();
    let mut subs_ok = true;
    for row in &fp_rows {
        let t = m.menu_table(row.id())?;
        subs.push(format!("{:#x}", row.id()));
        if t == 0 {
            subs_ok = false;
        }
    }
    r.check(subs_ok, format!("Film page submenus resolve [{}]", subs.join(", ")));

    let mut exp_pages = vec![0x0Eu16, 0x0F, film_page as u16];
    exp_pages.extend(0x10u16..0x18);
    let rd_pages = |raw: &[u8]| -> Vec<u16> { raw.as_chunks::<2>().0.iter().map(|&c| u16::from_be_bytes(c)).collect() };
    r.eq(rd_pages(b.img.rd(0xA07F_22E6, 22)), exp_pages.clone(), "menu page template (state + 0x14) = 11 pages");
    r.eq(rd_pages(b.img.rd(PAGE_LIST, 22)), exp_pages.clone(), "menu live page list (state + 0x3C) = 11 pages");
    m.wr(PAGE_LIST, &[0u8; 22]);
    m.call(PAGE_RESET, &[])?;
    r.eq(rd_pages(&m.rd(PAGE_LIST, 22)), exp_pages, "menu page list after the entry reset 0xA0058A0A");

    let pg = b.sym("page_group");
    let pg_sites = [0xA005_811Cu32, 0xA005_8144, 0xA005_8204, 0xA005_82C6, 0xA005_8308, 0xA005_84E8];
    r.check(pg_sites.iter().all(|&s| b.img.bl_target(s) == Some(pg)), "all 6 menu page_group call sites hooked");
    let mut pg_got = Vec::new();
    for p in 0..11u32 {
        pg_got.push(m.call(pg, &[p])?);
    }
    r.eq(pg_got, vec![0, 0, 0, 0, 0, 1, 1, 2, 2, 2, 2], "page_group(0..10) -> shoot 0..4, playback 5..6, setup 7..10");

    let tab_geom = [
        b.img.u16(0xA005_844A), b.img.u16(0xA005_8454),
        b.img.u16(0xA005_868A), b.img.u16(0xA005_86A0), b.img.u16(0xA005_871C), b.img.u16(0xA005_8734),
        b.img.u16(0xA04F_3100), b.img.u16(0xA04F_3104), b.img.u16(0xA04F_3110), b.img.u16(0xA04F_3114),
        b.img.u16(0xA04F_3118), b.img.u16(0xA04F_311C), b.img.u16(0xA04F_3120), b.img.u16(0xA04F_3124),
    ];
    r.eq(
        tab_geom,
        [0x3034, 0x2D0B, 0x210A, 0x210A, 0x210A, 0x210A, 34, 48, 32, 260, 292, 104, 396, 208],
        "11-tab strip stride 52 px, wrap 10, and 5/2/4 group spans",
    );

    let mut pin_got = Vec::new();
    for (expo, pages) in [(8u8, &[0u16, 4, 5, 6, 7, 10][..]), (1, &[0, 2, 5, 6, 10][..])] {
        m.w8(stock::NV_EXPO, expo);
        for &p in pages {
            m.w16(SCRATCH + 4, p);
            m.run_slice(0xA005_87F8, 0xA005_882E, 1_000_000, STACK.1, &[(4, SCRATCH)])?;
            pin_got.push(m.u16(SCRATCH + 4));
        }
    }
    r.eq(pin_got, vec![5, 5, 5, 6, 5, 5, 0, 2, 0, 0, 10], "playback pins menu to pages 5..6; shooting resets 5..6 -> 0");
    m.load_nv_defaults();

    for cls in 0..4u32 {
        let cm = b.catalog.films.iter().find(|f| f.paper_cls == cls).map_or(0, |f| f.cm);
        m.w8(stock::NV_CM, cm as u8);
        r.eq(m.menu_table(paper)?, gmap[&(paper0 + cls)], format!("Get_Menu_Table({paper:#x}) for cm {cm} -> paper class {cls}"));
        r.eq(
            m.menu_table(paper | qs)?,
            gmap[&((paper0 + cls) | qs)],
            format!("Get_Menu_Table({:#x}) for cm {cm} -> QS paper class {cls}", paper | qs),
        );
    }
    m.load_nv_defaults();

    let (lbl_l, lbl_s) = (b.cat("ICON_LABEL_L"), b.cat("ICON_LABEL_S"));
    for (&mid, &t) in gmap.iter().filter(|&(&mid, _)| mid & qs != 0) {
        let mut bad = Vec::new();
        for row in read_rows_mach(&m, t, 128) {
            let k = (row.qs_icon() & 0x3FF) as usize;
            let text = if row.sid() < SID0 {
                m.ui_string(row.sid(), 0)?.unwrap_or_default()
            } else {
                b.catalog.strings[(row.sid() - SID0) as usize].clone()
            };
            if row.qs_icon() != lbl_l + k as u32 || row.strip_icon() != lbl_s + k as u32 || b.catalog.strings.get(k) != Some(&text) {
                bad.push(row.sid());
            }
        }
        r.eq(bad, vec![], format!("QS copy {mid:#x}: value label icons match their text"));
    }

    for mid in 0xE5u32..0xED {
        let items: Vec<_> = read_rows_mach(&m, gmap[&mid], 128).into_iter().filter(|row| row.value() & qs != 0).collect();
        let (page, slot) = qs_slot_of(items[0].get() & !1).ok_or("unknown QS slot getter")?;
        let mut ok = items.len() == 5;
        for row in &items {
            m.call(row.set() & !1, &[row.value()])?;
            let got_slot = m.call(stock::QS_SLOT_ROWS, &[page, slot])?;
            let got_tbl = m.menu_table(row.value())?;
            let exp = gmap.get(&row.value()).copied().unwrap_or_else(|| gmap[&(paper0 | qs)]);
            if got_slot != got_tbl || got_tbl != exp {
                ok = false;
            }
        }
        r.check(ok, format!("QS slot menu {mid:#x} (page {page} slot {slot}): {} film items open their QS copies", items.len()));
    }
    m.load_nv_defaults();

    let mut iso_vas = BTreeSet::new();
    for (mid, n1, n3) in [
        (stock::MENU_ISO, "m_iso_1ev", "m_iso_3ev"),
        (stock::MENU_ISO_MIN, "m_iso_min_1ev", "m_iso_min_3ev"),
    ] {
        for (step, name) in [(0u8, n1), (1u8, n3)] {
            if let Some(sym) = b.syms.get(name) {
                iso_vas.insert(sym.va());
                m.w8(stock::NV_ISO_STEP, step);
                r.eq(
                    m.menu_table(mid)?,
                    sym.va(),
                    format!("Get_Menu_Table({mid:#x}) with ISO step {step} -> {name}"),
                );
            }
        }
    }
    m.load_nv_defaults();

    let st = menu_tables(&mut ms, 0..0x200);
    let bt = menu_tables(&mut m, 0..0x200);
    let all_ids: BTreeSet<u32> = st.keys().chain(bt.keys()).copied().collect();
    let changed: Vec<u32> = all_ids
        .into_iter()
        .filter(|i| {
            !gmap.contains_key(i)
                && *i != paper
                && !bt.get(i).is_some_and(|v| iso_vas.contains(v))
                && st.get(i) != bt.get(i)
        })
        .collect();
    r.eq(changed, vec![], format!("other menu ids resolve as stock ({} stock ids)", st.len()));
    Ok(())
}

fn check_strings(b: &Build, r: &mut Report) -> Result<(), String> {
    let mut m = Machine::new(&b.img, 0);
    let mut ms = Machine::new(&b.stock, 0);
    m.load_nv_defaults();
    ms.load_nv_defaults();

    let strs = &b.catalog.strings;
    r.eq(b.img.u32(b.sym("dp2m_string_count")), strs.len() as u32, "dp2m_string_count = catalog.c");
    let (sid0, title) = (stock::STOCK_SIDS, b.cat("SID_PAPER_TITLE"));
    let mut bad = Vec::new();
    for (i, s) in strs.iter().enumerate() {
        let sid = sid0 + i as u32;
        let exp = if sid == title { &strs[i + 1] } else { s };
        for lang in [0u32, 1, 16] {
            if m.ui_string(sid, lang)?.as_deref() != Some(exp.as_str()) {
                bad.push((sid, lang));
            }
        }
    }
    r.eq(bad, vec![], format!("UI string lookup of all {} dp2m strings (languages 0, 1, 16)", strs.len()));

    for cls in 0..4u32 {
        let cm = b.catalog.films.iter().find(|f| f.paper_cls == cls).map_or(0, |f| f.cm);
        m.w8(stock::NV_CM, cm as u8);
        let exp = &strs[(title - sid0 + 1 + cls) as usize];
        r.eq(m.ui_string(title, 0)?, Some(exp.clone()), format!("paper title for cm {cm} (class {cls})"));
    }
    m.load_nv_defaults();

    let mut diff = Vec::new();
    for sid in 0..sid0 {
        for lang in [0u32, 1] {
            let Ok(exp) = ms.ui_string(sid, lang) else { continue };
            if m.ui_string(sid, lang)? != exp {
                diff.push((sid, lang));
            }
        }
    }
    r.eq(diff, vec![], format!("stock sids 0..{} unchanged (languages 0, 1)", sid0 - 1));
    Ok(())
}

fn check_settings(b: &Build, r: &mut Report) -> Result<HashMap<String, u32>, String> {
    let mut m = Machine::new(&b.img, 0);
    m.load_nv_defaults();
    let mut values: HashMap<u32, BTreeSet<u32>> = HashMap::new();
    for &rows in menu_map(b).values() {
        for row in read_rows_img(&b.img, rows, 128) {
            values.entry(row.get() & !1).or_default().insert(row.value());
        }
    }
    let mut names: Vec<String> = b
        .syms
        .keys()
        .filter_map(|n| n.strip_prefix("nv_get_").filter(|&k| k != "none").map(str::to_owned))
        .collect();
    names.sort();

    let (mut addrs, mut vals) = (HashMap::new(), BTreeMap::new());
    for name in &names {
        let get = b.sym(&format!("nv_get_{name}"));
        let put = b.sym(&format!("nv_set_{name}"));
        let vs: Vec<u32> = values.get(&get).map(|s| s.iter().copied().collect()).unwrap_or_default();
        if !r.check(vs.len() > 1, format!("nv {name}: menu values {vs:?}")) {
            continue;
        }
        let before = m.rd(stock::NV_BLOCK, stock::NV_LEN);
        m.cpu.start_write_log(&[STACK, LV_DIRTY]);
        m.call(put, &[*vs.last().unwrap()])?;
        let log = m.cpu.stop_write_log();
        let after = m.rd(stock::NV_BLOCK, stock::NV_LEN);
        let changed: Vec<u32> = (0..stock::NV_LEN)
            .filter(|&i| before[i] != after[i])
            .map(|i| stock::NV_BLOCK + i as u32)
            .collect();
        let addr = (changed.len() == 1).then(|| changed[0]);
        let ok = addr.is_some_and(|a| a >= NV_RESERVED.0 && a < NV_RESERVED.1 && outside(&log, &[(a, a + 1)]).is_empty());
        r.check(
            ok,
            format!("nv_set_{name} writes only NVRAM {:?} (reserved +0x04..+0x13)", addr.map(|a| format!("{a:#x}"))),
        );
        let Some(a) = addr else { continue };
        let mut rt = Vec::new();
        for &v in &vs {
            m.call(put, &[v])?;
            if m.call(get, &[])? != v {
                rt.push(v);
            }
        }
        let oob = *vs.last().unwrap() + 1;
        m.w8(a, oob as u8);
        r.check(
            rt.is_empty() && m.call(get, &[])? == 0,
            format!("nv {name}: {}..{} round-trip, {oob} reads as 0", vs[0], vs.last().unwrap()),
        );
        addrs.insert(name.clone(), a);
        vals.insert(name.clone(), *vs.last().unwrap());
    }
    let uniq: BTreeSet<u32> = addrs.values().copied().collect();
    r.eq(uniq.len(), addrs.len(), "settings use distinct NVRAM bytes");

    let nor = Nor::install(&mut m);
    for (name, &v) in &vals {
        m.call(b.sym(&format!("nv_set_{name}")), &[v])?;
    }
    m.call(stock::NV_FLUSH, &[])?;
    let (off, n, buf) = nor.writes.borrow().last().cloned().unwrap_or((0, 0, Vec::new()));
    let mut rec = BTreeMap::new();
    if off == 8 {
        for (name, &a) in &addrs {
            rec.insert(name.clone(), buf[0x18 + (a - stock::NV_BLOCK) as usize] as u32);
        }
    }
    r.eq(rec, vals.clone(), "stock flush writes the settings into the NOR record (block at NOR 0x20)");
    {
        let mut d = nor.data.borrow_mut();
        d[0..4].copy_from_slice(&0x2012_0918u32.to_be_bytes());
        d[off as usize..(off + n) as usize].copy_from_slice(&buf);
    }
    m.wr(stock::NV_BLOCK, &vec![0u8; stock::NV_PERSIST]);
    m.call(stock::NV_LOAD, &[stock::NV_BLOCK])?;
    let mut got = BTreeMap::new();
    for name in vals.keys() {
        got.insert(name.clone(), m.call(b.sym(&format!("nv_get_{name}")), &[])?);
    }
    r.eq(got, vals, "settings survive NOR flush + boot load");
    Ok(addrs)
}

fn check_bracketing(b: &Build, r: &mut Report) -> Result<(), String> {
    let mut m = Machine::new(&b.img, 0);
    for idx in [1u16, 3] {
        m.w16(stock::BKT_STEP_IDX, idx);
        let step = b.img.u32(stock::BKT_STEP_TBL + 4 * idx as u32) as i32;
        for (order, ev) in [
            (0u8, &[0i32, 1, -1][..]),
            (4, &[0, 2, 1, -1, -2][..]),
            (6, &[2, 1, 0, -1, -2][..]),
            (7, &[-2, -1, 0, 1, 2][..]),
        ] {
            m.w8(stock::NV_BKT_ORD, order);
            let cnt = m.call(stock::BKT_COUNT, &[])? as usize;
            let mut got_ev = Vec::new();
            for i in 0..ev.len() {
                got_ev.push(m.call(stock::BKT_EV_Q16, &[i as u32])? as i32);
            }
            let exp_ev: Vec<i32> = ev.iter().map(|&e| e * step).collect();
            r.eq((cnt, got_ev), (ev.len(), exp_ev), format!("bracketing order {order} step {:.3} EV", step as f64 / 65536.0));
        }
    }
    Ok(())
}

fn check_min_shutter(b: &Build, r: &mut Report, nv: u32) -> Result<(), String> {
    let mut m = Machine::new(&b.img, 0);
    let tv = &b.catalog.tv_min;
    let (mut a_bad, mut p_bad) = (Vec::new(), Vec::new());
    for idx in [0u8, 1, 2, 3, 4, 5, 6, 7, 0xFF] {
        m.w8(nv, idx);
        m.run_slice(stock::AUTOISO_A, stock::AUTOISO_A + 14, 1_000_000, STACK.1, &[(0, 0), (1, 0x1234_5678)])?;
        let exp_a = tv[if idx <= 6 { idx as usize } else { 0 }] as u32;
        if (m.cpu.r[0], m.cpu.r[1]) != (exp_a, 0x1234_5678) {
            a_bad.push(idx);
        }
        for site in stock::AUTOISO_P {
            m.run_slice(site, site + 4, 1_000_000, STACK.1, &[(0, 0)])?;
            let exp_p = if (1..=6).contains(&idx) {
                (tv[idx as usize] + 0x30000) as u32
            } else {
                0x90000
            };
            if m.cpu.r[0] != exp_p {
                p_bad.push((site, idx));
            }
        }
    }
    r.eq(a_bad, vec![], "ISO Auto min shutter, A mode: table value, idx > 6 -> Auto, r1 kept");
    r.eq(p_bad, vec![], "ISO Auto min shutter, P mode (both sites): table value + 3 EV, Auto -> stock 9.0");
    Ok(())
}

fn check_auto_iso50(b: &Build, r: &mut Report) -> Result<(), String> {
    use std::cell::Cell;

    if !b.syms.contains_key("m_iso_min_1ev") {
        return Ok(());
    }

    const SET_AUTO_ISO_MIN: u32 = 0xA016_F572;
    const ISO_NEXT_1EV_UP: u32 = 0xA003_F30A;
    const UPDATE_D2_ROWS: u32 = 0xA004_B650;
    const UPDATE_D3_ROWS: u32 = 0xA004_CC08;
    const GET_COMMITTED_ISO: u32 = 0xA016_80EE;
    const ISO_IDX_TO_SPEED: u32 = 0xA000_1C0E;
    const ISO_SPEED_TO_M5: u32 = 0xA000_1D04;
    const ISO_SPEED_TO_GAIN: u32 = 0xA001_491C;
    const IPC_ISO_LOAD: u32 = 0xA003_5FE4;

    let mut m = Machine::new(&b.img, 0);
    let mut ms = Machine::new(&b.stock, 0);
    m.load_nv_defaults();
    ms.load_nv_defaults();

    // 1. Lower-limit menu 0xD2 row 0 is ISO 50 (value 1, sid 53, stock ISO 50 icons, id 0x157 untouched by 0xA004B650)
    for (step, name) in [(0u8, "m_iso_min_1ev"), (1u8, "m_iso_min_3ev")] {
        m.w8(stock::NV_ISO_STEP, step);
        let d2_va = m.menu_table(stock::MENU_ISO_MIN)?;
        let rows = read_rows_mach(&m, d2_va, 16);
        let r0 = &rows[0];
        r.check(
            rows.len() == 8
                && r0.id() == 0x0157
                && r0.w[1] == 0x0100_012B
                && r0.w[2] == 0x0070_0175
                && r0.sid() == 53
                && r0.value() == 1,
            format!("{name}: 8 rows, row 0 = ISO 50 (id 0x157, stock icons 0x12b/0x70/0x175, sid 53, v=1)"),
        );
    }

    // 2. Setter/getter round-trip for min = 1 in 1-EV and 1/3-EV modes + dynamic row enable updaters 0xA004B650/0xA004CC08
    for step in [0u8, 1] {
        m.w8(stock::NV_ISO_STEP, step);
        m.w8(stock::NV_AUTO_ISO_MAX, 5); // ISO 200 (lowest valid 0xD3 row)
        m.call(SET_AUTO_ISO_MIN, &[1])?;
        let got_ui = m.call(stock::GET_AUTO_ISO_MIN_UI, &[])?;
        let nv_min = m.u8(stock::NV_AUTO_ISO_MIN);
        let ae_min = m.u8(stock::AE_STATE + 0x20);
        m.call(UPDATE_D2_ROWS, &[])?;
        m.call(UPDATE_D3_ROWS, &[])?;
        let d2_va = m.menu_table(stock::MENU_ISO_MIN)?;
        let d3_va = m.menu_table(0xD3)?;
        let d2 = read_rows_mach(&m, d2_va, 16);
        let d3 = read_rows_mach(&m, d3_va, 32);
        let iso50_en = d2[0].w[1] >> 24 == 1;
        let d3_200_en = d3[0].w[1] >> 24 == 1;
        r.check(
            got_ui == 1 && nv_min == 1 && ae_min == 1 && iso50_en && d3_200_en,
            format!("Auto-ISO min=1 (step {step}): getter=1, NV=1, AE=1, 0xD2[0] & 0xD3[0] enabled"),
        );
    }
    r.eq(
        m.call(ISO_NEXT_1EV_UP, &[1])?,
        5,
        "0xA003F30A(1) -> 5 (ISO 200, lowest valid 0xD3 row)",
    );

    // 3. Native AE solver (ae_solve_core via AE_SOLVE_UI) in A, P and S modes across bright, borderline & dim EVs
    let bv_m = Rc::new(Cell::new(0i32));
    let bv_s = Rc::new(Cell::new(0i32));
    { let bv = bv_m.clone(); m.stub_on(stock::AE_SENSOR_EV100, move |_| Some(bv.get() as u32)); }
    { let bv = bv_s.clone(); ms.stub_on(stock::AE_SENSOR_EV100, move |_| Some(bv.get() as u32)); }
    m.load_nv_defaults();
    ms.load_nv_defaults();
    m.w8(stock::FLASH_STATE, 1);
    ms.w8(stock::FLASH_STATE, 1);
    m.call(stock::AE_ENABLE_LV_METER, &[1, 1])?;
    ms.call(stock::AE_ENABLE_LV_METER, &[1, 1])?;

    let solve = |mach: &mut Machine, bv_cell: &Cell<i32>, ae_mode: u32, step: u8, min_ui: u8, max_ui: u8, bv_q16: i32| -> Result<(i32, u8, u16, u16), String> {
        bv_cell.set(bv_q16);
        mach.load_nv_defaults();
        mach.w8(stock::FLASH_STATE, 1);
        mach.w8(stock::NV_ISO_STEP, step);
        mach.w8(stock::NV_AUTO_ISO_MIN, min_ui);
        mach.w8(stock::NV_AUTO_ISO_MAX, max_ui);
        mach.w8(stock::NV_ISO, 0); // Auto ISO
        mach.w8(0xA093_968B, 0);   // stock min shutter
        let min_ae = mach.call(stock::AE_MAP_ISO_IDX, &[min_ui as u32])?;
        let max_ae = mach.call(stock::AE_MAP_ISO_IDX, &[max_ui as u32])?;
        mach.call(stock::AE_SET_MODE, &[ae_mode])?;
        mach.call(stock::AE_SET_ISO_STEP, &[step as u32])?;
        mach.call(stock::AE_SET_AUTO_ISO_MIN, &[min_ae])?;
        mach.call(stock::AE_SET_AUTO_ISO_MAX, &[max_ae])?;
        mach.call(stock::AE_SYNC_ISO, &[0])?;
        mach.call(stock::AE_SYNC_AV, &[])?;
        mach.call(stock::AE_SYNC_TV, &[])?;
        mach.call(stock::AE_SYNC_EV, &[9])?; // 0.0 EV comp
        mach.call(stock::AE_SOLVE_UI, &[])?;
        let sv = mach.u32(stock::AE_STATE + 0x10) as i32;
        let committed = mach.call(GET_COMMITTED_ISO, &[])? as u8;
        let av = mach.u16(stock::AE_OUT_AV_IDX);
        let tv = mach.u16(stock::AE_OUT_TV_IDX);
        Ok((sv, committed, av, tv))
    };

    // Bright EV (BV 12.0): A (2), P (4), S (3) in both 1/3-EV and 1-EV settle on exact ISO 50; negative control min=2 -> ISO 100
    for (ae_mode, label) in [(2u32, "A"), (4, "P"), (3, "S")] {
        for step in [0u8, 1] {
            let (sv50, idx50, _, _) = solve(&mut m, &bv_m, ae_mode, step, 1, 14, 12 << 16)?;
            let (sv100, idx100, _, _) = solve(&mut m, &bv_m, ae_mode, step, 2, 14, 12 << 16)?;
            r.eq(
                (sv50, idx50, sv100, idx100),
                (0x40000, 1, 0x50000, 2),
                format!("AE {label} mode (step {step}) bright BV 12: min=50 -> Sv 4.0/idx 1; neg control min=100 -> Sv 5.0/idx 2"),
            );
        }
    }

    // Borderline EV in 1/3-EV mode (< 1 EV spare below ISO 100): must jump directly 4.0 -> 5.0 (no ISO 64/80) and match stock
    let mut border_bad = Vec::new();
    for (ae_mode, bv_q16) in [
        (2u32, 0x0009_0000i32), // A @ f/2.8: Tv(50)=5.0 < 5.67, Tv(100)=6.0 >= 5.67
        (2, 0x0009_5555),       // A @ f/2.8: Tv(50)=5.33 < 5.67, Tv(100)=6.33 >= 5.67
        (4, 0x0009_0000),       // P: EV(50)=8.0 < 8.67, EV(100)=9.0 >= 8.67
        (4, 0x0009_5555),       // P: EV(50)=8.33 < 8.67, EV(100)=9.33 >= 8.67
    ] {
        let got = solve(&mut m, &bv_m, ae_mode, 1, 1, 14, bv_q16)?;
        let exp = solve(&mut ms, &bv_s, ae_mode, 1, 2, 14, bv_q16)?;
        if got != exp || got.0 != 0x50000 || got.1 != 2 {
            border_bad.push((ae_mode, bv_q16, got, exp));
        }
    }
    r.eq(border_bad, vec![], "1/3-EV borderline EV (< 1 EV spare): jumps 4.0 -> 5.0 (no ISO 64/80), matches stock");

    // Dim EV (BV 6.0): min=50 produces exact stock Sv/idx/Av/Tv in A and P modes (both 1-EV and 1/3-EV)
    let mut dim_bad = Vec::new();
    for ae_mode in [2u32, 4] {
        for step in [0u8, 1] {
            let got = solve(&mut m, &bv_m, ae_mode, step, 1, 14, 6 << 16)?;
            let exp = solve(&mut ms, &bv_s, ae_mode, step, 2, 14, 6 << 16)?;
            if got != exp || got.0 <= 0x50000 {
                dim_bad.push((ae_mode, step, got, exp));
            }
        }
    }
    r.eq(dim_bad, vec![], "dim BV 6.0 (A & P, 1-EV & 1/3-EV): min=50 matches stock Auto-ISO exactly");

    // Stock-identical check when lower limit >= ISO 100 (min = 2 and 5 across dim/mid/bright EVs)
    let mut stock_bad = Vec::new();
    for ae_mode in [2u32, 4] {
        for step in [0u8, 1] {
            for min_ui in [2u8, 5] {
                for bv_ev in [6i32, 9, 12, 15] {
                    let got = solve(&mut m, &bv_m, ae_mode, step, min_ui, 14, bv_ev << 16)?;
                    let exp = solve(&mut ms, &bv_s, ae_mode, step, min_ui, 14, bv_ev << 16)?;
                    if got != exp {
                        stock_bad.push((ae_mode, step, min_ui, bv_ev, got, exp));
                    }
                }
            }
        }
    }
    r.eq(stock_bad, vec![], "lower limit >= ISO 100: byte-identical to stock across A/P, 1-EV/1/3-EV, BV 6..15");

    // 4. Committed ISO 1 -> capture speed 50 -> IPC cmd 0x20 byte 0x18 (M5 LinLUTs50) & sensor gain 0x01FF
    let (_, idx50, _, _) = solve(&mut m, &bv_m, 2, 1, 1, 14, 12 << 16)?;
    let speed = m.call(ISO_IDX_TO_SPEED, &[idx50 as u32])?;
    let ipc_code = m.call(ISO_SPEED_TO_M5, &[speed])?;
    let gain = m.call(ISO_SPEED_TO_GAIN, &[speed])?;
    // Run the actual M6M capture IPC ISO conversion slice (0xA0035FE4: ldrh r0,[r0,#0x1c]; bl 0xA0001D04)
    m.w16(SCRATCH + 0x1C, speed as u16);
    m.run_slice(IPC_ISO_LOAD, IPC_ISO_LOAD + 6, 100_000, STACK.1, &[(0, SCRATCH)])?;
    let slice_ipc = m.cpu.r[0];
    r.eq(
        (idx50, speed, ipc_code, slice_ipc, gain),
        (1, 50, 0x18, 0x18, 0x01FF),
        "committed ISO 1 -> speed 50 (EXIF/meta) -> IPC 0x18 (M5 LinLUTs50) & sensor gain 0x01FF",
    );
    Ok(())
}

fn check_ipc(b: &Build, r: &mut Report, addrs: &HashMap<String, u32>) -> Result<(), String> {
    let mut m = Machine::new(&b.img, 0);
    m.load_nv_defaults();
    let (cmd, meta) = (SCRATCH + 0x100, SCRATCH + 0x200);
    let mut regs_bad = Vec::new();

    let mut pack = |cm: u32, cs: u32, nv: &HashMap<&str, u32>| -> Result<(u8, u8), String> {
        for (&k, &v) in nv {
            if let Some(&a) = addrs.get(k) {
                m.w8(a, v as u8);
            }
        }
        m.wr(meta + 0x1D, &[cs as u8, cm as u8]);
        m.w32(STACK.1 + 0x14, meta);
        m.wr(cmd, &[0u8; 8]);
        m.run_slice(stock::IPC_CMD20, stock::IPC_CMD20 + 6, 1_000_000, STACK.1, &[(0, 0x55), (1, 0x3F80_0000), (4, cmd)])?;
        if (m.cpu.r[0], m.cpu.r[1]) != (0, 0x3F80_0000) {
            regs_bad.push((cm, cs));
        }
        Ok((m.u8(cmd + 5), m.u8(cmd + 6)))
    };

    let mut zero = HashMap::new();
    for k in ["engine", "grain", "halation", "fill", "color_paper", "bw_paper", "lut_slot"] {
        zero.insert(k, 0u32);
    }

    let mut bad_stock = Vec::new();
    for cm in 0..FILM_MODE0 {
        if pack(cm, 0, &zero)?.0 as u32 != cm {
            bad_stock.push(cm);
        }
    }
    r.eq(bad_stock, vec![], "cmd[5] = stock color modes 0..8 unchanged");

    for cls in 0..4u32 {
        let fs: Vec<&FilmEntry> = b.catalog.films.iter().filter(|f| f.paper_cls == cls).collect();
        let papers: Vec<u32> = if cls != 0 { (0..10).collect() } else { vec![0, 3] };
        let mut bad = Vec::new();
        for f in &fs {
            for &p in &papers {
                let mut nv = zero.clone();
                if cls != 0 {
                    nv.insert(paper_nv(cls), p);
                }
                if pack(f.cm, 0, &nv)?.0 as u32 != packed_of(f, &nv) {
                    bad.push((f.cm, p));
                }
            }
        }
        r.eq(
            bad,
            vec![],
            format!("cmd[5] packed mode, paper class {cls}: {} films x papers {}..{}", fs.len(), papers[0], papers.last().unwrap()),
        );
    }

    let f0 = &b.catalog.films[0];
    let mut bad_p1d = Vec::new();
    for e in 0..2u32 {
        for g in 0..2u32 {
            for h in 0..2u32 {
                for fill in 0..4u32 {
                    for cs in 0..4u32 {
                        let mut nv = zero.clone();
                        nv.insert("engine", e);
                        nv.insert("grain", g);
                        nv.insert("halation", h);
                        nv.insert("fill", fill);
                        let got = pack(f0.cm, cs, &nv)?.1 as u32;
                        let exp = cs | ((e | (g << 1) | (h << 2)) << 2) | (fill << 5);
                        if got != exp {
                            bad_p1d.push((e, g, h, fill, cs, got));
                        }
                    }
                }
            }
        }
    }
    r.eq(bad_p1d, vec![], "cmd[6] = colorspace | (3d | grain<<1 | halation<<2) << 2 | fill << 5 (128 combos)");
    r.eq(regs_bad, vec![], "IPC hook keeps live r1 (1.0f) and returns r0 = 0");
    Ok(())
}

fn check_makernote(b: &Build, r: &mut Report) -> Result<(), String> {
    let mut m = Machine::new(&b.img, 0);
    let (sp, rec, txt) = (STACK.1 - 0x200, SCRATCH, SCRATCH + 0x100);
    let mut cms: Vec<u32> = vec![0, 1, 2, 3, 4];
    cms.extend(b.catalog.films.iter().map(|f| f.cm));
    let mut bad = Vec::new();
    for cm in cms {
        m.w8(rec + 0x8E, cm as u8);
        m.run_slice(MN_SWITCH_SETUP, MN_SWITCH_DONE, 1_000_000, sp, &[(7, rec + 0x80)])?;
        let tag3d = m.rd(MN_TAG_3D, 11);
        let tag59 = m.rd(MN_TAG_59, 15);
        let ok = if cm < FILM_MODE0 {
            let exp: &[u8] = match cm {
                1 => b"Sepia",
                2 => b"Monochrome",
                4 => b"Vivid",
                _ => b"Standard",
            };
            tag3d.starts_with(exp) && tag59.starts_with(exp)
        } else {
            let mut exp3d = format!("Film {:02}", cm - FILM_MODE0).into_bytes();
            exp3d.resize(11, 0);
            let s = b.catalog.strings[(cm - FILM_MODE0) as usize].as_bytes();
            let mut exp59 = s[..s.len().min(14)].to_vec();
            exp59.resize(15, 0);
            tag3d == exp3d && tag59 == exp59
        };
        if !ok {
            bad.push(cm);
            continue;
        }
        for (site, until, rs, rr) in [
            (0xA014_7938u32, 0xA014_793Cu32, 7usize, 4usize),
            (0xA013_2E9A, 0xA013_2E9E, 6, 5),
        ] {
            let mut z = tag3d.clone();
            z.push(0);
            m.wr(txt, &z);
            m.w8(rec + 0x17, 0xEE);
            m.run_slice(site, until, 1_000_000, sp, &[(0, 0), (rs, txt), (rr, rec)])?;
            let exp = if cm >= FILM_MODE0 { cm as u8 } else { 0 };
            if m.u8(rec + 0x17) != exp {
                bad.push(cm);
            }
        }
    }
    r.eq(
        bad,
        vec![],
        format!("maker note: stock modes keep their names, {} films write 'Film NN' + name and parse back in JPG & X3F playback", b.catalog.films.len()),
    );
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LvCall {
    Gamma(u32, Vec<u8>),
    Mono(u32, u32),
    Cmcm(u32, u32),
    Mat([u16; 14]),
}

fn check_lv(b: &Build, r: &mut Report) -> Result<(), String> {
    for (site, name) in [
        (stock::LV_GAMMA_SITE, "lv_gamma_hook"),
        (stock::LV_MONO_SITE, "lv_mono_hook"),
        (stock::LV_CMCM_SITE, "lv_cmcm_hook"),
    ] {
        r.eq(b.img.bl_target(site), Some(b.sym(name)), format!("BL @ {site:#x} -> {name}"));
    }
    let mut m = Machine::new(&b.img, 0);
    m.load_nv_defaults();
    let calls: Rc<RefCell<Vec<LvCall>>> = Rc::new(RefCell::new(Vec::new()));

    {
        let c = calls.clone();
        m.stub_on(b.sym("stock_b2y_set_gamma"), move |cpu| {
            let (p, n) = (cpu.r[1], cpu.r[2] as usize);
            let mut buf = vec![0u8; n];
            cpu.read_bytes(p, &mut buf);
            c.borrow_mut().push(LvCall::Gamma(p, buf));
            None
        });
    }
    {
        let c = calls.clone();
        m.stub_on(b.sym("stock_lv_mono"), move |cpu| {
            c.borrow_mut().push(LvCall::Mono(cpu.r[0], cpu.r[1]));
            None
        });
    }
    {
        let c = calls.clone();
        m.stub_on(b.sym("stock_lv_cmcm"), move |cpu| {
            c.borrow_mut().push(LvCall::Cmcm(cpu.r[0], cpu.r[1]));
            None
        });
    }
    {
        let c = calls.clone();
        m.stub_on(b.sym("stock_b2y_set_mat"), move |cpu| {
            let p = cpu.r[0];
            let mat = core::array::from_fn(|k| cpu.r16_be(p + (k * 2) as u32));
            c.borrow_mut().push(LvCall::Mat(mat));
            None
        });
    }
    let cc7 = [96i32, -20, 5, -30, 120, 8, 2, -15, 110];
    let ccn = [130i32, -40, 10, -25, 140, -12, 0, -30, 150];
    for (cc, getter) in [(cc7, "stock_get_lv_cc7"), (ccn, "stock_get_lv_ccn")] {
        let mut raw = vec![0u8; 36];
        for (k, &v) in cc.iter().enumerate() {
            let w = ((v as u16) & 0x0FFF) | 0xA000;
            raw[10 + k * 2..12 + k * 2].copy_from_slice(&w.to_be_bytes());
        }
        m.stub_on(b.sym(getter), move |cpu| {
            cpu.write_bytes(cpu.r[0], &raw);
            None
        });
    }

    let empty = HashMap::new();
    let look = |f: &FilmEntry| &b.catalog.looks[(packed_of(f, &empty) - FILM_MODE0) as usize];
    let call_lv = |m: &mut Machine, fn_name: &str, args: &[u32]| -> Result<Vec<LvCall>, String> {
        calls.borrow_mut().clear();
        m.call(b.sym(fn_name), args)?;
        Ok(calls.borrow().clone())
    };

    let lut = SCRATCH + 0x1000;
    let stock_lut: Vec<u8> = (0..1024).map(|i| (i & 0xFF) as u8).collect();
    m.wr(lut, &stock_lut);
    let gbuf = b.sym("gamma_buf");
    let kf = b.catalog.films.iter().find(|f| look(f).flags & LV_KNOTS != 0).unwrap();
    let curve = expand_knots(&look(kf).knots);

    // Dirty-.bss boot regression: before lv_gamma_hook runs, nonzero garbage in payload .bss
    // (including lut = 0xDEADBEEF) must NOT fool lv_mono_hook into dereferencing gamma.lut,
    // and labels_boot must clear stale GAMMA_MAGIC retained across warm power cycles.
    {
        let gamma_sym = b.sym_info("gamma");
        m.wr(gamma_sym.va(), &vec![0xFFu8; gamma_sym.size as usize]);
        m.w32(gamma_sym.va() + 4, 0xDEAD_BEEF);
        m.w8(stock::NV_CM, 0);
        r.eq(
            call_lv(&mut m, "lv_mono_hook", &[1, 0])?,
            vec![LvCall::Mono(1, 0)],
            "dirty .bss (0xFF) before lv_gamma_hook: lv_mono_hook compares full 32-bit magic and skips garbage lut",
        );
        m.w32(gamma_sym.va(), 0x474D_4131);
        m.w32(gamma_sym.va() + 4, lut);
        m.w32(gamma_sym.va() + 8, 0x400);
        m.stub_on(b.sym("stock_disp_init"), |_| None);
        m.call(b.sym("labels_boot"), &[])?;
        r.eq(
            m.u32(gamma_sym.va()),
            0,
            "labels_boot resets gamma.magic across warm reboots",
        );
    }

    m.w8(stock::NV_CM, 0);
    r.eq(call_lv(&mut m, "lv_gamma_hook", &[0, lut, 0x400])?, vec![LvCall::Gamma(lut, stock_lut.clone())], "gamma hook, stock color mode: stock LUT");
    m.w8(stock::NV_CM, kf.cm as u8);
    r.eq(call_lv(&mut m, "lv_gamma_hook", &[0, lut, 0x400])?, vec![LvCall::Gamma(gbuf, curve.clone())], format!("gamma hook, film cm {}: expanded film knots", kf.cm));
    m.w8(stock::NV_CM, 0);
    call_lv(&mut m, "lv_gamma_hook", &[0, lut, 0x400])?;
    let mono_cm = if kf.mono != 0 { 2 } else { 3 };
    r.eq(
        call_lv(&mut m, "lv_mono_hook", &[1, kf.cm])?,
        vec![LvCall::Mono(1, mono_cm), LvCall::Gamma(gbuf, curve)],
        format!("color mode 0 -> {}: mono routing + gamma re-applied", kf.cm),
    );
    r.eq(call_lv(&mut m, "lv_mono_hook", &[1, kf.cm])?, vec![LvCall::Mono(1, mono_cm)], "same color mode: gamma not re-applied");
    r.eq(
        call_lv(&mut m, "lv_mono_hook", &[1, 0])?,
        vec![LvCall::Mono(1, 0), LvCall::Gamma(lut, stock_lut)],
        format!("color mode {} -> 0: stock gamma restored", kf.cm),
    );
    r.eq(call_lv(&mut m, "lv_cmcm_hook", &[1, 0])?, vec![LvCall::Cmcm(1, 0)], "CMCM hook, stock color mode: stock path");
    let cf = b.catalog.films.iter().find(|f| f.cm == 13).unwrap();
    let official_teal = [1092, 59, -127, 257, 392, 375, -17, -43, 1084];
    r.eq(look(cf).cmcm, official_teal, "Teal & Orange LV uses official DP2Q CMCM entry 10 in Q10");
    r.eq(look(cf).flags, LV_CMCM, "Teal & Orange LV enables CMCM without mono weights or gamma knots");
    for (mode, cc) in [(7u32, &cc7), (1, &ccn)] {
        let exp = lv_matrix(&look(cf).cmcm, look(cf).flags, cc);
        r.eq(call_lv(&mut m, "lv_cmcm_hook", &[mode, cf.cm])?, vec![LvCall::Mat(exp)], format!("CMCM hook, film cm {}, LV mode {mode}: q10 film x live q7", cf.cm));
    }
    if let Some(mf) = b.catalog.films.iter().find(|f| look(f).flags & LV_MONO_Q7 != 0) {
        let exp = lv_matrix(&look(mf).cmcm, look(mf).flags, &ccn);
        r.eq(call_lv(&mut m, "lv_cmcm_hook", &[1, mf.cm])?, vec![LvCall::Mat(exp)], format!("CMCM hook, mono film cm {}: direct q7 weights", mf.cm));
    }
    Ok(())
}

fn check_ycap(b: &Build, r: &mut Report) -> Result<(), String> {
    let mut m = Machine::new(&b.img, 0);
    let lp = b.sym_info("lv_planes");
    r.check(
        !(stock::NV_BLOCK..stock::NV_BLOCK + stock::NV_LEN as u32).contains(&lp.va()),
        format!("lv_planes @ {:#x} outside the NVRAM block", lp.va()),
    );
    let site = stock::LV_YCAP + 2;
    let lit = b.stock.u32(((site + 4) & !3) + ((b.stock.u16(site) & 0xFF) as u32) * 4);
    let planes = [0xA230_0000u32, 0xA236_0000, 0xA239_0000];
    let mut pbuf = Vec::with_capacity(12);
    for p in planes {
        pbuf.extend_from_slice(&p.to_be_bytes());
    }
    m.wr(SCRATCH, &pbuf);
    let f0 = m.u32(lp.va() + 12);
    m.cpu.start_write_log(&[STACK]);
    for _ in 0..3 {
        m.run_slice(stock::LV_YCAP, stock::LV_YCAP + 6, 1_000_000, STACK.1, &[(0, SCRATCH)])?;
    }
    let log = m.cpu.stop_write_log();
    r.eq(
        (m.cpu.r[5], m.cpu.r[7], m.rd(lp.va(), 12), m.u32(lp.va() + 12)),
        (lit, SCRATCH, pbuf, f0 + 3),
        "LV Y-capture hook: planes + frame count, stock r5/r7",
    );
    r.eq(outside(&log, &[(lp.va(), lp.va() + lp.size)]), vec![], "LV Y-capture hook writes only lv_planes (never NVRAM)");
    Ok(())
}

fn check_hud(b: &Build, r: &mut Report, peakcol_addr: u32, peaksens_addr: u32, zebralvl_addr: u32) -> Result<(), String> {
    let (wget, gdraw) = (b.sym("grid_widget_get"), b.sym("grid_draw_hook"));
    r.eq(b.img.u32(0xA07F_A4A4), wget | 1, "LV widget 0x14 getter -> grid_widget_get");
    r.check(
        b.img.bl_target(0xA016_C708) == Some(gdraw) && b.img.bl_target(0xA004_C5AE) == Some(gdraw),
        "both stock_grid_draw call sites -> grid_draw_hook",
    );

    let mut m = Machine::new(&b.img, 0);
    m.load_nv_defaults();
    let st = Rc::new(RefCell::new((0x15u32, 1u32, 0i8, 64i8, Vec::<u32>::new())));
    { let s = st.clone(); m.stub_on(b.sym("stock_ui_screen_id"), move |_| Some(s.borrow().0)); }
    { let s = st.clone(); m.stub_on(b.sym("stock_lv_active"), move |_| Some(s.borrow().1)); }
    { let s = st.clone(); m.stub_on(b.sym("stock_accel_x"), move |_| Some(s.borrow().2 as u8 as u32)); }
    { let s = st.clone(); m.stub_on(b.sym("stock_accel_y"), move |_| Some(s.borrow().3 as u8 as u32)); }
    { let s = st.clone(); m.stub_on(b.sym("stock_grid_draw"), move |c| { s.borrow_mut().4.push(c.r[0]); None }); }

    m.w8(stock::NV_GRID, 3);
    r.eq(m.call(wget, &[])?, 3, "grid_widget_get passes stock grid 3 through");
    m.call(gdraw, &[0x1234])?;
    r.eq(st.borrow().4.clone(), vec![0x1234], "grid_draw_hook delegates stock grids 0..6 to stock_grid_draw");
    m.call(gdraw, &[0x5678])?;
    r.eq(st.borrow().4.clone(), vec![0x1234, 0x5678], "grid_draw_hook steady-state stock grid fast path still delegates to stock_grid_draw");

    m.w8(stock::NV_GRID, 7);
    for (scr, lv) in [(0x1Bu32, 1u32), (0x15, 0)] {
        { let mut s = st.borrow_mut(); s.0 = scr; s.1 = lv; }
        m.cpu.start_write_log(&[STACK]);
        let k = m.call(wget, &[])?;
        m.call(gdraw, &[SCRATCH])?;
        let log = m.cpu.stop_write_log();
        r.check(k == 7 && log.is_empty(), format!("HUD inactive when screen={scr:#x} lv_active={lv}: key=m, zero ARGB writes"));
    }
    { let mut s = st.borrow_mut(); s.0 = 0x15; s.1 = 1; }

    let lp = b.sym("lv_planes");
    let k0 = m.call(wget, &[])?;
    st.borrow_mut().2 = 1;
    let k_dead = m.call(wget, &[])?;
    st.borrow_mut().2 = 20;
    let k_tilt = m.call(wget, &[])?;
    { let mut s = st.borrow_mut(); s.2 = 64; s.3 = 0; }
    let k_port = m.call(wget, &[])?;
    r.check(k0 == k_dead && k_tilt != k0 && ((k_port >> 8) & 1) == 1, "grid_widget_get tilt: deadband |slope|<=1, landscape vs portrait");
    { let mut s = st.borrow_mut(); s.2 = 0; s.3 = 64; }

    let mut fkeys = Vec::new();
    let mut fexp = Vec::new();
    for mode in [11u32, 12, 13, 14, 17] {
        m.w8(stock::NV_GRID, mode as u8);
        for fc in [4u32, 5, 6] {
            m.w32(lp + 12, fc);
            fkeys.push(m.call(wget, &[])?);
            fexp.push((mode << 16) | (fc >> 1));
        }
    }
    r.eq(fkeys, fexp, "grid_widget_get modes 11..14 & 17 key on fcnt >> 1");

    m.wr(lp, &[0u8; 16]);
    m.w8(stock::NV_GRID, 11);
    m.cpu.start_write_log(&[STACK]);
    m.call(gdraw, &[SCRATCH])?;
    let log = m.cpu.stop_write_log();
    r.check(
        outside(&log, &[(gdraw, END)]).len() <= 1 && log.iter().all(|&(a, _)| a != stock::ARGB_FB),
        "grid_draw_hook skips peaking/zebras when lv_planes.y is unset",
    );

    let (yv, cbv, crv) = (0xA240_0000u32, 0xA245_0000u32, 0xA248_0000u32);
    let mut y_plane = vec![80u8; 640 * 424];
    for y in 0..424usize {
        let row = y * 640;
        y_plane[row + 160..row + 200].fill(200);
        if (40..80).contains(&y) {
            y_plane[row + 40..row + 80].fill(252);
        }
    }
    m.wr(yv, &y_plane);
    m.wr(cbv, &vec![110u8; 320 * 424]);
    m.wr(crv, &vec![160u8; 320 * 424]);
    let mut lp_init = Vec::with_capacity(16);
    for v in [yv, cbv, crv, 4] {
        lp_init.extend_from_slice(&v.to_be_bytes());
    }
    m.wr(lp, &lp_init);

    let px = |m: &Machine, x: usize, y: usize| m.u16(stock::ARGB_FB + ((y * 640 + x) * 2) as u32);
    let draw = |m: &mut Machine, mode: u8| -> Result<(), String> {
        m.w8(stock::NV_GRID, mode);
        m.call(gdraw, &[SCRATCH])?;
        Ok(())
    };

    { let mut s = st.borrow_mut(); s.2 = 0; s.3 = 64; }
    draw(&mut m, 7)?;
    r.check(px(&m, 212, 211) == 0x1F3E && px(&m, 319, 211) == 0x1F3E && px(&m, 10, 10) == 0, "grid 7 level: green horizon & center tick");
    st.borrow_mut().2 = 20;
    draw(&mut m, 7)?;
    r.eq(px(&m, 212, 211), 0xFFFE, "grid 7 tilted: white horizon");
    { let mut s = st.borrow_mut(); s.2 = 64; s.3 = 0; }
    draw(&mut m, 7)?;
    r.eq(px(&m, 319, 112), 0x1F3E, "grid 7 portrait: vertical end-cap");
    { let mut s = st.borrow_mut(); s.2 = 0; s.3 = 64; }

    for (mode, vert, alpha, s) in [(8u8, false, 10u16, 74usize), (9, false, 10, 31), (10, true, 10, 107), (15, true, 9, 70), (16, true, 9, 37)] {
        draw(&mut m, mode)?;
        let ok = if vert {
            px(&m, s - 1, 10) == alpha && px(&m, s, 10) == 0xFFFC && px(&m, 640 - s - 1, 10) == 0xFFFC && px(&m, s + 4, 10) == 0
        } else {
            px(&m, 10, s - 1) == alpha && px(&m, 10, s) == 0xFFFC && px(&m, 10, 424 - s - 1) == 0xFFFC && px(&m, 10, s + 4) == 0
        };
        r.check(ok, format!("grid {mode} aspect mask ({} {s}px): matte, 1px white guide, clean interior", if vert { "vert" } else { "horiz" }));
    }

    m.w16(stock::ARGB_FB, 0xDEAD);
    m.w16(stock::ARGB_FB + ((423 * 640 + 10) * 2) as u32, 0xDEAD);
    let mut got_cols = Vec::new();
    for c in 0..4u8 {
        m.w8(peakcol_addr, c);
        draw(&mut m, 11)?;
        got_cols.push(px(&m, 160, 120));
    }
    r.eq(got_cols, vec![0xF11E, 0xFE1E, 0xFFFE, 0x111E], "grid 11 focus peaking: edge detected across all 4 peaking colors");
    r.check(px(&m, 0, 0) == 0 && px(&m, 10, 423) == 0 && px(&m, 300, 120) == 0, "grid 11 focus peaking: 2px border clear & flat background = 0");
    m.w8(peakcol_addr, 0);
    // Runtime peaking sensitivity: High (24) detects soft 30-step edge, Med (44) skips it, Low (72) skips 56-step edge, Auto boosts on MF_SPOT
    let mut soft_y = vec![80u8; 640 * 424];
    for y in 0..424usize {
        let row = y * 640;
        soft_y[row + 160..row + 200].fill(110); // step = 30
        soft_y[row + 320..row + 360].fill(136); // step = 56
    }
    m.wr(yv, &soft_y);
    m.w8(0xA093_9A53, 0); // MF_SPOT_ACTIVE = 0
    m.w8(peaksens_addr, 0); // Med (44)
    draw(&mut m, 11)?;
    let med_ok = px(&m, 160, 120) == 0 && px(&m, 320, 120) == 0xF11E;
    m.w8(peaksens_addr, 1); // High (24)
    draw(&mut m, 11)?;
    let high_ok = px(&m, 160, 120) == 0xF11E && px(&m, 320, 120) == 0xF11E;
    m.w8(peaksens_addr, 2); // Low (72)
    draw(&mut m, 11)?;
    let low_ok = px(&m, 160, 120) == 0 && px(&m, 320, 120) == 0;
    m.w8(peaksens_addr, 3); // Auto (MF High)
    m.w8(0xA093_9A53, 0);
    draw(&mut m, 11)?;
    let auto_1x = px(&m, 160, 120) == 0 && px(&m, 320, 120) == 0xF11E;
    m.w8(0xA093_9A53, 1); // MF magnification engaged
    draw(&mut m, 11)?;
    let auto_mf = px(&m, 160, 120) == 0xF11E && px(&m, 320, 120) == 0xF11E;
    m.w8(0xA093_9A53, 0);
    m.w8(peaksens_addr, 0);
    r.check(
        med_ok && high_ok && low_ok && auto_1x && auto_mf,
        "grid 11 peaking sensitivity: Med(44)/High(24)/Low(72) & Auto MF-magnify boost",
    );
    // Legacy scalar full-frame peaking oracle (note: `borders` below indexes +1/+5, whereas
    // big-endian `draw_peaking_arm` samples even offsets 0, 2, 4, 6; the three frames below
    // agree either way, and `peak_even_scalar` below verifies the exact even-offset borders).
    let peak_expected = |y: &[u8], on: u16, thresh: u32| {
        let mut out = vec![0u8; 640 * 424 * 2];
        let mut paint = |row: usize, word: usize| {
            for rr in [row, row + 1] {
                for x in [2 * word, 2 * word + 1] {
                    let o = (rr * 640 + x) * 2;
                    out[o..o + 2].copy_from_slice(&on.to_be_bytes());
                }
            }
        };
        let hit = |gx: i32, gy: i32| (gx.abs() + gy.abs()) as u32 >= thresh;
        for s in 0..105usize {
            let (row0, row2, row4, row6) = (4 * s * 640, (4 * s + 2) * 640, (4 * s + 4) * 640, (4 * s + 6) * 640);
            let borders = [
                (2 + 4 * s, 1, i32::from(y[row2 + 1]) - i32::from(y[row2 + 5]), i32::from(y[row0 + 2]) - i32::from(y[row4 + 2])),
                (2 + 4 * s, 2, i32::from(y[row2 + 2]) - i32::from(y[row2 + 6]), i32::from(y[row0 + 5]) - i32::from(y[row4 + 5])),
                (4 + 4 * s, 1, i32::from(y[row4 + 1]) - i32::from(y[row4 + 5]), i32::from(y[row2 + 2]) - i32::from(y[row6 + 2])),
                (4 + 4 * s, 2, i32::from(y[row4 + 2]) - i32::from(y[row4 + 6]), i32::from(y[row2 + 5]) - i32::from(y[row6 + 5])),
            ];
            for (row, word, gx, gy) in borders {
                if hit(gx, gy) {
                    paint(row, word);
                }
            }
            for i in 0..79usize {
                let base = 4 * s * 640 + 8 + 8 * i;
                for k in 0..4usize {
                    let word = 3 + 4 * i + k;
                    let gx_a = i32::from(y[base - 4 + 2 * k + 1280]) - i32::from(y[base + 2 * k + 1280]);
                    let gy_a = i32::from(y[base - 2 + 2 * k]) - i32::from(y[base - 2 + 2 * k + 2560]);
                    if hit(gx_a, gy_a) {
                        paint(2 + 4 * s, word);
                    }
                    let gx_b = i32::from(y[base - 4 + 2 * k + 2560]) - i32::from(y[base + 2 * k + 2560]);
                    let gy_b = i32::from(y[base - 2 + 2 * k + 1280]) - i32::from(y[base - 2 + 2 * k + 3840]);
                    if hit(gx_b, gy_b) {
                        paint(4 + 4 * s, word);
                    }
                }
            }
        }
        out
    };
    let dense_y: Vec<u8> = (0..424)
        .flat_map(|y| (0..640).map(move |x| {
            let mut n = (x as u32).wrapping_mul(0x045D_9F3B) ^ (y as u32).wrapping_mul(0x27D4_EB2D);
            n ^= n >> 16;
            n as u8
        }))
        .collect();
    let boundary_y = |th: u8| {
        let mut y = vec![80u8; 640 * 424];
        for row in y.as_chunks_mut::<640>().0 {
            row[160..200].fill(80 + th - 1);
            row[320..360].fill(80 + th);
        }
        y
    };
    let boundary24_y = boundary_y(24);
    let boundary72_y = boundary_y(72);
    let peak_cases: [(&str, &Vec<u8>, u32); 3] = [
        ("dense PRNG", &dense_y, 44),
        ("threshold 24 boundaries", &boundary24_y, 24),
        ("threshold 72 boundaries", &boundary72_y, 72),
    ];
    for (name, frame_y, thresh) in peak_cases {
        m.wr(yv, frame_y);
        m.cpu.fill_bytes(stock::ARGB_FB, 0, 640 * 424 * 2);
        m.cpu.call(b.sym("draw_peaking_arm"), &[yv, 0xF11E_F11E, thresh], &[], STACK.1, 100_000_000)?;
        let got = m.rd(stock::ARGB_FB, 640 * 424 * 2);
        let expected = peak_expected(frame_y, 0xF11E, thresh);
        let diff = got.iter().zip(&expected).position(|(g, e)| g != e);
        r.check(
            got == expected,
            format!("Peaking {name} full-frame scalar reference; first diff {diff:?}, got {:?}, expected {:?}", diff.map(|i| got[i]), diff.map(|i| expected[i])),
        );
    }

    // Permanent peaking prefilter superset check over adversarial inputs (dense PRNG,
    // impulses at even/odd/border/block-boundary coords, border-only edges, and SWAR bias/borrow
    // threshold boundaries across runtime thresholds 24, 44, 72).
    {
        use crate::arm9::HookAction;
        let peak_even_scalar = |y: &[u8], on: u16, thresh: u32| -> (Vec<u8>, BTreeSet<(usize, usize)>) {
            let mut out = vec![0u8; 640 * 424 * 2];
            let mut hit_blocks = BTreeSet::new();
            let mut paint = |row: usize, word: usize| {
                for rr in [row, row + 1] {
                    for x in [2 * word, 2 * word + 1] {
                        let o = (rr * 640 + x) * 2;
                        out[o..o + 2].copy_from_slice(&on.to_be_bytes());
                    }
                }
            };
            let hit = |gx: i32, gy: i32| (gx.abs() + gy.abs()) as u32 >= thresh;
            for s in 0..105usize {
                let (row0, row2, row4, row6) = (4 * s * 640, (4 * s + 2) * 640, (4 * s + 4) * 640, (4 * s + 6) * 640);
                // Big-endian (rX[0..1] >> 8) & 0x00FF00FF samples even offsets 0, 2, 4, 6.
                let borders = [
                    (2 + 4 * s, 1, i32::from(y[row2]) - i32::from(y[row2 + 4]), i32::from(y[row0 + 2]) - i32::from(y[row4 + 2])),
                    (2 + 4 * s, 2, i32::from(y[row2 + 2]) - i32::from(y[row2 + 6]), i32::from(y[row0 + 4]) - i32::from(y[row4 + 4])),
                    (4 + 4 * s, 1, i32::from(y[row4]) - i32::from(y[row4 + 4]), i32::from(y[row2 + 2]) - i32::from(y[row6 + 2])),
                    (4 + 4 * s, 2, i32::from(y[row4 + 2]) - i32::from(y[row4 + 6]), i32::from(y[row2 + 4]) - i32::from(y[row6 + 4])),
                ];
                for (row, word, gx, gy) in borders {
                    if hit(gx, gy) {
                        paint(row, word);
                    }
                }
                for i in 0..79usize {
                    let base = 4 * s * 640 + 8 + 8 * i;
                    let mut blk_hit = false;
                    for k in 0..4usize {
                        let word = 3 + 4 * i + k;
                        let gx_a = i32::from(y[base - 4 + 2 * k + 1280]) - i32::from(y[base + 2 * k + 1280]);
                        let gy_a = i32::from(y[base - 2 + 2 * k]) - i32::from(y[base - 2 + 2 * k + 2560]);
                        if hit(gx_a, gy_a) {
                            paint(2 + 4 * s, word);
                            blk_hit = true;
                        }
                        let gx_b = i32::from(y[base - 4 + 2 * k + 2560]) - i32::from(y[base + 2 * k + 2560]);
                        let gy_b = i32::from(y[base - 2 + 2 * k + 1280]) - i32::from(y[base - 2 + 2 * k + 3840]);
                        if hit(gx_b, gy_b) {
                            paint(4 + 4 * s, word);
                            blk_hit = true;
                        }
                    }
                    if blk_hit {
                        hit_blocks.insert((s, i));
                    }
                }
            }
            (out, hit_blocks)
        };

        let mut impulse_y = vec![100u8; 640 * 424];
        for &(x, y) in &[
            (0usize, 2usize), (1, 2), (2, 2), (3, 2), (4, 2), (5, 2), (6, 2),
            (8, 6), (9, 6), (14, 10), (15, 10), (16, 14), (634, 18), (636, 22), (638, 26),
            (320, 0), (320, 420), (320, 422), (100, 100), (101, 100),
        ] {
            impulse_y[y * 640 + x] = 250;
        }
        let mut border_y = vec![80u8; 640 * 424];
        for y in 0..424usize {
            border_y[y * 640] = 200;
            border_y[y * 640 + 4] = 20;
            border_y[y * 640 + 638] = 250;
        }
        let mut adv_frames: Vec<(&str, Vec<u8>, u32)> = vec![
            ("dense PRNG t=24", dense_y.clone(), 24),
            ("dense PRNG t=44", dense_y.clone(), 44),
            ("dense PRNG t=72", dense_y.clone(), 72),
            ("impulses t=24", impulse_y.clone(), 24),
            ("impulses t=72", impulse_y, 72),
            ("borders t=24", border_y.clone(), 24),
            ("borders t=72", border_y, 72),
        ];
        for (thresh, bias) in [(24u32, 8i32), (44, 16), (72, 32)] {
            let mut bnd = vec![128u8; 640 * 424];
            let th = thresh as i32;
            let half = th / 2;
            let rem = th - half;
            // Row-pair s=2 (rows 8,10,12,14): exact threshold & SWAR borrow-across-lane boundaries
            let r0 = 8 * 640;
            let r1 = 10 * 640;
            let r2 = 12 * 640;
            let r3 = 14 * 640;
            // Block i=1 (base = 16): horizontal -(th-1), -th, +th
            bnd[r1 + 16] = (128 - (th - 1)) as u8;
            bnd[r1 + 24] = (128 - th) as u8;
            bnd[r1 + 32] = (128 + th) as u8;
            // Block i=5 (base = 48): split |gx|=half, |gy|=rem (and rem-1)
            bnd[r1 + 48] = (128 - half) as u8;
            bnd[r0 + 46] = (128 + rem) as u8;
            bnd[r1 + 56] = (128 - half) as u8;
            bnd[r0 + 54] = (128 + rem - 1) as u8;
            // Block i=9 (base = 80): negative low-lane borrow -(bias+1) with canceling high lane
            bnd[r1 + 78] = (128 - (bias + 1)) as u8;
            bnd[r0 + 78] = (128 + (th - (bias + 1))) as u8;
            // Cross-block dv0a/dv0b negative borrow at base-2 (x = 94 for block i=11, base=96)
            bnd[r0 + 94] = (128 - th) as u8;
            bnd[r3 + 102] = (128 + th) as u8;
            // Extremes 0 vs 255 in adjacent lanes
            bnd[r2 + 120] = 0;
            bnd[r2 + 124] = 255;
            adv_frames.push((
                if thresh == 24 { "swar-bounds t=24" } else if thresh == 44 { "swar-bounds t=44" } else { "swar-bounds t=72" },
                bnd,
                thresh,
            ));
        }
        let mut mp = Machine::new(&b.img, 0);
        let called: Rc<RefCell<BTreeSet<(usize, usize)>>> = Rc::new(RefCell::new(BTreeSet::new()));
        {
            let called_c = called.clone();
            mp.cpu.add_hook(b.sym("peaking_pair_slow"), move |cpu| {
                let off = cpu.r[0].wrapping_sub(yv) as usize;
                let row = off / 640;
                let col = off % 640;
                if row.is_multiple_of(4) && col >= 8 && (col - 8).is_multiple_of(8) {
                    called_c.borrow_mut().insert((row / 4, (col - 8) / 8));
                }
                HookAction::Continue
            });
        }
        let mut superset_bad: Vec<String> = Vec::new();
        for (name, frame_y, thresh) in &adv_frames {
            called.borrow_mut().clear();
            mp.wr(yv, frame_y);
            mp.cpu.fill_bytes(stock::ARGB_FB, 0, 640 * 424 * 2);
            mp.cpu.call(b.sym("draw_peaking_arm"), &[yv, 0xF11E_F11E, *thresh], &[], STACK.1, 100_000_000)?;
            let got = mp.rd(stock::ARGB_FB, 640 * 424 * 2);
            let (expected, hit_blocks) = peak_even_scalar(frame_y, 0xF11E, *thresh);
            let missed: Vec<_> = hit_blocks.difference(&called.borrow()).copied().collect();
            if !missed.is_empty() {
                superset_bad.push(format!("{name}: prefilter suppressed {} blocks (first {:?})", missed.len(), missed[0]));
            }
            if got != expected {
                let diff = got.iter().zip(&expected).position(|(g, e)| g != e);
                superset_bad.push(format!("{name}: output mismatch at {diff:?}"));
            }
        }
        r.eq(
            superset_bad,
            Vec::<String>::new(),
            format!("Peaking prefilter superset of writer hits & even-parity oracle across {} adversarial frames (t=24/44/72)", adv_frames.len()),
        );
    }
    m.wr(yv, &y_plane);

    draw(&mut m, 12)?;
    r.check(px(&m, 160, 120) == 0xF11E && px(&m, 319, 211) == 0x1F3E, "grid 12 peaking + level: combines edge overlay and level");

    m.w32(lp + 12, 0);
    draw(&mut m, 13)?;
    let z0 = px(&m, 48, 48);
    m.w32(lp + 12, 2);
    draw(&mut m, 13)?;
    let z2 = px(&m, 48, 48);
    r.check(
        (z0 == 0xF40E || z0 == 0x111A) && (z2 == 0xF40E || z2 == 0x111A) && z0 != z2 && px(&m, 300, 200) == 0,
        "grid 13 zebras: highlights >= ZEBRA_THRESH striped, phase shifts on fcnt, midtones = 0",
    );

    // Runtime zebra thresholds (90% Safe = 230, 100% Clip = 252) & Dual Highlight+Shadow (<= 12 cyan)
    let mut z_y = vec![80u8; 640 * 424];
    for y in 40..80usize {
        let row = y * 640;
        z_y[row + 40..row + 80].fill(235);  // 90% Safe only
        z_y[row + 120..row + 160].fill(246); // 95% & 90%, below 100%
        z_y[row + 200..row + 240].fill(253); // >= 100% Clip
        z_y[row + 280..row + 320].fill(6);   // Crushed shadow <= 12
    }
    m.wr(yv, &z_y);
    m.w32(lp + 12, 0);
    m.w8(zebralvl_addr, 1); // 90% Safe (230)
    draw(&mut m, 13)?;
    let z90_ok = px(&m, 48, 48) != 0 && px(&m, 288, 48) == 0;
    m.w8(zebralvl_addr, 2); // 100% Clip (252)
    draw(&mut m, 13)?;
    let z100_ok = px(&m, 128, 48) == 0 && px(&m, 208, 48) != 0;
    m.w8(zebralvl_addr, 3); // Dual Hi + Lo
    draw(&mut m, 13)?;
    let zh = px(&m, 208, 48);
    let zl0 = px(&m, 288, 48);
    let zl1 = px(&m, 292, 48);
    let zdual_ok = (zh == 0xF40E || zh == 0x111A)
        && ((zl0 == 0x1DFE && zl1 == 0x111A) || (zl0 == 0x111A && zl1 == 0x1DFE))
        && px(&m, 400, 200) == 0;
    m.w8(zebralvl_addr, 0);
    r.check(z90_ok && z100_ok && zdual_ok, "grid 13 zebra levels: 90% Safe, 100% Clip & Dual Hi(orange)+Lo(cyan)");

    // Full-frame scalar oracle for single Hi zebra (draw_zebras_arm) covering 2x-unrolled A/B
    // block pairs, register-rotated row stripes, all 3 thresholds (230/242/252), phases 0..7,
    // threshold boundaries (thresh-1/thresh), odd-column non-interference, and poisoned FB contract.
    let hi_zebra_expected = |y: &[u8], phase: u32, thresh: u8, poison: Option<u8>| {
        let hi = [0xF40Eu16, 0xF40E, 0x111A, 0x111A];
        let phase = phase as usize;
        let mut out = vec![poison.unwrap_or(0); 640 * 424 * 2];
        for sy in (0..424).step_by(2) {
            let row = sy / 2;
            for bx in 0..40usize {
                let any_hit = (0..8).any(|k| y[sy * 640 + bx * 16 + 2 * k] >= thresh);
                if !any_hit {
                    continue;
                }
                for k in 0..8 {
                    let sx = bx * 16 + 2 * k;
                    let v = y[sy * 640 + sx];
                    let px = if v >= thresh {
                        hi[(((row + phase) & 3) + ((sx / 2) & 3)) & 3]
                    } else {
                        0
                    };
                    for dy in [sy, sy + 1] {
                        for dx in [sx, sx + 1] {
                            let o = (dy * 640 + dx) * 2;
                            out[o..o + 2].copy_from_slice(&px.to_be_bytes());
                        }
                    }
                }
            }
        }
        out
    };
    let mut hi_bad = Vec::new();
    for (idx, &thresh) in [230u8, 242, 252].iter().enumerate() {
        let mut frame = vec![0u8; 640 * 424];
        for sy in 0..424usize {
            for x in 0..640usize {
                // Mix threshold-1, threshold, all-256 luma ramp, and odd-column poison 255
                frame[sy * 640 + x] = if x & 1 == 1 {
                    255
                } else if (x / 16).is_multiple_of(5) {
                    thresh - 1
                } else if (x / 16) % 5 == 1 {
                    thresh
                } else {
                    ((x * 13 + sy * 29 + idx * 7) & 255) as u8
                };
            }
        }
        m.wr(yv, &frame);
        for phase in 0..8u32 {
            m.cpu.fill_bytes(stock::ARGB_FB, 0xA5, 640 * 424 * 2);
            m.cpu.call(b.sym("draw_zebras_arm"), &[yv, phase, u32::from(thresh)], &[], STACK.1, 100_000_000)?;
            let got = m.rd(stock::ARGB_FB, 640 * 424 * 2);
            let expected = hi_zebra_expected(&frame, phase, thresh, Some(0xA5));
            if got != expected {
                hi_bad.push((thresh, phase, got.iter().zip(&expected).position(|(g, e)| g != e)));
            }
        }
    }
    r.eq(hi_bad, vec![], "Single Hi zebra direct full-frame poisoned oracle across thresholds 230/242/252 & phases 0..7");

    // Exhaustive A/B 32-px unroll pair combinations: {clear, all-above, mixed-first, mixed-last} x {same}
    let mut ab_y = vec![241u8; 640 * 424];
    for sy in (0..424).step_by(2) {
        let row = sy / 2;
        for pair in 0..20usize {
            let pat_a = (row + pair) % 4;
            let pat_b = ((row + pair) / 4) % 4;
            for (half, pat) in [(0usize, pat_a), (16usize, pat_b)] {
                let bx = pair * 32 + half;
                match pat {
                    0 => {} // all-below (241)
                    1 => for k in 0..8 { ab_y[sy * 640 + bx + 2 * k] = 242; }, // all-above
                    2 => ab_y[sy * 640 + bx] = 242,                            // single hit lane 0
                    _ => ab_y[sy * 640 + bx + 14] = 255,                       // single hit lane 7
                }
                // Odd columns carry opposite values to prove odd-byte non-interference
                for k in 0..8 {
                    ab_y[sy * 640 + bx + 2 * k + 1] = if pat == 0 { 255 } else { 0 };
                }
            }
        }
    }
    m.wr(yv, &ab_y);
    m.cpu.fill_bytes(stock::ARGB_FB, 0xA5, 640 * 424 * 2);
    m.cpu.call(b.sym("draw_zebras_arm"), &[yv, 5, 242], &[], STACK.1, 100_000_000)?;
    let ab_got = m.rd(stock::ARGB_FB, 640 * 424 * 2);
    let ab_exp = hi_zebra_expected(&ab_y, 5, 242, Some(0xA5));
    r.check(
        ab_got == ab_exp,
        format!("Single Hi zebra 2x-unrolled A/B block-pair combinations & odd-column isolation; first diff {:?}", ab_got.iter().zip(&ab_exp).position(|(g, e)| g != e)),
    );

    let mut hi_grid_bad = Vec::new();
    for (lvl, thresh) in [(0u8, 242u8), (1, 230), (2, 252)] {
        m.w8(zebralvl_addr, lvl);
        m.w32(lp + 12, u32::from(lvl) + 1);
        draw(&mut m, 13)?;
        let got = m.rd(stock::ARGB_FB, 640 * 424 * 2);
        let exp = hi_zebra_expected(&ab_y, u32::from(lvl) + 1, thresh, None);
        if got != exp {
            hi_grid_bad.push(lvl);
        }
    }
    m.w8(zebralvl_addr, 0);
    r.eq(hi_grid_bad, vec![], "Grid 13 single Hi zebra zeroed-caller full-frame oracle (95%/90%/100% IRE)");

    let dual_expected = |y: &[u8], phase: u32, hi_thresh: u8| {
        let hi = [0xF40Eu16, 0xF40E, 0x111A, 0x111A];
        let lo = [0x1DFEu16, 0x1DFE, 0x111A, 0x111A];
        let phase = phase as usize;
        let mut out = vec![0u8; 640 * 424 * 2];
        for sy in (0..424).step_by(2) {
            let row = sy / 2;
            for sx in (0..640).step_by(2) {
                let stripe = (sx / 2) & 3;
                let v = y[sy * 640 + sx];
                let px = if v >= hi_thresh {
                    hi[(((row + phase) & 3) + stripe) & 3]
                } else if v <= 12 {
                    lo[(((424 - row + phase) & 3) + stripe) & 3]
                } else {
                    0
                };
                for dy in [sy, sy + 1] {
                    for dx in [sx, sx + 1] {
                        let o = (dy * 640 + dx) * 2;
                        out[o..o + 2].copy_from_slice(&px.to_be_bytes());
                    }
                }
            }
        }
        out
    };
    let noisy_y: Vec<u8> = (0..640 * 424).map(|i| ((i * 37 + (i / 640) * 101) & 255) as u8).collect();
    m.wr(yv, &noisy_y);
    m.w8(zebralvl_addr, 3);
    for phase in 0..8u32 {
        m.w32(lp + 12, phase);
        draw(&mut m, 13)?;
        let mut got = vec![0u8; 640 * 424 * 2];
        m.cpu.read_bytes(stock::ARGB_FB, &mut got);
        let expected = dual_expected(&noisy_y, phase, 242);
        let diff = got.iter().zip(&expected).position(|(g, e)| g != e);
        r.check(got == expected, format!("Dual Hi+Lo full-frame reference, phase {phase}, all 256 luma values; first diff {diff:?}, got {:?}, expected {:?}", diff.map(|i| got[i]), diff.map(|i| expected[i])));
    }
    let low_y: Vec<u8> = (0..640 * 424).map(|i| ((i * 5 + i / 640) % 13) as u8).collect();
    let low_mid_y: Vec<u8> = (0..640 * 424).map(|i| if i % 16 < 8 { (i % 13) as u8 } else { 80 }).collect();
    for (name, frame_y) in [("all-low", &low_y), ("low/midtone", &low_mid_y)] {
        m.wr(yv, frame_y);
        for phase in 0..8u32 {
            m.w32(lp + 12, phase);
            draw(&mut m, 13)?;
            let mut got = vec![0u8; 640 * 424 * 2];
            m.cpu.read_bytes(stock::ARGB_FB, &mut got);
            let expected = dual_expected(frame_y, phase, 242);
            let diff = got.iter().zip(&expected).position(|(g, e)| g != e);
            r.check(got == expected, format!("Dual Hi+Lo {name} full-frame reference, phase {phase}; first diff {diff:?}, got {:?}, expected {:?}", diff.map(|i| got[i]), diff.map(|i| expected[i])));
        }
    }
    // The only C caller uses the compile-time ZEBRA_THRESH, so clang deliberately specializes
    // the two-argument emitted routine. Extra r2 values are ignored, not runtime thresholds.
    m.wr(yv, &noisy_y);
    let expected_specialized = dual_expected(&noisy_y, 3, 242);
    let mut specialized_ok = true;
    for ignored in [0u32, 128, 255] {
        m.cpu.fill_bytes(stock::ARGB_FB, 0, 640 * 424 * 2);
        m.cpu.call(b.sym("draw_zebras_dual_arm"), &[yv, 3, ignored], &[], STACK.1, 100_000_000)?;
        let got = m.rd(stock::ARGB_FB, 640 * 424 * 2);
        specialized_ok &= got == expected_specialized;
    }
    r.check(specialized_ok, "Dual Hi+Lo keeps the built ZEBRA_THRESH=242 specialization (ignored r2 = 0, 128, 255)");

    // Two frames contain every ordered pair in adjacent SWAR lanes (bytes 0 and 2 of each
    // sampled word): 33,920 pairs per frame, 65,536 total. This covers all carry/borrow and
    // shadow/highlight boundary combinations without testing a nonexistent runtime argument.
    for page in 0..2usize {
        let mut pair_y = vec![80u8; 640 * 424];
        for sy in (0..424).step_by(2) {
            for w in 0..160usize {
                let pair = page * 33_920 + (sy / 2) * 160 + w;
                if pair < 65_536 {
                    pair_y[sy * 640 + 4 * w] = (pair >> 8) as u8;
                    pair_y[sy * 640 + 4 * w + 2] = pair as u8;
                }
            }
        }
        m.wr(yv, &pair_y);
        m.cpu.fill_bytes(stock::ARGB_FB, 0, 640 * 424 * 2);
        m.cpu.call(b.sym("draw_zebras_dual_arm"), &[yv, 3], &[], STACK.1, 100_000_000)?;
        let got = m.rd(stock::ARGB_FB, 640 * 424 * 2);
        let expected = dual_expected(&pair_y, 3, 242);
        let diff = got.iter().zip(&expected).position(|(g, e)| g != e);
        r.check(
            got == expected,
            format!("Dual Hi+Lo adjacent-luma pairs page {page}/2 (65,536 total); first diff {diff:?}, got {:?}, expected {:?}", diff.map(|i| got[i]), diff.map(|i| expected[i])),
        );
    }
    // Dual mixed-block checker + boundary + poisoned-buffer contract (clear blocks keep poison,
    // hit blocks burst-store all 8 words including 0 for midtones).
    {
        let mut mix_y = vec![128u8; 640 * 424];
        let vals = [0u8, 12, 13, 128, 241, 242, 255, 13];
        for sy in (0..424).step_by(2) {
            for bx in 0..40usize {
                let kind = (bx + sy / 2) % 5;
                for k in 0..8 {
                    let v = match kind {
                        0 => 80,                                  // all-clear (must keep poison)
                        1 => if k & 1 == 0 { 255 } else { 0 },    // checker hi/lo
                        2 => vals[(k + bx) & 7],                  // hi/lo/mid threshold boundaries
                        3 => if k == 7 { 12 } else { 13 },        // single shadow boundary
                        _ => if k == 0 { 242 } else { 241 },      // single highlight boundary
                    };
                    mix_y[sy * 640 + bx * 16 + 2 * k] = v;
                    mix_y[sy * 640 + bx * 16 + 2 * k + 1] = 255;  // unsampled odd column
                }
            }
        }
        m.wr(yv, &mix_y);
        m.cpu.fill_bytes(stock::ARGB_FB, 0xA5, 640 * 424 * 2);
        m.cpu.call(b.sym("draw_zebras_dual_arm"), &[yv, 6], &[], STACK.1, 100_000_000)?;
        let got = m.rd(stock::ARGB_FB, 640 * 424 * 2);
        let mut expected = dual_expected(&mix_y, 6, 242);
        for sy in (0..424).step_by(2) {
            for bx in 0..40usize {
                let all_clear = (0..8).all(|k| {
                    let v = mix_y[sy * 640 + bx * 16 + 2 * k];
                    v > 12 && v < 242
                });
                if all_clear {
                    for dy in [sy, sy + 1] {
                        for dx in bx * 16..(bx + 1) * 16 {
                            let o = (dy * 640 + dx) * 2;
                            expected[o..o + 2].fill(0xA5);
                        }
                    }
                }
            }
        }
        let diff = got.iter().zip(&expected).position(|(g, e)| g != e);
        r.check(
            got == expected,
            format!("Dual Hi+Lo checker/boundary mixed-block full-frame poisoned oracle; first diff {diff:?}, got {:?}, expected {:?}", diff.map(|i| got[i]), diff.map(|i| expected[i])),
        );
    }
    m.w8(zebralvl_addr, 0);

    // Mode 17: Foveon False Color (IRE) 6 zones + vertical IRE scale bar
    let mut fc_y = vec![80u8; 640 * 424];
    for y in 40..80usize {
        let row = y * 640;
        fc_y[row + 20..row + 60].fill(6);    // Violet (0..5% IRE)
        fc_y[row + 80..row + 120].fill(20);  // Blue (5..11% IRE)
        fc_y[row + 140..row + 180].fill(116); // Green (42..48% IRE, 18% gray)
        fc_y[row + 200..row + 240].fill(156); // Pink (56..66% IRE, skin)
        fc_y[row + 260..row + 300].fill(232); // Yellow (88..94% IRE, Foveon knee)
        fc_y[row + 320..row + 360].fill(250); // Red (94..100% IRE, clipped)
    }
    m.wr(yv, &fc_y);
    draw(&mut m, 17)?;
    let fc_zones = [
        px(&m, 30, 50), px(&m, 90, 50), px(&m, 150, 50),
        px(&m, 210, 50), px(&m, 270, 50), px(&m, 330, 50),
        px(&m, 400, 200), px(&m, 626, 124), px(&m, 626, 284),
    ];
    r.eq(
        fc_zones,
        [0x61FD, 0x17FB, 0x1E3C, 0xF7BC, 0xFE1C, 0xF11E, 0x0000, 0xF11F, 0x61FF],
        "grid 17 false color: 6 IRE zones, transparent midtones & right-edge legend bar",
    );
    let fc_color = |bin: u8| -> u16 {
        match bin {
            0..=2 => 0x61FD,
            3..=6 => 0x17FB,
            27..=30 => 0x1E3C,
            36..=41 => 0xF7BC,
            56..=59 => 0xFE1C,
            60..=63 => 0xF11E,
            _ => 0,
        }
    };
    let expected_lut: Vec<u32> = (0..64u8)
        .map(|bin| {
            let px = u32::from(fc_color(bin));
            px << 16 | px
        })
        .collect();
    let actual_lut: Vec<u32> = (0..64u32).map(|i| b.img.u32(b.sym("fc_lut32") + 4 * i)).collect();
    r.eq(actual_lut, expected_lut, "false-color LUT matches the six independent IRE zone definitions");

    // Direct oracle with poison: all-zero LUT blocks retain poison; every nonzero block writes
    // all eight words, including zero-valued pixels. This pins the zeroed-FB call contract.
    let fc_expected = |y: &[u8], poison: u8| {
        let mut out = vec![poison; 640 * 424 * 2];
        for sy in (0..424).step_by(2) {
            for bx in 0..40usize {
                let sample = |k: usize| fc_color(y[sy * 640 + bx * 16 + 2 * k] >> 2);
                if (0..8).all(|k| sample(k) == 0) {
                    continue;
                }
                for k in 0..8 {
                    let sx = bx * 16 + 2 * k;
                    let bytes = sample(k).to_be_bytes();
                    for dy in [sy, sy + 1] {
                        for dx in [sx, sx + 1] {
                            let o = (dy * 640 + dx) * 2;
                            out[o..o + 2].copy_from_slice(&bytes);
                        }
                    }
                }
            }
        }
        out
    };
    let ramp_y: Vec<u8> = (0..640 * 424).map(|i| i as u8).collect();
    let patterns = [
        [20, 20, 3, 3, 20, 20, 3, 3],
        [0, 0, 0, 0, 3, 3, 3, 3],
        [8, 8, 10, 10, 8, 8, 10, 10],
        [14, 14, 15, 15, 14, 14, 15, 15],
        [20, 20, 3, 3, 10, 10, 14, 14],
        [3, 4, 5, 6, 3, 4, 5, 6],
    ];
    let mut patterned_y = vec![80u8; 640 * 424];
    for sy in (0..424).step_by(2) {
        for bx in 0..40usize {
            let pattern = &patterns[(bx + sy / 2) % patterns.len()];
            for k in 0..8 {
                patterned_y[sy * 640 + bx * 16 + 2 * k] = pattern[k] << 2 | ((bx + k + sy / 2) & 3) as u8;
            }
        }
    }
    let mut one_off_y = vec![80u8; 640 * 424];
    for sy in (0..424).step_by(2) {
        for bx in 0..40usize {
            let k = (bx + sy / 2) & 7;
            one_off_y[sy * 640 + bx * 16 + 2 * k] = 12;
        }
    }
    let fc_frames = [
        ("all 256 luma values", ramp_y),
        ("noisy", noisy_y.clone()),
        ("same-zone and adversarial XOR patterns", patterned_y.clone()),
        ("one nonzero sample per block", one_off_y),
        ("all-midtone poison preservation", vec![80; 640 * 424]),
    ];
    for (name, frame_y) in &fc_frames {
        m.wr(yv, frame_y);
        m.cpu.fill_bytes(stock::ARGB_FB, 0xA5, 640 * 424 * 2);
        m.cpu.call(b.sym("draw_false_color_arm"), &[yv], &[], STACK.1, 100_000_000)?;
        let mut got = vec![0u8; 640 * 424 * 2];
        m.cpu.read_bytes(stock::ARGB_FB, &mut got);
        let expected = fc_expected(frame_y, 0xA5);
        let diff = got.iter().zip(&expected).position(|(g, e)| g != e);
        r.check(
            got == expected,
            format!("False Color direct {name} full-frame poisoned reference; first diff {diff:?}, got {:?}, expected {:?}", diff.map(|i| got[i]), diff.map(|i| expected[i])),
        );
    }

    m.wr(yv, &patterned_y);
    draw(&mut m, 17)?;
    let mut got = vec![0u8; 640 * 424 * 2];
    m.cpu.read_bytes(stock::ARGB_FB, &mut got);
    let expected = fc_expected(&patterned_y, 0);
    let diff = (0..got.len()).find(|&i| {
        let p = i / 2;
        let (x, y) = (p % 640, p / 640);
        let legend = (622..634).contains(&x) && (114..310).contains(&y);
        !legend && got[i] != expected[i]
    });
    r.check(diff.is_none(), format!("False Color grid 17 zeroed-caller full-frame reference outside legend; first diff {diff:?}"));
    m.wr(yv, &y_plane);

    draw(&mut m, 14)?;
    let mut has_bar = false;
    for y in 360..408 {
        for x in 496..624 {
            let p = px(&m, x, y);
            if (p & 0xF) == 14 && (p >> 4) != 0 {
                has_bar = true;
            }
        }
    }
    r.check(px(&m, 492, 354) == 0xBBBC && has_bar, "grid 14 RGB histogram: frame & bars drawn");
    m.wr(yv, &vec![255u8; 640 * 424]);
    draw(&mut m, 14)?;
    r.eq(px(&m, 625, 360), 0xF11F, "grid 14 RGB histogram: highlight clip bar");

    // Independent full-frame scalar oracle for draw_histogram_arm covering extreme BT.601 chroma
    // clamp bounds (R[-45..108], G[-34..97], B[-57..120]), Cr-neutral/gray fast paths, luma-only
    // fallback, and highlight clip-bar threshold 180 vs 181.
    let hist_expected = |y: &[u8], cb: Option<&[u8]>, cr: Option<&[u8]>| -> Vec<u8> {
        let mut hr = [0u32; 64];
        let mut hg = [0u32; 64];
        let mut hb = [0u32; 64];
        match (cb, cr) {
            (Some(cb), Some(cr)) => {
                for row in 0..52usize {
                    for col in 0..79usize {
                        let yv = i32::from(y[(4 + row * 8) * 640 + 4 + col * 8]);
                        let cbv = i32::from(cb[(4 + row * 8) * 320 + 2 + col * 4]) - 128;
                        let crv = i32::from(cr[(4 + row * 8) * 320 + 2 + col * 4]) - 128;
                        let r = (yv + ((crv * 359) >> 8)).clamp(0, 255) as usize >> 2;
                        let g = (yv - ((cbv * 88 + crv * 183) >> 8)).clamp(0, 255) as usize >> 2;
                        let b = (yv + ((cbv * 454) >> 8)).clamp(0, 255) as usize >> 2;
                        hr[r] += 1;
                        hg[g] += 1;
                        hb[b] += 1;
                    }
                }
            }
            _ => {
                for row in 0..52usize {
                    for col in 0..79usize {
                        let bin = (y[(4 + row * 8) * 640 + 4 + col * 8] >> 2) as usize;
                        hr[bin] += 1;
                        hg[bin] += 1;
                        hb[bin] += 1;
                    }
                }
            }
        }
        let mut peak = 24u32;
        for i in 2..62 {
            peak = peak.max(hr[i]).max(hg[i]).max(hb[i]);
        }
        let mut out = vec![0u8; 640 * 424 * 2];
        let mut put = |x: usize, y: usize, px: u16| {
            let o = (y * 640 + x) * 2;
            out[o..o + 2].copy_from_slice(&px.to_be_bytes());
        };
        const HX: usize = 492;
        const HY: usize = 354;
        const HW: usize = 136;
        const HH: usize = 58;
        for dy in 1..HH - 1 {
            for dx in 1..HW - 1 {
                put(HX + dx, HY + dy, 0x000B);
            }
        }
        for dx in 0..HW {
            put(HX + dx, HY, 0xBBBC);
            put(HX + dx, HY + HH - 1, 0xBBBC);
        }
        for dy in 1..HH - 1 {
            put(HX, HY + dy, 0xBBBC);
            put(HX + HW - 1, HY + dy, 0xBBBC);
        }
        for dy in (2..HH - 2).step_by(2) {
            for gx in [34usize, 66, 98] {
                put(HX + gx, HY + dy, 0x5559);
            }
        }
        for i in 0..64usize {
            let mut bars = [
                (if hr[i] >= peak { 48 } else { ((hr[i] * 48) / peak) as usize }, 14u16 << 12),
                (if hg[i] >= peak { 48 } else { ((hg[i] * 48) / peak) as usize }, 14u16 << 8),
                (if hb[i] >= peak { 48 } else { ((hb[i] * 48) / peak) as usize }, 15u16 << 4),
            ];
            bars.sort_by_key(|b| b.0);
            let mut cy = HY + HH - 5;
            let segs = [
                (bars[0].0, bars[0].1 | bars[1].1 | bars[2].1 | 14),
                (bars[1].0 - bars[0].0, bars[1].1 | bars[2].1 | 14),
                (bars[2].0 - bars[1].0, bars[2].1 | 14),
            ];
            for (n, c) in segs {
                for _ in 0..n {
                    put(HX + 4 + 2 * i, cy, c);
                    put(HX + 4 + 2 * i + 1, cy, c);
                    cy -= 1;
                }
            }
        }
        if hr[63] + hg[63] + hb[63] > 180 {
            for dy in 2..HH - 2 {
                put(HX + HW - 3, HY + dy, 0xF11F);
                put(HX + HW - 2, HY + dy, 0xF11F);
            }
        }
        out
    };
    {
        // Extreme chroma corners: exercises every negative & positive out-of-range padded bin
        let mut ext_y = vec![0u8; 640 * 424];
        let mut ext_cb = vec![128u8; 320 * 424];
        let mut ext_cr = vec![128u8; 320 * 424];
        let y_vals = [0u8, 1, 64, 128, 192, 254, 255];
        let c_vals = [0u8, 1, 64, 128, 192, 254, 255];
        for row in 0..52usize {
            for col in 0..79usize {
                let idx = row * 79 + col;
                ext_y[(4 + row * 8) * 640 + 4 + col * 8] = y_vals[idx % y_vals.len()];
                ext_cb[(4 + row * 8) * 320 + 2 + col * 4] = c_vals[(idx / 7) % c_vals.len()];
                ext_cr[(4 + row * 8) * 320 + 2 + col * 4] = c_vals[(idx / 49) % c_vals.len()];
            }
        }
        m.wr(yv, &ext_y);
        m.wr(cbv, &ext_cb);
        m.wr(crv, &ext_cr);
        m.cpu.fill_bytes(stock::ARGB_FB, 0, 640 * 424 * 2);
        m.cpu.call(b.sym("draw_histogram_arm"), &[yv, cbv, crv], &[], STACK.1, 100_000_000)?;
        let got = m.rd(stock::ARGB_FB, 640 * 424 * 2);
        let exp = hist_expected(&ext_y, Some(&ext_cb), Some(&ext_cr));
        r.check(
            got == exp,
            format!("Histogram full-frame scalar oracle: extreme chroma clamp bounds; first diff {:?}", got.iter().zip(&exp).position(|(g, e)| g != e)),
        );

        // Mixed paths (color + Cr-neutral + gray), pure gray, luma-only (Cb=Cr=0), clip bar 180 vs 181
        let mut hist_bad = Vec::new();
        // 1. Mixed color/Cr-neutral/gray with varied bins
        let mut mix_hy = vec![0u8; 640 * 424];
        let mut mix_cb = vec![128u8; 320 * 424];
        let mut mix_cr = vec![128u8; 320 * 424];
        for row in 0..52usize {
            for col in 0..79usize {
                let idx = row * 79 + col;
                mix_hy[(4 + row * 8) * 640 + 4 + col * 8] = ((idx * 17) & 255) as u8;
                match idx % 3 {
                    0 => {} // gray (Cb=128, Cr=128)
                    1 => mix_cb[(4 + row * 8) * 320 + 2 + col * 4] = ((idx * 31) & 255) as u8, // Cr=128, Cb!=128
                    _ => {
                        mix_cb[(4 + row * 8) * 320 + 2 + col * 4] = ((idx * 23) & 255) as u8;
                        mix_cr[(4 + row * 8) * 320 + 2 + col * 4] = ((idx * 41) & 255) as u8;
                    }
                }
            }
        }
        // 2. Pure neutral gray
        let gray_cb = vec![128u8; 320 * 424];
        let gray_cr = vec![128u8; 320 * 424];
        // 3 & 4. Clip-bar exact threshold: 60 white gray samples -> hr[63]+hg[63]+hb[63]=180 (off) vs 61 -> 183 (on)
        let make_clip = |n_white: usize| {
            let mut fy = vec![128u8; 640 * 424];
            for k in 0..n_white {
                let (row, col) = (k / 79, k % 79);
                fy[(4 + row * 8) * 640 + 4 + col * 8] = 252;
            }
            fy
        };
        let clip_180 = make_clip(60);
        let clip_183 = make_clip(61);
        let cases = [
            ("mixed", &mix_hy[..], Some(&mix_cb[..]), Some(&mix_cr[..])),
            ("pure-gray", &mix_hy[..], Some(&gray_cb[..]), Some(&gray_cr[..])),
            ("luma-only", &mix_hy[..], None, None),
            ("clip-180", &clip_180[..], Some(&gray_cb[..]), Some(&gray_cr[..])),
            ("clip-183", &clip_183[..], Some(&gray_cb[..]), Some(&gray_cr[..])),
        ];
        for (name, fy, fcb, fcr) in cases {
            m.wr(yv, fy);
            let (cb_arg, cr_arg) = match (fcb, fcr) {
                (Some(c), Some(r)) => { m.wr(cbv, c); m.wr(crv, r); (cbv, crv) }
                _ => (0, 0),
            };
            m.cpu.fill_bytes(stock::ARGB_FB, 0, 640 * 424 * 2);
            m.cpu.call(b.sym("draw_histogram_arm"), &[yv, cb_arg, cr_arg], &[], STACK.1, 100_000_000)?;
            let got = m.rd(stock::ARGB_FB, 640 * 424 * 2);
            let exp = hist_expected(fy, fcb, fcr);
            if got != exp {
                hist_bad.push((name, got.iter().zip(&exp).position(|(g, e)| g != e)));
            }
        }
        r.eq(hist_bad, vec![], "Histogram full-frame scalar oracle: mixed/Cr-neutral/gray, pure gray, luma-only & clip-bar 180/183");

        // Torn-read regression: simulate lv_ycap_hook rotating all 4 lv_planes fields to a new
        // buffer triple right before the cb load or right before the cr load inside draw_histogram.
        {
            use std::cell::Cell;
            let (ya, cba, cra) = (0xA240_0000u32, 0xA245_0000u32, 0xA248_0000u32);
            let (yb, cbb, crb) = (0xA250_0000u32, 0xA255_0000u32, 0xA258_0000u32);
            let y_a = vec![64u8; 640 * 424];
            let cb_a = vec![128u8; 320 * 424];
            let cr_a = vec![128u8; 320 * 424];
            let y_b = vec![180u8; 640 * 424];
            let cb_b = vec![40u8; 320 * 424];
            let cr_b = vec![220u8; 320 * 424];
            let exp_b = hist_expected(&y_b, Some(&cb_b), Some(&cr_b));
            let hist_box = |fb: &[u8]| -> Vec<u8> {
                let mut box_px = Vec::with_capacity(136 * 58 * 2);
                for y in 354..412usize {
                    let o = (y * 640 + 492) * 2;
                    box_px.extend_from_slice(&fb[o..o + 136 * 2]);
                }
                box_px
            };
            let exp_box_b = hist_box(&exp_b);
            let gd = b.sym_info("grid_draw_hook");
            let mut scan_ranges = vec![(gd.va(), gd.size)];
            if let Some(dh) = b.syms.get("draw_histogram") {
                scan_ranges.push((dh.va(), dh.size));
            }
            let mut torn_bad = Vec::new();
            for (tag, imm_op) in [("before-cb", 0x6840u16), ("before-cr", 0x6880u16)] {
                let mut mt = Machine::new(&b.img, 0);
                mt.load_nv_defaults();
                mt.stub_on(b.sym("stock_ui_screen_id"), |_| Some(0x15));
                mt.stub_on(b.sym("stock_lv_active"), |_| Some(1));
                mt.stub_on(b.sym("stock_accel_x"), |_| Some(0));
                mt.stub_on(b.sym("stock_accel_y"), |_| Some(64));
                mt.w8(stock::NV_GRID, 14);
                mt.wr(ya, &y_a);
                mt.wr(cba, &cb_a);
                mt.wr(cra, &cr_a);
                mt.wr(yb, &y_b);
                mt.wr(cbb, &cb_b);
                mt.wr(crb, &cr_b);
                mt.w32(lp, ya);
                mt.w32(lp + 4, cba);
                mt.w32(lp + 8, cra);
                mt.w32(lp + 12, 10);
                let fired = Rc::new(Cell::new(false));
                for &(start, size) in &scan_ranges {
                    for off in (0..size).step_by(2) {
                        let pc = start + off;
                        let h = b.img.u16(pc);
                        if (h & 0xFFC0) == imm_op {
                            let rn = ((h >> 3) & 7) as usize;
                            let f = fired.clone();
                            mt.watch(pc, move |cpu| {
                                if cpu.r[rn] == lp && !f.get() {
                                    f.set(true);
                                    cpu.w32(lp, yb);
                                    cpu.w32(lp + 4, cbb);
                                    cpu.w32(lp + 8, crb);
                                    cpu.w32(lp + 12, 11);
                                }
                            });
                        }
                    }
                }
                mt.call(gdraw, &[SCRATCH])?;
                let got_box = hist_box(&mt.rd(stock::ARGB_FB, 640 * 424 * 2));
                if !fired.get() || got_box != exp_box_b {
                    torn_bad.push((tag, fired.get(), got_box == exp_box_b));
                }
            }
            r.eq(torn_bad, vec![], "Histogram lv_planes re-read loop rejects torn (y, cb, cr) triples across concurrent rotation");
        }

        m.wr(yv, &y_plane);
        m.wr(cbv, &vec![110u8; 320 * 424]);
        m.wr(crv, &vec![160u8; 320 * 424]);
    }

    draw(&mut m, 7)?;
    r.check(
        px(&m, 48, 48) == 0 && px(&m, 160, 120) == 0 && px(&m, 500, 360) == 0 && px(&m, 319, 211) == 0x1F3E,
        "HUD transition 14 -> 7 clears outer overlay plane before drawing level",
    );
    draw(&mut m, 0)?;
    r.check(m.rd(stock::ARGB_FB, 640 * 424 * 2).iter().all(|&b| b == 0), "HUD transition 7 -> 0 clears entire 640x424 RGBA4444 plane");
    Ok(())
}

fn check_updater_and_package(b: &Build, r: &mut Report) -> Result<(), String> {
    let ver = b.pkg.version();
    let mut parts = ver.split('.');
    let maj: u8 = parts.next().unwrap_or("0").parse().map_err(|_| "bad major ver")?;
    let mi: u8 = parts.next().unwrap_or("0").parse().map_err(|_| "bad minor ver")?;
    r.eq(
        b.img.rd(stock::VERSION_IMM, 4),
        &[0x22, maj, 0x23, mi][..],
        format!("'Current version' immediates = {maj}.{mi:02}"),
    );
    for va in stock::UPDATER_BANNERS {
        let old = b.stock.cstr(va, 64);
        let new = b.img.cstr(va, 64);
        let slot = (old.len() + 4) & !3;
        let mut padded = new.as_bytes().to_vec();
        padded.resize(slot, 0);
        let ok = old.contains("Debug Write")
            && !new.is_empty()
            && new != old
            && b.img.rd(va, slot) == padded.as_slice()
            && b.img.rd(va + slot as u32, 4) == b.stock.rd(va + slot as u32, 4);
        r.check(ok, format!("updater banner {va:#x}: {old:?} -> {new:?}"));
    }

    let s = b.stock.pkg.as_ref().ok_or("stock firmware must be a DP2MVxxx.bin package")?;
    r.eq(b.pkg_name.as_str(), &format!("DP2MV{maj}{mi:02}.bin"), "package file name matches the version");
    let vs = b.pkg.versions();
    let ver_ok = vs[0] == vs[1] && vs[1] == vs[2] && ver.len() == 4 && ver.as_bytes()[1] == b'.';
    r.check(ver_ok, format!("header versions {vs:?}"));
    let date = b.pkg.date();
    r.check(date.len() == 8 && date.starts_with("20") && date.bytes().all(|c| c.is_ascii_digit()), format!("header date {date}"));
    r.eq(
        (b.pkg.raw.len(), b.pkg.flags(), b.pkg.c78_off()),
        (s.raw.len(), s.flags(), s.c78_off()),
        "size, stage flags, M6M segment offset = stock",
    );
    let diff_hdr: Vec<usize> = (0..0x80)
        .filter(|&i| !(0x10..0x20).contains(&i) && !(0x30..0x58).contains(&i) && b.pkg.raw[i] != s.raw[i])
        .collect();
    r.eq(diff_hdr, vec![], "header bytes outside version/date = stock");
    r.check(b.pkg.segment("bridge") == s.segment("bridge"), "bridge M6M segment is stock");
    let firm = fs::read(b.dir.join("Firm.bin")).map_err(|e| format!("read Firm.bin: {e}"))?;
    r.check(b.pkg.segment("m5") == firm.as_slice(), "M5 segment = Firm.bin");
    r.check(b.pkg.segment("m6m") == b.img.c78.as_slice(), "M6M segment = C78FRM.bin");
    Ok(())
}

pub fn run(b: &Build, r: &mut Report) -> Result<bool, String> {
    check_invariants(b, r);
    check_hooks(b, r)?;
    check_menus(b, r)?;
    check_strings(b, r)?;
    let addrs = check_settings(b, r)?;
    check_bracketing(b, r)?;
    check_min_shutter(b, r, addrs["minshutter"])?;
    check_auto_iso50(b, r)?;
    check_ipc(b, r, &addrs)?;
    check_makernote(b, r)?;
    check_lv(b, r)?;
    check_ycap(b, r)?;
    check_hud(b, r, addrs["peakcol"], addrs["peaksens"], addrs["zebralvl"])?;
    check_updater_and_package(b, r)?;
    Ok(true)
}
