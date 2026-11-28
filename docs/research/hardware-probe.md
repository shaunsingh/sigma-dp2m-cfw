# On-camera hardware probe

Probe V2 (`42cf7ec`, package SHA-256 `1ef3dc3a239659454eacd6f1abf005ab9d4b27c9a0ed85baccbb113bee351c71`) was run on a DP2 Merrill on 2026-11-27. These are synthetic on-camera measurements, not emulator instruction counts and not end-to-end application frame rates.

The probe replaces the custom DOOM applet when built with `dp2m.probe.fnl`. It writes only its own RAM, the overlay framebuffer, display dirty state, and a probe-exclusive power-save bit. It does not write NOR, NVRAM, or the SD file system. The normal build remains byte-identical when probe support is disabled.

## Input and frame pacing

The first screen message was `(4,0)` at 9,178 ms. After 16.655 seconds without input, pressing OK produced `(5,1)` at 25,835 ms. No other message arrived in that interval; `(4,2)` and `(4,3)` counts stayed zero.

**Stock UI messages do not provide an autonomous applet tick.** The current portal renderer therefore advances on input on hardware. `Camera::tick()` in the simulator is an explicit test driver that injects `(4,3)`, not evidence that stock firmware posts it periodically. A real engine needs explicit `get_tim`/`dly_tsk` pacing, held-key polling, and a safe UI-semaphore strategy.

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

The final two rows violate their superset relationship, so single-shot present/commit values have at least 24% uncertainty. The 320x200 blit cost is 5.3 ms per frame; a rough blit-plus-present budget is 7-9 ms, before any game or renderer work. This is not an engine FPS measurement.

## Framebuffer write patterns

Each value is the total for eight complete 640x424 passes. Every pass has 16,960 32-byte blocks. W1 and W4 write 542,720 bytes per pass; W2, W3, and W5 write 67,840 bytes.

| Pattern | ARM insns/block | Word stores/pass | Warm | Cold |
| --- | ---: | ---: | ---: | ---: |
| W1: dense eight-word bursts | 4 | 135,680 | 20 ms | 13 ms |
| W2: sparse conditional single store | 7 | 16,960 | 20 ms | 21 ms |
| W3: eight-word burst every eighth block | 8 | 16,960 | 28 ms | 21 ms |
| W4: dense burst, one colour plus seven zeros | 9 | 135,680 | 27 ms | 36 ms |
| W5: eight conditional stores, one taken | 21 | 16,960 | 27 ms | 34 ms |

W4 versus W5 is the cleanest comparison: identical source loads and test shape, but eight times the write bytes. Their pair means are 3.94 and 3.81 ms per pass, only 3% apart, and the warm cells tie. W1 also costs less than W2 despite eight times the write volume. **Dense and zero-filled burst stores do not show the severe hardware penalty that the emulator-only analysis could not rule out.** Keep the optimized overlay writers; further store-avoidance work is not a priority.

The exact magnitude is less certain. Cells were single-shot and ordered warm then cold, producing 25-35% inconsistencies. W2-W5 read only two of the four source words per block, there is no store-free baseline, and the source cannot fit in the 16 KiB D-cache. Future timing should alternate repeated cells and report minima or medians, add a store-free W0, and measure `clear_dma` plus a real zebra frame end to end. Histogram counter read-modify-write cost is not modeled by these patterns.

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

## Remaining hardware gates

1. Find an SD-loaded code and heap arena that stays unchanged across screen entry/exit, capture, playback/zoom, LV, SD, USB, and power-save transitions. Until then, stream the WAD and shrink the engine footprint; do not use the rejected `0xA6000000` window.
2. Exercise a timed, yielding frame loop and clean exit without relying on `(4,3)`.
3. Measure the stock 216 MHz clock mode rather than assuming its scaling.
4. V3 adds alternating minimum-of-three cells, W0, direct portal-upscale and DMA-clear measurements. Its minima are not directly comparable to the V2 single-shot table. Representative LV overlays still need end-to-end timing.
5. SD storage still needs a successful read test with `DOOM1.WAD` at the card root; the absent-file path returned `-1` as expected.
