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
    pub sdram: Box<[u8; SDRAM_SIZE]>,
    pub total_steps: u64,
    pub last_error: Option<String>,
    pub watch_hits: Vec<(u32, u32, u32)>,
    watch_range: Option<(u32, u32)>,
    write_log_active: bool,
    write_allow: Vec<(u32, u32)>,
    write_log: Vec<(u32, u32)>,
    mmio: Vec<(u32, Box<[u8; 4096]>)>,
    hook_bits: Vec<u64>,
    has_high_hooks: bool,
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
            sdram: vec![0u8; SDRAM_SIZE].into_boxed_slice().try_into().unwrap(),
            total_steps: 0,
            last_error: None,
            watch_hits: Vec::new(),
            watch_range: None,
            write_log_active: false,
            write_allow: Vec::new(),
            write_log: Vec::new(),
            mmio: Vec::new(),
            hook_bits: vec![0; (CODE_BITMAP_SPAN / 2) / 64],
            has_high_hooks: false,
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
        &self.sdram[..]
    }

    #[inline(always)]
    pub fn sdram_mut(&mut self) -> &mut [u8] {
        &mut self.sdram[..]
    }

    pub fn fill_bytes(&mut self, dst: u32, val: u8, len: usize) {
        if dst < SDRAM_BASE || len == 0 {
            return;
        }
        let d = (dst - SDRAM_BASE) as usize;
        if let Some(end) = d.checked_add(len)
            && end <= self.sdram.len()
        {
            self.check_watch(dst, len.min(u32::MAX as usize) as u32);
            self.sdram[d..end].fill(val);
        }
    }

    pub fn copy_within_bytes(&mut self, dst: u32, src: u32, len: usize) {
        if dst < SDRAM_BASE || src < SDRAM_BASE || len == 0 {
            return;
        }
        let d = (dst - SDRAM_BASE) as usize;
        let s = (src - SDRAM_BASE) as usize;
        if let (Some(d_end), Some(s_end)) = (d.checked_add(len), s.checked_add(len))
            && d_end <= self.sdram.len()
            && s_end <= self.sdram.len()
        {
            self.check_watch(dst, len.min(u32::MAX as usize) as u32);
            self.sdram.copy_within(s..s_end, d);
        }
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
        let off = (va.wrapping_sub(SDRAM_BASE) & !1) as usize;
        if off < SDRAM_SIZE {
            u16::from_be_bytes(self.sdram.as_chunks::<2>().0[off >> 1])
        } else {
            u16::from_be_bytes(self.mmio_read(va & !1))
        }
    }

    #[inline(always)]
    pub fn r16_be(&self, va: u32) -> u16 {
        let off = va.wrapping_sub(SDRAM_BASE) as usize;
        if let Some(bytes) = self.sdram.get(off..).and_then(<[_]>::first_chunk::<2>) {
            return u16::from_be_bytes(*bytes);
        }
        u16::from_be_bytes([self.r8(va), self.r8(va.wrapping_add(1))])
    }

    #[inline(always)]
    pub fn r32(&self, va: u32) -> u32 {
        let off = (va.wrapping_sub(SDRAM_BASE) & !3) as usize;
        if off < SDRAM_SIZE {
            u32::from_be_bytes(self.sdram.as_chunks::<4>().0[off >> 2])
        } else {
            u32::from_be_bytes(self.mmio_read(va & !3))
        }
    }

    #[inline(always)]
    pub fn r32_be(&self, va: u32) -> u32 {
        let off = va.wrapping_sub(SDRAM_BASE) as usize;
        if let Some(bytes) = self.sdram.get(off..).and_then(<[_]>::first_chunk::<4>) {
            return u32::from_be_bytes(*bytes);
        }
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

    pub fn start_write_log(&mut self, allowed: &[(u32, u32)]) {
        self.write_allow.clear();
        self.write_allow.extend_from_slice(allowed);
        self.write_log.clear();
        self.write_log_active = true;
    }

    pub fn clear_write_log(&mut self) {
        self.write_log.clear();
    }

    pub fn write_log(&self) -> &[(u32, u32)] {
        &self.write_log
    }

    pub fn stop_write_log(&mut self) -> Vec<(u32, u32)> {
        self.write_log_active = false;
        self.write_allow.clear();
        std::mem::take(&mut self.write_log)
    }

    #[cold]
    fn check_watch_slow(&mut self, va: u32, len: u32) {
        let end = va.saturating_add(len);
        if let Some((lo, hi)) = self.watch_range
            && va < hi
            && end > lo
            && self.watch_hits.len() < 64
        {
            self.watch_hits.push((self.r[15], self.r[14], va));
        }
        if self.write_log_active
            && !self.write_allow.iter().any(|&(lo, hi)| va >= lo && end <= hi)
        {
            self.write_log.push((va, len));
        }
    }

    #[inline(always)]
    fn check_watch(&mut self, va: u32, len: u32) {
        if self.watch_range.is_none() && !self.write_log_active {
            return;
        }
        self.check_watch_slow(va, len);
    }

    #[inline(always)]
    fn w8_inner<const WATCH: bool>(&mut self, va: u32, val: u8) {
        if WATCH {
            self.check_watch_slow(va, 1);
        }
        let off = va.wrapping_sub(SDRAM_BASE) as usize;
        if off < SDRAM_SIZE {
            self.sdram[off] = val;
        } else {
            self.mmio_w8(va, val);
        }
    }

    #[inline(always)]
    fn w16_inner<const WATCH: bool>(&mut self, va: u32, val: u16) {
        let va = va & !1;
        if WATCH {
            self.check_watch_slow(va, 2);
        }
        let off = (va.wrapping_sub(SDRAM_BASE) & !1) as usize;
        if off < SDRAM_SIZE {
            self.sdram.as_chunks_mut::<2>().0[off >> 1] = val.to_be_bytes();
        } else {
            self.mmio_write(va, val.to_be_bytes());
        }
    }

    #[inline(always)]
    fn w32_inner<const WATCH: bool>(&mut self, va: u32, val: u32) {
        let va = va & !3;
        if WATCH {
            self.check_watch_slow(va, 4);
        }
        let off = (va.wrapping_sub(SDRAM_BASE) & !3) as usize;
        if off < SDRAM_SIZE {
            self.sdram.as_chunks_mut::<4>().0[off >> 2] = val.to_be_bytes();
        } else {
            self.mmio_write(va, val.to_be_bytes());
        }
    }

    #[inline(always)]
    pub fn w8(&mut self, va: u32, val: u8) {
        if self.watch_range.is_none() && !self.write_log_active {
            self.w8_inner::<false>(va, val);
        } else {
            self.w8_inner::<true>(va, val);
        }
    }

    #[inline(always)]
    pub fn w16(&mut self, va: u32, val: u16) {
        if self.watch_range.is_none() && !self.write_log_active {
            self.w16_inner::<false>(va, val);
        } else {
            self.w16_inner::<true>(va, val);
        }
    }

    #[inline(always)]
    pub fn w32(&mut self, va: u32, val: u32) {
        if self.watch_range.is_none() && !self.write_log_active {
            self.w32_inner::<false>(va, val);
        } else {
            self.w32_inner::<true>(va, val);
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
            self.check_watch(va, src.len() as u32);
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
    #[inline(never)]
    fn mmio_read<const N: usize>(&self, va: u32) -> [u8; N] {
        std::array::from_fn(|i| self.mmio_r8(va.wrapping_add(i as u32)))
    }

    #[cold]
    #[inline(never)]
    fn mmio_write<const N: usize>(&mut self, va: u32, bytes: [u8; N]) {
        for (i, b) in bytes.into_iter().enumerate() {
            self.mmio_w8(va.wrapping_add(i as u32), b);
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
        } else if off < SDRAM_SIZE {
            self.has_high_hooks = true;
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
        if self.watch_range.is_none() && !self.write_log_active {
            self.run_inner::<false>(until, max_steps)
        } else {
            self.run_inner::<true>(until, max_steps)
        }
    }

    #[cold]
    #[inline(never)]
    fn fault<T>(&mut self, msg: String) -> Result<T, ()> {
        self.last_error = Some(msg);
        Err(())
    }

    #[inline(never)]
    fn run_inner<const WATCH: bool>(&mut self, until: u32, max_steps: u64) -> Result<u64, String> {
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
                return Err(self.wild_pc(pc));
            }
            // The precise bitmap tells us when the entire 128-byte line is
            // hook-free. Only trace lines that don't contain the exit PC.
            let word = if off < CODE_BITMAP_SPAN {
                self.hook_bits[off >> 7]
            } else if self.has_high_hooks {
                u64::MAX
            } else {
                0
            };
            let line = pc & !0x7F;
            if word == 0 && line != (until & !0x7F) {
                if self.thumb {
                    loop {
                        let i = (pc as usize & (SDRAM_SIZE - 1)) >> 1;
                        let op = u16::from_be_bytes(self.sdram.as_chunks::<2>().0[i]) as u32;
                        let Ok(still_thumb) = self.step_thumb_op::<WATCH>(pc, op) else {
                            self.total_steps = self.total_steps.wrapping_add(step);
                            return Err(self.last_error.clone().unwrap_or_default());
                        };
                        step += 1;
                        pc = self.r[15];
                        if !still_thumb || (pc & !0x7F) != line || step >= max_steps {
                            break;
                        }
                    }
                } else {
                    loop {
                        let i = (pc as usize & (SDRAM_SIZE - 1)) >> 2;
                        let insn = u32::from_be_bytes(self.sdram.as_chunks::<4>().0[i]);
                        if self.step_arm_op::<WATCH>(pc, insn).is_err() {
                            self.total_steps = self.total_steps.wrapping_add(step);
                            return Err(self.last_error.clone().unwrap_or_default());
                        }
                        step += 1;
                        pc = self.r[15];
                        if self.thumb || (pc & !0x7F) != line || step >= max_steps {
                            break;
                        }
                    }
                }
                continue;
            }
            if (word >> ((off >> 1) & 63)) & 1 != 0 && self.fire_hook(pc) {
                step += 1;
                continue;
            }
            let res = if self.thumb {
                let op = self.r16(pc) as u32;
                self.step_thumb_op::<WATCH>(pc, op).map(|_| ())
            } else {
                let insn = self.r32(pc);
                self.step_arm_op::<WATCH>(pc, insn)
            };
            if res.is_err() {
                self.total_steps = self.total_steps.wrapping_add(step);
                return Err(self.last_error.clone().unwrap_or_default());
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
    #[inline(never)]
    fn wild_pc(&mut self, pc: u32) -> String {
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
        err
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

    #[inline(always)]
    fn step_arm_op<const WATCH: bool>(&mut self, pc: u32, insn: u32) -> Result<(), ()> {
        let cond = insn >> 28;
        if cond != 0xE {
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
                    let mut m = list;
                    while m != 0 {
                        let reg = m.trailing_zeros() as usize;
                        m &= m - 1;
                        let val = self.r32(addr);
                        addr = addr.wrapping_add(4);
                        if reg == 15 {
                            self.branch_exchange(val);
                        } else {
                            self.r[reg] = val;
                        }
                    }
                } else {
                    let mut m = list;
                    while m != 0 {
                        let reg = m.trailing_zeros() as usize;
                        m &= m - 1;
                        let val = if reg == 15 { pc8 } else { self.r[reg] };
                        self.w32_inner::<WATCH>(addr, val);
                        addr = addr.wrapping_add(4);
                    }
                    if w {
                        self.r[rn] = wb;
                    }
                }
            }
            0b010 | 0b011 => {
                // Single Data Transfer: LDR / STR / LDRB / STRB
                if top3 == 0b011 && (insn & 0x10) != 0 {
                    return self.fault(format!("ARM undefined @ {:#010x}: {:#010x}", pc, insn));
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
                    let rm = self.arm_reg((insn & 0xF) as usize, pc8);
                    if (insn & 0xFF0) == 0 {
                        rm
                    } else {
                        self.shift_imm(rm, (insn >> 5) & 3, (insn >> 7) & 0x1F, self.c).0
                    }
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
                        self.w8_inner::<WATCH>(addr, val as u8);
                    } else {
                        self.w32_inner::<WATCH>(addr, val);
                    }
                    if !p || w {
                        self.r[rn] = ea;
                    }
                }
            }
            0b000 | 0b001 => self.step_arm_dp_misc::<WATCH>(pc, pc8, insn)?,
            0b110 | 0b111 => {
                // SVC -> E_OK (0) in r0; Coprocessor MRC p15 -> 0, MCR/LDC/STC -> NOP
                if (insn & 0x0F00_0000) == 0x0F00_0000 {
                    self.r[0] = 0;
                } else if (insn & 0x0F10_0010) == 0x0E10_0010 {
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

    #[inline(always)]
    fn step_arm_dp_misc<const WATCH: bool>(&mut self, pc: u32, pc8: u32, insn: u32) -> Result<(), ()> {
        let is_imm = (insn >> 25) & 1 != 0;

        // Multiply / Extra Load-Store (when I=0, bit7=1, bit4=1)
        if !is_imm && (insn & 0x90) == 0x90 {
            return self.step_arm_extra_mem_mul::<WATCH>(pc, pc8, insn);
        }

        let opcode = (insn >> 21) & 0xF;
        let s = (insn >> 20) & 1 != 0;

        // Miscellaneous space: !S && opcode in 8..=11
        if !s && (8..=11).contains(&opcode) {
            return self.step_arm_misc(pc, pc8, insn, is_imm, opcode);
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
            if (insn & 0xFF0) == 0 {
                (rm, self.c)
            } else {
                let stype = (insn >> 5) & 3;
                if reg_shift {
                    let rs = self.r[((insn >> 8) & 0xF) as usize];
                    self.shift_reg(rm, stype, rs, self.c)
                } else {
                    self.shift_imm(rm, stype, (insn >> 7) & 0x1F, self.c)
                }
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
            0x2 => if logic_s { self.sub_flags(rn, op2) } else { rn.wrapping_sub(op2) },
            0x3 => if logic_s { self.sub_flags(op2, rn) } else { op2.wrapping_sub(rn) },
            0x4 => if logic_s { self.add_flags(rn, op2) } else { rn.wrapping_add(op2) },
            0x5 => if logic_s { self.adc_flags(rn, op2, cin) } else { rn.wrapping_add(op2).wrapping_add(cin as u32) },
            0x6 => if logic_s { self.sbc_flags(rn, op2, cin) } else { rn.wrapping_add(!op2).wrapping_add(cin as u32) },
            0x7 => if logic_s { self.sbc_flags(op2, rn, cin) } else { op2.wrapping_add(!rn).wrapping_add(cin as u32) },
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

    #[inline(never)]
    fn step_arm_extra_mem_mul<const WATCH: bool>(&mut self, pc: u32, pc8: u32, insn: u32) -> Result<(), ()> {
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
                    self.w8_inner::<WATCH>(addr, src as u8);
                    self.r[rd] = old;
                } else {
                    let old = self.r32(addr);
                    self.w32_inner::<WATCH>(addr, src);
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
                _ => return self.fault(format!("ARM unknown mul @ {:#010x}: {:#010x}", pc, insn)),
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
                self.w16_inner::<WATCH>(addr, self.arm_reg(rd, pc8) as u16);
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
                self.w32_inner::<WATCH>(addr, self.r[rd]);
                self.w32_inner::<WATCH>(addr.wrapping_add(4), self.r[rd + 1]);
                if !p || w {
                    self.r[rn] = ea;
                }
            }
            _ => return self.fault(format!("ARM extra mem @ {:#010x}: {:#010x}", pc, insn)),
        }
        Ok(())
    }

    #[inline(never)]
    fn step_arm_misc(&mut self, pc: u32, pc8: u32, insn: u32, is_imm: bool, opcode: u32) -> Result<(), ()> {
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
                        let rd = ((insn >> 12) & 0xF) as usize;
                        self.r[rd] = self.cpsr();
                    } else {
                        let mask = msr_mask((insn >> 16) & 0xF);
                        let val = self.r[(insn & 0xF) as usize];
                        self.set_cpsr(val, mask);
                    }
                    return Ok(());
                }
                1 => {
                    if opcode == 9 {
                        let tgt = self.arm_reg((insn & 0xF) as usize, pc8);
                        self.branch_exchange(tgt);
                    } else if opcode == 11 {
                        let rd = ((insn >> 12) & 0xF) as usize;
                        let rm = (insn & 0xF) as usize;
                        self.r[rd] = self.r[rm].leading_zeros();
                    }
                    return Ok(());
                }
                3 => {
                    let tgt = self.arm_reg((insn & 0xF) as usize, pc8);
                    self.r[14] = pc.wrapping_add(4);
                    self.branch_exchange(tgt);
                    return Ok(());
                }
                5 => {
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
                _ => return self.fault(format!("ARM misc @ {:#010x}: {:#010x}", pc, insn)),
            }
        } else if opcode & 1 == 1 {
            let rot = ((insn >> 8) & 0xF) * 2;
            let imm = (insn & 0xFF).rotate_right(rot);
            let mask = msr_mask((insn >> 16) & 0xF);
            self.set_cpsr(imm, mask);
            return Ok(());
        }
        Ok(())
    }

    // On success, indicate whether the instruction left the CPU in Thumb mode.
    #[inline(always)]
    fn step_thumb_op<const WATCH: bool>(&mut self, pc: u32, op: u32) -> Result<bool, ()> {
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
                                return Ok(self.thumb);
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
                            0b000 => self.w32_inner::<WATCH>(addr, self.r[rd]),
                            0b001 => self.w16_inner::<WATCH>(addr, self.r[rd] as u16),
                            0b010 => self.w8_inner::<WATCH>(addr, self.r[rd] as u8),
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
                    0 => self.w32_inner::<WATCH>(self.r[rb].wrapping_add(imm << 2), self.r[rd]),
                    1 => self.r[rd] = self.r32(self.r[rb].wrapping_add(imm << 2)),
                    2 => self.w8_inner::<WATCH>(self.r[rb].wrapping_add(imm), self.r[rd] as u8),
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
                        self.w16_inner::<WATCH>(addr, self.r[rd] as u16);
                    }
                } else {
                    // Format 11: SP-relative LDR / STR
                    let rd = ((op >> 8) & 7) as usize;
                    let addr = self.r[13].wrapping_add((op & 0xFF) << 2);
                    if (op >> 11) & 1 != 0 {
                        self.r[rd] = self.r32(addr);
                    } else {
                        self.w32_inner::<WATCH>(addr, self.r[rd]);
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
                            let mut m = list;
                            while m != 0 {
                                let reg = m.trailing_zeros() as usize;
                                m &= m - 1;
                                self.w32_inner::<WATCH>(a, self.r[reg]);
                                a = a.wrapping_add(4);
                            }
                            if rbit {
                                self.w32_inner::<WATCH>(a, self.r[14]);
                            }
                            self.r[13] = new_sp;
                        } else {
                            // POP
                            let mut a = self.r[13];
                            let mut m = list;
                            while m != 0 {
                                let reg = m.trailing_zeros() as usize;
                                m &= m - 1;
                                self.r[reg] = self.r32(a);
                                a = a.wrapping_add(4);
                            }
                            if rbit {
                                let tgt = self.r32(a);
                                a = a.wrapping_add(4);
                                self.r[13] = a;
                                self.branch_exchange(tgt);
                                return Ok(self.thumb);
                            } else {
                                self.r[13] = a;
                            }
                        }
                    } else {
                        return self.fault(format!("Thumb undefined @ {:#010x}: {:#06x}", pc, op));
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
                        let mut m = list;
                        while m != 0 {
                            let reg = m.trailing_zeros() as usize;
                            m &= m - 1;
                            self.w32_inner::<WATCH>(a, self.r[reg]);
                            a = a.wrapping_add(4);
                        }
                        self.r[rb] = a;
                    } else {
                        let mut m = list;
                        while m != 0 {
                            let reg = m.trailing_zeros() as usize;
                            m &= m - 1;
                            self.r[reg] = self.r32(a);
                            a = a.wrapping_add(4);
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
                        return self.fault(format!("Thumb UDF @ {:#010x}: {:#06x}", pc, op));
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
                        if op & 1 != 0 {
                            return self.fault(format!("Thumb undefined BLX @ {:#010x}: {:#06x}", pc, op));
                        }
                        let tgt = self.r[14].wrapping_add((op & 0x7FF) << 1) & !3;
                        self.r[14] = pc.wrapping_add(2) | 1;
                        self.thumb = false;
                        self.r[15] = tgt;
                        return Ok(false);
                    }
                    _ => unreachable!(),
                }
            }
            _ => unreachable!(),
        }
        Ok(true)
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
    fn step_counts_through_hooks_and_page_edges() {
        let mut cpu = Cpu::new();
        let a = SDRAM_BASE + 0x0FFE;
        for n in 0..4 {
            cpu.w16(a + n * 2, 0x3001); // adds r0, #1
        }
        cpu.add_hook(a + 2, move |cpu| {
            assert_eq!(cpu.r[0], 1);
            cpu.add_hook(a + 6, move |_| HookAction::Jump((a + 8) | 1));
            HookAction::Continue
        });
        cpu.branch_exchange(a | 1);
        assert_eq!(cpu.run(a + 8, 5), Ok(4));
        assert_eq!(cpu.r[0], 3); // The last instruction was handled by the hook.
        assert_eq!(cpu.total_steps, 4);

        // A limit reached on the last instruction is still a limit error,
        // even when that instruction's successor is the exit address.
        cpu.branch_exchange(a | 1);
        assert!(cpu.run(a + 2, 1).unwrap_err().contains("step limit"));
        assert_eq!(cpu.total_steps, 5);
        assert_eq!(cpu.r[15], a + 2);
    }

    #[test]
    fn aligned_and_unaligned_big_endian_at_sdram_edge() {
        let mut cpu = Cpu::new();
        let end = SDRAM_BASE + SDRAM_SIZE as u32;
        cpu.w32(end - 1, 0x1122_3344);
        cpu.w16(end + 1, 0x5566);
        assert_eq!(cpu.r32(end - 1), 0x1122_3344);
        assert_eq!(cpu.r16(end - 1), 0x3344);
        assert_eq!(cpu.r32_be(end - 2), 0x3344_5566);
        assert_eq!(cpu.r16_be(end - 1), 0x4455);
    }

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
