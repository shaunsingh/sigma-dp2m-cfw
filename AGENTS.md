# AGENTS.md

## Architectural Mandate

`dp2m-sim` and `dp2m-web` are **hardware-level firmware emulators**, never UI or ISP recreations:

- Every screen (`0x15` Live View, `0x17` Mode, `0x19` Playback, `0x1A` Quick Set, `0x1B` Menu, submenus), the AE solver (`0xA000D6B6` + `autoiso_*_hook`), White Balance CCMatrix, Live View B2Y setup, MakerNote packing/parsing, and FR80 capture stages (`hook_ipc_exif`, `hook_matrix`, `hook_lutsel`, `hook_mcas`, `spatial_driver`, `ycc_engine`) must execute the compiled `C78FRM.bin` (ARM926EJ-S BE) and `Firm.bin` (FR80 BE) binaries.
- Never draw synthetic menus, HUDs, playback overlays, histograms, or exposure solvers in Rust or JS. Host hooks only model hardware registers, Milbeaut 2D DMA / B2Y / IIP blocks, µITRON 4.0 kernel primitives, and virtual NOR / SD / DCF storage.
- Bit-equivalent acceleration (libc `memcpy`/`memset`, basic-block decoding, AoT/JIT translation) is welcome.

## Environment & Commands

Always pass `--no-write-lock-file` to `nix develop` so `flake.lock` stays untracked. The gate, in order (`build` precedes `test` because `crates/dp2m-sim/tests/camera_flow.rs` and `tools.verify` read the images in `out/`):

```sh
nix develop --no-write-lock-file -c cargo clippy --offline --all-targets -- -D warnings
nix develop --no-write-lock-file -c cargo run --release -- build
nix develop --no-write-lock-file -c cargo test --release --offline
nix develop --no-write-lock-file -c python3 -m tools.verify out
```

## Merging

- Work happens on `ws/<area>` branches in git worktrees. Only the integration agent merges into `main` and pushes `main` to `origin` (no force pushes, no history rewrites); everyone else requests a merge through the orchestrator.
- The four commands above are the merge gate, run in that order on the rebased branch.

Inspection & simulator workflows:

- `cargo run --release -- sim [--port 8080]`: builds `dp2m-web.wasm` and serves `web/index.html`, `dp2m_web.wasm`, `out/C78FRM.bin`, and `out/Firm.bin` fresh from disk on every HTTP request (rebuild WASM or firmware and refresh the browser tab; no server restart needed).
- `cargo run --release -- sim-trace [out/DP2MV200.bin] [--keys "Menu,DialRight,DialRight,Down,Right"] [--ppm frame.ppm] [--watch-text]`: headless dual-CPU step runner; prints screen ID, AE state, NVRAM/mode values, OSD/RGB stats, CPU faults, `.text` write watchpoints, and optional `640x480` PPM output. Accepts `data/DP2MV105.bin` for stock-vs-custom A/B comparison.
- `python3 -m tools.re [--image out/C78FRM.bin] <dis|xrefs|callers|sid|strings|menus|screens|icons|nvram>`: static & Unicorn-backed M6M inspection (`tools/re/stock.py` holds shared symbol names).
- `cargo run --release -- disasm-fr80 out/Firm.bin <va_start> <va_end>` and `verify-m5 out/Firm.bin`: M5Slave FR80 disassembly and verification.
- On macOS environments with Microsoft Edge instead of Chrome, pass `--executable-path "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"` to `agent-browser`.

## Firmware & Hardware Invariants

### M6M (`C78FRM.bin` — ARM926EJ-S Big-Endian, S3 @ `0xA0000000`)

- **Protected regions** (enforced by `crates/dp2m/src/m6m/mod.rs`):
  - `C78FRM.bin[0..0x20000]`: IPL bootloader.
  - `0xA0179000..0xA023D35A`: Spartan-6 FPGA bitstream (`803,674` B). An accidental byte diff causes a white screen and dead shutter, so the builder check stays on by default. Deliberate bitstream work is allowed: it is opt-in in the config, must keep the configuration CRCs valid, and needs a hardware test plan with a recovery path.
  - `0xA07FBC1C..0xA0800000`: exception vector source, zero-filled by boot scatter (`0xA07AD3CC`).
  - `0xA004D3D0`: shared literal pool word (`0xA07EEFCC`); hook `Get_Menu_Table` (`0xA004CD0A`) instead.
  - `0xA005D924..0xA005F1A0` and `0xA07DEBF0..0xA07DEC0B`: live screen 50..55 handlers and vtable slots.
  - NVRAM `0xA0939774..0xA0939783`: language (`0xA0939776`) and LCD brightness (`0xA0939778`). Custom settings belong in `0xA0939688..0xA0939697`; volatile runtime state belongs in payload `.bss` (`0xA1C00000..0xA1E00000`).
- **Writable code in SDRAM**: S3 executes directly from SDRAM (`0xA0000000..0xA0800000`). Any wild pointer write into `0xA0000000..0xA0179000` silently corrupts instructions and surfaces later as an unaligned jump. Use `dp2m sim-trace --watch-text` first whenever an unfamiliar screen or menu crashes.
- **µITRON 4.0 out-pointer syscalls**: Never stub `get_mpf`/`pget_mpf`/`tget_mpf` (`0xA0071830`/`1970`/`1C54`), `get_mpl`/`pget_mpl`/`tget_mpl` (`0xA0071F30`/`212C`/`22A0`), `ref_mbx` (`0xA0072C74`), `get_tid` (`0xA0073B70`), or `ref_tsk` (`0xA00752D4`) with a bare `return 0`. Returning `E_OK` without writing `*p_blk` leaves uninitialized stack addresses aimed into `.text`; returning `tskstat == 0` instead of `TTS_DMT (0x10)` from `ref_tsk` tricks `SCREEN_PB` (`0xA0045948`, `0xA0057C0E`) into thinking background slideshow/movie tasks are active.
- **Video buffer separation**: `SCREEN_PB` (`0xA000F222`) cycles four `640x480` YCbCr buffers at `0xA235E600`, `0xA243F600`, `0xA2520600`, and `0xA2601600`. Keep Live View and scratch buffers above `0xA2A00000`.
- **Histogram NaN clamp**: M6M's histogram bar drawer (`0xA0048F7C`) divides each column by total bin count in IEEE-754 float and clamps with signed integer `cmp r0, 0x3F800000` (`1.0f`). All-zero bins yield `0.0f / 0.0f = 0x7FC00000 > 0x3F800000`, filling every bar to 100%. `0xA004A7AC` must populate real pixel counts.

### M5Slave (`Firm.bin` — Fujitsu FR80 Big-Endian, `VA = file_off + 0x3BF80`)

- **NOR flash & Error `0002`**: Physical NOR ends at `VA = 0x00800000` (`file_off = 0x7C4080`). Bytes at `file_off >= 0x7C4080` must stay bit-exact stock, and `u32_be(Firm.bin[0x4C..0x50])` must equal `sum(Firm.bin[0x80..0x800080])` mod `2^32`.
- **FR80 shift opcodes**: `0xB0` = `LSR`, `0xB4` = `LSL`, `0xB8` = `ASR`. Mixing `0xB0` and `0xB8` breaks signed Foveon CCMatrix off-diagonals and Q1 3D-LUT residuals. Use unsigned `blo` (`0xE4`) / `bhs` (`0xE5`) for `0x80000000+` SDRAM pointer bounds.
- **SDRAM hazards**: Never use `0x81046580..0x8105A580` (`THUMB_Y/CB/CR` written asynchronously by background EXIF task `0x4E9714`). Keep `0x4D3EC8` at `0.70f` (`0x3F333333`) so JPEG output cannot overflow `PREVIEW_Y` (`0x80FB0580`). Read live color-space matrices from SDRAM shadow `0x80012E46`, not MMIO `0x40100500`.

## Style

- Comments explain hardware/ABI constraints, register contracts, or non-obvious invariants only; never restate code or narrate edits.
- Keep documentation concise, factual, and indexed by hex address (`docs/hardware.md`, `docs/research/`).
