# M6M ROM reclamation

The S3 image has no free space: the 158 zero runs of 1 KB or more (366,571 B in total) all sit inside data blobs (fonts, icons, FPGA bitstream, LUTs) and are referenced. Payload space therefore comes from code and data that no live path reaches. This file records how that was established and what was found. The full candidate list is [m6m-rom-candidates.tsv](m6m-rom-candidates.tsv).

## Method

1. **Code map.** Recursive descent with Capstone from the reset entry `0xA0000000` (ARM), the main entry `0xA01657EA` (Thumb) and the exception vectors (`0xA07FBC1C..+0x338`). Each byte is classed as ARM, Thumb, or literal / inline table. Switches are followed through ARM `ldr pc, [pc, rm, lsl #2]` tables and the Thumb byte-table helper (`blx 0xA0173358`, table after the call). A second phase adds functions found only by their prologue (orphans).
2. **Pointer fixpoint.** Every word outside code and literal slots (excluding the FPGA bitstream) that points at a decoded function entry with the matching mode (Thumb bit set for Thumb code, word aligned for ARM) becomes a root. This covers the screen vtable, menu row getters / setters / handlers, the icon table, task config records and callback tables.
3. **Liveness.** A breadth-first walk from the roots over call and literal edges marks live functions. The pass ran on the stock image and on the patched image (factory screens retired, factory submenu tables treated as dead data), so code that only factory screens reach shows up as "newly dead".
4. **Risk checks per run.** What precedes the run (end of a function, literal pool, fall-through), how the run starts (ARM or Thumb prologue), and whether any data word anywhere points into it. Of the two ARM functions that load through `lr` (`0xA00EFD10`, `0xA0173358`), only `0xA0173358` is a switch helper, and the code map already follows it.
5. **Data.** References into data ranges are classified by whether the referencing code is live.

## Results

| Image | Functions | Live | Dead | Dead bytes |
| --- | --- | --- | --- | --- |
| stock | 11,484 | 9,635 | 1,849 | 211,330 |
| patched, factory screens retired | 11,451 | 9,569 | 1,882 | 214,408 |

After removing live islands (351) from dead runs, the final pass found 230,784 dead bytes in 894 runs: 43 runs of at least 1 KB (97,464 B), 18 of at least 2 KB, 4 of at least 4 KB, 1 of at least 8 KB. Most stock orphans are unused driver code (Im_TFR, Im_VLD, Im_HIF, Im_REC, JPEG decode, audio, SDIO).

### Regions used by dp2m

Defined in `REGIONS` in `crates/dp2m/src/m6m/mod.rs` and emitted as the linker `MEMORY` block.

| Region | Range | Size | Origin |
| --- | --- | --- | --- |
| `CODE` | `0xA0059D50..0xA005D924` | 15,316 B | factory screens 33..49; their vtable slots `0xA07DEBAC..0xA07DEBEF` now point at `dp2m_ret0` |
| `DATA` | `0xA0265670..0xA0268870` | 12,800 B | f64 table referenced only from orphaned `0xA0034870` / `0xA00348CA` |
| `LOOKS` | `0xA0094658..0xA009688C` | 8,756 B | orphaned Im_TFR driver, 23 functions, no references |
| `MENU` | `0xA07EC9CC..0xA07EDC0C` | 4,672 B | factory service menus 0x18..0x22 and their submenus; menu 0xA3 at `0xA07EDC0C` stays |
| `RAM` | `0xA1C00000..0xA1E00000` | 2 MB | unreferenced SDRAM (see [m6m-memory.md](m6m-memory.md)), `.bss` only |

The `CODE` region is contiguous: dead code below the former cave 1 (`0xA0059D50..0xA005A400`), cave 1 (`0xA005A400..0xA005BE38`, screens 33..42), cave 2S (`0xA005BE38..0xA005CDF4`, screens 43..46) and screens 47..49 (`0xA005CDF4..0xA005D924`). The builder refuses to retire a vtable slot that does not point into `CODE`.

### Unused candidates

| Range | Size | Content | Note |
| --- | --- | --- | --- |
| `0xA005DADC..0xA005E344` | 2,152 B | screen 51 (Shutter Breakdown Test, factory) | slot `0xA07DEBF4` is inside the protected live vtable range; two data words (`0xA02DDFCC`, `0xA042BE06`) point into it and need checking |
| `0xA005EB4C..0xA005EDCC` | 640 B | screen 54 (factory Face AF) | slot `0xA07DEC00`, same protection |
| `0xA07F23F0..0xA07F274C` | 860 B | factory screen state / layout | |
| `0xA04F31A8..0xA04F3594` | 1,004 B | factory screen read-only layout | |
| `0xA07EDC6C..0xA07EDE4C` | 480 B | factory submenus 0xB6, 0xA4 | |
| `0xA07F100C..0xA07F106C` | 96 B | factory submenu 0xB9 (AF Debug Mode) | referenced by live pool word `0xA004D450` |
| scattered | about 230 KB | stock orphans | largest run 8,756 B (used as `LOOKS`) |

Factory-only submenu ids: 0x5A..0x62, 0x66, 0x6B, 0x6C, 0x6F..0x78, 0x7A..0x7D, 0x7F, 0x87, 0x8A, 0x8E, 0x95, 0x97, 0x9C..0x9E, 0xA0, 0xA2, 0xA4..0xB9, 0xC7, 0xC8, 0xCC, 0xDA..0xE5 (tables total 6,016 B, mostly outside the `MENU` region).

### Screen 50

Screen 50 looks factory-only from the menus, but `0xA0042CBC` / `0xA0042CCE` post it and PTP command 0x66 (`0xA000FB06`) reaches it. It stays live, together with the screen code `0xA005D924..0xA005F1A0` and vtable `0xA07DEBF0..0xA07DEC0B`, which the builder verifies as unchanged.

## Candidate list format

`m6m-rom-candidates.tsv` columns: `start`, `end` (exclusive), `size`, `kind`, `fns` (function count), `hint` (first string referenced from the run).

| Kind | Meaning |
| --- | --- |
| `code:stock-orphan` | dead in the stock image |
| `code:newly-dead(factory)` | dead once the factory screens are retired |
| `data:*` | data referenced only by dead code |
| `menu:*` | factory menu tables |

`python -m tools.re liveness --fw data/DP2MV105.bin` (RE CLI, in progress on a separate branch) regenerates the analysis.
