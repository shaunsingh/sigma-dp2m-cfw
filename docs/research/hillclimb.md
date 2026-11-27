# Build metrics & emulator benchmarks

All performance numbers below were measured in the built-in ARM9 and FR80 emulators (`dp2m-emu`). Build `a441ed3` was installed on a DP2 Merrill and used by hand (film captures with the expected MakerNote tags, menus, live-view looks, DOOM). Nothing has been measured on the camera.

## 1. Hardware & build invariants

- **M6M CP15 MPU & cache SWIs (`0xA07FBE64` / `0xA07FBECC`)**: Stock MPU init (`0xA01720F0..0xA01721EC`) enables the 16 KB 4-way D-cache and write buffer over `0xA0000000..0xAFFFFFFF` (Region 2: OSD `0xA2130000`, `ARGB_FB` `0xA21D8C00`, `lv_planes`) plus I-cache over `0xA0000000..0xA1FFFFFF` (Region 3). ITCM `SWI 10` (`0x00000248`) cleans all 4 ways × 128 sets by index (`c7, c10, 2`), drains the write buffer (`c7, c10, 4`), invalidates the D-cache (`c7, c6, 0`), leaves the I-cache untouched, and restores `r0`/`r1`/`r2`. `hw_dcache_sync()` (`src/m6m/dp2m.h`) brackets full-frame CPU/Milbeaut-2D-DMA handoffs in `hud.c` and `doom.c`. Newly rendered label tiles instead use stock `SWI 12` through `hw_dcache_clean_range()`, which aligns and extends the range, cleans only those lines by address, and drains the write buffer without a whole-cache invalidate.
- **Spartan-6 XC6SLX25 FPGA (`0xA0179000..0xA023D35A`)**: `803,674 B` (`IDCODE = 0x04004093`, `COR1 = 0x3D00`), verified byte-exact stock at build time.
- **M5 FR80 SDRAM scratch (`0x80100000..0x8010C800`)**: verified zero-collision against `PREVIEW_Y` (`0x80FB0580`), `THUMB_Y/CB/CR` (`0x81046580`), and RAW/RGB buffers (`0x8104B590+`) in `verify-m5`.
- **Executable-region check**: `build_m6m` bars `SHF_EXECINSTR` sections from `0xA0070000..0xA00B8000` (simulator HLE driver stub range).

## 2. Current build sizes & emulator measurements

Default config (`dp2m.example.fnl`: `24/36` films, `11/11` grids, DOOM enabled):

### M5Slave FR80 (`Firm.bin`, checksum `0x32b300c0`)
- **Code sizes**: `matrix=242 B`, `lutsel=56 B`, `mcas=140 B`, `ipc/exif=90 B`, `ycc_engine=3,974 B` (fits inside the 4 KB FR80 I-cache), `spatial_driver=2,494 B`.
- **`dp2m verify-m5`**: 198 modes × 2 CS, signed CCMatrix, IPC/EXIF, `4704×3136` pyramid (`754,794` emulated insns), `4704×8` `fx 0..7` (`fx=7 fill=1`: `144` insns/pair, `18` SDRAM/pair), 256 `fx × fill × CS` stage runs, zero-collision `0x80100000`.
- **`ycc_bench` (FR80 emulator)**: see [../hardware.md](../hardware.md#25-color-pipeline--d2ml-sd-card-3d-lut-format) for per-stage insns/pair (`B&W toner fx=0/1`: `23.0` insns/pair on `4704×8`; `fill=1 fx=0`: `29.9`; `fx=1`: `65.4`; `fx=7 fill=1`: `147.6`).

### M6M ARM946E-S BE (`C78FRM.bin`)
- **Payload**: `126` hooks, `118` strings, `42` menus, `431` rows (`codegen_all_36_films_and_subset_grids` also verifies `36/36` films + `11/11` grids link within budget).
- **Region usage**:
  - `CODE`: `12,920 / 15,316 B` | `DATA`: `12,436 / 12,800 B` | `LOOKS`: `7,476 / 8,756 B` | `MENU`: `4,148 / 4,672 B`
  - `DOOM_A`: `4,752 / 4,876 B` | `DOOM_B`: `4,092 / 4,668 B` | `DOOM_C`: `2,180 / 2,492 B` | `DOOM_D`: `1,104 / 2,304 B` | `DOOM_E`: `2,820 / 3,500 B`
- **Live-view overlays (`m6m_bench`, `draw only` emulated ARM9 insns; host DMA fills/flushes excluded)**:

| Grid | Mode | `default` | `checker` | `white` | `black` | `fine-detail` |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `7` | Electronic Level | `10,832` | `10,832` | `10,832` | `10,832` | `10,832` |
| `8` | 21:9 XPan + Level | `29,794` | `29,794` | `29,794` | `29,794` | `29,794` |
| `10` | 1:1 Square + Level | `63,287` | `63,287` | `63,287` | `63,287` | `63,287` |
| `11` | Focus Peaking | `615,558` | `368,620` | `368,620` | `368,620` | `1,148,350` |
| `12` | Peaking + Level | `626,134` | `379,196` | `379,196` | `379,196` | `1,158,926` |
| `13` | Zebras | `150,310` | `411,112` | `241,724` | `148,444` | `204,719` |
| `14` | RGB Hist + Level | `154,952` | `150,076` | `150,076` | `150,306` | `160,668` |
| `17` | False Color (IRE) | `266,756` | `287,772` | `287,772` | `287,772` | `287,354` |

- **Focus peaking, direct draw (`m6m_bench`)**: independent lane order now reuses adjacent samples, eliminating the writer's stack spill (`360 -> 344 B`, `16 B` less `CODE`). Full-frame scalar oracles cover a dense PRNG frame and threshold 24/72 boundaries; output is byte-exact. Flat scenes are unchanged. The adaptive dense-scene bypass was not included.

| Scene | Before reorder | Current | Change |
| --- | ---: | ---: | ---: |
| `default` | `625,813` | `615,305` | `-1.7%` |
| `fine-detail` | `1,181,277` | `1,148,097` | `-2.8%` |

- **Dual Hi+Lo zebras, direct draw (`m6m_bench`)**: scalar C → ARM/SWAR with uniform-highlight and uniform-shadow bulk stores. Independent full-frame oracles cover all eight phases, all 256 luma values, varied all-shadow blocks, and shadow/midtone fallback blocks:

| Scene | Before low store | Current | Change |
| --- | ---: | ---: | ---: |
| `default` | `221,030` | `221,122` | `+0.04%` |
| `checker` | `852,888` | `869,848` | `+2.0%` |
| `white` | `318,860` | `318,860` | `0%` |
| `black` | `852,888` | `429,100` | `-49.7%` |

  The uniform-shadow path adds `88 B` of `CODE`: two instructions to each non-uniform block containing highlights, and thirteen to each non-uniform block with shadows but no highlights (the all-low probe ahead of the scalar fallback). These are emulator instruction counts, not measured hardware frame times. The dual threshold is the compile-time `ZEBRA_THRESH=242`, not a runtime argument. Two additional full-frame oracles cover all `65,536` ordered adjacent-luma pairs, including every shadow/highlight boundary and SWAR lane carry/borrow combination. Camera A/B remains pending.

- **False Color, direct draw (`m6m_bench`)**: nonzero blocks now use four 16-byte burst stores (two per FB row) instead of eight compare/conditional-store pairs. The all-zero LUT block skip is unchanged; writes of zero-valued words are exact because the caller DMA-zeroes the framebuffer. Independent poisoned full-frame oracles cover all 256 luma values, same-zone/nonuniform-luma blocks, adversarial mixed patterns, one-nonzero-sample blocks, and all-midtone preservation.

| Scene | Before burst stores | Current | Change |
| --- | ---: | ---: | ---: |
| `default` | `316,856` | `252,896` | `-20.2%` |
| `checker` / `white` / `black` | `443,512` | `273,912` | `-38.2%` |

  The path removes `80 B` from `DOOM_D`. Sparse nonzero blocks write 64 B rather than only their nonzero words, so DRAM/write-buffer behavior and camera A/B still need hardware validation; emulated instruction counts are not frame times.

- **Label renderer (`labels.c`, `m6m_bench`)**: `41` emulated insns cached (`O(1)` `(class, sid)` table), `271,272..3,108,375` cold across classes 1..6. Cold renders clean `22..242` tile cache lines instead of SWI 10's `512`, with no whole-cache invalidate. Cache CP15 cost is not modeled: the extra `9` cold emulated instructions are address setup, not a hardware timing measurement. On-camera visual and latency A/B remain pending.
- **Custom DOOM applet (`SCREEN_DOOM`, `0x21`)**: custom 2.5D sector/portal renderer with a hand-built 13-sector map (not a port of id DOOM); `320×240` 8bpp → `640×480` `RGBA4444` 2×2 upscale. `camera_flow` enforces `< 2.5M` emulated ARM9 insns/tick and clean exit back to `SCREEN_LV` (`0x15`).

## 3. Emulator & verification commands

- `crates/dp2m-emu/src/arm9.rs` uses a direct-fetch SDRAM loop with a 2 MB hook bitmap (`~231 MIPS` overlay / `~493 MIPS` label host rate; `m6m_bench` ~1.0 s wall time). `crates/dp2m-emu/src/fr80/emu.rs` uses direct ROM fetch with a 64 KB SDRAM TLB (`1024` pages) and handler bloom filter.
- `dp2m verify out`: **575 / 575** checks (`m6m: 299`, `custom: 130`, `labels: 146`) in ~0.80 s; `--artifacts <dir>` writes `labels.png`.
- `camera_flow` verifies Film page OSD FNV-1a64 (`0x16a06c94433a64d3`), menus, AE, WB, native `SCREEN_PB`, dual-CPU capture, and DOOM gameplay.

```sh
cargo run --release -- build
cargo run --release -- verify out
cargo run --release -- verify-m5 out/Firm.bin
cargo test --release --workspace
cargo test --release -p dp2m m6m_bench -- --ignored --nocapture
cargo test --release -p dp2m ycc_bench -- --ignored --nocapture
```
