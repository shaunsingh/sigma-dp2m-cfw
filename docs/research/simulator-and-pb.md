# Dual-CPU Simulator, Native AE, µITRON & `SCREEN_PB` Reference

Addresses are M6M S3 virtual addresses (`0xA0000000..0xA0800000` in `C78FRM.bin`, Thumb bit cleared unless noted as ARM).

## 1. Dual-CPU Simulator Architecture (`crates/arm9`, `crates/dp2m-sim`, `crates/dp2m-web`)

- **M6M (`arm9::Cpu`)**: Big-endian ARMv5TE + Thumb-1 interpreter with 128 MB SDRAM (`0xA0000000..0xA8000000`), basic-block SDRAM execution for unhooked Thumb/ARM code below `0xA0200000`, 16-bit/32-bit Hook bitmap (`HookAction::Jump`, `Return`, `Continue`), peripheral MMIO pages (`0x60000000+`), and optional SDRAM write watchpoints (`set_write_watch`).
- **M5Slave (`fr80::Emulator`)**: Big-endian FR80 interpreter running `Firm.bin` (`0x0003BF80..0x00800000`) on capture:
  1. M6M `ipc_hook` (`0xA00360A2`) packs `cmd 0x20` (`cmd[5] = packed_cm`, `cmd[6] = (fill << 5) | (fx << 2) | cs`).
  2. M6M MakerNote writer (`0xA0148CF8`) emits tag `0x3D` (`"Film NN"` or stock name) and tag `0x59` (full film name), then `0xA01478A6` parses tag `0x3D` back into `decoded_cm` (`rec+0x17`).
  3. M5 `0x005D0340` (`hook_ipc_exif`) unpacks `cmd 0x20` into job slot `+0x150` / `+0x14C`.
  4. M5 `0x005D0000` (`hook_matrix`) multiplies the active White Balance matrix (`0x8000CC44`) with the film CMCM and commits via `0x004B1F20`.
  5. M5 `0x005D0180` (`hook_lutsel`) and `0x005D0200` (`hook_mcas`) select the 4096-B 1D tone LUT and 12-sector MCAS table (`0x004B2088`).
  6. M5 `0x005D1600` (`spatial_driver`) + `0x005D03C0` (`ycc_engine`) run X3 Fill Light, halation pyramid, Sakamoto 17×9×9 Q1 3D LUT (or `D:\LUTn.BIN`), film grain, and B&W paper toning across planar YCbCr.
- **LCD Compositor (`render_lcd`)**: Composites 3 Milbeaut planes at `640×480`:
  1. Video plane (YCbCr 4:2:2 in Live View at `0xA2A00000` / `0xA2A50000` / `0xA2A80000`, `640×424`; YCbCr 4:2:0 in Playback at the active `0xA000F222` buffer, `640×480`).
  2. RGBA4444 graphics overlay at `0xA21D8C00` (`R:12..15, G:8..11, B:4..7, A:0..3`).
  3. 8 bpp indexed OSD plane at `0xA2130000` decoded through the 256-entry `[A, Y, Cb, Cr]` CLUT at `0xA07DEC54` (`alpha 0..15`).

## 2. Native M6M AE Solver & Photometry

M6M computes exposure in signed Q16 APEX units (`1 EV = 0x10000`):

| Address | Symbol / Role | Notes |
| --- | --- | --- |
| `0xA000273C` | `ae_sensor_ev100` | Returns raw uncompensated scene EV100 in Q16 |
| `0xA0002CB2` | `ae_enable_lv_meter(en, mode)` | `(1, 1)` enables Live View photometry updates |
| `0xA0002C36` | `ae_update_bv` | Reads `0xA000273C`, applies EV comp `[0xA07FBE58]`, writes `BV_Q16` to `0xA07AD8B8` |
| `0xA000D1A2` | `ae_set_mode(ae_mode)` | `1 = M`, `2 = A`, `3 = S`, `4 = P` (stored at `AE_STATE[0]`; note UI `0xA09396C0` uses `1=M, 2=P, 3=A, 4=S`) |
| `0xA000D2AA` | `ae_set_auto_iso_min(ae_iso)` | Sets Auto-ISO floor in `AE_STATE` (mapped via `0xA003F288`) |
| `0xA000D2B2` | `ae_set_auto_iso_max(ae_iso)` | Sets Auto-ISO ceiling in `AE_STATE` (mapped via `0xA003F288`) |
| `0xA000D6B6` | `ae_solve_core` | Solves `(Av_Q16, Tv_Q16, SV_Q16)` from `BV_Q16`; hosts `autoiso_a_hook` (`0xA000D83E`) and `autoiso_p_hook` (`0xA000D9A2`, `0xA000D9E8`) |
| `0xA000DDF6` | `ae_av_to_q16(av_idx)` | Converts aperture table index (`0..15`, F2.8..F16) to `Av_Q16` |
| `0xA000DE26` | `ae_tv_to_q16(tv_idx)` | Converts shutter table index (`0..48`, 30s..1/2000s) to `Tv_Q16` |
| `0xA003EABE` | `ae_solve_ui` | Runs `0xA0002C36` + `0xA000D6B6` and writes solved UI indices to `0xA07DD6E0` (`u16 av_idx`) and `0xA07DD6E4` (`u16 tv_idx`) |
| `0xA003EC88` | `ae_sync_ev(ev_idx)` | Syncs UI EV index (`0..18`, `9 = ±0.0 EV` in 1/3-stop steps) into `[0xA07FBE58]` |
| `0xA003EDD0` | `ae_sync_tv` | Syncs manual/S-mode shutter setting from NVRAM into `AE_STATE` |
| `0xA003EECC` | `ae_sync_av` | Syncs manual/A-mode aperture setting from NVRAM into `AE_STATE` |
| `0xA003F288` | `ae_map_iso_idx(ui_iso)` | Maps UI ISO menu value (`0=Auto, 1=ISO 50, 2=ISO 100, ...`) to internal AE table index |
| `0xA003F3A6` | `ae_sync_iso(ui_iso)` | Syncs ISO mode/index into `AE_STATE` |
| `0xA016F4FE` | `get_auto_iso_min_ui` | Reads Auto-ISO minimum UI value (`0xA09396DB`, default `2 = ISO 100`) |
| `0xA016F556` | `get_auto_iso_max_ui` | Reads Auto-ISO maximum UI value (`0xA09396DC`, default `8 = ISO 400`) |
| `0xA016F5AE` | `get_iso_idx` | Reads active ISO UI index (`0xA09396DA`) |
| `0xA016F5DE` | `ae_set_iso_step(ext)` | Syncs 1-EV vs 1/3-EV ISO stepping (`0xA09396DD`) |
| `0xA08008F8` | `AE_STATE` | `+0x00` mode, `+0x10` solved `SV_Q16` (`5.0 = ISO 100`, `+1.0` per doubling) |

APEX sensor brightness law in `0xA000D6B6`: `EV_uncomp_Q16 = (EV100_Q16 - 0x00050000) + SV_Q16`, compared against `Av_Q16 + Tv_Q16` to compute residual exposure gain when physical limits clip or EV compensation / Manual exposure shifts away from metered neutrality.

## 3. µITRON 4.0 Syscalls & Heap Allocator (`0xA0070000..0xA0076000`)

The generic driver stub range `0xA0070000..0xA00B8000` must override syscalls that write caller-supplied pointers or return non-zero timeout/dormant codes:

| Address (ARM) | Syscall | Signature & Required Emulator Behavior |
| --- | --- | --- |
| `0xA0071830` / `1970` / `1C54` | `get_mpf` / `pget_mpf` / `tget_mpf` | `(mpfid, void **p_blk [, tmout])`: allocate zeroed 256-B block, write `*p_blk`, return `0` (`E_OK`) or `-33` (`E_NOMEM`). Used by `0xA011512A` during Tab 0 `ISO Sensitivity` formatting |
| `0xA0071B00` | `rel_mpf` | `(mpfid, void *blk)`: return block to freelist |
| `0xA0071F30` / `212C` / `22A0` | `get_mpl` / `pget_mpl` / `tget_mpl` | `(mplid, uint blksz, void **p_blk [, tmout])`: allocate `blksz` bytes, write `*p_blk`, return `0` |
| `0xA0072388` | `rel_mpl` | `(mplid, void *blk)`: return block to freelist |
| `0xA0072C74` | `ref_mbx` | `(mbxid, T_RMBX *pk_rmbx)`: zero `pk_rmbx[0..8]` (no waiting task, no queued message) |
| `0xA0073B70` | `get_tid` | `(ID *p_tskid)`: write `*p_tskid = 0` (`u16`), return `0` |
| `0xA00752D4` | `ref_tsk` | `(tskid, T_RTSK *pk_rtsk)`: zero `0x24` B and set `pk_rtsk->tskstat = 0x10` (`TTS_DMT`). `0xA0045948` and `0xA0057C0E` check `tskstat == 0x10` before allowing `DISP` view changes in `SCREEN_PB` |
| `0xA00729B8` / `3240` / `4808` / `4AC0` / `4C50` | `prcv_mbx` / `trcv_mbx` / `prcv_dtq` / `trcv_dtq` | Return `-50` (`E_TMOUT`) so polling loops exit immediately |

C heap symbols (`0xA0A20000..0xA1F00000`): `malloc` `0xA003D9C0`, `free` `0xA003D9AC`, `calloc` `0xA003D990`, `realloc` `0xA003D9D8` (required by dynamic menu list builders when scrolling past row 6).

## 4. Milbeaut 2D DMA, B2Y & IIP Hardware Wrappers

| Address | Mode | Signature & Behavior |
| --- | --- | --- |
| `0xA00B125C` | ARM | `dma_2d_copy_mode(mode, src, src_stride, dst_stride, [sp] dst, w_bytes, rows)` |
| `0xA00B14A0` | ARM | `dma_2d_copy(src, src_stride, dst_stride, dst, [sp] w_bytes, rows)` — blits Y/Cb/Cr planes in `SCREEN_PB` thumbnails and zoom |
| `0xA00B1870` | ARM | `dma_rect_fill(mode, fill_u8, stride, dst, [sp] w_bytes, rows)` — clears OSD/ARGB rectangles on menu close |
| `0xA00B23A8` | ARM | `dma_mem_copy(dst, src, len)` |
| `0xA009C334` | ARM | `Im_B2Y_Set_Color_Matrix(ch, mat_i16_q8_ptr)` — 3×3 RGB matrix (`1.0 = 0x100`, shifted `>> 1` to Q7) |
| `0xA009CD7C` | ARM | `Im_B2Y_Set_Mono(ch, ctrl_ptr)` — `[0]` enable, `[2]` signed Cb offset, `[3]` signed Cr offset |
| `0xA009CF84` | ARM | `Im_B2Y_Set_Chroma_Gain(ch, gain_u8)` — `200` (`0xC8`) = unity saturation |
| `0xA009D650` | ARM | `Im_B2Y_Set_Sharp_LUT` — sharpness preset in `r4` |
| `0xA009D940` | ARM | `Im_B2Y_Set_Gamma_Table(ch, lut_1024_u8_ptr, len)` |
| `0xA0054AB2` | Thumb | `pb_ycbcr_to_rgb_planar(y_va, cb_va, cr_va, w, [sp] h, plane_step, rgb_dst_va)` — converts 4:2:0 thumbnail into planar `R`, `G`, `B` (`rgb_dst + {0, 1, 2} * plane_step`) |
| `0xA004A7AC` | Thumb | `pb_iip_hist(plane_va, w, h, hist_u32_256_va, [sp] out_flag_u8_ptr)` — wraps ARM `0xA00B1D60` to count 256 byte-plane bins (`u32 BE`) |

## 5. `SCREEN_PB` (`0x19` / `0xA00575E2`) & Virtual DCF Pipeline

- **State**: `PB_STATE = 0xA07F21D8`. `+0x14` (`u16`) = No-Image flag, `+0x34` (`0xA07F220C`, `u32`) = current 1-based DCF index (reset to `0` on new capture so `0xA0053354(0)` loads the latest image), `+0x3C` (`u32`) = total DCF image count.
- **Video buffer selector**: `0xA000F222(mask)` maps `1 → 0xA235E600`, `2 → 0xA243F600`, `4 → 0xA2520600`, `8 → 0xA2601600` (each `640×480` 4:2:0 planar: `Y` `0x4B000` B, `Cb` `0x12C00` B, `Cr` `0x12C00` B = `0x70800` B; buffer stride `0xE1000` B).
- **View cycle (`Key::Disp = 0x07` → `0xA0057096`)**: Steps `[0xA0939698]` display mode across 4 layouts in `PB_OSD_DISPATCH` (`0xA005300E`):
  1. `layout = 1` (`0xA00530AC`): Basic Info (`4704x3136`, `100-0001`, `cur/total`, Tv, Av, EV, ISO).
  2. `layout = 4` (`0xA005308C`): Clean full-frame image (`0` OSD pixels).
  3. `layout = 5` (`0xA0055810` + `0xA004653E`): Detailed Info (`384×256` thumbnail at `(0, 48)`, Y/R/G/B histograms via `0xA0054AB2` + `0xA004A7AC` + `0xA0048F7C`, file size, MakerNote film stock via `pb_info_cm`, Contrast/Sharpness/Saturation, full EXIF, date/time).
  4. `layout = 2` (`0xA0053102`): Exposure Info overlay.
- **Zoom (`Key::DialRight = 0x40` / `Key::DialLeft = 0x20`)**: `0xA005625E` → `PB_ZOOM_CORE` (`0xA00544A8`) resamples into `0xA53132C0` via `0xA0053F5C` and blits the panned `640×480` crop + green picture-in-picture navigator box.
- **DCF & decode hooks**:
  - `0xA0116A4C` (`sd_mount_status` → `3`), `0xA014FDE6` (`dcf_scan_ready` → `1`), `0xA01149EC` (`dcf_sync_cur` → `0`), `0xA0155144` (`sd_free_space(p_free, p_total)`).
  - `0xA0134234` / `0xA01539DE` (`dcf_count_images` / `dcf_get_total`): must also write `u32` count into `DCF_DIR_COUNTS = 0xA0962758` (999-entry per-folder table summed as low `u16` by `0xA004589C` for the bottom-left `cur/total` badge).
  - `0xA0053354(mode)` (`pb_step_or_cur`: `0` = current, `1` = next, `2` = prev; returns 1-based index), `0xA01539BE` (`dcf_get_cur`), `0xA0153CE6` (`dcf_set_cur`), `0xA0153C22` / `0xA0153C7E` (`dcf_step_next` / `prev`).
  - `0xA01151D0(idx, &dir, &fnum, &ftype, [sp] &a1, &a2)`: sets `dir = 100`, `fnum = idx`, `ftype = 1` (JPEG; bit 0 routes `0xA0053068` to `0xA01342EC` `dcf_read_jpeg_exif`, bit 4 routes to `0xA0134326` `dcf_read_x3f_exif`).
  - `0xA0057DE6` / `0xA0057D40` (`pb_open_jpeg_a/b`) and `0xA0057F16` / `0xA0057EB0` (`pb_open_image_a/b`): populate `dec_ctx` (`[sp+0]`, `0x3C` B), `exif_main` (`[sp+4]`, `0x188` B), and `exif_ext` (`[sp+8]`, `0x44` B).
  - `0xA0053F5C(dec_ctx, dst_va, dst_w, dst_h, [sp] stride, out_desc)`: resamples captured image into planar 4:2:0 at `dst_va` (`Y` = `stride * max(dst_h, 480)`, `Cb` = `Y + ...`, `Cr` = `Cb + .../4`); letters `3:2` (`640×424`) inside `640×480` only when `(dst_w, dst_h) == (640, 480)`, and fills `0..dst_h` directly for thumbnails and zoom buffers.

### `exif_main` Struct Layout (`0x188` B, Big-Endian)

| Offset | Type | Field | Consumers |
| --- | --- | --- | --- |
| `+0x02` | `u16` | DCF directory (`100`) | `0xA0045FE8` (`"%d-%04d"` top-right badge) |
| `+0x04` | `u16` | DCF file number (`1..9999`) | `0xA0045FE8` |
| `+0x10` | `u8` | Protect flag (`0` = normal, `1` = key icon) | `0xA00460CA` |
| `+0x1C` | `u8` | EXIF Orientation (`1` = horizontal normal) | `0xA00544A8`, `0xA0055810` |
| `+0x1E` | `u16` | Image width (`4704`) | `0xA0046226` (`"%dx%d"`), `0xA00544BE` |
| `+0x20` | `u16` | Image height (`3136`) | `0xA0046226`, `0xA00544BE` |
| `+0x22` | `u16` | ISO speed (`50..6400`) | `0xA0045E9A` |
| `+0x24` | `u16` | Metering mode (`5` = Evaluative) | `0xA004657C` |
| `+0x28` / `+0x2C` | `u32` / `u32` | ExposureTime `num` / `den` | `0xA0045D20` |
| `+0x30` / `+0x34` | `u32` / `u32` | FNumber `num` / `den` (`F4.0 = 40/10`) | `0xA0045DEC` |
| `+0x38` | `u16` | EV format (`1` = rational `i32` at `+0x170`/`+0x174`, `0` = APEX at `+0x90`/`+0x94`) | `0xA0046592` |
| `+0x3A` | `u16` | ExposureProgram (`1=M, 2=P, 3=A, 4=S`) | `0xA0046650` |
| `+0x3E` | `u16` | Flash mode (`0` = off) | `0xA0046710` |
| `+0x48..+0x4E` | `u16, 5×u8` | Capture timestamp (`year, mon, day, hr, min, sec`) | `0xA00468B0` |
| `+0x5C` | `char[12]` | Make (`"SIGMA\0"`) | `0xA00462A2` |
| `+0x68` | `char[32]` | Model (`"SIGMA DP2 Merrill\0"`) | `0xA00462A2`, `0xA0046840` |
| `+0x90` / `+0x94` | `i32` / `u32` | ExposureBias rational (`steps` / `3`) | `0xA00465A8` |
| `+0x112` | `u8` | White Balance ID | `0xA0046780` |
| `+0x113` | `u8` | Focus mode (`1` = AF) | `0xA00467A0` |
| `+0x114` | `char[8]` | Lens focal string (`"30\0"`) | `0xA0046810` |
| `+0x124` | `u8` | JPEG quality (`3` = FINE) | `0xA0046690` |
| `+0x128` / `130` / `138` | `3×f64` | Sharpness / Contrast / Saturation (`-1.0..+1.0` in `0.2` steps) | `0xA00467D0` |
| `+0x140` | `u8` | File format (`1` = JPEG) | `0xA00466B0` |
| `+0x143` | `u8` | Drive mode (`3` = Single) | `0xA0046750` |
| `+0x156` | `u8` | ColorSpace (`0` = sRGB, `1` = AdobeRGB) | `0xA004612C` |
| `+0x157` | `u8` | Color mode ID (`1..8` stock, `9..` custom film decoded by `pb_cm_parse`) | `pb_info_cm` (`0xA00465C4` hook) |
| `+0x160` | `f64` | FocalLength (`30.0`) | `0xA0046810` |
| `+0x170` / `+0x174` | `i32` / `u32` | Rational EV compensation (`steps` / `3`) when `+0x38 == 1` | `0xA004659C` |
