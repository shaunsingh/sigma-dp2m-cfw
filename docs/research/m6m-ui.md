# M6M UI: screens, menus, strings, icons, text

Addresses are M6M S3 virtual addresses in the stock 1.05 image. "Thumb" functions are listed without the Thumb bit.

## Screens

- A screen is opened by posting `(type, id)` to `0xA0042AEC`; `type = 1` opens a screen. Stubs at `0xA0042BA8..0xA0042CF0` post fixed ids (`movs r1,#id; movs r0,#1; b 0xA0042AEC`). Container menu rows reference these stubs in `w7`.
- The dispatcher `0xA004338C` calls `*(0xA07DEB28 + 4 * id)` with `(msg, arg)`. Message 5 carries keys; message 4 carries init (0), confirm (0x12) and close (0x11) sub-messages.
- Some older notes used `0xA07DEB2C` as the table base, so their "screen N" is id N+1 here.

| Id | Handler | Opened from | Status |
| --- | --- | --- | --- |
| 21 | `0xA016A952` | | live view (`SCREEN_LV`) |
| 23 | `0xA005243E` | `0xA0042C46` | MODE screen (exposure mode strip) |
| 25 | `0xA00575E2` | | sets exposure mode 8 on entry (`0xA005742A`), likely playback |
| 26 | `0xA0044392` | | Quick Set (QS) |
| 29 | `0xA00593E0` | menu 0x17 "Camera reset" | user |
| 30 | `0xA0059758` | menu 0x0E "Auto Bracketing" | user |
| 31 | `0xA0059A4A` | menu 0x14 "Custom Mode Setting" | user |
| 32 | `0xA0059CA2` | menu 0x06 "Custom" | user |
| 33..46 | `0xA0059D7C`.. | factory menus 0x18..0x1E (Save Adjust Data, AF tests, Camfile, Font Benchmark, RAW debug, FPGA register save, Version Check) | factory only; retired by dp2m |
| 47 | | factory submenu 0xB9 (AF Debug Mode) | factory only; retired |
| 48 / 49 | `0xA005D8E4` (49) | factory menu 0x1F (MF Ring Adjust, AF PRO Parameter) | factory only; retired |
| 50 | `0xA005DA8E` | PTP command 0x66 via `0xA000FB06` | live |
| 51 | | factory menu 0x20 (Shutter Breakdown Test) | factory only |
| 52 | | menu 0x10 "Reverse Dial Direction Setting" | user |
| 54 | `0xA005ED7A` | menu 0x21 (Face detection AF, factory) | factory only |
| 59..71 | | Firmware Update, Interval Timer, ISO Auto, Key arrangement, Language, LCD Brightness, Format Card, Picture Settings, Custom QS, Date/Time, Sound, Flash | user |

dp2m points the vtable slots of ids 33..49 (`0xA07DEBAC..0xA07DEBEF`) at `dp2m_ret0` and reuses their code as the payload `CODE` region. See [m6m-rom.md](m6m-rom.md).

## Menu tables

`Get_Menu_Table` `0xA004CD08(id)` returns a row table. Literal pools near `0xA004D3D0` hold the table pointers. The pool word at `0xA004D3D0` (`0xA07EEFCC`, grid menu) is shared with other code; patch the lookup, not that word. dp2m hooks after the prologue at `0xA004CD0A` and serves payload tables first.

### Row format (32 B, big endian)

| Word | Content |
| --- | --- |
| w0 | row id; on page rows the low 16 bits are the submenu id |
| w1 | `0x01000000 \| icon`: 110x70 QS value icon |
| w2 | `(76x26 OSD icon << 16) \| 54x42 strip icon` |
| w3 | `sid << 16` |
| w4 | value written by the setter |
| w5 | getter, Thumb bit set |
| w6 | setter, Thumb bit set |
| w7 | extra handler; container rows open a screen through `0xA0042AEC` |

A table ends with an all-zero row. Shooting and Setup pages clip rows beyond 6.

### Menus used by dp2m

| Id | Stock table | Content | dp2m change |
| --- | --- | --- | --- |
| 0x05 | `0xA07EC8AC` | exposure mode: Movie 4, M 0, S 1, A 2, P 3, C1..C3 5..7 | + C4 (9), C5 (10) |
| 0x09 | `0xA07EBFEC` | color mode 1..8 (getter `0xA0167926`, setter `0xA016F86E`) | + films 9.. |
| 0x0E..0x17 | | menu pages | Film page 0xF7 inserted after 0x0F |
| 0x0F | `0xA07EA60C` | shooting page 2 | color mode row moved to the Film page |
| 0x10 | `0xA07EA6EC` | shooting page 3 | + Peaking Color |
| 0x14 | | setup page 1 (Custom Mode Setting row 0x2E) | |
| 0x17 | `0xA07EAC2C` | setup page 4 | + Imager Temp (menu 0xA3) |
| 0x28 | `0xA07EDF8C` | bracketing order | + three 5-shot orders (4, 6, 7) |
| 0x37 | `0xA07EEFCC` | grid | + 10 HUD modes (7..16) |
| 0x3E | `0xA07F188C` | ISO Auto | + Min Shutter |
| | `0xA07EAC8C` / `0xA07EADAC` | ISO 1 EV / 1/3 EV (pool words `0xA004D3DC` / `0xA004D3E0`) | + ISO 50 |
| 0xA3 | `0xA07EDC0C` | Imager Temp (hidden in stock) | linked from setup page 4 |
| 0xF0..0xF7 | | payload | paper (by class 0x1F0..0x1F3), engine, fill, min shutter, peaking color, grain, halation, Film page |

### Menu state and pages

- State struct `0xA07F22D2`: `+4` page index, `+8` row cursor (`0xA07F22DA`), `+0x3C` page list (halfword menu ids, zero terminated). A second list at `+0x14` was noted for service mode; not re-verified.
- Stock page list: `0x0E 0x0F 0x10 0x11 | 0x12 0x13 | 0x14 0x15 0x16 0x17` (shooting 0..3, playback 4..5, setup 6..9). The list is part of the image, so dp2m rewrites it in place.
- `0xA0058994(page)` maps a page to its tab group. Called from `0xA005811C`, `0xA0058144`, `0xA0058204`, `0xA00582C6`, `0xA0058308`, `0xA00584E8`.
- Page stepping wraps at index 9 (`movs r1,#9` at `0xA005868A`, `0xA00586A0`, `0xA005871C`, `0xA0058734`). The tab strip loops over 10 tabs (`0xA0058454`) at a 56 px pitch (`adds r0,#0x38` at `0xA005844A`).
- Exposure mode 8 pins the menu to the playback pages (`cmp`/`movs` with 4 and 5 at `0xA0058806..0xA0058822`).
- Tab geometry `0xA04F3100` (u16): first tab `{40, 56, 52, 2}`, group header rects `{38,3,224,51}`, `{262,3,112,51}`, `{374,3,224,51}`, group icon rects at x 132, 298, 464 (38x40).
- The main menu handler is `0xA00585C8`. Up/down (`0xA005845E`) only move the cursor. The setter runs on OK or right (`0xA00587D4` / `0xA0058786`).
- A main menu row shows its w2-low icon as the value, or the sid text (`0xA0044EFC`) when w2 low is 0. The option list (`0xA00498E4` / `0xA0049956`) draws both.

## Strings

- `0xA0044AAC(sid, lang)` returns `*(0xA07DF144 + sid * 0x44 + lang * 4)`. Rows hold 17 language pointers; 672 stock sids (0..671), sid 672 has no entry.
- Stock sids used by dp2m: Off 5, On 6, ISO 50 53, Auto 0x55, Color Mode 93, Imager Temp 463. Others seen: "Custom 1..3" 519..521, "Save settings?" 594, "Further Options" 0xF1.
- Row w0 is a row id, not a sid.
- dp2m serves sids from 672 up (English only) by hooking the lookup after its prologue at `0xA0044AAE`.

## Quick Set (QS, screen 26)

- Handler `0xA0044392`, keys `0xA004411C`, full redraw `0xA00443D4`. State `0xA07DF008`: `+4` page (QS1/QS2), `+6` strip scroll, `+8 + 2 * page` focused slot, `+0xBC` slot label positions.
- Four slots in a cross; labels at (265,24), (55,155), (475,155), (265,286). The option strip is at y = 434 with one tile every 58 px.

| Row field | Size | Used by | Read at |
| --- | --- | --- | --- |
| w1 low | 110x70 | slot value (`0xA004369C`, blit `0xA0043916`) | `0xA00438E8` |
| w2 low | 54x42 | option strip (`0xA0043E5C`); scrolling variant `0xA0043CC6` | `0xA0043F88`, `0xA0043D8C` |
| w2 high | 76x26 | not QS; `0xA0044FCC` for screens `0xA00595A4`, `0xA005E3A4`, `0xA005EBBA`, `0xA0061B44`, `0xA0062CA8` | |
| w3 sid | text | not QS; main menu only | |

- QS draws no value text. The only font text is sid 0xF1 "Further Options" (`0xA0043814..0xA00438C0`), shown for rows with a w7 handler. Slider items draw digit icons through `0xA004BE2E`. A NULL w1 icon draws nothing (no text fallback).
- The strip starts at `x = (640 - 58 * n) / 2` (`0xA0043F2E`). With 44 rows that is -956, so only items 17..27 are visible and the strip does not follow the selection. Only the ISO getter `0xA016F5AE` (compared at `0xA0043E9C`) uses the scrolling variant.
- QS commits on every step. Arrows (key codes 2..5) and dials (0x40 / 0x20) call step forward `0xA0044078` / back `0xA0043FCA`, which call the row setter (`0xA0044106` / `0xA0044062`) and redraw. Leaving QS (keys 0xA / 0xC) restores nothing.
- The LV status icon switch `0xA004C7BC` (called from `0xA0048A62`) has no case for color modes >= 9. The other OSD switch `0xA016BC4A` reads the color mode at `0xA016BC4E`; dp2m maps films to the Vivid / B&W icon there.

## Icons

- `0xA0042AAC(id)` serves ids 1..0x1A1 from `0xA07DD784 + 12 * id`: three ARM functions returning bits, width and height. The result struct is at `0xA07DD77C` (`{bits, u16 w, u16 h}`). Other ids return NULL and every caller skips the draw.
- No caller keeps a returned pointer across a second lookup. Functions with two lookups (`0xA0045982`, `0xA004AC20`, `0xA0058B04`, `0xA016AE80`, `0xA016BAE4`, `0xA016CD60`) use them as if/else alternatives. `0xA00496A8` looks up once for the width and again just before the blit. One scratch buffer per icon size is therefore enough.
- Pixels are 8 bpp indices 0..16: 0 transparent, 1 white fill, 16 black outline (the only index next to transparent pixels).
- Blit `0xA0040DD8(src, x, y, w, [sp] h, hi, lo, transparent0, mode)`. Mode 1: `out = hi + 1 - v`. Mode 0: `v < 9 -> hi`, else `lo`. Enabled rows use `hi = 0xC8, lo = 0xB9`; greyed rows `0xA8 / 0x99`.
- CLUT `0xA07DEC54`: 219 x `[A, Y, Cb, Cr]`, loaded by `0xA00434EC` into `0x600D0400 + 4 * i`. `0xB9..0xC8` is a grey ramp from Y 0x00 to 0xFF; `0x99..0xA8` from 0x00 to 0x78. `0xA0043510` selects other 16-entry ramps.
- Icons and text go to the 8 bpp index plane `0xA2130000` (flush `0xA004190E`). The 640x480 ARGB4444 overlay at `0xA21D8C00` is separate (fill `0xA00419A6`, flush `0xA0041B48`; `0xA0041C7A` merges its dirty rectangle).

Icon sizes by use:

| Use | Size | C1 / C2 / C3 example |
| --- | --- | --- |
| QS value (w1 low) | 110x70 | `0x144..0x146` |
| QS strip (w2 low) | 54x42 | `0x18E..0x190` |
| OSD (w2 high) | 76x26 | |
| MODE and Custom Mode Setting screens | 51x31 | `0xB4..0xB6` |
| LV widget 0x2E | 35x20 | `0xB7..0xB9` |
| "modified" star | 11x11 | `0x2F` |

`python -m tools.re icons --sheet icons.png` renders the stock icon set; extracted images are not kept in the repository.

### dp2m label icon ids

Payload rows use ids `class + string index` (string index = sid - 672): `0x400` 110x70, `0x800` 54x42, `0xC00` 76x26, `0x1000` 51x31, `0x1400` 35x20. The stock lookup returns NULL for them, so they stay blank until a renderer serves them. A renderer (font engine into cached tiles in payload RAM, outline by dilation) is in progress on a separate branch.

## Text

- `0xA0042684(lang, str, maxlen = -1, px, [sp] 5, 0, 1, rect *, style, 0)`; style `0x21` left, `0x22` center, `0x24` right.
- Only engine 0 is live: `0xA01683DC` always returns 0 and engines 1/2 (`0xA0042262` / `0xA0042266`) are empty stubs. The Monotype engine is compiled out; the "Ricoh / Monotype" setting has no effect.
- Fonts are "BP4" outline data: Latin `0xA04E0E02`, alternative Latin `0xA04CF64C`, Cyrillic `0xA04D8266`, plus CJK tables. Any pixel size works (24, 40 and 80 px checked in emulation).
- Glyphs rasterize via `0xA0041F9A` into `0xA2304C00`. `0xA0042606` composites the line into `0xA093A514` (640 wide, coverage 1..15, 1 = solid). `0xA0042274` measures width; `0xA004226C(num, den)` sets a scale.
- A rect at y >= 480 leaves the OSD untouched while the line buffer still holds the coverage, so text can be rendered offscreen.
- Style bits only change the color ramp; there is no outline mode.
- Cost: 0.66 to 1.9 M instructions per label in emulation. Cache rendered labels.
