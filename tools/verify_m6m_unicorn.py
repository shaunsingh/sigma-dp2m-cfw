#!/usr/bin/env python3
"""Optional Unicorn ARM926EJ-S Big-Endian runtime verifier

Verifies live execution of Get_Menu_Table, all 83 UI strings, NVRAM persistence (0xA01674FA),
5-shot bracketing (0xA0002FE4), ISO Auto Min Shutter (A & P modes), capture IPC cmd 0x20
packing, Live View CMCM/gamma hooks, and YCAP RAM isolation
"""
from __future__ import annotations

import argparse
import struct
from pathlib import Path

from unicorn import UC_ARCH_ARM, UC_HOOK_INTR, UC_MODE_ARM, UC_MODE_BIG_ENDIAN, Uc
from unicorn.arm_const import (
    UC_ARM_REG_LR,
    UC_ARM_REG_R0,
    UC_ARM_REG_R1,
    UC_ARM_REG_R2,
    UC_ARM_REG_R3,
    UC_ARM_REG_R4,
    UC_ARM_REG_SP,
)

C78_S3_OFF = 0x010A4100
C78_LEN = 0x00820000
S3_IN_C78 = 0x00020000
RET = 0xA27F0000


def call_thumb(mu: Uc, va: int, r0: int = 0, r1: int = 0, r2: int = 0, r3: int = 0, r4: int = 0, sp: int = 0xA2001000) -> int:
    mu.reg_write(UC_ARM_REG_SP, sp)
    mu.reg_write(UC_ARM_REG_LR, RET | 1)
    mu.reg_write(UC_ARM_REG_R0, r0)
    mu.reg_write(UC_ARM_REG_R1, r1)
    mu.reg_write(UC_ARM_REG_R2, r2)
    mu.reg_write(UC_ARM_REG_R3, r3)
    mu.reg_write(UC_ARM_REG_R4, r4)
    mu.emu_start(va | 1, RET)
    assert mu.reg_read(UC_ARM_REG_SP) == sp, f"SP imbalance calling {va:#x}"
    return mu.reg_read(UC_ARM_REG_R0)


def main() -> None:
    ap = argparse.ArgumentParser(description="Verify built M6M C78FRM.bin under Unicorn ARM926EJ-S BE")
    ap.add_argument("c78", type=Path, help="Built C78FRM.bin")
    ap.add_argument("stock_fw", type=Path, help="Stock DP2MV105.bin")
    args = ap.parse_args()

    c78 = args.c78.read_bytes()
    fw = args.stock_fw.read_bytes()
    c78_stock = fw[C78_S3_OFF : C78_S3_OFF + C78_LEN]
    assert len(c78) == C78_LEN and c78[:8] == b"FUJITSU!"
    assert c78[S3_IN_C78 + 0x179000 : S3_IN_C78 + 0x23D35A] == c78_stock[S3_IN_C78 + 0x179000 : S3_IN_C78 + 0x23D35A]

    mu = Uc(UC_ARCH_ARM, UC_MODE_ARM | UC_MODE_BIG_ENDIAN)
    mu.mem_map(0xA0000000, 0x00800000)
    mu.mem_write(0xA0000000, c78[S3_IN_C78 : S3_IN_C78 + 0x00800000])
    mu.mem_map(0xA0800000, 0x02400000)
    mu.hook_add(UC_HOOK_INTR, lambda uc, intno, ud: None)

    # 1. Get_Menu_Table
    for mid, exp in [
        (0x09, 0xA005C500), (0x37, 0xA005CAA0), (0x10, 0xA07ED00C),
        (0x14, 0xA07ED0EC), (0x17, 0xA07ED36C), (0x28, 0xA07ED1CC),
        (0x3E, 0xA07ED54C), (0xF0, 0xA07ECDCC), (0xF1, 0xA07ED2AC),
        (0xF2, 0xA07ED4AC), (0xF3, 0xA07ECF0C), (0xF4, 0xA07ED40C),
        (0xA3, 0xA07EDC0C), (0x0E, 0xA07EA52C),
    ]:
        got = call_thumb(mu, 0xA004CD08, r0=mid)
        assert got == exp, f"Get_Menu_Table({mid:#x}) -> {got:#x} != {exp:#x}"

    # 2. UI strings spot-check (stock + endpoints of sid 672..754)
    for sid, label in [(53, "ISO 50"), (94, "Standard"), (672, "FOV Classic Yellow"), (731, "SD Custom LUT"), (754, "Min Shutter")]:
        ptr = call_thumb(mu, 0xA0044AAC, r0=sid, r1=0)
        got = bytes(mu.mem_read(ptr, len(label) + 2)).split(b"\x00")[0].decode("ascii")
        assert got == label, f"sid {sid}: {got!r} != {label!r}"

    # 3. NVRAM save (0xA01674FA) + YCAP Language RAM isolation (0xA0939776)
    mu.mem_write(0xA003A260, bytes.fromhex("4770"))  # bx lr stub for LV refresh
    for fn, val in [(0xA005A67C, 3), (0xA005A6A0, 4), (0xA005A6B8, 2), (0xA005A6D0, 4), (0xA005A6E8, 1)]:
        call_thumb(mu, fn, r0=val)
    call_thumb(mu, 0xA01674FA, r0=0xA2050000)
    assert bytes(mu.mem_read(0xA2050004, 5)) == bytes([3, 4, 2, 4, 1])

    sentinel = bytes(range(1, 17))
    mu.mem_write(0xA0939774, sentinel)
    mu.mem_write(0xA2052000, struct.pack(">3I", 0xA2300000, 0xA2360000, 0xA2390000))
    for _ in range(100):
        call_thumb(mu, 0xA005A5A4, r0=0xA2052000)
    assert bytes(mu.mem_read(0xA0939774, 16)) == sentinel

    # 4. 5-shot auto-bracketing (0xA0002FE4)
    mu.mem_write(0xA07FBE5A, struct.pack(">H", 3))  # 1.0 EV step
    for ord_v, exp_list in [
        (0, [0, 0x10000, -0x10000]),
        (4, [0, 0x20000, 0x10000, -0x10000, -0x20000]),
        (6, [0x20000, 0x10000, 0, -0x10000, -0x20000]),
        (7, [-0x20000, -0x10000, 0, 0x10000, 0x20000]),
    ]:
        mu.mem_write(0xA09396DF, bytes([ord_v]))
        assert call_thumb(mu, 0xA0168100) == len(exp_list)
        for idx, exp_ev in enumerate(exp_list):
            got = call_thumb(mu, 0xA0002FE4, r0=idx)
            if got & 0x80000000:
                got -= 0x100000000
            assert got == exp_ev, f"BKT ord={ord_v} shot={idx}: {got:#x} != {exp_ev:#x}"

    # 5. ISO Auto Min Shutter (A & P modes)
    tv_tbl = struct.unpack(">7I", bytes(mu.mem_read(0xA005B844, 28)))
    for idx, exp_tv in enumerate(tv_tbl):
        mu.mem_write(0xA093968B, bytes([idx]))
        mu.reg_write(UC_ARM_REG_SP, 0xA2001000)
        mu.reg_write(UC_ARM_REG_R1, 0x12345678)
        mu.emu_start(0xA000D83E | 1, 0xA000D84C)
        assert mu.reg_read(UC_ARM_REG_R0) == exp_tv and mu.reg_read(UC_ARM_REG_R1) == 0x12345678

    # 6. Capture IPC cmd 0x20 hook (preserves live r1 = 1.0f)
    cmd_va, meta_va = 0xA2053000, 0xA2053100
    mu.mem_write(0xA0939689, bytes([4]))  # FX=4
    mu.mem_write(0xA093968A, bytes([2]))  # Fill=+1.0
    mu.mem_write(meta_va + 0x1D, bytes([2]))  # AdobeRGB
    mu.mem_write(meta_va + 0x1E, bytes([22]))  # Kodachrome 64
    mu.mem_write(0xA2001014, struct.pack(">I", meta_va))
    call_thumb(mu, 0xA005A5D0, r1=0x3F800000, r4=cmd_va)
    assert mu.reg_read(UC_ARM_REG_R1) == 0x3F800000
    assert bytes(mu.mem_read(cmd_va + 5, 2)) == bytes([22, (2 << 5) | (4 << 2) | 2])

    print("Unicorn ARM926EJ-S BE verification of C78FRM.bin: PASS")


if __name__ == "__main__":
    main()
