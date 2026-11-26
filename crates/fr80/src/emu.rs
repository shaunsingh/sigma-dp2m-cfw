//! fr80 BE emulator for fw verification and capture simulation

pub type Handler = Box<dyn FnMut(&mut Emulator) -> u32>;

/// `rp` on entry to [`Emulator::run`]; returning to it ends the run
pub const RETURN: u32 = 0xDEAD_BEEF;

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
    sparse: SparseMem,
    /// data accesses per instruction: [sdram (>= 0x8000_0000), rom/other]
    pub mem: [u64; 2],
    /// optional per-halfword execution counts over `base..base + 2 * len`
    pub prof: Option<(u32, Vec<u64>)>,
    handlers: Vec<(u32, Option<Handler>)>,
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
            sparse: SparseMem::new(),
            mem: [0; 2],
            prof: None,
            handlers: Vec::new(),
        }
    }

    #[inline(always)]
    fn tally(&mut self, addr: u32) -> u32 {
        self.mem[(addr < 0x8000_0000) as usize] += 1;
        addr
    }

    pub fn map_sdram(&mut self, base: u32, size: usize) -> usize {
        let idx = self.sdram.len();
        self.sdram.push(SdramRegion { base, data: vec![0u8; size] });
        idx
    }

    pub fn set_handler<F>(&mut self, va: u32, f: F)
    where
        F: FnMut(&mut Emulator) -> u32 + 'static,
    {
        if let Some(slot) = self.handlers.iter_mut().find(|(a, _)| *a == va) {
            slot.1 = Some(Box::new(f));
        } else {
            self.handlers.push((va, Some(Box::new(f))));
        }
    }

    #[inline(always)]
    pub fn r8(&self, addr: u32) -> u8 {
        if addr >= 0x8000_0000 {
            for reg in &self.sdram {
                let off = addr.wrapping_sub(reg.base) as usize;
                if addr >= reg.base && off < reg.data.len() {
                    return reg.data[off];
                }
            }
            return self.sparse.r8(addr);
        }
        let off = addr.wrapping_sub(self.rom_delta) as usize;
        if addr >= self.rom_delta && off < self.rom.len() {
            return self.rom[off];
        }
        self.sparse.r8(addr)
    }

    #[inline(always)]
    pub fn w8(&mut self, addr: u32, v: u8) {
        if addr >= 0x8000_0000 {
            for reg in &mut self.sdram {
                let off = addr.wrapping_sub(reg.base) as usize;
                if addr >= reg.base && off < reg.data.len() {
                    reg.data[off] = v;
                    return;
                }
            }
        }
        self.sparse.w8(addr, v);
    }

    #[inline(always)]
    pub fn r16(&self, addr: u32) -> u16 {
        if addr >= 0x8000_0000 {
            for reg in &self.sdram {
                let off = addr.wrapping_sub(reg.base) as usize;
                if addr >= reg.base && off + 2 <= reg.data.len() {
                    return u16::from_be_bytes([reg.data[off], reg.data[off + 1]]);
                }
            }
            return self.sparse.r16(addr);
        }
        let off = addr.wrapping_sub(self.rom_delta) as usize;
        if addr >= self.rom_delta && off + 2 <= self.rom.len() {
            return u16::from_be_bytes([self.rom[off], self.rom[off + 1]]);
        }
        self.sparse.r16(addr)
    }

    #[inline(always)]
    pub fn w16(&mut self, addr: u32, v: u16) {
        if addr >= 0x8000_0000 {
            for reg in &mut self.sdram {
                let off = addr.wrapping_sub(reg.base) as usize;
                if addr >= reg.base && off + 2 <= reg.data.len() {
                    reg.data[off..off + 2].copy_from_slice(&v.to_be_bytes());
                    return;
                }
            }
        }
        self.sparse.w16(addr, v);
    }

    #[inline(always)]
    pub fn r32(&self, addr: u32) -> u32 {
        if addr >= 0x8000_0000 {
            for reg in &self.sdram {
                let off = addr.wrapping_sub(reg.base) as usize;
                if addr >= reg.base && off + 4 <= reg.data.len() {
                    return u32::from_be_bytes([
                        reg.data[off],
                        reg.data[off + 1],
                        reg.data[off + 2],
                        reg.data[off + 3],
                    ]);
                }
            }
            return self.sparse.r32(addr);
        }
        let off = addr.wrapping_sub(self.rom_delta) as usize;
        if addr >= self.rom_delta && off + 4 <= self.rom.len() {
            return u32::from_be_bytes([
                self.rom[off],
                self.rom[off + 1],
                self.rom[off + 2],
                self.rom[off + 3],
            ]);
        }
        self.sparse.r32(addr)
    }

    #[inline(always)]
    pub fn w32(&mut self, addr: u32, v: u32) {
        if addr >= 0x8000_0000 {
            for reg in &mut self.sdram {
                let off = addr.wrapping_sub(reg.base) as usize;
                if addr >= reg.base && off + 4 <= reg.data.len() {
                    reg.data[off..off + 4].copy_from_slice(&v.to_be_bytes());
                    return;
                }
            }
        }
        self.sparse.w32(addr, v);
    }

    #[inline(always)]
    fn set_cmp(&mut self, dst: u32, src: u32) {
        let sa = dst as i32;
        let sb = src as i32;
        let (r32, c) = dst.overflowing_sub(src);
        let (_, v) = sa.overflowing_sub(sb);
        self.flags = ((r32 as i32) < 0, r32 == 0, v, c);
    }

    #[inline(always)]
    fn fetch16(&self, pc: u32) -> u16 {
        let off = pc.wrapping_sub(self.rom_delta) as usize;
        if pc >= self.rom_delta && off + 1 < self.rom.len() {
            return u16::from_be_bytes([self.rom[off], self.rom[off + 1]]);
        }
        self.r16(pc)
    }

    pub fn run(&mut self, start_va: u32, max_steps: u64) -> Result<u64, String> {
        let mut pc = start_va;
        self.rp = RETURN;
        let mut steps = 0u64;

        while steps < max_steps {
            if pc == RETURN {
                return Ok(steps);
            }
            if !self.handlers.is_empty()
                && let Some(idx) = self.handlers.iter().position(|(a, _)| *a == pc)
                && let Some(mut h) = self.handlers[idx].1.take()
            {
                let next_pc = h(self);
                self.handlers[idx].1 = Some(h);
                pc = next_pc;
                continue;
            }
            if let Some((base, counts)) = &mut self.prof
                && let Some(c) = counts.get_mut((pc.wrapping_sub(*base) / 2) as usize)
            {
                *c += 1;
            }
            let op = self.fetch16(pc);
            pc = pc.wrapping_add(2);
            steps += 1;

            let hi8 = (op >> 8) as u8;
            let lo8 = op as u8;
            let n1 = ((op >> 4) & 0xF) as usize;
            let n0 = (op & 0xF) as usize;

            match hi8 {
                0x00 => {
                    let a = self.tally(self.r[n1].wrapping_add(self.r[13]));
                    self.r[n0] = self.r32(a & !3);
                }
                0x01 => {
                    let a = self.tally(self.r[n1].wrapping_add(self.r[13]));
                    self.r[n0] = self.r16(a & !1) as u32;
                }
                0x02 => {
                    let a = self.tally(self.r[n1].wrapping_add(self.r[13]));
                    self.r[n0] = self.r8(a) as u32;
                }
                0x03 => {
                    let a = self.tally(self.r[15].wrapping_add((n1 as u32) * 4));
                    self.r[n0] = self.r32(a);
                }
                0x04 => {
                    let a = self.tally(self.r[n1]);
                    self.r[n0] = self.r32(a & !3);
                }
                0x05 => {
                    let a = self.tally(self.r[n1]);
                    self.r[n0] = self.r16(a & !1) as u32;
                }
                0x06 => {
                    let a = self.tally(self.r[n1]);
                    self.r[n0] = self.r8(a) as u32;
                }
                0x07 if n1 == 0 => {
                    let a = self.tally(self.r[15]);
                    self.r[n0] = self.r32(a);
                    self.r[15] = self.r[15].wrapping_add(4);
                }
                0x07 if n1 == 8 && n0 == 1 => {
                    let a = self.tally(self.r[15]);
                    self.rp = self.r32(a);
                    self.r[15] = self.r[15].wrapping_add(4);
                }
                0x0F => {
                    self.r[15] = self.r[15].wrapping_sub(4);
                    let a = self.tally(self.r[15]);
                    self.w32(a, self.r[14]);
                    self.r[14] = self.r[15];
                    self.r[15] = self.r[15].wrapping_sub((lo8 as u32) * 4);
                }
                0x10 => {
                    let a = self.tally(self.r[n1].wrapping_add(self.r[13]));
                    self.w32(a & !3, self.r[n0]);
                }
                0x11 => {
                    let a = self.tally(self.r[n1].wrapping_add(self.r[13]));
                    self.w16(a & !1, self.r[n0] as u16);
                }
                0x12 => {
                    let a = self.tally(self.r[n1].wrapping_add(self.r[13]));
                    self.w8(a, self.r[n0] as u8);
                }
                0x13 => {
                    let a = self.tally(self.r[15].wrapping_add((n1 as u32) * 4));
                    self.w32(a, self.r[n0]);
                }
                0x14 => {
                    let a = self.tally(self.r[n1]);
                    self.w32(a & !3, self.r[n0]);
                }
                0x15 => {
                    let a = self.tally(self.r[n1]);
                    self.w16(a & !1, self.r[n0] as u16);
                }
                0x16 => {
                    let a = self.tally(self.r[n1]);
                    self.w8(a, self.r[n0] as u8);
                }
                0x17 if n1 == 0 => {
                    self.r[15] = self.r[15].wrapping_sub(4);
                    let a = self.tally(self.r[15]);
                    self.w32(a, self.r[n0]);
                }
                0x17 if n1 == 8 && n0 == 1 => {
                    self.r[15] = self.r[15].wrapping_sub(4);
                    let a = self.tally(self.r[15]);
                    self.w32(a, self.rp);
                }
                0x20..=0x2F => {
                    let d = (((op >> 4) as u8 as i8) as i32) * 4;
                    let a = self.tally(self.r[14].wrapping_add(d as u32));
                    self.r[n0] = self.r32(a);
                }
                0x30..=0x3F => {
                    let d = (((op >> 4) as u8 as i8) as i32) * 4;
                    let a = self.tally(self.r[14].wrapping_add(d as u32));
                    self.w32(a, self.r[n0]);
                }
                0x82 => {
                    self.r[n0] &= self.r[n1];
                    let v = self.r[n0];
                    self.flags = ((v as i32) < 0, v == 0, false, false);
                }
                0x8B => self.r[n0] = self.r[n1],
                0x92 => self.r[n0] |= self.r[n1],
                0x97 => match n1 {
                    0x0 => pc = self.r[n0],
                    0x1 => {
                        self.rp = pc;
                        pc = self.r[n0];
                    }
                    0x2 if n0 == 0 => pc = self.rp,
                    0x8 => self.r[n0] = (self.r[n0] as u8 as i8 as i32) as u32,
                    0x9 => self.r[n0] &= 0xFF,
                    0xA => self.r[n0] = (self.r[n0] as u16 as i16 as i32) as u32,
                    0xB => self.r[n0] &= 0xFFFF,
                    _ => return Err(format!("unsupported FR80 op 0x{:04X} @ 0x{:06X}", op, pc - 2)),
                },
                0x9A => self.r[n0] ^= self.r[n1],
                0x9B => {
                    self.r[n0] = ((n1 as u32) << 16) | (self.r16(pc) as u32);
                    pc = pc.wrapping_add(2);
                }
                0x9F if n1 == 8 => {
                    self.r[n0] = self.r32(pc);
                    pc = pc.wrapping_add(4);
                }
                0x9F if op == 0x9F90 => {
                    self.r[15] = self.r[14];
                    let a = self.tally(self.r[15]);
                    self.r[14] = self.r32(a);
                    self.r[15] = self.r[15].wrapping_add(4);
                }
                0x9F if op == 0x9FA0 => {}
                0xA3 => {
                    let d = (lo8 as i8 as i32) * 4;
                    self.r[15] = self.r[15].wrapping_add(d as u32);
                }
                0xA4 => self.r[n0] = self.r[n0].wrapping_add(n1 as u32),
                0xA5 => self.r[n0] = self.r[n0].wrapping_add(((n1 as i32) - 16) as u32),
                0xA6 => self.r[n0] = self.r[n0].wrapping_add(self.r[n1]),
                0xA8 => self.set_cmp(self.r[n0], n1 as u32),
                0xAA => self.set_cmp(self.r[n0], self.r[n1]),
                0xAC => self.r[n0] = self.r[n0].wrapping_sub(self.r[n1]),
                0xAF => self.mdl = (self.r[n0] as i32).wrapping_mul(self.r[n1] as i32) as u32,
                0xB0 | 0xB1 => self.r[n0] = self.r[n0].wrapping_shr(n1 as u32 + ((hi8 & 1) as u32) * 16),
                0xB4 | 0xB5 => self.r[n0] = self.r[n0].wrapping_shl(n1 as u32 + ((hi8 & 1) as u32) * 16),
                0xB7 if n1 == 5 => self.r[n0] = self.mdl,
                0xB8 | 0xB9 => self.r[n0] = ((self.r[n0] as i32) >> (n1 + ((hi8 & 1) as usize) * 16)) as u32,
                0xC0..=0xCF => self.r[n0] = ((op >> 4) & 0xFF) as u32,
                0xE0..=0xEF => {
                    let (n, z, v, c) = self.flags;
                    let take = match hi8 & 0xF {
                        0x0 => true,
                        0x2 => z,
                        0x3 => !z,
                        0x4 => c,
                        0x5 => !c,
                        0xA => n ^ v,
                        0xB => !(n ^ v),
                        0xC => z || (n ^ v),
                        0xD => !z && !(n ^ v),
                        0xE => c || z,
                        0xF => !c && !z,
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
        Err(format!("FR80 step limit ({}) reached @ 0x{:06X}", max_steps, pc))
    }
}
