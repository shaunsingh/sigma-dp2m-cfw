# Live view

Addresses are M6M virtual addresses in the stock 1.05 image. The implementation is `src/m6m/lv.c` (color) and `src/m6m/hud.c` (overlays).

## Refresh path

- The color mode setter `0xA016F86E` writes NVRAM `0xA09396C3` through `0xA0167946` when the value changed (cache `0xA07FBC10`) and calls `0xA003A260`, which sets the color-dirty flag `0xA07B1398 + 2`. The getter `0xA0167926` also sets the flag (at `0xA016793E`) when it forces Sepia to Standard.
- The LV config task takes messages from mailbox 0xC (handler table `0xA07388CC`), posted by LV / AE control code (`0xA0036CA8..0xA00379FE`). Its handlers call `0xA003A27C → 0xA003A180(lv_mode, sub, iso)`.
- Inside `0xA003A180`:
  - gamma (`0xA0039864`, using `0xA0039720(lv_mode, iso)`; the `Im_B2Y_Set_Gamma_Table` call is at `0xA0039890`) runs only when the LV mode changes;
  - the mono routine (`0xA003989C`, call site `0xA003A1E6`) runs on an LV mode change or when the color-dirty flag is set;
  - the color matrix routine (`0xA0039CE2`, call site `0xA003A20A`) runs on every call.
- So in stock, a color mode change updates the matrix live but not the tone curve. `0xA003A180` zeroes r6 at `0xA003A1B8`, so the original `sub` argument is only on the caller's stack (`[sp + 0x48]` at that point).
- `Im_B2Y_Set_Gamma_Table` `0xA009D940` takes its own hardware lock (`0xA009B368(6)`, copy to `0x60024000 + ch * 0x400`, `0xA009B798(6)`), so calling it again from the mono call site is safe. `0xA009CC28` does not need to be re-run.

## B2Y hardware constraints

- **Gamma** `0xA009D940(ch, lut, size)`: use `ch = 0` and keep `ctrl_ptr[1] = 0` (`MMIO[0x60020700] = 0x0800`, normal luminance mode). `ctrl_ptr[1] = 1` (`0x0801`, GMACTL bit 0) causes highlight speckling and blotchy banding.
- **Color matrix** `0xA009C334` (`Im_B2Y_Set_Color_Matrix`): fill BT.601 Y weights `m[9..11] = {0x004D, 0x0096, 0x001D}` and `m[12] = 2 << 8`. Entries are signed 12-bit. Normalize film matrices to unit mean row gain (1024) to keep the 10-bit B2Y SNR, and evaluate the tone curve at `x_lin * gain`.
- Live CCMatrix for the current LV mode: `0xA01633CA` (mode 7) / `0xA016339A` (other modes), 36 B with nine signed 12-bit BE values from byte 10.
- Stock LV gamma table: `0xA0268A14` (1024 B).

## dp2m hooks

| Site | Function | Change |
| --- | --- | --- |
| `0xA0039890` | `lv_gamma_hook` | remembers the stock gamma pointer and size; applies the film curve for the current packed mode |
| `0xA003A1E6` | `lv_mono_hook` | maps films to stock mono (2) or standard (3); re-applies gamma when the packed mode changed, so the tone curve follows mode and paper changes live |
| `0xA003A20A` | `lv_cmcm_hook` | film CMCM (Q10) times the live CCMatrix (Q7); mono films write Q7 channel weights directly |
| `0xA0033F1C` | `lv_ycap_hook` | records the Y / Cb / Cr plane pointers and a frame counter in payload `.bss` |
| `0xA016BC4E` | `osd_cm` | routes films through Vivid (4) or B&W (2) via `osd_map`, then `osd_icon` (`0xA016BC86`) substitutes their `0xC00` label tile |

Paper setters call `0xA003A260` so a paper change also marks the color dirty.

### Looks table

`dp2m_lv_looks` (generated into `catalog.c`, section `.lv`) has one entry per packed mode 9..198:

| Field | Content |
| --- | --- |
| `cmcm[9]` | Q10 matrix left-multiplied onto the live CCMatrix, or Q7 direct weights with `LV_MONO_Q7` |
| `knots[18]` | tone curve; all zero keeps the stock gamma |
| `flags` | `LV_CMCM` 1, `LV_MONO_Q7` 2, `LV_KNOTS` 4 |

Knots sit at x = 0..128 step 16, 192..384 step 64, 512..1023 step 128 and are linearly expanded to the 1024-entry gamma table on the camera. The builder rejects non-monotonic knots.

## Overlays (grid modes 7..17)

From `crates/dp2m/src/catalog.rs` and `src/m6m/hud.c` (stock grids are `0..6`):

| Id | Slug | Menu label | Drawing |
| --- | --- | --- | --- |
| `7` | `level` | Electronic Level | tilt horizon + center tick |
| `8` | `xpan-21x9` | 21:9 XPan + Level | horiz 74 px matte + level |
| `9` | `cinema-16x9` | 16:9 Cinema + Level | horiz 31 px matte + level |
| `10` | `square-1x1` | 1:1 Square + Level | vert 107 px matte + level |
| `11` | `peaking` | Focus Peaking | 2×2 edge pass (`NV_PEAKCOL`, `NV_PEAKSENS`) |
| `12` | `peaking-level` | Peaking + Level | peaking + level |
| `13` | `zebras` | Zebras | 2×2 animated stripe pass (`NV_ZEBRALVL`) |
| `14` | `rgb-hist` | RGB Hist + Level | level + 64-bin RGB histogram |
| `15` | `frame-7x6` | 7:6 + Level | vert 70 px matte + level |
| `16` | `frame-4x3` | 4:3 + Level | vert 37 px matte + level |
| `17` | `false-color` | False Color (IRE) | 6-zone IRE heatmap + right-edge legend |

- The grid widget getter (word `0xA07FA4A4`) returns a dirty key; `grid_widget_get` returns `mode << 16 | portrait << 8 | ((slope + 64) & 0xFF)` (deadband `|slope| <= 1`) for tilt modes (`7..10, 15, 16`) and `mode << 16 | ((fcnt >> 1) & 0xFFFF)` for per-frame modes (`11..14, 17`). Overlays draw only on `SCREEN_LV` (id 21) with LV active (`0xA00344F2 == 1`).
- Grid draw call sites `0xA016C708` and `0xA004C5AE` (`0xA004C3EC`) are hooked.
- Drawing goes to the 640×480 BE RGBA4444 (`R:12..15, G:8..11, B:4..7, A:0..3`) overlay `0xA21D8C00` with fill `0xA00419A6` and flush `0xA0041C7A`, bracketed by `hw_dcache_sync()` (`SWI 10`). The LV image area is 640×424. Full-frame overlays (peaking, zebras, false color) write 2×2 blocks as 32-bit pixel pairs on a DMA-zeroed back-buffer.
- Planes: Y 640 B per row, Cb / Cr 320 B per row. The pointers are only trusted inside `0xA0800000..0xB0000000` (4-byte aligned).
- Accelerometer: MEMSIC MXC6226XC on I2C `0x15`, signed 8-bit X / Y getters `0xA001A38E` / `0xA001A3A2`.
