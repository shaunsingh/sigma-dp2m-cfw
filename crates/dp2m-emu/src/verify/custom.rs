//! custom suite (133 checks): F1/F2 custom exposure modes, NOR persistence, setter transitions,
//! slot save/compare/reload, LV widget 0x2E, menu 0x05, MODE screen 23, Custom Mode Setting screen 31.

use std::cell::RefCell;
use std::rc::Rc;

use crate::machine::{Machine, Nor, STACK, outside};
use crate::stock;

use super::{Build, Report};

const SLOTS: u32 = 0xA093_9818;
const BACKUP: u32 = 0xA093_99BC;
const SZ: usize = 0x8C;
const SHADOW: u32 = 0xA07F_BC0D;
const PREV: u32 = 0xA07F_BC0C;
const CS_MAGIC: u32 = 0x4334_4335;
const NV_MAGIC: u32 = 0x2012_0918;
const CUSTOM_STATE: u32 = 0xA07F_23E4;
const FAKE_ICON: u32 = 0xA1F0_0000;
const HDR: (u32, u32) = (0xA07D_D720, 0xA07D_D728);

fn slot_of(mode: u32) -> Option<usize> {
    match mode {
        5 => Some(0),
        6 => Some(1),
        7 => Some(2),
        9 => Some(3),
        10 => Some(4),
        _ => None,
    }
}

fn pat(tag: u8, prog: u8) -> Vec<u8> {
    let mut v = Vec::with_capacity(SZ);
    v.push(prog);
    for i in 1..SZ {
        v.push(tag.wrapping_add(i as u8));
    }
    v
}

pub fn run(b: &Build, r: &mut Report) -> Result<bool, String> {
    if !b.has_all(&["cs", "expo_switch", "slot_ptr", "mode_strip", "custom_icons"]) {
        return Ok(false);
    }
    let cs_sym = b.sym_info("cs");
    let (cs, cs_end) = (cs_sym.va(), cs_sym.va() + cs_sym.size);
    let si4 = b.cat("SID_F1") - stock::STOCK_SIDS;
    let si5 = b.cat("SID_F2") - stock::STOCK_SIDS;
    let (mode_icon, widget_icon) = (b.cat("ICON_LABEL_MODE"), b.cat("ICON_LABEL_WIDGET"));
    let rom_cur = b.img.rd(stock::NV_DEFAULTS + 0x3C, SZ).to_vec();

    // Static checks
    for &(site, sym) in &[
        (0xA005_9A7Eu32, "slot_save"),
        (0xA016_761C, "slot_ptr"),
        (0xA016_FA2E, "expo_switch_hook"),
        (0xA016_75E2, "program_hook"),
        (0xA016_89C2, "expo_boot_reload"),
        (0xA016_A7FC, "expo_mode_c1"),
        (0xA016_9884, "expo_mode_c1"),
        (0xA016_BAE8, "expo_mode_c1"),
        (0xA016_BC26, "widget_icon"),
        (0xA016_BBBA, "widget_slot_same"),
        (0xA004_0168, "nv_load_copy"),
        (0xA004_01E6, "nv_flush_write"),
        (0xA016_757E, "nv_reset_copy"),
    ] {
        r.eq(b.img.bl_target(site), Some(b.sym(sym)), format!("BL @ {site:#x} -> {sym}"));
    }
    r.eq(b.img.u32(0xA005_247C), b.sym("mode_strip") + 4, "MODE pool word -> mode_strip.cell");
    r.eq(b.img.u32(0xA005_9AE0), b.sym("custom_icons"), "Custom Mode Setting pool word -> custom_icons");
    let imms: &[(u32, u16)] = &[
        (0xA005_22A2, 0x2C0A), (0xA005_22EC, 0x2109), (0xA005_2342, 0x2109), (0xA005_23AC, 0x280A),
        (0xA005_220E, 0x2082), (0xA005_2210, 0x0080), (0xA005_9898, 0x4991), (0xA005_9914, 0x4872),
        (0xA005_98F4, 0x2C05), (0xA005_99E4, 0x2104), (0xA005_9A02, 0x2104), (0xA016_7622, 0x38FF),
        (0xA016_7628, 0x3895),
    ];
    let got_imms: Vec<(u32, u16)> = imms.iter().map(|&(a, _)| (a, b.img.u16(a))).collect();
    r.eq(got_imms, imms.to_vec(), format!("{} immediates", imms.len()));

    type CallLog = Rc<RefCell<Vec<(&'static str, [u32; 4])>>>;
    let mut m = Machine::new(&b.img, 0);
    m.load_nv_defaults();
    let calls: CallLog = Rc::new(RefCell::new(Vec::new()));
    let rects: Rc<RefCell<Vec<[u16; 4]>>> = Rc::new(RefCell::new(Vec::new()));

    for &(name, va, ret) in &[
        ("prog_set", 0xA016_F81Eu32, None),
        ("f_1CEDE", 0xA001_CEDE, None),
        ("f_403EE", 0xA004_03EE, None),
        ("f_7B42", 0xA016_7B42, None),
        ("apply", 0xA016_F962, None),
        ("still", 0xA003_D9FA, None),
        ("movie", 0xA003_E85A, None),
        ("f_80FA", 0xA016_80FA, Some(0)),
        ("f_809A", 0xA016_809A, Some(1)),
        ("clear", stock::FILL_BOX, None),
        ("blit", stock::BLIT, None),
        ("f_37C68", 0xA003_7C68, None),
        ("text", 0xA004_18AE, None),
        ("f_4BB98", 0xA004_BB98, None),
        ("redraw", 0xA005_246C, None),
        ("f_4190E", 0xA004_190E, None),
        ("f_41B48", 0xA004_1B48, None),
        ("close", 0xA004_2B80, None),
        ("str", stock::UI_STRING, Some(0)),
        ("f_3E78C", 0xA003_E78C, Some(0)),
        ("f_3F418", 0xA003_F418, Some(0)),
    ] {
        let cc = calls.clone();
        m.stub_on(va, move |c| {
            cc.borrow_mut().push((name, [c.r[0], c.r[1], c.r[2], c.r[3]]));
            ret
        });
    }
    for (name, va) in [("hilite", 0xA004_1474u32), ("box", 0xA004_4B76u32)] {
        let (cc, rc) = (calls.clone(), rects.clone());
        m.stub_on(va, move |c| {
            cc.borrow_mut().push((name, [c.r[0], c.r[1], c.r[2], c.r[3]]));
            let p = c.r[0];
            rc.borrow_mut().push(core::array::from_fn(|k| c.r16_be(p + (2 * k) as u32)));
            None
        });
    }
    {
        let cc = calls.clone();
        m.stub_on(stock::ICON_GET, move |c| {
            let i = c.r[0];
            cc.borrow_mut().push(("icon", [i, c.r[1], c.r[2], c.r[3]]));
            let (w, h): (u16, u16) = if (0xAF..0xB7).contains(&i) || (i >> 10) == (mode_icon >> 10) {
                (51, 31)
            } else if (0xB7..0xBA).contains(&i) || (i >> 10) == (widget_icon >> 10) {
                (35, 20)
            } else if i == 0x96 || i == 0x97 {
                (13, 16)
            } else {
                (11, 11)
            };
            c.w32(FAKE_ICON, FAKE_ICON + 0x100);
            c.w16(FAKE_ICON + 4, w);
            c.w16(FAKE_ICON + 6, h);
            Some(FAKE_ICON)
        });
    }
    let nor = Nor::install(&mut m);

    let nv = (stock::NV_BLOCK, stock::NV_BLOCK + stock::NV_LEN as u32);
    let csr = (cs, cs_end);
    let sh = (PREV, SHADOW + 1);
    let buf = (Nor::BUF, Nor::BUF + 0x800);

    let mut cs_rom = CS_MAGIC.to_be_bytes().to_vec();
    cs_rom.extend_from_slice(&rom_cur);
    cs_rom.extend_from_slice(&rom_cur);
    let mut cs_test = CS_MAGIC.to_be_bytes().to_vec();
    cs_test.extend_from_slice(&pat(0x50, 3));
    cs_test.extend_from_slice(&pat(0x60, 4));

    let slot_rd = |m: &Machine, i: usize| -> Vec<u8> {
        if i < 3 {
            m.rd(SLOTS + (i * SZ) as u32, SZ)
        } else {
            m.rd(cs + 4 + ((i - 3) * SZ) as u32, SZ)
        }
    };
    let seq = || -> Vec<&'static str> {
        calls
            .borrow()
            .iter()
            .map(|c| c.0)
            .filter(|&n| matches!(n, "apply" | "still" | "movie"))
            .collect()
    };

    m.cpu.start_write_log(&[STACK, (FAKE_ICON, FAKE_ICON + 8)]);
    let call = |m: &mut Machine, va: u32, args: &[u32]| -> Result<u32, String> {
        calls.borrow_mut().clear();
        rects.borrow_mut().clear();
        m.cpu.clear_write_log();
        m.call_ext(va, args, &[], 2_000_000)
    };

    // NOR load (boot) with and without record
    let mut nv_img = b.img.rd(stock::NV_DEFAULTS, stock::NV_PERSIST).to_vec();
    nv_img[0x3C..0x3C + SZ].copy_from_slice(&pat(0x10, 2));
    {
        let mut d = nor.data.borrow_mut();
        d[0..4].copy_from_slice(&NV_MAGIC.to_be_bytes());
        d[0x20..0x3E4].copy_from_slice(&nv_img);
        d[0x400..0x51C].copy_from_slice(&cs_test);
    }
    m.wr(cs, &[0xA5; 0x11C]);
    call(&mut m, stock::NV_LOAD, &[stock::NV_BLOCK])?;
    r.check(m.rd(stock::NV_BLOCK, stock::NV_PERSIST) == nv_img, "load: stock 0x3C4 B copied");
    r.check(m.rd(cs, 0x11C) == nor.data.borrow()[0x400..0x51C], "load: F1/F2 record restored from NOR 0x400");
    r.eq(outside(m.cpu.write_log(), &[nv, csr, HDR, buf]), vec![], "load: no stray writes");
    nor.data.borrow_mut()[0x400..0x404].fill(0xFF);
    call(&mut m, stock::NV_LOAD, &[stock::NV_BLOCK])?;
    r.check(m.rd(cs, 0x11C) == cs_rom, "load: missing magic -> ROM defaults");
    nor.data.borrow_mut()[0x400..0x404].copy_from_slice(&CS_MAGIC.to_be_bytes());
    call(&mut m, stock::NV_LOAD, &[stock::NV_BLOCK])?;

    let mut s012 = pat(0x20, 1);
    s012.extend_from_slice(&pat(0x30, 2));
    s012.extend_from_slice(&pat(0x40, 3));
    m.wr(SLOTS, &s012);
    m.wr(BACKUP, &pat(0x70, 0));
    nor.writes.borrow_mut().clear();
    m.wr(Nor::BUF, &[0xEE; 0x800]);
    call(&mut m, stock::NV_FLUSH, &[])?;
    let nw: Vec<(u32, u32)> = nor.writes.borrow().iter().map(|&(o, n, _)| (o, n)).collect();
    r.eq(nw, vec![(8, 0x514)], "flush: one NOR write (8, 0x514)");
    let wbuf = nor.writes.borrow().first().map_or_else(|| vec![0u8; 0x800], |w| w.2.clone());
    r.check(wbuf[0x18..0x3DC] == m.rd(stock::NV_BLOCK, stock::NV_PERSIST)[..], "flush: stock NVRAM block at NOR 0x20");
    r.check(wbuf[0x3DC..0x3F8].iter().all(|&x| x == 0), "flush: NOR 0x3E4..0x3FF zeroed");
    r.check(wbuf[0x3F8..0x514] == cs_test[..], "flush: magic + slots 3/4 at NOR 0x400");
    r.eq(outside(m.cpu.write_log(), &[nv, buf]), vec![], "flush: no stray writes");
    nor.data.borrow_mut()[8..8 + 0x514].copy_from_slice(&wbuf);
    m.wr(cs, &[0u8; 0x11C]);
    call(&mut m, stock::NV_LOAD, &[stock::NV_BLOCK])?;
    r.check(m.rd(cs + 4, 2 * SZ) == cs_test[4..], "flush -> load round trip");
    m.wr(cs, &[0xA5; 0x11C]);
    call(&mut m, stock::NV_FLUSH, &[])?;
    r.check(nor.writes.borrow().last().unwrap().2[0x3F8..0x514] == cs_rom[..], "flush with uninitialized RAM -> ROM defaults");
    m.wr(cs, &cs_test);

    let saved = m.rd(stock::NV_BLOCK, stock::NV_LEN);
    call(&mut m, stock::NV_RESET, &[])?;
    r.check(m.rd(stock::NV_BLOCK, stock::NV_PERSIST) == b.img.rd(stock::NV_DEFAULTS, stock::NV_PERSIST), "reset: NVRAM block = ROM");
    r.check(m.rd(cs, 0x11C) == cs_rom, "reset: F1/F2 = ROM defaults");
    r.eq(outside(m.cpu.write_log(), &[nv, csr]), vec![], "reset: no stray writes");
    m.wr(stock::NV_BLOCK, &saved);
    m.wr(cs, &cs_test);

    // Program resolver
    let mut bad_res = Vec::new();
    for prog in 0..6u8 {
        m.w8(stock::NV_CUR, prog);
        for mode in 0..11u8 {
            m.w8(stock::NV_EXPO, mode);
            let got = call(&mut m, 0xA016_75DA, &[])?;
            let exp = if mode <= 4 || mode == 8 {
                mode as u32
            } else {
                match prog { 1 => 0, 2 => 3, 3 => 2, 4 => 1, _ => 3 }
            };
            if got != exp {
                bad_res.push((mode, prog, got, exp));
            }
        }
    }
    r.eq(bad_res, vec![], "resolver: modes 0..10 x program 0..5");

    // Setter transitions
    let p_set = pat(0x10, 2);
    let trans = |m: &mut Machine, r: &mut Report, label: &str, old: u8, new: u8, cur: &[u8], exp_cur: &[u8], exp_backup: Option<&[u8]>, exp_seq: &[&str]| -> Result<(), String> {
        m.w8(stock::NV_EXPO, old);
        m.w8(SHADOW, old);
        m.wr(stock::NV_CUR, cur);
        let b0 = m.rd(BACKUP, SZ);
        call(m, 0xA016_F9EC, &[new as u32])?;
        let exp_b = exp_backup.unwrap_or(&b0);
        let got = (
            m.rd(stock::NV_CUR, SZ) == exp_cur,
            m.rd(BACKUP, SZ) == exp_b,
            seq(),
            (m.u8(stock::NV_EXPO), m.u8(SHADOW), m.u8(PREV)),
            outside(m.cpu.write_log(), &[nv, csr, sh]),
        );
        r.eq(got, (true, true, exp_seq.to_vec(), (new, new, old), vec![]), format!("setter {label}"));
        Ok(())
    };

    m.wr(BACKUP, &pat(0x70, 0));
    let s3 = slot_rd(&m, 3);
    trans(&mut m, r, "P->F1", 3, 9, &p_set, &s3, Some(&p_set), &["apply"])?;
    let mut edit = slot_rd(&m, 3);
    edit[5] ^= 0xFF;
    let s4 = slot_rd(&m, 4);
    trans(&mut m, r, "F1(edited)->F2", 9, 10, &edit, &s4, None, &["apply"])?;
    r.check(slot_rd(&m, 3) != edit, "F1 edits not written back to slot 3 without save");
    trans(&mut m, r, "F2->M", 10, 0, &s4, &p_set, None, &["apply", "still"])?;
    let s1 = slot_rd(&m, 1);
    trans(&mut m, r, "M->C2", 0, 6, &p_set, &s1, Some(&p_set), &["apply"])?;
    trans(&mut m, r, "C2->F2", 6, 10, &s1, &s4, None, &["apply"])?;
    let mut mark = s4.clone();
    mark[7] ^= 0x55;
    trans(&mut m, r, "F2->playback 8", 10, 8, &mark, &mark, None, &[])?;
    trans(&mut m, r, "playback 8->F2 (no reload)", 8, 10, &mark, &mark, None, &["apply"])?;
    trans(&mut m, r, "F2->Movie", 10, 4, &s4, &p_set, None, &["movie", "apply"])?;
    trans(&mut m, r, "Movie->F2", 4, 10, &p_set, &s4, Some(&p_set), &["apply"])?;
    let s0 = slot_rd(&m, 0);
    trans(&mut m, r, "F2->C1", 10, 5, &s4, &s0, None, &["apply"])?;
    trans(&mut m, r, "C1->P", 5, 3, &s0, &p_set, None, &["apply", "still"])?;
    trans(&mut m, r, "P->C1", 3, 5, &p_set, &s0, Some(&p_set), &["apply"])?;
    let s2 = slot_rd(&m, 2);
    trans(&mut m, r, "C3->Movie", 7, 4, &s2, &p_set, None, &["movie", "apply"])?;
    trans(&mut m, r, "P->M", 3, 0, &p_set, &p_set, None, &["still"])?;
    trans(&mut m, r, "M->Movie", 0, 4, &p_set, &p_set, None, &["movie"])?;
    m.w8(stock::NV_EXPO, 9);
    m.w8(SHADOW, 9);
    m.wr(stock::NV_CUR, &p_set);
    call(&mut m, 0xA016_F9EC, &[9])?;
    r.check(seq().is_empty() && m.rd(stock::NV_CUR, SZ) == p_set, "F1->F1 is a no-op");

    // Slot save / boot reload / compare
    for cursor in 0..5usize {
        m.w16(CUSTOM_STATE, cursor as u16);
        m.w16(CUSTOM_STATE + 2, 0);
        let cur = pat((0x80 + cursor * 8) as u8, (1 + cursor % 4) as u8);
        m.wr(stock::NV_CUR, &cur);
        let others: Vec<Vec<u8>> = (0..5).filter(|&i| i != cursor).map(|i| slot_rd(&m, i)).collect();
        call(&mut m, 0xA005_9A4A, &[4, 0x12])?;
        let after_others: Vec<Vec<u8>> = (0..5).filter(|&i| i != cursor).map(|i| slot_rd(&m, i)).collect();
        r.check(
            slot_rd(&m, cursor) == cur && after_others == others && outside(m.cpu.write_log(), &[nv, csr]).is_empty(),
            format!("save cursor {cursor} -> slot {cursor} only"),
        );
    }
    for mode in 0..11u32 {
        m.w8(stock::NV_EXPO, mode as u8);
        m.wr(stock::NV_CUR, &p_set);
        call(&mut m, b.sym("expo_boot_reload"), &[])?;
        let exp = slot_of(mode).map_or_else(|| p_set.clone(), |i| slot_rd(&m, i));
        r.check(
            m.rd(stock::NV_CUR, SZ) == exp && outside(m.cpu.write_log(), &[nv, csr]).is_empty(),
            format!("boot reload mode {mode} -> slot {:?}", slot_of(mode)),
        );
    }
    for (i, mode) in [5u8, 6, 7, 9, 10].into_iter().enumerate() {
        m.w8(stock::NV_EXPO, mode);
        let s = slot_rd(&m, i);
        m.wr(stock::NV_CUR, &s);
        let same = call(&mut m, 0xA016_7618, &[i as u32])?;
        let mut d = s;
        d[0x1C] ^= 1;
        m.wr(stock::NV_CUR, &d);
        let diff = call(&mut m, 0xA016_7618, &[i as u32])?;
        r.check(same == 1 && diff == 0 && m.cpu.write_log().is_empty(), format!("compare slot {i}: same={same} diff={diff}"));
    }
    let ptrs: Vec<u32> = [0u32, 1, 2, 3, 4, 5, 0xFF]
        .into_iter()
        .map(|i| call(&mut m, b.sym("slot_ptr"), &[i]))
        .collect::<Result<_, _>>()?;
    r.eq(
        ptrs,
        vec![SLOTS, SLOTS + SZ as u32, SLOTS + 2 * SZ as u32, cs + 4, cs + 4 + SZ as u32, cs + 4, cs + 4],
        "slot_ptr(0..4) exact; out-of-range indices 5 & 0xff clamp to cs.slot[0]",
    );
    let mut oob_save_clean = true;
    for bad in [5u32, 0xFF] {
        call(&mut m, b.sym("slot_save"), &[bad])?;
        oob_save_clean &= m.cpu.write_log().is_empty();
    }
    r.check(oob_save_clean, "slot_save(5 / 0xff) performs zero writes");
    m.wr(stock::NV_CUR, &slot_rd(&m, 3));
    let oob_cmp = call(&mut m, 0xA016_7618, &[5])?;
    r.check(
        oob_cmp == 1 && m.cpu.write_log().is_empty(),
        format!("compare slot 5 stays in bounds via clamped slot_ptr (ret={oob_cmp})"),
    );

    // LV widget 0x2E
    for mode in 0..11u32 {
        m.w8(stock::NV_EXPO, mode as u8);
        m.w16(0xA07F_A63E, 0);
        m.run_slice(0xA016_A7FC, 0xA016_A81A, 10_000, STACK.1, &[(4, 0x1234)])?;
        r.eq(m.u16(0xA07F_A63E) == 0x1234, slot_of(mode).is_some(), format!("widget visible in mode {mode}"));
    }
    for mode in 0..11u32 {
        m.w8(stock::NV_EXPO, mode as u8);
        let si = slot_of(mode);
        let mods: &[bool] = if si.is_some() { &[false, true] } else { &[false] };
        for &modified in mods {
            let mut cur = si.map_or_else(|| p_set.clone(), |i| slot_rd(&m, i));
            if modified {
                cur[0x1C] ^= 1;
            }
            m.wr(stock::NV_CUR, &cur);
            call(&mut m, 0xA016_BAE4, &[])?;
            let ids: Vec<u32> = calls.borrow().iter().filter(|c| c.0 == "icon").map(|c| c.1[0]).collect();
            let mut exp = Vec::new();
            if si.is_some() {
                exp.push(match mode {
                    5 => 0xB7,
                    6 => 0xB8,
                    7 => 0xB9,
                    9 => widget_icon + si4,
                    _ => widget_icon + si5,
                });
                if modified {
                    exp.push(0x2F);
                }
            }
            r.check(
                ids.get(1..).unwrap_or(&[]) == exp.as_slice() && m.cpu.write_log().is_empty(),
                format!("widget draw mode {mode} modified={modified}: icons {ids:#x?}"),
            );
        }
    }
    for mode in 0..11u32 {
        m.w8(stock::NV_EXPO, mode as u8);
        call(&mut m, 0xA016_987A, &[])?;
        let got: Vec<&str> = calls.borrow().iter().map(|c| c.0).filter(|&n| n == "still" || n == "movie").collect();
        let exp: Vec<&str> = match mode { 4 => vec!["movie"], 8 => vec![], _ => vec!["still"] };
        r.eq(got, exp, format!("still-mode apply switch, mode {mode}"));
    }

    // Menu 0x05
    let t = call(&mut m, stock::GET_MENU_TABLE, &[5])?;
    r.eq(t, b.sym("m_5"), "Get_Menu_Table(0x05) -> m_5");
    let mut rows = Vec::new();
    for k in 0..16u32 {
        let w: [u32; 8] = core::array::from_fn(|i| m.u32(t + 32 * k + 4 * i as u32));
        if w.iter().all(|&x| x == 0) {
            break;
        }
        rows.push(w);
    }
    let st: Vec<[u32; 8]> = (0..8u32)
        .map(|k| core::array::from_fn(|i| b.img.u32(0xA07E_C8AC + 32 * k + 4 * i as u32)))
        .collect();
    r.check(rows.len() >= 8 && rows[..8] == st[..], "menu 0x05 rows 0..7 = stock");
    let c3 = st[7];
    for (k, (si, v, sid)) in [(si4, 9u32, "SID_F1"), (si5, 10u32, "SID_F2")].into_iter().enumerate() {
        let exp = [
            c3[0],
            0x0100_0000 | (b.cat("ICON_LABEL_L") + si),
            ((mode_icon + si) << 16) | (b.cat("ICON_LABEL_S") + si),
            (b.cat(sid) - 1) << 16,
            v,
            c3[5],
            c3[6],
            c3[7],
        ];
        r.eq(rows.get(8 + k).copied(), Some(exp), format!("menu 0x05 row {} (mode {v})", 8 + k));
    }
    r.eq(rows.len(), 10, "menu 0x05 row count");

    // MODE screen 23
    let ms = b.sym("mode_strip");
    let cells: Vec<(u32, u16, u16)> = (0..11u32)
        .map(|k| (m.u32(ms + 4 + 8 * k), m.u16(ms + 8 + 8 * k), m.u16(ms + 10 + 8 * k)))
        .collect();
    let exp_cells = vec![
        (4, 0xAF, 0), (0, 0xB0, 0), (1, 0xB1, 0), (2, 0xB2, 0), (3, 0xB3, 0),
        (5, 0xB4, 0), (6, 0xB5, 0), (7, 0xB6, 0),
        (9, (mode_icon + si4) as u16, 0), (10, (mode_icon + si5) as u16, 0), (0, 0, 0),
    ];
    r.eq(cells, exp_cells.clone(), "MODE strip cells");
    m.w8(stock::NV_EXPO, 9);
    call(&mut m, 0xA005_21DC, &[])?;
    let xs: Vec<u32> = calls.borrow().iter().filter(|c| c.0 == "blit").map(|c| c.1[1]).collect();
    let ic: Vec<u32> = calls.borrow().iter().filter(|c| c.0 == "icon").map(|c| c.1[0]).collect();
    let exp_ic: Vec<u32> = exp_cells[..10].iter().map(|c| c.1 as u32).collect();
    r.eq(ic, exp_ic, "MODE draw fetches 10 icons in ring order");
    let exp_xs: Vec<u32> = (0..10).map(|k| 24 + 60 * k).collect();
    r.check(xs == exp_xs && xs.iter().copied().max().unwrap_or(0) + 51 <= 640, format!("MODE icon x {xs:?}"));
    r.eq(rects.borrow().clone(), vec![[20 + 60 * 8, 0x1B0, 60, 0x2E]], "MODE highlight on the F1 cell");
    r.eq(m.cpu.write_log().to_vec(), vec![], "MODE draw: no stray writes");

    let key = |m: &mut Machine, k: u32, cursor: u16, mode: u8| -> Result<(u16, Vec<u32>), String> {
        m.w16(ms, cursor);
        m.w8(stock::NV_EXPO, mode);
        m.w8(SHADOW, mode);
        let cur = if mode == 10 { slot_rd(m, 4) } else { p_set.clone() };
        m.wr(stock::NV_CUR, &cur);
        call(m, 0xA005_22AA, &[k])?;
        let ps: Vec<u32> = calls.borrow().iter().filter(|c| c.0 == "prog_set").map(|c| c.1[0]).collect();
        Ok((m.u16(ms), ps))
    };
    r.eq(key(&mut m, 0x40, 7, 7)?, (8, vec![9]), "MODE key 0x40: C3 -> F1");
    r.eq(key(&mut m, 0x40, 8, 9)?, (9, vec![10]), "MODE key 0x40: F1 -> F2");
    r.eq(key(&mut m, 0x40, 9, 10)?, (0, vec![4]), "MODE key 0x40: F2 -> Movie (wrap)");
    r.eq(key(&mut m, 0x20, 0, 4)?, (9, vec![10]), "MODE key 0x20: Movie -> F2 (wrap)");
    r.eq(outside(m.cpu.write_log(), &[nv, csr, sh, (ms, ms + 2)]), vec![], "MODE keys: no stray writes");

    m.w8(stock::NV_EXPO, 10);
    m.w16(ms, 0);
    m.cpu.clear_write_log();
    m.run_slice(0xA005_238E, 0xA005_23B2, 10_000, STACK.1, &[])?;
    r.check(
        m.u16(ms) == 9 && outside(m.cpu.write_log(), &[(ms, ms + 2)]).is_empty(),
        format!("MODE open: cursor search finds F2 -> {}", m.u16(ms)),
    );

    // Custom Mode Setting screen 31
    let icons5 = [0xB4u16, 0xB5, 0xB6, (mode_icon + si4) as u16, (mode_icon + si5) as u16];
    let got_icons5: [u16; 5] = core::array::from_fn(|k| b.img.u16(b.sym("custom_icons") + (2 * k) as u32));
    r.eq(got_icons5, icons5, "Custom Mode Setting icon table");
    for cursor in 0..5u16 {
        m.w16(CUSTOM_STATE, cursor);
        m.w16(CUSTOM_STATE + 2, 0);
        call(&mut m, 0xA005_9790, &[])?;
        let ids: Vec<u32> = calls.borrow().iter().filter(|c| c.0 == "icon").map(|c| c.1[0]).collect();
        let bl: Vec<u32> = calls.borrow().iter().filter(|c| c.0 == "blit").map(|c| c.1[1]).collect();
        let mut exp_ids = vec![0x96u32, 0x97];
        exp_ids.extend(icons5.iter().map(|&x| x as u32));
        let mut exp_bl = vec![1 + 120 * cursor as u32, 146 + 120 * cursor as u32];
        exp_bl.extend((0..5u32).map(|k| 54 + 120 * k));
        let ok = rects.borrow().as_slice() == [[20 + 120 * cursor, 0xBB, 120, 0x38]]
            && ids == exp_ids
            && bl == exp_bl
            && bl[..2].iter().all(|&x| x + 13 <= 640)
            && bl[2..].iter().all(|&x| x + 51 <= 640)
            && m.cpu.write_log().is_empty();
        r.check(ok, format!("Custom Mode Setting cursor {cursor}: box {:?} blit x {bl:?}", rects.borrow()));
    }
    m.w16(CUSTOM_STATE, 4);
    m.w16(CUSTOM_STATE + 2, 1);
    call(&mut m, 0xA005_9790, &[])?;
    let ids: Vec<u32> = calls.borrow().iter().filter(|c| c.0 == "icon").map(|c| c.1[0]).collect();
    r.eq(ids, vec![mode_icon + si5], "confirm view shows the F2 icon");
    m.cpu.stop_write_log();
    Ok(true)
}
