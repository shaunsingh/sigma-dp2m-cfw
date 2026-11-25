//! native zero-dep parser for fennel config tables (`dp2m.fnl`)
//! supports `;` comments, `{:k v}` tables, `[a b]` vecs, `:keywords`, strings, bools, `nil`, hex/dec ints, floats

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

// m5slave paper slot order (0..7)
pub const PAPER_KEYS: [&str; 8] = [
    "endura-premier",
    "ektacolor-edge",
    "supra-endura",
    "portra-endura",
    "crystal-archive",
    "kodak-2383",
    "kodak-2393",
    "ultra-endura",
];

pub fn paper_index(key: &str) -> Option<u8> {
    PAPER_KEYS.iter().position(|&k| k == key).map(|i| i as u8)
}

#[derive(Debug, Clone)]
pub struct ModeOverride {
    pub enabled: bool,
    pub paper: Option<u8>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub firmware: Option<PathBuf>,
    pub out_dir: Option<PathBuf>,
    pub pack: Option<PathBuf>,
    pub verify: bool,

    pub modes: HashMap<String, ModeOverride>,

    pub grain_scale: f64,
    pub halo_scale: f64,
    pub sd_lut_meta: [u8; 4], // [grain_amp, grain_profile, halo_gain, halo_tint]

    pub iso50: bool,
    pub bkt5: bool,
    pub peaking_threshold: u32,
    pub zebra_threshold: u32,
    pub min_shutter_q16: [u32; 7],
    pub grid_modes: HashMap<String, bool>,

    pub sd_mount: Option<PathBuf>,
    pub sd_copy_firm: bool,
    pub sd_copy_c78: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            firmware: None,
            out_dir: None,
            pack: None,
            verify: true,
            modes: HashMap::new(),
            grain_scale: 1.0,
            halo_scale: 1.0,
            sd_lut_meta: [24, 0, 64, 0],
            iso50: true,
            bkt5: true,
            peaking_threshold: 44,
            zebra_threshold: 242,
            min_shutter_q16: [
                0x0005_AAAA, 0x0004_0000, 0x0005_0000, 0x0006_0000,
                0x0007_0000, 0x0008_0000, 0x0009_0000,
            ],
            grid_modes: HashMap::new(),
            sd_mount: None,
            sd_copy_firm: true,
            sd_copy_c78: true,
        }
    }
}

impl Config {
    pub fn mode_enabled(&self, slug: &str) -> bool {
        self.modes.get(slug).map(|m| m.enabled).unwrap_or(true)
    }

    pub fn mode_paper(&self, slug: &str, default_paper: u8) -> u8 {
        self.modes
            .get(slug)
            .and_then(|m| m.paper)
            .unwrap_or(default_paper)
    }

    pub fn grid_enabled(&self, slug: &str) -> bool {
        self.grid_modes.get(slug).copied().unwrap_or(true)
    }
}

#[derive(Debug, Clone)]
enum Val {
    Nil,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Seq(Vec<Val>),
    Map(HashMap<String, Val>),
}

impl Val {
    fn as_map(&self) -> Option<&HashMap<String, Val>> {
        match self { Val::Map(m) => Some(m), _ => None }
    }
    fn as_str(&self) -> Option<&str> {
        match self { Val::Str(s) => Some(s), _ => None }
    }
    fn as_bool(&self) -> Option<bool> {
        match self { Val::Bool(b) => Some(*b), _ => None }
    }
    fn as_u32(&self) -> Option<u32> {
        match self {
            Val::Int(n) if *n >= 0 && *n <= u32::MAX as i64 => Some(*n as u32),
            _ => None,
        }
    }
    fn as_f64(&self) -> Option<f64> {
        match self {
            Val::Float(f) => Some(*f),
            Val::Int(n) => Some(*n as f64),
            _ => None,
        }
    }
    fn as_seq(&self) -> Option<&[Val]> {
        match self { Val::Seq(v) => Some(v), _ => None }
    }
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b.is_ascii_whitespace() || b == b',' {
                self.pos += 1;
            } else if b == b';' {
                while self.pos < self.bytes.len() && self.bytes[self.pos] != b'\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_ws();
        self.bytes.get(self.pos).copied()
    }

    fn parse_val(&mut self) -> Result<Val, String> {
        let b = self.peek().ok_or("unexpected EOF in Fennel config")?;
        match b {
            b'{' => self.parse_map(),
            b'[' => self.parse_seq(),
            b'"' => self.parse_str().map(Val::Str),
            b':' => {
                self.pos += 1;
                Ok(Val::Str(self.parse_atom()))
            }
            _ => {
                let tok = self.parse_atom();
                if tok == "true" { return Ok(Val::Bool(true)); }
                if tok == "false" { return Ok(Val::Bool(false)); }
                if tok == "nil" { return Ok(Val::Nil); }
                let clean = tok.replace('_', "");
                if let Some(hex) = clean.strip_prefix("0x").or_else(|| clean.strip_prefix("0X")) {
                    return i64::from_str_radix(hex, 16)
                        .map(Val::Int)
                        .map_err(|_| format!("invalid hex '{}'", tok));
                }
                if let Ok(i) = clean.parse::<i64>() {
                    return Ok(Val::Int(i));
                }
                if let Ok(f) = clean.parse::<f64>() {
                    return Ok(Val::Float(f));
                }
                Err(format!("unsupported Fennel token '{}'", tok))
            }
        }
    }

    fn parse_map(&mut self) -> Result<Val, String> {
        self.pos += 1;
        let mut map = HashMap::new();
        loop {
            match self.peek() {
                Some(b'}') => { self.pos += 1; return Ok(Val::Map(map)); }
                Some(_) => {
                    let k = match self.parse_val()? {
                        Val::Str(s) => s,
                        other => return Err(format!("expected keyword/string key, got {:?}", other)),
                    };
                    let v = self.parse_val()?;
                    map.insert(k, v);
                }
                None => return Err("unterminated '{' in Fennel config".into()),
            }
        }
    }

    fn parse_seq(&mut self) -> Result<Val, String> {
        self.pos += 1;
        let mut seq = Vec::new();
        loop {
            match self.peek() {
                Some(b']') => { self.pos += 1; return Ok(Val::Seq(seq)); }
                Some(_) => seq.push(self.parse_val()?),
                None => return Err("unterminated '[' in Fennel config".into()),
            }
        }
    }

    fn parse_str(&mut self) -> Result<String, String> {
        self.pos += 1;
        let mut s = String::new();
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            self.pos += 1;
            if b == b'"' {
                return Ok(s);
            }
            if b == b'\\' && self.pos < self.bytes.len() {
                let esc = self.bytes[self.pos];
                self.pos += 1;
                match esc {
                    b'n' => s.push('\n'),
                    b't' => s.push('\t'),
                    b'\\' => s.push('\\'),
                    b'"' => s.push('"'),
                    other => s.push(other as char),
                }
            } else {
                s.push(b as char);
            }
        }
        Err("unterminated string in Fennel config".into())
    }

    fn parse_atom(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b.is_ascii_whitespace()
                || matches!(b, b'{' | b'}' | b'[' | b']' | b'(' | b')' | b';' | b'"' | b',')
            {
                break;
            }
            self.pos += 1;
        }
        String::from_utf8_lossy(&self.bytes[start..self.pos]).to_string()
    }
}

fn parse_grain_profile(s: &str) -> Result<u8, String> {
    match s {
        "negative" => Ok(0),
        "slide" => Ok(1),
        "bw" => Ok(2),
        _ => Err(format!("unknown :grain-profile '{}' (expected :negative, :slide, or :bw)", s)),
    }
}

fn parse_halo_tint(s: &str) -> Result<u8, String> {
    match s {
        "crimson" => Ok(0),
        "neutral" => Ok(1),
        "amber" => Ok(2),
        _ => Err(format!("unknown :halo-tint '{}' (expected :crimson, :neutral, or :amber)", s)),
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let src = fs::read_to_string(path)
            .map_err(|e| format!("read config {}: {}", path.display(), e))?;
        Self::parse(&src)
    }

    pub fn parse(src: &str) -> Result<Self, String> {
        let mut p = Parser { bytes: src.as_bytes(), pos: 0 };
        let root_val = p.parse_val()?;
        let root = root_val.as_map().ok_or("Fennel config root must be a table `{...}`")?;
        let mut cfg = Config::default();

        if let Some(v) = root.get("firmware").and_then(Val::as_str) {
            cfg.firmware = Some(PathBuf::from(v));
        }
        if let Some(v) = root.get("out-dir").and_then(Val::as_str) {
            cfg.out_dir = Some(PathBuf::from(v));
        }
        if let Some(v) = root.get("pack").and_then(Val::as_str) {
            cfg.pack = Some(PathBuf::from(v));
        }
        if let Some(v) = root.get("verify").and_then(Val::as_bool) {
            cfg.verify = v;
        }

        if let Some(modes_map) = root.get("modes").and_then(Val::as_map) {
            for (slug, val) in modes_map {
                let entry = match val {
                    Val::Bool(b) => ModeOverride { enabled: *b, paper: None },
                    Val::Map(m) => {
                        let enabled = m.get("enabled").and_then(Val::as_bool).unwrap_or(true);
                        let paper = match m.get("paper") {
                            None | Some(Val::Nil) => None,
                            Some(v) => {
                                let pk = v.as_str().ok_or_else(|| format!("mode {}: :paper must be a keyword", slug))?;
                                Some(paper_index(pk).ok_or_else(|| format!("mode {}: unknown paper ':{}'", slug, pk))?)
                            }
                        };
                        ModeOverride { enabled, paper }
                    }
                    _ => return Err(format!("mode '{}' must be a boolean or table", slug)),
                };
                cfg.modes.insert(slug.clone(), entry);
            }
        }

        if let Some(m5) = root.get("m5").and_then(Val::as_map) {
            if let Some(v) = m5.get("grain-scale").and_then(Val::as_f64) {
                cfg.grain_scale = v.clamp(0.0, 4.0);
            }
            if let Some(v) = m5.get("halo-scale").and_then(Val::as_f64) {
                cfg.halo_scale = v.clamp(0.0, 4.0);
            }
            if let Some(sd) = m5.get("sd-lut-defaults").and_then(Val::as_map) {
                if let Some(v) = sd.get("grain").and_then(Val::as_u32) {
                    cfg.sd_lut_meta[0] = v.min(64) as u8;
                }
                if let Some(s) = sd.get("grain-profile").and_then(Val::as_str) {
                    cfg.sd_lut_meta[1] = parse_grain_profile(s)?;
                }
                if let Some(v) = sd.get("halo").and_then(Val::as_u32) {
                    cfg.sd_lut_meta[2] = v.min(255) as u8;
                }
                if let Some(s) = sd.get("halo-tint").and_then(Val::as_str) {
                    cfg.sd_lut_meta[3] = parse_halo_tint(s)?;
                }
            }
        }

        if let Some(m6) = root.get("m6m").and_then(Val::as_map) {
            if let Some(v) = m6.get("iso50").and_then(Val::as_bool) {
                cfg.iso50 = v;
            }
            if let Some(v) = m6.get("bkt5").and_then(Val::as_bool) {
                cfg.bkt5 = v;
            }
            if let Some(v) = m6.get("peaking-threshold").and_then(Val::as_u32) {
                cfg.peaking_threshold = v;
            }
            if let Some(v) = m6.get("zebra-threshold").and_then(Val::as_u32) {
                cfg.zebra_threshold = v.min(255);
            }
            if let Some(seq) = m6.get("min-shutter-q16").and_then(Val::as_seq) {
                if seq.len() != 7 {
                    return Err(format!("m6m.min-shutter-q16 must have 7 entries, got {}", seq.len()));
                }
                for (i, item) in seq.iter().enumerate() {
                    cfg.min_shutter_q16[i] = item
                        .as_u32()
                        .ok_or_else(|| format!("invalid u32 in min-shutter-q16[{}]", i))?;
                }
            }
            if let Some(gm) = m6.get("grid-modes").and_then(Val::as_map) {
                for (k, v) in gm {
                    if let Some(b) = v.as_bool() {
                        cfg.grid_modes.insert(k.clone(), b);
                    }
                }
            }
        }

        if let Some(sd) = root.get("sd").and_then(Val::as_map) {
            if let Some(v) = sd.get("mount").and_then(Val::as_str) {
                cfg.sd_mount = Some(PathBuf::from(v));
            }
            if let Some(v) = sd.get("copy-firm").and_then(Val::as_bool) {
                cfg.sd_copy_firm = v;
            }
            if let Some(v) = sd.get("copy-c78").and_then(Val::as_bool) {
                cfg.sd_copy_c78 = v;
            }
        }
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_example_fennel_template() {
        let src = include_str!("../../../dp2m.example.fnl");
        let cfg = Config::parse(src).unwrap();
        assert!(cfg.verify);
        assert!(cfg.iso50 && cfg.bkt5);
        assert_eq!(cfg.modes.len(), 36);
        assert_eq!(cfg.mode_paper("portra-400", 0), 3);
        assert_eq!(cfg.mode_paper("vision3-500t", 0), 5);
        assert_eq!(cfg.peaking_threshold, 44);
        assert_eq!(cfg.zebra_threshold, 242);
        assert_eq!(cfg.sd_lut_meta, [24, 0, 64, 0]);
        assert!(cfg.grid_enabled("rgb-hist"));
    }

    #[test]
    fn supports_mode_disable_and_paper_override() {
        let cfg = Config::parse(
            r#"{:modes {:velvia-100 false
                        :portra-400 {:enabled true :paper :kodak-2383}}
                :m5 {:grain-scale 0.5 :halo-scale 1.25}}"#,
        )
        .unwrap();
        assert!(!cfg.mode_enabled("velvia-100"));
        assert!(cfg.mode_enabled("portra-400"));
        assert_eq!(cfg.mode_paper("portra-400", 3), 5);
        assert!((cfg.grain_scale - 0.5).abs() < 1e-9);
    }
}
