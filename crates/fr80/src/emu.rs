//! fr80 BE emulator for fw verification

use std::collections::HashMap;

pub type Handler = Box<dyn FnMut(&mut Emulator) -> u32>;

pub struct SdramRegion {
    pub base: u32,
    pub data: Vec<u8>,
}

pub struct Emulator<'a> {
    rom: &'a [u8],
    rom_delta: u32,
    pub r: [u32; 16],
    pub rp: u32,
    pub mdl: u32,
    pub flags: (bool, bool, bool, bool), // (N, Z, V, C)
    pub sdram: Vec<SdramRegion>,
    pub sparse_ram: HashMap<u32, u8>,
    handlers: HashMap<u32, Handler>,
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
            sparse_ram: HashMap::new(),
            handlers: HashMap::new(),
        }
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
        self.handlers.insert(va, Box::new(f));
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
            return *self.sparse_ram.get(&addr).unwrap_or(&0);
        }
        let off = addr.wrapping_sub(self.rom_delta) as usize;
        if addr >= self.rom_delta && off < self.rom.len() {
            return self.rom[off];
        }
        *self.sparse_ram.get(&addr).unwrap_or(&0)
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
        self.sparse_ram.insert(addr, v);
    }

    #[inline(always)]
    pub fn r16(&self, addr: u32) -> u16 {
        ((self.r8(addr) as u16) << 8) | (self.r8(addr.wrapping_add(1)) as u16)
    }

    #[inline(always)]
    pub fn w16(&mut self, addr: u32, v: u16) {
        self.w8(addr, (v >> 8) as u8);
        self.w8(addr.wrapping_add(1), (v & 0xFF) as u8);
    }

    #[inline(always)]
    pub fn r32(&self, addr: u32) -> u32 {
        ((self.r16(addr) as u32) << 16) | (self.r16(addr.wrapping_add(2)) as u32)
    }

    #[inline(always)]
    pub fn w32(&mut self, addr: u32, v: u32) {
        self.w16(addr, (v >> 16) as u16);
        self.w16(addr.wrapping_add(2), (v & 0xFFFF) as u16);
    }

    #[inline(always)]
    fn set_cmp(&mut self, dst: u32, src: u32) {
        let sa = dst as i32 as i64;
        let sb = src as i32 as i64;
        let res = sa - sb;
        let r32 = res as u32;
        let n = (r32 & 0x8000_0000) != 0;
        let z = r32 == 0;
        let v = ((sa ^ sb) & (sa ^ res)) < 0;
        let c = dst < src;
        self.flags = (n, z, v, c);
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
        const SENTINEL: u32 = 0xDEAD_BEEF;
        let mut pc = start_va;
        self.rp = SENTINEL;
        let mut steps = 0u64;

        while steps < max_steps {
            if pc == SENTINEL {
                return Ok(steps);
            }
            if let Some(mut h) = self.handlers.remove(&pc) {
                let next_pc = h(self);
                self.handlers.insert(pc, h);
                pc = next_pc;
                continue;
            }
            let op = self.fetch16(pc);
            pc = pc.wrapping_add(2);
            steps += 1;

            let hi8 = (op >> 8) as u8;
            let lo8 = (op & 0xFF) as u8;
            let n3 = ((op >> 12) & 0xF) as usize;
            let n2 = ((op >> 8) & 0xF) as usize;
            let n1 = ((op >> 4) & 0xF) as usize;
            let n0 = (op & 0xF) as usize;

            if hi8 == 0x9F && n1 == 8 {
                self.r[n0] = self.r32(pc);
                pc = pc.wrapping_add(4);
            } else if op == 0x9FA0 {
                // nop
            } else if op == 0x9F90 {
                self.r[15] = self.r[14];
                self.r[14] = self.r32(self.r[15]);
                self.r[15] = self.r[15].wrapping_add(4);
            } else if hi8 == 0x0F {
                self.r[15] = self.r[15].wrapping_sub(4);
                self.w32(self.r[15], self.r[14]);
                self.r[14] = self.r[15];
                self.r[15] = self.r[15].wrapping_sub((lo8 as u32) * 4);
            } else if hi8 == 0x9B {
                self.r[n0] = ((n1 as u32) << 16) | (self.r16(pc) as u32);
                pc = pc.wrapping_add(2);
            } else if n3 == 0xC {
                self.r[n0] = ((n2 << 4) | n1) as u32;
            } else if hi8 == 0x8B {
                self.r[n0] = self.r[n1];
            } else if hi8 == 0xB7 && n1 == 5 {
                self.r[n0] = self.mdl;
            } else if hi8 == 0xA6 {
                self.r[n0] = self.r[n0].wrapping_add(self.r[n1]);
            } else if hi8 == 0xA4 {
                self.r[n0] = self.r[n0].wrapping_add(n1 as u32);
            } else if hi8 == 0xA5 {
                self.r[n0] = self.r[n0].wrapping_add(((n1 as i32) - 16) as u32);
            } else if hi8 == 0xAC {
                self.r[n0] = self.r[n0].wrapping_sub(self.r[n1]);
            } else if hi8 == 0xAF {
                self.mdl = (self.r[n0] as i32).wrapping_mul(self.r[n1] as i32) as u32;
            } else if hi8 == 0x82 {
                self.r[n0] &= self.r[n1];
                let v = self.r[n0];
                self.flags = ((v & 0x8000_0000) != 0, v == 0, false, false);
            } else if hi8 == 0x92 {
                self.r[n0] |= self.r[n1];
            } else if hi8 == 0x9A {
                self.r[n0] ^= self.r[n1];
            } else if hi8 == 0xAA {
                self.set_cmp(self.r[n0], self.r[n1]);
            } else if hi8 == 0xA8 {
                self.set_cmp(self.r[n0], n1 as u32);
            } else if hi8 == 0xB4 {
                self.r[n0] = self.r[n0].wrapping_shl(n1 as u32);
            } else if hi8 == 0xB0 {
                self.r[n0] = self.r[n0].wrapping_shr(n1 as u32);
            } else if hi8 == 0xB8 {
                self.r[n0] = ((self.r[n0] as i32) >> n1) as u32;
            } else if hi8 == 0x97 && n1 == 0x8 {
                self.r[n0] = (self.r[n0] as u8 as i8 as i32) as u32;
            } else if hi8 == 0x97 && n1 == 0x9 {
                self.r[n0] &= 0xFF;
            } else if hi8 == 0x97 && n1 == 0xA {
                self.r[n0] = (self.r[n0] as u16 as i16 as i32) as u32;
            } else if hi8 == 0x97 && n1 == 0xB {
                self.r[n0] &= 0xFFFF;
            } else if hi8 == 0x97 && n1 == 0 {
                pc = self.r[n0];
            } else if hi8 == 0x97 && n1 == 1 {
                self.rp = pc;
                pc = self.r[n0];
            } else if op == 0x9720 {
                pc = self.rp;
            } else if hi8 == 0x17 && n1 == 0 {
                self.r[15] = self.r[15].wrapping_sub(4);
                self.w32(self.r[15], self.r[n0]);
            } else if hi8 == 0x17 && n1 == 8 && n0 == 1 {
                self.r[15] = self.r[15].wrapping_sub(4);
                self.w32(self.r[15], self.rp);
            } else if hi8 == 0x07 && n1 == 0 {
                self.r[n0] = self.r32(self.r[15]);
                self.r[15] = self.r[15].wrapping_add(4);
            } else if hi8 == 0x07 && n1 == 8 && n0 == 1 {
                self.rp = self.r32(self.r[15]);
                self.r[15] = self.r[15].wrapping_add(4);
            } else if hi8 == 0xA3 {
                let d = (lo8 as i8 as i32) * 4;
                self.r[15] = self.r[15].wrapping_add(d as u32);
            } else if n3 == 0x2 {
                let d = ((((n2 << 4) | n1) as u8 as i8) as i32) * 4;
                self.r[n0] = self.r32(self.r[14].wrapping_add(d as u32));
            } else if n3 == 0x3 {
                let d = ((((n2 << 4) | n1) as u8 as i8) as i32) * 4;
                self.w32(self.r[14].wrapping_add(d as u32), self.r[n0]);
            } else if hi8 == 0x03 {
                self.r[n0] = self.r32(self.r[15].wrapping_add((n1 as u32) * 4));
            } else if hi8 == 0x13 {
                self.w32(self.r[15].wrapping_add((n1 as u32) * 4), self.r[n0]);
            } else if hi8 == 0x00 {
                self.r[n0] = self.r32(self.r[n1].wrapping_add(self.r[13]));
            } else if hi8 == 0x01 {
                self.r[n0] = self.r16(self.r[n1].wrapping_add(self.r[13])) as u32;
            } else if hi8 == 0x02 {
                self.r[n0] = self.r8(self.r[n1].wrapping_add(self.r[13])) as u32;
            } else if hi8 == 0x04 {
                self.r[n0] = self.r32(self.r[n1]);
            } else if hi8 == 0x05 {
                self.r[n0] = self.r16(self.r[n1]) as u32;
            } else if hi8 == 0x06 {
                self.r[n0] = self.r8(self.r[n1]) as u32;
            } else if hi8 == 0x10 {
                self.w32(self.r[n1].wrapping_add(self.r[13]), self.r[n0]);
            } else if hi8 == 0x11 {
                self.w16(self.r[n1].wrapping_add(self.r[13]), self.r[n0] as u16);
            } else if hi8 == 0x12 {
                self.w8(self.r[n1].wrapping_add(self.r[13]), self.r[n0] as u8);
            } else if hi8 == 0x14 {
                self.w32(self.r[n1], self.r[n0]);
            } else if hi8 == 0x15 {
                self.w16(self.r[n1], self.r[n0] as u16);
            } else if hi8 == 0x16 {
                self.w8(self.r[n1], self.r[n0] as u8);
            } else if n3 == 0xE {
                let (n, z, v, c) = self.flags;
                let take = match n2 {
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
            } else {
                return Err(format!("unsupported FR80 op 0x{:04X} @ 0x{:06X}", op, pc - 2));
            }
        }
        Err(format!("FR80 step limit ({}) reached @ 0x{:06X}", max_steps, pc))
    }
}
