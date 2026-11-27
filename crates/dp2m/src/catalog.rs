//! film, paper, grid & setting catalog shared by the config loader, both chip builders & codegen

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilmKind {
    Quattro,
    OpticalMono,
    Reversal,
    Negative,
    BwSilver,
    SdLut,
}

impl FilmKind {
    pub const fn key(self) -> &'static str {
        match self {
            Self::Quattro => "quattro",
            Self::OpticalMono => "mono",
            Self::Reversal => "reversal",
            Self::Negative => "negative",
            Self::BwSilver => "bw",
            Self::SdLut => "sd-lut",
        }
    }

    pub const fn is_mono(self) -> bool {
        matches!(self, Self::OpticalMono | Self::BwSilver)
    }

    /// Paper menu shown for this kind: color print papers, darkroom papers, or SD LUT slots.
    pub const fn papers(self) -> Option<&'static [Paper]> {
        match self {
            Self::Negative => Some(COLOR_PAPERS),
            Self::BwSilver => Some(BW_PAPERS),
            Self::SdLut => Some(LUT_SLOTS),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Film {
    pub slug: &'static str,
    pub label: &'static str,
    pub kind: FilmKind,
}

#[derive(Debug, Clone, Copy)]
pub struct Paper {
    pub slug: &'static str,
    pub label: &'static str,
}

/// First on-camera color mode id; stock modes are 1..8.
pub const FILM_MODE0: u8 = 9;

macro_rules! films {
    ($($kind:ident $slug:literal $label:literal;)*) => {
        &[$(Film { slug: $slug, label: $label, kind: FilmKind::$kind },)*]
    };
}

/// Order defines on-camera color mode ids (`FILM_MODE0 + index`).
pub const FILMS: &[Film] = films! {
    Quattro "fov-classic-yellow" "FOV Classic Yellow";
    Quattro "cinema" "Cinema";
    Quattro "sunset-red" "Sunset Red";
    Quattro "forest-green" "Forest Green";
    Quattro "teal-and-orange" "Teal & Orange";
    OpticalMono "true-mono" "True Mono";
    OpticalMono "mono-yellow-8" "Mono Yellow #8";
    OpticalMono "mono-orange-21" "Mono Orange #21";
    OpticalMono "mono-red-25" "Mono Red #25";
    OpticalMono "mono-green-11" "Mono Green #11";
    Reversal "velvia-100" "Velvia 100";
    Reversal "provia-100f" "Provia 100F";
    Reversal "ektachrome-100" "Ektachrome 100";
    Reversal "kodachrome-64" "Kodachrome 64";
    Negative "portra-160" "Portra 160";
    Negative "portra-400" "Portra 400";
    Negative "portra-800" "Portra 800";
    Negative "portra-800-p1" "Portra 800 +1";
    Negative "portra-800-p2" "Portra 800 +2";
    Negative "gold-200" "Gold 200";
    Negative "ultramax-400" "UltraMax 400";
    Negative "ektar-100" "Ektar 100";
    Negative "verita-200d" "Verita 200D";
    Negative "vision3-50d" "Vision3 50D";
    Negative "vision3-250d" "Vision3 250D";
    Negative "vision3-200t" "Vision3 200T";
    Negative "vision3-500t" "Vision3 500T";
    Negative "pro-400h" "Pro 400H";
    Negative "superia-xtra" "Superia X-TRA";
    Negative "fujicolor-c200" "Fujicolor C200";
    BwSilver "kodak-trix-400" "Kodak Tri-X 400";
    BwSilver "ilford-hp5-plus" "Ilford HP5 Plus";
    BwSilver "fuji-acros-100" "Fuji Acros 100";
    BwSilver "kodak-tmax-100" "Kodak T-Max 100";
    BwSilver "rollei-retro-80s" "Rollei Retro 80S";
    SdLut "sd-custom-lut" "SD Custom LUT";
};

macro_rules! papers {
    ($($slug:literal $label:literal;)*) => { &[$(Paper { slug: $slug, label: $label },)*] };
}

/// M5Slave color paper slot order (D2MP pack order).
pub const COLOR_PAPERS: &[Paper] = papers! {
    "endura-premier" "Endura Premier";
    "ektacolor-edge" "Ektacolor Edge";
    "supra-endura" "Supra Endura";
    "portra-endura" "Portra Endura";
    "crystal-archive" "Crystal Archive";
    "kodak-2383" "Kodak 2383";
    "kodak-2393" "Kodak 2393";
    "ultra-endura" "Ultra Endura";
};

/// Darkroom paper / toner order (D2MP pack order).
pub const BW_PAPERS: &[Paper] = papers! {
    "neutral-fiber" "Neutral Fiber";
    "selenium-tone" "Selenium Tone";
    "warmtone-fiber" "Warmtone Fiber";
    "platinum-cool" "Platinum Cool";
    "grade4-punch" "Grade 4 Punch";
    "grade1-soft" "Grade 1 Soft";
    "split-sepia" "Split Sepia";
    "lith-print" "Lith Print";
};

pub const LUT_SLOTS: &[Paper] = papers! {
    "lut1" "LUT1.BIN"; "lut2" "LUT2.BIN"; "lut3" "LUT3.BIN"; "lut4" "LUT4.BIN";
    "lut5" "LUT5.BIN"; "lut6" "LUT6.BIN"; "lut7" "LUT7.BIN"; "lut8" "LUT8.BIN";
};

#[derive(Debug, Clone, Copy)]
pub struct Grid {
    pub slug: &'static str,
    pub label: &'static str,
}

/// Order defines on-camera grid ids (`GRID_MODE0 + index`); stock grids are 0..6.
pub const GRID_MODE0: u8 = 7;
pub const GRIDS: &[Grid] = &[
    Grid { slug: "level", label: "Electronic Level" },
    Grid { slug: "xpan-21x9", label: "21:9 XPan + Level" },
    Grid { slug: "cinema-16x9", label: "16:9 Cinema + Level" },
    Grid { slug: "square-1x1", label: "1:1 Square + Level" },
    Grid { slug: "peaking", label: "Focus Peaking" },
    Grid { slug: "peaking-level", label: "Peaking + Level" },
    Grid { slug: "zebras", label: "Zebras" },
    Grid { slug: "rgb-hist", label: "RGB Hist + Level" },
    Grid { slug: "frame-7x6", label: "7:6 + Level" },
    Grid { slug: "frame-4x3", label: "4:3 + Level" },
    Grid { slug: "false-color", label: "False Color (IRE)" },
];

pub fn film(slug: &str) -> Option<(usize, &'static Film)> {
    FILMS.iter().enumerate().find(|(_, f)| f.slug == slug)
}

pub fn films_of(kind: FilmKind) -> impl Iterator<Item = (usize, &'static Film)> {
    FILMS.iter().enumerate().filter(move |(_, f)| f.kind == kind)
}

pub fn paper_index(papers: &[Paper], slug: &str) -> Option<u8> {
    papers.iter().position(|p| p.slug == slug).map(|i| i as u8)
}

/// IPC/M5 packed mode for (film index, 0-based paper); mirrors `packed_cm` in src/m6m.
pub fn packed_mode(film_idx: usize, paper: u8) -> u8 {
    let f = &FILMS[film_idx];
    let k = films_of(f.kind).position(|(i, _)| i == film_idx).unwrap() as u8;
    let mode = FILM_MODE0 + film_idx as u8;
    match f.kind {
        FilmKind::Negative => PACKED_NEG0 + k * 8 + paper,
        FilmKind::BwSilver => PACKED_BW0 + k * 8 + paper,
        FilmKind::SdLut => PACKED_SD0 + paper,
        _ => mode,
    }
}

pub const PACKED_NEG0: u8 = 23;
pub const PACKED_BW0: u8 = 151;
pub const PACKED_SD0: u8 = 191;
pub const PACKED_MAX: u8 = 198;

/// Q7 R/G/B luma weights of the optical-filter mono films (true, #8, #21, #25, #11).
pub const MONO_FILTER_Q7: [(i16, i16, i16); 5] = [(26, 51, 51), (46, 58, 24), (60, 52, 16), (72, 46, 10), (22, 82, 24)];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_layout_is_dense() {
        assert_eq!(FILMS.len(), 36);
        assert_eq!(packed_mode(film("velvia-100").unwrap().0, 0), 19);
        assert_eq!(packed_mode(film("portra-160").unwrap().0, 0), PACKED_NEG0);
        assert_eq!(packed_mode(film("fujicolor-c200").unwrap().0, 7), PACKED_BW0 - 1);
        assert_eq!(packed_mode(film("kodak-trix-400").unwrap().0, 0), PACKED_BW0);
        assert_eq!(packed_mode(film("rollei-retro-80s").unwrap().0, 7), PACKED_SD0 - 1);
        assert_eq!(packed_mode(film("sd-custom-lut").unwrap().0, 7), PACKED_MAX);
    }
}
