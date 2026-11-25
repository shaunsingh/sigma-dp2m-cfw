# dp2m-fw

Custom Firmware, toolchain, documentation, & emulator for the Sigma DP2 Merrill

- **M6M (`C78FRM.bin`)**: ARM926EJ-S Big-Endian host/UI/Live View MCU.
- **M5Slave (`Firm.bin`)**: Fujitsu FR80 Big-Endian capture ISP and JPEG/X3F pipeline MCU.

## Requirements

- Rust (`rustc` / `cargo`, no external crate dependencies)
- `clang` with `armeb-none-eabi` target support (included in macOS Xcode Command Line Tools / LLVM)
- Stock Sigma `DP2MV105.bin` firmware image (not included; supply your own copy)
- Optional (for offline LUT compilation or ARM emulation): Python 3 with `numpy`, `scipy`, `unicorn`

## Quick Start

Copy the Fennel configuration template and set `:firmware` to your local `DP2MV105.bin`:

```sh
cp dp2m.example.fnl dp2m.fnl
```

Build and verify `Firm.bin`, `C78FRM.bin`, and combined `C78_M6M5.bin`:

```sh
cargo run --release -- build
# or pass paths directly without a config file:
cargo run --release -- build /path/to/DP2MV105.bin --out out
```

Other commands:

```sh
cargo run --release -- stage                         # copy built images to :sd {:mount ...} with read-back check
cargo run --release -- verify-m5 out/Firm.bin        # run FR80 emulation verifier on a Firm.bin image
cargo run --release -- disasm-fr80 out/Firm.bin 0x5D0000 0x5D0100
cargo run --release -- inspect out/Firm.bin
```

Custom SD-card `.cube` 3D LUTs (`D:\LUT1.BIN`..`D:\LUT8.BIN`, loaded at capture time for mode `SD Custom LUT`):

```sh
python3 tools/cube_to_lut.py input.cube /Volumes/SIGMA\ DP2M/LUT1.BIN --grain 28 --halo 64
```

Optional Unicorn ARM926EJ-S runtime check for `C78FRM.bin`:

```sh
python3 tools/verify_m6m_unicorn.py out/C78FRM.bin /path/to/DP2MV105.bin
```

## Repository Layout

```text
dp2m.example.fnl        Fennel config template
crates/
  fr80/                 Assembler, disassembler, & emulator for Fujitsu FR80 BE ('True II')
  dp2m/                 Config/FW/ELF/Verifier
src/
  asm/
    m5/                 Fujitsu FR80 asm
    m6/                 ARM926EJ-S Thumb-1 asm
  c/
    m6/                 ARM926EJ-S Thumb-1 C & headers
data/
  film_pack.bin         Pre-compiled D2MP v1 calibration tables
  npl_sigma_merrill.json NPL Foveon X3 F20 spectral sensitivity reference
tools/                  Offline Python/C utils (.cube compiler, D2MP pack builder, Unicorn check, X3F tooling)
docs/
  hardware.md           consolidated hw reference (memory maps, hooks, IPC, constraints)
```