# Firmware package and updater

Addresses are M6M S3 virtual addresses (`C78FRM.bin[0x20000..]` mapped at `0xA0000000`) in the stock 1.05 image. Package offsets are file offsets in `DP2MV105.bin`.

## Package layout

`DP2MV105.bin` is 25,968,896 B (`0x18C4100`).

| Offset | Size | Content |
| --- | --- | --- |
| `0x00` | 16 | magic `SIGMA.CODP2M`, zero padded |
| `0x10` | 16 | version string `1.05`, read by the updater |
| `0x30` | 16 | version string `1.05` |
| `0x40` | 16 | version string `1.05` |
| `0x50` | 8 | build date `YYYYMMDD` (`20130524`) |
| `0x64` | 4 | LE u32 file offset of the final M6M image (`0x010A4100`) |
| `0x70` | 1 | stage flags, ASCII `1` (`0x31`) |
| `0x80` | `0x820000` | segment 1: bridge M6M image (`FUJITSU!`) |
| `0x820080` | `0x884080` | segment 2: M5Slave image (`SIGMA.CODSLRSD15`, same bytes as `Firm.bin`) |
| `0x10A4100` | `0x820000` | segment 3: final M6M image (`FUJITSU!`, same bytes as `C78FRM.bin`) |

The version at `0x10` is parsed with `strtoul` on both halves of `major.minor` and printed as `%d.%02d`. The updater rejects packages below 1.05. No use of the `0x30` / `0x40` strings was traced; dp2m writes the same version to all three.

### Stage flags (`0x70`)

| Bit | Handled by | Action |
| --- | --- | --- |
| 0 | running main firmware | flash segment 1 (bridge) into the M6M flash, then reboot into it |
| 4 | bridge | flash the final M6M from the offset at `0x64` |
| 5 | bridge | flash the M5 from segment 2 (`0x820080`) |

`0x31` sets bits 0, 4 and 5, so a stock update is: flash bridge, reboot, flash M5, flash final M6M.

### Bridge image

Segment 1 is a separate M6M build. Its source-path strings reference the C79 tree (`.../C79/M6M/M6M_API/...`), while segment 3 references C78. It contains its own copy of the update scanner, file names and debug banners. dp2m keeps segment 1 stock and only replaces segments 2 and 3.

## Update code in the main M6M

| Address | Role |
| --- | --- |
| `0xA005FB04` | scan the card for update files (Setup > Firmware Update) |
| `0xA000EB04` | scan at boot |
| `0xA000ED26` | dispatch on the detected file type |
| `0xA000EC62` | flash a package |
| `0xA005F818` | update confirmation screen |
| `0xA005F87C` | `movs r2,#1; movs r3,#5`: "Current version" printed as `sprintf("%d.%02d", 1, 5)` |
| `0xA005FC3C` / `0xA005FC5C` / `0xA005FC7C` | 24-byte banner slots `M6M5 Both Debug Write.` / `M6M Only Debug Write.` / `M5 Only Debug Write.` |

File names known to the scanner (strings in both M6M images): `DP2MVxxx.bin`, `C78FRM.bin`, `Firm.bin`, `C78_M6M5.bin`.

- `DP2MVxxx.bin` shows "Current version" (from the immediates) and the new version (from header `0x10`). Because the new version comes from the package, a dp2m package already shows its own version when the stock firmware installs it.
- `C78FRM.bin`, `Firm.bin` and `C78_M6M5.bin` are debug files. Instead of versions the screen shows the matching banner and the file timestamp. There is no version check. Older dp2m builds wrote the full package layout (header and all three segments) as `C78_M6M5.bin`; the current builder does not emit it.

## dp2m changes

| What | Where | Value |
| --- | --- | --- |
| Header version strings | package `0x10`, `0x30`, `0x40` | `:version` from the config, default `2.00` |
| Header date | package `0x50` | build date, from `SOURCE_DATE_EPOCH` if set |
| File name | | `DP2MV<major><minor>.bin`, e.g. `DP2MV200.bin` |
| "Current version" immediates | `0xA005F87C` | `movs r2,#major; movs r3,#minor` (checked against stock `22 01 23 05` first) |
| Debug banners | `0xA005FC3C` / `5C` / `7C` | `dp2m-fw (M6M + M5)` / `dp2m-fw (M6M)` / `dp2m-fw (M5)` |

The banners and immediates belong to the firmware that runs the update, so they show up from the second install on. The package builder (`crates/dp2m/src/package.rs`) also checks the stock offset at `0x64` and that flag bits `0x31` are set.

## `Firm.bin` flash rules (M5Slave)

Confirmed from `0xA000CFCA`, `0xA000D3BC..0xA000D40A` and on-camera flashing:

- Error `0002` if `Firm.bin` has non-stock bytes at VA `>= 0x00800000` (outside the 8 MB NOR) or if the additive checksum at `Firm.bin[0x4C..0x50]` does not equal `sum(Firm.bin[0x80..0x800080])`.
- If error `0002` leaves the screen unresponsive, remove the battery. The camera boots from its previous flash image.

## Tools

- `cargo run --release -- inspect <file>` identifies a package, `Firm.bin` or `C78FRM.bin` and prints the package version and date.
- `python -m tools.re package --fw data/DP2MV105.bin` (RE CLI, in progress on a separate branch) dumps the header and segments.
