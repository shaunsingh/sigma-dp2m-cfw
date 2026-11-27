use std::cell::RefCell;
use std::rc::Rc;

use crate::arm9::{Cpu, HookAction, SDRAM_BASE};

use super::isp::{wb_q7_matrix, ycbcr_to_rgb};
use super::{
    ACCEL_X, ACCEL_Y, AE_SENSOR_EV100, B2Y_SET_CHROMA, B2Y_SET_GAMMA, B2Y_SET_MAT, B2Y_SET_MONO,
    B2Y_SET_SHARP_LUT, CALLOC, CaptureFrame, DCF_COUNT_IMAGES, DCF_DIR_COUNTS, DCF_GET_CUR,
    DCF_GET_ENTRY, DCF_GET_FILE_INFO, DCF_GET_TOTAL, DCF_READ_JPEG_EXIF, DCF_READ_X3F_EXIF,
    DCF_SCAN_READY, DCF_SET_CUR, DCF_STEP_NEXT, DCF_STEP_PREV, DCF_SYNC_CUR, FREE, GET_LANG,
    GET_LV_CC7, GET_LV_CCN, HeapBlock, HwState, LCD_H, LCD_W, LIBC_AEABI_MEMCLR,
    LIBC_AEABI_MEMCLR4, LIBC_AEABI_MEMMOVE, LIBC_AEABI_MEMSET, LIBC_MEMCPY, LIBC_MEMSET, LV_ACTIVE,
    LV_H, MALLOC, NOR_READ, NOR_WRITE, PB_DEC_INIT, PB_DECODE_RESAMPLE, PB_HW_JPEG_DEC,
    PB_IIP_HIST, PB_OPEN_IMAGE_A, PB_OPEN_IMAGE_B, PB_OPEN_JPEG_A, PB_OPEN_JPEG_B, PB_PLANE_C_LEN,
    PB_PLANE_Y_LEN, PB_SELECT_VBUF, PB_STEP_OR_CUR, PB_VBUF_DEFAULT_VA, PB_YCBCR_TO_RGB_PLANAR,
    REALLOC, SCRATCH_VA, SCREEN_POST_MBOX, SCREEN_POST_RET, SD_FREE_SPACE, SD_MOUNT_STATUS,
};

const DMA_2D_COPY_MODE: u32 = 0xA00B_125C;
const DMA_2D_COPY: u32 = 0xA00B_14A0;
const DMA_RECT_FILL: u32 = 0xA00B_1870;
const DMA_MEM_COPY: u32 = 0xA00B_23A8;

pub(crate) fn install_m6m_hooks(cpu: &mut Cpu, hw: &Rc<RefCell<HwState>>) {
    // uITRON kernel + Milbeaut driver range stub; stops below ICON_TBL getters at 0xA00BD7FC..0xA00C0594
    cpu.stub_range(0xA007_0000, 0xA00B_8000, Some(0));
    // uITRON 4.0 memory pool, task-ID, and mailbox/dataqueue out-pointer syscalls
    for &va in &[0xA007_1830, 0xA007_1970, 0xA007_1C54] {
        let st = Rc::clone(hw);
        cpu.add_hook(va, move |c| {
            let p_blk = c.r[1];
            let ptr = heap_alloc(&mut st.borrow_mut(), 256);
            if ptr == 0 {
                return HookAction::Return(Some((-33i32) as u32));
            }
            let off = (ptr - SDRAM_BASE) as usize;
            c.sdram_mut()[off..off + 256].fill(0);
            if p_blk >= SDRAM_BASE {
                c.w32(p_blk, ptr);
            }
            HookAction::Return(Some(0))
        });
    }
    for &va in &[0xA007_1F30, 0xA007_212C, 0xA007_22A0] {
        let st = Rc::clone(hw);
        cpu.add_hook(va, move |c| {
            let Some(sz) = c.r[1].checked_add(15).map(|n| (n & !15).max(16)) else {
                return HookAction::Return(Some((-33i32) as u32));
            };
            let p_blk = c.r[2];
            let ptr = heap_alloc(&mut st.borrow_mut(), sz);
            if ptr == 0 {
                return HookAction::Return(Some((-33i32) as u32));
            }
            c.fill_bytes(ptr, 0, sz as usize);
            if p_blk >= SDRAM_BASE {
                c.w32(p_blk, ptr);
            }
            HookAction::Return(Some(0))
        });
    }
    for &va in &[0xA007_1B00, 0xA007_2388] {
        let st = Rc::clone(hw);
        cpu.add_hook(va, move |c| {
            let ptr = c.r[1];
            if is_sim_heap_ptr(ptr) {
                let mut s = st.borrow_mut();
                if let Some(pos) = s.alloc_caps.iter().position(|&(va, _)| va == ptr) {
                    let (va, cap) = s.alloc_caps.swap_remove(pos);
                    s.free_list.push(HeapBlock { va, cap });
                }
            }
            HookAction::Return(Some(0))
        });
    }
    cpu.add_hook(0xA007_3B70, |c| {
        let p_tskid = c.r[0];
        if p_tskid >= SDRAM_BASE {
            c.w16(p_tskid, 0);
        }
        HookAction::Return(Some(0))
    });
    // uITRON ref_tsk(tskid, pk_rtsk): background slideshow/movie/audio tasks are TTS_DMT (0x10)
    cpu.add_hook(0xA007_52D4, |c| {
        let pk_rtsk = c.r[1];
        if pk_rtsk >= SDRAM_BASE {
            let off = (pk_rtsk - SDRAM_BASE) as usize;
            if off + 0x24 <= c.sdram().len() {
                c.sdram_mut()[off..off + 0x24].fill(0);
                c.w16(pk_rtsk, 0x10);
            }
        }
        HookAction::Return(Some(0))
    });
    // uITRON ref_flg (0xA0072BCC) and ref_mbx (0xA0072C74): zero 8-byte status packet at r1
    for &va in &[0xA007_2BCC, 0xA007_2C74] {
        cpu.add_hook(va, |c| {
            let pk_out = c.r[1];
            if pk_out >= SDRAM_BASE {
                c.w32(pk_out, 0);
                c.w32(pk_out.wrapping_add(4), 0);
            }
            HookAction::Return(Some(0))
        });
    }
    for &va in &[0xA007_29B8, 0xA007_3240, 0xA007_4808, 0xA007_4AC0, 0xA007_4C50] {
        cpu.stub(va, Some((-50i32) as u32));
    }
    cpu.stub(0xA010_EF48, Some(0));
    cpu.stub(0xA010_EF28, Some(0));
    cpu.stub(GET_LANG, Some(0));

    // ARM libc memory intrinsics (bit-exact with compiled ARM routines)
    for &(va, ret_r0) in &[(LIBC_MEMCPY, true), (LIBC_AEABI_MEMMOVE, false)] {
        cpu.add_hook(va, move |c| {
            let (dst, src, len) = (c.r[0], c.r[1], c.r[2] as usize);
            c.copy_within_bytes(dst, src, len);
            HookAction::Return(if ret_r0 { Some(dst) } else { None })
        });
    }
    cpu.add_hook(LIBC_MEMSET, |c| {
        let (dst, val, len) = (c.r[0], c.r[1] as u8, c.r[2] as usize);
        c.fill_bytes(dst, val, len);
        HookAction::Return(Some(dst))
    });
    cpu.add_hook(LIBC_AEABI_MEMSET, |c| {
        let (dst, len, val) = (c.r[0], c.r[1] as usize, c.r[2] as u8);
        c.fill_bytes(dst, val, len);
        HookAction::Return(None)
    });
    for &va in &[LIBC_AEABI_MEMCLR, LIBC_AEABI_MEMCLR4] {
        cpu.add_hook(va, |c| {
            let (dst, len) = (c.r[0], c.r[1] as usize);
            c.fill_bytes(dst, 0, len);
            HookAction::Return(None)
        });
    }

    // Milbeaut 2D DMA rectangular fill: (mode=r0, fill=r1, stride=r2, dst=r3, [sp]=w_bytes, [sp+4]=rows)
    cpu.add_hook(DMA_RECT_FILL, |c| {
        let fill = c.r[1] as u8;
        let stride = c.r[2];
        let dst = c.r[3];
        let sp = c.r[13];
        let w_bytes = c.r32_be(sp) as usize;
        let rows = c.r32_be(sp.wrapping_add(4));
        if dst >= SDRAM_BASE && w_bytes > 0 && rows > 0 && rows <= 4096 && stride <= 16384 {
            for r in 0..rows {
                if let Some(va) = dst.checked_add(r.saturating_mul(stride)) {
                    c.fill_bytes(va, fill, w_bytes);
                }
            }
        }
        HookAction::Return(Some(0))
    });

    // Milbeaut 2D DMA copy with mode arg: (mode=r0, src=r1, src_stride=r2, dst_stride=r3, [sp]=dst, [sp+4]=w_bytes, [sp+8]=rows)
    cpu.add_hook(DMA_2D_COPY_MODE, |c| {
        let src = c.r[1];
        let src_stride = c.r[2];
        let dst_stride = c.r[3];
        let sp = c.r[13];
        let dst = c.r32_be(sp);
        let w_bytes = c.r32_be(sp.wrapping_add(4)) as usize;
        let rows = c.r32_be(sp.wrapping_add(8));
        dma_copy_2d(c, src, src_stride, dst, dst_stride, w_bytes, rows);
        HookAction::Return(Some(0))
    });

    // Milbeaut 2D DMA block copy: (src=r0, src_stride=r1, dst_stride=r2, dst=r3, [sp]=w_bytes, [sp+4]=rows)
    cpu.add_hook(DMA_2D_COPY, |c| {
        let src = c.r[0];
        let src_stride = c.r[1];
        let dst_stride = c.r[2];
        let dst = c.r[3];
        let sp = c.r[13];
        let w_bytes = c.r32_be(sp) as usize;
        let rows = c.r32_be(sp.wrapping_add(4));
        dma_copy_2d(c, src, src_stride, dst, dst_stride, w_bytes, rows);
        HookAction::Return(Some(0))
    });

    // Milbeaut 1D DMA copy: (dst=r0, src=r1, len=r2)
    cpu.add_hook(DMA_MEM_COPY, |c| {
        let (dst, src, len) = (c.r[0], c.r[1], c.r[2] as usize);
        c.copy_within_bytes(dst, src, len);
        HookAction::Return(Some(0))
    });

    // NOR record counter & sync helpers
    cpu.stub(0xA001_B020, None);
    cpu.stub(0xA001_B026, Some(0));
    cpu.stub(0xA014_3336, None);
    cpu.stub(0xA014_5650, Some(0));

    {
        let st = Rc::clone(hw);
        cpu.add_hook(NOR_READ, move |c| {
            let off = c.r[0] as usize;
            let n = c.r[1] as usize;
            let buf = c.r[2];
            let s = st.borrow();
            if off < s.nor.len() {
                let end = (off + n).min(s.nor.len());
                c.write_bytes(buf, &s.nor[off..end]);
            }
            HookAction::Return(Some(0))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(NOR_WRITE, move |c| {
            let off = c.r[0] as usize;
            let n = c.r[1] as usize;
            let buf = c.r[2];
            let mut s = st.borrow_mut();
            if off < s.nor.len() {
                let end = (off + n).min(s.nor.len());
                c.read_bytes(buf, &mut s.nor[off..end]);
            }
            HookAction::Return(Some(0))
        });
    }

    // Recycling freelist + bump allocator in [0xA0A20000, 0xA1C00000) U [0xA3000000, 0xA7F00000)
    {
        let st = Rc::clone(hw);
        cpu.add_hook(MALLOC, move |c| {
            let Some(sz) = c.r[0].checked_add(15).map(|n| (n & !15).max(16)) else {
                return HookAction::Return(Some(0));
            };
            HookAction::Return(Some(heap_alloc(&mut st.borrow_mut(), sz)))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(CALLOC, move |c| {
            let Some(sz) = c.r[0]
                .checked_mul(c.r[1])
                .and_then(|t| t.checked_add(15))
                .map(|n| (n & !15).max(16))
            else {
                return HookAction::Return(Some(0));
            };
            let ptr = heap_alloc(&mut st.borrow_mut(), sz);
            if ptr != 0 {
                c.fill_bytes(ptr, 0, sz as usize);
            }
            HookAction::Return(Some(ptr))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(FREE, move |c| {
            let ptr = c.r[0];
            if is_sim_heap_ptr(ptr) {
                let mut s = st.borrow_mut();
                if let Some(pos) = s.alloc_caps.iter().position(|&(va, _)| va == ptr) {
                    let (va, cap) = s.alloc_caps.swap_remove(pos);
                    s.free_list.push(HeapBlock { va, cap });
                }
            }
            HookAction::Return(Some(0))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(REALLOC, move |c| {
            let ptr = c.r[0];
            let new_sz = c.r[1];
            if ptr == 0 {
                let Some(sz) = new_sz.checked_add(15).map(|n| (n & !15).max(16)) else {
                    return HookAction::Return(Some(0));
                };
                return HookAction::Return(Some(heap_alloc(&mut st.borrow_mut(), sz)));
            }
            if new_sz == 0 {
                let mut s = st.borrow_mut();
                if let Some(pos) = s.alloc_caps.iter().position(|&(va, _)| va == ptr) {
                    let (va, cap) = s.alloc_caps.swap_remove(pos);
                    s.free_list.push(HeapBlock { va, cap });
                }
                return HookAction::Return(Some(0));
            }
            let Some(aligned) = new_sz.checked_add(15).map(|n| (n & !15).max(16)) else {
                return HookAction::Return(Some(0));
            };
            let mut s = st.borrow_mut();
            if let Some(pos) = s.alloc_caps.iter().position(|&(va, _)| va == ptr) {
                let old_cap = s.alloc_caps[pos].1;
                if aligned <= old_cap {
                    return HookAction::Return(Some(ptr));
                }
                let new_ptr = heap_alloc(&mut s, aligned);
                if new_ptr != 0 {
                    let src_off = (ptr - SDRAM_BASE) as usize;
                    let dst_off = (new_ptr - SDRAM_BASE) as usize;
                    c.sdram_mut().copy_within(src_off..src_off + old_cap as usize, dst_off);
                    if let Some(p2) = s.alloc_caps.iter().position(|&(va, _)| va == ptr) {
                        let (va, cap) = s.alloc_caps.swap_remove(p2);
                        s.free_list.push(HeapBlock { va, cap });
                    }
                }
                return HookAction::Return(Some(new_ptr));
            }
            HookAction::Return(Some(heap_alloc(&mut s, aligned)))
        });
    }

    // Intercept SCREEN_POST mailbox send after stock deduplication/guards
    {
        let st = Rc::clone(hw);
        cpu.add_hook(SCREEN_POST_MBOX, move |c| {
            st.borrow_mut().events.push_back((c.r[6], c.r[5]));
            c.r[7] = 0;
            HookAction::Jump(SCREEN_POST_RET | 1)
        });
    }

    {
        let st = Rc::clone(hw);
        cpu.add_hook(AE_SENSOR_EV100, move |_| {
            HookAction::Return(Some(st.borrow().scene_ev100_q16 as u32))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(LV_ACTIVE, move |_| HookAction::Return(Some(st.borrow().lv_active as u32)));
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(ACCEL_X, move |_| HookAction::Return(Some((st.borrow().accel_x as u8) as u32)));
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(ACCEL_Y, move |_| HookAction::Return(Some((st.borrow().accel_y as u8) as u32)));
    }

    // Sensor AWB/Preset CCMatrix getters: write active Q7 3x3 WB matrix at buf + 10..28
    for &va in &[GET_LV_CC7, GET_LV_CCN] {
        cpu.add_hook(va, |c| {
            let dst = c.r[0];
            let wb = wb_q7_matrix(c);
            let mut buf = [0u8; 28];
            for (k, &v) in wb.iter().enumerate() {
                let raw = ((v as i16) as u16) & 0x0FFF;
                buf[10 + k * 2..12 + k * 2].copy_from_slice(&raw.to_be_bytes());
            }
            c.write_bytes(dst, &buf);
            HookAction::Return(None)
        });
    }

    // Capture live B2Y ISP registers programmed by M6M firmware
    {
        let st = Rc::clone(hw);
        cpu.add_hook(B2Y_SET_MAT, move |c| {
            let ptr = c.r[0];
            let mut m = [0i32; 9];
            for (i, slot) in m.iter_mut().enumerate() {
                let raw = c.r16_be(ptr + (i as u32) * 2);
                *slot = if raw & 0x800 != 0 {
                    ((raw | 0xF000) as i16) as i32
                } else {
                    (raw & 0x7FF) as i32
                };
            }
            st.borrow_mut().b2y_mat = m;
            HookAction::Return(Some(0))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(B2Y_SET_MONO, move |c| {
            let ptr = c.r[0];
            let mut s = st.borrow_mut();
            s.b2y_mono_en = c.r8(ptr + 1) != 0;
            s.b2y_mono_cb = c.r8(ptr + 6) as i8;
            s.b2y_mono_cr = c.r8(ptr + 7) as i8;
            HookAction::Return(Some(0))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(B2Y_SET_CHROMA, move |c| {
            let ptr = c.r[0];
            st.borrow_mut().b2y_chroma_gain = c.r8(ptr + 0x11);
            HookAction::Return(Some(0))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(B2Y_SET_SHARP_LUT, move |c| {
            st.borrow_mut().b2y_sharp_idx = (c.r[4] & 0xFF) as u8;
            HookAction::Return(Some(0))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(B2Y_SET_GAMMA, move |c| {
            let lut_ptr = c.r[1];
            let sz = c.r[2] as usize;
            if sz >= 1024 {
                c.read_bytes(lut_ptr, &mut st.borrow_mut().b2y_gamma);
            }
            HookAction::Return(Some(0))
        });
    }

    // Native SCREEN_PB (0x19) video buffer selector + virtual DCF SD card & JPEG/EXIF pipeline
    {
        let st = Rc::clone(hw);
        cpu.add_hook(PB_SELECT_VBUF, move |c| {
            let va = match c.r[0] {
                1 => 0xA235_E600,
                2 => 0xA243_F600,
                4 => 0xA252_0600,
                8 => 0xA260_1600,
                _ => PB_VBUF_DEFAULT_VA,
            };
            st.borrow_mut().pb_vbuf_va = va;
            HookAction::Return(Some(0))
        });
    }
    cpu.stub(PB_DEC_INIT, Some(1));
    cpu.stub(PB_HW_JPEG_DEC, Some(0));
    cpu.stub(SD_MOUNT_STATUS, Some(3));
    cpu.stub(DCF_SCAN_READY, Some(1));
    cpu.stub(DCF_SYNC_CUR, Some(0));
    cpu.add_hook(SD_FREE_SPACE, |c| {
        if c.r[0] >= SDRAM_BASE {
            c.w32(c.r[0], 999);
        }
        if c.r[1] >= SDRAM_BASE {
            c.w32(c.r[1], 999);
        }
        HookAction::Return(Some(0))
    });
    for &va in &[DCF_COUNT_IMAGES, DCF_GET_TOTAL] {
        let st = Rc::clone(hw);
        cpu.add_hook(va, move |c| {
            let n = st.borrow().captures.len() as u32;
            c.w32(DCF_DIR_COUNTS, n);
            HookAction::Return(Some(n))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(PB_STEP_OR_CUR, move |c| {
            let mode = c.r[0];
            let mut s = st.borrow_mut();
            if s.captures.is_empty() {
                return HookAction::Return(Some(0));
            }
            let len = s.captures.len();
            let cur = s.dcf_cur.min(len - 1);
            s.dcf_cur = match mode {
                1 => (cur + 1) % len,
                2 => if cur == 0 { len - 1 } else { cur - 1 },
                _ => cur,
            };
            HookAction::Return(Some((s.dcf_cur + 1) as u32))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(DCF_GET_FILE_INFO, move |c| {
            let idx = c.r[0] as usize;
            let p_dir = c.r[1];
            let p_fnum = c.r[2];
            let p_ftype = c.r[3];
            let sp = c.r[13];
            let p_a1 = c.r32_be(sp);
            let p_a2 = c.r32_be(sp.wrapping_add(4));
            let s = st.borrow();
            if idx < 1 || idx > s.captures.len() {
                return HookAction::Return(Some((-1i32) as u32));
            }
            if p_dir >= SDRAM_BASE {
                c.w32(p_dir, 100);
            }
            if p_fnum >= SDRAM_BASE {
                c.w32(p_fnum, idx as u32);
            }
            if p_ftype >= SDRAM_BASE {
                c.w16(p_ftype, 1); // JPEG
            }
            if p_a1 >= SDRAM_BASE {
                c.w32(p_a1, 0);
            }
            if p_a2 >= SDRAM_BASE {
                c.w32(p_a2, 0);
            }
            HookAction::Return(Some(0))
        });
    }
    for &va in &[DCF_READ_JPEG_EXIF, DCF_READ_X3F_EXIF] {
        let st = Rc::clone(hw);
        cpu.add_hook(va, move |c| {
            let idx = c.r[0] as usize;
            let exif_main = c.r[1];
            let exif_ext = c.r[2];
            let cap = {
                let s = st.borrow();
                let Some(cap) = idx.checked_sub(1).and_then(|i| s.captures.get(i)) else {
                    return HookAction::Return(Some((-1i32) as u32));
                };
                cap.clone()
            };
            if exif_main >= SDRAM_BASE {
                populate_exif_main(c, exif_main, idx as u16, &cap);
            }
            if exif_ext >= SDRAM_BASE {
                populate_exif_ext(c, exif_ext, idx);
            }
            HookAction::Return(Some(0))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(DCF_GET_CUR, move |_| {
            let s = st.borrow();
            let cur = if s.captures.is_empty() {
                0
            } else {
                (s.dcf_cur.min(s.captures.len() - 1) + 1) as u32
            };
            HookAction::Return(Some(cur))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(DCF_SET_CUR, move |c| {
            let req = c.r[0] as usize;
            let mut s = st.borrow_mut();
            if req >= 1 && req <= s.captures.len() {
                s.dcf_cur = req - 1;
                HookAction::Return(Some(1))
            } else {
                HookAction::Return(Some(0))
            }
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(DCF_STEP_NEXT, move |_| {
            let mut s = st.borrow_mut();
            if s.captures.is_empty() {
                return HookAction::Return(Some(0));
            }
            s.dcf_cur = (s.dcf_cur + 1) % s.captures.len();
            HookAction::Return(Some((s.dcf_cur + 1) as u32))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(DCF_STEP_PREV, move |_| {
            let mut s = st.borrow_mut();
            if s.captures.is_empty() {
                return HookAction::Return(Some(0));
            }
            s.dcf_cur = if s.dcf_cur == 0 { s.captures.len() - 1 } else { s.dcf_cur - 1 };
            HookAction::Return(Some((s.dcf_cur + 1) as u32))
        });
    }
    {
        let st = Rc::clone(hw);
        cpu.add_hook(DCF_GET_ENTRY, move |c| {
            let idx = c.r[0] as usize;
            let out = c.r[1];
            if out >= SDRAM_BASE {
                let d = (out - SDRAM_BASE) as usize;
                c.sdram_mut()[d..d + 0x20].fill(0);
                let s = st.borrow();
                if idx >= 1 && idx <= s.captures.len() {
                    c.w16(out + 2, 4704);
                    c.w16(out + 4, 3136);
                    c.w8(out + 0x10, 3); // FINE JPEG
                    return HookAction::Return(Some(1));
                }
            }
            HookAction::Return(Some(0))
        });
    }

    for &va in &[PB_OPEN_JPEG_A, PB_OPEN_JPEG_B, PB_OPEN_IMAGE_A, PB_OPEN_IMAGE_B] {
        let st = Rc::clone(hw);
        cpu.add_hook(va, move |c| {
            let idx = c.r[0] as usize;
            let raw_buf = c.r[1];
            let yuv_buf = c.r[2];
            let sp = c.r[13];
            let dec_ctx = c.r32_be(sp);
            let exif_main = c.r32_be(sp.wrapping_add(4));
            let exif_ext = c.r32_be(sp.wrapping_add(8));

            let cap = {
                let mut s = st.borrow_mut();
                if idx < 1 || idx > s.captures.len() {
                    return HookAction::Return(Some(0));
                }
                s.dcf_cur = idx - 1;
                s.pb_open_idx = Some(idx - 1);
                s.captures[idx - 1].clone()
            };

            if dec_ctx >= SDRAM_BASE {
                let d = (dec_ctx - SDRAM_BASE) as usize;
                c.sdram_mut()[d..d + 0x3C].fill(0);
                c.w16(dec_ctx, LCD_W as u16);
                c.w16(dec_ctx + 2, LCD_H as u16);
                c.w16(dec_ctx + 4, LCD_W as u16);
                c.w16(dec_ctx + 6, LCD_H as u16);
                c.w32(dec_ctx + 8, 2); // JPEG ready
                c.w32(dec_ctx + 0x14, 4096);
                let yuv = if yuv_buf >= SDRAM_BASE { yuv_buf } else { SCRATCH_VA };
                c.w32(dec_ctx + 0x18, yuv);
                c.w32(dec_ctx + 0x1C, yuv + PB_PLANE_Y_LEN as u32);
                c.w32(dec_ctx + 0x20, yuv + (PB_PLANE_Y_LEN + PB_PLANE_C_LEN) as u32);
                c.w32(dec_ctx + 0x24, raw_buf);
                c.w8(dec_ctx + 0x29, 1);
            }
            if exif_main >= SDRAM_BASE {
                populate_exif_main(c, exif_main, idx as u16, &cap);
            }
            if exif_ext >= SDRAM_BASE {
                populate_exif_ext(c, exif_ext, idx);
            }
            HookAction::Return(Some(1))
        });
    }

    {
        let st = Rc::clone(hw);
        cpu.add_hook(PB_DECODE_RESAMPLE, move |c| {
            let dst_va = c.r[1];
            let dst_w = c.r[2] as usize;
            let dst_h = c.r[3] as usize;
            let sp = c.r[13];
            let stride = c.r32_be(sp) as usize;
            let out_desc = c.r32_be(sp.wrapping_add(4));

            let rgba = {
                let s = st.borrow();
                let idx = s.pb_open_idx.unwrap_or(s.dcf_cur);
                s.captures.get(idx).map(|cap| cap.rgba.clone())
            };
            let Some(rgba) = rgba else {
                return HookAction::Return(Some(1));
            };
            if dst_va < SDRAM_BASE || dst_w == 0 || dst_h == 0 || stride < dst_w {
                return HookAction::Return(Some(1));
            }

            // Match yuv_desc_init (0xA0127F9E) 4:2:0 plane spacing & 3:2 vertical centering inside 4:3
            let h_plane = dst_h.max(LCD_H);
            let y_base = (dst_va - SDRAM_BASE) as usize;
            let cb_base = y_base + stride * h_plane;
            let cr_base = cb_base + (stride * h_plane) / 4;
            let c_stride = stride / 2;

            let (img_h, y_pad) = if dst_w == LCD_W && dst_h == LCD_H {
                (LV_H, (LCD_H - LV_H) / 2)
            } else {
                (dst_h & !1, 0)
            };

            let sdram = c.sdram_mut();
            if cr_base + (stride * h_plane) / 4 > sdram.len() {
                return HookAction::Return(Some(1));
            }
            sdram[y_base..y_base + stride * dst_h].fill(0);
            sdram[cb_base..cb_base + c_stride * (dst_h / 2)].fill(128);
            sdram[cr_base..cr_base + c_stride * (dst_h / 2)].fill(128);

            for py in (0..img_h).step_by(2) {
                let dy0 = y_pad + py;
                let dy1 = dy0 + 1;
                let sy0 = (py * LV_H / img_h).min(LV_H - 1);
                let sy1 = ((py + 1) * LV_H / img_h).min(LV_H - 1);
                let c_row = cb_base + (dy0 / 2) * c_stride;
                let cr_row = cr_base + (dy0 / 2) * c_stride;

                for px in (0..dst_w).step_by(2) {
                    let sx0 = (px * LCD_W / dst_w).min(LCD_W - 1);
                    let sx1 = ((px + 1) * LCD_W / dst_w).min(LCD_W - 1);
                    let coords = [(sx0, sy0), (sx1, sy0), (sx0, sy1), (sx1, sy1)];
                    let mut rs = 0i32;
                    let mut gs = 0i32;
                    let mut bs = 0i32;
                    let mut ys = [0u8; 4];
                    for (k, &(sx, sy)) in coords.iter().enumerate() {
                        let p = (sy * LCD_W + sx) * 4;
                        let r = rgba[p] as i32;
                        let g = rgba[p + 1] as i32;
                        let b = rgba[p + 2] as i32;
                        ys[k] = ((77 * r + 150 * g + 29 * b + 128) >> 8).clamp(0, 255) as u8;
                        rs += r;
                        gs += g;
                        bs += b;
                    }
                    sdram[y_base + dy0 * stride + px] = ys[0];
                    if px + 1 < dst_w {
                        sdram[y_base + dy0 * stride + px + 1] = ys[1];
                    }
                    if dy1 < dst_h {
                        sdram[y_base + dy1 * stride + px] = ys[2];
                        if px + 1 < dst_w {
                            sdram[y_base + dy1 * stride + px + 1] = ys[3];
                        }
                    }
                    let r_avg = (rs + 2) >> 2;
                    let g_avg = (gs + 2) >> 2;
                    let b_avg = (bs + 2) >> 2;
                    let cb = (((-43 * r_avg - 85 * g_avg + 128 * b_avg + 128) >> 8) + 128).clamp(0, 255) as u8;
                    let cr = (((128 * r_avg - 107 * g_avg - 21 * b_avg + 128) >> 8) + 128).clamp(0, 255) as u8;
                    sdram[c_row + px / 2] = cb;
                    sdram[cr_row + px / 2] = cr;
                }
            }

            if out_desc >= SDRAM_BASE {
                c.w16(out_desc, dst_w as u16);
                c.w16(out_desc + 2, dst_h as u16);
                c.w16(out_desc + 4, stride as u16);
                c.w16(out_desc + 6, ((dst_w + 15) & !15) as u16);
                c.w32(out_desc + 8, dst_va);
                c.w32(out_desc + 0x0C, SDRAM_BASE + cb_base as u32);
                c.w32(out_desc + 0x10, SDRAM_BASE + cr_base as u32);
            }
            HookAction::Return(Some(0))
        });
    }

    // Milbeaut IIP YCbCr420 -> planar RGB888 converter (`0xA0054AB2`) for SCREEN_PB R/G/B histograms
    cpu.add_hook(PB_YCBCR_TO_RGB_PLANAR, |c| {
        let y_va = c.r[0];
        let cb_va = c.r[1];
        let cr_va = c.r[2];
        let w = c.r[3] as usize;
        let sp = c.r[13];
        let h = c.r32_be(sp) as usize;
        let plane_step = c.r32_be(sp.wrapping_add(4)) as usize;
        let rgb_va = c.r32_be(sp.wrapping_add(8));
        if y_va < SDRAM_BASE || cb_va < SDRAM_BASE || cr_va < SDRAM_BASE || rgb_va < SDRAM_BASE || w == 0 || h == 0 {
            return HookAction::Return(Some(0));
        }
        let y_off = (y_va - SDRAM_BASE) as usize;
        let cb_off = (cb_va - SDRAM_BASE) as usize;
        let cr_off = (cr_va - SDRAM_BASE) as usize;
        let r_off = (rgb_va - SDRAM_BASE) as usize;
        let g_off = r_off.saturating_add(plane_step);
        let b_off = g_off.saturating_add(plane_step);
        let cw = w / 2;
        let sdram = c.sdram_mut();
        if y_off + w * h > sdram.len()
            || cb_off + cw * (h / 2) > sdram.len()
            || cr_off + cw * (h / 2) > sdram.len()
            || b_off + w * h > sdram.len()
        {
            return HookAction::Return(Some(0));
        }
        for ry in 0..h {
            let cy = ry / 2;
            for rx in 0..w {
                let idx = ry * w + rx;
                let cidx = cy * cw + rx / 2;
                let [r, g, b] = ycbcr_to_rgb(
                    sdram[y_off + idx] as i32,
                    sdram[cb_off + cidx] as i32,
                    sdram[cr_off + cidx] as i32,
                );
                sdram[r_off + idx] = r;
                sdram[g_off + idx] = g;
                sdram[b_off + idx] = b;
            }
        }
        HookAction::Return(Some(0))
    });

    // Milbeaut IIP 256-bin byte-plane histogram engine (`0xA004A7AC` -> `0xA00B1D60`)
    cpu.add_hook(PB_IIP_HIST, |c| {
        let src_va = c.r[0];
        let w = c.r[1] as usize;
        let h = c.r[2] as usize;
        let hist_va = c.r[3];
        let out_flag_va = c.r32_be(c.r[13]);
        if src_va < SDRAM_BASE || hist_va < SDRAM_BASE || w == 0 || h == 0 {
            return HookAction::Return(Some(0));
        }
        let s_off = (src_va - SDRAM_BASE) as usize;
        let len = w.saturating_mul(h);
        let mut bins = [0u32; 256];
        {
            let sdram = c.sdram();
            if s_off + len > sdram.len() {
                return HookAction::Return(Some(0));
            }
            for &px in &sdram[s_off..s_off + len] {
                bins[px as usize] += 1;
            }
        }
        let h_off = (hist_va - SDRAM_BASE) as usize;
        if h_off + 1024 <= c.sdram().len() {
            for (i, &cnt) in bins.iter().enumerate() {
                c.w32(hist_va + (i as u32) * 4, cnt);
            }
        }
        if out_flag_va >= SDRAM_BASE {
            c.w8(out_flag_va, 0);
        }
        HookAction::Return(Some(0))
    });
}

fn populate_exif_ext(c: &mut Cpu, exif_ext: u32, idx: usize) {
    let e = (exif_ext - SDRAM_BASE) as usize;
    c.sdram_mut()[e..e + 0x44].fill(0);
    c.w32(exif_ext + 8, 2);
    c.w8(exif_ext + 0x1C, 1);
    c.w8(exif_ext + 0x39, 4);
    c.w8(exif_ext + 0x3A, 18);
    c.w8(exif_ext + 0x3B, 12);
    c.w8(exif_ext + 0x3C, (idx / 60) as u8);
    c.w8(exif_ext + 0x3D, (idx % 60) as u8);
    c.w16(exif_ext + 0x3E, 2026);
}

fn populate_exif_main(c: &mut Cpu, base: u32, fnum: u16, cap: &CaptureFrame) {
    let off = (base - SDRAM_BASE) as usize;
    c.sdram_mut()[off..off + 0x188].fill(0);
    c.w16(base + 0x02, 100); // DCF directory number (100SIGMA -> 100-xxxx)
    c.w16(base + 0x04, fnum);
    c.w8(base + 0x10, 0); // Unprotected
    c.w8(base + 0x1C, 1); // Orientation: horizontal normal
    c.w16(base + 0x1E, 4704);
    c.w16(base + 0x20, 3136);
    c.w16(base + 0x22, cap.iso);
    c.w16(base + 0x24, 5); // Evaluative metering
    c.w32(base + 0x28, cap.tv_num);
    c.w32(base + 0x2C, cap.tv_den);
    c.w32(base + 0x30, cap.av_num);
    c.w32(base + 0x34, cap.av_den);
    c.w16(base + 0x38, 1); // rational EV comp at +0x170/+0x174
    c.w16(base + 0x3A, cap.expo_prog as u16);
    c.w16(base + 0x3E, 0); // Flash off
    c.w16(base + 0x48, 2026);
    c.w8(base + 0x4A, 4);
    c.w8(base + 0x4B, 18);
    c.w8(base + 0x4C, 12);
    c.w8(base + 0x4D, (fnum / 60) as u8);
    c.w8(base + 0x4E, (fnum % 60) as u8);
    c.write_bytes(base + 0x5C, b"SIGMA\0");
    c.write_bytes(base + 0x68, b"SIGMA DP2 Merrill\0");
    c.w32(base + 0x90, cap.ev_steps as u32);
    c.w32(base + 0x94, 3);
    c.w8(base + 0x112, cap.wb);
    c.w8(base + 0x113, 1); // AF
    c.write_bytes(base + 0x114, b"30\0");
    c.w8(base + 0x124, 3); // FINE
    write_f64_be(c, base + 0x128, ((cap.sharp as f64) - 5.0) * 0.2);
    write_f64_be(c, base + 0x130, ((cap.contrast as f64) - 5.0) * 0.2);
    write_f64_be(c, base + 0x138, ((cap.sat as f64) - 5.0) * 0.2);
    c.w8(base + 0x140, 1); // JPEG
    c.w8(base + 0x143, 3); // Single drive
    c.w8(base + 0x156, cap.cs.saturating_sub(1));
    c.w8(base + 0x157, cap.decoded_cm);
    write_f64_be(c, base + 0x160, 30.0); // 30mm focal length
    c.w32(base + 0x170, cap.ev_steps as u32);
    c.w32(base + 0x174, 3);
    // Center AF box on thumbnail (scaled inside 640x424)
    c.w16(base + 0x17C, 292);
    c.w16(base + 0x17E, 193);
    c.w16(base + 0x180, 56);
    c.w16(base + 0x182, 38);
    c.w32(base + 0x184, 4_820_000);
}

fn write_f64_be(c: &mut Cpu, va: u32, val: f64) {
    let bits = val.to_bits();
    c.w32(va, (bits >> 32) as u32);
    c.w32(va + 4, bits as u32);
}

pub(crate) fn clear_yuv420_buf(sdram: &mut [u8], vbuf_va: u32) {
    if vbuf_va < SDRAM_BASE {
        return;
    }
    let y_off = (vbuf_va - SDRAM_BASE) as usize;
    let cb_off = y_off + PB_PLANE_Y_LEN;
    let end = cb_off + PB_PLANE_C_LEN * 2;
    if end <= sdram.len() {
        sdram[y_off..cb_off].fill(0);
        sdram[cb_off..end].fill(128);
    }
}

fn dma_copy_2d(
    cpu: &mut Cpu,
    src: u32,
    src_stride: u32,
    dst: u32,
    dst_stride: u32,
    w_bytes: usize,
    rows: u32,
) {
    if src < SDRAM_BASE || dst < SDRAM_BASE || w_bytes == 0 || rows == 0 || rows > 4096 {
        return;
    }
    for r in 0..rows {
        if let (Some(s_va), Some(d_va)) = (
            src.checked_add(r.saturating_mul(src_stride)),
            dst.checked_add(r.saturating_mul(dst_stride)),
        ) {
            cpu.copy_within_bytes(d_va, s_va, w_bytes);
        }
    }
}

#[inline]
fn is_sim_heap_ptr(ptr: u32) -> bool {
    (0xA0A2_0000..0xA1C0_0000).contains(&ptr) || (0xA300_0000..0xA7F0_0000).contains(&ptr)
}

fn heap_alloc(st: &mut HwState, sz: u32) -> u32 {
    if let Some(pos) = st
        .free_list
        .iter()
        .position(|b| b.cap >= sz && b.cap <= sz.saturating_mul(2))
    {
        let blk = st.free_list.swap_remove(pos);
        st.alloc_caps.push((blk.va, blk.cap));
        return blk.va;
    }
    let mut base = st.heap_bump;
    if base < 0xA1C0_0000 && base.checked_add(sz).is_none_or(|end| end > 0xA1C0_0000) {
        // Skip payload .bss (0xA1C00000..0xA1E00000) and LCD/PB/LV planes (0xA2130000..0xA2B00000)
        base = 0xA300_0000;
    }
    let Some(next) = base.checked_add(sz) else {
        return 0;
    };
    if next > 0xA7F0_0000 {
        return 0;
    }
    st.heap_bump = next;
    st.alloc_caps.push((base, sz));
    base
}

