//! One-pass S3 xref indexer & static Thumb NVRAM register-tracking scanner.

use std::collections::{BTreeMap, BTreeSet};

use crate::image::{BASE, CODE_END, END, FPGA, Image, SDRAM_END};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RefKind {
    Tbl, Tblx, Tb, Tbc, Tlit, Tadr,
    Abl, Ablx, Ab, Alit, Aadr,
    W, W2,
}

impl RefKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tbl => "tbl", Self::Tblx => "tblx", Self::Tb => "tb", Self::Tbc => "tbc",
            Self::Tlit => "tlit", Self::Tadr => "tadr", Self::Abl => "abl", Self::Ablx => "ablx",
            Self::Ab => "ab", Self::Alit => "alit", Self::Aadr => "aadr", Self::W => "w", Self::W2 => "w2",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "tbl" => Self::Tbl, "tblx" => Self::Tblx, "tb" => Self::Tb, "tbc" => Self::Tbc,
            "tlit" => Self::Tlit, "tadr" => Self::Tadr, "abl" => Self::Abl, "ablx" => Self::Ablx,
            "ab" => Self::Ab, "alit" => Self::Alit, "aadr" => Self::Aadr, "w" => Self::W, "w2" => Self::W2,
            _ => return None,
        })
    }

    pub fn is_call(self) -> bool {
        matches!(self, Self::Tbl | Self::Tblx | Self::Abl | Self::Ablx)
    }

    pub fn is_data_ref(self) -> bool {
        matches!(self, Self::Tlit | Self::Alit | Self::Tadr | Self::Aadr | Self::W)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Xref {
    pub src: u32,
    pub val: u32,
    pub slot: u32,
    pub kind: RefKind,
}

pub struct XrefIndex {
    by_val: Vec<Xref>,
}

impl XrefIndex {
    /// Builds full S3 reference index in a single linear pass (~15 ms in release).
    pub fn build(img: &Image) -> Self {
        let s3 = img.s3();
        let code_len = (CODE_END - BASE) as usize;
        let mut refs = Vec::with_capacity(600_000);

        let rd32 = |va: u32| -> Option<u32> {
            let off = va.checked_sub(BASE)? as usize;
            (off + 4 <= s3.len()).then(|| u32::from_be_bytes(s3[off..off + 4].try_into().unwrap()))
        };

        // Thumb scan over code region
        let n_half = code_len / 2;
        for i in 0..n_half {
            let va = BASE + (i * 2) as u32;
            let h = u16::from_be_bytes([s3[i * 2], s3[i * 2 + 1]]) as u32;
            if (h & 0xF800) == 0xF000 && i + 1 < n_half {
                let lo = u16::from_be_bytes([s3[i * 2 + 2], s3[i * 2 + 3]]) as u32;
                if (lo & 0xE800) == 0xE800 {
                    let off = ((h & 0x7FF) << 12) | ((lo & 0x7FF) << 1);
                    let sext = ((off << 9) as i32) >> 9;
                    let t = va.wrapping_add(4).wrapping_add(sext as u32);
                    if (lo & 0xF800) == 0xE800 {
                        refs.push(Xref { src: va, val: t & !3, slot: va, kind: RefKind::Tblx });
                    } else {
                        refs.push(Xref { src: va, val: t, slot: va, kind: RefKind::Tbl });
                    }
                }
            } else if (h & 0xF800) == 0xE000 {
                let sext = (((h & 0x7FF) as i32) << 21) >> 20;
                refs.push(Xref { src: va, val: va.wrapping_add(4).wrapping_add(sext as u32), slot: va, kind: RefKind::Tb });
            } else if (h & 0xF000) == 0xD000 && (h & 0x0F00) < 0x0E00 {
                let sext = ((h as i8) as i32) << 1;
                refs.push(Xref { src: va, val: va.wrapping_add(4).wrapping_add(sext as u32), slot: va, kind: RefKind::Tbc });
            } else if (h & 0xF800) == 0x4800 {
                let slot = ((va + 4) & !3) + (h & 0xFF) * 4;
                if let Some(v) = rd32(slot) {
                    refs.push(Xref { src: va, val: v, slot, kind: RefKind::Tlit });
                }
            } else if (h & 0xF800) == 0xA000 {
                let target = ((va + 4) & !3) + (h & 0xFF) * 4;
                refs.push(Xref { src: va, val: target, slot: va, kind: RefKind::Tadr });
            }
        }

        // ARM scan over code region
        for i in 0..(code_len / 4) {
            let va = BASE + (i * 4) as u32;
            let x = u32::from_be_bytes(s3[i * 4..i * 4 + 4].try_into().unwrap());
            if (x & 0x0E00_0000) == 0x0A00_0000 {
                let sext = (((x & 0x00FF_FFFF) as i32) << 8) >> 6;
                let t = va.wrapping_add(8).wrapping_add(sext as u32);
                let kind = if (x >> 28) == 0xF {
                    refs.push(Xref { src: va, val: t.wrapping_add((x >> 23) & 2), slot: va, kind: RefKind::Ablx });
                    continue;
                } else if (x & 0x0100_0000) != 0 {
                    RefKind::Abl
                } else {
                    RefKind::Ab
                };
                refs.push(Xref { src: va, val: t, slot: va, kind });
            } else if (x & 0x0FEF_0000) == 0x028F_0000 {
                let rot = ((x >> 8) & 0xF) * 2;
                let imm = (x & 0xFF).rotate_right(rot);
                let target = if ((x >> 21) & 0xF) == 4 {
                    va.wrapping_add(8).wrapping_add(imm)
                } else {
                    va.wrapping_add(8).wrapping_sub(imm)
                };
                refs.push(Xref { src: va, val: target, slot: va, kind: RefKind::Aadr });
            } else if (x & 0x0F3F_0000) == 0x051F_0000 {
                let imm = x & 0xFFF;
                let slot = if (x & 0x0080_0000) != 0 {
                    va.wrapping_add(8).wrapping_add(imm)
                } else {
                    va.wrapping_add(8).wrapping_sub(imm)
                };
                if let Some(v) = rd32(slot) {
                    refs.push(Xref { src: va, val: v, slot, kind: RefKind::Alit });
                }
            }
        }

        // SDRAM pointer words outside FPGA bitstream
        for i in 0..(s3.len() / 4) {
            let va = BASE + (i * 4) as u32;
            if (FPGA.0..FPGA.1).contains(&va) {
                continue;
            }
            let w = u32::from_be_bytes(s3[i * 4..i * 4 + 4].try_into().unwrap());
            if (BASE..SDRAM_END).contains(&w) {
                refs.push(Xref { src: va, val: w, slot: va, kind: RefKind::W });
            }
        }
        // Halfword-aligned SDRAM pointers past FPGA bitstream
        let start_w2 = ((FPGA.1 - BASE) as usize).max(2);
        let mut o = start_w2 | 2;
        while o + 4 <= s3.len() {
            let w = u32::from_be_bytes(s3[o..o + 4].try_into().unwrap());
            if (BASE..SDRAM_END).contains(&w) {
                let va = BASE + o as u32;
                refs.push(Xref { src: va, val: w, slot: va, kind: RefKind::W2 });
            }
            o += 4;
        }

        refs.sort_unstable_by_key(|r| (r.val, r.src));
        Self { by_val: refs }
    }

    pub fn refs(&self, value: u32, filter: Option<&[RefKind]>) -> Vec<Xref> {
        let mut out = BTreeSet::new();
        for &v in &[value & !1, value | 1] {
            let lo = self.by_val.partition_point(|r| r.val < v);
            for r in &self.by_val[lo..] {
                if r.val != v {
                    break;
                }
                if filter.is_none_or(|f| f.contains(&r.kind)) {
                    out.insert(*r);
                }
            }
        }
        // Also literal loads whose pool slot equals `value`
        if filter.is_none_or(|f| f.contains(&RefKind::Tlit) || f.contains(&RefKind::Alit)) {
            for r in &self.by_val {
                if r.slot == value && r.slot != r.src && filter.is_none_or(|f| f.contains(&r.kind)) {
                    out.insert(*r);
                }
            }
        }
        let mut v: Vec<Xref> = out.into_iter().collect();
        v.sort_by_key(|r| (r.src, r.kind));
        v
    }

    pub fn count_data_refs(&self, value: u32) -> usize {
        let mut n = 0;
        for &v in &[value & !1, value | 1] {
            let lo = self.by_val.partition_point(|r| r.val < v);
            for r in &self.by_val[lo..] {
                if r.val != v {
                    break;
                }
                if r.kind.is_data_ref() {
                    n += 1;
                }
            }
        }
        n
    }

    pub fn tlit_in_range(&self, lo_val: u32, hi_val: u32) -> Vec<Xref> {
        let lo = self.by_val.partition_point(|r| r.val < lo_val);
        let mut out: Vec<Xref> = self.by_val[lo..]
            .iter()
            .take_while(|r| r.val < hi_val)
            .filter(|r| r.kind == RefKind::Tlit && (BASE..CODE_END).contains(&r.src))
            .copied()
            .collect();
        out.sort_by_key(|r| r.src);
        out
    }
}

/// Nearest preceding Thumb `push {.., lr}` within `back` bytes.
pub fn fn_start(img: &Image, va: u32, back: u32) -> Option<u32> {
    let lo = BASE.max(va.saturating_sub(back));
    let mut a = va & !1;
    while a > lo {
        if (img.u16(a) & 0xFF00) == 0xB500 {
            return Some(a);
        }
        a -= 2;
    }
    None
}

#[derive(Clone, Debug)]
pub struct NvAccess {
    pub addr: u32,
    pub size: u32,
    pub kind: String,
    pub va: u32,
}

#[derive(Clone, Debug)]
pub struct NvCall {
    pub va: u32,
    pub target: Option<u32>,
    pub args: [Option<u32>; 3],
}

/// Thumb register-tracking NVRAM coverage scanner.
pub fn nvram_scan(img: &Image, lo: u32, hi: u32, idx: &XrefIndex, maxn: usize) -> (Vec<NvAccess>, Vec<NvCall>) {
    let mut acc = Vec::new();
    let mut calls = Vec::new();
    let win = |v: Option<u32>| v.is_some_and(|x| (lo..hi).contains(&x));

    for seed in idx.tlit_in_range(lo.saturating_sub(0x100), hi.saturating_add(0x100)) {
        let mut regs: [Option<u32>; 16] = [None; 16];
        let rd0 = ((img.u16(seed.src) >> 8) & 7) as usize;
        regs[rd0] = Some(seed.val);

        let mut va = seed.src + 2;
        for _ in 0..maxn {
            if va >= CODE_END || !regs.iter().any(|&v| win(v)) {
                break;
            }
            let op = img.u16(va) as u32;
            let top3 = op >> 13;
            let mut nxt = va + 2;

            match top3 {
                0b000 => {
                    let sub = (op >> 11) & 3;
                    let rd = (op & 7) as usize;
                    let rs = ((op >> 3) & 7) as usize;
                    if sub == 0 {
                        let imm = (op >> 6) & 0x1F;
                        regs[rd] = regs[rs].map(|a| a.wrapping_shl(imm));
                    } else if sub == 3 {
                        let rn_imm = (op >> 6) & 7;
                        let rhs = if (op >> 10) & 1 != 0 { Some(rn_imm) } else { regs[rn_imm as usize] };
                        regs[rd] = match (regs[rs], rhs) {
                            (Some(a), Some(b)) => Some(if (op >> 9) & 1 != 0 { a.wrapping_sub(b) } else { a.wrapping_add(b) }),
                            _ => None,
                        };
                    } else {
                        regs[rd] = None;
                    }
                }
                0b001 => {
                    let sub = (op >> 11) & 3;
                    let rd = ((op >> 8) & 7) as usize;
                    let imm = op & 0xFF;
                    match sub {
                        0 => regs[rd] = Some(imm),
                        1 => {} // cmp
                        2 => regs[rd] = regs[rd].map(|a| a.wrapping_add(imm)),
                        _ => regs[rd] = regs[rd].map(|a| a.wrapping_sub(imm)),
                    }
                }
                0b010 => match (op >> 10) & 7 {
                    0 => {
                        let alu = (op >> 6) & 0xF;
                        let rs = ((op >> 3) & 7) as usize;
                        let rd = (op & 7) as usize;
                        match alu {
                            0x8 | 0xA | 0xB => {}
                            0xD => regs[rd] = regs[rd].zip(regs[rs]).map(|(a, b)| a.wrapping_mul(b)),
                            _ => regs[rd] = None,
                        }
                    }
                    1 => {
                        let hi_op = (op >> 8) & 3;
                        let rd = (((op >> 4) & 8) | (op & 7)) as usize;
                        let rm = ((op >> 3) & 0xF) as usize;
                        match hi_op {
                            0 => regs[rd] = regs[rd].zip(regs[rm]).map(|(a, b)| a.wrapping_add(b)),
                            1 => {}
                            2 => regs[rd] = regs[rm],
                            _ => break, // bx / blx reg
                        }
                    }
                    2 | 3 => {
                        let rd = ((op >> 8) & 7) as usize;
                        let slot = ((va + 4) & !3) + (op & 0xFF) * 4;
                        regs[rd] = (slot + 4 <= END).then(|| img.u32(slot));
                    }
                    _ => {
                        let rb = ((op >> 3) & 7) as usize;
                        let rd = (op & 7) as usize;
                        let opc = (op >> 9) & 7;
                        let (kind, sz, is_load) = match opc {
                            0 => ("str", 4, false), 1 => ("strh", 2, false), 2 => ("strb", 1, false),
                            3 => ("ldrsb", 1, true), 4 => ("ldr", 4, true), 5 => ("ldrh", 2, true),
                            6 => ("ldrb", 1, true), _ => ("ldrsh", 2, true),
                        };
                        if let Some(base) = regs[rb]
                            && win(Some(base))
                        {
                            acc.push(NvAccess { addr: base, size: sz, kind: format!("{kind}[idx]"), va });
                        }
                        if is_load {
                            regs[rd] = None;
                        }
                    }
                },
                0b011 => {
                    let imm = (op >> 6) & 0x1F;
                    let rb = ((op >> 3) & 7) as usize;
                    let rd = (op & 7) as usize;
                    let (kind, sz, disp, is_load) = match (op >> 11) & 3 {
                        0 => ("str", 4, imm << 2, false),
                        1 => ("ldr", 4, imm << 2, true),
                        2 => ("strb", 1, imm, false),
                        _ => ("ldrb", 1, imm, true),
                    };
                    if let Some(base) = regs[rb]
                        && win(Some(base))
                    {
                        acc.push(NvAccess { addr: base.wrapping_add(disp), size: sz, kind: kind.into(), va });
                    }
                    if is_load {
                        regs[rd] = None;
                    }
                }
                0b100 => {
                    if (op >> 12) & 1 == 0 {
                        let imm = ((op >> 6) & 0x1F) << 1;
                        let rb = ((op >> 3) & 7) as usize;
                        let rd = (op & 7) as usize;
                        let is_load = (op >> 11) & 1 != 0;
                        if let Some(base) = regs[rb]
                            && win(Some(base))
                        {
                            acc.push(NvAccess {
                                addr: base.wrapping_add(imm),
                                size: 2,
                                kind: if is_load { "ldrh" } else { "strh" }.into(),
                                va,
                            });
                        }
                        if is_load {
                            regs[rd] = None;
                        }
                    } else if (op >> 11) & 1 != 0 {
                        regs[((op >> 8) & 7) as usize] = None;
                    }
                }
                0b101 => {
                    if (op >> 12) & 1 == 0 {
                        regs[((op >> 8) & 7) as usize] = None;
                    } else if (op & 0x0600) == 0x0400 && (op >> 11) & 1 != 0 {
                        // POP
                        for (r, slot) in regs.iter_mut().enumerate().take(8) {
                            if (op >> r) & 1 != 0 {
                                *slot = None;
                            }
                        }
                        if (op >> 8) & 1 != 0 {
                            break; // pop {.., pc}
                        }
                    }
                }
                0b110 => {
                    if (op >> 12) & 1 == 0 {
                        let rb = ((op >> 8) & 7) as usize;
                        let list = op & 0xFF;
                        let is_load = (op >> 11) & 1 != 0;
                        let cnt = list.count_ones();
                        if let Some(base) = regs[rb]
                            && win(Some(base))
                        {
                            for k in 0..cnt {
                                acc.push(NvAccess {
                                    addr: base + 4 * k,
                                    size: 4,
                                    kind: if is_load { "ldmia" } else { "stmia" }.into(),
                                    va,
                                });
                            }
                        }
                        if is_load {
                            for (r, slot) in regs.iter_mut().enumerate().take(8) {
                                if (list >> r) & 1 != 0 {
                                    *slot = None;
                                }
                            }
                        }
                        if let Some(b) = regs[rb]
                            && (!is_load || (list >> rb) & 1 == 0)
                        {
                            regs[rb] = Some(b.wrapping_add(4 * cnt));
                        }
                    }
                }
                _ => {
                    let sub = (op >> 11) & 3;
                    if sub == 0 {
                        break; // unconditional b
                    } else if sub == 2 && va + 2 < CODE_END {
                        let lo_h = img.u16(va + 2) as u32;
                        if (lo_h & 0xE800) == 0xE800 {
                            calls.push(NvCall {
                                va,
                                target: img.bl_target(va),
                                args: [regs[0], regs[1], regs[2]],
                            });
                            for r in [0usize, 1, 2, 3, 12, 14] {
                                regs[r] = None;
                            }
                            nxt = va + 4;
                        }
                    }
                }
            }
            va = nxt;
        }
    }
    (acc, calls)
}

pub fn nvram_report(img: &Image, acc: &[NvAccess], calls: &[NvCall], block: u32, len: usize, defaults: u32, offsets: &[u32]) -> Vec<String> {
    let mut cov: BTreeMap<u32, Vec<(String, u32)>> = BTreeMap::new();
    for x in acc {
        for i in 0..x.size {
            cov.entry(x.addr + i).or_default().push((x.kind.clone(), x.va));
        }
    }
    let mut lines = vec!["calls with window arguments:".to_owned()];
    for c in calls {
        let fmt = |v: Option<u32>| v.map_or_else(|| "?".into(), |x| format!("{x:#x}"));
        lines.push(format!("  {:#x} -> {} r0={} r1={} r2={}", c.va, fmt(c.target), fmt(c.args[0]), fmt(c.args[1]), fmt(c.args[2])));
    }
    lines.push(format!("block offsets without static field access (+0..+{len:#x}):"));
    let mut start: Option<usize> = None;
    for o in 0..=len {
        let uncov = o < len && !cov.contains_key(&(block + o as u32));
        if uncov && start.is_none() {
            start = Some(o);
        } else if !uncov
            && let Some(s) = start.take()
        {
            let d = img.rd(defaults + s as u32, (o - s).min(24));
            let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
            lines.push(format!("  +{s:#05x}..+{o:#05x} ({:4} B) @ {:#x}  defaults {hex}", o - s, block + s as u32));
        }
    }
    lines.push("accesses outside the block:".to_owned());
    let mut out_addrs: BTreeSet<u32> = BTreeSet::new();
    for x in acc {
        if !(block..block + len as u32).contains(&x.addr) {
            out_addrs.insert(x.addr);
        }
    }
    for a in out_addrs {
        let users: Vec<String> = cov[&a]
            .iter()
            .map(|(_, v)| format!("{v:#x}"))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .take(6)
            .collect();
        lines.push(format!("  {a:#x} by {users:?}"));
    }
    for &o in offsets {
        let mut items: Vec<(String, u32)> = cov.get(&(block + o)).cloned().unwrap_or_default();
        items.sort_by_key(|x| (x.1, x.0.clone()));
        items.dedup();
        let s: Vec<String> = items.into_iter().map(|(k, v)| format!("{k}@{v:#x}")).collect();
        lines.push(format!("+{o:#x} @ {:#x}: {}", block + o, s.join(" ")));
    }
    lines
}
