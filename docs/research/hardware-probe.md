# On-camera hardware probe

Probe V2 (`42cf7ec`, package SHA-256 `1ef3dc3a239659454eacd6f1abf005ab9d4b27c9a0ed85baccbb113bee351c71`) and Probe V3 (`dd67465`, package SHA-256 `5dae1e2084e8e90d09e23fa42387f6c22e91db7dd1d9daaa75d5a0986e343783`) were run on a DP2 Merrill on 2026-11-27. These are synthetic on-camera measurements, not emulator instruction counts and not end-to-end application frame rates.

The probe replaces the custom DOOM applet when built with `dp2m.probe.fnl`. It writes only its own RAM, the overlay framebuffer, display dirty state, and a probe-exclusive power-save bit. It does not write NOR, NVRAM, or the SD file system. The normal build remains byte-identical when probe support is disabled.

## Input and frame pacing

The first screen message was `(4,0)` at 9,178 ms. After 16.655 seconds without input, pressing OK produced `(5,1)` at 25,835 ms. No other message arrived in that interval; `(4,2)` and `(4,3)` counts stayed zero.

**Stock UI messages do not provide an autonomous applet tick.** The current portal renderer therefore advances on input on hardware. `Camera::tick()` in the simulator is an explicit test driver that injects `(4,3)`, not evidence that stock firmware posts it periodically. A real engine needs explicit `get_tim`/`dly_tsk` pacing, held-key polling, and a safe UI-semaphore strategy.

Both captures were drawn in response to OK (opening the probe from the menu, then the `(5,1)` key event) with the shutter untouched. `KEY0` (`0xA07FAA1C`) and its edge snapshot `0xA07FAB24` read `0x40000000`, which is the OK slot in the pressed state and is consistent with OK still being down. `SW28` (`0xA07FAA28`) also read `0x40000000`: bit 30 is set at idle. Stock routine `0xA016DD94` builds that word from GPIO: bit 31 is half-press (S1 closed, S2 open), bit 26 is full press (both closed), and bit 30 follows a separate active-low line that the stock shutter scan `0xA01690B4` never acts on. DOOM therefore reads the shutter as `SW28 & 0x84000000` (`DP2M_SW28_SHUTTER`); including bit 30 would report the shutter as permanently held on hardware. `DOOM.BIN` applies that mask to `raw_sw28` itself, so its fire button does not depend on the mask compiled into the flashed firmware.

## CPU, memory, and display timing

`CPSR=0x60000010` confirms user mode. The stock clock getters returned 108 MHz and 27 MHz; the 108 MHz value is not proven to be the core clock. The ALU result is consistent with a core near 200-216 MHz, but does not directly identify the PLL setting.

| Operation | Measured total |
| --- | ---: |
| 2,000,000-iteration ARM ALU loop | 101 ms |
| 16 MiB sequential SDRAM read | 161 ms |
| 4 MiB repeated 64 KiB cached/write-buffer writes | 24 ms |
| 524,288 random reads | 191 ms |
| 10 indexed 320x200 to 640x400 ARGB blits | 53 ms |
| 5,000 full D-cache syncs | 126 ms |
| 10 flush+ARGB commits | 21 ms |
| 10 D-cache sync+flush+ARGB commits | 16 ms |

The final two rows violate their superset relationship, so single-shot present/commit values have at least 24% uncertainty. The 320x200 blit cost is 5.3 ms per frame; a rough blit-plus-present budget is 7-9 ms, before any game or renderer work. This is not an engine FPS measurement. V3 below directly measures the retained portal upscaler instead of estimating it from the generic blit.

### V3 repeated display timing

V3 retained the V2 single-shot CPU and memory kernels, added direct portal-upscale and DMA-clear tests, and measured commit/present as minima of three cells with alternating order.

| Operation | V3 measured total |
| --- | ---: |
| 2,000,000-iteration ARM ALU loop | 120 ms |
| 16 MiB sequential SDRAM read | 184 ms |
| 4 MiB repeated 64 KiB cached/write-buffer writes | 30 ms |
| 524,288 random reads | 227 ms |
| 10 indexed 320x200 to 640x400 ARGB blits | 60 ms |
| 10 direct 320x240 to 640x480 portal 2x2 upscales | 52 ms |
| 10 D-cache sync plus 640x424 DMA clears | 30 ms |
| 5,000 full D-cache syncs | 155 ms |
| 10 flush+ARGB commits, minimum of three | 17 ms |
| 10 D-cache sync+flush+ARGB commits, minimum of three | 23 ms |

The unchanged single-shot kernels are 13-25% slower than their V2 cells. This is timing variability across runs and builds, not an algorithmic kernel regression, and reinforces the need for repeated timing. The direct portal upscale is **5.2 ms/frame** and the clear is **3.0 ms/frame**. Minimum-of-three commit and present are **1.7 and 2.3 ms/frame**, respectively; `PRESENT >= COMMIT` now holds as required. Their 0.6 ms/frame difference is not a precise SWI 10 cost because the two minima can come from different repetitions.

The arithmetic sum of the separately measured portal upscale and present cells is 7.5 ms before game and scene-rendering work. Adding the full clear gives 10.5 ms. These are component budgets, not measured end-to-end pipeline times or application frame rates.

## Framebuffer write patterns

Each value is the total for eight complete 640x424 passes. Every pass has 16,960 32-byte blocks. W1 and W4 write 542,720 bytes per pass; W2, W3, and W5 write 67,840 bytes.

| Pattern | ARM insns/block | Word stores/pass | Warm | Cold |
| --- | ---: | ---: | ---: | ---: |
| W1: dense eight-word bursts | 4 | 135,680 | 20 ms | 13 ms |
| W2: sparse conditional single store | 7 | 16,960 | 20 ms | 21 ms |
| W3: eight-word burst every eighth block | 8 | 16,960 | 28 ms | 21 ms |
| W4: dense burst, one colour plus seven zeros | 9 | 135,680 | 27 ms | 36 ms |
| W5: eight conditional stores, one taken | 21 | 16,960 | 27 ms | 34 ms |

W4 versus W5 is the cleanest available comparison: the same two source loads and one taken colour store per block, but different instruction counts and eight times the write bytes. Their pair means are 3.94 and 3.81 ms per pass, only 3% apart, and the warm cells tie. W1 also costs less than W2 despite eight times the write volume. **Dense and zero-filled burst stores do not show the severe hardware penalty that the emulator-only analysis could not rule out.** Keep the optimized overlay writers; further store-avoidance work is not a priority.

The exact magnitude is less certain. V2 cells were single-shot and ordered warm then cold, producing 25-35% inconsistencies. W2-W5 read only two of the four source words per block, and the source cannot fit in the 16 KiB D-cache. Histogram counter read-modify-write cost is not modeled by these patterns.

### V3 W0-W5 repeated cells

V3 added the store-free W0 baseline. Each value below is the minimum of three cells, with eight complete 640x424 passes per cell. Pattern order reverses on odd repetitions, and warm/cold order also alternates.

| Pattern | ARM insns/block | Word stores/pass | Warm, eight passes | Cold, eight passes |
| --- | ---: | ---: | ---: | ---: |
| W0: two luma loads plus OR, no stores | 6 | 0 | 18 ms | 25 ms |
| W1: dense eight-word bursts | 4 | 135,680 | 12 ms | 20 ms |
| W2: sparse conditional single store | 7 | 16,960 | 21 ms | 19 ms |
| W3: eight-word burst every eighth block | 8 | 16,960 | 26 ms | 28 ms |
| W4: dense burst, one colour plus seven zeros | 9 | 135,680 | 35 ms | 34 ms |
| W5: eight conditional stores, one taken | 21 | 16,960 | 33 ms | 33 ms |

The W0 sanity check passes: cold is 25 ms versus warm at 18 ms. W1 is faster than W0 in both cells and faster than W2 on pair mean despite its dense stores, so instruction and store counts alone do not predict hardware time. W4 versus W5 gives warm/cold means of **4.31 and 4.13 ms/pass**, only 4.5% apart. This repeats the V2 conclusion on a new run: **there is no severe dense/zero-burst store penalty, and further store-avoidance work is not a priority.** W4/W5 still differ in instruction count, so this is evidence against a severe penalty rather than a pure store-volume isolation.

W2-W0 is +3 ms warm but -6 ms cold across eight passes, so it does not isolate a reliable single-store cost. W2 and W4 also invert the expected cold/warm ordering and W5 ties. Minimum-of-three sampling reduced single-shot noise but did not eliminate it; these small differences must not be overinterpreted. Representative LV overlays, histogram counter traffic, and complete application frames still need end-to-end measurement.

## Memory and ownership evidence

- All 248 one-MiB windows from `0xA0800000` through `0xAFFFFFFF` completed aligned reads without an abort. The full 256 MiB aperture is present and readable.
- Payload RAM is `0xA1C00000..0xA1E00000`. Probe `.bss` uses 1,169 KiB and leaves 878 KiB.
- Two variable-memory pools responded: MPL1 free/max `0x68D58` (429,400 B) and MPL2 free/max `0x8548` (34,120 B). MPL3-10 returned `-42`.
- Twenty fixed-block pools responded. These uITRON pools are distinct from the 17 MiB stock C heap.
- No one-MiB window had all 256 sampled words equal to zero. Fourteen windows changed between two passes. High memory through `0xAF5xxxxx` was mostly stable and nonzero; `0xAF7xxxxx` contained changing samples.
- **Zero or stable contents do not prove that RAM is free.** An independent review subsequently refuted the proposed `0xA6000000..0xA79FFFFF` arena; see below.

The first 64 KiB after `.bss` contained 65,536 nonzero bytes at both entry and rescan. Initial bytes were `FF DF F7 FF FF FF E7 B7 FF FF FB FE DE FF FD FF`. Payload RAM is dirty at boot, not zero-filled.

V1 skipped initialization because it tested an uninitialized `pst.inited` as a boolean. V2 clears the state and uses an explicit magic value. `probe_flow` now poisons `pst` before exercising all pages, so this camera failure is covered by a regression test.

## MPU and arena review

Independent DeepSeek review of the stock ARM946E-S boot sequence confirmed:

- Region 3 is **8 MB** (`0xA0000000..0xA07FFFFF`), not 32 MB. Region 2 has instruction AP 0, so instruction fetch above `0xA0800000` aborts in every mode. Payload tasks run in user mode and cannot reprogram CP15.
- The proposed patches at `0xA0172170` (`E3A00008` to `E3A0000C`) and `0xA0172190` (`E3800000` to `E3800C06`) have the intended encodings: enable Region 2 I-cache and set its instruction AP to read/execute in both modes. They have **not** been applied or tested on hardware. Resizing Region 3 is unnecessary.
- `0xA6000000..0xA79FFFFF` is **not available for DOOM**. Stock descriptor routine `0xA0035AAC` installs capture plane bases `0xA53132C0`, `0xA6A622C0`, `0xA81B12C0`, and further planes with exact stride `0x174F000`. The rejected window overlaps the first two stock capture buffers. Playback and shared scratch paths also reference this arena.

A future execution test must use an arena that first passes an operation-matrix ownership test. After a legitimate patched-firmware install, a minimal stub can test return value and I-cache behavior with SWI 9 followed by SWI 15. A missing or incorrect MPU patch can hang or reset the camera, so battery-pull recovery is required. The emulator does not model CP15 and cannot validate this test.

## Implemented `D2AP` v1 architecture & remaining on-camera checks

Based on the V2/V3 probe findings and the MPU/arena audit above, the standalone `DOOM.BIN` platform was implemented as follows:

1. **Safe arena (`0xA1CC0000..0xA1E00000`) + stock C heap**: Instead of the refuted `0xA6000000` capture-buffer window, `DOOM.BIN` (`318,000 B` image `0xA1CC0000..0xA1D0DA30` + `244.6 KB` `.bss`, ending at `0xA1D4ACB0` with `724.8 KB` headroom) loads into the upper `1.25 MB` of our unreferenced `RAM` window above `__bss_end` (`0xA1CB7E58`). Both the `4 MB` DOOM zone and preloaded `<= 6 MB` `.WAD` buffer allocate from the `17 MiB` stock C heap (`0xA003D9C0` / `0xA003D9AC`) and are 100% freed on `DP2M_APP_SHUTDOWN` (`Play` exit), with a `16 KB` sector-aligned MDF read-ahead cache fallback (and tracked descriptor cleanup) for larger `.WAD` files. The engine switches to a guarded 8 KB private stack instead of relying on the unknown stock UI-task stack margin; WAD directories and all later lump reads are bounds-checked.
2. **MPU Region 2 execution & dirty `.bss` guards**: Boot hooks at `0xA0172170` (`c2_i = 0xC`) and `0xA0172190` (`c5_i` R2 = `6`) enable cached execution in `0xA1CC0000..0xA1E00000`, synchronized on load with `SWI 10` (`hw_dcache_sync`) + `SWI 15` (`hw_icache_inv_all`). All payload `.bss` state (`lv.c`, `labels.c`, `custom.c`, `doom.c`, `probe.c`) uses explicit boot resets (`lv_boot_reset()` and `doom_boot_reset()` in `labels_boot()`) and 32-bit `volatile` magic tags, verified under poisoned SDRAM (`0xFF` / `0xA5`).
3. **Clock safety (`0xA0040658` left untouched)**: Disassembly of `0xA0040658` confirmed it switches sensor/capture peripheral dividers rather than boosting the ARM946E-S core (which already runs near ~200-216 MHz per the ALU loop above). Calling it while LCD/LV DMA is active risks display/sensor desync; `SCREEN_DOOM` instead inhibits auto-power-off exclusively via `stock_psave_inhibit(0x40000000)` / `stock_psave_release(0x40000000)`.
4. **Autonomous 35 Hz frame loop, hardware controls & clean LV/power-off exit**: When `get_tid != 0` (real camera UI task), `SCREEN_DOOM` paces frames to exact `35.000 Hz` tic boundaries via `get_tim` / `dly_tsk`, polls active-high `REG32(0xA07FAA1C)` (`KEY0`), `REG32(0xA07FAA28) & 0x84000000` (`Shutter`: bit 31 half-press, bit 26 full press), `REG8(0xA07AF38C)` (`Dial`), and `0xA001A38E`/`0xA001A3A2` (`Accel`), and blocks background photo capture via `shutter_guard_hook` (`0xA01690B6`). Each frame also polls `MBX 0x11` so normal UI packets return immediately to `MPF 0x10` while system power-off packets (`type == 6`, `0xA004330C`) break out and re-queue via `stock_snd_mbx(0x11, sys_msg)`; on normal `Play` exit it syncs both shutter and key edge snapshots (`0xA07FAB20 = 0xA07FAA28`, `0xA07FAB24 = 0xA07FAA1C`), clears `ARGB_FB`/`OSD`, and posts `(2, SCREEN_LV)` through `stock_screen_goto`.

Emulator suites (`dp2m verify out` `607/607` and `doom_app_flow`) exercise the software-side invariants above, including private-stack placement/guarding, inherited-button suppression, malformed-WAD rejection, and cleanup. They do not establish CP15 enforcement, physical input behavior, SD/display throughput, or cache/MPU correctness on hardware. The explorer-only `0x9000010C` / `0x90000110` MMIO reads also remain unverified; they are not read by the always-on dashboard. Current `DP2MV200.bin` + `DOOM.BIN` + `DOOM1.WAD` therefore still require the on-camera validation described in the final handoff.
