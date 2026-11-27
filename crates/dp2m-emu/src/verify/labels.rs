//! labels suite (135 checks): runtime-rendered BP4 label icons across all 6 classes,
//! arena allocation & reset, stray-write hygiene, QS strip routing, LV status/info/menu icons.

use std::cell::RefCell;
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
    "Mono Orange #21", "SD Custom LUT", "Rollei Retro 80S", "Fujicolor C200", "Zebras (95% IRE)",
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
    for (site, sym) in [(stock::ICON_GET + 2, "icon_hook"), (stock::QS_STRIP_CHECK, "qs_strip_hook")] {
        if b.syms.contains_key(sym) {
            r.eq(b.img.bl_target(site), Some(b.sym(sym)), format!("BL @ {site:#x} -> {sym}"));
        }
    }

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
    {
        let bl = blits.clone();
        m.watch(stock::BLIT, move |c| {
            bl.borrow_mut().push([
                c.r[0], c.r[1], c.r[2], c.r[3],
                c.r32(c.r[13]), c.r32(c.r[13] + 4), c.r32(c.r[13] + 8),
            ]);
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
        m.w8(stock::NV_CM, cm as u8);
        m.wr(stock::OSD, &vec![0u8; 640 * 480]);
        blits.borrow_mut().clear();
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
            scroll as usize <= sel && sel <= scroll as usize + 7 && tiles == exp_tiles,
            format!("QS strip cm {} (row {sel}/{n_rows}): scroll {scroll}, {tiles} tiles", values[sel]),
        );
    }
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
