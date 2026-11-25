# Sigma DP2 Merrill (C78) Hardware & Firmware Reference

Empirical findings verified against `DP2MV105.bin` (May 2013 S3 M6M + M5Slave) and physical DP2 Merrill hardware.

## 1. Firmware Package & Updater Behavior

`DP2MV105.bin` (`25,968,896` bytes) contains three concatenated segments:

| Offset | Length | Contents |
| --- | --- | --- |
| `0x00000000` | `0x00820080` | Package header + earlier M6M image |
| `0x00820080` | `0x00884080` | M5Slave (`Firm.bin`, magic `SIGMA.CODSLRSD15`, `VA = file_off + 0x3BF80`) |
| `0x010A4100` | `0x00820000` | M6M (`C78FRM.bin`, magic `FUJITSU!`: `0x20000` IPL + `0x800000` S3 mapped at `0xA0000000`) |

SD-card updater rules (confirmed from `0xA000CFCA`, `0xA000D3BC..0xA000D40A`, and on-camera flashing):
- Placing `Firm.bin` and/or `C78FRM.bin` in the SD root flashes each MCU independently.
- Flash error **`0002`** occurs if `Firm.bin` has non-stock bytes at `VA >= 0x00800000` (outside the physical 8 MB NOR flash) or if the 32-bit additive checksum at `Firm.bin[0x4C..0x50]` fails to match `sum(s2[0x80:0x800080])`. If error `0002` locks input on screen, pull the battery — the camera boots cleanly on its prior flash image.

---

## 2. M5Slave (`Firm.bin` — Fujitsu FR80 Big-Endian, "TRUE II")

### 2.1 ISA & Silicon Pitfalls

- **Shift opcodes** (verified against GNU binutils `cpu/fr30.cpu` and stock ROM `0x4D3F6E..0x4D3F72`: `b1f1 lsr2 #31; a610 add; b810 asr #1`):
  - `0xB0` = `LSR #u4, Ri` (logical right shift)
  - `0xB4` = `LSL #u4, Ri` (logical left shift)
  - `0xB8` = `ASR #u4, Ri` (arithmetic right shift)
  - Swapping `0xB0`/`0xB8` corrupts signed negative Foveon CCMatrix off-diagonals (`-154 >> 10` becomes `+4194303`), producing severe magenta blowouts and clamping signed grain/3D-LUT deltas to `255`.
- **Sign/zero extensions**: `0x978r = extsb`, `0x979r = extub`, `0x97Ar = extsh`, `0x97Br = extuh`.
- **Unsigned pointer compares**: `0xE4 = blo/bc` (borrow set, `dst < src` unsigned), `0xE5 = bhs/bnc` (`dst >= src` unsigned). Use `blo`/`bhs` for SDRAM pointer bounds rather than signed `blt`/`bge`.
- **Indexed load hygiene**: Avoid `ldub @(r13, Rx), r13` (`dst == index`); `crates/fr80` rejects it at assembly time.
- **D-Cache**: Disabled on M5Slave (`0x483064: stb #0, @0x03E3`). CPU SDRAM writes are immediately visible to DMA engines, but uncached byte-wise SDRAM loads/stores dominate loop wall-clock cost.

### 2.2 NOR Flash Constraints & Layout

- Physical NOR flash holds `VA < 0x00800000` (`file_off < 0x7C4080`). `s2[0x7C4080..]` must remain bit-exact stock.
- Checksum invariant: `u32_be(s2[0x4C..0x50]) == sum(s2[0x80..]) == sum(s2[0x80..0x800080])` (mod `2^32`).
- Verified zero-filled free flash banks in stock `DP2MV105.bin`:
  - **Bank A**: `0x005D0000..0x00800000` (`2,293,760` B) — hooks, YCC engine, spatial driver, ROM LUTs, CMCM, MCAS (sRGB + AdobeRGB), 1D LUTs (sRGB + AdobeRGB), 40 B&W toners, 132 sRGB 3D LUTs (`ends 0x007FF4A0`).
  - **Bank B**: `0x00380000..0x00410000` (`589,824` B) — 132 AdobeRGB 3D LUTs (`ends 0x004054A0`).

### 2.3 SDRAM Map (Audited Across All `ldi:32` Pointers in `0x480000..0x550000`)

| Range | Size | Usage & Notes |
| --- | --- | --- |
| `0x80000000..0x800F4C60` | ~979 KB | `.bss`, RTOS fixed pools (`0x8005AB50..0x800EDBE0`), system stack (`sp` init `0x800F4C60` @ `0x482B88`) |
| `0x800F5000..0x8050DFFF` | ~4.1 MB | Unused after boot (`0x801A0000` @ `0x48311C` is only an offline factory RAM test) |
| `0x80100000..0x80104560` | ~17.3 KB | **Custom scratch**: halation grids (`0x80100000`, `0x80101100`), fill grid (`0x80102200`), 5-tap blur line (`0x80103300`), pre-scaled grain LUT (`0x80103400`), SD LUT load buffer (`0x80103500..0x80104560`) |
| `0x8050E000..0x80FB0580` | 10.63 MB | Hardware JPEG output buffer (`0x4D4DD6`) |
| `0x80FB0580..0x81046580` | 600 KB | `PREVIEW_Y` (`0x80FB0580`), `PREVIEW_CB` (`0x80FFB580`), `PREVIEW_CR` (`0x81020D80`) (`0x4D71CC`, `0x4E9758`) |
| `0x81046580..0x8105A580` | 80 KB | `THUMB_Y/CB/CR` (`0x4E9770..0x4E9780`) — **written concurrently by background EXIF/SD task `0x4E9714`** |
| `0x8104B590+` | — | Main RAW / RGB / YCC pipeline buffers (`0x4D4C90`, `0x4D64F2`) |

Critical concurrency & buffer lessons learned on hardware:
- **Never place scratch or cross-shot state in `0x81046580..0x8105A580`**: On back-to-back shots, Shot #1's background EXIF task (`0x4E9714`) DMAs thumbnail pixels over `0x81046580..0x81050580` while Shot #2 is executing.
- **Keep `0x4D3EC8` at stock `0.70f` (`0x3F333333`)**: This constant is a `W * H * factor` JPEG byte-budget multiplier, not a quantization scalar. At `4704x3136`, `0.70f` caps output at `9.85 MB` (`<= 0x80EE71EC`, `824 KB` below `PREVIEW_Y`). Raising it to `0.80f` (`11.80 MB`) overflows `0x80FB0580` (`PREVIEW_Y`), corrupting EXIF thumbnails and leaving a bottom gray bar.
- **Keep `0x4D7398` (`call 0x004D7F9E`) untouched**: `0x4D7F9E` waits on both async hardware resizer DMA channels (`ch=0` Preview, `ch=1` Thumb via `0x4A88CA`). Stock `0x4D7DE2` generates gallery previews and EXIF thumbnails directly from the hardware B2Y output (`CCMatrix + 1D LUT + 12-Sector MCAS`) with zero cross-shot state.

### 2.4 Stock Symbols & Hook Table

| Stock VA | Original / Role | Hook VA | Source |
| --- | --- | --- | --- |
| `0x004D3782` | `0x0053BEF0` (CCMatrix) | `0x005D0000` | `src/asm/m5/hook_matrix.fr80` |
| `0x004E6414` | `0x004950E0` (4096B LUT sel) | `0x005D0180` | `src/asm/m5/hook_lutsel.fr80` |
| `0x004E6424` | `0x004B3546` (12-sector MCAS) | `0x005D0200` | `src/asm/m5/hook_mcas.fr80` |
| `0x004DC8F6` | `0x00497674` (set CS) | `0x005D0300` | `src/asm/m5/hook_ipc_exif.fr80` |
| `0x004D76E2` | `0x004D4FF4` (EXIF CS) | `0x005D0320` | `src/asm/m5/hook_ipc_exif.fr80` |
| `0x004E2862` | `0x004DCC48` (IPC cmd 0x20) | `0x005D0340` | `src/asm/m5/hook_ipc_exif.fr80` |
| `0x005147C4` | `0x00512E30` (JPEG EXIF CS) | `0x005D0380` | `src/asm/m5/hook_ipc_exif.fr80` |
| `0x00514680` | inline 22 B (X3F EXIF CS) | inline | `crates/dp2m/src/m5.rs` |
| `0x004D738E` | `0x004D3E82` (HW JPEG enc) | `0x005D1600` | `src/asm/m5/spatial_driver.fr80` |

Additional verified stock M5Slave addresses:
- `0x8000CC44`: live active Foveon Q7 signed 12-bit CCMatrix (9 × `u16 BE`, sign bit `0x0800`). Committed via `0x004B1F20(struct_ptr)` where `struct[0..2] = 0x0200`.
- `0x80001B88`: live ColorSpace RAM flag (`1 = sRGB`, `2 = AdobeRGB`).
- `0x80012E46`: 18-byte SDRAM shadow of the live sRGB/AdobeRGB primary matrix (maintained by `0x4B20A6`). Always copy from `0x80012E46` rather than MMIO `0x40100500` (byte-read hazard on MMIO).
- `0x0054CC94` / `0x00552C94`: stock 4096-byte Standard / Monochrome 1D tone LUTs (used to invert DP2M's native linear exposure scale).
- `0x00571368` (32 B `MCB`) / `0x0057139C` (20 B `MCID`): stock MCAS sector boundary & ID tables (`0x1AE`-byte MCAS struct committed via `0x004B2088(1, buf)`).
- `0x0048AB80` (`MDF_W_open`), `0x0048ABAA` (`MDF_W_close`), `0x0048ABD0` (`MDF_W_read`): native FAT32 SD I/O wrappers used to hot-load `D:\LUT1.BIN..D:\LUT8.BIN`.
- `0x004DD702(job_id)` / `0x004DD72A(job_id)`: per-job `packed_cm` (`slot+0x150`) and `packed_1d` (`slot+0x14C`) getters. Stock modes `1..8` translate to native M5Slave enum (`3=Standard, 1=Sepia, 2=Mono`), so `ipc_unpack` only overrides `slot+0x150` when `cmd[5] >= 9`.

### 2.5 Color Pipeline & `D2ML` SD-Card 3D LUT Format

- **Dual-colorspace 17×9×9 Q1 Sakamoto 3D residual LUT**: `17` luma (`step 16`) × `9` Cb (`step 32`) × `9` Cr (`step 32`) × `3` signed `i8` (`0.5`-LSB Q1 deltas) = `4,131` B (`LUT3D_STRIDE = 0x1028`).
- **`D2ML` SD-card binary (`4,160` B = `0x1040`, Big-Endian)**:
  - `0x0000` (4 B): `"D2ML"` (`0x44324D4C`)
  - `0x0004` (1 B): version `0x01`
  - `0x0005` (1 B): `grain_amp` (`0..64`)
  - `0x0006` (1 B): `grain_profile` (`0=Neg, 1=Slide, 2=B&W`)
  - `0x0007` (1 B): `halo_gain` (`0..255`)
  - `0x0008` (1 B): `halo_tint` (`0=Crimson, 1=Neutral B&W, 2=Warm Amber`)
  - `0x0009` (7 B): ASCII tag (zero-padded)
  - `0x0010` (4,131 B): `17×9×9×3` Q1 `i8` residual LUT, zero-padded to `0x1040`.
- **Measured native 1:1 crop throughput** (`SDIM4449.JPG` 256×128, ~70M effective FR80 insns/s with uncached SDRAM):
  - `FX=0` (HW Fast): `0.0` insns/pair -> `0.0 s`
  - `FX=1` (3D LUT): `73.4` insns/pair -> `~7.7 s` (`L`), `~3.7 s` (`M`), `~1.9 s` (`S`)
  - `FX=2` (+Grain): `126.6` insns/pair -> `~13.3 s` (`L`), `~6.4 s` (`M`), `~3.3 s` (`S`)
  - `FX=3` (+Halation): `173.6` insns/pair -> `~18.3 s` (`L`), `~8.8 s` (`M`), `~4.5 s` (`S`)
  - `FX=4` (Full Analog): `222.7` insns/pair -> `~23.5 s` (`L`), `~11.3 s` (`M`), `~5.8 s` (`S`)

---

## 3. M6M (`C78FRM.bin` — ARM926EJ-S Big-Endian)

### 3.1 Protected Regions & Verified Safe ROM Caves

Never modify:
- **Xilinx Spartan-6 FPGA bitstream**: `0xA0179000..0xA023D35A` (`803,674` B; length stored at `0xA07AE444`). Corrupting any byte breaks FPGA boot configuration (white screen, unresponsive shutter).
- **`.bss` tail**: `0xA07FBC1C..0xA0800000`.
- **Shared literal pool**: `0xA004D3D0` (holds `SYM_GRID_MENU = 0xA07EEFCC` and is shared by other callers; hook `Get_Menu_Table` @ `0xA004CD08` instead).
- **User-reachable UI screens `47..54`**: `0xA005CDF4..0xA005F1A0` and vtable slots `0xA07DEBE8..0xA07DEC07` (Reverse Dial, DPOF, Delete, Face AF).

Safe zero-external-caller ROM caves:
- **Cave 1**: `0xA005A400..0xA005BE38` (`6,712` B; factory screens `33..42`, vtable `0xA07DEBB0..0xA07DEBD4` stubbed to `VA_SAFE_RET0 | 1`).
- **Cave 2S**: `0xA005BE38..0xA005CDF4` (`4,028` B; factory screens `43..46`, vtable `0xA07DEBD8..0xA07DEBE4` stubbed to `VA_SAFE_RET0 | 1`).
- **FacMenu Cave**: `0xA07EC9CC..0xA07ED82C` (`3,680` B; factory service menus `0x18..0x22`, strictly below `Imager Temp` menu `0xA3` at `0xA07EDC0C`).

### 3.2 NVRAM & Live View RAM Map

- Stock NVRAM settings block: `0xA0939684..0xA0939A47` (flushed to NOR flash by `0xA01674FA`; factory defaults at `0xA07F9E68`, where bytes `+4..+19` are 16 zero-reserved bytes).
- **Do not write `0xA0939774..0xA0939783`**: Sigma stores **Language** at `0xA0939776` and **LCD settings** at `0xA0939778`. Writing a frame counter there cycles the camera UI language on every menu exit.
- Safe allocations inside the 36-byte reserved header `0xA0939688..0xA09396AB`:
  - `0xA0939688`: `Paper` (`0=Auto`, `1..8`) — NVRAM persisted
  - `0xA0939689`: `FilmFX` (`0..4`) — NVRAM persisted
  - `0xA093968A`: `X3Fill` (`0..3`) — NVRAM persisted
  - `0xA093968B`: `MinShutter` (`0..6`) — NVRAM persisted
  - `0xA093968C`: `PeakColor` (`0=Red, 1=Yellow, 2=White, 3=Black`) — NVRAM persisted
  - `0xA0939694`: volatile Live View frame counter (`u32`)
  - `0xA0939698..0xA09396A3`: volatile Live View `Y`, `Cb`, `Cr` SDRAM plane pointers (`3 × u32`)
- Exposure mode `0xA09396AC` (`+0x28`): `0..3` M/S/A/P, `4` Movie, `5..7` C1..C3, `8` temporary (playback), `9/10` C4/C5. C1..C3 slots are NVRAM `+0x194` (`3 × 0x8C`); C4/C5 slots live in payload `.bss` and persist in NOR bank 0 at `0x400..0x51B` (magic `"C4C5"` + `2 × 0x8C`; the stock record ends at `0x3E4`).

### 3.3 Milbeaut B2Y Live View Hardware Findings

- **Gamma table (`0xA009D940`, `Im_B2Y_Set_Gamma_Table`)**: Always call with `ch = 0` and leave `ctrl_ptr[1] = 0` (`MMIO[0x60020700] = 0x0800`, stock normal luminance mode). Setting `ctrl_ptr[1] = 1` (`0x0801`, GMACTL bit 0 differential/full mode) causes highlight speckling and blotchy banding on the live-view display.
- **Color matrix (`0xA009C334`, `Im_B2Y_Set_Color_Matrix`)**: Always populate BT.601 Y weights `out_m[9..12] = {0x004D, 0x0096, 0x001D}` and `out_struct[0x18] = 2`. Normalize CMCM rows to unit mean gain (`1024`) to preserve 10-bit B2Y SNR, and evaluate the 18-knot perceptual live-view gamma spline at `x_lin * gain`.
- **Menu row limit**: Shooting/Setup menu pages clip off-screen if they exceed 6 rows.

### 3.4 `sd Quattro H` Backports & Hardware Peripherals

- **ISO 50 unlock**: Menu tables `0xA07EAC8C` (1-EV) / `0xA07EADAC` (1/3-EV) updated via literal pool `0xA004D3DC`; clamps `0xA003F2CC` and `0xA003F30A` pass `val = 1` (`sid = 53` "ISO 50") through to M5Slave `LinLUTs50`.
- **5-shot auto-bracketing**: Stock `0xA0168100` already returns `5` for `BktOrd` values `4, 6, 7` (stored at `0xA09396DF`). Hooking `0xA0002FE4` (`Get_Bkt_EV_Q16`) computes signed Q16 APEX offsets using `((i32*)0xA0176F14)[*(u16*)0xA07FBE5A]`.
- **ISO Auto Min Shutter**: Hooks A-mode `0xA000D83E` (stock `0x0005AAAA` ≈ 1/50.8 s) and P-mode `0xA000D9A2` / `0xA000D9E8` (stock `0x00090000`, overridden to `Tv_min + 3.0 EV`).
- **Accelerometer & OSD**: MEMSIC `MXC6226XC` (`I2C 0x15`) signed 8-bit X/Y getters at `0xA001A38E` / `0xA001A3A2`; `640×480` ARGB4444 overlay framebuffer at `0xA21D8C00` (flushed via `0xA0041C7A`; filled via `0xA00419A6`); hidden `Imager Temp` screen at menu ID `0xA3` (`0xA07EDC0C`).

### 3.5 M6M -> M5Slave Capture IPC Wire Format (`cmd 0x20`)

Stock `0xA0035FCE` clamps `meta[+0x1E]` to `0..7` and `meta[+0x1D]` to `0..1` and zeroes `cmd[5]` / `cmd[6]`. Hook `0xA00360A2` (`src/asm/m6/ipc_capture.S`) preserves live `r1` (`1.0f`) and packs:
- `cmd[5]`: full 8-bit `packed_cm` (`1..198`):
  - `1..8`: stock modes
  - `9..13`: Quattro modes (FOV Classic Yellow, Cinema, Sunset Red, Forest Green, Teal & Orange)
  - `14..18`: 5 layer-direct optical filter monochrome modes
  - `19..22`: 4 reversal films
  - `23..150`: `23 + neg_idx * 8 + paper_idx` (16 negative films × 8 print papers)
  - `151..190`: `151 + bw_idx * 8 + paper_idx` (5 B&W stocks × 8 darkroom papers/toners)
  - `191..198`: SD-card custom 3D LUTs (`D:\LUT1.BIN..D:\LUT8.BIN`)
- `cmd[6]`: `(fill << 5) | (fx << 2) | cs` (`cs`: `1=sRGB, 2=AdobeRGB`; `fx`: `0..4`; `fill`: `0..3`).

---

- **Updater display strings**: Stock `0xA005FC3C` (`M6M5 Both Debug Write.`), `0xA005FC5C` (`M6M Only Debug Write.`), and `0xA005FC7C` (`M5 Only Debug Write.`) live in 24-byte NUL-padded slots; patched in-place to `dp2m-fw (M6M + M5)` / `dp2m-fw (M6M)` / `dp2m-fw (M5)`.

---

## 4. Optimization Notes for Future Work

1. **32-bit word-packed Y loads/stores in `src/asm/m5/ycc_engine.fr80`**: Because M5Slave D-cache is disabled, uncached byte loads/stores dominate wall-clock latency. Loading 4 horizontal Y bytes with one aligned 32-bit `ld` and writing back one 32-bit `st` per `4×2` tile row (when `Y_row & 3 == 0` and `y_stride & 3 == 0`) cuts Y-plane SDRAM bus transactions by 4×.
2. **Coarse `8×4` tile coherence for `FX=1`**: Extending the fast smooth-tile check to `8×4` (with fallback to `4×2` then per-pair) halves Sakamoto evaluations across smooth sky/wall gradients.
3. **Live View paper-change sync**: `VA_CMCM_PACKED_30` currently stores each negative film's configured `:paper` CMCM (set via `dp2m.fnl`). Adding a runtime paper lookup table in Cave 2S would let toggling `Print Paper` on-camera update Live View dynamically for any paper as well.
   *(Halation exact row zero-skip `OR(row0 | row1) < 2 -> skip bilerp_2d` is already implemented in `ycc_engine.fr80`.)*
