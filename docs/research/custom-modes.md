# Custom exposure modes (C1..C3, F1, F2)

Addresses are M6M virtual addresses in the stock 1.05 image. The implementation is `src/m6m/custom.c`.

## Mode byte

| Item | Address | Notes |
| --- | --- | --- |
| Exposure mode | `0xA09396AC` (NVRAM `+0x28`) | ROM default 3 (P) |
| Getter | `0xA0167892` (`4898 3840 7a00 4770`) | 67 callers |
| Raw setter | `0xA016789A` (`strb r0,[r1,#8]`) | callers `0xA016FA00` and shutdown `0xA01694A6` |
| Full setter | `0xA016F9EC` (menu 0x05 setter) | shadow of the current mode `0xA07FBC0D`, previous mode `0xA07FBC0C` |
| Program resolver | `0xA01675DA` | 52 callers (exposure logic, OSD) |

| Value | Mode |
| --- | --- |
| 0 / 1 / 2 / 3 | M / S / A / P |
| 4 | Movie |
| 5..7 | C1..C3 |
| 8 | temporary playback mode set by `SCREEN_PB` (screen 25, `0xA005742A`) and cleared at `0xA0057B64`; shutdown (`0xA016949A`) restores the previous mode. About 30 sites test `cmp r0,#8`. |
| 9 / 10 | F1 / F2 (dp2m) |

Stock code treats values above 8 as P in the resolver, so a camera downgraded with mode 9/10 saved falls back to P.

## Slots

- A slot is a `0x8C` B copy of the current shooting settings at `0xA09396C0` (NVRAM `+0x3C`).
- C1..C3 slots: `0xA0939818 + i * 0x8C` (NVRAM `+0x194`, `+0x220`, `+0x2AC`). Backup slot: `0xA09399BC` (`+0x338`). All are inside the persisted block; the ROM default slots equal the default settings.
- Save `0xA0167584(i)`: settings to slot. Recall `0xA016759C(i)` then `0xA016F962` (re-applies about 30 settings). Compare `0xA0167618(i)`: 1 when the slot equals the settings. None of them range-check `i`; `i = 3` would hit the backup slot.
- Backup `0xA01675B6` (settings to backup) and restore `0xA01675C8`.
- Stock setter (`switch(new - 4)` at `0xA016FA2E`, table `0xA016FA34`): backup only when stepping P→C1 or Movie→C3, restore only when stepping C1→M/S/A/P or C3→Movie, and no slot reload when coming back from mode 8.
- Boot `0xA01689BC → 0xA016F400` reloads the slot for modes 5..7, so edits made in a custom mode are lost at power-off.
- Resolver for 5..7 maps settings byte `0xA09396C0` {1 → M, 2 → P, 3 → A, 4 → S, other → P}.

The persisted block has only about 159 scattered free bytes, so two more slots (`0x118` B) do not fit in NVRAM. They live in payload RAM and in the unused tail of NOR bank 0 (`0x400..0x51B`, magic `"C4C5"`); see [m6m-memory.md](m6m-memory.md).

## Screens

### MODE screen (id 23)

- Handler `0xA005243E`, opened by `0xA0042C46` (`movs r1,#0x17; movs r0,#1; bl 0xA0042AEC`, 10 callers).
- Table `0xA07F215C`: 8 x `{u32 mode, u16 icon, u16 0}` plus a zero row, order Movie, M, S, A, P, C1, C2, C3 with icons `0xAF..0xB6`. The u16 cursor is at table - 4. One pool word references it: `0xA005247C`.
- Keys 0x40 / 0x20 step through `0xA004A7C6` / `0xA004A7DE` (wrap at max 7). Each step calls the full setter, so the mode changes live.
- Constants: draw loop `cmp r4,#8` at `0xA00522A2`; max index `#7` at `0xA00522EC` and `0xA0052342`; open-time search `cmp #8` at `0xA00523AC`; x origin `(640 - 8w) / 2` at `0xA005220E`; cell rect `0xA04F30A0` (w = 60).

### Custom Mode Setting (id 31)

- Opened from menu 0x14 row 0x2E (w7 = `0xA0042C81`: `movs r1,#0x1F; movs r0,#1; b 0xA0042AEC`).
- Handler `0xA0059A4A`; keys `0xA00599A2`; draw `0xA0059790`; state `0xA07F23E4` `{u16 cursor 0..2, u16 confirm}`. OK on "Yes" (message 0x12) saves through `0xA0059A7E → 0xA0167584(cursor)`; the confirm view shows sid 594 "Save settings?".
- Icons from `0xA04F3188` `{B4, B5, B6, 0}`, copied to an 8 B stack array (`add r1/r0, sp` at `0xA0059898` / `0xA0059914`). Pool word `0xA0059AE0`.
- Layout: 3 cells, pitch `0xBC` (188 px) from x 36 (`movs #0xBC` at `0xA00597D6`, `0xA00597F0`, `0xA0059838`, `0xA0059880`). Cursor max `#2` at `0xA00599E4` / `0xA0059A02`; draw loop `cmp r4,#3` at `0xA00598F4`.

### LV widget 0x2E

- Draw `0xA016BAE4`: icons `0xB7..0xB9` (35x20), slot index per mode, "modified" star `0x2F` when `0xA0167618(slot) != 1`.
- Visibility `0xA016A7FC..0xA016A813` tests 5..7.
- Still-mode apply switch `0xA0169884` (table `0xA016988E`): modes 8 and above skip `0xA003D9FA`.

### Icons by use

| Use | C1 / C2 / C3 | Size |
| --- | --- | --- |
| MODE screen, Custom Mode Setting, menu row w2 high | `0xB4..0xB6` | 51x31 |
| LV widget 0x2E | `0xB7..0xB9` | 35x20 |
| menu row w1 low (QS value) | `0x144..0x146` | 110x70 |
| menu row w2 low (QS strip) | `0x18E..0x190` | 54x42 |
| text (w3) | sids 519..521 "Custom 1..3" | |

## dp2m implementation

| Hook | Site | Change |
| --- | --- | --- |
| menu 0x05 | payload table | adds F1 (9) and F2 (10), copied from the C3 row |
| setter tail | `0xA016FA2E` | `expo_switch(old, new)`: back up when entering the custom range from 0..4, restore when leaving it for 0..4, load the slot unless coming from 8; covers direct jumps (for example QS C2 → M) |
| resolver | `0xA01675E2` | 9/10 take the C1..C3 path |
| slot compare | `0xA016761C`, `0xA0167622`, `0xA0167628` | `slot_ptr(i)` for i = 0..4 |
| save | `0xA0059A7E` | `slot_save(i)` for i = 0..4 |
| boot reload | `0xA01689C2` | reloads slots 0..4 |
| "is custom" checks | `0xA016A7FC`, `0xA0169884`, `0xA016BAE8` | map 9/10 to 5 |
| widget icon / star | `0xA016BC26`, `0xA016BBBA` | per-slot icon and compare |
| MODE screen | `0xA005247C`, `0xA00522A2`, `0xA00522EC`, `0xA0052342`, `0xA00523AC`, `0xA005220E` | 10 cells, 60 px from x 20 |
| Custom Mode Setting | `0xA0059AE0`, `0xA0059898`, `0xA0059914`, `0xA00598F4`, `0xA00599E4`, `0xA0059A02`, cell geometry sites | 5 cells, 120 px from x 20 |
| NVRAM load / flush / reset | `0xA0040168`, `0xA00401E6`, `0xA016757E` | F1/F2 record in NOR bank 0 at `0x400` |

F1/F2 icons use label icon ids (`0x1000` / `0x1400` / `0x400` / `0x800` + string index), rendered at run time; see [m6m-ui.md](m6m-ui.md#dp2m-label-icon-ids).

Not patched because they already behave for 9/10: display-class switches `0xA0167832` / `0xA0167876`, program setter `0xA016F81E`, consumers of the effective program (`0xA004CC94`, `0xA016AC2E`), all `cmp #4` / `cmp #8` sites, QS menus 0xE5..0xEC (QS positions, not modes). `is_custom` at `0xA016F3C8` has no static references.
