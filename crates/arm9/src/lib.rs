//! Big-endian ARM926EJ-S (ARMv5TE + Thumb-1) interpreter for Milbeaut M6M (`C78FRM.bin`).

pub const SDRAM_BASE: u32 = 0xA000_0000;
pub const SDRAM_SIZE: usize = 0x0800_0000; // 128 MB: 0xA0000000..0xA8000000
pub const RET_SENTINEL: u32 = 0xA27F_0000;

const CODE_BITMAP_SPAN: usize = 0x0020_0000; // 2 MB of code halfwords (0xA0000000..0xA0200000)

pub type Hook = Box<dyn FnMut(&mut Cpu) -> HookAction>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HookAction {
    /// Hook handled the call/site and updated `pc` (or wants `bx lr` via `Return(r0)`).
    Jump(u32),
    /// Return to `LR` (`r[14]`), optionally setting `r0`.
    Return(Option<u32>),
    /// Run the underlying instruction after inspecting state (`watch` semantics).
    Continue,
}

struct RangeStub {
    lo: u32,
    hi: u32,
    ret: Option<u32>,
}

pub struct Cpu {
    pub r: [u32; 16],
    pub n: bool,
    pub z: bool,
    pub c: bool,
    pub v: bool,
    pub q: bool,
    pub thumb: bool,
    pub cpsr_low: u32,
    pub sdram: Vec<u8>,
    pub total_steps: u64,
    pub last_error: Option<String>,
    pub watch_hits: Vec<(u32, u32, u32)>,
    watch_range: Option<(u32, u32)>,
    mmio: Vec<(u32, Box<[u8; 4096]>)>,
    hook_bits: Vec<u64>,
    hooks: Vec<(u32, Hook)>,
    stubs: Vec<RangeStub>,
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Cpu {
    pub fn new() -> Self {
        Self {
            r: [0; 16],
            n: false,
            z: false,
            c: false,
            v: false,
            q: false,
            thumb: false,
            cpsr_low: 0x13,
            sdram: vec![0; SDRAM_SIZE],
            total_steps: 0,
            last_error: None,
            watch_hits: Vec::new(),
            watch_range: None,
            mmio: Vec::new(),
            hook_bits: vec![0; (CODE_BITMAP_SPAN / 2) / 64],
            hooks: Vec::new(),
            stubs: Vec::new(),
        }
    }

    pub fn load_s3(&mut self, s3: &[u8]) {
        let n = s3.len().min(SDRAM_SIZE);
        self.sdram[..n].copy_from_slice(&s3[..n]);
    }

    #[inline]
    pub fn cpsr(&self) -> u32 {
        ((self.n as u32) << 31)
            | ((self.z as u32) << 30)
            | ((self.c as u32) << 29)
            | ((self.v as u32) << 28)
            | ((self.q as u32) << 27)
            | ((self.thumb as u32) << 5)
            | (self.cpsr_low & 0x07FF_FFDF)
    }

    #[inline]
    pub fn set_cpsr(&mut self, val: u32, mask: u32) {
        let cur = (self.cpsr() & !mask) | (val & mask);
        self.n = (cur >> 31) & 1 != 0;
        self.z = (cur >> 30) & 1 != 0;
        self.c = (cur >> 29) & 1 != 0;
        self.v = (cur >> 28) & 1 != 0;
        self.q = (cur >> 27) & 1 != 0;
        self.thumb = (cur >> 5) & 1 != 0;
        self.cpsr_low = cur & 0x07FF_FFDF;
    }

    #[inline(always)]
    pub fn sdram(&self) -> &[u8] {
        &self.sdram
    }

    #[inline(always)]
    pub fn sdram_mut(&mut self) -> &mut [u8] {
        &mut self.sdram
    }

    #[inline(always)]
    pub fn r8(&self, va: u32) -> u8 {
        let off = va.wrapping_sub(SDRAM_BASE) as usize;
        if off < SDRAM_SIZE {
            self.sdram[off]
        } else {
            self.mmio_r8(va)
        }
    }

    #[inline(always)]
    pub fn r16(&self, va: u32) -> u16 {
        let va = va & !1;
        let off = va.wrapping_sub(SDRAM_BASE) as usize;
        if off + 2 <= SDRAM_SIZE {
            u16::from_be_bytes([self.sdram[off], self.sdram[off + 1]])
        } else {
            u16::from_be_bytes([self.mmio_r8(va), self.mmio_r8(va.wrapping_add(1))])
        }
    }

    #[inline(always)]
    pub fn r16_be(&self, va: u32) -> u16 {
        u16::from_be_bytes([self.r8(va), self.r8(va.wrapping_add(1))])
    }

    #[inline(always)]
    pub fn r32(&self, va: u32) -> u32 {
        let va = va & !3;
        let off = va.wrapping_sub(SDRAM_BASE) as usize;
        if off + 4 <= SDRAM_SIZE {
            let s = &self.sdram[off..off + 4];
            u32::from_be_bytes([s[0], s[1], s[2], s[3]])
        } else {
            u32::from_be_bytes([
                self.mmio_r8(va),
                self.mmio_r8(va.wrapping_add(1)),
                self.mmio_r8(va.wrapping_add(2)),
                self.mmio_r8(va.wrapping_add(3)),
            ])
        }
    }

    #[inline(always)]
    pub fn r32_be(&self, va: u32) -> u32 {
        u32::from_be_bytes([
            self.r8(va),
            self.r8(va.wrapping_add(1)),
            self.r8(va.wrapping_add(2)),
            self.r8(va.wrapping_add(3)),
        ])
    }

    pub fn set_write_watch(&mut self, range: Option<(u32, u32)>) {
        self.watch_range = range;
        self.watch_hits.clear();
    }

    #[cold]
    fn record_watch(&mut self, va: u32) {
        if self.watch_hits.len() < 64 {
            self.watch_hits.push((self.r[15], self.r[14], va));
        }
    }

    #[inline(always)]
    fn check_watch(&mut self, va: u32, end: u32) {
        if let Some((lo, hi)) = self.watch_range
            && va < hi
            && end > lo
        {
            self.record_watch(va);
        }
    }

    #[inline(always)]
    pub fn w8(&mut self, va: u32, val: u8) {
        self.check_watch(va, va.saturating_add(1));
        let off = va.wrapping_sub(SDRAM_BASE) as usize;
        if off < SDRAM_SIZE {
            self.sdram[off] = val;
        } else {
            self.mmio_w8(va, val);
        }
    }

    #[inline(always)]
    pub fn w16(&mut self, va: u32, val: u16) {
        let va = va & !1;
        self.check_watch(va, va.saturating_add(2));
        let off = va.wrapping_sub(SDRAM_BASE) as usize;
        if off + 2 <= SDRAM_SIZE {
            let b = val.to_be_bytes();
            self.sdram[off] = b[0];
            self.sdram[off + 1] = b[1];
        } else {
            let b = val.to_be_bytes();
            self.mmio_w8(va, b[0]);
            self.mmio_w8(va.wrapping_add(1), b[1]);
        }
    }

    #[inline(always)]
    pub fn w32(&mut self, va: u32, val: u32) {
        let va = va & !3;
        self.check_watch(va, va.saturating_add(4));
        let off = va.wrapping_sub(SDRAM_BASE) as usize;
        if off + 4 <= SDRAM_SIZE {
            self.sdram[off..off + 4].copy_from_slice(&val.to_be_bytes());
        } else {
            let b = val.to_be_bytes();
            for (i, &x) in b.iter().enumerate() {
                self.mmio_w8(va.wrapping_add(i as u32), x);
            }
        }
    }

    pub fn read_bytes(&self, va: u32, dst: &mut [u8]) {
        let off = va.wrapping_sub(SDRAM_BASE) as usize;
        if off.saturating_add(dst.len()) <= SDRAM_SIZE {
            dst.copy_from_slice(&self.sdram[off..off + dst.len()]);
        } else {
            for (i, b) in dst.iter_mut().enumerate() {
                *b = self.r8(va.wrapping_add(i as u32));
            }
        }
    }

    pub fn write_bytes(&mut self, va: u32, src: &[u8]) {
        if !src.is_empty() {
            self.check_watch(va, va.saturating_add(src.len() as u32));
        }
        let off = va.wrapping_sub(SDRAM_BASE) as usize;
        if off.saturating_add(src.len()) <= SDRAM_SIZE {
            self.sdram[off..off + src.len()].copy_from_slice(src);
        } else {
            for (i, &b) in src.iter().enumerate() {
                self.w8(va.wrapping_add(i as u32), b);
            }
        }
    }

    #[cold]
    fn mmio_r8(&self, va: u32) -> u8 {
        let page = va & !0xFFF;
        for (p, data) in &self.mmio {
            if *p == page {
                return data[(va & 0xFFF) as usize];
            }
        }
        0
    }

    #[cold]
    fn mmio_w8(&mut self, va: u32, val: u8) {
        let page = va & !0xFFF;
        for (p, data) in &mut self.mmio {
            if *p == page {
                data[(va & 0xFFF) as usize] = val;
                return;
            }
        }
        let mut pg = Box::new([0u8; 4096]);
        pg[(va & 0xFFF) as usize] = val;
        self.mmio.push((page, pg));
    }

    fn mark_bit(&mut self, va: u32) {
        let off = (va & !1).wrapping_sub(SDRAM_BASE) as usize;
        if off < CODE_BITMAP_SPAN {
            let idx = off >> 1;
            self.hook_bits[idx >> 6] |= 1u64 << (idx & 63);
        }
    }

    pub fn add_hook(&mut self, va: u32, f: impl FnMut(&mut Cpu) -> HookAction + 'static) {
        let va = va & !1;
        self.mark_bit(va);
        if let Some(slot) = self.hooks.iter_mut().find(|(a, _)| *a == va) {
            slot.1 = Box::new(f);
        } else {
            self.hooks.push((va, Box::new(f)));
        }
    }

    pub fn stub(&mut self, va: u32, ret: Option<u32>) {
        self.add_hook(va, move |_| HookAction::Return(ret));
    }

    pub fn stub_range(&mut self, lo: u32, hi: u32, ret: Option<u32>) {
        let lo = lo & !1;
        let hi = hi & !1;
        let mut a = lo;
        while a < hi {
            self.mark_bit(a);
            a = a.wrapping_add(2);
        }
        self.stubs.push(RangeStub { lo, hi, ret });
    }

    #[inline]
    pub fn branch_exchange(&mut self, target: u32) {
        if target & 1 != 0 {
            self.thumb = true;
            self.r[15] = target & !1;
        } else {
            self.thumb = false;
            self.r[15] = target & !3;
        }
    }

    pub fn call(&mut self, entry: u32, args: &[u32], stack_args: &[u32], sp: u32, max_steps: u64) -> Result<(u32, u64), String> {
        self.r[13] = sp;
        for (i, &v) in stack_args.iter().enumerate() {
            self.w32(sp.wrapping_add((i as u32) * 4), v);
        }
        for (i, &v) in args.iter().take(4).enumerate() {
            self.r[i] = v;
        }
        self.r[14] = RET_SENTINEL | 1;
        self.branch_exchange(entry);
        let steps = self.run(RET_SENTINEL, max_steps)?;
        if self.r[13] != sp {
            let err = format!("call {:#x}: SP imbalance ({:#x} != {:#x})", entry, self.r[13], sp);
            self.last_error = Some(err.clone());
            return Err(err);
        }
        Ok((self.r[0], steps))
    }

    pub fn run(&mut self, until: u32, max_steps: u64) -> Result<u64, String> {
        let until = until & !1;
        let mut step = 0u64;
        while step < max_steps {
            let mut pc = self.r[15];
            if pc == until {
                self.total_steps = self.total_steps.wrapping_add(step);
                return Ok(step);
            }
            let off = pc.wrapping_sub(SDRAM_BASE) as usize;
            if off >= SDRAM_SIZE {
                self.total_steps = self.total_steps.wrapping_add(step);
                let mut stack_dump = Vec::new();
                for k in 0..24u32 {
                    stack_dump.push(format!("{:#010x}", self.r32_be(self.r[13].wrapping_add(k * 4))));
                }
                let err = format!(
                    "ARM9 wild PC={:#010x} ({}) LR={:#010x} SP={:#010x} R0..R7=[{:#x},{:#x},{:#x},{:#x},{:#x},{:#x},{:#x},{:#x}] STK=[{}]",
                    pc,
                    if self.thumb { "Thumb" } else { "ARM" },
                    self.r[14],
                    self.r[13],
                    self.r[0], self.r[1], self.r[2], self.r[3], self.r[4], self.r[5], self.r[6], self.r[7],
                    stack_dump.join(",")
                );
                self.last_error = Some(err.clone());
                return Err(err);
            }
            if off < CODE_BITMAP_SPAN {
                let word = self.hook_bits[off >> 7];
                if word == 0 {
                    // All 64 halfwords in this 128-byte SDRAM line are hook-free
                    let line = pc & !0x7F;
                    loop {
                        let res = if self.thumb {
                            self.step_thumb(pc)
                        } else {
                            self.step_arm(pc)
                        };
                        if let Err(e) = res {
                            self.total_steps = self.total_steps.wrapping_add(step);
                            self.last_error = Some(e.clone());
                            return Err(e);
                        }
                        step += 1;
                        pc = self.r[15];
                        if (pc & !0x7F) != line || pc == until || step >= max_steps {
                            break;
                        }
                    }
                    continue;
                }
                let bit = (off >> 1) & 63;
                if (word >> bit) & 1 != 0 && self.fire_hook(pc) {
                    step += 1;
                    continue;
                }
            }
            let res = if self.thumb {
                self.step_thumb(pc)
            } else {
                self.step_arm(pc)
            };
            if let Err(e) = res {
                self.total_steps = self.total_steps.wrapping_add(step);
                self.last_error = Some(e.clone());
                return Err(e);
            }
            step += 1;
        }
        self.total_steps = self.total_steps.wrapping_add(step);
        let err = format!(
            "ARM9 step limit ({}) at PC={:#010x} ({}) LR={:#010x}",
            max_steps,
            self.r[15],
            if self.thumb { "Thumb" } else { "ARM" },
            self.r[14]
        );
        self.last_error = Some(err.clone());
        Err(err)
    }

    #[cold]
    fn fire_hook(&mut self, pc: u32) -> bool {
        if let Some(pos) = self.hooks.iter().position(|(a, _)| *a == pc) {
            let mut h = self.hooks.swap_remove(pos);
            let act = (h.1)(self);
            self.hooks.push(h);
            match act {
                HookAction::Jump(tgt) => {
                    self.branch_exchange(tgt);
                    return true;
                }
                HookAction::Return(ret) => {
                    if let Some(v) = ret {
                        self.r[0] = v;
                    }
                    let lr = self.r[14];
                    self.branch_exchange(lr);
                    return true;
                }
                HookAction::Continue => return false,
            }
        }
        for s in &self.stubs {
            if pc >= s.lo && pc < s.hi {
                if let Some(v) = s.ret {
                    self.r[0] = v;
                }
                let lr = self.r[14];
                self.branch_exchange(lr);
                return true;
            }
        }
        false
    }

    #[inline(always)]
    fn cond_ok(&self, cond: u32) -> bool {
        match cond {
            0x0 => self.z,
            0x1 => !self.z,
            0x2 => self.c,
            0x3 => !self.c,
            0x4 => self.n,
            0x5 => !self.n,
            0x6 => self.v,
            0x7 => !self.v,
            0x8 => self.c && !self.z,
            0x9 => !self.c || self.z,
            0xA => self.n == self.v,
            0xB => self.n != self.v,
            0xC => !self.z && (self.n == self.v),
            0xD => self.z || (self.n != self.v),
            0xE => true,
            _ => false,
        }
    }

    #[inline(always)]
    fn set_nz(&mut self, res: u32) {
        self.n = (res as i32) < 0;
        self.z = res == 0;
    }

    #[inline(always)]
    fn add_flags(&mut self, a: u32, b: u32) -> u32 {
        let (res, c) = a.overflowing_add(b);
        let (_, v) = (a as i32).overflowing_add(b as i32);
        self.n = (res as i32) < 0;
        self.z = res == 0;
        self.c = c;
        self.v = v;
        res
    }

    #[inline(always)]
    fn sub_flags(&mut self, a: u32, b: u32) -> u32 {
        let (res, borrow) = a.overflowing_sub(b);
        let (_, v) = (a as i32).overflowing_sub(b as i32);
        self.n = (res as i32) < 0;
        self.z = res == 0;
        self.c = !borrow;
        self.v = v;
        res
    }

    #[inline(always)]
    fn adc_flags(&mut self, a: u32, b: u32, cin: bool) -> u32 {
        let full = (a as u64) + (b as u64) + (cin as u64);
        let res = full as u32;
        self.n = (res as i32) < 0;
        self.z = res == 0;
        self.c = full >> 32 != 0;
        self.v = ((!(a ^ b) & (a ^ res)) >> 31) != 0;
        res
    }

    #[inline(always)]
    fn sbc_flags(&mut self, a: u32, b: u32, cin: bool) -> u32 {
        self.adc_flags(a, !b, cin)
    }

    #[inline(always)]
    fn arm_reg(&self, idx: usize, pc8: u32) -> u32 {
        if idx == 15 { pc8 } else { self.r[idx] }
    }

    #[inline(always)]
    fn shift_imm(&self, val: u32, stype: u32, imm: u32, cin: bool) -> (u32, bool) {
        match stype {
            0 => {
                if imm == 0 {
                    (val, cin)
                } else {
                    (val << imm, (val >> (32 - imm)) & 1 != 0)
                }
            }
            1 => {
                if imm == 0 {
                    (0, (val >> 31) != 0)
                } else {
                    (val >> imm, (val >> (imm - 1)) & 1 != 0)
                }
            }
            2 => {
                if imm == 0 {
                    let bit = (val >> 31) != 0;
                    (if bit { u32::MAX } else { 0 }, bit)
                } else {
                    (((val as i32) >> imm) as u32, (val >> (imm - 1)) & 1 != 0)
                }
            }
            _ => {
                if imm == 0 {
                    ((val >> 1) | ((cin as u32) << 31), (val & 1) != 0)
                } else {
                    (val.rotate_right(imm), (val >> (imm - 1)) & 1 != 0)
                }
            }
        }
    }

    #[inline(always)]
    fn shift_reg(&self, val: u32, stype: u32, rs: u32, cin: bool) -> (u32, bool) {
        let amt = rs & 0xFF;
        if amt == 0 {
            return (val, cin);
        }
        match stype {
            0 => {
                if amt < 32 {
                    (val << amt, (val >> (32 - amt)) & 1 != 0)
                } else if amt == 32 {
                    (0, (val & 1) != 0)
                } else {
                    (0, false)
                }
            }
            1 => {
                if amt < 32 {
                    (val >> amt, (val >> (amt - 1)) & 1 != 0)
                } else if amt == 32 {
                    (0, (val >> 31) != 0)
                } else {
                    (0, false)
                }
            }
            2 => {
                if amt < 32 {
                    (((val as i32) >> amt) as u32, (val >> (amt - 1)) & 1 != 0)
                } else {
                    let bit = (val >> 31) != 0;
                    (if bit { u32::MAX } else { 0 }, bit)
                }
            }
            _ => {
                let r = amt & 31;
                if r == 0 {
                    (val, (val >> 31) != 0)
                } else {
                    (val.rotate_right(r), (val >> (r - 1)) & 1 != 0)
                }
            }
        }
    }

    fn step_arm(&mut self, pc: u32) -> Result<(), String> {
        let insn = self.r32(pc);
        let cond = insn >> 28;
        if cond == 0xF {
            // Unconditional ARMv5TE space: BLX #imm or PLD/cache hint
            if (insn & 0x0E00_0000) == 0x0A00_0000 {
                let h = (insn >> 23) & 2;
                let off26 = (((insn & 0x00FF_FFFF) << 2) | h) as i32;
                let simm = (off26 << 6) >> 6;
                self.r[14] = pc.wrapping_add(4);
                self.thumb = true;
                self.r[15] = pc.wrapping_add(8).wrapping_add(simm as u32) & !1;
            } else {
                self.r[15] = pc.wrapping_add(4);
            }
            return Ok(());
        }
        if !self.cond_ok(cond) {
            self.r[15] = pc.wrapping_add(4);
            return Ok(());
        }

        let pc8 = pc.wrapping_add(8);
        self.r[15] = pc.wrapping_add(4);
        let top3 = (insn >> 25) & 7;

        match top3 {
            0b101 => {
                // B / BL
                if (insn >> 24) & 1 != 0 {
                    self.r[14] = pc.wrapping_add(4);
                }
                let simm = (((insn & 0x00FF_FFFF) as i32) << 8) >> 6;
                self.r[15] = pc8.wrapping_add(simm as u32) & !3;
            }
            0b100 => {
                // Block Data Transfer: LDM / STM
                let p = (insn >> 24) & 1 != 0;
                let u = (insn >> 23) & 1 != 0;
                let w = (insn >> 21) & 1 != 0;
                let l = (insn >> 20) & 1 != 0;
                let rn = ((insn >> 16) & 0xF) as usize;
                let list = insn & 0xFFFF;
                let count = list.count_ones();
                let base = self.r[rn];
                let (mut addr, wb) = match (p, u) {
                    (false, true) => (base, base.wrapping_add(count * 4)),
                    (true, true) => (base.wrapping_add(4), base.wrapping_add(count * 4)),
                    (false, false) => (base.wrapping_sub(count * 4).wrapping_add(4), base.wrapping_sub(count * 4)),
                    (true, false) => (base.wrapping_sub(count * 4), base.wrapping_sub(count * 4)),
                };
                if l {
                    if w && (list & (1 << rn)) == 0 {
                        self.r[rn] = wb;
                    }
                    for reg in 0..16usize {
                        if (list >> reg) & 1 != 0 {
                            let val = self.r32(addr);
                            addr = addr.wrapping_add(4);
                            if reg == 15 {
                                self.branch_exchange(val);
                            } else {
                                self.r[reg] = val;
                            }
                        }
                    }
                } else {
                    for reg in 0..16usize {
                        if (list >> reg) & 1 != 0 {
                            let val = if reg == 15 { pc8 } else { self.r[reg] };
                            self.w32(addr, val);
                            addr = addr.wrapping_add(4);
                        }
                    }
                    if w {
                        self.r[rn] = wb;
                    }
                }
            }
            0b010 | 0b011 => {
                // Single Data Transfer: LDR / STR / LDRB / STRB
                if top3 == 0b011 && (insn & 0x10) != 0 {
                    return Err(format!("ARM undefined @ {:#010x}: {:#010x}", pc, insn));
                }
                let reg_off = top3 == 0b011;
                let p = (insn >> 24) & 1 != 0;
                let u = (insn >> 23) & 1 != 0;
                let b = (insn >> 22) & 1 != 0;
                let w = (insn >> 21) & 1 != 0;
                let l = (insn >> 20) & 1 != 0;
                let rn = ((insn >> 16) & 0xF) as usize;
                let rd = ((insn >> 12) & 0xF) as usize;
                let base = self.arm_reg(rn, pc8);
                let off = if !reg_off {
                    insn & 0xFFF
                } else {
                    let rm = (insn & 0xF) as usize;
                    let stype = (insn >> 5) & 3;
                    let imm = (insn >> 7) & 0x1F;
                    self.shift_imm(self.arm_reg(rm, pc8), stype, imm, self.c).0
                };
                let ea = if u { base.wrapping_add(off) } else { base.wrapping_sub(off) };
                let addr = if p { ea } else { base };
                if l {
                    let val = if b { self.r8(addr) as u32 } else { self.r32(addr) };
                    if (!p || w) && rn != rd {
                        self.r[rn] = ea;
                    }
                    if rd == 15 {
                        self.branch_exchange(val);
                    } else {
                        self.r[rd] = val;
                    }
                } else {
                    let val = self.arm_reg(rd, pc8);
                    if b {
                        self.w8(addr, val as u8);
                    } else {
                        self.w32(addr, val);
                    }
                    if !p || w {
                        self.r[rn] = ea;
                    }
                }
            }
            0b000 | 0b001 => self.step_arm_dp_misc(pc, pc8, insn)?,
            0b110 | 0b111 => {
                // SVC / Coprocessor (MRC p15 -> 0, MCR/LDC/STC -> NOP)
                if (insn & 0x0F10_0010) == 0x0E10_0010 {
                    let rd = ((insn >> 12) & 0xF) as usize;
                    if rd != 15 {
                        self.r[rd] = 0;
                    }
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    fn step_arm_dp_misc(&mut self, pc: u32, pc8: u32, insn: u32) -> Result<(), String> {
        let is_imm = (insn >> 25) & 1 != 0;

        // Multiply / Extra Load-Store (when I=0, bit7=1, bit4=1)
        if !is_imm && (insn & 0x90) == 0x90 {
            let op_sh = (insn >> 5) & 3;
            if op_sh == 0 {
                let top4 = (insn >> 24) & 0xF;
                if top4 == 0x1 {
                    // SWP / SWPB
                    let b = (insn >> 22) & 1 != 0;
                    let rn = ((insn >> 16) & 0xF) as usize;
                    let rd = ((insn >> 12) & 0xF) as usize;
                    let rm = (insn & 0xF) as usize;
                    let addr = self.r[rn];
                    let src = self.r[rm];
                    if b {
                        let old = self.r8(addr) as u32;
                        self.w8(addr, src as u8);
                        self.r[rd] = old;
                    } else {
                        let old = self.r32(addr);
                        self.w32(addr, src);
                        self.r[rd] = old;
                    }
                    return Ok(());
                }
                let mul_op = (insn >> 21) & 7;
                let s = (insn >> 20) & 1 != 0;
                let rd = ((insn >> 16) & 0xF) as usize;
                let rn = ((insn >> 12) & 0xF) as usize;
                let rs = ((insn >> 8) & 0xF) as usize;
                let rm = (insn & 0xF) as usize;
                match mul_op {
                    0 | 1 => {
                        let mut res = self.r[rm].wrapping_mul(self.r[rs]);
                        if mul_op == 1 {
                            res = res.wrapping_add(self.r[rn]);
                        }
                        self.r[rd] = res;
                        if s {
                            self.set_nz(res);
                        }
                    }
                    4..=7 => {
                        let signed = (mul_op & 2) != 0;
                        let accum = (mul_op & 1) != 0;
                        let mut prod: u64 = if signed {
                            ((self.r[rm] as i32 as i64) * (self.r[rs] as i32 as i64)) as u64
                        } else {
                            (self.r[rm] as u64) * (self.r[rs] as u64)
                        };
                        if accum {
                            let acc = ((self.r[rd] as u64) << 32) | (self.r[rn] as u64);
                            prod = prod.wrapping_add(acc);
                        }
                        self.r[rn] = prod as u32;
                        self.r[rd] = (prod >> 32) as u32;
                        if s {
                            self.n = (self.r[rd] as i32) < 0;
                            self.z = prod == 0;
                        }
                    }
                    _ => return Err(format!("ARM unknown mul @ {:#010x}: {:#010x}", pc, insn)),
                }
                return Ok(());
            }

            // Extra Load/Store: STRH, LDRD, STRD, LDRH, LDRSB, LDRSH
            let p = (insn >> 24) & 1 != 0;
            let u = (insn >> 23) & 1 != 0;
            let i_imm = (insn >> 22) & 1 != 0;
            let w = (insn >> 21) & 1 != 0;
            let l = (insn >> 20) & 1 != 0;
            let rn = ((insn >> 16) & 0xF) as usize;
            let rd = ((insn >> 12) & 0xF) as usize;
            let base = self.arm_reg(rn, pc8);
            let off = if i_imm {
                ((insn >> 4) & 0xF0) | (insn & 0xF)
            } else {
                self.arm_reg((insn & 0xF) as usize, pc8)
            };
            let ea = if u { base.wrapping_add(off) } else { base.wrapping_sub(off) };
            let addr = if p { ea } else { base };
            match (l, op_sh) {
                (false, 1) => {
                    self.w16(addr, self.arm_reg(rd, pc8) as u16);
                    if !p || w {
                        self.r[rn] = ea;
                    }
                }
                (true, 1) => {
                    let v = self.r16(addr) as u32;
                    if (!p || w) && rn != rd {
                        self.r[rn] = ea;
                    }
                    self.r[rd] = v;
                }
                (true, 2) => {
                    let v = (self.r8(addr) as i8) as i32 as u32;
                    if (!p || w) && rn != rd {
                        self.r[rn] = ea;
                    }
                    self.r[rd] = v;
                }
                (true, 3) => {
                    let v = (self.r16(addr) as i16) as i32 as u32;
                    if (!p || w) && rn != rd {
                        self.r[rn] = ea;
                    }
                    self.r[rd] = v;
                }
                (false, 2) => {
                    // LDRD (ARMv5TE: L=0, S=1, H=0)
                    let v0 = self.r32(addr);
                    let v1 = self.r32(addr.wrapping_add(4));
                    if (!p || w) && rn != rd && rn != rd + 1 {
                        self.r[rn] = ea;
                    }
                    self.r[rd] = v0;
                    self.r[rd + 1] = v1;
                }
                (false, 3) => {
                    // STRD (ARMv5TE: L=0, S=1, H=1)
                    self.w32(addr, self.r[rd]);
                    self.w32(addr.wrapping_add(4), self.r[rd + 1]);
                    if !p || w {
                        self.r[rn] = ea;
                    }
                }
                _ => return Err(format!("ARM extra mem @ {:#010x}: {:#010x}", pc, insn)),
            }
            return Ok(());
        }

        let opcode = (insn >> 21) & 0xF;
        let s = (insn >> 20) & 1 != 0;

        // Miscellaneous space: !S && opcode in 8..=11
        if !s && (8..=11).contains(&opcode) {
            if !is_imm && (insn & 0x90) == 0x80 {
                // ARMv5TE 16-bit DSP multiplies: SMLAxy, SMLAWy, SMULWy, SMLALxy, SMULxy
                let rd = ((insn >> 16) & 0xF) as usize;
                let rn = ((insn >> 12) & 0xF) as usize;
                let rs = ((insn >> 8) & 0xF) as usize;
                let rm = (insn & 0xF) as usize;
                let x_top = (insn >> 5) & 1 != 0;
                let y_top = (insn >> 6) & 1 != 0;
                let mx = if x_top { (self.r[rm] >> 16) as i16 } else { self.r[rm] as i16 } as i32;
                let sy = if y_top { (self.r[rs] >> 16) as i16 } else { self.r[rs] as i16 } as i32;
                match opcode {
                    8 => {
                        let (res, ovf) = (mx * sy).overflowing_add(self.r[rn] as i32);
                        if ovf {
                            self.q = true;
                        }
                        self.r[rd] = res as u32;
                    }
                    9 => {
                        let prod = ((self.r[rm] as i32 as i64 * sy as i64) >> 16) as i32;
                        if x_top {
                            self.r[rd] = prod as u32;
                        } else {
                            let (res, ovf) = prod.overflowing_add(self.r[rn] as i32);
                            if ovf {
                                self.q = true;
                            }
                            self.r[rd] = res as u32;
                        }
                    }
                    10 => {
                        let acc = ((self.r[rd] as u64) << 32 | (self.r[rn] as u64)) as i64;
                        let sum = acc.wrapping_add((mx as i64) * (sy as i64)) as u64;
                        self.r[rn] = sum as u32;
                        self.r[rd] = (sum >> 32) as u32;
                    }
                    11 => {
                        self.r[rd] = (mx * sy) as u32;
                    }
                    _ => unreachable!(),
                }
                return Ok(());
            }
            if !is_imm {
                let sub = (insn >> 4) & 0xF;
                match sub {
                    0 => {
                        if opcode & 1 == 0 {
                            // MRS
                            let rd = ((insn >> 12) & 0xF) as usize;
                            self.r[rd] = self.cpsr();
                        } else {
                            // MSR reg
                            let mask = msr_mask((insn >> 16) & 0xF);
                            let val = self.r[(insn & 0xF) as usize];
                            self.set_cpsr(val, mask);
                        }
                        return Ok(());
                    }
                    1 => {
                        if opcode == 9 {
                            // BX Rm
                            let tgt = self.arm_reg((insn & 0xF) as usize, pc8);
                            self.branch_exchange(tgt);
                        } else if opcode == 11 {
                            // CLZ Rd, Rm
                            let rd = ((insn >> 12) & 0xF) as usize;
                            let rm = (insn & 0xF) as usize;
                            self.r[rd] = self.r[rm].leading_zeros();
                        }
                        return Ok(());
                    }
                    3 => {
                        // BLX Rm
                        let tgt = self.arm_reg((insn & 0xF) as usize, pc8);
                        self.r[14] = pc.wrapping_add(4);
                        self.branch_exchange(tgt);
                        return Ok(());
                    }
                    5 => {
                        // QADD / QSUB / QDADD / QDSUB
                        let rn = self.r[((insn >> 16) & 0xF) as usize] as i32;
                        let rm = self.r[(insn & 0xF) as usize] as i32;
                        let rd = ((insn >> 12) & 0xF) as usize;
                        let mut q = false;
                        let rhs = if opcode >= 10 {
                            let (d, o) = rn.overflowing_mul(2);
                            if o {
                                q = true;
                                if rn < 0 { i32::MIN } else { i32::MAX }
                            } else {
                                d
                            }
                        } else {
                            rn
                        };
                        let res = if opcode & 1 == 0 {
                            let (v, o) = rm.overflowing_add(rhs);
                            if o {
                                q = true;
                                if rm < 0 { i32::MIN } else { i32::MAX }
                            } else {
                                v
                            }
                        } else {
                            let (v, o) = rm.overflowing_sub(rhs);
                            if o {
                                q = true;
                                if rm < 0 { i32::MIN } else { i32::MAX }
                            } else {
                                v
                            }
                        };
                        if q {
                            self.q = true;
                        }
                        self.r[rd] = res as u32;
                        return Ok(());
                    }
                    _ => return Err(format!("ARM misc @ {:#010x}: {:#010x}", pc, insn)),
                }
            } else if opcode & 1 == 1 {
                // MSR imm
                let rot = ((insn >> 8) & 0xF) * 2;
                let imm = (insn & 0xFF).rotate_right(rot);
                let mask = msr_mask((insn >> 16) & 0xF);
                self.set_cpsr(imm, mask);
                return Ok(());
            }
        }

        // Standard Data Processing
        let rn_idx = ((insn >> 16) & 0xF) as usize;
        let rd = ((insn >> 12) & 0xF) as usize;
        let reg_shift = !is_imm && ((insn >> 4) & 1 != 0);
        let r15_val = if reg_shift { pc.wrapping_add(12) } else { pc8 };
        let rn = self.arm_reg(rn_idx, r15_val);
        let (op2, sc) = if is_imm {
            let rot = ((insn >> 8) & 0xF) * 2;
            let v = (insn & 0xFF).rotate_right(rot);
            let c = if rot == 0 { self.c } else { (v >> 31) != 0 };
            (v, c)
        } else {
            let rm = self.arm_reg((insn & 0xF) as usize, r15_val);
            let stype = (insn >> 5) & 3;
            if reg_shift {
                let rs = self.r[((insn >> 8) & 0xF) as usize];
                self.shift_reg(rm, stype, rs, self.c)
            } else {
                self.shift_imm(rm, stype, (insn >> 7) & 0x1F, self.c)
            }
        };

        let cin = self.c;
        let write_rd = !matches!(opcode, 8..=11);
        let logic_s = s && rd != 15;
        let res = match opcode {
            0x0 => {
                let r = rn & op2;
                if logic_s { self.set_nz(r); self.c = sc; }
                r
            }
            0x1 => {
                let r = rn ^ op2;
                if logic_s { self.set_nz(r); self.c = sc; }
                r
            }
            0x2 => if s && rd != 15 { self.sub_flags(rn, op2) } else { rn.wrapping_sub(op2) },
            0x3 => if s && rd != 15 { self.sub_flags(op2, rn) } else { op2.wrapping_sub(rn) },
            0x4 => if s && rd != 15 { self.add_flags(rn, op2) } else { rn.wrapping_add(op2) },
            0x5 => if s && rd != 15 { self.adc_flags(rn, op2, cin) } else { rn.wrapping_add(op2).wrapping_add(cin as u32) },
            0x6 => if s && rd != 15 { self.sbc_flags(rn, op2, cin) } else { rn.wrapping_add(!op2).wrapping_add(cin as u32) },
            0x7 => if s && rd != 15 { self.sbc_flags(op2, rn, cin) } else { op2.wrapping_add(!rn).wrapping_add(cin as u32) },
            0x8 => { self.set_nz(rn & op2); self.c = sc; 0 }
            0x9 => { self.set_nz(rn ^ op2); self.c = sc; 0 }
            0xA => { self.sub_flags(rn, op2); 0 }
            0xB => { self.add_flags(rn, op2); 0 }
            0xC => {
                let r = rn | op2;
                if logic_s { self.set_nz(r); self.c = sc; }
                r
            }
            0xD => {
                if logic_s { self.set_nz(op2); self.c = sc; }
                op2
            }
            0xE => {
                let r = rn & !op2;
                if logic_s { self.set_nz(r); self.c = sc; }
                r
            }
            0xF => {
                let r = !op2;
                if logic_s { self.set_nz(r); self.c = sc; }
                r
            }
            _ => unreachable!(),
        };

        if write_rd {
            if rd == 15 {
                self.branch_exchange(res);
            } else {
                self.r[rd] = res;
            }
        }
        Ok(())
    }

    fn step_thumb(&mut self, pc: u32) -> Result<(), String> {
        let op = self.r16(pc) as u32;
        self.r[15] = pc.wrapping_add(2);
        let top3 = op >> 13;

        match top3 {
            0b000 => {
                let sub = (op >> 11) & 3;
                let rd = (op & 7) as usize;
                let rs = ((op >> 3) & 7) as usize;
                if sub != 3 {
                    // Format 1: LSL / LSR / ASR #imm5
                    let imm = (op >> 6) & 0x1F;
                    let (res, cout) = self.shift_imm(self.r[rs], sub, imm, self.c);
                    self.r[rd] = res;
                    self.set_nz(res);
                    self.c = cout;
                } else {
                    // Format 2: ADD / SUB reg or imm3
                    let rn_imm = (op >> 6) & 7;
                    let rhs = if (op >> 10) & 1 != 0 { rn_imm } else { self.r[rn_imm as usize] };
                    self.r[rd] = if (op >> 9) & 1 != 0 {
                        self.sub_flags(self.r[rs], rhs)
                    } else {
                        self.add_flags(self.r[rs], rhs)
                    };
                }
            }
            0b001 => {
                // Format 3: MOV / CMP / ADD / SUB #imm8
                let sub = (op >> 11) & 3;
                let rd = ((op >> 8) & 7) as usize;
                let imm = op & 0xFF;
                match sub {
                    0 => {
                        self.r[rd] = imm;
                        self.set_nz(imm);
                    }
                    1 => {
                        self.sub_flags(self.r[rd], imm);
                    }
                    2 => {
                        self.r[rd] = self.add_flags(self.r[rd], imm);
                    }
                    _ => {
                        self.r[rd] = self.sub_flags(self.r[rd], imm);
                    }
                }
            }
            0b010 => {
                let mid = (op >> 10) & 7;
                match mid {
                    0 => {
                        // Format 4: ALU
                        let alu = (op >> 6) & 0xF;
                        let rs = ((op >> 3) & 7) as usize;
                        let rd = (op & 7) as usize;
                        let a = self.r[rd];
                        let b = self.r[rs];
                        match alu {
                            0x0 => { let r = a & b; self.r[rd] = r; self.set_nz(r); }
                            0x1 => { let r = a ^ b; self.r[rd] = r; self.set_nz(r); }
                            0x2 => { let (r, c) = self.shift_reg(a, 0, b, self.c); self.r[rd] = r; self.set_nz(r); self.c = c; }
                            0x3 => { let (r, c) = self.shift_reg(a, 1, b, self.c); self.r[rd] = r; self.set_nz(r); self.c = c; }
                            0x4 => { let (r, c) = self.shift_reg(a, 2, b, self.c); self.r[rd] = r; self.set_nz(r); self.c = c; }
                            0x5 => { self.r[rd] = self.adc_flags(a, b, self.c); }
                            0x6 => { self.r[rd] = self.sbc_flags(a, b, self.c); }
                            0x7 => { let (r, c) = self.shift_reg(a, 3, b, self.c); self.r[rd] = r; self.set_nz(r); self.c = c; }
                            0x8 => { self.set_nz(a & b); }
                            0x9 => { self.r[rd] = self.sub_flags(0, b); }
                            0xA => { self.sub_flags(a, b); }
                            0xB => { self.add_flags(a, b); }
                            0xC => { let r = a | b; self.r[rd] = r; self.set_nz(r); }
                            0xD => { let r = a.wrapping_mul(b); self.r[rd] = r; self.set_nz(r); }
                            0xE => { let r = a & !b; self.r[rd] = r; self.set_nz(r); }
                            _ => { let r = !b; self.r[rd] = r; self.set_nz(r); }
                        }
                    }
                    1 => {
                        // Format 5: Hi reg / BX / BLX
                        let hi_op = (op >> 8) & 3;
                        let rd = (((op >> 4) & 8) | (op & 7)) as usize;
                        let rm = ((op >> 3) & 0xF) as usize;
                        let pc4 = pc.wrapping_add(4);
                        let vm = if rm == 15 {
                            if hi_op == 3 { pc4 & !2 } else { pc4 }
                        } else {
                            self.r[rm]
                        };
                        match hi_op {
                            0 => {
                                let vd = if rd == 15 { pc4 } else { self.r[rd] };
                                let r = vd.wrapping_add(vm);
                                if rd == 15 {
                                    self.r[15] = r & !1;
                                } else {
                                    self.r[rd] = r;
                                }
                            }
                            1 => {
                                let vd = if rd == 15 { pc4 } else { self.r[rd] };
                                self.sub_flags(vd, vm);
                            }
                            2 => {
                                if rd == 15 {
                                    self.r[15] = vm & !1;
                                } else {
                                    self.r[rd] = vm;
                                }
                            }
                            _ => {
                                if (op >> 7) & 1 != 0 {
                                    self.r[14] = pc.wrapping_add(2) | 1;
                                }
                                self.branch_exchange(vm);
                            }
                        }
                    }
                    2 | 3 => {
                        // Format 6: LDR Rd, [PC, #imm8*4]
                        let rd = ((op >> 8) & 7) as usize;
                        let addr = (pc.wrapping_add(4) & !2).wrapping_add((op & 0xFF) << 2);
                        self.r[rd] = self.r32(addr);
                    }
                    _ => {
                        // Format 7 & 8: Load/store register offset
                        let ro = ((op >> 6) & 7) as usize;
                        let rb = ((op >> 3) & 7) as usize;
                        let rd = (op & 7) as usize;
                        let addr = self.r[rb].wrapping_add(self.r[ro]);
                        match (op >> 9) & 7 {
                            0b000 => self.w32(addr, self.r[rd]),
                            0b001 => self.w16(addr, self.r[rd] as u16),
                            0b010 => self.w8(addr, self.r[rd] as u8),
                            0b011 => self.r[rd] = (self.r8(addr) as i8) as i32 as u32,
                            0b100 => self.r[rd] = self.r32(addr),
                            0b101 => self.r[rd] = self.r16(addr) as u32,
                            0b110 => self.r[rd] = self.r8(addr) as u32,
                            _ => self.r[rd] = (self.r16(addr) as i16) as i32 as u32,
                        }
                    }
                }
            }
            0b011 => {
                // Format 9: LDR/STR/LDRB/STRB #imm5
                let imm = (op >> 6) & 0x1F;
                let rb = ((op >> 3) & 7) as usize;
                let rd = (op & 7) as usize;
                match (op >> 11) & 3 {
                    0 => self.w32(self.r[rb].wrapping_add(imm << 2), self.r[rd]),
                    1 => self.r[rd] = self.r32(self.r[rb].wrapping_add(imm << 2)),
                    2 => self.w8(self.r[rb].wrapping_add(imm), self.r[rd] as u8),
                    _ => self.r[rd] = self.r8(self.r[rb].wrapping_add(imm)) as u32,
                }
            }
            0b100 => {
                if (op >> 12) & 1 == 0 {
                    // Format 10: LDRH / STRH #imm5*2
                    let imm = ((op >> 6) & 0x1F) << 1;
                    let rb = ((op >> 3) & 7) as usize;
                    let rd = (op & 7) as usize;
                    let addr = self.r[rb].wrapping_add(imm);
                    if (op >> 11) & 1 != 0 {
                        self.r[rd] = self.r16(addr) as u32;
                    } else {
                        self.w16(addr, self.r[rd] as u16);
                    }
                } else {
                    // Format 11: SP-relative LDR / STR
                    let rd = ((op >> 8) & 7) as usize;
                    let addr = self.r[13].wrapping_add((op & 0xFF) << 2);
                    if (op >> 11) & 1 != 0 {
                        self.r[rd] = self.r32(addr);
                    } else {
                        self.w32(addr, self.r[rd]);
                    }
                }
            }
            0b101 => {
                if (op >> 12) & 1 == 0 {
                    // Format 12: ADD Rd, PC/SP, #imm8*4
                    let rd = ((op >> 8) & 7) as usize;
                    let base = if (op >> 11) & 1 != 0 { self.r[13] } else { pc.wrapping_add(4) & !2 };
                    self.r[rd] = base.wrapping_add((op & 0xFF) << 2);
                } else {
                    // Format 13 / 14: ADD SP, #imm7 | PUSH / POP
                    let mid = (op >> 8) & 0xF;
                    if mid == 0 {
                        let off = (op & 0x7F) << 2;
                        self.r[13] = if (op >> 7) & 1 != 0 {
                            self.r[13].wrapping_sub(off)
                        } else {
                            self.r[13].wrapping_add(off)
                        };
                    } else if (mid & 0b0110) == 0b0100 {
                        let list = op & 0xFF;
                        let rbit = (op >> 8) & 1 != 0;
                        if (op >> 11) & 1 == 0 {
                            // PUSH
                            let count = list.count_ones() + (rbit as u32);
                            let new_sp = self.r[13].wrapping_sub(count * 4);
                            let mut a = new_sp;
                            for reg in 0..8usize {
                                if (list >> reg) & 1 != 0 {
                                    self.w32(a, self.r[reg]);
                                    a = a.wrapping_add(4);
                                }
                            }
                            if rbit {
                                self.w32(a, self.r[14]);
                            }
                            self.r[13] = new_sp;
                        } else {
                            // POP
                            let mut a = self.r[13];
                            for reg in 0..8usize {
                                if (list >> reg) & 1 != 0 {
                                    self.r[reg] = self.r32(a);
                                    a = a.wrapping_add(4);
                                }
                            }
                            if rbit {
                                let tgt = self.r32(a);
                                a = a.wrapping_add(4);
                                self.r[13] = a;
                                self.branch_exchange(tgt);
                            } else {
                                self.r[13] = a;
                            }
                        }
                    } else {
                        return Err(format!("Thumb undefined @ {:#010x}: {:#06x}", pc, op));
                    }
                }
            }
            0b110 => {
                if (op >> 12) & 1 == 0 {
                    // Format 15: STMIA / LDMIA Rb!, {rlist}
                    let rb = ((op >> 8) & 7) as usize;
                    let list = op & 0xFF;
                    let mut a = self.r[rb];
                    if (op >> 11) & 1 == 0 {
                        for reg in 0..8usize {
                            if (list >> reg) & 1 != 0 {
                                self.w32(a, self.r[reg]);
                                a = a.wrapping_add(4);
                            }
                        }
                        self.r[rb] = a;
                    } else {
                        for reg in 0..8usize {
                            if (list >> reg) & 1 != 0 {
                                self.r[reg] = self.r32(a);
                                a = a.wrapping_add(4);
                            }
                        }
                        if (list >> rb) & 1 == 0 {
                            self.r[rb] = a;
                        }
                    }
                } else {
                    // Format 16 / 17: B<cond> or SVC
                    let cond = (op >> 8) & 0xF;
                    if cond == 0xF {
                        // SVC #imm8: uITRON critical section / syscall -> return E_OK (0)
                        self.r[0] = 0;
                    } else if cond == 0xE {
                        return Err(format!("Thumb UDF @ {:#010x}: {:#06x}", pc, op));
                    } else if self.cond_ok(cond) {
                        let simm = ((op as i8) as i32) << 1;
                        self.r[15] = pc.wrapping_add(4).wrapping_add(simm as u32) & !1;
                    }
                }
            }
            0b111 => {
                let sub = (op >> 11) & 3;
                match sub {
                    0 => {
                        // Format 18: B label
                        let simm = ((((op & 0x7FF) as i32) << 21) >> 20) as u32;
                        self.r[15] = pc.wrapping_add(4).wrapping_add(simm) & !1;
                    }
                    2 => {
                        // Format 19 prefix: LR = (pc + 4) + sign_ext11(imm11) << 12
                        let simm = ((((op & 0x7FF) as i32) << 21) >> 9) as u32;
                        self.r[14] = pc.wrapping_add(4).wrapping_add(simm);
                    }
                    3 => {
                        // Format 19 suffix: BL #tgt
                        let tgt = self.r[14].wrapping_add((op & 0x7FF) << 1) & !1;
                        self.r[14] = pc.wrapping_add(2) | 1;
                        self.r[15] = tgt;
                    }
                    1 => {
                        // ARMv5TE BLX #tgt (op bit 0 must be 0)
                        let tgt = self.r[14].wrapping_add((op & 0x7FF) << 1) & !3;
                        self.r[14] = pc.wrapping_add(2) | 1;
                        self.thumb = false;
                        self.r[15] = tgt;
                    }
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}

#[inline]
fn msr_mask(field: u32) -> u32 {
    let mut m = 0u32;
    if field & 8 != 0 {
        m |= 0xFF00_0000;
    }
    if field & 4 != 0 {
        m |= 0x00FF_0000;
    }
    if field & 2 != 0 {
        m |= 0x0000_FF00;
    }
    if field & 1 != 0 {
        m |= 0x0000_00FF;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arm_thumb_interwork_and_swar() {
        let mut cpu = Cpu::new();
        let arm_fn = 0xA005_0000u32;
        // ARM @ 0xA0050000:
        // subs r0, r0, r1       (0xE0500001)
        // rsbmi r0, r0, #0      (0x42600000)
        // bx lr                 (0xE12FFF1E)
        cpu.w32(arm_fn, 0xE050_0001);
        cpu.w32(arm_fn + 4, 0x4260_0000);
        cpu.w32(arm_fn + 8, 0xE12F_FF1E);

        let thm_fn = 0xA005_0100u32;
        // Thumb @ 0xA0050100:
        // push {lr}             (0xB500)
        // blx #0xA0050000       (off = 0xA0050000 - 0xA0050106 = -0x106 => F7FF EF7C)
        // adds r0, #10          (0x300A)
        // pop {pc}              (0xBD00)
        cpu.w16(thm_fn, 0xB500);
        cpu.w16(thm_fn + 2, 0xF7FF);
        cpu.w16(thm_fn + 4, 0xEF7C);
        cpu.w16(thm_fn + 6, 0x300A);
        cpu.w16(thm_fn + 8, 0xBD00);

        let (res, _) = cpu.call(thm_fn | 1, &[12, 50], &[], 0xA200_0000, 100).unwrap();
        assert_eq!(res, 48);
    }
}
