//! DP2M firmware containers, ELF/symbol helpers, and PNG output for verifier artifacts.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

pub const PKG_MAGIC: &[u8] = b"SIGMA.CODP2M";
pub const C78_MAGIC: &[u8] = b"FUJITSU!";
pub const C78_OFF: usize = 0x010A_4100;
pub const C78_LEN: usize = 0x0082_0000;
pub const IPL_LEN: usize = 0x0002_0000;
pub const BASE: u32 = 0xA000_0000;
pub const S3_LEN: usize = 0x0080_0000;
pub const END: u32 = BASE + S3_LEN as u32;
pub const CODE_END: u32 = 0xA017_9000; // Spartan-6 bitstream starts here
pub const FPGA: (u32, u32) = (0xA017_9000, 0xA023_D35A);
pub const BSS: u32 = 0xA07F_BC1C;
pub const SDRAM_END: u32 = 0xB000_0000;

pub const VERSION_FIELDS: [usize; 3] = [0x10, 0x30, 0x40];

#[derive(Clone)]
pub struct Package {
    pub raw: Vec<u8>,
}

impl Package {
    pub fn from_bytes(raw: Vec<u8>) -> Result<Self, String> {
        if !raw.starts_with(PKG_MAGIC) || raw.len() < C78_OFF + C78_LEN {
            return Err("not a DP2M updater package".into());
        }
        Ok(Self { raw })
    }

    pub fn field(&self, off: usize, n: usize) -> String {
        let s = &self.raw[off..off + n];
        let end = s.iter().position(|&b| b == 0).unwrap_or(n);
        s[..end].iter().map(|&b| b as char).collect()
    }

    pub fn versions(&self) -> [String; 3] {
        VERSION_FIELDS.map(|o| self.field(o, 16))
    }

    pub fn version(&self) -> String {
        self.field(VERSION_FIELDS[0], 16)
    }

    pub fn date(&self) -> String {
        self.field(0x50, 8)
    }

    pub fn c78_off(&self) -> usize {
        u32::from_le_bytes(self.raw[0x64..0x68].try_into().unwrap()) as usize
    }

    pub fn flags(&self) -> u8 {
        self.raw[0x70]
    }

    pub fn segments(&self) -> [(&'static str, usize, usize); 4] {
        [
            ("header", 0, 0x80),
            ("bridge", 0x80, 0x0082_0000),
            ("m5", 0x0082_0080, 0x0088_4080),
            ("m6m", self.c78_off(), C78_LEN),
        ]
    }

    pub fn segment(&self, name: &str) -> &[u8] {
        let (_, off, len) = self.segments().into_iter().find(|(n, _, _)| *n == name).unwrap();
        &self.raw[off..off + len]
    }
}

#[derive(Clone)]
pub struct Image {
    pub c78: Vec<u8>,
    pub pkg: Option<Package>,
}

impl Image {
    pub fn load(path: &Path) -> Result<Self, String> {
        let raw = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        Self::from_bytes(raw)
    }

    pub fn from_bytes(raw: Vec<u8>) -> Result<Self, String> {
        if raw.starts_with(PKG_MAGIC) {
            let pkg = Package::from_bytes(raw)?;
            let c78 = pkg.segment("m6m").to_vec();
            return Ok(Self { c78, pkg: Some(pkg) });
        }
        if raw.len() != C78_LEN || !raw.starts_with(C78_MAGIC) {
            return Err("not a DP2M package or C78FRM.bin".into());
        }
        Ok(Self { c78: raw, pkg: None })
    }

    #[inline]
    pub fn s3(&self) -> &[u8] {
        &self.c78[IPL_LEN..IPL_LEN + S3_LEN]
    }

    #[inline]
    pub fn rd(&self, va: u32, n: usize) -> &[u8] {
        let o = (va - BASE) as usize;
        &self.s3()[o..o + n]
    }

    #[inline]
    pub fn u8(&self, va: u32) -> u8 {
        self.s3()[(va - BASE) as usize]
    }

    #[inline]
    pub fn u16(&self, va: u32) -> u16 {
        let o = (va - BASE) as usize;
        u16::from_be_bytes([self.s3()[o], self.s3()[o + 1]])
    }

    #[inline]
    pub fn u32(&self, va: u32) -> u32 {
        let o = (va - BASE) as usize;
        u32::from_be_bytes(self.s3()[o..o + 4].try_into().unwrap())
    }

    pub fn cstr(&self, va: u32, max: usize) -> String {
        let o = (va - BASE) as usize;
        let lim = max.min(S3_LEN - o);
        let slice = &self.s3()[o..o + lim];
        let end = slice.iter().position(|&b| b == 0).unwrap_or(lim);
        String::from_utf8_lossy(&slice[..end]).into_owned()
    }

    pub fn bl_target(&self, va: u32) -> Option<u32> {
        let hi = self.u16(va) as u32;
        let lo = self.u16(va + 2) as u32;
        if (hi & 0xF800) != 0xF000 || (lo & 0xE800) != 0xE800 {
            return None;
        }
        let off = ((hi & 0x7FF) << 12) | ((lo & 0x7FF) << 1);
        let sext = ((off << 9) as i32) >> 9;
        let t = va.wrapping_add(4).wrapping_add(sext as u32);
        Some(if (lo & 0xF800) == 0xE800 { t & !3 } else { t })
    }

    pub fn text_at(&self, va: u32, min_len: usize) -> Option<String> {
        if !(BASE..END).contains(&va) {
            return None;
        }
        let s = self.cstr(va, 80);
        (s.len() >= min_len && s.bytes().all(|b| (32..127).contains(&b))).then_some(s)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Sym {
    pub addr: u32,
    pub size: u32,
}

impl Sym {
    #[inline]
    pub fn va(self) -> u32 {
        self.addr & !1
    }
}

/// Parses `m6m.syms` (`"<hex_addr> <dec_size> <name>"`), dropping ELF `$a/$t/$d` mapping symbols.
pub fn load_syms(path: &Path) -> Result<HashMap<String, Sym>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut out = HashMap::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(a), Some(n), Some(name)) = (it.next(), it.next(), it.next()) else { continue };
        if name.starts_with('$') {
            continue;
        }
        let addr = u32::from_str_radix(a, 16).map_err(|_| format!("bad sym addr '{a}'"))?;
        let size = n.parse::<u32>().map_err(|_| format!("bad sym size '{n}'"))?;
        out.insert(name.to_owned(), Sym { addr, size });
    }
    Ok(out)
}

/// Extracts a named section from a big-endian 32-bit ELF (`m6m.elf`).
pub fn elf_section<'a>(raw: &'a [u8], want: &[u8]) -> Result<&'a [u8], String> {
    let be32 = |o: usize| u32::from_be_bytes(raw[o..o + 4].try_into().unwrap()) as usize;
    let be16 = |o: usize| u16::from_be_bytes(raw[o..o + 2].try_into().unwrap()) as usize;
    let shoff = be32(0x20);
    let shentsize = be16(0x2E);
    let shnum = be16(0x30);
    let shstrndx = be16(0x32);
    let str_hdr = shoff + shstrndx * shentsize;
    let strtab = &raw[be32(str_hdr + 16)..be32(str_hdr + 16) + be32(str_hdr + 20)];
    for i in 0..shnum {
        let h = shoff + i * shentsize;
        let name_off = be32(h);
        let end = strtab[name_off..].iter().position(|&b| b == 0).unwrap_or(0);
        if &strtab[name_off..name_off + end] == want {
            let off = be32(h + 16);
            let sz = be32(h + 20);
            return Ok(&raw[off..off + sz]);
        }
    }
    Err(format!("ELF section {:?} not found", String::from_utf8_lossy(want)))
}

/// Encode a compact RGB8 PNG using PNG Sub/Up filtering and RFC 1951 fixed-Huffman LZ77 (`std` only).
pub fn write_rgb_png(
    mut out: impl Write,
    width: u32,
    height: u32,
    pixels: &[[u8; 3]],
) -> io::Result<()> {
    if !(1..=0x7fff_ffff).contains(&width)
        || !(1..=0x7fff_ffff).contains(&height)
        || (width as usize).checked_mul(height as usize) != Some(pixels.len())
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid RGB PNG dimensions"));
    }

    fn chunk(out: &mut impl Write, tag: &[u8; 4], data: &[u8]) -> io::Result<()> {
        let mut crc = !0u32;
        for &byte in tag.iter().chain(data) {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xEDB8_8320 & 0u32.wrapping_sub(crc & 1));
            }
        }
        out.write_all(&(data.len() as u32).to_be_bytes())?;
        out.write_all(tag)?;
        out.write_all(data)?;
        out.write_all(&(!crc).to_be_bytes())
    }

    struct Bits {
        buf: Vec<u8>,
        acc: u64,
        nbits: u32,
    }
    impl Bits {
        #[inline]
        fn put(&mut self, val: u32, n: u32) {
            self.acc |= u64::from(val) << self.nbits;
            self.nbits += n;
            while self.nbits >= 8 {
                self.buf.push(self.acc as u8);
                self.acc >>= 8;
                self.nbits -= 8;
            }
        }
        #[inline]
        fn put_msb(&mut self, code: u32, n: u32) {
            self.put(code.reverse_bits() >> (32 - n), n);
        }
        #[inline]
        fn lit(&mut self, sym: u32) {
            match sym {
                0..=143 => self.put_msb(0x30 + sym, 8),
                144..=255 => self.put_msb(0x190 + (sym - 144), 9),
                256..=279 => self.put_msb(sym - 256, 7),
                _ => self.put_msb(0xC0 + (sym - 280), 8),
            }
        }
        #[inline]
        fn match_code(&mut self, mut len: usize, dist: usize) {
            const LBASE: [u16; 29] = [
                3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83,
                99, 115, 131, 163, 195, 227, 258,
            ];
            const LEXT: [u8; 29] = [
                0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5,
                0,
            ];
            const DBASE: [u16; 30] = [
                1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769,
                1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
            ];
            const DEXT: [u8; 30] = [
                0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11,
                12, 12, 13, 13,
            ];
            while len >= 3 {
                let step = if len > 258 { (len - 3).min(258) } else { len };
                let li = LBASE.partition_point(|&b| usize::from(b) <= step) - 1;
                self.lit(257 + li as u32);
                if LEXT[li] > 0 {
                    self.put((step - usize::from(LBASE[li])) as u32, u32::from(LEXT[li]));
                }
                let di = DBASE.partition_point(|&b| usize::from(b) <= dist) - 1;
                self.put_msb(di as u32, 5);
                if DEXT[di] > 0 {
                    self.put((dist - usize::from(DBASE[di])) as u32, u32::from(DEXT[di]));
                }
                len -= step;
            }
        }
    }

    let row_bytes = width as usize * 3;
    let mut raw = Vec::with_capacity((row_bytes + 1) * height as usize);
    let mut prev_row: &[u8] = &[];
    for row_px in pixels.chunks_exact(width as usize) {
        let row = row_px.as_flattened();
        if !prev_row.is_empty() && row == prev_row {
            raw.push(2); // Up filter: identical scanline becomes all zeros
            raw.resize(raw.len() + row_bytes, 0);
        } else {
            raw.push(1); // Sub filter (3-byte RGB stride)
            for (i, &b) in row.iter().enumerate() {
                raw.push(b.wrapping_sub(if i >= 3 { row[i - 3] } else { 0 }));
            }
        }
        prev_row = row;
    }

    let (mut a, mut b) = (1u32, 0u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }

    let mut bits = Bits { buf: vec![0x78, 0x01], acc: 0, nbits: 0 };
    bits.put(0b011, 3); // BFINAL=1, BTYPE=01 (fixed Huffman)
    let mut ht = vec![usize::MAX; 4096];
    let mut i = 0;
    while i < raw.len() {
        let mut best_len = 0usize;
        let mut best_dist = 1usize;
        if i + 3 <= raw.len() {
            let h = (((u32::from(raw[i]) << 16)
                | (u32::from(raw[i + 1]) << 8)
                | u32::from(raw[i + 2]))
                .wrapping_mul(0x1E35_A7BD)
                >> 20) as usize;
            let cand = ht[h];
            ht[h] = i;
            let max_l = (raw.len() - i).min(258);
            for d in [1usize, 3, row_bytes + 1, i.wrapping_sub(cand)] {
                if (1..=32_768).contains(&d) && d <= i && raw[i - d..i - d + 3] == raw[i..i + 3] {
                    let l = 3 + raw[i + 3..i + max_l]
                        .iter()
                        .zip(&raw[i - d + 3..i - d + max_l])
                        .take_while(|(x, y)| x == y)
                        .count();
                    if l > best_len {
                        best_len = l;
                        best_dist = d;
                        if l == max_l {
                            break;
                        }
                    }
                }
            }
        }
        if best_len >= 4 {
            bits.match_code(best_len, best_dist);
            i += best_len;
        } else {
            bits.lit(u32::from(raw[i]));
            i += 1;
        }
    }
    bits.lit(256); // end-of-block
    if bits.nbits > 0 {
        bits.buf.push(bits.acc as u8);
    }
    bits.buf.extend_from_slice(&((b << 16) | a).to_be_bytes());

    out.write_all(b"\x89PNG\r\n\x1a\n")?;
    let mut ihdr = [0; 13];
    ihdr[..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8..10].copy_from_slice(&[8, 2]);
    chunk(&mut out, b"IHDR", &ihdr)?;
    for data in bits.buf.chunks(0x7fff_ffff) {
        chunk(&mut out, b"IDAT", data)?;
    }
    chunk(&mut out, b"IEND", &[])
}
