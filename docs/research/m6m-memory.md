# M6M memory: SDRAM, NVRAM, NOR settings record

Addresses are M6M virtual addresses in the stock 1.05 image.

## Image

`C78FRM.bin` (8,519,680 B) is a `0x20000` B IPL followed by the 8 MB S3 image. S3 is mapped at `0xA0000000` and runs from SDRAM, so every byte of it (code, tables, menus) is writable at runtime.

## SDRAM map

SDRAM is 256 MB (`0xA0000000..0xB0000000`; hard-coded references reach `0xAF7EB1C0`).

| Range | Size | Use | Evidence |
| --- | --- | --- | --- |
| `0xA0000000..0xA0800000` | 8 MB | S3 image | |
| `0xA0179000..0xA023D35A` | 803,674 B | Spartan-6 FPGA bitstream (length at `0xA07AE444`) | corrupting it breaks FPGA configuration (white screen, dead shutter) |
| `0xA07FBC1C..0xA0A1F398` | 2.2 MB | zero-init RAM: statics, µITRON pools, task control blocks, text line buffer `0xA093A514` (160 KB), NVRAM block | scatter table `0xA07AD3CC` |
| `0xA0A1F398..0xA1B1F398` | 17 MB | C heap | `0xA01657E4` returns base / limit |
| `0xA1B1F3A0..0xA1B5717C` | 224 KB | µITRON task stacks | 46 config records `0xA0561EE4..0xA05623E8` |
| `0xA1B57180..0xA2130000` | 5.9 MB | no references found | payload `.bss` at `0xA1C00000..0xA1CB7E58`, `D2AP` standalone applet arena at `0xA1CC0000..0xA1E00000` |
| `0xA2130000..` | | OSD planes (8 bpp index plane `0xA2130000`, RGBA4444 overlay `0xA21D8C00`), glyph raster `0xA2304C00`, LV and capture buffers | literals in `0xA0165938`, `0xA00419B0`, `0xA0041FAC` |

Scatter table at `0xA07AD3CC` (16 B entries `{load, exec, size, fn}`):

| Entry | Load | Exec | Size | Function |
| --- | --- | --- | --- | --- |
| `0xA07AD3CC` | `0xA07FBC1C` | `0x00000000` | `0x338` | copy (`0xA000090C`): exception vectors to on-chip RAM |
| `0xA07AD3DC` | `0xA07FBC1C` | `0xA07FBC1C` | `0x22377C` | zero fill (`0xA0000934`) |

So the image bytes from `0xA07FBC1C` to the end of S3 are the vector source and are then cleared. Nothing placed there survives boot.

The free range has no PC-relative literal or config pointer into it; the hits are false decodes inside ASCII or Thumb data, and the boot mode stacks are in on-chip RAM (`0x4200..0x8000`). Static analysis cannot rule out runtime-computed addresses or DMA from the M5, so the payload uses a 2 MB window away from both edges. Payload RAM is not cleared at boot; code that needs a known state checks a magic value (see `cs_ready` in `src/m6m/custom.c`).

## NVRAM block

The settings block lives at `0xA0939684` (`0x44C` B in RAM, initialised from ROM defaults `0xA07F9E68`, `ldr r4,=0x44C` at `0xA01673EC`). The first `0x3C4` B are persisted; the tail `0xA0939A48..0xA0939ACF` is reset from ROM on every boot (`0xA01673FE..0xA0167416`).

| Offset | Address | Content |
| --- | --- | --- |
| `+0x00` | `0xA0939684` | version magic `0x20120918` |
| `+0x04..+0x13` | `0xA0939688..0xA0939697` | reserved, zero in the ROM defaults, no stock accessor |
| `+0x14..+0x16` | `0xA0939698..` | display level per mode class, 0..6 (getters `0xA0167824` / `0xA0167864`) |
| `+0x28` | `0xA09396AC` | exposure mode (see [custom-modes.md](custom-modes.md)) |
| `+0x3C..+0xC7` | `0xA09396C0` | current shooting settings (`0x8C` B, the unit a custom slot stores) |
| `+0x3E` / `+0x3F` | `0xA09396C2` / `0xA09396C3` | read by the color mode getter `0xA0167926`; `+0x3F` is the color mode |
| `+0xF2` | `0xA0939776` | language |
| `+0xF4` | `0xA0939778` | LCD settings |
| `+0x194..+0x337` | `0xA0939818` | C1..C3 slots (3 x `0x8C`) |
| `+0x338..+0x3C3` | `0xA09399BC` | pre-custom backup (`0x8C`) |

Bytes that no code reads or writes (static scan): `+0x20..+0x27`, `+0x29..+0x3B`, `+0xCD..+0xEF`, `+0x129..+0x167`, `+0x172..+0x193`, about 159 B, plus 46 B at `+0x9A..+0xC7` inside the settings struct.

`+0xF0..+0xFF` must not be used as scratch: writing a counter there cycles the UI language on every menu exit.

### dp2m bytes (reserved area)

| Address | Offset | Setting | Values (`src/m6m/hud.c`, `ui.c`) |
| --- | --- | --- | --- |
| `0xA0939688` | `+0x04` | color print paper | 0 = film's Auto paper, 1..8 |
| `0xA0939689` | `+0x05` | 3D spectral engine | 0 HW Fast, 1 3D Spectral (fx bit 0) |
| `0xA093968A` | `+0x06` | X3 fill | 0 Off, 1 +0.5, 2 +1.0, 3 -0.5 |
| `0xA093968B` | `+0x07` | ISO-auto min shutter | 0..6 |
| `0xA093968C` | `+0x08` | peaking color | 0 red, 1 yellow, 2 white, 3 black |
| `0xA093968D` | `+0x09` | grain | 0/1 (fx bit 1) |
| `0xA093968E` | `+0x0A` | halation | 0/1 (fx bit 2) |
| `0xA093968F` | `+0x0B` | B&W paper | 0 = Auto, 1..8 |
| `0xA0939690` | `+0x0C` | SD LUT slot | 0 = Auto, 1..8 |
| `0xA0939691` | `+0x0D` | peaking sensitivity | 0 Medium (`PEAKING_THRESH`, default 44), 1 High (24), 2 Low (72), 3 Auto (High when `MF_SPOT_ACTIVE` `0xA0939A53` != 0, else Medium) |
| `0xA0939692` | `+0x0E` | zebra level | 0 95% IRE (`ZEBRA_THRESH`, default 242), 1 90% IRE (230), 2 100% IRE (252), 3 Dual Hi+Lo (`>= ZEBRA_THRESH` orange + `<= 12` cyan) |

`+0x0F..+0x13` (`0xA0939693..0xA0939697`) are free. Getters clamp out-of-range values to 0.

Only `+0x04..+0x13` is reserved. Writing volatile data at `+0x14..` (for example live-view plane pointers every frame) overwrites the display-level bytes, and the next flush persists them. Volatile state belongs in payload `.bss`.

## NOR settings record (bank 0)

| Item | Detail |
| --- | --- |
| Geometry | 2 KB: 4 x 512 B sectors from sector 0, mirror at sectors `0x100..0x103`; the read path falls back to the mirror |
| Write | `0xA00206FA(off, n, buf)`: reads the bank into the cache at `[0xA07AF490]`, copies the new bytes, erases and rewrites primary and mirror |
| `0x00..0x03` | magic (`0x20120918`) |
| `0x08..0x0F` | header (`0xA07DD720`, `0xA07DD722`, `0xA001B026()`) |
| `0x10..0x1F` | used by `0xA001D35E`, `0xA001D5BC`, `0xA01702FA` |
| `0x20..0x3E3` | NVRAM block (`0x3C4` B) |
| `0x3E4..0x7FF` | no accessor (every caller of the bank-0 helpers checked) |
| `0x400..0x51B` | dp2m: `"C4C5"` magic + 2 x `0x8C` (F1/F2 slots) |

- **Flush** `0xA004017A` (callers `0xA0061430`, `0xA01694AA`, `0xA0169790`, `0xA01697A8`): allocates `0x800` B, reads the header, copies the block with `0xA01674FA` (`0xA0167538`), writes `0xA00206FA(8, 0x3DC, buf)` at `0xA00401E6`. dp2m extends this write to `0x514` B with the F1/F2 record; still one erase cycle.
- **Load** `0xA0165912 -> 0xA01673C2 -> 0xA0040110`: reads `0x800` B and copies `buf + 0x20` into RAM at `0xA0040168`. Version `0x20120605` (via `0xA0166DE8`, `0x1C0` B) and `0x20120725` (via `0xA016675C`, `0x1D4` B) are migrated; anything else gets ROM defaults. dp2m copies `buf + 0x400` into payload RAM in the same call.
- **Camera reset** `0xA0167574` restores the block from ROM (`0xA016757E`); dp2m also resets F1/F2 there.
- Other banks seen as helper arguments: bank 1 `(0x200, 0x300, 0x10)`, bank 2 `(0x400, 0x2000, 0x1C00)`, bank 3 `(0x3C00, 0x4500, 0x900)`; not analysed.
