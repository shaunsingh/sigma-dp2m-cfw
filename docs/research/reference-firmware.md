# Reference firmware

`data/ref/` holds other Sigma firmware for comparison and backports. Only the package headers and top-level structure were examined; nothing from these images is used by the build.

| File | Camera | Version | Size |
| --- | --- | --- | --- |
| `DP2QV203.BIN` | dp2 Quattro | 2.03.0.10129 | 29,753,408 (`0x1C60040`) |
| `SDQ_V113.BIN` | sd Quattro | 1.13.0.9351 | 39,535,776 (`0x25B44A0`) |
| `SDQHV109.BIN` | sd Quattro H | 1.09.0.4027 | 41,936,544 (`0x27FE6A0`) |
| `SD1M_109.BIN` | SD1 Merrill | 1.09.0.0000 | 17,596,672 (`0x10C8100`) |
| `FP__V502.bin` | fp | 5.02 | 25,692,160 (`0x1880800`) |

Inspect with `xxd -l 256 <file>` and `strings -n 8 <file>`.

## SD1 Merrill 1.09

Same header layout as `DP2MV105.bin`:

| Offset | Content |
| --- | --- |
| `0x00` | `SIGMA.COSD1_MRL` |
| `0x10` | `1.09.0.0000` |
| `0x20`, `0x30`, `0x40` | `0000` |
| `0x50` | `20140212` |
| `0x58..0x63` | 12 binary bytes (`03 38 f9 37 18 b7 e5 a6 0f e7 e0 fa`), unknown |
| `0x70` | flags `0x07` (DP2M uses `0x31`) |

- Segment 2 starts at `0x8C4080` with `SIGMA.CODSLR_SD1` and the version string, the same kind of FR (TRUE II) image header as the DP2M M5 (`SIGMA.CODSLRSD15`). It is the closest reference for M5 work.
- Segment 1 (`0x80..0x8C4080`) starts with zeros and has no `FUJITSU!` magic; not analysed.
- CAMF strings name the F20 sensor (`Camsettings-F20-SD1Merrill-Rev3.0`, `Converted using Foveon CamXML2Bin 1.2.15.3194`), the same sensor as `data/npl_sigma_merrill.json`.

## dp2 Quattro 2.03

- `0x00` `SIGMA.CODP2Q`, `0x10` `2.03.0.10129`.
- `0x20`: three BE `{offset, length}` pairs: `{0x40, 0xE20000}`, `{0xE20040, 0xE20000}`, `{0x1C40040, 0x20000}`.
- The first two segments start with a `SIGMA_FW` sub-header (tag bytes `b` and `c`, then LE u32 2 and `05 05 04 05 ...`). The third starts with zeros.
- `FUJITSU!` appears at `0xEEFE0` and `0xF0FF28`, inside the segments.
- The Quattro looks in the film pack (`dp2q_cmcm_4`, `dp2q_to_mcas` in `crates/dp2m/src/pack.rs`) come from this camera.

## sd Quattro 1.13 and sd Quattro H 1.09

- `0x00` `SIGMA.COsd Quat ` / `SIGMA.COsd QuatH`, `0x10` version, `0x1C` `00 00 07 20`.
- From `0x20`, 16-byte entries `{u8 tag, u8 sub, 6 x 0, LE u32 offset, LE u32 length}`. Tags are single bits, like the DP2M stage flags:

| Tag | Sub | sd Quattro H offset / length | Start of data |
| --- | --- | --- | --- |
| `0x01` | 2 | `0xA0` / `0xE20000` | `SIGMA_FW` sub-header |
| `0x01` | 1 | `0xE200A0` / `0xC69C00` | little-endian ARM vector table (`18 f0 9f e5` = `ldr pc, [pc, #0x18]`) |
| `0x02` | 1 | `0x1A89CA0` / `0x620000` | `SIGMA_FW` sub-header |
| `0x04` | 0 | `0x20A9CA0` / `0x40000` | Cortex-M style vector table (SP `0x20001B00`, reset `0x8141`) |
| `0x08` | 0 | `0x20E9CA0` / `0x24F700` | `G4M-Designer` header (length 0 on the sd Quattro) |
| `0x10` | 0 | `0x23393A0` / `0x9F500` | `LENSDATA_VER0013` |
| `0x20` | 0 | `0x23D88A0` / `0x425E00` | `SL_Old_Lens_v004` (sd Quattro: `v006`) |

- The main ARM image is little-endian; the DP2M M6M is big-endian, so code cannot be copied across, only logic and constants.
- Version print format on the sd Quattro H: `%s %s ver%d.%d.0.%04X`.
- The ISO 50, 5-shot bracketing and ISO-auto minimum shutter features in dp2m follow the sd Quattro H behaviour (see [../hardware.md](../hardware.md#backports-and-peripherals)).

## fp 5.02

Different platform. The file starts with an ASCII header padded with spaces: `LENGTH=24503296 w71c1 VER=V91 DVR=5.02 SUM=2768501654 IPL PTBL DAT1`. `LENGTH` is smaller than the file (`0x175E400` vs `0x1880800`). Not analysed further.
