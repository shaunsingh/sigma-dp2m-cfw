//! C codegen for the M6M payload: strings, menus, film table & live-view looks derived from pack + config

use std::fmt::Write;

use crate::catalog::{self, FilmKind, Paper, FILMS, FILM_MODE0, GRIDS, GRID_MODE0, PACKED_MAX};
use crate::config::Config;
use crate::pack::{norm_lv_cmcm, pack_lv_18_knots, FilmPack};

use super::{read_u32s, S3};

const SID0: u32 = 672;
// stock (localized) strings
const SID_OFF: u32 = 0x05;
const SID_ON: u32 = 0x06;
const SID_AUTO: u32 = 0x55;
const SID_COLOR_MODE: u32 = 93;
const SID_ISO50: u32 = 53;
const SID_IMAGER_TEMP: u32 = 463;

pub const MENU_PAPER: u32 = 0xF0;
const MENU_ENGINE: u32 = 0xF1;
const MENU_FILL: u32 = 0xF2;
const MENU_MINSHUTTER: u32 = 0xF3;
const MENU_PEAKCOL: u32 = 0xF4;
const MENU_GRAIN: u32 = 0xF5;
const MENU_HALATION: u32 = 0xF6;
pub const MENU_FILM_PAGE: u32 = 0xF7;
const MENU_PAPER_CLS0: u32 = 0x1F0;
// QS copy of a setting menu: values carry label icons, which the menu page would show in place of their text
const MENU_QS_COPY: u32 = 0x200;
// Custom QS slot menus 0xE5..0xEC: a slot stores the chosen row's w4 and QS opens Get_Menu_Table(w4)
const STOCK_QS_SLOTS: [u32; 8] = [0xA07F_032C, 0xA07E_FF6C, 0xA07F_014C, 0xA07E_FD8C, 0xA07F_0AAC, 0xA07F_06EC, 0xA07F_08CC, 0xA07F_050C];
// row ids of added slot items: unused ids that resolve to the stock empty table, like the stock items 0xED..0x100
const QS_ROW0: u32 = 0x109;

// label icons rendered on camera from string index (`class + index`): 110x70 QS value, 54x42 QS strip,
// 76x26 OSD, 51x31 MODE screen, 35x20 LV widget, 42x34 QS item
const ICON_LABEL_L: u32 = 0x400;
const ICON_LABEL_S: u32 = 0x800;
const ICON_LABEL_OSD: u32 = 0xC00;
const ICON_LABEL_MODE: u32 = 0x1000;
const ICON_LABEL_WIDGET: u32 = 0x1400;
const ICON_LABEL_QS: u32 = 0x1800;

const STOCK_COLOR_MENU: u32 = 0xA07E_BFEC;
const STOCK_EXPO_MENU: u32 = 0xA07E_C8AC;
const STOCK_GRID_MENU: u32 = 0xA07E_EFCC;
const STOCK_ISO_MENU_1EV: u32 = 0xA07E_AC8C;
const STOCK_ISO_MENU_3EV: u32 = 0xA07E_ADAC;
const STOCK_ISO_MENU_POOL: u32 = 0xA004_D3DC;
const STOCK_SHOOT_P2: u32 = 0xA07E_A60C;
const STOCK_SHOOT_P3: u32 = 0xA07E_A6EC;
const STOCK_SETUP_P4: u32 = 0xA07E_AC2C;
const STOCK_ISO_AUTO: u32 = 0xA07F_188C;
const STOCK_BKT_ORDER: u32 = 0xA07E_DF8C;
const STOCK_LV_GAMMA: u32 = 0xA026_8A14;

#[derive(Clone, Copy)]
enum Fp {
    Null,
    Addr(u32),
    Sym(&'static str),
}

#[derive(Clone, Copy)]
struct Row {
    w: [u32; 5],
    get: Fp,
    set: Fp,
    extra: u32,
}

impl Row {
    fn stock(w: [u32; 8]) -> Self {
        let fp = |v| if v == 0 { Fp::Null } else { Fp::Addr(v) };
        Self { w: [w[0], w[1], w[2], w[3], w[4]], get: fp(w[5]), set: fp(w[6]), extra: w[7] }
    }

    fn page(submenu: u32, sid: u32) -> Self {
        Self { w: [submenu, 0x0100_0000, 0, sid << 16, 0], get: Fp::Null, set: Fp::Null, extra: 0 }
    }
}

struct Strings(Vec<String>);

impl Strings {
    fn add(&mut self, s: &str) -> u32 {
        self.0.push(s.to_owned());
        SID0 + self.0.len() as u32 - 1
    }
}

pub struct Generated {
    pub header: String,
    pub source: String,
    pub stats: String,
}

fn stock_rows(s3: &S3, va: u32) -> Vec<[u32; 8]> {
    (0..)
        .map(|i| read_u32s::<8>(s3, va + i * 32))
        .take_while(|w| w.iter().any(|&x| x != 0))
        .collect()
}

pub fn generate(s3: &S3, pack: &FilmPack, cfg: &Config) -> Result<Generated, String> {
    let mut st = Strings(Vec::new());
    let mut menus: Vec<(u32, &'static str, Vec<Row>)> = Vec::new();
    let mut hooks = String::new();

    // film labels first: string index == label icon index
    let film_sid: Vec<u32> = FILMS.iter().map(|f| st.add(f.label)).collect();

    let color = stock_rows(s3, STOCK_COLOR_MENU);
    let (vivid, bw) = (color[1], color[6]);
    let mut color_rows: Vec<Row> = color.iter().copied().map(Row::stock).collect();
    for (i, f) in FILMS.iter().enumerate().filter(|(_, f)| cfg.film_enabled(f.slug)) {
        let t = if f.kind.is_mono() { bw } else { vivid };
        let si = film_sid[i] - SID0;
        color_rows.push(Row::stock([
            t[0],
            0x0100_0000 | (ICON_LABEL_L + si),
            (ICON_LABEL_OSD + si) << 16 | (ICON_LABEL_S + si),
            film_sid[i] << 16,
            FILM_MODE0 as u32 + i as u32,
            t[5],
            t[6],
            0,
        ]));
    }
    menus.push((0x09, ".menu.color", color_rows));

    let grid = stock_rows(s3, STOCK_GRID_MENU);
    let gt = grid[1];
    let value_row = |sid: u32, value: u32, get: Fp, set: Fp| Row { w: [gt[0], gt[1], 0, sid << 16, value], get, set, extra: 0 };
    let mut grid_rows: Vec<Row> = grid.iter().copied().map(Row::stock).collect();
    for (i, g) in GRIDS.iter().enumerate() {
        let sid = st.add(g.label);
        if cfg.grid_enabled(g.slug) {
            grid_rows.push(value_row(sid, GRID_MODE0 as u32 + i as u32, Fp::Addr(gt[5]), Fp::Addr(gt[6])));
        }
    }
    menus.push((0x37, ".menu.grid", grid_rows));

    let iso_1ev = stock_rows(s3, STOCK_ISO_MENU_1EV);
    let iso_3ev = stock_rows(s3, STOCK_ISO_MENU_3EV);
    if cfg.m6m.iso50 {
        let mut iso50 = iso_1ev[1];
        iso50[3] = SID_ISO50 << 16;
        iso50[4] = 1;
        for (name, stock, va) in [("iso_1ev", &iso_1ev, STOCK_ISO_MENU_1EV), ("iso_3ev", &iso_3ev, STOCK_ISO_MENU_3EV)] {
            let mut rows: Vec<Row> = stock.iter().copied().map(Row::stock).collect();
            rows.insert(1, Row::stock(iso50));
            menus.push((0, if name == "iso_1ev" { ".menu.iso_1ev" } else { ".menu.iso_3ev" }, rows));
            let pool = STOCK_ISO_MENU_POOL + if name == "iso_1ev" { 0 } else { 4 };
            writeln!(hooks, "HOOK_WORD({pool:#X}, m_{}, {va:#X});", name).unwrap();
        }
    }

    // paper menus by class: none / color / bw / lut; titles are contiguous so the UI picks `title0 + class`
    let paper_title = st.add("Print Paper");
    for t in ["Print Paper", "Darkroom Paper", "LUT Slot"] {
        st.add(t);
    }
    let paper_sets: [(Option<&[Paper]>, &str); 4] = [
        (None, "none"),
        (Some(catalog::COLOR_PAPERS), "color_paper"),
        (Some(catalog::BW_PAPERS), "bw_paper"),
        (Some(catalog::LUT_SLOTS), "lut_slot"),
    ];
    for (cls, (papers, nv)) in paper_sets.iter().enumerate() {
        let (get, set) = (Fp::Sym(nv_get(nv)), Fp::Sym(nv_set(nv)));
        let mut rows = vec![value_row(SID_AUTO, 0, get, set)];
        for (k, p) in papers.unwrap_or(&[]).iter().enumerate() {
            rows.push(value_row(st.add(p.label), k as u32 + 1, get, set));
        }
        menus.push((MENU_PAPER_CLS0 + cls as u32, ".data.menu", rows));
    }

    let mut setting = |id: u32, nv: &'static str, labels: &[u32]| {
        let rows = labels
            .iter()
            .enumerate()
            .map(|(v, &sid)| value_row(sid, v as u32, Fp::Sym(nv_get(nv)), Fp::Sym(nv_set(nv))))
            .collect();
        menus.push((id, ".data.menu", rows));
    };
    let engine = [st.add("HW Fast"), st.add("3D Spectral")];
    setting(MENU_ENGINE, "engine", &engine);
    setting(MENU_GRAIN, "grain", &[SID_OFF, SID_ON]);
    setting(MENU_HALATION, "halation", &[SID_OFF, SID_ON]);
    let fill = [SID_OFF, st.add("Fill +0.5 (Subtle)"), st.add("Fill +1.0 (HDR)"), st.add("Fill -0.5 (Contrast)")];
    setting(MENU_FILL, "fill", &fill);
    let minsh = ["Auto (1/50s)", "1/15s (Slow)", "1/30s", "1/60s", "1/125s", "1/250s", "1/500s (Fast)"].map(|s| st.add(s));
    setting(MENU_MINSHUTTER, "minshutter", &minsh);
    let peak = ["Red", "Yellow", "White", "Black"].map(|s| st.add(s));
    setting(MENU_PEAKCOL, "peakcol", &peak);

    let t_engine = st.add("Film Engine");
    let t_grain = st.add("Film Grain");
    let t_halation = st.add("Halation");
    let t_fill = st.add("X3 Fill Light");
    let t_minsh = st.add("Min Shutter");
    let t_peak = st.add("Peaking Color");
    menus.push((
        MENU_FILM_PAGE,
        ".data.menu",
        vec![
            Row::page(0x09, SID_COLOR_MODE),
            Row::page(MENU_PAPER, paper_title),
            Row::page(MENU_ENGINE, t_engine),
            Row::page(MENU_GRAIN, t_grain),
            Row::page(MENU_HALATION, t_halation),
            Row::page(MENU_FILL, t_fill),
        ],
    ));

    let qs_items = [(MENU_PAPER, paper_title), (MENU_ENGINE, t_engine), (MENU_GRAIN, t_grain), (MENU_HALATION, t_halation), (MENU_FILL, t_fill)];
    let stock_label = [(SID_OFF, st.add("Off")), (SID_ON, st.add("On")), (SID_AUTO, st.add("Auto"))];
    let label = |sid: u32| stock_label.iter().find(|l| l.0 == sid).map_or(sid, |l| l.1).checked_sub(SID0).expect("QS value without a dp2m label");
    let qs_copies: Vec<_> = menus
        .iter()
        .filter(|(id, ..)| qs_items.iter().any(|q| q.0 == *id) || (MENU_PAPER_CLS0..MENU_PAPER_CLS0 + 4).contains(id))
        .map(|(id, _, rows)| {
            let rows = rows
                .iter()
                .map(|r| {
                    let si = label(r.w[3] >> 16);
                    Row { w: [r.w[0], 0x0100_0000 | (ICON_LABEL_L + si), ICON_LABEL_S + si, r.w[3], r.w[4]], ..*r }
                })
                .collect();
            (id | MENU_QS_COPY, ".data.menu", rows)
        })
        .collect();
    menus.extend(qs_copies);
    for (k, &va) in STOCK_QS_SLOTS.iter().enumerate() {
        let slot = stock_rows(s3, va);
        let t = slot[0];
        let mut rows: Vec<Row> = slot.iter().copied().map(Row::stock).collect();
        for (j, &(id, title)) in qs_items.iter().enumerate() {
            let icon = 0x0100_0000 | (ICON_LABEL_QS + title - SID0);
            rows.push(Row::stock([QS_ROW0 + j as u32, icon, 0, title << 16, id | MENU_QS_COPY, t[5], t[6], 0]));
        }
        menus.push((0xE5 + k as u32, ".data.menu", rows));
    }

    let page = |va: u32, drop: u32, add: &[Row]| -> Vec<Row> {
        let mut rows: Vec<Row> = stock_rows(s3, va).into_iter().filter(|w| w[0] != drop).map(Row::stock).collect();
        rows.extend_from_slice(add);
        rows
    };
    menus.push((0x0F, ".data.menu", page(STOCK_SHOOT_P2, 0x09, &[])));
    menus.push((0x10, ".data.menu", page(STOCK_SHOOT_P3, u32::MAX, &[Row::page(MENU_PEAKCOL, t_peak)])));
    menus.push((0x17, ".data.menu", page(STOCK_SETUP_P4, u32::MAX, &[Row::page(0xA3, SID_IMAGER_TEMP)])));
    menus.push((0x3E, ".data.menu", page(STOCK_ISO_AUTO, u32::MAX, &[Row::page(MENU_MINSHUTTER, t_minsh)])));

    let bkt = stock_rows(s3, STOCK_BKT_ORDER);
    let mut bkt_rows: Vec<Row> = bkt.iter().copied().map(Row::stock).collect();
    if cfg.m6m.bkt5 {
        for (v, label) in [(4, "5-Shot: 0 / - / +"), (6, "5-Shot: - / 0 / +"), (7, "5-Shot: + / 0 / -")] {
            bkt_rows.push(value_row(st.add(label), v, Fp::Addr(bkt[0][5]), Fp::Addr(bkt[0][6])));
        }
    }
    menus.push((0x28, ".data.menu", bkt_rows));

    // exposure modes C4/C5 (9, 10) as copies of C3; the "C4"/"C5" strings label their icons like the stock "C1" glyphs
    let expo = stock_rows(s3, STOCK_EXPO_MENU);
    let c3 = *expo.iter().find(|w| w[4] == 7).ok_or("stock exposure menu 0x05 has no C3 row")?;
    let mut expo_rows: Vec<Row> = expo.iter().copied().map(Row::stock).collect();
    let mut c_label = [0; 2];
    for (k, (text, label)) in [("Custom 4", "C4"), ("Custom 5", "C5")].into_iter().enumerate() {
        let sid = st.add(text);
        c_label[k] = st.add(label);
        let si = c_label[k] - SID0;
        expo_rows.push(Row::stock([
            c3[0],
            0x0100_0000 | (ICON_LABEL_L + si),
            (ICON_LABEL_MODE + si) << 16 | (ICON_LABEL_S + si),
            sid << 16,
            9 + k as u32,
            c3[5],
            c3[6],
            c3[7],
        ]));
    }
    menus.push((0x05, ".data.menu", expo_rows));

    let looks = lv_looks(s3, pack)?;
    let films = film_infos(pack, cfg);

    let mut h = String::new();
    writeln!(h, "// generated by dp2m; do not edit\n#pragma once").unwrap();
    for (name, v) in [
        ("MENU_PAPER", MENU_PAPER),
        ("MENU_PAPER_CLS0", MENU_PAPER_CLS0),
        ("MENU_FILM_PAGE", MENU_FILM_PAGE),
        ("MENU_QS_COPY", MENU_QS_COPY),
        ("SID_PAPER_TITLE", paper_title),
        ("SID_C4", c_label[0]),
        ("SID_C5", c_label[1]),
        ("ICON_LABEL_L", ICON_LABEL_L),
        ("ICON_LABEL_S", ICON_LABEL_S),
        ("ICON_LABEL_OSD", ICON_LABEL_OSD),
        ("ICON_LABEL_MODE", ICON_LABEL_MODE),
        ("ICON_LABEL_WIDGET", ICON_LABEL_WIDGET),
        ("ICON_LABEL_QS", ICON_LABEL_QS),
        ("PACKED_MAX", PACKED_MAX as u32),
        ("PEAKING_THRESH", cfg.m6m.peaking_threshold),
        ("ZEBRA_THRESH", cfg.m6m.zebra_threshold as u32),
    ] {
        writeln!(h, "#define {name} {v}u").unwrap();
    }
    let mut c = String::new();
    writeln!(c, "// generated by dp2m; do not edit\n#include \"dp2m.h\"\n#include \"catalog.h\"\n").unwrap();
    writeln!(c, "#define G(x) ((uint32_t (*)(void))(x))\n#define S(x) ((void (*)(uint32_t))(x))").unwrap();
    for nv in ["color_paper", "bw_paper", "lut_slot", "engine", "grain", "halation", "fill", "minshutter", "peakcol", "none"] {
        writeln!(c, "uint32_t nv_get_{nv}(void);\nvoid nv_set_{nv}(uint32_t);").unwrap();
    }

    let strs = &st.0;
    if strs.len() > (ICON_LABEL_S - ICON_LABEL_L) as usize {
        return Err(format!("{} strings overflow the label icon index (class << 10 | string index)", strs.len()));
    }
    writeln!(c, "\nconst char *const dp2m_strings[] __attribute__((section(\".menu.str\"))) = {{").unwrap();
    for s in strs {
        writeln!(c, "    \"{}\",", s.replace('\\', "\\\\").replace('"', "\\\"")).unwrap();
    }
    writeln!(c, "}};\nconst uint32_t dp2m_string_count = {};", strs.len()).unwrap();

    writeln!(c, "\nconst struct film_info dp2m_films[] = {{").unwrap();
    for (f, info) in FILMS.iter().zip(&films) {
        writeln!(c, "    {{ {}, {}, {}, {} }}, /* {} */", info[0], info[1], info[2], info[3], f.slug).unwrap();
    }
    writeln!(c, "}};\nconst uint32_t dp2m_film_count = {};", FILMS.len()).unwrap();

    writeln!(c, "\nconst struct lv_look dp2m_lv_looks[] __attribute__((section(\".lv\"))) = {{").unwrap();
    for (p, l) in looks.iter().enumerate() {
        let m: Vec<String> = l.0.iter().map(|v| v.to_string()).collect();
        let k: Vec<String> = l.1.iter().map(|v| v.to_string()).collect();
        writeln!(c, "    {{ {{{}}}, {{{}}}, {}, 0 }}, /* {} */", m.join(","), k.join(","), l.2, p + FILM_MODE0 as usize).unwrap();
    }
    writeln!(c, "}};").unwrap();

    let tv: Vec<String> = cfg.m6m.min_shutter_q16.iter().map(|v| format!("{v:#010X}")).collect();
    writeln!(c, "\nconst uint32_t dp2m_tv_min_q16[7] = {{ {} }};", tv.join(", ")).unwrap();

    let mut rows_total = 0;
    let mut map = String::from("\nconst struct menu_map dp2m_menus[] = {\n");
    for (id, section, rows) in &menus {
        let name = if *id == 0 {
            section.trim_start_matches(".menu.").to_owned()
        } else {
            format!("{id:x}")
        };
        writeln!(c, "\nstruct menu_row m_{name}[] __attribute__((section(\"{section}.{name}\"))) = {{").unwrap();
        for r in rows {
            let fp = |f: Fp, cast: &str| match f {
                Fp::Null => "0".to_owned(),
                Fp::Addr(a) => format!("{cast}({a:#010X})"),
                Fp::Sym(s) => s.to_owned(),
            };
            writeln!(
                c,
                "    {{ {:#X}, {:#X}, {:#X}, {:#X}, {}, {}, {}, {:#X} }},",
                r.w[0], r.w[1], r.w[2], r.w[3], r.w[4], fp(r.get, "G"), fp(r.set, "S"), r.extra
            )
            .unwrap();
        }
        writeln!(c, "    {{ 0 }},\n}};").unwrap();
        rows_total += rows.len() + 1;
        if *id != 0 {
            writeln!(map, "    {{ {id:#X}, 0, m_{name} }},").unwrap();
        }
    }
    map.push_str("    { 0, 0, 0 },\n};\n");
    c.push_str(&map);
    c.push('\n');
    c.push_str(&hooks);

    let stats = format!(
        "strings={} menus={} rows={} films={}/{} grids={}/{}",
        strs.len(),
        menus.len(),
        rows_total,
        FILMS.iter().filter(|f| cfg.film_enabled(f.slug)).count(),
        FILMS.len(),
        GRIDS.iter().filter(|g| cfg.grid_enabled(g.slug)).count(),
        GRIDS.len()
    );
    Ok(Generated { header: h, source: c, stats })
}

fn nv_get(nv: &str) -> &'static str {
    match nv {
        "color_paper" => "nv_get_color_paper",
        "bw_paper" => "nv_get_bw_paper",
        "lut_slot" => "nv_get_lut_slot",
        "engine" => "nv_get_engine",
        "grain" => "nv_get_grain",
        "halation" => "nv_get_halation",
        "fill" => "nv_get_fill",
        "minshutter" => "nv_get_minshutter",
        "peakcol" => "nv_get_peakcol",
        _ => "nv_get_none",
    }
}

fn nv_set(nv: &str) -> &'static str {
    match nv {
        "color_paper" => "nv_set_color_paper",
        "bw_paper" => "nv_set_bw_paper",
        "lut_slot" => "nv_set_lut_slot",
        "engine" => "nv_set_engine",
        "grain" => "nv_set_grain",
        "halation" => "nv_set_halation",
        "fill" => "nv_set_fill",
        "minshutter" => "nv_set_minshutter",
        "peakcol" => "nv_set_peakcol",
        _ => "nv_set_none",
    }
}

/// `[packed base, paper class, auto paper, mono]` per film.
fn film_infos(pack: &FilmPack, cfg: &Config) -> Vec<[u8; 4]> {
    FILMS
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let sub = catalog::films_of(f.kind).position(|(j, _)| j == i).unwrap();
            let cls = match f.kind {
                FilmKind::Negative => 1,
                FilmKind::BwSilver => 2,
                FilmKind::SdLut => 3,
                _ => 0,
            };
            let default = if f.kind == FilmKind::Negative { pack.neg_default_paper[sub] } else { 0 };
            let auto = cfg.film_paper(f.slug).unwrap_or(default);
            [catalog::packed_mode(i, 0), cls, auto, f.kind.is_mono() as u8]
        })
        .collect()
}

type Look = ([i16; 9], [u8; 18], u8);

const LV_CMCM: u8 = 1;
const LV_MONO_Q7: u8 = 2;
const LV_KNOTS: u8 = 4;

/// One look per packed mode 9..=198, mirroring the capture recipe the M5 applies.
fn lv_looks(s3: &S3, pack: &FilmPack) -> Result<Vec<Look>, String> {
    let stock_lv = super::read(s3, STOCK_LV_GAMMA, 1024);
    let with = |cmcm: &[i16; 9], curve: &[u8; 4096]| -> Look {
        let (m, g) = norm_lv_cmcm(cmcm);
        (m, pack_lv_18_knots(curve, stock_lv, g), LV_CMCM | LV_KNOTS)
    };
    let mut out: Vec<Look> = vec![([0; 9], [0; 18], 0); (PACKED_MAX - FILM_MODE0 + 1) as usize];
    let at = |p: u8| (p - FILM_MODE0) as usize;

    for (i, m) in pack.dp2q_cmcm_4.iter().enumerate() {
        out[at(FILM_MODE0 + i as u8)] = (*m, [0; 18], LV_CMCM);
    }
    for (i, &(r, g, b)) in catalog::MONO_FILTER_Q7.iter().enumerate() {
        let (fi, _) = catalog::films_of(FilmKind::OpticalMono).nth(i).unwrap();
        out[at(catalog::packed_mode(fi, 0))] = ([r, g, b, r, g, b, r, g, b], [0; 18], LV_MONO_Q7);
    }
    for (k, (fi, _)) in catalog::films_of(FilmKind::Reversal).enumerate() {
        out[at(catalog::packed_mode(fi, 0))] = with(&pack.lv_color_cmcm[k], &pack.lv_color_curves[k]);
    }
    for (k, (fi, _)) in catalog::films_of(FilmKind::Negative).enumerate() {
        for p in 0..8u8 {
            let look = if p == pack.neg_default_paper[k] {
                with(&pack.lv_color_cmcm[4 + k], &pack.lv_color_curves[4 + k])
            } else {
                let r = &pack.color_recipes[4 + k * 8 + p as usize];
                with(&r.cmcm_q10, &r.lut_srgb_raw)
            };
            out[at(catalog::packed_mode(fi, p))] = look;
        }
    }
    for (k, (fi, _)) in catalog::films_of(FilmKind::BwSilver).enumerate() {
        for p in 0..8u8 {
            let r = &pack.bw_recipes[k * 8 + p as usize];
            out[at(catalog::packed_mode(fi, p))] = with(&r.cmcm_q10, &r.lut_srgb_raw);
        }
    }
    for (i, l) in out.iter().enumerate() {
        if l.2 & LV_KNOTS != 0 && !l.1.windows(2).all(|w| w[0] <= w[1]) {
            return Err(format!("non-monotonic LV knots for packed mode {}", i + FILM_MODE0 as usize));
        }
    }
    Ok(out)
}
