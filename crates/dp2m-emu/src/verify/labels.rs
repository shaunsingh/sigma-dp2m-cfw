//! labels suite (160 checks): runtime-rendered BP4 label icons across all 6 classes,
//! arena allocation & reset, stray-write hygiene, QS strip routing, LV status/info/menu icons.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::BufWriter;
use std::path::Path;
use std::rc::Rc;

use crate::image::{Image, write_rgb_png};
use crate::machine::{Machine, SCRATCH, STACK};
use crate::stock;

use super::{Build, Report};

const TOKEN_VA: u32 = SCRATCH + 0x10000;
const GET_CM: u32 = stock::GET_CM | 1;
const GET_ISO: u32 = 0xA016_F5AF;
const LABELS_BOOT_SITE: u32 = 0xA016_89BE;
const LABEL_MAGIC: u32 = 0x4C41_424C;

const DIMS: &[(&str, usize, usize, &[u32])] = &[
    ("L", 110, 70, &[0x10E, 0x10F, 0x115, 0x113]),
    ("S", 54, 42, &[0x159, 0x15A, 0x160]),
    ("OSD", 76, 26, &[0x27, 0x28, 0x2E]),
    ("MODE", 51, 31, &[0xB4, 0xB6]),
    ("WIDGET", 35, 20, &[0xB7, 0xB9]),
    ("QS", 42, 34, &[0xA1, 0x9B]),
];

const NAMES: &[&str] = &[
    "Velvia 100", "FOV Classic Yellow", "Kodak Tri-X 400", "Teal & Orange", "Cinema", "Portra 800 +2",
    "Mono Orange #21", "SD Custom LUT", "Rollei Retro 80S", "Fujicolor C200", "Zebras",
    "Fill -0.5 (Contrast)",
];
const QS_ITEMS: &[&str] = &["Print Paper", "Film Engine", "Film Grain", "Halation", "X3 Fill Light"];
const TOKENS: &[(&str, &str)] = &[("Red", "F1"), ("Yellow", "F2"), ("White", "M2"), ("Black", "P")];

struct RenderedIcon {
    res: u32,
    ptr: u32,
    w: usize,
    h: usize,
    px: Vec<u8>,
}

#[derive(Default)]
struct IconBand {
    stock: Vec<RenderedIcon>,
    labels: Vec<RenderedIcon>,
}

fn write_sheet(dir: &Path, img: &Image, bands: &[IconBand]) -> Result<(), String> {
    // Stock references on the left; rendered samples on the right. Each band
    // is one class, ordered as in DIMS, with native pixels sampled at 2x.
    const STOCK_COLS: usize = 4;
    const LABEL_COLS: usize = 8;
    const PAD: usize = 8;
    const SCALE: usize = 2;
    let width = (STOCK_COLS + LABEL_COLS) * (DIMS[0].1 * SCALE + PAD) + 3 * PAD;
    let band_height = |h: usize| 2 * (h * SCALE + PAD) + PAD;
    let height = DIMS.iter().map(|&(_, _, h, _)| band_height(h)).sum::<usize>() + PAD;
    let mut pixels = vec![[24, 24, 24]; width * height];
    let mut palette = [[0; 3]; stock::CLUT_N];
    for (rgb, ayuv) in palette.iter_mut().zip(img.rd(stock::CLUT, 4 * stock::CLUT_N).as_chunks::<4>().0) {
        let (y, cb, cr) = (f64::from(ayuv[1]), f64::from(ayuv[2]) - 128.0, f64::from(ayuv[3]) - 128.0);
        *rgb = [y + 1.402 * cr, y - 0.344 * cb - 0.714 * cr, y + 1.772 * cb]
            .map(|v| v.clamp(0.0, 255.0) as u8);
    }
    let mut y = PAD;
    for (band, &(_, w, h, _)) in bands.iter().zip(DIMS) {
        let (cw, ch) = (SCALE * w + PAD, SCALE * h + PAD);
        let split = PAD + STOCK_COLS * cw;
        for row in pixels.chunks_exact_mut(width).skip(y).take(band_height(h) - PAD) {
            row[PAD..split].fill([57, 47, 35]); // mark the stock-reference area
            row[split + PAD..split + PAD + LABEL_COLS * cw].fill([37, 40, 46]);
        }
        for (tiles, cols, x) in [(&band.stock, STOCK_COLS, PAD), (&band.labels, LABEL_COLS, split + PAD)] {
            for (k, tile) in tiles.iter().enumerate() {
                let (tx, ty) = (x + k % cols * cw + PAD / 2, y + k / cols * ch + PAD / 2);
                for dy in 0..SCALE * tile.h.min(h) {
                    for dx in 0..SCALE * tile.w.min(w) {
                        let v = tile.px[dy / SCALE * tile.w + dx / SCALE];
                        pixels[(ty + dy) * width + tx + dx] = match v {
                            0 => [52, 58, 70],
                            1..=16 => palette[usize::from(stock::ICON_HI + 1 - v)],
                            _ => palette[usize::from(v).min(palette.len() - 1)],
                        };
                    }
                }
            }
        }
        y += band_height(h);
    }
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = dir.join("labels.png");
    let write = || {
        use std::io::Write;
        let mut file = BufWriter::new(fs::File::create(&path)?);
        write_rgb_png(&mut file, width as u32, height as u32, &pixels)?;
        file.flush()
    };
    write().map_err(|e| format!("write {}: {e}", path.display()))
}

fn render_icon(m: &mut Machine, id: u32) -> Result<Option<RenderedIcon>, String> {
    let res = m.call_ext(stock::ICON_GET, &[id], &[], 100_000_000)?;
    if res == 0 {
        return Ok(None);
    }
    let ptr = m.u32(res);
    let w = m.u16(res + 4) as usize;
    let h = m.u16(res + 6) as usize;
    let px = m.rd(ptr, w * h);
    Ok(Some(RenderedIcon { res, ptr, w, h, px }))
}

fn outline_gaps(px: &[u8], w: usize, h: usize) -> usize {
    let mut gaps = 0;
    for y in 0..h {
        for x in 0..w {
            let v = px[y * w + x];
            if v > 0 && v <= 12 {
                for (dy, dx) in [(0isize, 1isize), (0, -1), (1, 0), (-1, 0)] {
                    let ny = y as isize + dy;
                    let nx = x as isize + dx;
                    let nb = if ny >= 0 && ny < h as isize && nx >= 0 && nx < w as isize {
                        px[ny as usize * w + nx as usize]
                    } else {
                        0
                    };
                    if nb == 0 {
                        gaps += 1;
                    }
                }
            }
        }
    }
    gaps
}

pub fn run(b: &Build, r: &mut Report) -> Result<bool, String> {
    run_with_artifacts(b, r, None)
}

pub(crate) fn run_with_artifacts(b: &Build, r: &mut Report, artifacts: Option<&Path>) -> Result<bool, String> {
    if !b.has_all(&["label_icon", "cache", "arena", "lv_status_cm"]) {
        return Ok(false);
    }
    for (site, sym) in [
        (stock::ICON_GET + 2, "icon_hook"),
        (stock::QS_STRIP_CHECK, "qs_strip_hook"),
        (0xA004_3EAA, "qs_scroll_clamp"),
        (0xA004_3D06, "qs_sel_clamp"),
    ] {
        if b.syms.contains_key(sym) {
            r.eq(b.img.bl_target(site), Some(b.sym(sym)), format!("BL @ {site:#x} -> {sym}"));
        }
    }
    r.eq(b.img.u16(0xA004_D51A), 0xE001, "0xA004D51A stays stock (0xA004D4F0 returns n on no-match)");

    let mut m = Machine::new(&b.img, 0);
    let arena = b.sym_info("arena");
    let (a0, a1) = (arena.va(), arena.va() + arena.size);
    let bss_lo = b.sym("cache");
    let bss_hi = b
        .syms
        .values()
        .filter(|s| (0xA1C0_0000..0xA1E0_0000).contains(&s.va()))
        .map(|s| s.va() + s.size)
        .max()
        .unwrap_or(bss_lo);
    let bss = (bss_lo, bss_hi);

    // Seed deterministic garbage across payload RAM (never zeroed on camera)
    let mut rng = 0x1357_9BDFu32;
    let mut garbage = vec![0u8; 0x6000];
    for g in &mut garbage {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        *g = rng as u8;
    }
    m.wr(bss.0, &garbage);

    let table = b.sym("dp2m_strings");
    let mut strings = b.catalog.strings.clone();
    for (k, &(orig, tok)) in TOKENS.iter().enumerate() {
        let i = strings.iter().position(|s| s == orig).ok_or("missing token string")?;
        let mut tb = tok.as_bytes().to_vec();
        tb.push(0);
        m.wr(TOKEN_VA + 0x40 * k as u32, &tb);
        m.w32(table + 4 * i as u32, TOKEN_VA + 0x40 * k as u32);
        strings[i] = tok.to_owned();
    }
    let str_idx = |s: &str| strings.iter().position(|x| x == s).unwrap() as u32;

    let allowed = [
        STACK,
        bss,
        (stock::LINE_BUF, stock::LINE_BUF + 0x28000),
        (stock::ICON_RESULT, stock::ICON_RESULT + 8),
        (stock::FONT_SCALE, stock::FONT_SCALE + 8),
        (stock::OSD_DIRTY, stock::OSD_DIRTY + 8),
        stock::GLYPH_RASTER,
        stock::FONT_WORK,
    ];

    m.cpu.start_write_log(&allowed);
    let mut bad_stock = Vec::new();
    let mut bands = artifacts.map(|_| DIMS.iter().map(|_| IconBand::default()).collect::<Vec<_>>());
    for (class, &(_, _, _, ids)) in DIMS.iter().enumerate() {
        for &id in ids {
            let x = render_icon(&mut m, id)?;
            if !matches!(x, Some(ref ic) if ic.res == stock::ICON_RESULT) {
                bad_stock.push(id);
            }
            if let (Some(bands), Some(icon)) = (&mut bands, x) {
                bands[class].stock.push(icon);
            }
        }
    }
    r.eq(bad_stock, vec![], "stock icon ids pass through the hooked getter");
    m.w32(stock::FONT_SCALE, 5);
    m.w32(stock::FONT_SCALE + 4, 7);
    let dirty0 = m.rd(stock::OSD_DIRTY, 16);

    for (class, &(cls_name, w, h, _)) in DIMS.iter().enumerate() {
        let base = b.cat(&format!("ICON_LABEL_{cls_name}"));
        let list: Vec<&str> = match cls_name {
            "MODE" | "WIDGET" => TOKENS.iter().map(|&(_, t)| t).chain(std::iter::once("Cinema")).collect(),
            "QS" => QS_ITEMS.to_vec(),
            _ => NAMES.to_vec(),
        };
        for name in list {
            let id = base + str_idx(name);
            let x = render_icon(&mut m, id)?;
            let x2 = render_icon(&mut m, id)?;
            if !r.check(x.is_some(), format!("{cls_name} '{name}' ({id:#x}) renders")) {
                continue;
            }
            let RenderedIcon { res, ptr: p, w: tw, h: th, px } = x.unwrap();
            let p2 = x2.map_or(0, |t| t.ptr);
            let mut has1 = false;
            let mut has16 = false;
            let mut in_range = true;
            for &v in &px {
                if v == 1 { has1 = true; }
                if v == 16 { has16 = true; }
                if v > 16 { in_range = false; }
            }
            let gap = outline_gaps(&px, tw, th);
            let ok = res == stock::ICON_RESULT
                && tw == w
                && th == h
                && p >= a0
                && p + (w * h) as u32 <= a1
                && p2 == p
                && in_range
                && has1
                && has16
                && gap == 0;
            r.check(
                ok,
                format!("{cls_name} '{name}': {w}x{h} tile in arena, cached, levels 0..16 with fill & outline, outline gaps {gap}"),
            );
            if let Some(bands) = &mut bands {
                bands[class].labels.push(RenderedIcon { res, ptr: p, w: tw, h: th, px });
            }
        }
    }
    r.eq((m.u32(stock::FONT_SCALE), m.u32(stock::FONT_SCALE + 4)), (5, 7), "font scale restored");
    r.check(m.rd(stock::OSD_DIRTY, 16) == dirty0, "OSD dirty rects unchanged");

    r.check(
        b.stock.u32(0xA07F_BED0) == 0xE281_101F
            && b.stock.u32(0xA07F_BED4) == 0xE1A0_12A1
            && b.stock.u32(0xA07F_BED8) == 0xEE07_0F3A
            && b.stock.u32(0xA07F_BEE8) == 0xEE07_1F9A
            && b.stock.u32(0xA07F_BEEC) == 0xE8BD_0003,
        "stock SWI 12 cleans by address, drains the write buffer, and restores r0/r1",
    );
    let mut cache_m = Machine::new(&b.img, 0);
    cache_m.w32(b.sym("cache"), LABEL_MAGIC);
    cache_m.w32(b.sym("cache") + 4, 4u32.wrapping_sub(a0) & 31);
    cache_m.cpu.start_cache_log();
    let cold = render_icon(&mut cache_m, b.cat("ICON_LABEL_L"))?.ok_or("cold cache-test label did not render")?;
    let cached = render_icon(&mut cache_m, b.cat("ICON_LABEL_L"))?.ok_or("cached cache-test label did not render")?;
    let cache_log = cache_m.cpu.stop_cache_log();
    r.eq(cold.ptr & 31, 4, "cache-range test tile starts four bytes into a cache line");
    r.eq(
        cache_log,
        vec![(12, cold.ptr & !31, (cold.w * cold.h) as u32 + (cold.ptr & 31))],
        format!("cold label cleans its aligned, extended tile range; cached lookup issues no cache SWI (same tile: {})", cold.ptr == cached.ptr),
    );

    let log = m.cpu.stop_write_log();
    let stray: Vec<u32> = log.iter().map(|&(a, _)| a).collect::<BTreeSet<_>>().into_iter().take(8).collect();
    r.eq(stray, vec![], "label rendering writes only allowed ranges");

    let onscreen = |mm: &mut Machine| -> Result<Vec<u8>, String> {
        mm.w32(stock::FONT_SCALE, 1);
        mm.w32(stock::FONT_SCALE + 4, 1);
        mm.wr(stock::OSD, &vec![0u8; 640 * 480]);
        mm.text("ISO 100 F2.8", 24, 0x22, [20, 200, 600, 60], 0)?;
        Ok(mm.rd(stock::OSD, 640 * 480))
    };
    let after = onscreen(&mut m)?;
    let mut fresh_m = Machine::new(&b.img, 0);
    let fresh = onscreen(&mut fresh_m)?;
    r.check(after == fresh && after.iter().any(|&b| b != 0), "stock text drawn after label renders matches a fresh camera");

    if b.syms.contains_key("labels_boot") {
        r.eq(b.img.bl_target(LABELS_BOOT_SITE), Some(b.sym("labels_boot")), format!("BL @ {LABELS_BOOT_SITE:#x} -> labels_boot"));
        let cache = b.sym("cache");
        if let Some(orig) = b.stock.bl_target(LABELS_BOOT_SITE) {
            m.stub(orig, None);
        }
        let mut seed = Vec::with_capacity(72);
        seed.extend_from_slice(&LABEL_MAGIC.to_be_bytes());
        seed.extend_from_slice(&0x1234u32.to_be_bytes());
        for _ in 0..16 {
            seed.extend_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        }
        m.wr(cache, &seed);
        m.call(b.sym("labels_boot"), &[])?;
        let words: Vec<u32> = (0..18u32).map(|k| m.u32(cache + 4 * k)).collect();
        r.check(
            words[0] == LABEL_MAGIC && words[1..].iter().all(|&w| w == 0),
            "labels_boot resets a cache that kept its magic over a power cycle",
        );
    }

    m.cpu.start_write_log(&allowed);
    m.w32(b.sym("cache") + 4, arena.size - 100);
    let x = render_icon(&mut m, b.cat("ICON_LABEL_L") + str_idx("Velvia 100"))?;
    r.check(matches!(x, Some(ref ic) if ic.ptr == a0), "arena full -> reset & re-render at the arena base");
    let mut bad_inv = Vec::new();
    for id in [0x1C01u32, b.cat("ICON_LABEL_L") + strings.len() as u32, 0x1FFF] {
        if m.call_ext(stock::ICON_GET, &[id], &[], 100_000_000)? != 0 {
            bad_inv.push(id);
        }
    }
    r.eq(bad_inv, vec![], "invalid label ids return NULL");
    let log = m.cpu.stop_write_log();
    let stray: Vec<u32> = log.iter().map(|&(a, _)| a).collect::<BTreeSet<_>>().into_iter().take(8).collect();
    r.eq(stray, vec![], "arena reset writes only allowed ranges");

    if let (Some(dir), Some(bands)) = (artifacts, bands) {
        write_sheet(dir, &b.stock, &bands)?;
    }

    // UI checks: QS strip routing & LV icons
    m.load_nv_defaults();
    m.stub(stock::STRIP_BG_CLEAR, None);
    let in_arena = |p: u32| p >= a0 && p < a1;
    let blits: Rc<RefCell<Vec<[u32; 7]>>> = Rc::new(RefCell::new(Vec::new()));
    let fills: Rc<RefCell<Vec<[u16; 4]>>> = Rc::new(RefCell::new(Vec::new()));
    let null_calls = Rc::new(Cell::new(0u32));
    {
        let bl = blits.clone();
        m.watch(stock::BLIT, move |c| {
            bl.borrow_mut().push([
                c.r[0], c.r[1], c.r[2], c.r[3],
                c.r32(c.r[13]), c.r32(c.r[13] + 4), c.r32(c.r[13] + 8),
            ]);
        });
    }
    {
        let fl = fills.clone();
        m.watch(stock::FILL_BOX, move |c| {
            let p = c.r[0];
            fl.borrow_mut().push(core::array::from_fn(|k| c.r16_be(p + (2 * k) as u32)));
        });
    }
    {
        let nc = null_calls.clone();
        m.watch(0xA004_3D62, move |c| {
            if c.r[0] == 0 {
                nc.set(nc.get() + 1);
            }
        });
    }

    let mut slots = BTreeMap::new();
    for page in 0..2u32 {
        for s in 0..4u32 {
            let rows = m.call(stock::QS_SLOT_ROWS, &[page, s])?;
            if rows != 0 {
                slots.insert((page, s), (rows, m.u32(rows + 0x14)));
            }
        }
    }
    let cm_slot = slots.iter().find(|&(_, &(_, g))| g == GET_CM).map(|(&k, _)| k);
    if !r.check(cm_slot.is_some(), format!("QS color mode slot found in {} slots", slots.len())) {
        return Ok(true);
    }
    let cm_slot = cm_slot.unwrap();
    let rows = slots[&cm_slot].0;
    let n_rows = m.call(stock::MENU_ROW_COUNT, &[rows])? as usize;
    let values: Vec<u32> = (0..n_rows).map(|i| m.u32(rows + 32 * i as u32 + 0x10)).collect();

    let strip = |m: &mut Machine, slot: (u32, u32), cm: u32| -> Result<(u16, Vec<[u32; 7]>), String> {
        m.w16(stock::QS_STATE + 4, slot.0 as u16);
        m.w16(stock::QS_STATE + 8 + 2 * slot.0, slot.1 as u16);
        match slots[&slot].1 {
            GET_CM => m.w8(stock::NV_CM, cm as u8),
            GET_ISO => m.w8(stock::NV_ISO, cm as u8),
            _ => {}
        }
        m.wr(stock::OSD, &vec![0u8; 640 * 480]);
        blits.borrow_mut().clear();
        fills.borrow_mut().clear();
        null_calls.set(0);
        m.call_ext(stock::QS_STRIP, &[], &[], 200_000_000)?;
        Ok((m.u16(stock::QS_STATE + 6), blits.borrow().clone()))
    };

    m.w16(stock::QS_STATE + 6, 0);
    let mid_idx = values.iter().position(|&v| v == 19).unwrap_or(n_rows / 2);
    let sels: BTreeSet<usize> = [0, 7.min(n_rows - 1), 8.min(n_rows - 1), mid_idx, n_rows - 1].into_iter().collect();
    let exp_tiles = n_rows.min(8);
    for sel in sels {
        let (scroll, bl) = strip(&mut m, cm_slot, values[sel])?;
        let tiles = bl.iter().filter(|x| x[3] == 54).count();
        r.check(
            scroll as usize <= sel
                && sel <= scroll as usize + 7
                && scroll as usize + 8 <= n_rows
                && tiles == exp_tiles,
            format!("QS strip cm {} (row {sel}/{n_rows}): scroll {scroll}, {tiles} tiles", values[sel]),
        );
    }

    // Every scroll-routed table must have eight drawable rows and a null getter after its terminator.
    for (&slot, &(initial_rows, getter)) in slots.iter().filter(|&(_, &(_, g))| g == GET_CM || g == GET_ISO) {
        for step in if getter == GET_ISO { &[0u8, 1][..] } else { &[0u8][..] } {
            m.w8(stock::NV_ISO_STEP, *step);
            let rows = if getter == GET_ISO { m.call(stock::QS_SLOT_ROWS, &[slot.0, slot.1])? } else { initial_rows };
            let n = m.call(stock::MENU_ROW_COUNT, &[rows])? as usize;
            r.check(
                n >= 8 && m.u32(rows + 32 * (n as u32 - 1) + 0x14) != 0 && m.u32(rows + 32 * n as u32 + 0x14) == 0,
                format!("QS slot {slot:?} step {step}: {n} rows, valid last getter and null terminator"),
            );
        }
    }

    // Colour Mode leaves its long-list scroll in shared state. Focusing ISO must clamp it before
    // the stock eight-iteration draw loop reaches the shorter table's null getter (blx 0).
    let iso_slot = slots.iter().find(|&(_, &(_, g))| g == GET_ISO).map(|(&k, _)| k);
    if !r.check(iso_slot.is_some(), "QS ISO slot found") {
        return Ok(true);
    }
    let iso_slot = iso_slot.unwrap();
    for (step, sel) in [(0u8, 2usize), (1, 19)] {
        m.w8(stock::NV_ISO_STEP, step);
        m.w16(stock::QS_STATE + 6, 0);
        let (cm_scroll, _) = strip(&mut m, cm_slot, values[n_rows - 1])?;
        let iso_rows = m.call(stock::QS_SLOT_ROWS, &[iso_slot.0, iso_slot.1])?;
        let iso_n = m.call(stock::MENU_ROW_COUNT, &[iso_rows])? as usize;
        let iso_value = m.u32(iso_rows + 32 * sel as u32 + 0x10);
        let result = strip(&mut m, iso_slot, iso_value);
        let ok = match result {
            Ok((scroll, bl)) => {
                let tiles = bl.iter().filter(|x| x[3] == 54).count();
                scroll as usize <= sel
                    && sel <= scroll as usize + 7
                    && scroll as usize + 8 <= iso_n
                    && tiles == 8
            }
            Err(_) => false,
        };
        r.check(
            cm_scroll as usize + 8 == n_rows && ok,
            format!("QS Colour Mode last row -> ISO step {step} row {sel}/{iso_n} stays in range"),
        );
    }

    m.w8(stock::NV_ISO_STEP, 0);
    let iso_rows = m.call(stock::QS_SLOT_ROWS, &[iso_slot.0, iso_slot.1])?;
    let iso_value = m.u32(iso_rows + 32 * 2 + 0x10);
    let mut stale_ok = true;
    for stale in [2u16, 3, u16::MAX] {
        m.w16(stock::QS_STATE + 6, stale);
        match strip(&mut m, iso_slot, iso_value) {
            Ok((scroll, bl)) => {
                stale_ok &= scroll <= 1 && bl.iter().filter(|x| x[3] == 54).count() == 8;
            }
            Err(_) => stale_ok = false,
        }
    }
    r.check(stale_ok, "QS ISO strip clamps stale scroll values 2, 3, and 0xffff");

    // Narrow QS strip clamp at 0xA0043D06 (`qs_sel_clamp`): stock 0xA004D4F0 stays unmodified
    // (returning row count n on no-match for callers like QS slot icons at 0xA00439C4), while
    // the strip clamps sel >= n to n-1 so the 8-tile loop never dereferences the terminator.
    const MENU_SEL: u32 = 0xA004_D4F0;
    const QS_SLOT_ICONS: u32 = 0xA004_3976;
    let qs_sel_clamp = b.sym("qs_sel_clamp");
    m.w8(stock::NV_CM, values[mid_idx] as u8);
    r.eq(m.call(MENU_SEL, &[rows])?, mid_idx as u32, "0xA004D4F0 matches valid Colour Mode row");
    r.eq(m.call(MENU_SEL, &[0])?, u32::MAX, "0xA004D4F0(NULL) keeps stock -1 return");
    r.eq(
        (m.call(qs_sel_clamp, &[0])?, m.call(qs_sel_clamp, &[rows])?),
        (u32::MAX, mid_idx as u32),
        "qs_sel_clamp preserves -1 on NULL and passes through valid selection",
    );

    // Valid selections (first, middle, last row) keep exact stock scroll and highlight the right tile.
    for sel in [0usize, mid_idx, n_rows - 1] {
        let mut ok = true;
        for init_scroll in [0u16, (n_rows - 8) as u16] {
            m.w16(stock::QS_STATE + 6, init_scroll);
            let (scroll, bl) = strip(&mut m, cm_slot, values[sel])?;
            let tiles: Vec<[u32; 7]> = bl.into_iter().filter(|x| x[3] == 54).collect();
            let exp_scroll = if sel < init_scroll as usize {
                sel as u16
            } else if init_scroll as usize + 7 < sel {
                (sel - 7) as u16
            } else {
                init_scroll
            };
            let hl = fills.borrow();
            let tile_idx = sel.wrapping_sub(scroll as usize);
            ok &= scroll == exp_scroll
                && tiles.len() == 8
                && null_calls.get() == 0
                && hl.len() == 1
                && tile_idx < tiles.len()
                && hl[0][0] as u32 + 2 == tiles[tile_idx][1];
        }
        r.check(
            ok,
            format!("QS strip valid cm {} (row {sel}/{n_rows}) keeps stock scroll and highlights exact tile", values[sel]),
        );
    }

    // No-match values (out-of-range NVRAM and disabled catalog film absent from m_9) leave
    // 0xA004D4F0 returning n, clamp strip scroll to n-8 with 8 tiles, no null call, no highlight.
    let disabled_cm = b.catalog.films.iter().map(|f| f.cm).find(|cm| !values.contains(cm));
    for (tag, bad_cm) in [
        ("corrupt NVRAM", 0x63u32),
        ("corrupt NVRAM", 0xFF),
    ]
    .into_iter()
    .chain(disabled_cm.map(|cm| ("disabled catalog film", cm)))
    {
        m.w8(stock::NV_CM, bad_cm as u8);
        let sel = m.call(MENU_SEL, &[rows])?;
        let clamped = m.call(qs_sel_clamp, &[rows])?;
        let mut strip_ok = true;
        for init_scroll in [0u16, (n_rows - 8) as u16] {
            m.w16(stock::QS_STATE + 6, init_scroll);
            match strip(&mut m, cm_slot, bad_cm) {
                Ok((scroll, bl)) => {
                    strip_ok &= scroll == (n_rows - 8) as u16
                        && bl.iter().filter(|x| x[3] == 54).count() == 8
                        && fills.borrow().is_empty()
                        && null_calls.get() == 0;
                }
                Err(_) => strip_ok = false,
            }
        }
        r.check(
            sel == n_rows as u32 && clamped == (n_rows - 1) as u32 && strip_ok,
            format!("QS Colour Mode {tag} ({bad_cm:#x}): 0xA004D4F0={sel}, clamped={clamped}, 8 tiles, no highlight"),
        );
    }

    m.w8(stock::NV_ISO_STEP, 1);
    let iso3_rows = m.call(stock::QS_SLOT_ROWS, &[iso_slot.0, iso_slot.1])?;
    let iso3_n = m.call(stock::MENU_ROW_COUNT, &[iso3_rows])? as usize;
    for bad_iso in [0x19u32, 0xFF] {
        m.w8(stock::NV_ISO, bad_iso as u8);
        let sel = m.call(MENU_SEL, &[iso3_rows])?;
        let clamped = m.call(qs_sel_clamp, &[iso3_rows])?;
        let mut strip_ok = true;
        for init_scroll in [0u16, (iso3_n - 8) as u16] {
            m.w16(stock::QS_STATE + 6, init_scroll);
            match strip(&mut m, iso_slot, bad_iso) {
                Ok((scroll, bl)) => {
                    strip_ok &= scroll == (iso3_n - 8) as u16
                        && bl.iter().filter(|x| x[3] == 54).count() == 8
                        && fills.borrow().is_empty()
                        && null_calls.get() == 0;
                }
                Err(_) => strip_ok = false,
            }
        }
        r.check(
            sel == iso3_n as u32 && clamped == (iso3_n - 1) as u32 && strip_ok,
            format!("QS ISO 1/3-EV no-match {bad_iso:#x}: 0xA004D4F0={sel}, clamped={clamped}, 8 tiles, no highlight"),
        );
    }

    // QS slot icon path 0xA00439C4 stays stock on no-match: zero terminator yields icon 0 -> no blit.
    m.load_nv_defaults();
    m.w16(stock::QS_STATE + 4, 0);
    let slot0_xy = (m.u16(stock::QS_STATE + 0x5C) as u32, m.u16(stock::QS_STATE + 0x5E) as u32);
    blits.borrow_mut().clear();
    m.call_ext(QS_SLOT_ICONS, &[], &[], 100_000_000)?;
    let def_slot_blits: Vec<[u32; 4]> = blits.borrow().iter().map(|x| [x[1], x[2], x[3], x[4]]).collect();
    let t_e8 = m.call(stock::GET_MENU_TABLE, &[0xE8])?;
    let slot0_set = m.u32(t_e8 + 0x18) & !1;
    m.call(slot0_set, &[0xFFFF])?;
    blits.borrow_mut().clear();
    m.call_ext(QS_SLOT_ICONS, &[], &[], 100_000_000)?;
    let nomatch_slot_blits: Vec<[u32; 4]> = blits.borrow().iter().map(|x| [x[1], x[2], x[3], x[4]]).collect();
    let mut stock_m = Machine::new(&b.stock, 0);
    stock_m.load_nv_defaults();
    let stock_blits: Rc<RefCell<Vec<[u32; 4]>>> = Rc::new(RefCell::new(Vec::new()));
    {
        let sb = stock_blits.clone();
        stock_m.watch(stock::BLIT, move |c| {
            sb.borrow_mut().push([c.r[1], c.r[2], c.r[3], c.r32(c.r[13])]);
        });
    }
    stock_m.w16(stock::QS_STATE + 4, 0);
    stock_m.call(slot0_set, &[0xFFFF])?;
    stock_m.call_ext(QS_SLOT_ICONS, &[], &[], 100_000_000)?;
    r.check(
        def_slot_blits.len() == 4
            && def_slot_blits.iter().any(|b| (b[0], b[1]) == slot0_xy)
            && nomatch_slot_blits.len() == 3
            && !nomatch_slot_blits.iter().any(|b| (b[0], b[1]) == slot0_xy)
            && nomatch_slot_blits == *stock_blits.borrow(),
        format!(
            "QS slot icons (0xA00439C4): default draws 4, no-match draws {} without slot 0 (matches stock)",
            nomatch_slot_blits.len()
        ),
    );
    m.load_nv_defaults();

    if let Some(&other) = slots.iter().find(|&(_, &(_, g))| g != GET_CM && g != GET_ISO).map(|(k, _)| k) {
        m.w16(stock::QS_STATE + 6, 0);
        let (_, bl) = strip(&mut m, other, values[0])?;
        r.check(!bl.is_empty(), format!("other QS slot {other:?} keeps the generic strip ({} blits)", bl.len()));
    }

    let film_cm = |name: &str| 9 + str_idx(name);
    for cm in [4, 2, film_cm("Velvia 100"), film_cm("Kodak Tri-X 400"), film_cm("FOV Classic Yellow"), film_cm("Teal & Orange")] {
        m.w8(stock::NV_CM, cm as u8);
        for (name, fn_va, args) in [
            ("LV status", b.sym("lv_status_cm"), &[cm][..]),
            ("LV info", stock::LV_INFO_CM, &[][..]),
        ] {
            blits.borrow_mut().clear();
            m.call_ext(fn_va, args, &[], 100_000_000)?;
            let last = blits.borrow().last().copied();
            let ok = last.is_some_and(|x| in_arena(x[0]) == (cm >= 9) && (x[3], x[4]) == (76, 26));
            r.check(ok, format!("{name} icon cm {cm}: {} 76x26", if cm >= 9 { "label tile" } else { "stock icon" }));
        }
    }
    for cm in [4, film_cm("Velvia 100")] {
        m.w8(stock::NV_CM, cm as u8);
        let i = m.call(stock::ROW_OSD_ICON, &[b.sym("m_f7"), 0])?;
        let res = m.call(stock::ICON_GET, &[i])?;
        let (p, w, h) = if res != 0 {
            (m.u32(res), m.u16(res + 4), m.u16(res + 6))
        } else {
            (0, 0, 0)
        };
        let exp = if cm >= 9 { b.cat("ICON_LABEL_OSD") + cm - 9 } else { 0x28 };
        r.check(
            i == exp && (w, h) == (76, 26) && in_arena(p) == (cm >= 9),
            format!("Film page row 0 OSD icon cm {cm}: {i:#x} -> {w}x{h}"),
        );
    }
    Ok(true)
}
