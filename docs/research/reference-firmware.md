# Reference firmware

`data/ref/` holds other Sigma firmware images used for comparison and backports. The five Quattro color modes in `crates/dp2m/src/m5.rs` / `catalog.rs` / `data/film_pack.bin` (`dp2q_cmcm_4` for FOV Classic Yellow, Cinema, Sunset Red, Forest Green; `dp2q_to_mcas` for Teal & Orange) come from `DP2QV203.BIN`, and ISO 50 / 5-shot bracketing / ISO-auto minimum shutter follow sd Quattro H behaviour. All offsets below are file offsets into `data/ref/*`; "CAMF" denotes uncompressed `FCMb` v`0x20002` blocks parsed from each firmware.

| File | Camera | Version | Size |
| --- | --- | --- | --- |
| `DP2QV203.BIN` | dp2 Quattro | 2.03.0.10129 | 29,753,408 (`0x1C60040`) |
| `SDQ_V113.BIN` | sd Quattro | 1.13.0.9351 | 39,535,776 (`0x25B44A0`) |
| `SDQHV109.BIN` | sd Quattro H | 1.09.0.4027 | 41,936,544 (`0x27FE6A0`) |
| `SD1M_109.BIN` | SD1 Merrill | 1.09.0.0000 | 17,596,672 (`0x10C8100`) |
| `FP__V502.bin` | fp | 5.02 | 25,692,160 (`0x1880800`) |

## SD1 Merrill 1.09 (`SD1M_109.BIN`)

Same header layout as `DP2MV105.bin`:

| Offset | Content |
| --- | --- |
| `0x00` | `SIGMA.COSD1_MRL` |
| `0x10` | `1.09.0.0000` |
| `0x20`, `0x30`, `0x40` | `0000` |
| `0x50` | `20140212` |
| `0x58..0x63` | 12 binary bytes (`03 38 f9 37 18 b7 e5 a6 0f e7 e0 fa`), unknown |
| `0x70` | flags `0x07` (DP2M uses `0x31`) |

- Segment 2 starts at `0x8C4080` with `SIGMA.CODSLR_SD1` and the version string, the same FR (TRUE II) header layout as DP2M M5 (`SIGMA.CODSLRSD15`). Segment 1 (`0x80..0x8C4080`) starts with zeros and has no `FUJITSU!` magic.
- CAMF strings name the F20 sensor (`Camsettings-F20-SD1Merrill-Rev3.0`, `Converted using Foveon CamXML2Bin 1.2.15.3194`).
- **CAMF vs `DP2MV105.bin` (verified)**:
  - `CMCM_FCBlue`: SD1M `[1.60778, 0.291276, -0.899056, 0.383872, 1.36512, -0.748988, 0.148312, 0.0812079, 0.770481]` vs DP2M `[1.55391, 0.357035, -0.910933, 0.367226, 1.4933, -0.860522, 0.123477, 0.176646, 0.699866]` (Q10 `[1591, 366, -933, 376, 1529, -881, 126, 181, 717]`; already stock DP2M color mode 8).
  - `CMCM_Portrait` bottom row: SD1M `[0.07968637, -0.08498758, 1.005301]` vs DP2M `[0.02, -0.02, 1.0]`.
  - `TCColorModeSettings` rows differ: `[1, -1.17, 1.65, 3, 0.45, 0.1, 1.7]` vs `[1, -1.17, 1.55, 3, 0.45, 0.1, 1.85]`.

## dp2 Quattro 2.03 (`DP2QV203.BIN`)

- Header: `0x00` `SIGMA.CODP2Q`, `0x10` `2.03.0.10129`; `0x20` three BE `{offset, length}` pairs `{0x40, 0xE20000}`, `{0xE20040, 0xE20000}`, `{0x1C40040, 0x20000}`. The first two segments start with `SIGMA_FW` sub-headers; `FUJITSU!` appears at `0xEEFE0` and `0xF0FF28`.
- **In-camera CMCM table (verified by table read)**: Q16 little-endian, 36 B/entry at `0x479548` (copies at `0x50CFE4`, `0x12A5A28`, `0x132F02C`; `SDQHV109.BIN` at `0x10A19F8`, `0x112AFB8`; `SDQ_V113.BIN` at `0x331364`).
  - Entry order (0..10): Standard, Vivid, Neutral, Portrait, Landscape, FCBlue, SunsetRed, ForestGreen, Cinema, FCYellow, TealAndOrange.
  - **Entry 10 — Teal & Orange (`0x4796B0`)**:
    - Raw Q16: `[69910, 3773, -8136, 16440, 25096, 24009, -1094, -2747, 69385]`
    - Floats: `[1.066742, 0.057571, -0.124146, 0.250854, 0.382935, 0.366348, -0.016693, -0.041916, 1.058731]`
    - Q10: `[1092, 59, -127, 257, 392, 375, -17, -43, 1084]` (each row sums to 1024).
    - `dp2m` uses this Q10 matrix for both the packed-mode-13 capture recipe and its Live View preview, paired with `dp2q_to_mcas`. No T&O entries exist in any CAMF block; values exist only in the in-camera tables.
  - Following the CMCM table: 7 saturation-step matrices at `0x4796D4..0x4797AC`, then identity floats at `0x4797D0`.
- **In-camera MCK / MCB / MCID tables (verified; MCK mode order inferred by alignment with CMCM table order)**:
  - **MCK** at `0x24816C`: LE `i16`, 12 sets × 216 B. Set 0 Standard; 1 Vivid (differs in sectors 3..5, 9..11); 2 Neutral; 3 Portrait (= Vivid in sectors 3..5); 4 Landscape (= Vivid in sectors 9..11); 5..9 (FCBlue, SunsetRed, ForestGreen, Cinema, FCYellow) identical to Standard; 10 Teal & Orange (= `dp2m`'s `dp2q_to_mcas` exactly); 11 zero. (`dp2m` uses identity MCK for Quattro modes 9..12 rather than Quattro's Standard MCK.)
  - **MCB** (sector boundaries) at `0x247FE4`: 11 identical sets `24732, 18978, 13789, 8036` × 4 (= CAMF MCB = stock DP2M MCB).
  - **MCID** tables: `0x248144` `[0,0,1,2,0,0,3,4,5,0,6,7,8,0,0,0,9,10,11,0]` and `0x248158` `[0,0,0,1,0,2,3,4,5,6,0,7,8,9,0,0,0,10,11,0]`. **Inference (unverified)**: the second table belongs to Teal & Orange; selecting code has not been located.
- **CAMF contrast (`CMCC`, from `0x4816B0`, verified)**: Standard `0`, Vivid `+0.3`, Neutral `-0.3`, Portrait `-0.25`, Landscape `+0.25`, FCBlue `+0.3`; FCYellow, Cinema, ForestGreen, SunsetRed, Monochrome `0`. No per-mode tone curves exist in any Quattro CAMF, so `dp2m` using the stock Standard tone curve for Quattro modes matches stock Quattro behaviour.
- **CAMF `CMCM_FCBlue` (`0x47E7D0`, verified)**: floats `[0.52138, 2.25495, -1.77632, -0.34919, 2.849225, -1.500019, -0.07509, 1.029421, 0.045695]`, Q10 `[534, 2309, -1819, -358, 2918, -1536, -77, 1054, 47]`, `CMCC +0.3`, MCK = Standard.

## sd Quattro 1.13 (`SDQ_V113.BIN`) and sd Quattro H 1.09 (`SDQHV109.BIN`)

- `0x00` `SIGMA.COsd Quat ` / `SIGMA.COsd QuatH`, `0x10` version, `0x1C` `00 00 07 20`.
- From `0x20`, 16-byte entries `{u8 tag, u8 sub, 6 × 0, LE u32 offset, LE u32 length}`:

| Tag | Sub | sd Quattro H offset / length | Start of data |
| --- | --- | --- | --- |
| `0x01` | 2 | `0xA0` / `0xE20000` | `SIGMA_FW` sub-header |
| `0x01` | 1 | `0xE200A0` / `0xC69C00` | little-endian ARM vector table (`18 f0 9f e5` = `ldr pc, [pc, #0x18]`) |
| `0x02` | 1 | `0x1A89CA0` / `0x620000` | `SIGMA_FW` sub-header |
| `0x04` | 0 | `0x20A9CA0` / `0x40000` | Cortex-M style vector table (SP `0x20001B00`, reset `0x8141`) |
| `0x08` | 0 | `0x20E9CA0` / `0x24F700` | `G4M-Designer` header (length 0 on sd Quattro) |
| `0x10` | 0 | `0x23393A0` / `0x9F500` | `LENSDATA_VER0013` |
| `0x20` | 0 | `0x23D88A0` / `0x425E00` | `SL_Old_Lens_v004` (sd Quattro: `v006`) |

- The main ARM image is little-endian (DP2M M6M is big-endian). Version print format on sd Quattro H: `%s %s ver%d.%d.0.%04X`.

## fp 5.02 (`FP__V502.bin`)

- ASCII header: `LENGTH=24503296 w71c1 VER=V91 DVR=5.02 SUM=2768501654 IPL PTBL DAT1` (`LENGTH` `0x175E400` < file size `0x1880800`).
- **Payload (verified)**: LZSS (4096-byte ring, initial offset `0xFEE`) from `0x300`. Contains mode-name lists only (`Standard`, `Vivid`, `Neutral`, `Portrait`, `Landscape`, `Cinematic`, `WarmGold`, `TealAndOrange`, `SunsetRed`, `ForestGreen`, `PowderBlue`, `FCBlue`, `FCYellow`, `DuoTone`, `Monochrome`, `OFF`). No numeric color tables were found, so Powder Blue, Warm Gold, and Duotone are not backportable as numbers.

## Open questions

1. Whether MCID at `DP2QV203.BIN` `0x248158` is the Teal & Orange sector index (and whether `dp2m`'s T&O needs a distinct MCID).
2. Whether `dp2m` Quattro modes 9..12 should use the Standard MCK (as stock Quattro does) rather than identity MCK.
3. Unidentified `DP2QV203.BIN` curve tables: 256-entry `u16` at `0x46AF68` / `0x46B168` / `0x46B368` (0..2046), `0x46E768` / `0x46E968` (0..2608), `u8` curves `0x820724..0x821538`, and eight 2048-entry `u16` near-linear LUTs at `0x50DB7C`.
