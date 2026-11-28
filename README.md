# dp2m-fw

Custom firmware for the Sigma DP2 Merrill, built from Sigma's stock 1.05 update.

The camera has two processors: an ARM946E-S running the UI and capture controls, and a Fujitsu FR80 handling image processing. The builder patches both and writes one package for Sigma's firmware updater.

## Features

- Film simulations, Quattro color modes, monochrome filters, print papers, and custom SD-card 3D LUTs.
- Optional 3D spectral processing, grain, halation, and exposure fill.
- Live-view film previews and extra grids: level, aspect masks, focus peaking, zebras, histogram, and false color.
- Custom exposure modes, ISO 50, five-shot bracketing, and a minimum shutter speed for Auto ISO.
- Standalone DOOM, loaded from the SD card.

See `dp2m.example.fnl` for every configuration option and [docs/hardware.md](docs/hardware.md) for the technical reference.

## Requirements

- Rust 1.85 or newer (edition 2024), with the `wasm32-unknown-unknown` target for the simulator.
- LLVM `clang` and `ld.lld` on `PATH`.
- A host C compiler and `make` for the vendored LuaJIT build.
- `data/DP2MV105.bin`, the stock 1.05 package.

With Nix, `nix develop` provides the complete toolchain, including Git.

## Build

```sh
cp dp2m.example.fnl dp2m.fnl   # optional
cargo run --release -- build
```

Without `dp2m.fnl`, the builder uses `dp2m.example.fnl`.

Outputs go to `out/`:

| File | Purpose |
| --- | --- |
| `DP2MV200.bin` | Complete package for Sigma's firmware updater |
| `Firm.bin` | M5 image for a debug update |
| `C78FRM.bin` | M6M image for a debug update |
| `DOOM.BIN` | Standalone DOOM applet, after `build-doom` |
| `m6m/` | M6M ELF, linker map, symbols, and generated tables |

The package name follows `:version` in the configuration. Set `SOURCE_DATE_EPOCH` for a reproducible package date.

## Configure

`dp2m.fnl` is a Fennel program. It can enable films, choose default papers, adjust processing, and hide overlays. Unknown keys and names are rejected.

```fennel
{:firmware "data/DP2MV105.bin"
 :version  "2.00"
 :m5 {:grain-scale 0.5}
 :m6m {:iso50 true
       :bkt5 true
       :grids {:zebras false}}}
```

`dp2m.example.fnl` documents film selection, paper defaults, SD LUTs, output paths, staging, and all other settings.

## Install

1. Copy `out/DP2MV200.bin` to the SD card root.
2. On the camera, open **Setup > Firmware Update**.
3. Confirm the update. Do not remove power until both processors have flashed and the camera has restarted.

The updater accepts package versions from 1.05 onward.

For a single-processor debug update, copy `Firm.bin` or `C78FRM.bin` to the card root and use the same menu. Debug images skip the package version check. If an M5 update fails with error `0002`, remove the battery; the camera will boot its previous image.

To copy firmware and optional DOOM files to a mounted card and verify the copies:

```sh
cargo run --release -- stage --to /path/to/card --wad /path/to/DOOM1.WAD
```

The `:sd` configuration controls which firmware files `stage` copies.

## DOOM

DOOM uses [doomgeneric](https://github.com/ozkl/doomgeneric) and a WAD you supply. The GPL-licensed doomgeneric sources stay outside this repository.

```sh
git clone https://github.com/ozkl/doomgeneric vendor/doomgeneric
cargo run --release -- build-doom
cargo run --release -- stage --to /path/to/card --wad /path/to/DOOM1.WAD
```

You can also pass `--doomgeneric <dir>` or set `DOOMGENERIC_DIR`. Open **Setup page 4 > DOOM**. If `DOOM.BIN` or the WAD is missing or invalid, the camera shows a short notice and returns to Live View; press PLAY to leave immediately.

The applet looks for `DOOM1.WAD`, `DOOM.WAD`, `DOOMU.WAD`, `DOOM2.WAD`, `PLUTONIA.WAD`, `TNT.WAD`, `FREEDOOM1.WAD`, and `FREEDOOM2.WAD` in that order. WADs up to 6 MB are preloaded; larger WADs stream through a 16 KB cache.

| Control | Action |
| --- | --- |
| Up / Down | Move |
| Left / Right | Turn |
| Command dial | Turn; move the cursor in menus |
| Half/full shutter or OK | Fire; confirm in menus |
| AEL | Use |
| MENU | DOOM menu |
| DISP | Automap |
| MODE | Cycle telemetry, fullscreen, and hardware explorer views |
| QS + Left / Right | Strafe |
| QS + Up / Down | Run; change explorer region |
| QS + dial | Change weapon |
| QS + AEL | Toggle tilt-strafe |
| PLAY | Exit to Live View |

Buttons held while DOOM opens are ignored until released. Auto power-off is suspended while DOOM runs.

## Development

Run commands as `cargo run --release -- <command>`. Run without a command to print the full usage.

| Command | Purpose |
| --- | --- |
| `build` | Build both firmware images and the update package |
| `build-doom [--warp]` | Build `DOOM.BIN`; `--warp` starts in E1M1 |
| `verify [out]` | Run M6M firmware checks |
| `verify-m5 <Firm.bin\|out>` | Check and emulate the M5 image |
| `stage --to <card>` | Copy selected outputs and verify read-back |
| `sim` | Run the browser simulator |
| `sim-trace` | Run scripted camera steps without a browser |
| `inspect <file>` | Identify a firmware package, image, or applet |
| `lut <in.cube> <LUTn.BIN>` | Compile a `.cube` LUT for the camera |

```sh
cargo clippy --release --workspace --all-targets -- -D warnings
cargo test --release --workspace
cargo run --release -- verify out
cargo run --release -- verify-m5 out/Firm.bin
```

Integration tests build into temporary directories. Standalone DOOM tests run when doomgeneric and a WAD are available through `DOOMGENERIC_DIR` and `DP2M_WAD`, or in their default local paths.

## Repository layout

```text
crates/
  dp2m/                  CLI and firmware builder
  dp2m-emu/              ARM9 and FR80 emulation, simulation, and verification
  dp2m-web/              Browser simulator bindings
firmware/
  m5/                    FR80 capture-pipeline assembly
  m6m/                   ARM9 UI, live-view, and applet-loader payload
  doom/                  Standalone DOOM hardware layer
include/dp2m_app.h       Shared DOOM applet ABI
web/                     Simulator frontend
data/                    Stock package and calibration data
dp2m.example.fnl         Configuration reference
```
