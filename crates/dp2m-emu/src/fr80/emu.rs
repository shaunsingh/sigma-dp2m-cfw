//! Big-endian FR80 interpreter for M5 firmware routines.

pub type Handler = Box<dyn FnMut(&mut Emulator) -> u32>;

/// `rp` on entry to [`Emulator::run`]; returning to it ends the run
pub const RETURN: u32 = 0xDEAD_BEEF;

const TLB_PAGES: usize = 1024; // 64 MB SDRAM window: 0x8000_0000..0x8400_0000 in 64 KB pages
const TLB_NONE: u8 = 0xFF;
const TLB_MULTI: u8 = 0xFE;

pub struct SdramRegion {
    pub base: u32,
    pub data: Vec<u8>,
}

struct SparseMem {
    pages: Vec<(u32, Box<[u8; 4096]>)>,
    last: (u32, usize),
}

impl SparseMem {
    fn new() -> Self {
        Self { pages: Vec::new(), last: (u32::MAX, 0) }
    }

    #[inline(always)]
    fn find(&self, tag: u32) -> Option<usize> {
        if self.last.0 == tag {
            return Some(self.last.1);
        }
        self.pages.iter().position(|&(t, _)| t == tag)
    }

    #[inline(always)]
    fn page_mut(&mut self, tag: u32) -> &mut [u8; 4096] {
        if self.last.0 == tag {
            return &mut self.pages[self.last.1].1;
        }
        if let Some(i) = self.pages.iter().position(|&(t, _)| t == tag) {
            self.last = (tag, i);
            return &mut self.pages[i].1;
        }
        let i = self.pages.len();
        self.pages.push((tag, Box::new([0u8; 4096])));
        self.last = (tag, i);
        &mut self.pages[i].1
    }

    #[inline(always)]
    fn r8(&self, addr: u32) -> u8 {
        self.find(addr >> 12).map_or(0, |i| self.pages[i].1[(addr & 0xFFF) as usize])
    }

    #[inline(always)]
    fn w8(&mut self, addr: u32, v: u8) {
        self.page_mut(addr >> 12)[(addr & 0xFFF) as usize] = v;
    }

    #[inline(always)]
    fn r16(&self, addr: u32) -> u16 {
        let off = (addr & 0xFFF) as usize;
        if off < 4095 {
            return self.find(addr >> 12).map_or(0, |i| {
                let p = &self.pages[i].1;
                u16::from_be_bytes([p[off], p[off + 1]])
            });
        }
        ((self.r8(addr) as u16) << 8) | (self.r8(addr.wrapping_add(1)) as u16)
    }

    #[inline(always)]
    fn w16(&mut self, addr: u32, v: u16) {
        let off = (addr & 0xFFF) as usize;
        if off < 4095 {
            let p = self.page_mut(addr >> 12);
            p[off..off + 2].copy_from_slice(&v.to_be_bytes());
            return;
        }
        self.w8(addr, (v >> 8) as u8);
        self.w8(addr.wrapping_add(1), v as u8);
    }

    #[inline(always)]
    fn r32(&self, addr: u32) -> u32 {
        let off = (addr & 0xFFF) as usize;
        if off <= 4092 {
            return self.find(addr >> 12).map_or(0, |i| {
                let p = &self.pages[i].1;
                u32::from_be_bytes([p[off], p[off + 1], p[off + 2], p[off + 3]])
            });
        }
        ((self.r16(addr) as u32) << 16) | (self.r16(addr.wrapping_add(2)) as u32)
    }

    #[inline(always)]
    fn w32(&mut self, addr: u32, v: u32) {
        let off = (addr & 0xFFF) as usize;
        if off <= 4092 {
            let p = self.page_mut(addr >> 12);
            p[off..off + 4].copy_from_slice(&v.to_be_bytes());
            return;
        }
        self.w16(addr, (v >> 16) as u16);
        self.w16(addr.wrapping_add(2), v as u16);
    }
}

pub struct Emulator<'a> {
    rom: &'a [u8],
    rom_delta: u32,
    pub r: [u32; 16],
    pub rp: u32,
    pub mdl: u32,
    pub flags: (bool, bool, bool, bool), // (N, Z, V, C)
    pub sdram: Vec<SdramRegion>,
    sdram_tlb: [u8; TLB_PAGES],
    sparse: SparseMem,
    /// data accesses per instruction: [sdram (>= 0x8000_0000), rom/other]
    pub mem: [u64; 2],
    /// optional per-halfword execution counts over `base..base + 2 * len`
    pub prof: Option<(u32, Vec<u64>)>,
    handlers: Vec<(u32, Option<Handler>)>,
    handler_bloom: u64,
}

impl<'a> Emulator<'a> {
    pub fn new(rom: &'a [u8], rom_delta: u32) -> Self {
        Self {
            rom,
            rom_delta,
            r: [0; 16],
            rp: 0,
            mdl: 0,
            flags: (false, false, false, false),
            sdram: Vec::new(),
            sdram_tlb: [TLB_NONE; TLB_PAGES],
            sparse: SparseMem::new(),
            mem: [0; 2],
            prof: None,
            handlers: Vec::new(),
            handler_bloom: 0,
        }
    }

    pub fn map_sdram(&mut self, base: u32, size: usize) -> usize {
        let idx = self.sdram.len();
        self.sdram.push(SdramRegion { base, data: vec![0u8; size] });
        if size > 0 && base >= 0x8000_0000 {
            let first = ((base - 0x8000_0000) >> 16) as usize;
            let last = ((base.saturating_add(size as u32 - 1) - 0x8000_0000) >> 16) as usize;
            for p in first..=last.min(TLB_PAGES - 1) {
                self.sdram_tlb[p] = if self.sdram_tlb[p] == TLB_NONE && idx < TLB_MULTI as usize {
                    idx as u8
                } else {
                    TLB_MULTI
                };
            }
        }
        idx
    }

    pub fn set_handler<F>(&mut self, va: u32, f: F)
    where
        F: FnMut(&mut Emulator) -> u32 + 'static,
    {
        self.handler_bloom |= 1u64 << ((va >> 1) & 63);
        if let Some(slot) = self.handlers.iter_mut().find(|(a, _)| *a == va) {
            slot.1 = Some(Box::new(f));
        } else {
            self.handlers.push((va, Some(Box::new(f))));
        }
    }

    #[inline(always)]
    fn sdram_region_idx(&self, addr: u32) -> Option<usize> {
        let pg = (addr.wrapping_sub(0x8000_0000) >> 16) as usize;
        if pg < TLB_PAGES {
            let slot = self.sdram_tlb[pg];
            if slot < TLB_MULTI {
                return Some(slot as usize);
            }
            if slot == TLB_NONE {
                return None;
            }
        }
        self.find_sdram_region(addr)
    }

    #[cold]
    fn find_sdram_region(&self, addr: u32) -> Option<usize> {
        self.sdram.iter().position(|reg| {
            let off = addr.wrapping_sub(reg.base) as usize;
            addr >= reg.base && off < reg.data.len()
        })
    }

    // Keep the sparse/MMIO search out of the instruction decoder's hot paths.
    #[cold]
    #[inline(never)]
    fn read_sparse<const N: usize>(&self, addr: u32) -> [u8; N] {
        let mut bytes = [0; N];
        match N {
            1 => bytes[0] = self.sparse.r8(addr),
            2 => bytes.copy_from_slice(&self.sparse.r16(addr).to_be_bytes()),
            4 => bytes.copy_from_slice(&self.sparse.r32(addr).to_be_bytes()),
            _ => unreachable!(),
        }
        bytes
    }

    #[inline(always)]
    fn rom_bytes<const N: usize>(&self, addr: u32) -> Option<&[u8; N]> {
        let off = addr.checked_sub(self.rom_delta)? as usize;
        self.rom.get(off..)?.first_chunk()
    }

    #[inline(always)]
    fn read_bytes<const N: usize>(&self, addr: u32) -> [u8; N] {
        if addr >= 0x8000_0000 {
            if let Some(i) = self.sdram_region_idx(addr) {
                let reg = &self.sdram[i];
                if let Some(off) = addr.checked_sub(reg.base)
                    && let Some(bytes) = reg.data.get(off as usize..).and_then(<[_]>::first_chunk::<N>)
                {
                    return *bytes;
                }
            }
        } else if let Some(bytes) = self.rom_bytes(addr) {
            return *bytes;
        }
        self.read_sparse(addr)
    }

    #[cold]
    #[inline(never)]
    fn write_sparse<const N: usize>(&mut self, addr: u32, bytes: [u8; N]) {
        match N {
            1 => self.sparse.w8(addr, bytes[0]),
            2 => self.sparse.w16(addr, u16::from_be_bytes([bytes[0], bytes[1]])),
            4 => self.sparse.w32(addr, u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
            _ => unreachable!(),
        }
    }

    #[inline(always)]
    fn write_bytes<const N: usize>(&mut self, addr: u32, bytes: [u8; N]) {
        if addr >= 0x8000_0000
            && let Some(i) = self.sdram_region_idx(addr)
        {
            let reg = &mut self.sdram[i];
            if let Some(off) = addr.checked_sub(reg.base)
                && let Some(dst) = reg.data.get_mut(off as usize..).and_then(<[_]>::first_chunk_mut::<N>)
            {
                *dst = bytes;
                return;
            }
        }
        self.write_sparse(addr, bytes);
    }

    #[inline(always)]
    pub fn r8(&self, addr: u32) -> u8 {
        self.read_bytes::<1>(addr)[0]
    }

    #[inline(always)]
    pub fn w8(&mut self, addr: u32, v: u8) {
        self.write_bytes(addr, [v]);
    }

    #[inline(always)]
    pub fn r16(&self, addr: u32) -> u16 {
        u16::from_be_bytes(self.read_bytes(addr))
    }

    #[inline(always)]
    pub fn w16(&mut self, addr: u32, v: u16) {
        self.write_bytes(addr, v.to_be_bytes());
    }

    #[inline(always)]
    pub fn r32(&self, addr: u32) -> u32 {
        u32::from_be_bytes(self.read_bytes(addr))
    }

    #[inline(always)]
    pub fn w32(&mut self, addr: u32, v: u32) {
        self.write_bytes(addr, v.to_be_bytes());
    }

    #[inline(always)]
    fn fetch<const N: usize>(&self, pc: u32) -> [u8; N] {
        // Fetches prefer ROM even at high addresses. On a miss below SDRAM,
        // read_bytes would just repeat the same ROM check.
        if let Some(bytes) = self.rom_bytes(pc) {
            *bytes
        } else if pc >= 0x8000_0000 {
            self.read_bytes(pc)
        } else {
            self.read_sparse(pc)
        }
    }

    pub fn run(&mut self, start_va: u32, max_steps: u64) -> Result<u64, String> {
        match (self.prof.is_some(), self.handler_bloom != 0) {
            (false, false) => self.run_loop::<false, false>(start_va, max_steps),
            (false, true) => self.run_loop::<false, true>(start_va, max_steps),
            (true, false) => self.run_loop::<true, false>(start_va, max_steps),
            (true, true) => self.run_loop::<true, true>(start_va, max_steps),
        }
    }

    #[inline(never)]
    fn run_loop<const PROF: bool, const HANDLERS: bool>(&mut self, start_va: u32, max_steps: u64) -> Result<u64, String> {
        let mut pc = start_va;
        let mut r = self.r;
        let mut rp = RETURN;
        let mut mdl = self.mdl;
        let (mut fl_n, mut fl_z, mut fl_v, mut fl_c) = self.flags;
        let mut mem = self.mem;
        let mut steps = 0u64;

        macro_rules! tally {
            ($addr:expr) => {{
                let a: u32 = $addr;
                mem[(a < 0x8000_0000) as usize] += 1;
                a
            }};
        }
        macro_rules! set_nz {
            ($res:expr) => {{
                let res: u32 = $res;
                fl_n = (res as i32) < 0;
                fl_z = res == 0;
            }};
        }
        macro_rules! add {
            ($dst:expr, $src:expr) => {{
                let (dst, src) = ($dst, $src);
                let (res, carry) = u32::overflowing_add(dst, src);
                fl_c = carry;
                fl_v = (dst as i32).overflowing_add(src as i32).1;
                set_nz!(res);
                res
            }};
        }
        macro_rules! sub {
            ($dst:expr, $src:expr) => {{
                let (dst, src) = ($dst, $src);
                let (res, borrow) = u32::overflowing_sub(dst, src);
                fl_c = borrow;
                fl_v = (dst as i32).overflowing_sub(src as i32).1;
                set_nz!(res);
                res
            }};
        }

        while steps < max_steps {
            if pc == RETURN {
                self.r = r;
                self.rp = rp;
                self.mdl = mdl;
                self.flags = (fl_n, fl_z, fl_v, fl_c);
                self.mem = mem;
                return Ok(steps);
            }
            if HANDLERS
                && (self.handler_bloom >> ((pc >> 1) & 63)) & 1 != 0
                && let Some(idx) = self.handlers.iter().position(|(a, _)| *a == pc)
                && let Some(mut h) = self.handlers[idx].1.take()
            {
                self.r = r;
                self.rp = rp;
                self.mdl = mdl;
                self.flags = (fl_n, fl_z, fl_v, fl_c);
                self.mem = mem;
                let next_pc = h(self);
                self.handlers[idx].1 = Some(h);
                r = self.r;
                rp = self.rp;
                mdl = self.mdl;
                (fl_n, fl_z, fl_v, fl_c) = self.flags;
                mem = self.mem;
                pc = next_pc;
                continue;
            }
            if PROF
                && let Some((base, counts)) = &mut self.prof
                && let Some(c) = counts.get_mut((pc.wrapping_sub(*base) / 2) as usize)
            {
                *c += 1;
            }
            let op = u16::from_be_bytes(self.fetch(pc));
            pc = pc.wrapping_add(2);
            steps += 1;

            let hi8 = (op >> 8) as u8;
            let lo8 = op as u8;
            let n1 = ((op >> 4) & 0xF) as usize;
            let n0 = (op & 0xF) as usize;

            match hi8 {
                0x00 => {
                    let a = tally!(r[n1].wrapping_add(r[13]));
                    r[n0] = self.r32(a & !3);
                }
                0x01 => {
                    let a = tally!(r[n1].wrapping_add(r[13]));
                    r[n0] = self.r16(a & !1) as u32;
                }
                0x02 => {
                    let a = tally!(r[n1].wrapping_add(r[13]));
                    r[n0] = self.r8(a) as u32;
                }
                0x03 => {
                    let a = tally!(r[15].wrapping_add((n1 as u32) * 4));
                    r[n0] = self.r32(a);
                }
                0x04 => {
                    let a = tally!(r[n1]);
                    r[n0] = self.r32(a & !3);
                }
                0x05 => {
                    let a = tally!(r[n1]);
                    r[n0] = self.r16(a & !1) as u32;
                }
                0x06 => {
                    let a = tally!(r[n1]);
                    r[n0] = self.r8(a) as u32;
                }
                0x07 if n1 == 0 => {
                    let a = tally!(r[15]);
                    r[15] = r[15].wrapping_add(4);
                    r[n0] = self.r32(a);
                }
                0x07 if n1 == 8 && n0 == 1 => {
                    let a = tally!(r[15]);
                    rp = self.r32(a);
                    r[15] = r[15].wrapping_add(4);
                }
                0x0F => {
                    let a = tally!(r[15].wrapping_sub(4));
                    self.w32(a, r[14]);
                    r[14] = a;
                    r[15] = r[15].wrapping_sub((lo8 as u32) * 4);
                }
                0x10 => {
                    let a = tally!(r[n1].wrapping_add(r[13]));
                    self.w32(a & !3, r[n0]);
                }
                0x11 => {
                    let a = tally!(r[n1].wrapping_add(r[13]));
                    self.w16(a & !1, r[n0] as u16);
                }
                0x12 => {
                    let a = tally!(r[n1].wrapping_add(r[13]));
                    self.w8(a, r[n0] as u8);
                }
                0x13 => {
                    let a = tally!(r[15].wrapping_add((n1 as u32) * 4));
                    self.w32(a, r[n0]);
                }
                0x14 => {
                    let a = tally!(r[n1]);
                    self.w32(a & !3, r[n0]);
                }
                0x15 => {
                    let a = tally!(r[n1]);
                    self.w16(a & !1, r[n0] as u16);
                }
                0x16 => {
                    let a = tally!(r[n1]);
                    self.w8(a, r[n0] as u8);
                }
                0x17 if n1 == 0 => {
                    let val = r[n0];
                    r[15] = r[15].wrapping_sub(4);
                    let a = tally!(r[15]);
                    self.w32(a, val);
                }
                0x17 if n1 == 8 && n0 == 1 => {
                    r[15] = r[15].wrapping_sub(4);
                    let a = tally!(r[15]);
                    self.w32(a, rp);
                }
                0x20..=0x2F => {
                    let d = (((op >> 4) as u8 as i8) as i32) * 4;
                    let a = tally!(r[14].wrapping_add(d as u32));
                    r[n0] = self.r32(a);
                }
                0x30..=0x3F => {
                    let d = (((op >> 4) as u8 as i8) as i32) * 4;
                    let a = tally!(r[14].wrapping_add(d as u32));
                    self.w32(a, r[n0]);
                }
                0x82 => {
                    r[n0] &= r[n1];
                    set_nz!(r[n0]);
                }
                0x8B => r[n0] = r[n1],
                0x92 => {
                    r[n0] |= r[n1];
                    set_nz!(r[n0]);
                }
                0x97 => match n1 {
                    0x0 => pc = r[n0],
                    0x1 => {
                        rp = pc;
                        pc = r[n0];
                    }
                    0x2 if n0 == 0 => pc = rp,
                    0x8 => r[n0] = (r[n0] as u8 as i8 as i32) as u32,
                    0x9 => r[n0] &= 0xFF,
                    0xA => r[n0] = (r[n0] as u16 as i16 as i32) as u32,
                    0xB => r[n0] &= 0xFFFF,
                    _ => return Err(format!("unsupported FR80 op 0x{:04X} @ 0x{:06X}", op, pc - 2)),
                },
                0x9A => {
                    r[n0] ^= r[n1];
                    set_nz!(r[n0]);
                }
                0x9B => {
                    r[n0] = ((n1 as u32) << 16) | (u16::from_be_bytes(self.fetch(pc)) as u32);
                    pc = pc.wrapping_add(2);
                }
                0x9F if n1 == 8 => {
                    r[n0] = u32::from_be_bytes(self.fetch(pc));
                    pc = pc.wrapping_add(4);
                }
                0x9F if op == 0x9F90 => {
                    r[15] = r[14];
                    let a = tally!(r[15]);
                    r[14] = self.r32(a);
                    r[15] = r[15].wrapping_add(4);
                }
                0x9F if op == 0x9FA0 => {}
                0xA3 => {
                    let d = (lo8 as i8 as i32) * 4;
                    r[15] = r[15].wrapping_add(d as u32);
                }
                0xA4 => r[n0] = add!(r[n0], n1 as u32),
                0xA5 => r[n0] = add!(r[n0], n1 as u32 | 0xFFFF_FFF0),
                0xA6 => r[n0] = add!(r[n0], r[n1]),
                0xA8 => { sub!(r[n0], n1 as u32); }
                0xAA => { sub!(r[n0], r[n1]); }
                0xAC => r[n0] = sub!(r[n0], r[n1]),
                0xAF => {
                    let prod = r[n0] as i32 as i64 * r[n1] as i32 as i64;
                    mdl = prod as u32;
                    fl_n = (mdl as i32) < 0;
                    fl_z = prod == 0;
                    fl_v = prod != mdl as i32 as i64;
                }
                0xB0 | 0xB1 | 0xB8 | 0xB9 => {
                    let amount = n1 as u32 + u32::from(hi8 & 1) * 16;
                    let old = r[n0];
                    r[n0] = if hi8 & 8 == 0 { old >> amount } else { ((old as i32) >> amount) as u32 };
                    fl_c = amount != 0 && old.wrapping_shr(amount.wrapping_sub(1)) & 1 != 0;
                    set_nz!(r[n0]);
                }
                0xB4 | 0xB5 => {
                    let amount = n1 as u32 + u32::from(hi8 & 1) * 16;
                    let old = r[n0];
                    r[n0] = old << amount;
                    fl_c = amount != 0 && old.wrapping_shr(32 - amount) & 1 != 0;
                    set_nz!(r[n0]);
                }
                0xB7 if n1 == 5 => r[n0] = mdl,
                0xC0..=0xCF => r[n0] = ((op >> 4) & 0xFF) as u32,
                0xD0..=0xD7 => {
                    rp = pc;
                    let disp = ((((op & 0x7FF) as i32) << 21) >> 21) * 2;
                    pc = pc.wrapping_add(disp as u32);
                }
                0xE0..=0xEF => {
                    let take = match hi8 & 0xF {
                        0x0 => true,
                        0x2 => fl_z,
                        0x3 => !fl_z,
                        0x4 => fl_c,
                        0x5 => !fl_c,
                        0xA => fl_n ^ fl_v,
                        0xB => !(fl_n ^ fl_v),
                        0xC => fl_z || (fl_n ^ fl_v),
                        0xD => !fl_z && !(fl_n ^ fl_v),
                        0xE => fl_c || fl_z,
                        0xF => !fl_c && !fl_z,
                        _ => return Err(format!("unsupported Bcc 0x{:04X} @ 0x{:06X}", op, pc - 2)),
                    };
                    if take {
                        let disp = (lo8 as i8 as i32) * 2;
                        pc = pc.wrapping_add(disp as u32);
                    }
                }
                _ => return Err(format!("unsupported FR80 op 0x{:04X} @ 0x{:06X}", op, pc - 2)),
            }
        }
        self.r = r;
        self.rp = rp;
        self.mdl = mdl;
        self.flags = (fl_n, fl_z, fl_v, fl_c);
        self.mem = mem;
        Err(format!("FR80 step limit ({}) reached @ 0x{:06X}", max_steps, pc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic_updates_nzvc() {
        for (op, dst, src, result, expected) in [
            (0xA410u16, 0xFFFF_FFFF, 0, 0, (false, true, false, true)), // ADD #1
            (0xA5F0, 0x8000_0000, 0, 0x7FFF_FFFF, (false, false, true, true)), // ADD2 #-1
            (0xA610, 0x7FFF_FFFF, 1, 0x8000_0000, (true, false, true, false)), // ADD R1
            (0xAC10, 0, 1, 0xFFFF_FFFF, (true, false, false, true)), // SUB R1
            (0xAA10, 0, 1, 0, (true, false, false, true)), // CMP R1
        ] {
            let [a, b] = op.to_be_bytes();
            let rom = [a, b, 0x97, 0x20];
            let mut e = Emulator::new(&rom, 0);
            e.r[..2].copy_from_slice(&[dst, src]);
            e.run(0, 3).unwrap();
            assert_eq!((e.r[0], e.flags), (result, expected), "{op:04x}");
        }
    }

    #[test]
    fn logical_ops_preserve_vc_and_set_nz() {
        for (op, src, result) in [(0x82, 0, 0), (0x92, 0, 0x8000_0000), (0x9A, 0x8000_0000, 0)] {
            let rom = [op, 0x10, 0x97, 0x20];
            let mut e = Emulator::new(&rom, 0);
            e.r[..2].copy_from_slice(&[0x8000_0000, src]);
            e.flags = (false, false, true, true);
            e.run(0, 3).unwrap();
            assert_eq!((e.r[0], e.flags), (result, (result != 0, result == 0, true, true)));
        }
    }

    #[test]
    fn immediate_shifts_set_nzc_preserving_v() {
        for (op, lo, dst, result, carry) in [
            (0xB4, 0, 0x8000_0000, 0x8000_0000, false), // LSL #0 clears C
            (0xB4, 0x10, 0x8000_0000, 0, true), // LSL #1
            (0xB1, 0, 0x0000_8000, 0, true), // LSR2 #16
            (0xB9, 0xF0, 0xC000_0000, 0xFFFF_FFFF, true), // ASR2 #31
        ] {
            let rom = [op, lo, 0x97, 0x20];
            let mut e = Emulator::new(&rom, 0);
            e.r[0] = dst;
            e.flags = (false, false, true, true);
            e.run(0, 3).unwrap();
            assert_eq!((e.r[0], e.flags), (result, ((result as i32) < 0, result == 0, true, carry)));
        }
    }

    #[test]
    fn mul_flags_distinguish_low_word_from_full_product() {
        let rom = [0xAF, 0x10, 0x97, 0x20]; // mul r1, r0; ret
        let mut e = Emulator::new(&rom, 0);
        e.r[..2].copy_from_slice(&[0x8000_0000, 2]);
        e.flags = (true, true, false, true);
        e.run(0, 3).unwrap();
        assert_eq!((e.mdl, e.flags), (0, (false, false, true, true)));
    }

    #[test]
    fn enter_immediate_includes_saved_frame_pointer() {
        let rom = [0x0F, 0x04, 0x8B, 0xF0, 0x9F, 0x90, 0x97, 0x20]; // enter #16; mov sp,r0; leave; ret
        let mut e = Emulator::new(&rom, 0);
        e.r[14..].copy_from_slice(&[0x1234, 0x8000_0020]);
        e.run(0, 5).unwrap();
        assert_eq!(e.r[0], 0x8000_0010);
        assert_eq!(e.r32(0x8000_001C), 0x1234);
        assert_eq!(e.r[14..], [0x1234, 0x8000_0020]);
    }

    #[test]
    fn push_and_pop_sp_use_original_source_and_loaded_destination() {
        let rom = [0x17, 0x0F, 0x07, 0x0F, 0x97, 0x20]; // st sp,@-sp; ld @sp+,sp; ret
        let mut e = Emulator::new(&rom, 0);
        e.r[15] = 0x8000_0020;
        e.run(0, 4).unwrap();
        assert_eq!(e.r32(0x8000_001C), 0x8000_0020);
        assert_eq!(e.r[15], 0x8000_0020);
    }

    #[test]
    fn fetch_and_data_access_counters() {
        const ROM: u32 = 0x0003_BF80;
        const RAM: u32 = 0x8000_0000;
        let rom = [
            0x9F, 0x81, 0x00, 0x03, 0xBF, 0x90, // ldi:32 #ROM+16, r1
            0x04, 0x10, // ld @r1, r0
            0x14, 0x20, // st r0, @r2
            0x04, 0x23, // ld @r2, r3
            0x9F, 0xA0, // nop (or handled)
            0x97, 0x20, // ret
            0x12, 0x34, 0x56, 0x78,
        ];
        for profile in [false, true] {
            for handler in [false, true] {
                let mut emu = Emulator::new(&rom, ROM);
                emu.r[2] = RAM;
                emu.map_sdram(RAM, 4);
                if profile {
                    emu.prof = Some((ROM, vec![0; 10]));
                }
                if handler {
                    emu.set_handler(ROM + 12, |e| {
                        assert_eq!(e.mem, [2, 1]);
                        ROM + 14
                    });
                }
                assert_eq!(emu.run(ROM, 8).unwrap(), if handler { 5 } else { 6 });
                assert_eq!(emu.mem, [2, 1]);
                assert_eq!(emu.r[3], 0x1234_5678);
                if let Some((_, hits)) = emu.prof {
                    assert_eq!(hits, [1, 0, 0, 1, 1, 1, u64::from(!handler), 1, 0, 0]);
                }
            }
        }
    }

    #[test]
    fn mapped_edges_and_fetch_priority() {
        const BASE: u32 = 0x8000_0000;
        let mut emu = Emulator::new(&[0x97, 0x20], BASE);
        emu.map_sdram(BASE, 4);
        emu.w32(BASE, 0xFFFF_FF12);
        assert_eq!(emu.r16(BASE), 0xFFFF); // Data reads prefer SDRAM.
        assert_eq!(emu.run(BASE, 2).unwrap(), 1); // Fetch prefers ROM.

        // An access straddling a region end goes entirely to sparse memory.
        emu.w32(BASE + 3, 0xABCD_EF01);
        assert_eq!(emu.r32(BASE + 3), 0xABCD_EF01);
        assert_eq!(emu.r8(BASE + 3), 0x12);
        assert_eq!(emu.r8(BASE + 4), 0xCD);
    }
}
