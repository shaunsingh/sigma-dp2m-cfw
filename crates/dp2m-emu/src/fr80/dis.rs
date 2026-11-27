//! Big-endian FR disassembler for M5 firmware.

use std::fmt::Write;

const CC: [&str; 16] = [
    "ra", "no", "eq", "ne", "c", "nc", "n", "p",
    "v", "nv", "lt", "ge", "le", "gt", "ls", "hi",
];

fn dr_name(n: u8) -> &'static str {
    match n {
        0 => "TBR",
        1 => "RP",
        2 => "SSP",
        3 => "USP",
        4 => "MDH",
        5 => "MDL",
        _ => "DR?",
    }
}

fn dis_one(buf: &[u8], off: usize, va: u32) -> (usize, String) {
    if off + 2 > buf.len() {
        return (2, ".hword ???".to_string());
    }
    let op = u16::from_be_bytes([buf[off], buf[off + 1]]);
    let hi8 = (op >> 8) as u8;
    let lo8 = (op & 0xFF) as u8;
    let n3 = ((op >> 12) & 0xF) as u8;
    let n2 = ((op >> 8) & 0xF) as u8;
    let n1 = ((op >> 4) & 0xF) as u8;
    let n0 = (op & 0xF) as u8;

    let s8 = |v: u8| -> i32 { (v as i8) as i32 };
    let s11 = |v: u16| -> i32 {
        let x = v & 0x7FF;
        if x & 0x400 != 0 { x as i32 - 2048 } else { x as i32 }
    };

    if hi8 == 0x9F && n1 == 0x8 {
        let imm = if off + 6 <= buf.len() {
            u32::from_be_bytes([buf[off + 2], buf[off + 3], buf[off + 4], buf[off + 5]])
        } else {
            0
        };
        return (6, format!("ldi:32   #0x{:08x}, r{}", imm, n0));
    }
    if hi8 == 0x9B {
        let lo = if off + 4 <= buf.len() {
            u16::from_be_bytes([buf[off + 2], buf[off + 3]]) as u32
        } else {
            0
        };
        return (4, format!("ldi:20   #0x{:05x}, r{}", ((n1 as u32) << 16) | lo, n0));
    }
    if n3 == 0xC {
        return (2, format!("ldi:8    #0x{:02x}, r{}", (n2 << 4) | n1, n0));
    }
    if n3 == 0xD {
        let disp = s11((((n2 & 7) as u16) << 8) | lo8 as u16) * 2;
        let tgt = (va as i64 + 2 + disp as i64) as u32;
        let mnem = if n2 & 8 != 0 { "call:d" } else { "call" };
        return (2, format!("{:<8} 0x{:06x}", mnem, tgt));
    }
    if n3 == 0xE || n3 == 0xF {
        let tgt = (va as i64 + 2 + (s8(lo8) * 2) as i64) as u32;
        let suf = if n3 == 0xF { ":d" } else { "" };
        return (2, format!("b{}{:<6} 0x{:06x}", CC[n2 as usize], suf, tgt));
    }
    if n3 == 0x2 {
        return (2, format!("ld       @(r14, #{}), r{}", s8((n2 << 4) | n1) * 4, n0));
    }
    if n3 == 0x3 {
        return (2, format!("st       r{}, @(r14, #{})", n0, s8((n2 << 4) | n1) * 4));
    }

    let text = match hi8 {
        0x00 => format!("ld       @(r13, r{}), r{}", n1, n0),
        0x01 => format!("lduh     @(r13, r{}), r{}", n1, n0),
        0x02 => format!("ldub     @(r13, r{}), r{}", n1, n0),
        0x03 => format!("ld       @(r15, #{}), r{}", (n1 as u32) * 4, n0),
        0x04 => format!("ld       @r{}, r{}", n1, n0),
        0x05 => format!("lduh     @r{}, r{}", n1, n0),
        0x06 => format!("ldub     @r{}, r{}", n1, n0),
        0x07 if n1 == 0 => format!("ld       @r15+, r{}", n0),
        0x07 if n1 == 8 => format!("ld       @r15+, {}", dr_name(n0)),
        0x0F => format!("enter    #0x{:x}", (lo8 as u32) * 4),
        0x10 => format!("st       r{}, @(r13, r{})", n0, n1),
        0x11 => format!("sth      r{}, @(r13, r{})", n0, n1),
        0x12 => format!("stb      r{}, @(r13, r{})", n0, n1),
        0x13 => format!("st       r{}, @(r15, #{})", n0, (n1 as u32) * 4),
        0x14 => format!("st       r{}, @r{}", n0, n1),
        0x15 => format!("sth      r{}, @r{}", n0, n1),
        0x16 => format!("stb      r{}, @r{}", n0, n1),
        0x17 if n1 == 0 => format!("st       r{}, @-r15", n0),
        0x17 if n1 == 8 => format!("st       {}, @-r15", dr_name(n0)),
        0x82 => format!("and      r{}, r{}", n1, n0),
        0x8B => format!("mov      r{}, r{}", n1, n0),
        0x92 => format!("or       r{}, r{}", n1, n0),
        0x97 => match n1 {
            0x0 => format!("jmp      @r{}", n0),
            0x1 => format!("call     @r{}", n0),
            0x2 if n0 == 0 => "ret".to_string(),
            0x8 => format!("extsb    r{}", n0),
            0x9 => format!("extub    r{}", n0),
            0xA => format!("extsh    r{}", n0),
            0xB => format!("extuh    r{}", n0),
            _ => format!(".hword   0x{:04x}", op),
        },
        0x9A => format!("eor      r{}, r{}", n1, n0),
        0x9F if op == 0x9F90 => "leave".to_string(),
        0x9F if op == 0x9FA0 => "nop".to_string(),
        0xA3 => format!("addsp    #{}", s8(lo8) * 4),
        0xA4 => format!("add      #{}, r{}", n1, n0),
        0xA5 => format!("add2     #{}, r{}", (n1 as i32) - 16, n0),
        0xA6 => format!("add      r{}, r{}", n1, n0),
        0xA8 => format!("cmp      #{}, r{}", n1, n0),
        0xAA => format!("cmp      r{}, r{}", n1, n0),
        0xAC => format!("sub      r{}, r{}", n1, n0),
        0xAF => format!("mul      r{}, r{}", n1, n0),
        0xB0 => format!("lsr      #{}, r{}", n1, n0),
        0xB1 => format!("lsr2     #{}, r{}", (n1 as u32) + 16, n0),
        0xB4 => format!("lsl      #{}, r{}", n1, n0),
        0xB5 => format!("lsl2     #{}, r{}", (n1 as u32) + 16, n0),
        0xB7 => format!("mov      {}, r{}", dr_name(n1), n0),
        0xB8 => format!("asr      #{}, r{}", n1, n0),
        0xB9 => format!("asr2     #{}, r{}", (n1 as u32) + 16, n0),
        _ => format!(".hword   0x{:04x}", op),
    };
    (2, text)
}

pub fn dis_range(buf: &[u8], va_start: u32, va_end: u32, delta: u32) -> Vec<String> {
    let mut out = Vec::new();
    let mut va = va_start.max(delta);
    while va < va_end {
        let off = (va - delta) as usize;
        if off >= buf.len() {
            break;
        }
        let (sz, text) = dis_one(buf, off, va);
        let mut hex = String::with_capacity(sz * 2);
        for b in &buf[off..(off + sz).min(buf.len())] {
            write!(hex, "{b:02x}").unwrap();
        }
        out.push(format!("  0x{:06x}: {:<12}  {}", va, hex, text));
        va += sz as u32;
    }
    out
}
