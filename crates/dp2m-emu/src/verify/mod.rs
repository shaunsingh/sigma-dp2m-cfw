//! Native Rust M6M verification suites (`m6m`, `custom`, `labels`) running directly on `arm9::Cpu`.
//! Replaces Python+Unicorn `tools/verify/` (~40-50x faster, zero external deps).

pub mod custom;
pub mod labels;
pub mod m6m;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::image::{BASE, END, Image, Package, Sym, load_syms};
use crate::machine::Machine;

#[derive(Clone, Debug)]
pub struct LookEntry {
    pub cmcm: [i32; 9],
    pub knots: [u8; 18],
    pub flags: u32,
}

#[derive(Clone, Debug)]
pub struct FilmEntry {
    pub cm: u32,
    pub packed: u32,
    pub paper_cls: u32,
    pub auto_paper: u32,
    pub mono: u32,
}

/// Parsed tables from `out/m6m/catalog.c` — ground truth the built image must match.
#[derive(Clone, Debug)]
pub struct Generated {
    pub strings: Vec<String>,
    pub films: Vec<FilmEntry>,
    pub looks: Vec<LookEntry>,
    pub tv_min: Vec<i32>,
    pub menus: BTreeMap<String, Vec<Vec<String>>>,
}

fn parse_i64(tok: &str) -> Result<i64, String> {
    let s = tok.trim().trim_end_matches('u');
    if let Some(neg) = s.strip_prefix('-') {
        return parse_i64(neg).map(|v| -v);
    }
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).map_err(|_| format!("bad hex '{tok}'"))
    } else {
        s.parse::<i64>().map_err(|_| format!("bad int '{tok}'"))
    }
}

fn extract_ints(line: &str) -> Vec<i64> {
    let code = line.split("/*").next().unwrap_or(line);
    let mut out = Vec::new();
    let bytes = code.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let neg = bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit();
        if neg || bytes[i].is_ascii_digit() {
            let start = i;
            if neg {
                i += 1;
            }
            if i + 1 < bytes.len() && bytes[i] == b'0' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
                i += 2;
                while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                    i += 1;
                }
            } else {
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if let Ok(v) = parse_i64(&code[start..i]) {
                out.push(v);
            }
        } else {
            i += 1;
        }
    }
    out
}

fn extract_block<'a>(text: &'a str, marker: &str) -> Result<Vec<&'a str>, String> {
    let start = text.find(marker).ok_or_else(|| format!("missing '{marker}' in catalog.c"))?;
    let brace = text[start..].find("{\n").ok_or_else(|| format!("missing '{{\\n' after '{marker}'"))? + start + 2;
    let end = text[brace..].find("\n};").ok_or_else(|| format!("missing '\\n}};' for '{marker}'"))? + brace;
    Ok(text[brace..end].lines().collect())
}

impl Generated {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut strings = Vec::new();
        for line in extract_block(text, "dp2m_strings[")? {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix('"')
                && let Some(end) = rest.rfind("\",")
            {
                let mut s = String::new();
                let mut chars = rest[..end].chars();
                while let Some(c) = chars.next() {
                    if c == '\\' {
                        if let Some(n) = chars.next() {
                            s.push(n);
                        }
                    } else {
                        s.push(c);
                    }
                }
                strings.push(s);
            }
        }

        let mut films = Vec::new();
        for (idx, line) in extract_block(text, "dp2m_films[")?.into_iter().enumerate() {
            let v = extract_ints(line);
            if v.len() >= 4 {
                films.push(FilmEntry {
                    cm: 9 + idx as u32,
                    packed: v[0] as u32,
                    paper_cls: v[1] as u32,
                    auto_paper: v[2] as u32,
                    mono: v[3] as u32,
                });
            }
        }

        let mut looks = Vec::new();
        for line in extract_block(text, "dp2m_lv_looks[")? {
            let v = extract_ints(line);
            if v.len() >= 28 {
                let mut cmcm = [0i32; 9];
                for k in 0..9 {
                    cmcm[k] = v[k] as i32;
                }
                let mut knots = [0u8; 18];
                for k in 0..18 {
                    knots[k] = v[9 + k] as u8;
                }
                looks.push(LookEntry { cmcm, knots, flags: v[27] as u32 });
            }
        }

        let tv_pos = text.find("dp2m_tv_min_q16[7] = {").ok_or("missing dp2m_tv_min_q16")?;
        let tv_brace = text[tv_pos..].find('{').unwrap() + tv_pos + 1;
        let tv_end = text[tv_brace..].find('}').unwrap() + tv_brace;
        let tv_min: Vec<i32> = extract_ints(&text[tv_brace..tv_end]).into_iter().map(|v| v as i32).collect();

        let mut menus = BTreeMap::new();
        let mut cursor = 0;
        let pat = "struct menu_row ";
        while let Some(pos) = text[cursor..].find(pat) {
            let start = cursor + pos + pat.len();
            let Some(brack) = text[start..].find("[]") else { break };
            let name = text[start..start + brack].trim().to_owned();
            if name.starts_with("m_") {
                let brace = text[start..].find("{\n").ok_or("missing menu brace")? + start + 2;
                let end = text[brace..].find("\n};").ok_or("missing menu end")? + brace;
                let mut rows = Vec::new();
                for line in text[brace..end].lines() {
                    let inner = line.trim().trim_start_matches('{').trim_end_matches(',').trim_end_matches('}').trim();
                    let fields: Vec<String> = inner.split(',').map(|f| f.trim().to_owned()).collect();
                    if fields != ["0"] && !inner.is_empty() {
                        rows.push(fields);
                    }
                }
                menus.insert(name, rows);
                cursor = end + 3;
            } else {
                cursor = start + brack + 2;
            }
        }

        Ok(Self { strings, films, looks, tv_min, menus })
    }
}

pub struct Build {
    pub dir: PathBuf,
    pub stock: Image,
    pub img: Image,
    pub pkg: Package,
    pub pkg_name: String,
    pub syms: HashMap<String, Sym>,
    pub cat: HashMap<String, u32>,
    pub catalog: Generated,
}

impl Build {
    pub fn load(out: &Path, fw: &Path) -> Result<Self, String> {
        let mut pkgs = Vec::new();
        for entry in fs::read_dir(out).map_err(|e| format!("read_dir {}: {e}", out.display()))? {
            let p = entry.map_err(|e| e.to_string())?.path();
            if p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.starts_with("DP2MV") && s.ends_with(".bin"))
            {
                pkgs.push(p);
            }
        }
        pkgs.sort();
        if pkgs.len() != 1 {
            return Err(format!("{}: expected one DP2MVxxx.bin, found {}", out.display(), pkgs.len()));
        }
        let pkg_path = &pkgs[0];
        let pkg_name = pkg_path.file_name().unwrap().to_string_lossy().into_owned();
        let pkg = Package::from_bytes(fs::read(pkg_path).map_err(|e| format!("read {}: {e}", pkg_path.display()))?)?;
        let stock = Image::load(fw)?;
        let img = Image::load(&out.join("C78FRM.bin"))?;
        let syms = load_syms(&out.join("m6m/m6m.syms"))?;

        let cat_txt = fs::read_to_string(out.join("m6m/catalog.h")).map_err(|e| format!("read catalog.h: {e}"))?;
        let mut cat = HashMap::new();
        for line in cat_txt.lines() {
            let mut it = line.split_whitespace();
            if it.next() == Some("#define")
                && let (Some(k), Some(v)) = (it.next(), it.next())
                && let Ok(n) = parse_i64(v)
            {
                cat.insert(k.to_owned(), n as u32);
            }
        }

        let gen_txt = fs::read_to_string(out.join("m6m/catalog.c")).map_err(|e| format!("read catalog.c: {e}"))?;
        let catalog = Generated::parse(&gen_txt)?;

        Ok(Self { dir: out.to_path_buf(), stock, img, pkg, pkg_name, syms, cat, catalog })
    }

    pub fn sym(&self, name: &str) -> u32 {
        self.syms.get(name).unwrap_or_else(|| panic!("missing symbol {name}")).va()
    }

    pub fn sym_info(&self, name: &str) -> Sym {
        *self.syms.get(name).unwrap_or_else(|| panic!("missing symbol {name}"))
    }

    pub fn has_all(&self, names: &[&str]) -> bool {
        names.iter().all(|n| self.syms.contains_key(*n))
    }

    pub fn cat(&self, name: &str) -> u32 {
        *self.cat.get(name).unwrap_or_else(|| panic!("missing catalog.h define {name}"))
    }

    pub fn value(&self, field: &str) -> u32 {
        let f = field
            .strip_prefix("G(")
            .or_else(|| field.strip_prefix("S("))
            .and_then(|s| s.strip_suffix(')'))
            .unwrap_or(field);
        if let Some(s) = self.syms.get(f) {
            s.addr
        } else {
            parse_i64(f).unwrap_or_else(|e| panic!("catalog field '{field}': {e}")) as u32
        }
    }
}

pub struct Report {
    pub passed: usize,
    pub failed: usize,
    pub fails: Vec<String>,
    pub verbose: bool,
}

impl Report {
    pub fn new(verbose: bool) -> Self {
        Self { passed: 0, failed: 0, fails: Vec::new(), verbose }
    }

    pub fn check(&mut self, ok: bool, what: impl Into<String>) -> bool {
        let what = what.into();
        if self.verbose {
            println!("  {} {what}", if ok { "PASS" } else { "FAIL" });
        }
        if ok {
            self.passed += 1;
        } else {
            self.failed += 1;
            self.fails.push(what);
        }
        ok
    }

    pub fn eq<T: PartialEq + std::fmt::Debug>(&mut self, got: T, exp: T, what: impl Into<String>) -> bool {
        let what = what.into();
        if got == exp {
            self.check(true, what)
        } else {
            self.check(false, format!("{what}: got {got:?}, expected {exp:?}"))
        }
    }
}

/// 32-byte menu row helper shared across verify suites and RE.
#[derive(Clone, Copy, Debug)]
pub struct MenuRow {
    pub va: u32,
    pub w: [u32; 8],
}

impl MenuRow {
    pub fn id(self) -> u32 { self.w[0] }
    pub fn qs_icon(self) -> u32 { self.w[1] & 0xFFFF }
    pub fn strip_icon(self) -> u32 { self.w[2] & 0xFFFF }
    pub fn osd_icon(self) -> u32 { self.w[2] >> 16 }
    pub fn sid(self) -> u32 { self.w[3] >> 16 }
    pub fn value(self) -> u32 { self.w[4] }
    pub fn get(self) -> u32 { self.w[5] }
    pub fn set(self) -> u32 { self.w[6] }
    pub fn extra(self) -> u32 { self.w[7] }
}

pub fn read_rows_img(img: &Image, va: u32, limit: usize) -> Vec<MenuRow> {
    let mut out = Vec::new();
    for k in 0..limit {
        let a = va + (32 * k) as u32;
        let w = core::array::from_fn(|i| img.u32(a + (4 * i) as u32));
        if w.iter().all(|&x| x == 0) {
            break;
        }
        out.push(MenuRow { va: a, w });
    }
    out
}

pub fn read_rows_mach(m: &Machine, va: u32, limit: usize) -> Vec<MenuRow> {
    let mut out = Vec::new();
    for k in 0..limit {
        let a = va + (32 * k) as u32;
        let w = core::array::from_fn(|i| m.u32(a + (4 * i) as u32));
        if w.iter().all(|&x| x == 0) {
            break;
        }
        out.push(MenuRow { va: a, w });
    }
    out
}

pub fn menu_tables(m: &mut Machine, ids: impl IntoIterator<Item = u32>) -> BTreeMap<u32, u32> {
    let mut out = BTreeMap::new();
    for mid in ids {
        if let Ok(t) = m.menu_table(mid)
            && (BASE..END).contains(&t)
            && m.rd(t, 32).iter().any(|&b| b != 0)
        {
            out.insert(mid, t);
        }
    }
    out
}

pub struct SuiteSummary {
    pub name: &'static str,
    pub skipped: bool,
    pub passed: usize,
    pub failed: usize,
    pub fails: Vec<String>,
}

pub fn run_suites(b: &Build, suites: &[&str], verbose: bool) -> Vec<SuiteSummary> {
    run_suites_with_artifacts(b, suites, verbose, None)
}

pub fn run_suites_with_artifacts(
    b: &Build,
    suites: &[&str],
    verbose: bool,
    artifacts: Option<&Path>,
) -> Vec<SuiteSummary> {
    let all = ["m6m", "custom", "labels"];
    let selected: Vec<&str> = if suites.is_empty() { all.to_vec() } else { suites.to_vec() };
    let run_one = |name: &str| -> SuiteSummary {
        if verbose {
            println!("== {name}");
        }
        let mut r = Report::new(verbose);
        let res = match name {
            "m6m" => m6m::run(b, &mut r),
            "custom" => custom::run(b, &mut r),
            "labels" => labels::run_with_artifacts(b, &mut r, artifacts),
            _ => Err(format!("unknown suite '{name}'")),
        };
        let mut skipped = false;
        match res {
            Ok(true) => {}
            Ok(false) => {
                skipped = true;
                if verbose {
                    println!("  SKIP {name} (required symbols absent)");
                }
            }
            Err(e) => {
                r.check(false, format!("suite aborted: {e}"));
            }
        }
        let static_name = match name {
            "m6m" => "m6m",
            "custom" => "custom",
            "labels" => "labels",
            _ => "unknown",
        };
        SuiteSummary {
            name: static_name,
            skipped,
            passed: r.passed,
            failed: r.failed,
            fails: r.fails,
        }
    };
    // Artifact writers must not race if a suite was explicitly selected twice.
    if !verbose && selected.len() > 1 && !(artifacts.is_some() && selected.iter().filter(|&&s| s == "labels").count() > 1) {
        return std::thread::scope(|s| {
            let handles: Vec<_> = selected.iter().map(|&n| s.spawn(move || run_one(n))).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
    }
    selected.into_iter().map(run_one).collect()
}
