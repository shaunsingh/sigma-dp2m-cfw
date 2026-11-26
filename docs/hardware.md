# Sigma DP2 Merrill (C78) hardware and firmware reference

Findings from the stock `DP2MV105.bin` (1.05, May 2013) and tests on the camera. Detailed evidence and addresses are in [research/](research/).

| Chip | Image | CPU | Role |
| --- | --- | --- | --- |
| M6M | `C78FRM.bin` | ARM926EJ-S, big endian | UI, settings, live view, capture control |
| M5Slave | `Firm.bin` | Fujitsu FR80 ("TRUE II"), big endian | capture ISP, JPEG and X3F pipeline |

## 1. Firmware package and updater

`DP2MV105.bin` (25,968,896 B):

| Offset | Length | Contents |
| --- | --- | --- |
| `0x00000000` | `0x80` | header: magic `SIGMA.CODP2M`, version strings at `0x10` / `0x30` / `0x40`, date `0x50`, LE u32 final-M6M offset `0x64`, stage flags `0x70` |
| `0x00000080` | `0x00820000` | bridge M6M image (kept stock) |
| `0x00820080` | `0x00884080` | M5Slave (`Firm.bin`, magic `SIGMA.CODSLRSD15`, `VA = file_off + 0x3BF80`) |
| `0x010A4100` | `0x00820000` | M6M (`C78FRM.bin`, magic `FUJITSU!`: `0x20000` IPL + `0x800000` S3 mapped at `0xA0000000`) |

- Stage flags `0x31`: the running firmware flashes the bridge (bit 0) and reboots; the bridge flashes the M5 (bit 5) and the final M6M (bit 4).
- The version at `0x10` is parsed as `major.minor` and must be at least 1.05. The update screen prints the new version from the header and "Current version" from immediates in the running M6M (`0xA005F87C`).
- dp2m writes `DP2MV<major><minor>.bin` (default `DP2MV200.bin`) and patches the "Current version" immediates and the debug banners (`0xA005FC3C`, `0xA005FC5C`, `0xA005FC7C`) in its own M6M.
- `Firm.bin`, `C78FRM.bin` (and the stock name `C78_M6M5.bin`) in the card root are debug updates: no version check, the screen shows a banner and the file timestamp.
- Error **`0002`**: `Firm.bin` has non-stock bytes at `VA >= 0x00800000` (outside the 8 MB NOR) or the checksum at `Firm.bin[0x4C..0x50]` does not equal `sum(Firm.bin[0x80..0x800080])`. If the screen locks, remove the battery; the camera boots its previous image.

See [research/updater.md](research/updater.md).

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
- Bank A layout (`crates/dp2m/src/m5.rs`):

| VA | Content |
| --- | --- |
| `0x005D0000..0x005D03C0` | hook stubs (table in 2.4) |
| `0x005D03C0` | YCC engine (`src/asm/m5/ycc_engine.fr80`) |
| `0x005D1600` | spatial driver (`src/asm/m5/spatial_driver.fr80`) |
| `0x005D2000` / `0x005D2100` | halation LUT / grain envelopes |
| `0x005D2400` / `0x005D2800` / `0x005D2900` | per-mode meta / mono table / MCAS identity |
| `0x005D2B00` / `0x005D2C00` | SD LUT path template / clamp LUT |
| `0x005D3000` | CMCM table |
| `0x005D5000` / `0x005E7000` | MCAS sRGB / AdobeRGB |
| `0x005F9000` / `0x006B7000` | 1D LUTs sRGB / AdobeRGB |
| `0x00775000` | 40 B&W toners |
| `0x0077A000` | 132 sRGB 3D LUTs (`LUT3D_STRIDE = 0x1028`) |

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

The two EXIF CS hooks at `0x005147C4` / `0x00514680` sit in SD15-era maker note writers (`0x513530..0x5145FA`, `0x514AD8`) with no static references; the M6M writes the file metadata (3.8).

Additional verified stock M5Slave addresses:
- `0x8000CC44`: live active Foveon Q7 signed 12-bit CCMatrix (9 × `u16 BE`, sign bit `0x0800`). Committed via `0x004B1F20(struct_ptr)` where `struct[0..2] = 0x0200`.
- `0x80001B88`: live ColorSpace RAM flag (`1 = sRGB`, `2 = AdobeRGB`).
- `0x80012E46`: 18-byte SDRAM shadow of the live sRGB/AdobeRGB primary matrix (maintained by `0x4B20A6`). Always copy from `0x80012E46` rather than MMIO `0x40100500` (byte-read hazard on MMIO).
- `0x0054CC94` / `0x00552C94`: stock 4096-byte Standard / Monochrome 1D tone LUTs (used to invert DP2M's native linear exposure scale).
- `0x00571368` (32 B `MCB`) / `0x0057139C` (20 B `MCID`): stock MCAS sector boundary & ID tables (`0x1AE`-byte MCAS struct committed via `0x004B2088(1, buf)`).
- `0x0048AB80` (`MDF_W_open`), `0x0048ABAA` (`MDF_W_close`), `0x0048ABD0` (`MDF_W_read`): native FAT32 SD I/O wrappers used to hot-load `D:\LUT1.BIN..D:\LUT8.BIN`.
- `0x004DD702(job_id)` / `0x004DD72A(job_id)`: per-job `packed_cm` (`slot+0x150`) and `packed_1d` (`slot+0x14C`) getters. Stock modes `1..8` translate to native M5Slave enum (`3=Standard, 1=Sepia, 2=Mono`), so `ipc_unpack` only overrides `slot+0x150` when `cmd[5] >= 9`.

### 2.5 Color Pipeline & `D2ML` SD-Card 3D LUT Format

- **Dual-colorspace 17×9×9 Q1 Sakamoto 3D residual LUT**: `17` luma (`step 16`) × `9` Cb (`step 32`) × `9` Cr (`step 32`) × `3` signed `i8` (`0.5`-LSB Q1 deltas) = `4,131` B (`LUT3D_STRIDE = 0x1028`). The engine reads them biased (`s8 ^ 0x80`, folding `-128 × 32` into the rounding term) to skip sign extension: the ROM copies are stored biased, and the driver biases a `D2ML` file in place after loading it.
- **`D2ML` SD-card binary (`4,160` B = `0x1040`, Big-Endian)**:
  - `0x0000` (4 B): `"D2ML"` (`0x44324D4C`)
  - `0x0004` (1 B): version `0x01`
  - `0x0005` (1 B): `grain_amp` (`0..64`)
  - `0x0006` (1 B): `grain_profile` (`0=Neg, 1=Slide, 2=B&W`)
  - `0x0007` (1 B): `halo_gain` (`0..255`)
  - `0x0008` (1 B): `halo_tint` (`0=Crimson, 1=Neutral B&W, 2=Warm Amber`)
  - `0x0009` (7 B): ASCII tag (zero-padded)
  - `0x0010` (4,131 B): `17×9×9×3` Q1 `i8` residual LUT, zero-padded to `0x1040`.
- **Measured cost** in the FR80 emulator (`ycc_bench`) on synthetic frames: 256x128 (mostly smooth) and a 4704x8 strip (steep rows, hard edges, a noisy quarter, so most `4x2` tiles fall back to per-pair evaluation). L time assumes about 70 M effective FR80 instructions/s with uncached SDRAM; M ≈ 0.48 L, S ≈ 0.25 L. `cargo test --release --offline -p dp2m ycc_bench -- --ignored --nocapture` prints every fx × fill combination.

| Mode / `fx` | Stages | insns / pair (256x128) | L | insns / pair (4704x8) | L |
| --- | --- | --- | --- | --- | --- |
| `fx=0` | stock HW path | 0 | 0 | 0 | 0 |
| B&W `fx=0/1` | B&W toner (read-only Y) | 24.6 | ~2.6 s | 24.5 | ~2.6 s |
| `fill=1..3 fx=0` | X3 Fill only (read-only Cb/Cr) | 39.5 | ~4.2 s | 39.3 | ~4.1 s |
| `fx=1` | 3D LUT | 76.8 | ~8.1 s | 77.9 | ~8.2 s |
| `fx=3` | 3D LUT + grain | 132.7 | ~14.0 s | 135.3 | ~14.3 s |
| `fx=5` | 3D LUT + halation | 156.7 | ~16.5 s | 143.6 | ~15.1 s |
| `fx=7` | 3D LUT + grain + halation | 202.3 | ~21.3 s | 188.2 | ~19.8 s |

---

## 3. M6M (`C78FRM.bin`, ARM926EJ-S big endian)

### 3.1 Memory map

S3 runs from SDRAM at `0xA0000000` (256 MB SDRAM), so code and tables in the image are writable at runtime.

| Range | Use |
| --- | --- |
| `0xA0000000..0xA0800000` | S3 image |
| `0xA07FBC1C..0xA0A1F398` | zero-init RAM (scatter table `0xA07AD3CC`); the image bytes from `0xA07FBC1C` are the exception vector source and are cleared at boot |
| `0xA0939684..0xA0939ACF` | NVRAM settings block (3.4) |
| `0xA0A1F398..0xA1B1F398` | heap |
| `0xA1B1F3A0..0xA1B5717C` | µITRON task stacks |
| `0xA1B57180..0xA2130000` | no references; payload `.bss` at `0xA1C00000..0xA1E00000` |
| `0xA2130000..` | OSD: 8 bpp index plane `0xA2130000` (icons, text), RGBA4444 overlay `0xA21D8C00` (`R:12..15, G:8..11, B:4..7, A:0..3`, 640x480); LV and capture buffers |

See [research/m6m-memory.md](research/m6m-memory.md).

### 3.2 Protected regions

The builder checks these against the stock image after patching:

| Range | Reason |
| --- | --- |
| IPL, `C78FRM.bin[0..0x20000]` | boot loader |
| `0xA0179000..0xA023D35A` | Spartan-6 FPGA bitstream (803,674 B, length at `0xA07AE444`); any change breaks FPGA configuration (white screen, dead shutter) |
| `0xA07FBC1C..0xA0800000` | vector source, cleared at boot |
| word `0xA004D3D0` (`0xA07EEFCC`) | literal shared with other code; hook `Get_Menu_Table` instead |
| `0xA005D924..0xA005F1A0` | live screen code (ids 50..55) |
| `0xA07DEBF0..0xA07DEC0B` | their vtable slots |

Never write NVRAM `0xA0939774..0xA0939783`: language is at `0xA0939776` and LCD settings at `0xA0939778`. A counter written there cycles the UI language on every menu exit.

### 3.3 Payload and hooks

The M6M payload is C and assembly in `src/m6m/`, compiled with `clang --target=armeb-none-eabi -mcpu=arm926ej-s -mthumb` (`-O2` on `rt.c`, `hud.c`, `lv.c`; `-Os` elsewhere, with ARM interworking for word-wide loops) and linked with `ld.lld --gc-sections` into dead regions of the stock image ([research/m6m-rom.md](research/m6m-rom.md)):

| Region | Range | Size | Sections | Origin |
| --- | --- | --- | --- | --- |
| `CODE` | `0xA0059D50..0xA005D924` | 15,316 B | `.text` | factory screens 33..49 |
| `DATA` | `0xA0265670..0xA0268870` | 12,800 B | `.rodata`, `.data` | f64 table used only by orphaned code |
| `LOOKS` | `0xA0094658..0xA009688C` | 8,756 B | `.lv` | orphaned Im_TFR driver |
| `MENU` | `0xA07EC9CC..0xA07EDC0C` | 4,672 B | `.menu` | factory service menus 0x18..0x22 |
| `RAM` | `0xA1C00000..0xA1E00000` | 2 MB | `.bss`, not cleared at boot | unreferenced SDRAM |

- `REGIONS` in `crates/dp2m/src/m6m/mod.rs` generates the linker `MEMORY` block (`out/m6m/regions.ld`); `src/m6m/m6m.ld` maps sections to regions. The builder rejects sections outside them.
- The vtable slots of screens 33..49 (`0xA07DEBAC..0xA07DEBEF`) are set to `dp2m_ret0`, after checking that each still points into `CODE`.
- Stock functions are absolute symbols in `src/m6m/stock.S` with bit 0 set for Thumb, so lld picks BL or BLX.
- `crates/dp2m/src/m6m/codegen.rs` generates `catalog.c` / `catalog.h`: strings, menu tables, the film table and the live-view looks.

Every patch to stock code is a 16-byte record `{kind, site, target, expect}` (BE) in the non-allocated section `.dp2m.hooks`, written with the `HOOK_*` macros (`src/m6m/dp2m.h`) or the `hook` assembler macro (`src/m6m/hooks.S`). The builder checks `expect` against the stock bytes, fails on a mismatch, then applies:

| Kind | Effect | `expect` |
| --- | --- | --- |
| `HOOK_CALL` (1) | retarget the Thumb BL/BLX at `site` to `target` | original callee |
| `HOOK_BL` (2) | write a Thumb BL to `target` over 4 bytes | stock bytes |
| `HOOK_WORD` (3) | write a 32-bit value | stock word |
| `HOOK_HALF` (4) | write a 16-bit value | stock halfword |

Thumb BL reaches ±4 MB. Sources: `ui.c` (settings, menus, strings, IPC packing, Film page), `custom.c` (F1/F2), `lv.c` (live-view color), `hud.c` (overlays), `hooks.S` (register-exact trampolines), `rt.c` (`memcpy`, `memset`).

### 3.4 NVRAM

Block `0xA0939684`: `0x44C` B in RAM, initialised from ROM defaults `0xA07F9E68`. The first `0x3C4` B persist to NOR bank 0 at offset `0x20` (flush `0xA004017A`, copy `0xA01674FA`, write `0xA00206FA`). Only `+0x04..+0x13` is reserved (zero defaults, no stock accessor); `+0x14..+0x16` are stock display levels.

| Address | Setting | Values |
| --- | --- | --- |
| `0xA0939688` | color print paper | 0 = film's Auto paper, 1..8 |
| `0xA0939689` | 3D spectral engine | 0/1 (fx bit 0) |
| `0xA093968A` | X3 fill | 0..3 |
| `0xA093968B` | ISO-auto min shutter | 0..6 |
| `0xA093968C` | peaking color | 0 red, 1 yellow, 2 white, 3 black |
| `0xA093968D` | grain | 0/1 (fx bit 1) |
| `0xA093968E` | halation | 0/1 (fx bit 2) |
| `0xA093968F` | B&W paper | 0 = Auto, 1..8 |
| `0xA0939690` | SD LUT slot | 0 = Auto, 1..8 |

Stock fields used: exposure mode `0xA09396AC` (`+0x28`; 0..3 M/S/A/P, 4 Movie, 5..7 C1..C3, 8 temporary, 9/10 F1/F2), shooting settings `0xA09396C0` (`+0x3C`, `0x8C` B), color mode `0xA09396C3`, C1..C3 slots `0xA0939818`, backup `0xA09399BC`.

The 2 KB NOR record uses `0x00..0x3E3`; `0x3E4..0x7FF` has no accessor. F1/F2 live in payload `.bss` and persist at `0x400..0x51B` (magic `"C4C5"` + 2 x `0x8C`) through an extended flush write and boot load. See [research/m6m-memory.md](research/m6m-memory.md) and [research/custom-modes.md](research/custom-modes.md).

### 3.5 UI

- `Get_Menu_Table` `0xA004CD08(id)`. Rows are 32 B: w0 row id (submenu id on page rows), w1 `0x01000000 | 110x70 QS icon`, w2 `(76x26 OSD icon << 16) | 54x42 strip icon`, w3 `sid << 16`, w4 value, w5 getter `| 1`, w6 setter `| 1`, w7 extra handler. Pages clip beyond 6 rows.
- Strings: `0xA0044AAC(sid, lang)` = `*(0xA07DF144 + sid * 0x44 + lang * 4)`, 672 stock sids. dp2m serves sids from 672 (English).
- Screens: handler `*(0xA07DEB28 + 4 * id)`, opened with `0xA0042AEC(1, id)`. LV 21 (`0x15`), MODE 23 (`0x17`), PB 25 (`0x19`), QS 26 (`0x1A`), MENU 27 (`0x1B`), Custom Mode Setting 31 (`0x1F`). Native AE, µITRON, Milbeaut IIP/DMA and `SCREEN_PB` details are in [research/simulator-and-pb.md](research/simulator-and-pb.md).
- QS commits the setter on every step and draws only icons (w1 low for the value, w2 low in the strip). The main menu commits on OK or right and shows the w2-low icon, or the sid text when it is 0.
- Icons: `0xA0042AAC(id)` for ids 1..0x1A1 (table `0xA07DD784`); 8 bpp indices 0..16 (0 transparent, 1 white fill, 16 black outline), blitted by `0xA0040DD8` into the index plane `0xA2130000` (CLUT `0xA07DEC54`).
- Text: `0xA0042684`, engine 0 ("BP4" outline fonts) only, line buffer `0xA093A514`, about 1 M instructions per label.
- Film page: inserted as menu page 2 in the page template and live list (menu state `0xA07F22D2 + 0x14` / `+ 0x3C`; menu entry copies the template, now 11 pages); page-to-tab-group `0xA0058994` is hooked; tab geometry `0xA04F3100` (stock 56 px per tab, 52 px with 11 pages).
- File color mode: the M6M writes the Sigma maker note for JPG and X3F (`0xA0148442`). Its color mode switch (`0xA0148CF8`, cm at ctx `+0x8E`) writes tag 0x3D (11 B, `0xA07F7D78`) and tag 0x59 (15 B, `0xA07F8320`) and defaults to `"Standard"`. Playback parses tag 0x3D by prefix (`strncmp`) into info `+0x157` (JPG `0xA0147938`, X3F `0xA0132E9A`; unknown → 0). dp2m writes `"Film NN"` (NN = cm - 9) and the film name for films and maps `"Film NN"` back in both parsers; stock modes are unchanged.

See [research/m6m-ui.md](research/m6m-ui.md).

### 3.6 Live view

- Refresh `0xA003A180`: gamma only on an LV mode change; the mono routine (`0xA003A1E6`) on a mode change or when the color-dirty flag (`0xA07B1398 + 2`, set by `0xA003A260`) is set; the color matrix (`0xA003A20A`) on every call. dp2m re-applies the film tone curve from the mono call site, so curves follow mode and paper changes live.
- **Gamma** `0xA009D940` (`Im_B2Y_Set_Gamma_Table`): call with `ch = 0` and keep `ctrl_ptr[1] = 0` (`MMIO[0x60020700] = 0x0800`). `0x0801` (GMACTL bit 0) causes highlight speckling and blotchy banding.
- **Color matrix** `0xA009C334` (`Im_B2Y_Set_Color_Matrix`): set BT.601 Y weights `{0x004D, 0x0096, 0x001D}` at `m[9..11]` and `m[12] = 2 << 8`. Normalize film CMCM rows to unit mean gain (1024) to keep the 10-bit B2Y SNR, and evaluate the 18-knot tone curve at `x_lin * gain`.

See [research/live-view.md](research/live-view.md).

### 3.7 Backports and peripherals

- **ISO 50**: payload copies of the ISO menus `0xA07EAC8C` (1 EV) and `0xA07EADAC` (1/3 EV) with an ISO 50 row (value 1, sid 53), swapped in through pool words `0xA004D3DC` / `0xA004D3E0`. The clamps at `0xA003F2CC` and `0xA003F30A` let value 1 through to the M5 (`LinLUTs50`).
- **5-shot bracketing**: stock `0xA0168100` already returns 5 shots for bracket orders 4, 6 and 7 (stored at `0xA09396DF`). The hook in `Get_Bkt_EV_Q16` (`0xA0002FE4`) returns Q16 APEX offsets in steps of `((i32 *)0xA0176F14)[*(u16 *)0xA07FBE5A]`: order 4 `[0, +2, +1, -1, -2]`, 6 `[+2, +1, 0, -1, -2]`, 7 `[-2, -1, 0, +1, +2]`.
- **ISO-auto min shutter**: A mode `0xA000D83E` (stock `0x0005AAAA`, about 1/50.8 s) uses the configured APEX Tv; P mode `0xA000D9A2` / `0xA000D9E8` (stock `0x00090000`) uses that Tv + 3 EV.
- **Accelerometer**: MEMSIC MXC6226XC (I2C `0x15`), signed 8-bit X / Y getters `0xA001A38E` / `0xA001A3A2`.
- **Overlay**: 640x480 BE RGBA4444 (`R:12..15, G:8..11, B:4..7, A:0..3`) at `0xA21D8C00`, fill `0xA00419A6` (uses Milbeaut 2D DMA `0xA00B1870` when `rgba == 0`, `x & 7 == 0` and `w & 15 == 0`), flush `0xA0041C7A`.
- **Imager Temp**: hidden screen at menu 0xA3 (`0xA07EDC0C`), linked from setup page 4.

### 3.8 Capture IPC (`cmd 0x20`)

Stock `0xA0035FCE` remaps `meta[+0x1E]` into the low nibble of `cmd[8]` (table `0xA0035FFA`: 3→0, 4→1, 5→2, 6→3, 7→4, 2→5, 1→6, 8→7, else 0), clamps `meta[+0x1D]` to `0..1` and zeroes `cmd[5]` / `cmd[6]`. The hook at `0xA00360A2` (`ipc_hook` in `src/m6m/hooks.S`, `ipc_pack` in `src/m6m/ui.c`) keeps the live `r1` (`1.0f`) and packs:
- `cmd[5]`: full 8-bit `packed_cm` (`1..198`):
  - `1..8`: stock modes
  - `9..13`: Quattro modes (FOV Classic Yellow, Cinema, Sunset Red, Forest Green, Teal & Orange)
  - `14..18`: 5 layer-direct optical filter monochrome modes
  - `19..22`: 4 reversal films
  - `23..150`: `23 + neg_idx * 8 + paper_idx` (16 negative films × 8 print papers)
  - `151..190`: `151 + bw_idx * 8 + paper_idx` (5 B&W stocks × 8 darkroom papers/toners)
  - `191..198`: SD-card custom 3D LUTs (`D:\LUT1.BIN..D:\LUT8.BIN`)
- `cmd[6]`: `(fill << 5) | (fx << 2) | cs` (`cs`: `1=sRGB, 2=AdobeRGB`; `fill`: `0..3`), stored by M5Slave `ipc_unpack` in `slot+0x14C`. `fx` holds three independent toggles, all 8 combinations valid:
  - bit 0: 3D LUT engine (`19..150` film LUTs, `191..198` SD LUTs; ignored by other modes)
  - bit 1: film grain
  - bit 2: halation (needs the mode's `halo_gain > 0`)
  - Grain and halation also run on the HW color path (bit 0 clear). B&W toners (`151..190`) and X3 Fill always apply. SD-LUT modes load `D:\LUTn.BIN` whenever any `fx` bit is set (its header supplies grain/halation params). With no stage active, the hook falls through to the stock HW JPEG path untouched.

The packed mode follows `packed_mode` in `crates/dp2m/src/catalog.rs` (film order defines the on-camera mode ids `9 + index`) and `packed_cm` in `src/m6m/ui.c`.

---

## 4. Open work

Verified in emulation (`dp2m build`, `python -m tools.verify`, `dp2m-sim`); on-camera validation remains:

- **On-camera checks**: full `DP2MV200.bin` install from 1.05 and reinstall of stock 1.05; runtime label icons (QS, OSD, MODE, LV status, playback info); film name in playback info for new JPG and X3F files, and how Sigma Photo Pro treats `"Film NN"` X3F files; Custom QS film items in each slot; F1/F2 slot persistence across power cycles; payload RAM window `0xA1C00000..0xA1E00000` under LV, capture, playback, movie and USB; M5 engine wall time per fx combination.

### M5 performance notes

- **`ycc_engine.fr80` (`4,088 B`, fits inside the 4 KB FR80 I-Cache)**:
  - Biased 3D LUT bytes (`s8 ^ 0x80`, 12 instructions per Sakamoto evaluation) and SWAR 4-byte saturating luma adds (`x - e = ~(~x + e)`).
  - Universal `4x2` tile fast path (`tile_row`) active across all `fill = 0..3`, `fx = 0..7` whenever `Y`/`Cb`/`Cr` and strides are word/halfword-aligned and the tile's halation row check `(row0 | row1) < 2` passes, with signed `[X00 - 15, X00 + 16]` luma/chroma coherence (`94%..99%` smooth-tile rate on DP2M shots), `call.far` subroutines for X3 Fill / B&W toner / grain, and halfword `lduh`/`sth` chroma pairs.
  - Dedicated word-strided Paths **T** (B&W toner `fx=0/1`, read-only `Y`, `24.5` insns/pair) and **F** (`fill=1..3 fx=0`, read-only `Cb/Cr`, `39.3` insns/pair).
- **`spatial_driver.fr80` (`2,202 B`)**: specialized `pyr_halo_only`, `pyr_fill_only`, and `pyr_bx` box-downsample passes, register-history in-place `blur_5tap`, SWAR `hblend`, and word-copied SD LUT path (`4704x3136` pyramid: `1.17 M` instructions).
- **Rejected**: `8x4` block coherence (max `|Δ|` reached 15 levels vs `4x2` and regressed steep strips) and unverified `mulh`.
