//! Big-endian Thumb-1 & ARMv5TE disassembler with literal/call/string annotation.

use std::collections::HashMap;

use crate::image::{END, Image};

const COND: [&str; 16] = [
    "eq", "ne", "hs", "lo", "mi", "pl", "vs", "vc",
    "hi", "ls", "ge", "lt", "gt", "le", "", "nv",
];
const REG: [&str; 16] = [
    "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7",
    "r8", "r9", "r10", "r11", "ip", "sp", "lr", "pc",
];
const DP_OP: [&str; 16] = [
    "and", "eor", "sub", "rsb", "add", "adc", "sbc", "rsc",
    "tst", "teq", "cmp", "cmn", "orr", "mov", "bic", "mvn",
];
const SH_OP: [&str; 4] = ["lsl", "lsr", "asr", "ror"];

fn fmt_reglist(mask: u32) -> String {
    let mut parts = Vec::new();
    for (r, &name) in REG.iter().enumerate() {
        if (mask >> r) & 1 != 0 {
            parts.push(name);
        }
    }
    format!("{{{}}}", parts.join(", "))
}

fn note_val(img: &Image, names: &HashMap<u32, String>, v: u32) -> String {
    if let Some(n) = names.get(&(v & !1)) {
        return n.clone();
    }
    if let Some(s) = img.text_at(v, 3) {
        let cut: String = s.chars().take(48).collect();
        return format!("\"{cut}\"");
    }
    String::new()
}

pub fn dis_range(img: &Image, names: &HashMap<u32, String>, start: u32, end: u32, limit: Option<usize>, arm: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut va = start & if arm { !3 } else { !1 };
    let stop = end.min(END);
    let mut count = 0usize;

    while va < stop && limit.is_none_or(|n| count < n) {
        if let Some(lbl) = names.get(&va) {
            out.push(format!("{lbl}:"));
        }
        let (size, mn, ops, note) = if arm {
            dis_arm(img, names, va)
        } else {
            dis_thumb(img, names, va)
        };
        let raw_hex: String = img.rd(va, size).iter().map(|b| format!("{b:02x}")).collect();
        let line = if note.is_empty() {
            format!("{va:08x}: {raw_hex:<8} {mn:<7} {ops}")
        } else {
            format!("{va:08x}: {raw_hex:<8} {mn:<7} {ops}  ; {note}")
        };
        out.push(line);
        va += size as u32;
        count += 1;
    }
    out
}

fn dis_thumb(img: &Image, names: &HashMap<u32, String>, va: u32) -> (usize, String, String, String) {
    let op = img.u16(va) as u32;
    let top3 = op >> 13;
    match top3 {
        0b000 => {
            let sub = (op >> 11) & 3;
            let rd = (op & 7) as usize;
            let rs = ((op >> 3) & 7) as usize;
            if sub != 3 {
                let imm = (op >> 6) & 0x1F;
                let mn = ["lsls", "lsrs", "asrs"][sub as usize];
                (2, mn.into(), format!("{}, {}, #{imm}", REG[rd], REG[rs]), String::new())
            } else {
                let rn = (op >> 6) & 7;
                let mn = if (op >> 9) & 1 != 0 { "subs" } else { "adds" };
                let rhs = if (op >> 10) & 1 != 0 { format!("#{rn}") } else { REG[rn as usize].into() };
                (2, mn.into(), format!("{}, {}, {rhs}", REG[rd], REG[rs]), String::new())
            }
        }
        0b001 => {
            let mn = ["movs", "cmp", "adds", "subs"][((op >> 11) & 3) as usize];
            let rd = ((op >> 8) & 7) as usize;
            let imm = op & 0xFF;
            (2, mn.into(), format!("{}, #{imm:#x}", REG[rd]), String::new())
        }
        0b010 => match (op >> 10) & 7 {
            0 => {
                let alu = [
                    "ands", "eors", "lsls", "lsrs", "asrs", "adcs", "sbcs", "rors",
                    "tst", "rsbs", "cmp", "cmn", "orrs", "muls", "bics", "mvns",
                ][((op >> 6) & 0xF) as usize];
                let rs = ((op >> 3) & 7) as usize;
                let rd = (op & 7) as usize;
                (2, alu.into(), format!("{}, {}", REG[rd], REG[rs]), String::new())
            }
            1 => {
                let hi = (op >> 8) & 3;
                let rd = (((op >> 4) & 8) | (op & 7)) as usize;
                let rm = ((op >> 3) & 0xF) as usize;
                match hi {
                    0 => (2, "add".into(), format!("{}, {}", REG[rd], REG[rm]), String::new()),
                    1 => (2, "cmp".into(), format!("{}, {}", REG[rd], REG[rm]), String::new()),
                    2 => (2, "mov".into(), format!("{}, {}", REG[rd], REG[rm]), String::new()),
                    _ => {
                        let mn = if (op >> 7) & 1 != 0 { "blx" } else { "bx" };
                        (2, mn.into(), REG[rm].into(), String::new())
                    }
                }
            }
            2 | 3 => {
                let rd = ((op >> 8) & 7) as usize;
                let imm = (op & 0xFF) * 4;
                let slot = ((va + 4) & !3) + imm;
                let note = if slot + 4 <= END {
                    let v = img.u32(slot);
                    let extra = note_val(img, names, v);
                    if extra.is_empty() { format!("={v:#010x}") } else { format!("={v:#010x} {extra}") }
                } else {
                    String::new()
                };
                (2, "ldr".into(), format!("{}, [pc, #{imm:#x}]", REG[rd]), note)
            }
            _ => {
                let mn = ["str", "strh", "strb", "ldrsb", "ldr", "ldrh", "ldrb", "ldrsh"][((op >> 9) & 7) as usize];
                let ro = ((op >> 6) & 7) as usize;
                let rb = ((op >> 3) & 7) as usize;
                let rd = (op & 7) as usize;
                (2, mn.into(), format!("{}, [{}, {}]", REG[rd], REG[rb], REG[ro]), String::new())
            }
        },
        0b011 => {
            let sub = (op >> 11) & 3;
            let mn = ["str", "ldr", "strb", "ldrb"][sub as usize];
            let imm = ((op >> 6) & 0x1F) << if sub < 2 { 2 } else { 0 };
            let rb = ((op >> 3) & 7) as usize;
            let rd = (op & 7) as usize;
            (2, mn.into(), format!("{}, [{}, #{imm:#x}]", REG[rd], REG[rb]), String::new())
        }
        0b100 => {
            if (op >> 12) & 1 == 0 {
                let mn = if (op >> 11) & 1 != 0 { "ldrh" } else { "strh" };
                let imm = ((op >> 6) & 0x1F) << 1;
                let rb = ((op >> 3) & 7) as usize;
                let rd = (op & 7) as usize;
                (2, mn.into(), format!("{}, [{}, #{imm:#x}]", REG[rd], REG[rb]), String::new())
            } else {
                let mn = if (op >> 11) & 1 != 0 { "ldr" } else { "str" };
                let rd = ((op >> 8) & 7) as usize;
                let imm = (op & 0xFF) << 2;
                (2, mn.into(), format!("{}, [sp, #{imm:#x}]", REG[rd]), String::new())
            }
        }
        0b101 => {
            if (op >> 12) & 1 == 0 {
                let rd = ((op >> 8) & 7) as usize;
                let imm = (op & 0xFF) << 2;
                if (op >> 11) & 1 != 0 {
                    (2, "add".into(), format!("{}, sp, #{imm:#x}", REG[rd]), String::new())
                } else {
                    let tgt = ((va + 4) & !3) + imm;
                    let extra = note_val(img, names, tgt);
                    let note = if extra.is_empty() { format!("{tgt:#010x}") } else { format!("{tgt:#010x} {extra}") };
                    (2, "adr".into(), format!("{}, #{imm:#x}", REG[rd]), note)
                }
            } else {
                let mid = (op >> 8) & 0xF;
                if mid == 0 {
                    let mn = if (op >> 7) & 1 != 0 { "sub" } else { "add" };
                    (2, mn.into(), format!("sp, #{:#x}", (op & 0x7F) << 2), String::new())
                } else if (mid & 0b0110) == 0b0100 {
                    let is_pop = (op >> 11) & 1 != 0;
                    let mut mask = op & 0xFF;
                    if (op >> 8) & 1 != 0 {
                        mask |= 1 << if is_pop { 15 } else { 14 };
                    }
                    (2, if is_pop { "pop" } else { "push" }.into(), fmt_reglist(mask), String::new())
                } else {
                    (2, ".hword".into(), format!("{op:#06x}"), String::new())
                }
            }
        }
        0b110 => {
            if (op >> 12) & 1 == 0 {
                let mn = if (op >> 11) & 1 != 0 { "ldmia" } else { "stmia" };
                let rb = ((op >> 8) & 7) as usize;
                (2, mn.into(), format!("{}!, {}", REG[rb], fmt_reglist(op & 0xFF)), String::new())
            } else {
                let cond = ((op >> 8) & 0xF) as usize;
                if cond == 0xF {
                    (2, "svc".into(), format!("#{:#x}", op & 0xFF), String::new())
                } else if cond == 0xE {
                    (2, ".hword".into(), format!("{op:#06x}"), String::new())
                } else {
                    let simm = ((op as i8) as i32) << 1;
                    let tgt = va.wrapping_add(4).wrapping_add(simm as u32);
                    let note = names.get(&(tgt & !1)).cloned().unwrap_or_default();
                    (2, format!("b{}", COND[cond]), format!("#{tgt:#010x}"), note)
                }
            }
        }
        _ => {
            let sub = (op >> 11) & 3;
            match sub {
                0 => {
                    let simm = (((op & 0x7FF) as i32) << 21) >> 20;
                    let tgt = va.wrapping_add(4).wrapping_add(simm as u32);
                    let note = names.get(&(tgt & !1)).cloned().unwrap_or_default();
                    (2, "b".into(), format!("#{tgt:#010x}"), note)
                }
                2 if va + 4 <= END => {
                    let lo = img.u16(va + 2) as u32;
                    if let Some(tgt) = img.bl_target(va) {
                        let mn = if (lo & 0xF800) == 0xE800 { "blx" } else { "bl" };
                        let note = names.get(&(tgt & !1)).cloned().unwrap_or_default();
                        (4, mn.into(), format!("#{tgt:#010x}"), note)
                    } else {
                        (2, ".hword".into(), format!("{op:#06x}"), String::new())
                    }
                }
                _ => (2, ".hword".into(), format!("{op:#06x}"), String::new()),
            }
        }
    }
}

fn dis_arm(img: &Image, names: &HashMap<u32, String>, va: u32) -> (usize, String, String, String) {
    let w = img.u32(va);
    let c = ((w >> 28) & 0xF) as usize;
    let cc = COND[c];
    if c == 0xF {
        if (w & 0x0E00_0000) == 0x0A00_0000 {
            let h = (w >> 23) & 2;
            let off = (((w & 0x00FF_FFFF) << 2) | h) as i32;
            let simm = (off << 6) >> 6;
            let tgt = va.wrapping_add(8).wrapping_add(simm as u32);
            let note = names.get(&(tgt & !1)).cloned().unwrap_or_default();
            return (4, "blx".into(), format!("#{tgt:#010x}"), note);
        }
        return (4, ".word".into(), format!("{w:#010x}"), String::new());
    }
    if (w & 0x0FFF_FFF0) == 0x012F_FF10 {
        return (4, format!("bx{cc}"), REG[(w & 0xF) as usize].into(), String::new());
    }
    if (w & 0x0FFF_FFF0) == 0x012F_FF30 {
        return (4, format!("blx{cc}"), REG[(w & 0xF) as usize].into(), String::new());
    }
    if (w & 0x0F00_0010) == 0x0E00_0010 {
        let mn = if (w >> 20) & 1 != 0 { "mrc" } else { "mcr" };
        let ops = format!(
            "p{}, {}, {}, c{}, c{}, {}",
            (w >> 8) & 0xF,
            (w >> 21) & 7,
            REG[((w >> 12) & 0xF) as usize],
            (w >> 16) & 0xF,
            w & 0xF,
            (w >> 5) & 7
        );
        return (4, format!("{mn}{cc}"), ops, String::new());
    }
    if (w & 0x0E00_0090) == 0x0000_0090 {
        return dis_arm_ext(w, cc);
    }
    match (w >> 25) & 7 {
        0b101 => {
            let link = (w >> 24) & 1 != 0;
            let simm = (((w & 0x00FF_FFFF) as i32) << 8) >> 6;
            let tgt = va.wrapping_add(8).wrapping_add(simm as u32);
            let note = names.get(&(tgt & !1)).cloned().unwrap_or_default();
            (4, format!("{}{cc}", if link { "bl" } else { "b" }), format!("#{tgt:#010x}"), note)
        }
        0b100 => {
            let l = (w >> 20) & 1 != 0;
            let p = (w >> 24) & 1 != 0;
            let u = (w >> 23) & 1 != 0;
            let wb = if (w >> 21) & 1 != 0 { "!" } else { "" };
            let rn = ((w >> 16) & 0xF) as usize;
            let suf = match (p, u) { (false, false) => "da", (false, true) => "ia", (true, false) => "db", (true, true) => "ib" };
            (4, format!("{}{suf}{cc}", if l { "ldm" } else { "stm" }), format!("{}{wb}, {}", REG[rn], fmt_reglist(w & 0xFFFF)), String::new())
        }
        0b010 | 0b011 => {
            let l = (w >> 20) & 1 != 0;
            let b = if (w >> 22) & 1 != 0 { "b" } else { "" };
            let u = (w >> 23) & 1 != 0;
            let rn = ((w >> 16) & 0xF) as usize;
            let rd = ((w >> 12) & 0xF) as usize;
            let mn = format!("{}{b}{cc}", if l { "ldr" } else { "str" });
            if ((w >> 25) & 1) == 0 {
                let imm = w & 0xFFF;
                let sign = if u { "" } else { "-" };
                let note = if l && rn == 15 && b.is_empty() {
                    let slot = if u { va.wrapping_add(8).wrapping_add(imm) } else { va.wrapping_add(8).wrapping_sub(imm) };
                    if slot + 4 <= END {
                        let v = img.u32(slot);
                        let extra = note_val(img, names, v);
                        if extra.is_empty() { format!("={v:#010x}") } else { format!("={v:#010x} {extra}") }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                (4, mn, format!("{}, [{}, #{sign}{imm:#x}]", REG[rd], REG[rn]), note)
            } else {
                let rm = (w & 0xF) as usize;
                (4, mn, format!("{}, [{}, {}]", REG[rd], REG[rn], REG[rm]), String::new())
            }
        }
        0b000 | 0b001 => {
            let opc = ((w >> 21) & 0xF) as usize;
            let s = if (w >> 20) & 1 != 0 && !(8..=11).contains(&opc) { "s" } else { "" };
            let rn = ((w >> 16) & 0xF) as usize;
            let rd = ((w >> 12) & 0xF) as usize;
            let rhs = if ((w >> 25) & 1) != 0 {
                let rot = ((w >> 8) & 0xF) * 2;
                let imm = (w & 0xFF).rotate_right(rot);
                format!("#{imm:#x}")
            } else {
                let rm = (w & 0xF) as usize;
                let sh = ((w >> 5) & 3) as usize;
                let imm = (w >> 7) & 0x1F;
                if imm == 0 && sh == 0 { REG[rm].into() } else { format!("{}, {} #{imm}", REG[rm], SH_OP[sh]) }
            };
            let mn = format!("{}{s}{cc}", DP_OP[opc]);
            let ops = if opc == 13 || opc == 15 {
                format!("{}, {rhs}", REG[rd])
            } else if (8..=11).contains(&opc) {
                format!("{}, {rhs}", REG[rn])
            } else {
                format!("{}, {}, {rhs}", REG[rd], REG[rn])
            };
            (4, mn, ops, String::new())
        }
        _ => (4, ".word".into(), format!("{w:#010x}"), String::new()),
    }
}

/// ARM class 000 with bits 7 and 4 set: multiplies and halfword, signed, and doubleword transfers.
/// These share the data-processing class bits, so decoding them as ALU operations prints valid-looking nonsense.
fn dis_arm_ext(w: u32, cc: &str) -> (usize, String, String, String) {
    let rn = REG[((w >> 16) & 0xF) as usize];
    let rd = REG[((w >> 12) & 0xF) as usize];
    let rs = REG[((w >> 8) & 0xF) as usize];
    let rm = REG[(w & 0xF) as usize];
    let sh = (w >> 5) & 3;
    if sh == 0 {
        let s = if (w >> 20) & 1 != 0 { "s" } else { "" };
        return match (w >> 21) & 0xF {
            0 => (4, format!("mul{s}{cc}"), format!("{rn}, {rm}, {rs}"), String::new()),
            1 => (4, format!("mla{s}{cc}"), format!("{rn}, {rm}, {rs}, {rd}"), String::new()),
            op @ 4..=7 => {
                let mn = ["umull", "umlal", "smull", "smlal"][(op - 4) as usize];
                (4, format!("{mn}{s}{cc}"), format!("{rd}, {rn}, {rm}, {rs}"), String::new())
            }
            _ => (4, ".word".into(), format!("{w:#010x}"), String::new()),
        };
    }
    let mn = match ((w >> 20) & 1 != 0, sh) {
        (true, 1) => "ldrh",
        (true, 2) => "ldrsb",
        (true, _) => "ldrsh",
        (false, 1) => "strh",
        (false, 2) => "ldrd",
        (false, _) => "strd",
    };
    let sign = if (w >> 23) & 1 != 0 { "" } else { "-" };
    let off = if (w >> 22) & 1 != 0 {
        format!("#{sign}{:#x}", ((w >> 4) & 0xF0) | (w & 0xF))
    } else {
        format!("{sign}{rm}")
    };
    let ops = if (w >> 24) & 1 != 0 {
        format!("{rd}, [{rn}, {off}]{}", if (w >> 21) & 1 != 0 { "!" } else { "" })
    } else {
        format!("{rd}, [{rn}], {off}")
    };
    (4, format!("{mn}{cc}"), ops, String::new())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::dis_range;
    use crate::re::ReCtx;

    #[test]
    fn arm_decodes_coprocessor_and_extended_transfers() {
        let fw = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/DP2MV105.bin");
        let ctx = ReCtx::load(&fw, None).expect("stock DP2MV105.bin");
        let arm = |va: u32| dis_range(&ctx.img, &ctx.names, va, va + 4, None, true).join("\n");
        for (va, want) in [
            (0xA007_4610, "ldrd    r0, [r0, #0x0]"),              // get_tim reads the 64-bit tick
            (0xA007_4614, "strd    r0, [r4, #0x0]"),
            (0xA007_3B90, "ldrh    r0, [r0, #0x0]"),              // get_tid
            (0xA007_3B94, "strh    r0, [r4, #0x0]"),
            (0xA007_2A04, "ldrsh   r1, [r0, #-0x10]"),            // prcv_mbx existence check
            (0xA017_2174, "mcr     p15, 0, r0, c2, c0, 1"),       // boot MPU: instruction-cacheable regions
            (0xA017_21A8, "mcr     p15, 0, r0, c5, c0, 3"),       // boot MPU: instruction access permissions
            (0xA017_21DC, "mrc     p15, 0, r0, c1, c0, 0"),
            (0xA007_4600, "mvn     r1, #0x4d"),                   // plain data processing is unchanged
        ] {
            let got = arm(va);
            assert!(got.contains(want), "{va:#010x}: expected `{want}` in `{got}`");
        }
    }
}
