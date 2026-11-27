# Unified firmware, silicon alignment & emulator hill climb

Baseline: local snapshot `30a661c3b44dc1de2141ffbfed1a15e8eff140cc`. All five client-firmware and tooling tracks (`M5 FR80`, `M6M HUD/labels`, `Quattro/fp backports`, `ARM9 DOOM`, and `emulator/verify`) are unified into `dp2m-fw` with strict real-hardware memory, cache-coherence, and bit-exactness checks.

## 1. Real-hardware alignment & silicon invariants

- **M6M CP15 MPU & `SWI 10` (`0xA07FBE64`) D-cache coherence**:
  - Stock MPU init (`0xA01720F0..0xA01721EC`) enables the 16 KB 4-way D-cache and write buffer across `0xA0000000..0xAFFFFFFF` (Region 2, including `OSD` `0xA2130000`, `ARGB_FB` `0xA21D8C00`, and `lv_planes`) plus I-cache across `0xA0000000..0xA1FFFFFF` (Region 3, covering S3, payload regions, and `.bss` `RAM` `0xA1C00000..0xA1E00000`).
  - ITCM `SWI 10` (`0x00000248`, copied from `0xA07FBE64`) loops `4 ways × 128 sets` (`mcr p15, 0, r2, c7, c10, 2`, clean by index), drains the write buffer (`c7, c10, 4`), and invalidates the whole D-cache (`c7, c6, 0`); the I-cache is untouched.
  - `hw_dcache_sync()` (`svc #10` with `"r0", "lr", "cc", "memory"` clobbers) is placed at every CPU/Milbeaut-2D-DMA handoff (`clear_dma`, end of `grid_draw`, cold `label_icon` tile creation before `stock_blit_bp4`, and `doom_present` before `stock_argb_commit`).
- **Spartan-6 XC6SLX25 FPGA (`0xA0179000..0xA023D35A`)**:
  - Identified as `XC6SLX25` (`IDCODE = 0x04004093`, `803,674 B`, `COR1 = 0x3D00`), loaded over 20 MHz CSI/SelectMAP on every boot (`FPGA.c` @ `0xA0178E7C`). Sits solely on Foveon F20 sensor timing, AFE calibration, dark-frame accumulation, and raw readout (`0xAF64D7C0..0xAF6507C0`); kept 100% protected by build-time byte-exact verification.
- **M5 FR80 SDRAM scratch (`0x80100000..0x8010C800`)**:
  - Full `ldi:32` and table scan of stock `Firm.bin` confirmed zero overlap with stock `.bss`/RTOS pools/stack (`<= 0x800F4C60`), static capture buffers (`0x801D0000+`), HW JPEG (`0x8050E000+`), or concurrent background EXIF `THUMB_Y/CB/CR` (`0x81046580+`).
- **Menu `w7` (`extra`) & executable-region invariants (`0xA0058798`)**:
  - Stock menu dispatcher invokes non-zero `row->w7` (`+0x1C`) directly via `blx r0` at `0xA0058798` (`0` on value rows, `doom_open | 1` on the DOOM launcher row), and executable ELF sections (`SHF_EXECINSTR`) are barred at build time from `0xA0070000..0xA00B8000` (`LOOKS` overlaps `Camera`'s HLE driver stub range).

## 2. Client firmware hillclimb results

### M5 FR80 (`Firm.bin`, checksum `0x32b2fe61`)
- **`ycc_engine.fr80`**: `4,028 B` (`-50.0%` from initial `8,064 B`, fits inside the 4 KB FR80 I-cache) via split-tile `(X00, X01)` Sakamoto reuse on `4×2` fallback tiles and literal-free exact-key miss handling.
- **`spatial_driver.fr80`**: `2,470 B`; row-major `15×17×3` pyramid accumulation cuts `4704×3136` pyramid setup from `1,152,480` to **`755,156` instructions (`-34.5%`)**.
- **Bit-exact parity**: **0 differing bytes** across all synthetic stress fixtures (`DP2M_BENCH_STRESS=1 DP2M_REF_EXACT=1`, 423 runs) and across extracted real camera captures (`DP2M_BENCH_YCC=/tmp/dp2m-sd-ycc`, 14 DP2M SD-card JPEGs).

### M6M ARM926EJ-S BE (`C78FRM.bin`)
- **All 9 payload regions fit with headroom** (both default `24/36` films and full `36/36` films + `11/11` grids + `DOOM`):
  - `CODE`: `15,112 / 15,316 B` | `DATA`: `12,468 / 12,800 B` | `LOOKS`: `7,476 / 8,756 B` | `MENU`: `3,572 / 4,672 B` (`3,956 B` w/ 36 films)
  - `DOOM_A`: `4,796 / 4,876 B` | `DOOM_B`: `4,224 / 4,668 B` | `DOOM_C`: `2,212 / 2,492 B` | `DOOM_D`: `1,252 / 2,304 B` | `DOOM_E`: `3,168 / 3,500 B`
- **Live-view overlays (`m6m_bench`, `draw only` ARM9 instructions)**:
  - `grid 7` Electronic Level (Standard / Quattro Minimal): **`11,129`**
  - `grid 8` 21:9 + Level: **`30,059`** | `grid 10` 1:1 Mask: **`65,262`**
  - `grid 11` Focus Peaking (runtime `24/44/72/Auto MF`, zero-XOR 2-row-pair SWAR): **`626,068`** (`default`), **`368,622`** (`checker`/`white`, up to `2.02×` faster)
  - `grid 13` Zebras (runtime `90%/95%/100%/Dual Hi+Lo`, precomputed `stmia` stripe table): **`150,312`** (`default`), **`241,726`** (`white`, `2.64×` faster)
  - `grid 14` RGB Histogram (`ldmia` + register bins): **`155,313`** (`default`), **`150,437`** (`checker`/`white`)
  - `grid 17` False Color (16-px `ldmia` IRE pass + legend): **`331,438`** (`default`, `2.00×` faster), **`458,094`** (`checker`/`white`, `1.73×` faster)
- **Custom label renderer (`labels.c`)**: **`42` instructions** cached (`O(1)` `(class, sid)` table), **`271k..3.11M`** cold (`~4.4×` faster than stock BP4 text engine).
- **Bare-metal DOOM (`SCREEN_DOOM`)**: 2× unrolled vertical wall column mapper + word-aligned 32-bit BE flat span stores + 8-pixel `ldmia`/`stmia` `2×2` `RGBA4444` upscaler: **`1.99M..2.06M` ARM9 insns/tick** on E1M1 (`69` unique colors, clean exit to `SCREEN_MENU` / `SCREEN_LV`).

## 3. Emulator, native verifier & reproduce commands

- **Decoded I-cache** in `crates/dp2m-emu/src/arm9.rs` (`240.7 MIPS` overlay / `479.0 MIPS` label host rate; `m6m_bench` wall time **`0.73 s`**) and `crates/dp2m-emu/src/fr80/emu.rs`, with automatic write invalidation across `w8`/`w16`/`w32`.
- **`dp2m verify out --artifacts <dir>`**: **506 / 506** checks (`m6m: 241`, `custom: 130`, `labels: 135`) in **~0.38–0.44 s** (`~17×` faster than Python/Unicorn) plus native zero-crate `labels.png` contact-sheet emission; `python3 -m tools.verify out` passes all **506 / 506** checks with bit-exact Unicorn OSD parity (`FNV1a64 = 0x16a06c94433a64d3`).

```sh
nix develop --no-write-lock-file -c sh -c '
  export CARGO_BUILD_JOBS=4
  cargo run --release --offline -p dp2m -- build
  cargo run --release --offline -p dp2m -- verify out --artifacts /tmp/dp2m-artifacts
  cargo test --release --offline --workspace
  cargo test --release --offline -p dp2m m6m_bench -- --ignored --nocapture
  python3 -m tools.verify out
'
```
