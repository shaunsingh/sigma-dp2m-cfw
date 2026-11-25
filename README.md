# dp2m-fw

Custom firmware, toolchain and reverse-engineering notes for the Sigma DP2 Merrill.

The camera has two processors, both updated from one package:

- **M6M** (`C78FRM.bin`): ARM926EJ-S, big endian. UI, settings, live view, capture control.
- **M5Slave** (`Firm.bin`): Fujitsu FR80 ("TRUE II"), big endian. Capture ISP and the JPEG / X3F pipeline.

dp2m starts from the stock 1.05 package (`DP2MV105.bin`). It assembles FR80 code into free M5 flash, compiles a C payload and links it into unused parts of the M6M image, and writes a new package for the stock updater.

## Features

- **Film** menu page: color mode, print paper, 3D spectral engine, grain, halation, X3 fill.
- 36 color modes after the 8 stock ones: Quattro looks, optical-filter mono, reversal, negative and B&W films, and an SD-card LUT mode. Negative films take one of 8 print papers, B&W films one of 8 darkroom papers, the SD LUT mode one of `LUT1.BIN`..`LUT8.BIN`.
- 3D spectral engine, grain and halation are independent toggles.
- Live view uses each film's color matrix and tone curve, and follows mode and paper changes immediately.
- Custom exposure modes C1..C5.
- ISO 50, 5-shot bracketing, minimum shutter speed for ISO auto.
- Live-view overlays as extra grid modes: electronic level, aspect masks, focus peaking, zebras, RGB histogram.

## Requirements

- Rust 1.85 or newer (edition 2024) with `cargo`.
- LLVM `clang` and `ld.lld` on `PATH`. The M6M build calls them directly with `--target=armeb-none-eabi`.
- A host C compiler and `make`: `mlua` builds its vendored LuaJIT from source.
- Optional: Python 3 with `numpy`, `scipy`, `capstone`, `unicorn` and `pillow` for the scripts in `tools/`.
- Optional: Nix. `nix develop` opens a shell with all of the above (see `flake.nix`).

The build reads the stock package from `data/DP2MV105.bin`.

## Quick start

```sh
cp dp2m.example.fnl dp2m.fnl   # optional: dp2m.example.fnl is used when dp2m.fnl is missing
cargo run --release -- build
```

## Configuration

`dp2m.fnl` is a [Fennel](https://fennel-lang.org) program. The builder compiles and runs it with an embedded LuaJIT (`mlua`, with the bundled `crates/dp2m/lua/fennel.lua`) and reads the table it returns. A global `dp2m` exposes the catalog, so a config can compute its settings:

| Global | Content |
| --- | --- |
| `dp2m.films` | `[{:slug :label :kind}]`, kinds `quattro mono reversal negative bw sd-lut` |
| `dp2m.papers` | `{:negative [...] :bw [...] :sd-lut [...]}`, each `[{:slug :label}]` |
| `dp2m.grids` | `[{:slug :label}]` |

Example: show only negative and B&W films, use Kodak 2383 as the "Auto" paper for every negative, and hide the zebra overlay.

```fennel
(local keep {:negative true :bw true})

{:firmware "data/DP2MV105.bin"
 :version  "2.00"
 :films (collect [_ f (ipairs dp2m.films)]
          (values f.slug {:enabled (or (. keep f.kind) false)
                          :paper (if (= f.kind :negative) :kodak-2383)}))
 :m6m {:grids {:zebras false}}}
```

| Key | Default | Meaning |
| --- | --- | --- |
| `:firmware` | | stock `DP2MV105.bin`, relative to the repository root |
| `:out-dir` | `out` | output directory |
| `:pack` | `data/film_pack.bin` | film calibration pack |
| `:verify` | `true` | run the FR80 emulation check on `Firm.bin` |
| `:version` | `"2.00"` | package version `major.minor`, at least `1.05` |
| `:films` | all shown | per film slug: `true` / `false`, or `{:enabled bool :paper slug}`; `:paper` is the paper used for "Auto" |
| `:m5` | | `:grain-scale` and `:halo-scale` (0..4); `:sd-lut-defaults {:grain :grain-profile :halo :halo-tint}` for the SD LUT mode |
| `:m6m` | | `:iso50`, `:bkt5`, `:peaking-threshold`, `:zebra-threshold`, `:min-shutter-q16` (7 APEX Tv values in Q16), `:grids` (per grid slug) |
| `:sd` | | `:mount`, and the files `stage` copies: `:package` (default `true`), `:firm`, `:c78` |

Unknown keys and unknown film, paper or grid slugs are errors. `dp2m.example.fnl` lists every key.

## Build outputs

| File | Use |
| --- | --- |
| `out/DP2MV200.bin` | package for the stock updater; the name follows `:version` |
| `out/Firm.bin` | M5Slave image, for debug updates |
| `out/C78FRM.bin` | M6M image, for debug updates |
| `out/m6m/m6m.elf`, `m6m.map` | linked M6M payload and linker map |
| `out/m6m/m6m.syms` | payload symbols (`address size name`) |
| `out/m6m/catalog.h`, `catalog.c` | generated strings, menus, film table and live-view looks |

Set `SOURCE_DATE_EPOCH` to fix the date written into the package header.

## Installing

1. Copy `DP2MV200.bin` to the root of the SD card.
2. On the camera, run Setup > Firmware Update. The screen shows the current version and 2.00.
3. The camera flashes a bridge image, restarts, then flashes the M5 and the M6M. Do not remove power during the update.

The updater accepts any package version from 1.05 up.

### Debug images

`Firm.bin` and `C78FRM.bin` in the card root each flash one chip through the same menu, without a version check. The screen shows a banner and the file date instead of versions (`M6M Only Debug Write.` on stock firmware, `dp2m-fw (M6M)` once dp2m is installed). If `Firm.bin` fails with error 0002, remove the battery; the camera boots its previous image.

`cargo run --release -- stage --to <card>` copies the files enabled under `:sd` and reads them back.

## Commands

Run as `cargo run --release -- <command>`. Without `--config`, the builder uses `dp2m.fnl`, then `dp2m.example.fnl`, from the repository root.

| Command | Action |
| --- | --- |
| `build [DP2MV105.bin] [--config f] [--out dir] [--pack f] [--skip-verify]` | build all images; `--skip-verify` skips the FR80 check |
| `stage [--config f] [--from dir] [--to card]` | copy images to the SD card and read them back |
| `verify-m5 <Firm.bin>` | checksums, protected constants and emulated capture jobs |
| `disasm-fr80 <Firm.bin> <va_start> <va_end>` | disassemble an FR80 address range (hex) |
| `inspect <file>` | identify a package, `Firm.bin` or `C78FRM.bin`; print version or checksum |

- `cargo test --release` runs the unit tests (config loader, catalog layout, package date).
- `cargo test --release -p dp2m ycc_bench -- --ignored --nocapture` prints the M5 engine cost per stage.

Scripts in `tools/`:

| Script | Purpose |
| --- | --- |
| `cube_to_lut.py in.cube LUT1.BIN [--grain --grain-profile --halo --halo-tint]` | convert a `.cube` 3D LUT into a `LUTn.BIN` for the SD LUT mode |
| `pack_builder.py` | build `data/film_pack.bin` from calibration data |
| `x3f_camf.py` | decode X3F CAMF sections |
| `x3f_raw_dump.c` | decode Merrill X3F raw data to 16-bit samples |
| `verify_m6m_unicorn.py` | Unicorn run of M6M hooks; written for the earlier fixed-address payload |

## Repository layout

```text
Cargo.toml               workspace
dp2m.example.fnl         config template
flake.nix                Nix dev shell
crates/
  fr80/                  FR80 assembler, disassembler, emulator
  dp2m/                  builder
    lua/fennel.lua       Fennel compiler (embedded)
    src/main.rs          CLI
    src/config.rs        Fennel config loader
    src/catalog.rs       films, papers, grids, packed mode ids
    src/pack.rs          film_pack.bin reader
    src/m5.rs            M5 patcher and FR80 verifier
    src/m6m/mod.rs       M6M compile, link, hooks, invariant checks
    src/m6m/codegen.rs   generated C tables
    src/package.rs       DP2MVxxx.bin writer
    src/elf.rs           ELF reader
src/
  asm/m5/                FR80 assembly: hooks, YCC engine, spatial driver
  m6m/                   M6M payload: C, assembly, linker script
data/
  DP2MV105.bin           stock firmware 1.05
  film_pack.bin          calibration tables (D2MP v1)
  npl_sigma_merrill.json Foveon X3 F20 spectral sensitivity
  ref/                   other Sigma firmware, for comparison
tools/                   Python and C helpers
docs/                    reference and research notes
```

## Documentation

- [docs/hardware.md](docs/hardware.md): consolidated reference (package, memory maps, protected regions, payload and hooks, NVRAM, IPC, M5 pipeline).
- [docs/research/updater.md](docs/research/updater.md): package format and update flow.
- [docs/research/m6m-ui.md](docs/research/m6m-ui.md): screens, menus, strings, Quick Set, icons, fonts.
- [docs/research/m6m-memory.md](docs/research/m6m-memory.md): SDRAM map, NVRAM block, NOR settings record.
- [docs/research/m6m-rom.md](docs/research/m6m-rom.md): dead-code analysis and payload regions; [candidate list](docs/research/m6m-rom-candidates.tsv).
- [docs/research/custom-modes.md](docs/research/custom-modes.md): exposure modes and custom slots.
- [docs/research/live-view.md](docs/research/live-view.md): refresh path, B2Y gamma and matrix, overlays.
- [docs/research/reference-firmware.md](docs/research/reference-firmware.md): other Sigma firmware in `data/ref/`.
