//! Recursive-descent M6M code map, function liveness, and dead-region finder.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::image::{BASE, BSS, CODE_END, FPGA, Image, S3_LEN};

pub const MODE_NONE: u8 = 0;
pub const MODE_ARM: u8 = 1;
pub const MODE_THUMB: u8 = 2;
pub const MODE_LIT: u8 = 3;

const ROOTS: &[(u32, bool)] = &[(0xA000_0000, false), (0xA016_57EA, true)]; // (va, is_thumb): reset, main
const VECTORS: (u32, u32) = (0xA07F_BC1C, 0xA07F_BC1C + 0x338);
const SWITCH8: (u32, u32) = (0xE55E_C001, 0xE12F_FF1C); // __ARM_common_switch8 words 0 & 5

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Origin {
    Root,
    Call,
    PcRel,
    Ptr,
    Orphan,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EdgeKind {
    Call,
    Jmp,
    Lit,
    LitSlot,
}

#[inline]
fn rd16(s3: &[u8], va: u32) -> u16 {
    let o = (va - BASE) as usize;
    u16::from_be_bytes([s3[o], s3[o + 1]])
}

#[inline]
fn rd32(s3: &[u8], va: u32) -> u32 {
    let o = (va - BASE) as usize;
    u32::from_be_bytes([s3[o], s3[o + 1], s3[o + 2], s3[o + 3]])
}

#[inline]
fn in_code(va: u32) -> bool {
    (BASE..CODE_END).contains(&va)
}

#[inline]
fn in_s3(va: u32) -> bool {
    (BASE..BASE + S3_LEN as u32).contains(&va)
}

#[inline]
fn looks_arm(s3: &[u8], va: u32) -> bool {
    if (va & 3) != 0 || !(BASE..CODE_END - 12).contains(&va) {
        return false;
    }
    (rd32(s3, va) >> 28) == 0xE
        && (rd32(s3, va + 4) >> 28) == 0xE
        && (rd32(s3, va + 8) >> 28) == 0xE
}

#[inline]
fn rot_imm(w: u32) -> u32 {
    let rot = ((w >> 8) & 0xF) * 2;
    (w & 0xFF).rotate_right(rot)
}

#[inline]
fn in_any(va: u32, ranges: &[(u32, u32)]) -> bool {
    ranges.iter().any(|&(lo, hi)| va >= lo && va < hi)
}

pub struct CodeMap {
    pub mode: Vec<u8>,
    pub entry: HashMap<u32, (bool, Origin)>, // va -> (is_thumb, origin)
    pub edges: HashMap<u32, Vec<(u32, EdgeKind)>>,
    pub root: HashMap<u32, u32>,
    pub black: HashSet<u32>,
    cur: u32,
    sw_cache: HashMap<u32, bool>,
}

impl CodeMap {
    fn new(black: HashSet<u32>) -> Self {
        Self {
            mode: vec![0; S3_LEN],
            entry: HashMap::with_capacity(16384),
            edges: HashMap::with_capacity(65536),
            root: HashMap::with_capacity(65536),
            black,
            cur: 0,
            sw_cache: HashMap::new(),
        }
    }

    fn is_switch8(&mut self, s3: &[u8], t: u32) -> bool {
        if let Some(&v) = self.sw_cache.get(&t) {
            return v;
        }
        let v = in_code(t)
            && (t & 3) == 0
            && t + 24 <= CODE_END
            && (rd32(s3, t), rd32(s3, t + 20)) == SWITCH8;
        self.sw_cache.insert(t, v);
        v
    }

    fn add_edge(&mut self, src: u32, tgt: u32, kind: EdgeKind) {
        let e = self.edges.entry(src).or_default();
        if !e.contains(&(tgt, kind)) {
            e.push((tgt, kind));
        }
    }

    fn mark(&mut self, va: u32, n: usize, v: u8) {
        if v == MODE_ARM || v == MODE_THUMB {
            self.root.entry(va).or_insert(self.cur);
        }
        let o = (va - BASE) as usize;
        let end = (o + n).min(S3_LEN);
        for slot in &mut self.mode[o..end] {
            if *slot == MODE_NONE || ((v == MODE_ARM || v == MODE_THUMB) && *slot == MODE_LIT) {
                *slot = v;
            }
        }
    }

    fn add_fn(&mut self, va: u32, is_thumb: bool, wl: &mut VecDeque<(u32, bool)>, origin: Origin) {
        if !in_code(va) {
            return;
        }
        self.entry.entry(va).or_insert((is_thumb, origin));
        wl.push_back((va, is_thumb));
    }

    fn lit(&mut self, s3: &[u8], src: u32, la: u32) -> Option<u32> {
        if la < BASE || la > BASE + S3_LEN as u32 - 4 {
            return None;
        }
        self.mark(la, 4, MODE_LIT);
        let v = rd32(s3, la);
        self.add_edge(src, la, EdgeKind::LitSlot);
        if in_s3(v) {
            self.add_edge(src, v, EdgeKind::Lit);
        }
        Some(v)
    }

    fn run(&mut self, s3: &[u8], items: &[(u32, bool)], origin: Origin) {
        let mut wl = VecDeque::new();
        for &(va, is_thumb) in items {
            if self.black.contains(&va) {
                continue;
            }
            self.cur = va;
            self.add_fn(va, is_thumb, &mut wl, origin);
            while let Some((v, th)) = wl.pop_front() {
                if th {
                    self.walk_t(s3, v, &mut wl);
                } else {
                    self.walk_a(s3, v, &mut wl);
                }
            }
        }
    }

    fn walk_t(&mut self, s3: &[u8], mut va: u32, wl: &mut VecDeque<(u32, bool)>) {
        let mut regs = [None::<u32>; 16];
        while in_code(va) {
            let m = self.mode[(va - BASE) as usize];
            if m == MODE_ARM || m == MODE_THUMB {
                return;
            }
            let hw = rd16(s3, va) as u32;
            if (hw & 0xF800) == 0xF000 {
                if va + 4 > CODE_END {
                    return;
                }
                let lo = rd16(s3, va + 2) as u32;
                if (lo & 0xE800) != 0xE800 {
                    return;
                }
                let off = ((hw & 0x7FF) << 12) | ((lo & 0x7FF) << 1);
                let sext = ((off << 9) as i32) >> 9;
                let mut t = va.wrapping_add(4).wrapping_add(sext as u32);
                self.mark(va, 4, MODE_THUMB);
                if (lo & 0xF800) == 0xE800 {
                    t &= !3;
                    self.add_edge(va, t, EdgeKind::Call);
                    self.add_fn(t, false, wl, Origin::Call);
                    if self.is_switch8(s3, t) {
                        let ret = va + 4;
                        let n = s3[(ret - BASE) as usize] as usize;
                        self.mark(ret, n + 2, MODE_LIT);
                        for i in 0..=n {
                            let off_b = s3[(ret + 1 + i as u32 - BASE) as usize] as u32;
                            let tt = ret + 2 * off_b;
                            self.add_edge(va, tt | 1, EdgeKind::Jmp);
                            wl.push_back((tt, true));
                        }
                        return;
                    }
                } else {
                    self.add_edge(va, t | 1, EdgeKind::Call);
                    self.add_fn(t, true, wl, Origin::Call);
                }
                va += 4;
                regs = [None; 16];
                continue;
            }
            if (hw & 0xF800) == 0xE800 || (hw & 0xF800) == 0xF800 {
                return;
            }
            self.mark(va, 2, MODE_THUMB);
            if (hw & 0xF800) == 0xE000 {
                let sext = (((hw & 0x7FF) as i32) << 21) >> 20;
                let t = va.wrapping_add(4).wrapping_add(sext as u32);
                self.add_edge(va, t | 1, EdgeKind::Jmp);
                wl.push_back((t, true));
                return;
            }
            if (hw & 0xF000) == 0xD000 {
                let c = (hw >> 8) & 0xF;
                if c == 0xE {
                    return;
                }
                if c != 0xF {
                    let sext = ((hw as i8) as i32) << 1;
                    let t = va.wrapping_add(4).wrapping_add(sext as u32);
                    self.add_edge(va, t | 1, EdgeKind::Jmp);
                    wl.push_back((t, true));
                }
                va += 2;
                continue;
            }
            if hw == 0x4778 {
                // bx pc
                wl.push_back(((va + 4) & !3, false));
                return;
            }
            if (hw & 0xFF00) == 0x4700 {
                if (hw & 0x80) != 0 {
                    va += 2;
                    continue;
                }
                return;
            }
            if (hw & 0xFF00) == 0xBD00 {
                return;
            }
            if (hw & 0xFF87) == 0x4687 || (hw & 0xFF87) == 0x4487 {
                return;
            }
            if (hw & 0xFF78) == 0x4478 {
                let rd = ((hw & 7) | ((hw >> 4) & 8)) as usize;
                if let Some(rv) = regs[rd] {
                    let t = rv.wrapping_add(va + 4);
                    if in_s3(t) {
                        self.add_edge(va, t, EdgeKind::Lit);
                        if in_code(t) && (t & 1) != 0 {
                            self.add_fn(t & !1, true, wl, Origin::PcRel);
                        }
                    }
                }
            }
            if (hw & 0xF800) == 0x4800 {
                let la = ((va + 4) & !3) + (hw & 0xFF) * 4;
                if let Some(v) = self.lit(s3, va, la) {
                    regs[((hw >> 8) & 7) as usize] = Some(v);
                }
            } else if (hw & 0xF800) == 0xA000 {
                let t = ((va + 4) & !3) + (hw & 0xFF) * 4;
                if in_s3(t) {
                    self.add_edge(va, t, EdgeKind::Lit);
                }
            }
            va += 2;
        }
    }

    fn walk_a(&mut self, s3: &[u8], mut va: u32, wl: &mut VecDeque<(u32, bool)>) {
        va &= !3;
        let mut regs = [None::<u32>; 16];
        while in_code(va) {
            let o = (va - BASE) as usize;
            if self.mode[o] == MODE_ARM || self.mode[o] == MODE_THUMB {
                return;
            }
            let w = rd32(s3, va);
            let cond = w >> 28;
            self.mark(va, 4, MODE_ARM);
            if cond == 0xF {
                if (w & 0x0E00_0000) == 0x0A00_0000 {
                    let off = ((w & 0x00FF_FFFF) << 2) | ((w >> 23) & 2);
                    let sext = ((off << 6) as i32) >> 6;
                    let t = va.wrapping_add(8).wrapping_add(sext as u32);
                    self.add_edge(va, t | 1, EdgeKind::Call);
                    self.add_fn(t, true, wl, Origin::Call);
                    va += 4;
                    continue;
                }
                if (w & 0x0D70_F000) == 0x0550_F000 {
                    va += 4;
                    continue;
                }
                self.mode[o..o + 4].fill(MODE_NONE);
                return;
            }
            if (w & 0x0E00_0000) == 0x0A00_0000 {
                let sext = (((w & 0x00FF_FFFF) as i32) << 8) >> 6;
                let t = va.wrapping_add(8).wrapping_add(sext as u32);
                if (w & 0x0100_0000) != 0 {
                    self.add_edge(va, t, EdgeKind::Call);
                    self.add_fn(t, false, wl, Origin::Call);
                } else {
                    self.add_edge(va, t, EdgeKind::Jmp);
                    wl.push_back((t, false));
                    if cond == 0xE {
                        return;
                    }
                }
                va += 4;
                continue;
            }
            if (w & 0x0FFF_FFF0) == 0x012F_FF10 {
                if cond == 0xE {
                    return;
                }
                va += 4;
                continue;
            }
            if (w & 0x0FFF_FFF0) == 0x012F_FF30 {
                va += 4;
                continue;
            }
            if (w & 0x0FFF_FFF0) == 0x008F_F100 {
                // add pc, pc, rm, lsl #2: inline branch table
                let mut p = va + 4;
                for _ in 0..1024 {
                    if !in_code(p) {
                        break;
                    }
                    let ww = rd32(s3, p);
                    if (ww & 0xFF00_0000) != 0xEA00_0000 {
                        break;
                    }
                    let sext = (((ww & 0x00FF_FFFF) as i32) << 8) >> 6;
                    let t = p.wrapping_add(8).wrapping_add(sext as u32);
                    self.mark(p, 4, MODE_ARM);
                    self.add_edge(p, t, EdgeKind::Jmp);
                    wl.push_back((t, false));
                    p += 4;
                }
                return;
            }
            if (w & 0x0FFF_FFF0) == 0x079F_F100 {
                // ldr pc, [pc, rm, lsl #2]: inline pointer table
                let mut p = va + 8;
                for _ in 0..1024 {
                    if !in_code(p) || self.mode[(p - BASE) as usize] != MODE_NONE {
                        break;
                    }
                    let v = rd32(s3, p);
                    if !in_code(v) {
                        break;
                    }
                    self.mark(p, 4, MODE_LIT);
                    self.add_edge(va, v, EdgeKind::Jmp);
                    wl.push_back((v & !1, (v & 1) != 0));
                    p += 4;
                }
                va += 4;
                continue;
            }
            let rd = ((w >> 12) & 0xF) as usize;
            let rn = ((w >> 16) & 0xF) as usize;
            let dp = (w & 0x0C00_0000) == 0;
            let dp_test = dp && (w & 0x0180_0000) == 0x0100_0000;
            let mul = (w & 0x0E00_0090) == 0x0000_0090;
            if (w & 0x0F3F_0000) == 0x051F_0000 {
                let imm = w & 0xFFF;
                let la = if (w & 0x0080_0000) != 0 { va.wrapping_add(8).wrapping_add(imm) } else { va.wrapping_add(8).wrapping_sub(imm) };
                let v = self.lit(s3, va, la);
                if rd == 15 {
                    if let Some(vv) = v
                        && in_code(vv)
                    {
                        self.add_edge(va, vv, EdgeKind::Jmp);
                        wl.push_back((vv & !1, (vv & 1) != 0));
                    }
                    if cond == 0xE {
                        return;
                    }
                } else if let Some(vv) = v {
                    regs[rd] = Some(vv);
                }
                va += 4;
                continue;
            }
            if dp && !mul && !dp_test && (w & 0x0200_0000) == 0 && ((w >> 21) & 0xF) == 4 && (w & 0xFF0) == 0 {
                let rm = (w & 0xF) as usize;
                let other = if rn == 15 { Some(rm) } else if rm == 15 { Some(rn) } else { None };
                if let Some(o_reg) = other
                    && let Some(rv) = regs[o_reg]
                {
                    let t = rv.wrapping_add(va + 8);
                    if in_s3(t) {
                        self.add_edge(va, t, EdgeKind::Lit);
                        if in_code(t) {
                            self.add_fn(t & !1, (t & 1) != 0, wl, Origin::PcRel);
                        }
                    }
                }
            }
            if (w & 0x0FEF_0000) == 0x028F_0000 {
                let imm = rot_imm(w);
                let t = if ((w >> 21) & 0xF) == 4 { va.wrapping_add(8).wrapping_add(imm) } else { va.wrapping_add(8).wrapping_sub(imm) };
                if in_s3(t) {
                    self.add_edge(va, t, EdgeKind::Lit);
                }
            }
            if (rd == 15 && !dp_test && !mul && (dp || (w & 0x0C00_0000) == 0x0400_0000))
                || (w & 0x0E10_8000) == 0x0810_8000
            {
                if cond == 0xE {
                    return;
                }
            } else if (w & 0x0E00_0010) == 0x0600_0010 {
                self.mode[o..o + 4].fill(MODE_NONE);
                return;
            }
            va += 4;
        }
    }
}

/// Collects `(src_va, u32_be_val)` for every halfword-aligned word outside `FPGA` pointing into `[BASE, CODE_END)`.
fn code_ptr_words(s3: &[u8]) -> Vec<(u32, u32)> {
    let mut out = Vec::with_capacity(65536);
    let mut o = 0usize;
    while o + 4 <= s3.len() {
        let va = BASE + o as u32;
        if va == FPGA.0 {
            o = ((FPGA.1 - BASE) as usize + 1) & !1;
            continue;
        }
        let w = u32::from_be_bytes([s3[o], s3[o + 1], s3[o + 2], s3[o + 3]]);
        if in_code(w) {
            out.push((va, w));
        }
        o += 2;
    }
    out
}

pub fn build(s3: &[u8], black: HashSet<u32>) -> CodeMap {
    let mut cm = CodeMap::new(black);
    let mut roots: Vec<(u32, bool)> = ROOTS.to_vec();
    let mut a = VECTORS.0;
    while a < VECTORS.1 {
        let v = rd32(s3, a);
        if in_code(v) {
            roots.push((v & !1, (v & 1) != 0));
        }
        a += 4;
    }
    cm.run(s3, &roots, Origin::Root);

    let ptrs = code_ptr_words(s3);
    let mut done: HashSet<u32> = HashSet::with_capacity(16384);
    loop {
        let mut arm = Vec::new();
        let mut th_cand = Vec::new();
        for &(src, v) in &ptrs {
            if src < CODE_END {
                let m = cm.mode[(src - BASE) as usize];
                if m == MODE_ARM || m == MODE_THUMB {
                    continue;
                }
            }
            if !done.insert(v) {
                continue;
            }
            if (v & 1) != 0 {
                let t = v & !1;
                if cm.mode[(t - BASE) as usize] == MODE_NONE && !looks_arm(s3, t & !3) {
                    th_cand.push(t);
                }
            } else if looks_arm(s3, v) && cm.mode[(v - BASE) as usize] == MODE_NONE {
                arm.push((v, false));
            }
        }
        if arm.is_empty() && th_cand.is_empty() {
            break;
        }
        cm.run(s3, &arm, Origin::Ptr);
        let mut th = Vec::new();
        for t in th_cand {
            if cm.mode[(t - BASE) as usize] != MODE_NONE {
                continue;
            }
            let aligned = t & !3;
            let ok = if aligned >= BASE + 4 && cm.mode[(aligned - 4 - BASE) as usize] == MODE_ARM {
                let w = rd32(s3, aligned - 4);
                (w >> 28) == 0xE
                    && ((w & 0x0F00_0000) == 0x0A00_0000
                        || (w & 0x0FFF_FFF0) == 0x012F_FF10
                        || (w & 0x0E10_8000) == 0x0810_8000)
            } else {
                cm.mode[(aligned - BASE) as usize] != MODE_ARM
                    && (aligned + 4 >= CODE_END || cm.mode[(aligned + 4 - BASE) as usize] != MODE_ARM)
            };
            if ok {
                th.push((t, true));
            }
        }
        cm.run(s3, &th, Origin::Ptr);
        if arm.is_empty() && th.is_empty() {
            break;
        }
    }

    // Orphan prologues: Thumb push {.., lr} (0xB5xx) and ARM stmdb sp!, {.., lr} (0xE92D_4xxx)
    let lim = (CODE_END - BASE) as usize - 4;
    let mut o = 0usize;
    while o < lim {
        if cm.mode[o] == MODE_NONE {
            let va = BASE + o as u32;
            if s3[o] == 0xB5 {
                if !looks_arm(s3, va & !3) {
                    cm.run(s3, &[(va, true)], Origin::Orphan);
                }
            } else if (o & 3) == 0 && s3[o] == 0xE9 && s3[o + 1] == 0x2D && (s3[o + 2] & 0x40) != 0 {
                cm.run(s3, &[(va, false)], Origin::Orphan);
            }
        }
        o += 2;
    }
    cm
}

fn collisions(s3: &[u8], cm: &CodeMap) -> Vec<u32> {
    let mut out = Vec::new();
    let lim = (CODE_END - BASE) as usize - 8;
    let mut a = 0usize;
    while a < lim {
        if cm.mode[a] == MODE_ARM && cm.mode[a + 4] == MODE_THUMB {
            let w = u32::from_be_bytes([s3[a], s3[a + 1], s3[a + 2], s3[a + 3]]);
            let term = (w >> 28) == 0xE
                && ((w & 0x0F00_0000) == 0x0A00_0000
                    || (w & 0x0FFF_FFF0) == 0x012F_FF10
                    || (w & 0x0E10_8000) == 0x0810_8000
                    || ((w >> 12) & 0xF) == 15);
            if !term {
                out.push(BASE + a as u32 + 4);
            }
        }
        a += 4;
    }
    out
}

pub fn build_clean(s3: &[u8]) -> CodeMap {
    let mut black = HashSet::new();
    let mut cm = build(s3, black.clone());
    for _ in 0..12 {
        let mut added = false;
        for c in collisions(s3, &cm) {
            if let Some(&r) = cm.root.get(&c)
                && matches!(cm.entry.get(&r).map(|x| x.1), Some(Origin::Ptr | Origin::Orphan))
                && black.insert(r)
            {
                added = true;
            }
        }
        if !added {
            return cm;
        }
        cm = build(s3, black.clone());
    }
    cm
}

pub struct Liveness {
    pub cm: CodeMap,
    pub ents: Vec<u32>,
    pub live: HashSet<u32>,
    pub roots: HashSet<u32>,
}

impl Liveness {
    pub fn fn_of(&self, va: u32) -> Option<u32> {
        let va = va & !1;
        if !in_code(va) {
            return None;
        }
        let i = self.ents.partition_point(|&e| e <= va);
        (i > 0).then(|| self.ents[i - 1])
    }

    pub fn fn_end(&self, e: u32) -> u32 {
        let i = self.ents.partition_point(|&x| x <= e);
        if i < self.ents.len() { self.ents[i] } else { CODE_END }
    }
}

pub fn analyse(img: &Image, dead_data: &[(u32, u32)]) -> Liveness {
    let s3 = img.s3();
    let cm = build_clean(s3);
    let mut ents: Vec<u32> = cm.entry.keys().copied().collect();
    ents.sort_unstable();

    let fn_of = |va: u32| -> Option<u32> {
        let va = va & !1;
        if !in_code(va) {
            return None;
        }
        let i = ents.partition_point(|&e| e <= va);
        (i > 0).then(|| ents[i - 1])
    };

    let mut out: HashMap<u32, Vec<u32>> = HashMap::with_capacity(ents.len());
    let mut link = |f: Option<u32>, t: u32| {
        if let (Some(ff), Some(gg)) = (f, fn_of(t))
            && ff != gg
        {
            out.entry(ff).or_default().push(gg);
        }
    };

    for (&src, ts) in &cm.edges {
        let f = fn_of(src);
        for &(t, k) in ts {
            if k != EdgeKind::LitSlot {
                link(f, t);
            }
        }
    }
    let code_len = (CODE_END - BASE) as usize;
    let mut a = 0usize;
    while a + 4 <= code_len {
        if cm.mode[a] == MODE_LIT {
            let v = u32::from_be_bytes([s3[a], s3[a + 1], s3[a + 2], s3[a + 3]]);
            link(fn_of(BASE + a as u32), v);
        }
        a += 4;
    }

    // Conservative linear sweep of undecoded bytes inside each function extent
    for (i, &e) in ents.iter().enumerate() {
        let end = if i + 1 < ents.len() { ents[i + 1] } else { CODE_END };
        let is_thumb = cm.entry[&e].0;
        let mut va = e;
        while va + 3 < end {
            let o = (va - BASE) as usize;
            if cm.mode[o] != MODE_NONE {
                va += 2;
                continue;
            }
            if is_thumb {
                let hw = rd16(s3, va) as u32;
                if (hw & 0xF800) == 0xF000 {
                    let lo = rd16(s3, va + 2) as u32;
                    if (lo & 0xE800) == 0xE800 {
                        let off = ((hw & 0x7FF) << 12) | ((lo & 0x7FF) << 1);
                        let sext = ((off << 9) as i32) >> 9;
                        link(Some(e), va.wrapping_add(4).wrapping_add(sext as u32));
                    }
                } else if (hw & 0xF800) == 0x4800 {
                    let la = ((va + 4) & !3) + (hw & 0xFF) * 4;
                    if la + 4 <= BASE + S3_LEN as u32 {
                        link(Some(e), rd32(s3, la));
                    }
                }
                va += 2;
            } else if (va & 3) != 0 {
                va += 2;
            } else {
                let w = rd32(s3, va);
                if (w & 0x0E00_0000) == 0x0A00_0000 {
                    let sext = (((w & 0x00FF_FFFF) as i32) << 8) >> 6;
                    link(Some(e), va.wrapping_add(8).wrapping_add(sext as u32));
                } else if (w & 0x0F3F_0000) == 0x051F_0000 {
                    let imm = w & 0xFFF;
                    let la = if (w & 0x0080_0000) != 0 { va.wrapping_add(8).wrapping_add(imm) } else { va.wrapping_add(8).wrapping_sub(imm) };
                    if la >= BASE && la + 4 <= BASE + S3_LEN as u32 {
                        link(Some(e), rd32(s3, la));
                    }
                }
                va += 4;
            }
        }
    }

    let mut roots: HashSet<u32> = cm
        .entry
        .iter()
        .filter_map(|(&e, &(_, o))| (o == Origin::Root).then_some(e))
        .collect();
    for (src, val) in code_ptr_words(s3) {
        if in_any(src, dead_data) {
            continue;
        }
        if src < CODE_END && cm.mode[(src - BASE) as usize] != MODE_NONE {
            continue;
        }
        let tm = cm.mode[((val & !1) - BASE) as usize];
        let ok = ((val & 1) == 1 && tm == MODE_THUMB) || ((val & 3) == 0 && tm == MODE_ARM);
        if ok && let Some(f) = fn_of(val) {
            roots.insert(f);
        }
    }

    let mut live = HashSet::with_capacity(ents.len());
    let mut dq: VecDeque<u32> = roots.iter().copied().collect();
    while let Some(f) = dq.pop_front() {
        if live.insert(f)
            && let Some(nbrs) = out.get(&f)
        {
            for &g in nbrs {
                if !live.contains(&g) {
                    dq.push_back(g);
                }
            }
        }
    }

    Liveness { cm, ents, live, roots }
}

pub fn code_runs(img: &Image, lv: &Liveness, used: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let s3 = img.s3();
    let code_len = (CODE_END - BASE) as usize;
    let mut keep = vec![0u8; code_len];
    for &e in &lv.live {
        let a = (e - BASE) as usize;
        let b = (lv.fn_end(e) - BASE) as usize;
        keep[a..b].fill(1);
    }
    for (&src, ts) in &lv.cm.edges {
        if !lv.fn_of(src).is_some_and(|f| lv.live.contains(&f)) {
            continue;
        }
        for &(raw_t, k) in ts {
            let t = raw_t & !1;
            if !in_code(t) || keep[(t - BASE) as usize] != 0 {
                continue;
            }
            let o = (t - BASE) as usize;
            let n = match k {
                EdgeKind::LitSlot => 4,
                EdgeKind::Lit => {
                    let lim = (o + 512).min(s3.len());
                    if let Some(rel) = s3[o..lim].iter().position(|&b| b == 0)
                        && rel > 0
                        && s3[o..o + rel].iter().all(|&c| (32..127).contains(&c) || matches!(c, 9 | 10 | 13))
                    {
                        (rel + 4) & !3
                    } else {
                        4
                    }
                }
                _ => 2,
            };
            let end = (o + n).min(code_len);
            keep[o..end].fill(1);
        }
    }
    for &(lo, hi) in used {
        let a = (lo.max(BASE) - BASE) as usize;
        let b = (hi.min(CODE_END) - BASE) as usize;
        if a < b {
            keep[a..b].fill(1);
        }
    }
    let mut runs = Vec::new();
    let mut i = 0;
    while i < code_len {
        if keep[i] == 0 {
            let start = i;
            while i < code_len && keep[i] == 0 {
                i += 1;
            }
            runs.push((BASE + start as u32, BASE + i as u32));
        } else {
            i += 1;
        }
    }
    runs
}

pub fn data_runs(img: &Image, lv: &Liveness, dead_data: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let s3 = img.s3();
    let lo_d = FPGA.1;
    let mut live_ref: BTreeMap<u32, bool> = BTreeMap::new();
    let mut mark_ref = |t: u32, is_live: bool| {
        if (lo_d..BSS).contains(&t) {
            let e = live_ref.entry(t).or_insert(false);
            *e |= is_live;
        }
    };

    for (&src, ts) in &lv.cm.edges {
        let f_live = lv.fn_of(src).is_some_and(|f| lv.live.contains(&f));
        for &(t, k) in ts {
            if k == EdgeKind::Lit {
                mark_ref(t, f_live);
            }
        }
    }
    let code_len = (CODE_END - BASE) as usize;
    let mut a = 0usize;
    while a + 4 <= code_len {
        if lv.cm.mode[a] == MODE_LIT {
            let f_live = lv.fn_of(BASE + a as u32).is_some_and(|f| lv.live.contains(&f));
            let v = u32::from_be_bytes([s3[a], s3[a + 1], s3[a + 2], s3[a + 3]]);
            mark_ref(v, f_live);
        }
        a += 4;
    }

    let mut d = ((lo_d - BASE) as usize + 3) & !3;
    let bss_o = (BSS - BASE) as usize;
    while d + 4 <= bss_o {
        let va = BASE + d as u32;
        if !in_any(va, dead_data) {
            let w = u32::from_be_bytes([s3[d], s3[d + 1], s3[d + 2], s3[d + 3]]);
            if (lo_d..BSS).contains(&w) {
                mark_ref(w, true);
            }
        }
        d += 4;
    }

    let keys: Vec<(u32, bool)> = live_ref.into_iter().collect();
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for i in 0..keys.len() {
        let (a, is_live) = keys[i];
        if is_live {
            continue;
        }
        let b = if i + 1 < keys.len() { keys[i + 1].0 } else { BSS };
        if let Some(last) = runs.last_mut()
            && last.1 == a
        {
            last.1 = b;
        } else {
            runs.push((a, b));
        }
    }
    runs
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub start: u32,
    pub end: u32,
    pub kind: &'static str,
    pub fns: usize,
    pub hint: String,
}

impl Candidate {
    pub fn size(&self) -> u32 {
        self.end - self.start
    }

    pub fn tsv(&self) -> String {
        format!("{:#x}\t{:#x}\t{}\t{}\t{}\t{}", self.start, self.end, self.size(), self.kind, self.fns, self.hint)
    }
}

pub const TSV_HEADER: &str = "start\tend\tsize\tkind\tfns\thint";

fn ascii_hint(slice: &[u8]) -> String {
    let mut i = 0;
    while i < slice.len() {
        let b = slice[i];
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            i += 1;
            while i < slice.len() && (0x20..=0x7E).contains(&slice[i]) {
                i += 1;
            }
            if i - start >= 10 {
                let end = (start + 60).min(i);
                return String::from_utf8_lossy(&slice[start..end]).into_owned();
            }
        } else {
            i += 1;
        }
    }
    String::new()
}

pub fn candidates(
    img: &Image,
    lv: &Liveness,
    stock: Option<&Liveness>,
    min_size: u32,
    dead_data: &[(u32, u32)],
    used: &[(u32, u32)],
) -> Vec<Candidate> {
    let s3 = img.s3();
    let mut out = Vec::new();
    for (a, b) in code_runs(img, lv, used) {
        if b - a < min_size {
            continue;
        }
        let lo = lv.ents.partition_point(|&e| e < a);
        let hi = lv.ents.partition_point(|&e| e < b);
        let fs = &lv.ents[lo..hi];
        let newly = stock.is_some_and(|st| fs.iter().any(|f| st.live.contains(f)));
        let hint = ascii_hint(&s3[(a - BASE) as usize..(b - BASE) as usize]);
        out.push(Candidate {
            start: a,
            end: b,
            kind: if newly { "code:newly-dead" } else { "code:orphan" },
            fns: fs.len(),
            hint,
        });
    }
    for (a, b) in data_runs(img, lv, dead_data) {
        if b - a >= min_size {
            out.push(Candidate { start: a, end: b, kind: "data:dead-refs-only", fns: 0, hint: String::new() });
        }
    }
    out.sort_by_key(|x| std::cmp::Reverse(x.size()));
    out
}
