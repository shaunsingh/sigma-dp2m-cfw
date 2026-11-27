//! M6M reverse-engineering engine behind `dp2m re`.

pub mod dis;
pub mod liveness;
pub mod xref;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use crate::image::{BASE, CODE_END, END, FPGA, Image, Package, Sym, load_syms};
use crate::machine::Machine;
use crate::stock;
use crate::verify::{menu_tables, read_rows_mach};

pub struct ReCtx {
    pub img: Image,
    pub syms: HashMap<String, Sym>,
    pub names: HashMap<u32, String>,
    sized: Vec<(u32, u32, String)>,
}

impl ReCtx {
    pub fn load(image_path: &Path, syms_path: Option<&Path>) -> Result<Self, String> {
        let img = Image::load(image_path)?;
        let auto_syms = image_path.parent().map(|p| p.join("m6m/m6m.syms"));
        let syms = match syms_path {
            Some(p) if p.is_file() => load_syms(p)?,
            None => match auto_syms.as_deref() {
                Some(p) if p.is_file() => load_syms(p)?,
                _ => HashMap::new(),
            },
            _ => HashMap::new(),
        };
        let mut names: HashMap<u32, String> = stock::NAMES.iter().map(|&(va, n)| (va, n.to_owned())).collect();
        let mut sized = Vec::new();
        for (n, s) in &syms {
            names.insert(s.va(), n.clone());
            if s.size > 0 {
                sized.push((s.va(), s.va() + s.size, n.clone()));
            }
        }
        sized.sort_by_key(|x| x.0);
        Ok(Self { img, syms, names, sized })
    }

    pub fn where_va(&self, va: u32) -> String {
        let idx = self.sized.partition_point(|x| x.0 <= va);
        if idx > 0 {
            let (lo, hi, ref n) = self.sized[idx - 1];
            if va < hi {
                return format!("{n}+{:#x}", va - lo);
            }
        }
        if (BASE..END).contains(&va)
            && let Some(f) = xref::fn_start(&self.img, va, 0x1000)
        {
            return match self.names.get(&f) {
                Some(n) => format!("fn {f:#x} {n}"),
                None => format!("fn {f:#x}"),
            };
        }
        String::new()
    }

    pub fn machine(&self) -> Machine {
        let mut m = Machine::new(&self.img, 0);
        m.load_nv_defaults();
        m
    }
}

fn fnv64(data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub fn cmd_package(path: &Path, ref_fw: Option<&Path>) -> Result<Vec<String>, String> {
    let raw = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let pkg = Package::from_bytes(raw)?;
    let ref_pkg = match ref_fw {
        Some(p) if p.canonicalize().ok() != path.canonicalize().ok() && p.is_file() => {
            Some(Package::from_bytes(std::fs::read(p).map_err(|e| e.to_string())?)?)
        }
        _ => None,
    };
    let mut out = Vec::new();
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("pkg");
    out.push(format!(
        "{name}: {} B  version {} {:?}  date {}  m6m @ {:#x}",
        pkg.raw.len(),
        pkg.version(),
        pkg.versions(),
        pkg.date(),
        pkg.c78_off(),
    ));
    let fl = pkg.flags();
    let fdesc: Vec<&str> = [
        (0, "b0 running fw flashes bridge"),
        (4, "b4 bridge flashes M6M @ hdr[0x64]"),
        (5, "b5 bridge flashes M5"),
    ]
    .into_iter()
    .filter(|(b, _)| (fl >> b) & 1 != 0)
    .map(|(_, d)| d)
    .collect();
    out.push(format!("flags {fl:#04x}: {}", fdesc.join("; ")));
    for (seg_name, off, len) in pkg.segments() {
        let seg = &pkg.raw[off..off + len];
        let tag = match &ref_pkg {
            None => "",
            Some(r) => if seg == r.segment(seg_name) { "  stock" } else { "  modified" },
        };
        out.push(format!("  {seg_name:<7} {off:#010x} {len:#09x}  {:016x}{tag}", fnv64(seg)));
    }
    Ok(out)
}

pub fn cmd_strings(ctx: &ReCtx, needle: Option<&str>, min_len: usize, with_refs: bool) -> Vec<String> {
    let s3 = ctx.img.s3();
    let idx = with_refs.then(|| xref::XrefIndex::build(&ctx.img));
    let needle_lo = needle.map(str::to_ascii_lowercase);
    let mut out = Vec::new();
    let mut i = 0;
    while i < s3.len() {
        if (0x20..=0x7E).contains(&s3[i]) || s3[i] == b'\t' || s3[i] == b'\n' {
            let start = i;
            while i < s3.len() && ((0x20..=0x7E).contains(&s3[i]) || s3[i] == b'\t' || s3[i] == b'\n') {
                i += 1;
            }
            if i < s3.len() && s3[i] == 0 && i - start >= min_len {
                let va = BASE + start as u32;
                if !(FPGA.0..FPGA.1).contains(&va) {
                    let text = String::from_utf8_lossy(&s3[start..i]);
                    if needle_lo.as_ref().is_none_or(|p| text.to_ascii_lowercase().contains(p)) {
                        if let Some(ref ix) = idx {
                            out.push(format!("{va:08x} {:3}  {text:?}", ix.count_data_refs(va)));
                        } else {
                            out.push(format!("{va:08x}  {text:?}"));
                        }
                    }
                }
            }
        }
        i += 1;
    }
    out
}

pub fn cmd_sid(ctx: &ReCtx, sids: &[u32], find: Option<&str>, lang: u32) -> Vec<String> {
    let mut m = ctx.machine();
    let list: Vec<u32> = if let Some(q) = find {
        let qlo = q.to_ascii_lowercase();
        let extra = ctx
            .syms
            .get("dp2m_string_count")
            .map_or(0, |s| m.u32(s.va()));
        (0..stock::STOCK_SIDS + extra)
            .filter(|&s| {
                m.ui_string(s, lang)
                    .ok()
                    .flatten()
                    .is_some_and(|t| t.to_ascii_lowercase().contains(&qlo))
            })
            .collect()
    } else {
        sids.to_vec()
    };
    list.into_iter()
        .map(|s| {
            let txt = match m.ui_string(s, lang) {
                Ok(Some(t)) => format!("{t:?}"),
                Ok(None) => "None".into(),
                Err(e) => format!("<{e}>"),
            };
            format!("{s:4} {s:#05x}  {txt}")
        })
        .collect()
}

pub fn cmd_menus(ctx: &ReCtx, ids: &[u32]) -> Vec<String> {
    let mut m = ctx.machine();
    let tbls = if ids.is_empty() {
        menu_tables(&mut m, 0..0x200)
    } else {
        menu_tables(&mut m, ids.iter().copied())
    };
    let mut by_tbl: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (mid, t) in tbls {
        by_tbl.entry(t).or_default().push(mid);
    }
    let mut groups: Vec<(u32, Vec<u32>)> = by_tbl.into_iter().collect();
    groups.sort_by_key(|(_, mids)| mids[0]);

    let mut out = Vec::new();
    for (t, mids) in groups {
        let rows = read_rows_mach(&m, t, 128);
        let id_str: Vec<String> = mids.iter().map(|i| format!("{i:#x}")).collect();
        let tname = ctx.names.get(&t).map_or("", String::as_str);
        out.push(format!("menu {} @ {t:#x} {tname} ({} rows)", id_str.join(" "), rows.len()));
        for (k, r) in rows.into_iter().enumerate() {
            let text = if r.sid() != 0 {
                m.ui_string(r.sid(), 0).ok().flatten().unwrap_or_default()
            } else {
                String::new()
            };
            let gname = ctx.names.get(&(r.get() & !1)).cloned().unwrap_or_else(|| format!("{:#x}", r.get()));
            let sname = ctx.names.get(&(r.set() & !1)).cloned().unwrap_or_else(|| format!("{:#x}", r.set()));
            out.push(format!(
                "  {k:2} id {:#06x} sid {:4} {:<30} v={:<3} icons qs {:#x} strip {:#x} osd {:#x}  get {gname} set {sname} extra {:#x}",
                r.id(), r.sid(), format!("{text:?}"), r.value(), r.qs_icon(), r.strip_icon(), r.osd_icon(), r.extra()
            ));
        }
    }
    out
}

pub fn cmd_screens(ctx: &ReCtx) -> Vec<String> {
    let mut out = Vec::new();
    for i in 1..256u32 {
        let slot = stock::SCREEN_VT + 4 * i;
        let h = ctx.img.u32(slot);
        if !(BASE..CODE_END).contains(&(h & !1)) {
            break;
        }
        let name = ctx.names.get(&(h & !1)).map_or("", String::as_str);
        out.push(format!("{i:3} {slot:#x} -> {h:#x} {name}"));
    }
    out
}

fn arm_ret(img: &Image, fn_va: u32) -> Option<u32> {
    let w = img.u32(fn_va);
    if (w & 0xFFFF_F000) == 0xE59F_0000 {
        return Some(img.u32(fn_va + 8 + (w & 0xFFF)));
    }
    if (w & 0xFFFF_F000) == 0xE3A0_0000 {
        let rot = ((w >> 8) & 0xF) * 2;
        return Some((w & 0xFF).rotate_right(rot));
    }
    None
}

pub fn cmd_icons(ctx: &ReCtx, ids: &[u32]) -> Vec<String> {
    let mut table: BTreeMap<u32, (u32, u32, u32)> = BTreeMap::new();
    for i in 1..=stock::ICON_MAX {
        let f0 = ctx.img.u32(stock::ICON_TBL + 12 * i) & !1;
        let f1 = ctx.img.u32(stock::ICON_TBL + 12 * i + 4) & !1;
        let f2 = ctx.img.u32(stock::ICON_TBL + 12 * i + 8) & !1;
        if let (Some(bits), Some(w), Some(h)) = (arm_ret(&ctx.img, f0), arm_ret(&ctx.img, f1), arm_ret(&ctx.img, f2))
            && bits >= BASE
            && w > 0
            && h > 0
            && bits.saturating_add(w.saturating_mul(h)) <= END
        {
            table.insert(i, (bits, w, h));
        }
    }
    let list: Vec<u32> = if ids.is_empty() { table.keys().copied().collect() } else { ids.to_vec() };
    list.into_iter()
        .map(|i| match table.get(&i) {
            None => format!("{i:#05x} -"),
            Some(&(bits, w, h)) => {
                let px = ctx.img.rd(bits, (w * h) as usize);
                let lvls: Vec<u8> = px.iter().copied().collect::<BTreeSet<_>>().into_iter().take(18).collect();
                format!("{i:#05x} {bits:#x} {w}x{h} levels {lvls:?}")
            }
        })
        .collect()
}

pub fn cmd_text(ctx: &ReCtx, s: &str, px: u32, style: u32, lang: u32) -> Result<Vec<String>, String> {
    const RAMP: &[u8] = b" .:-=+*#%@@@@@@@";
    let mut m = ctx.machine();
    let buf = m.text(s, px, style, [0, 600, 600, 100], lang)?;
    let (w, h) = (stock::LINE_W, stock::LINE_ROWS);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if buf[y * w + x] != 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 >= x1 {
        return Ok(vec![format!("{s:?} px={px} style={style:#x}: empty")]);
    }
    let mut out = Vec::with_capacity(y1 - y0 + 1);
    out.push(format!("{s:?} px={px} style={style:#x} bbox={}x{} @ ({x0},{y0})", x1 - x0, y1 - y0));
    for y in y0..y1 {
        let mut line = String::with_capacity(x1 - x0);
        for x in x0..x1 {
            let v = buf[y * w + x] as usize;
            line.push(RAMP[v.min(RAMP.len() - 1)] as char);
        }
        out.push(line);
    }
    Ok(out)
}
