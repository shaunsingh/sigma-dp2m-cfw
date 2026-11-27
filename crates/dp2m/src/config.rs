//! `dp2m.fnl` loader: Fennel compiled & evaluated by embedded LuaJIT, then deserialized via serde

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use mlua::{Lua, LuaSerdeExt, Table, Value};
use serde::Deserialize;

use crate::catalog::{self, FilmKind, Paper};

const FENNEL: &str = include_str!("../lua/fennel.lua");

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    pub firmware: Option<PathBuf>,
    pub out_dir: Option<PathBuf>,
    pub pack: Option<PathBuf>,
    pub verify: bool,
    /// `major.minor` shown by the stock updater & "Current version"; must be >= 1.05.
    pub version: String,
    pub films: BTreeMap<String, FilmCfg>,
    pub m5: M5Cfg,
    pub m6m: M6mCfg,
    pub sd: SdCfg,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum FilmCfg {
    Enabled(bool),
    Spec {
        #[serde(default = "yes")]
        enabled: bool,
        paper: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct M5Cfg {
    pub grain_scale: f64,
    pub halo_scale: f64,
    pub sd_lut_defaults: SdLutCfg,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct SdLutCfg {
    pub grain: u8,
    pub grain_profile: GrainProfile,
    pub halo: u8,
    pub halo_tint: HaloTint,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GrainProfile {
    Negative,
    Slide,
    Bw,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HaloTint {
    Crimson,
    Neutral,
    Amber,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct M6mCfg {
    pub iso50: bool,
    pub bkt5: bool,
    pub doom: bool,
    pub probe: bool,
    pub peaking_threshold: u32,
    pub zebra_threshold: u8,
    /// APEX Tv Q16 for Auto(stock), 1/15 .. 1/500.
    pub min_shutter_q16: [u32; 7],
    pub grids: BTreeMap<String, bool>,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct SdCfg {
    pub mount: Option<PathBuf>,
    /// Copy `DP2MVxxx.bin` (stock updater path).
    pub package: bool,
    /// Copy loose `Firm.bin` / `C78FRM.bin` (debug updater path, no version check).
    pub firm: bool,
    pub c78: bool,
}

fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            firmware: None,
            out_dir: None,
            pack: None,
            verify: true,
            version: "2.00".into(),
            films: BTreeMap::new(),
            m5: M5Cfg::default(),
            m6m: M6mCfg::default(),
            sd: SdCfg::default(),
        }
    }
}

impl Default for M5Cfg {
    fn default() -> Self {
        Self { grain_scale: 1.0, halo_scale: 1.0, sd_lut_defaults: SdLutCfg::default() }
    }
}

impl Default for SdLutCfg {
    fn default() -> Self {
        Self { grain: 24, grain_profile: GrainProfile::Negative, halo: 64, halo_tint: HaloTint::Crimson }
    }
}

impl Default for M6mCfg {
    fn default() -> Self {
        Self {
            iso50: true,
            bkt5: true,
            doom: true,
            probe: false,
            peaking_threshold: 44,
            zebra_threshold: 242,
            min_shutter_q16: [0x0005_AAAA, 0x0004_0000, 0x0005_0000, 0x0006_0000, 0x0007_0000, 0x0008_0000, 0x0009_0000],
            grids: BTreeMap::new(),
        }
    }
}

impl Default for SdCfg {
    fn default() -> Self {
        Self { mount: None, package: true, firm: false, c78: false }
    }
}

impl SdLutCfg {
    /// D2ML header order: grain_amp, grain_profile, halo_gain, halo_tint.
    pub fn meta(&self) -> [u8; 4] {
        [self.grain.min(64), self.grain_profile as u8, self.halo, self.halo_tint as u8]
    }
}

impl Config {
    pub fn film_enabled(&self, slug: &str) -> bool {
        match self.films.get(slug) {
            Some(FilmCfg::Enabled(b)) => *b,
            Some(FilmCfg::Spec { enabled, .. }) => *enabled,
            None => true,
        }
    }

    /// 0-based paper chosen for "Auto"; `None` keeps the pack default.
    pub fn film_paper(&self, slug: &str) -> Option<u8> {
        match self.films.get(slug) {
            Some(FilmCfg::Spec { paper: Some(p), .. }) => {
                let (_, f) = catalog::film(slug)?;
                catalog::paper_index(f.kind.papers()?, p)
            }
            _ => None,
        }
    }

    pub fn grid_enabled(&self, slug: &str) -> bool {
        self.m6m.grids.get(slug).copied().unwrap_or(true)
    }

    /// `(major, minor)` as parsed by the stock updater (`strtoul` on both halves).
    pub fn version_pair(&self) -> Result<(u8, u8), String> {
        let bad = || format!("version '{}' must be 'major.minor' >= 1.05 (e.g. \"2.00\")", self.version);
        let (a, b) = self.version.split_once('.').ok_or_else(bad)?;
        let v = (a.parse::<u8>().map_err(|_| bad())?, b.parse::<u8>().map_err(|_| bad())?);
        if v < (1, 5) || b.len() != 2 {
            return Err(bad());
        }
        Ok(v)
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let lua = Lua::new();
        let err = |e: mlua::Error| format!("{}: {e}", path.display());
        let fennel: Table = lua.load(FENNEL).set_name("@fennel.lua").eval().map_err(err)?;
        let loaded: Table = lua.globals().get::<Table>("package").and_then(|p| p.get("loaded")).map_err(err)?;
        loaded.set("fennel", &fennel).map_err(err)?;
        lua.globals().set("dp2m", catalog_table(&lua).map_err(err)?).map_err(err)?;

        let dofile: mlua::Function = fennel.get("dofile").map_err(err)?;
        let value: Value = dofile.call(path.to_string_lossy().as_ref()).map_err(err)?;
        let cfg: Self = lua.from_value(value).map_err(err)?;
        cfg.validate().map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(cfg)
    }

    fn validate(&self) -> Result<(), String> {
        self.version_pair()?;
        for (slug, f) in &self.films {
            let (_, film) = catalog::film(slug).ok_or_else(|| format!("unknown film :{slug}"))?;
            if let FilmCfg::Spec { paper: Some(p), .. } = f {
                let papers = film.kind.papers().ok_or_else(|| format!(":{slug} takes no :paper"))?;
                catalog::paper_index(papers, p).ok_or_else(|| format!(":{slug} has no paper :{p}"))?;
            }
        }
        for slug in self.m6m.grids.keys() {
            if !catalog::GRIDS.iter().any(|g| g.slug == slug) {
                return Err(format!("unknown grid :{slug}"));
            }
        }
        if !(0.0..=4.0).contains(&self.m5.grain_scale) || !(0.0..=4.0).contains(&self.m5.halo_scale) {
            return Err("m5 grain/halo scales must be within 0..4".into());
        }
        Ok(())
    }
}

/// `dp2m` global: the catalog, so configs can be computed (`(icollect [_ f (ipairs dp2m.films)] ...)`).
fn catalog_table(lua: &Lua) -> mlua::Result<Table> {
    let papers = |list: &[Paper]| {
        lua.create_sequence_from(list.iter().map(|p| lua.create_table_from([("slug", p.slug), ("label", p.label)]).unwrap()))
    };
    let films = lua.create_sequence_from(catalog::FILMS.iter().map(|f| {
        lua.create_table_from([("slug", f.slug), ("label", f.label), ("kind", f.kind.key())]).unwrap()
    }))?;
    let grids = lua.create_sequence_from(
        catalog::GRIDS.iter().map(|g| lua.create_table_from([("slug", g.slug), ("label", g.label)]).unwrap()),
    )?;
    let paper_sets = lua.create_table()?;
    for kind in [FilmKind::Negative, FilmKind::BwSilver, FilmKind::SdLut] {
        paper_sets.set(kind.key(), papers(kind.papers().unwrap())?)?;
    }
    let t = lua.create_table()?;
    t.set("films", films)?;
    t.set("papers", paper_sets)?;
    t.set("grids", grids)?;
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_str(src: &str) -> Result<Config, String> {
        let dir = std::env::temp_dir().join(format!("dp2m-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{:x}.fnl", src.len() ^ src.as_ptr() as usize));
        std::fs::write(&p, src).unwrap();
        let r = Config::load(&p);
        let _ = std::fs::remove_file(&p);
        r
    }

    #[test]
    fn example_template_loads() {
        let cfg = Config::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dp2m.example.fnl")).unwrap();
        assert!(cfg.verify && cfg.m6m.iso50 && cfg.m6m.bkt5);
        assert_eq!(cfg.version_pair().unwrap(), (2, 0));
        assert_eq!(cfg.film_paper("portra-400"), catalog::paper_index(catalog::COLOR_PAPERS, "portra-endura"));
        assert_eq!(cfg.m5.sd_lut_defaults.meta(), [24, 0, 64, 0]);
    }

    #[test]
    fn programmatic_config_uses_catalog() {
        let cfg = load_str(
            r#"{:films (collect [_ f (ipairs dp2m.films)] (values f.slug (not= f.kind :mono)))
                :m5 {:grain-scale (/ 1 2)}}"#,
        )
        .unwrap();
        assert!(!cfg.film_enabled("mono-red-25") && cfg.film_enabled("velvia-100"));
        assert!((cfg.m5.grain_scale - 0.5).abs() < 1e-12);
    }

    #[test]
    fn rejects_typos_and_bad_papers() {
        assert!(load_str("{:verfy true}").is_err());
        assert!(load_str("{:films {:velvia-100 {:paper :kodak-2383}}}").is_err());
        assert!(load_str("{:films {:kodak-trix-400 {:paper :kodak-2383}}}").is_err());
        assert!(load_str("{:films {:kodak-trix-400 {:paper :selenium-tone}}}").is_ok());
        assert!(load_str("{:version \"1.04\"}").is_err());
    }
}
