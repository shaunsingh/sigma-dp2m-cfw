//! ARM926EJ-S firmware harness with M6M hardware and uITRON hooks.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use crate::arm9::{Cpu, HookAction, RET_SENTINEL, SDRAM_BASE};
use crate::image::{BASE, Image};
use crate::stock;

pub const RAM_END: u32 = 0xA400_0000;
pub const STACK: (u32, u32) = (0xA1FF_0000, 0xA200_1000);
pub const SCRATCH: u32 = 0xA2B0_0000;

/// Addresses of logged writes not fully inside any allowed `[lo, hi)` range.
pub fn outside(writes: &[(u32, u32)], allowed: &[(u32, u32)]) -> Vec<u32> {
    let mut out = BTreeSet::new();
    for &(a, n) in writes {
        let end = a.saturating_add(n);
        if !allowed.iter().any(|&(lo, hi)| a >= lo && end <= hi) {
            out.insert(a);
        }
    }
    out.into_iter().collect()
}

pub struct Machine {
    pub cpu: Cpu,
    pub img: Image,
}

impl Machine {
    pub fn new(img: &Image, lang: u32) -> Self {
        let mut cpu = Cpu::new();
        cpu.load_s3(img.s3());

        // uITRON bump allocator & out-pointer syscalls (never leave *p_blk uninitialized)
        let heap = Rc::new(Cell::new(0xA0A2_0000u32));
        let bump = move |c: &mut Cpu, n: u32| {
            let sz = n.max(16).next_multiple_of(16);
            let ptr = heap.get();
            heap.set(ptr.wrapping_add(sz));
            let off = (ptr - SDRAM_BASE) as usize;
            c.sdram_mut()[off..off + sz as usize].fill(0);
            ptr
        };

        cpu.stub_range(stock::UITRON.0, stock::UITRON.1, Some(0));
        for va in [0xA007_1830u32, 0xA007_1970, 0xA007_1C54] {
            let b = bump.clone();
            cpu.add_hook(va, move |c| {
                let p = c.r[1];
                if (BASE..RAM_END - 4).contains(&p) {
                    let blk = b(c, 256);
                    c.w32(p, blk);
                }
                HookAction::Return(Some(0))
            });
        }
        for va in [0xA007_1F30u32, 0xA007_212C, 0xA007_22A0] {
            let b = bump.clone();
            cpu.add_hook(va, move |c| {
                let (sz, p) = (c.r[1], c.r[2]);
                if (BASE..RAM_END - 4).contains(&p) {
                    let blk = b(c, sz);
                    c.w32(p, blk);
                }
                HookAction::Return(Some(0))
            });
        }
        cpu.add_hook(0xA007_2C74, |c| {
            let p = c.r[1];
            if (BASE..RAM_END - 8).contains(&p) {
                c.write_bytes(p, &[0; 8]);
            }
            HookAction::Return(Some(0))
        });
        cpu.add_hook(0xA007_3B70, |c| {
            let p = c.r[0];
            if (BASE..RAM_END - 2).contains(&p) {
                c.w16(p, 0);
            }
            HookAction::Return(Some(0))
        });
        cpu.add_hook(0xA007_52D4, |c| {
            let p = c.r[1];
            if (BASE..RAM_END - 0x24).contains(&p) {
                c.write_bytes(p, &[0; 0x24]);
                c.w16(p, 0x10); // TTS_DMT
            }
            HookAction::Return(Some(0))
        });

        cpu.stub(stock::GET_LANG, Some(lang));

        // Milbeaut 2D DMA engines
        cpu.add_hook(stock::DMA_2D_COPY_MODE, |c| {
            let (src, ss, ds) = (c.r[1], c.r[2], c.r[3]);
            let (dst, wb, rows) = (c.r32(c.r[13]), c.r32(c.r[13] + 4), c.r32(c.r[13] + 8));
            copy_2d(c, src, ss, dst, ds, wb, rows);
            HookAction::Return(Some(0))
        });
        cpu.add_hook(stock::DMA_2D_COPY, |c| {
            let (src, ss, ds, dst) = (c.r[0], c.r[1], c.r[2], c.r[3]);
            let (wb, rows) = (c.r32(c.r[13]), c.r32(c.r[13] + 4));
            copy_2d(c, src, ss, dst, ds, wb, rows);
            HookAction::Return(Some(0))
        });
        cpu.add_hook(stock::DMA_RECT_FILL, |c| {
            let (fill, stride, dst) = (c.r[1] as u8, c.r[2], c.r[3]);
            let (wb, rows) = (c.r32(c.r[13]), c.r32(c.r[13] + 4));
            if dst >= BASE && wb > 0 && rows > 0 && rows <= 2048 && stride <= 8192 {
                let mut row = None;
                for r in 0..rows {
                    let off = dst.wrapping_add(r.wrapping_mul(stride));
                    if off.saturating_add(wb) <= RAM_END {
                        if off >= BASE {
                            c.fill_bytes(off, fill, wb as usize);
                        } else {
                            c.write_bytes(off, row.get_or_insert_with(|| vec![fill; wb as usize]));
                        }
                    }
                }
            }
            HookAction::Return(Some(0))
        });
        cpu.add_hook(stock::DMA_MEM_COPY, |c| {
            let (dst, src, n) = (c.r[0], c.r[1], c.r[2]);
            if dst >= BASE && src >= BASE && n > 0 && dst.saturating_add(n) <= RAM_END && src.saturating_add(n) <= RAM_END {
                c.copy_within_bytes(dst, src, n as usize);
            }
            HookAction::Return(Some(0))
        });

        // Neutral font scale (1, 1); screens set their own
        cpu.w32(stock::FONT_SCALE, 1);
        cpu.w32(stock::FONT_SCALE + 4, 1);

        Self { cpu, img: img.clone() }
    }

    pub fn load_nv_defaults(&mut self) {
        self.cpu.write_bytes(stock::NV_BLOCK, self.img.rd(stock::NV_DEFAULTS, stock::NV_LEN));
    }

    pub fn rd(&self, va: u32, n: usize) -> Vec<u8> {
        let mut buf = vec![0u8; n];
        self.cpu.read_bytes(va, &mut buf);
        buf
    }

    pub fn u8(&self, va: u32) -> u8 { self.cpu.r8(va) }
    pub fn u16(&self, va: u32) -> u16 { self.cpu.r16_be(va) }
    pub fn u32(&self, va: u32) -> u32 { self.cpu.r32_be(va) }
    pub fn w8(&mut self, va: u32, v: u8) { self.cpu.w8(va, v); }
    pub fn w16(&mut self, va: u32, v: u16) { self.cpu.w16(va, v); }
    pub fn w32(&mut self, va: u32, v: u32) { self.cpu.w32(va, v); }
    pub fn wr(&mut self, va: u32, data: &[u8]) { self.cpu.write_bytes(va, data); }

    pub fn cstr(&self, va: u32, max: usize) -> String {
        let buf = self.rd(va, max);
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        String::from_utf8_lossy(&buf[..end]).into_owned()
    }

    pub fn call(&mut self, va: u32, args: &[u32]) -> Result<u32, String> {
        self.call_ext(va, args, &[], 50_000_000)
    }

    pub fn call_ext(&mut self, va: u32, args: &[u32], stack: &[u32], count: u64) -> Result<u32, String> {
        let (r0, _) = self.cpu.call(va | 1, args, stack, STACK.1, count)?;
        Ok(r0)
    }

    pub fn run_slice(&mut self, start: u32, until: u32, count: u64, sp: u32, regs: &[(usize, u32)]) -> Result<(), String> {
        self.cpu.r[13] = sp;
        self.cpu.r[14] = RET_SENTINEL | 1;
        for &(i, v) in regs {
            self.cpu.r[i] = v;
        }
        self.cpu.branch_exchange(start | 1);
        self.cpu.run(until, count)?;
        Ok(())
    }

    pub fn stub(&mut self, va: u32, ret: Option<u32>) {
        self.cpu.stub(va, ret);
    }

    pub fn stub_on(&mut self, va: u32, mut f: impl FnMut(&mut Cpu) -> Option<u32> + 'static) {
        self.cpu.add_hook(va, move |c| HookAction::Return(f(c)));
    }

    pub fn watch(&mut self, va: u32, mut f: impl FnMut(&mut Cpu) + 'static) {
        self.cpu.add_hook(va, move |c| {
            f(c);
            HookAction::Continue
        });
    }

    pub fn ui_string(&mut self, sid: u32, lang: u32) -> Result<Option<String>, String> {
        let p = self.call_ext(stock::UI_STRING, &[sid, lang], &[], 100_000)?;
        Ok((BASE..0xB000_0000).contains(&p).then(|| self.cstr(p, 256)))
    }

    pub fn menu_table(&mut self, mid: u32) -> Result<u32, String> {
        self.call_ext(stock::GET_MENU_TABLE, &[mid], &[], 100_000)
    }

    /// Renders `s` via the stock BP4 font engine (`rect.y >= 480` is offscreen).
    pub fn text(&mut self, s: &str, px: u32, style: u32, rect: [u16; 4], lang: u32) -> Result<Vec<u8>, String> {
        let mut rbuf = [0u8; 8];
        for (i, &v) in rect.iter().enumerate() {
            rbuf[i * 2..i * 2 + 2].copy_from_slice(&v.to_be_bytes());
        }
        self.wr(SCRATCH, &rbuf);
        let mut sb: Vec<u8> = s.bytes().collect();
        sb.push(0);
        self.wr(SCRATCH + 8, &sb);
        self.call_ext(
            stock::TEXT_DRAW,
            &[lang, SCRATCH + 8, u32::MAX, px],
            &[5, 0, 1, SCRATCH, style, 0],
            50_000_000,
        )?;
        Ok(self.rd(stock::LINE_BUF, stock::LINE_W * stock::LINE_ROWS))
    }
}

fn copy_2d(c: &mut Cpu, src: u32, ss: u32, dst: u32, ds: u32, wb: u32, rows: u32) {
    if src < BASE || dst < BASE || wb == 0 || rows == 0 || rows > 2048 {
        return;
    }
    let mut row = None;
    for r in 0..rows {
        let s = src.wrapping_add(r.wrapping_mul(ss));
        let d = dst.wrapping_add(r.wrapping_mul(ds));
        if s.saturating_add(wb) <= RAM_END && d.saturating_add(wb) <= RAM_END {
            if s >= BASE && d >= BASE {
                c.copy_within_bytes(d, s, wb as usize);
            } else {
                let row = row.get_or_insert_with(|| vec![0; wb as usize]);
                c.read_bytes(s, row);
                c.write_bytes(d, row);
            }
        }
    }
}

pub type NorWriteLog = Rc<RefCell<Vec<(u32, u32, Vec<u8>)>>>;

/// Virtual NOR bank 0 + malloc stub matching stock `NV_LOAD` / `NV_FLUSH`.
pub struct Nor {
    pub data: Rc<RefCell<Vec<u8>>>,
    pub writes: NorWriteLog,
}

impl Nor {
    pub const BUF: u32 = 0xA1E0_0000;

    pub fn install(m: &mut Machine) -> Self {
        let data = Rc::new(RefCell::new(vec![0u8; 0x800]));
        let writes = Rc::new(RefCell::new(Vec::new()));
        m.stub(stock::MALLOC, Some(Self::BUF));
        m.stub(stock::FREE, None);
        for &(va, ret) in &[
            (0xA001_B020u32, None),
            (0xA001_B026, Some(0)),
            (0xA014_3336, None),
            (0xA014_5650, Some(0)),
        ] {
            m.stub(va, ret);
        }
        let dr = data.clone();
        m.stub_on(stock::NOR_READ, move |c| {
            let (off, n, buf) = (c.r[0] as usize, c.r[1] as usize, c.r[2]);
            let d = dr.borrow();
            if off + n <= d.len() {
                c.write_bytes(buf, &d[off..off + n]);
            }
            None
        });
        let wr = writes.clone();
        m.stub_on(stock::NOR_WRITE, move |c| {
            let (off, n, buf) = (c.r[0], c.r[1], c.r[2]);
            let mut tmp = vec![0u8; n as usize];
            c.read_bytes(buf, &mut tmp);
            wr.borrow_mut().push((off, n, tmp));
            None
        });
        Self { data, writes }
    }
}
