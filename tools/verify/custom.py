"""custom suite: C4/C5 custom exposure modes (slots, NOR persistence, mode switching, MODE & Custom Mode Setting screens)."""

from __future__ import annotations

import struct

from tools.re import stock
from tools.re.emu import STACK, Machine, outside

from .harness import Build, Nor, Report

SLOTS, BACKUP, SZ = 0xA0939818, 0xA09399BC, 0x8C  # C1..C3 slots, pre-custom backup, slot size
SHADOW, PREV = 0xA07FBC0D, 0xA07FBC0C  # setter's current / previous mode
CS_MAGIC, NV_MAGIC = 0x43344335, 0x20120918
CUSTOM_STATE = 0xA07F23E4  # Custom Mode Setting screen {u16 cursor, u16 confirm}
FAKE_ICON = 0xA1F00000
HDR = (0xA07DD720, 0xA07DD728)  # stock NOR header globals the load refreshes
SLOT_OF = {5: 0, 6: 1, 7: 2, 9: 3, 10: 4}

STUBS = {  # name: (va, ret)
    "prog_set": (0xA016F81E, None), "f_1CEDE": (0xA001CEDE, None), "f_403EE": (0xA00403EE, None), "f_7B42": (0xA0167B42, None),
    "apply": (0xA016F962, None), "still": (0xA003D9FA, None), "movie": (0xA003E85A, None), "f_80FA": (0xA01680FA, 0),
    "f_809A": (0xA016809A, 1), "clear": (stock.FILL_BOX, None), "blit": (stock.BLIT, None), "f_37C68": (0xA0037C68, None),
    "text": (0xA00418AE, None), "f_4BB98": (0xA004BB98, None), "redraw": (0xA005246C, None), "f_4190E": (0xA004190E, None),
    "f_41B48": (0xA0041B48, None), "close": (0xA0042B80, None), "str": (stock.UI_STRING, 0), "f_3E78C": (0xA003E78C, 0),
    "f_3F418": (0xA003F418, 0),
}
HOOK_SITES = (
    (0xA0059A7E, "slot_save"), (0xA016761C, "slot_ptr"), (0xA016FA2E, "expo_switch_hook"), (0xA01675E2, "program_hook"),
    (0xA01689C2, "expo_boot_reload"), (0xA016A7FC, "expo_mode_c1"), (0xA0169884, "expo_mode_c1"), (0xA016BAE8, "expo_mode_c1"),
    (0xA016BC26, "widget_icon"), (0xA016BBBA, "widget_slot_same"), (0xA0040168, "nv_load_copy"), (0xA00401E6, "nv_flush_write"),
    (0xA016757E, "nv_reset_copy"),
)
IMMEDIATES = {
    0xA00522A2: 0x2C0A, 0xA00522EC: 0x2109, 0xA0052342: 0x2109, 0xA00523AC: 0x280A, 0xA005220E: 0x2082, 0xA0052210: 0x0080,
    0xA0059898: 0x4991, 0xA0059914: 0x4872, 0xA00598F4: 0x2C05, 0xA00599E4: 0x2104, 0xA0059A02: 0x2104, 0xA0167622: 0x38FF,
    0xA0167628: 0x3895,
}


def pat(tag: int, prog: int) -> bytes:
    """Recognizable slot contents with program byte `prog`."""
    return bytes([prog] + [(tag + i) & 0xFF for i in range(1, SZ)])


def run(b: Build, r: Report) -> None:
    b.require("cs", "expo_switch", "slot_ptr", "mode_strip", "custom_icons")
    img, cat = b.img, b.cat
    cs, cs_end = b.sym("cs"), b.sym("cs") + b.syms["cs"].size
    si4, si5 = cat["SID_C4"] - stock.STOCK_SIDS, cat["SID_C5"] - stock.STOCK_SIDS
    mode_icon, widget_icon = cat["ICON_LABEL_MODE"], cat["ICON_LABEL_WIDGET"]
    rom_cur = img.rd(stock.NV_DEFAULTS + 0x3C, SZ)
    m = Machine(img)
    m.load_nv_defaults()
    calls: list[tuple[str, list[int]]] = []
    rects: list[tuple[int, ...]] = []

    def record(name: str, then=None):
        def on(a: list[int]) -> None:
            calls.append((name, a))
            if then:
                then(a)
        return on

    def icon_dims(a: list[int]) -> None:
        i = a[0]
        w, h = ((51, 31) if i in range(0xAF, 0xB7) or i >> 10 == mode_icon >> 10 else (35, 20) if i in range(0xB7, 0xBA)
                or i >> 10 == widget_icon >> 10 else (13, 16) if i in (0x96, 0x97) else (11, 11))
        m.wr(FAKE_ICON, struct.pack(">IHH", FAKE_ICON + 0x100, w, h))

    def rect_at(a: list[int]) -> None:
        rects.append(struct.unpack(">4H", m.rd(a[0], 8)))

    for name, (va, ret) in STUBS.items():
        m.stub(va, ret=ret, on=record(name))
    m.stub(0xA0041474, on=record("hilite", rect_at))
    m.stub(0xA0044B76, on=record("box", rect_at))
    m.stub(stock.ICON_GET, ret=FAKE_ICON, on=record("icon", icon_dims))
    nor = Nor(m)

    def call(va: int, *args: int) -> int:
        calls.clear()
        writes.clear()
        rects.clear()
        return m.call(va, *args, count=2_000_000)

    def stray(*allowed: tuple[int, int]) -> list[int]:
        return outside(writes, STACK, *allowed)

    def slot(i: int) -> bytes:
        return m.rd(SLOTS + i * SZ, SZ) if i < 3 else m.rd(cs + 4 + (i - 3) * SZ, SZ)

    def seq() -> list[str]:
        return [c[0] for c in calls if c[0] in ("apply", "still", "movie")]

    nv = (stock.NV_BLOCK, stock.NV_BLOCK + stock.NV_LEN)
    csr, sh, buf = (cs, cs_end), (PREV, SHADOW + 1), (Nor.BUF, Nor.BUF + 0x800)
    cs_rom = struct.pack(">I", CS_MAGIC) + rom_cur * 2
    cs_test = struct.pack(">I", CS_MAGIC) + pat(0x50, 3) + pat(0x60, 4)

    # static: hook call sites point at the payload
    for site, sym in HOOK_SITES:
        r.eq(img.bl_target(site), b.sym(sym), f"BL @ {site:#x} -> {sym}")
    r.eq(img.u32(0xA005247C), b.sym("mode_strip") + 4, "MODE pool word -> mode_strip.cell")
    r.eq(img.u32(0xA0059AE0), b.sym("custom_icons"), "Custom Mode Setting pool word -> custom_icons")
    r.eq({a: img.u16(a) for a in IMMEDIATES}, IMMEDIATES, f"{len(IMMEDIATES)} immediates")

    with m.writes() as writes:
        # NOR load (boot) with and without the record
        nv_img = bytearray(img.rd(stock.NV_DEFAULTS, stock.NV_PERSIST))
        nv_img[0x3C : 0x3C + SZ] = pat(0x10, 2)
        nor.data[0:4] = struct.pack(">I", NV_MAGIC)
        nor.data[0x20:0x3E4] = nv_img
        nor.data[0x400:0x51C] = cs_test
        m.wr(cs, b"\xA5" * 0x11C)
        call(stock.NV_LOAD, stock.NV_BLOCK)
        r.check(m.rd(stock.NV_BLOCK, stock.NV_PERSIST) == bytes(nv_img), "load: stock 0x3C4 B copied")
        r.check(m.rd(cs, 0x11C) == bytes(nor.data[0x400:0x51C]), "load: C4/C5 record restored from NOR 0x400")
        r.eq(stray(nv, csr, HDR, buf), [], "load: no stray writes")
        nor.data[0x400:0x404] = b"\xFF" * 4
        call(stock.NV_LOAD, stock.NV_BLOCK)
        r.check(m.rd(cs, 0x11C) == cs_rom, "load: missing magic -> ROM defaults")
        nor.data[0x400:0x404] = struct.pack(">I", CS_MAGIC)
        call(stock.NV_LOAD, stock.NV_BLOCK)

        m.wr(SLOTS, pat(0x20, 1) + pat(0x30, 2) + pat(0x40, 3))
        m.wr(BACKUP, pat(0x70, 0))
        nor.writes.clear()
        m.wr(Nor.BUF, b"\xEE" * 0x800)
        call(stock.NV_FLUSH)
        r.eq([w[:2] for w in nor.writes], [(8, 0x514)], "flush: one NOR write (8, 0x514)")
        wbuf = nor.writes[0][2]
        r.check(wbuf[0x18:0x3DC] == m.rd(stock.NV_BLOCK, stock.NV_PERSIST), "flush: stock NVRAM block at NOR 0x20")
        r.check(wbuf[0x3DC:0x3F8] == bytes(0x1C), "flush: NOR 0x3E4..0x3FF zeroed")
        r.check(wbuf[0x3F8:0x514] == cs_test, "flush: magic + slots 3/4 at NOR 0x400")
        r.eq(stray(nv, buf), [], "flush: no stray writes")
        nor.data[8 : 8 + 0x514] = wbuf
        m.wr(cs, bytes(0x11C))
        call(stock.NV_LOAD, stock.NV_BLOCK)
        r.check(m.rd(cs + 4, 2 * SZ) == cs_test[4:], "flush -> load round trip")
        m.wr(cs, b"\xA5" * 0x11C)
        call(stock.NV_FLUSH)
        r.check(nor.writes[-1][2][0x3F8:0x514] == cs_rom, "flush with uninitialized RAM -> ROM defaults")
        m.wr(cs, cs_test)

        saved = m.rd(stock.NV_BLOCK, stock.NV_LEN)
        call(stock.NV_RESET)
        r.check(m.rd(stock.NV_BLOCK, stock.NV_PERSIST) == img.rd(stock.NV_DEFAULTS, stock.NV_PERSIST), "reset: NVRAM block = ROM")
        r.check(m.rd(cs, 0x11C) == cs_rom, "reset: C4/C5 = ROM defaults")
        r.eq(stray(nv, csr), [], "reset: no stray writes")
        m.wr(stock.NV_BLOCK, saved)
        m.wr(cs, cs_test)

        # program resolver: custom modes take their slot's program, 9/10 like C1..C3
        prog_mode = {1: 0, 2: 3, 3: 2, 4: 1}
        bad = []
        for prog in range(6):
            m.w8(stock.NV_CUR, prog)
            for mode in range(11):
                m.w8(stock.NV_EXPO, mode)
                got, exp = call(0xA01675DA), mode if mode <= 4 or mode == 8 else prog_mode.get(prog, 3)
                if got != exp:
                    bad.append((mode, prog, got, exp))
        r.eq(bad, [], "resolver: modes 0..10 x program 0..5")

        # setter transitions
        p_set = pat(0x10, 2)

        def trans(label: str, old: int, new: int, cur: bytes, exp_cur: bytes, exp_backup: bytes | None, exp_seq: list[str]) -> None:
            m.w8(stock.NV_EXPO, old)
            m.w8(SHADOW, old)
            m.wr(stock.NV_CUR, cur)
            b0 = m.rd(BACKUP, SZ)
            call(0xA016F9EC, new)
            got = (m.rd(stock.NV_CUR, SZ) == exp_cur, m.rd(BACKUP, SZ) == (b0 if exp_backup is None else exp_backup), seq(),
                   (m.u8(stock.NV_EXPO), m.u8(SHADOW), m.u8(PREV)), stray(nv, csr, sh))
            r.eq(got, (True, True, exp_seq, (new, new, old), []), f"setter {label}")

        m.wr(BACKUP, pat(0x70, 0))
        trans("P->C4", 3, 9, p_set, slot(3), p_set, ["apply"])
        edit = bytearray(slot(3))
        edit[5] ^= 0xFF
        trans("C4(edited)->C5", 9, 10, bytes(edit), slot(4), None, ["apply"])
        r.check(slot(3) != bytes(edit), "C4 edits not written back to slot 3 without save")
        trans("C5->M", 10, 0, slot(4), p_set, None, ["apply", "still"])
        trans("M->C2", 0, 6, p_set, slot(1), p_set, ["apply"])
        trans("C2->C5", 6, 10, slot(1), slot(4), None, ["apply"])
        mark = bytearray(slot(4))
        mark[7] ^= 0x55
        trans("C5->playback 8", 10, 8, bytes(mark), bytes(mark), None, [])
        trans("playback 8->C5 (no reload)", 8, 10, bytes(mark), bytes(mark), None, ["apply"])
        trans("C5->Movie", 10, 4, slot(4), p_set, None, ["movie", "apply"])
        trans("Movie->C5", 4, 10, p_set, slot(4), p_set, ["apply"])
        trans("C5->C1", 10, 5, slot(4), slot(0), None, ["apply"])
        trans("C1->P", 5, 3, slot(0), p_set, None, ["apply", "still"])
        trans("P->C1", 3, 5, p_set, slot(0), p_set, ["apply"])
        trans("C3->Movie", 7, 4, slot(2), p_set, None, ["movie", "apply"])
        trans("P->M", 3, 0, p_set, p_set, None, ["still"])
        trans("M->Movie", 0, 4, p_set, p_set, None, ["movie"])
        m.w8(stock.NV_EXPO, 9)
        m.w8(SHADOW, 9)
        m.wr(stock.NV_CUR, p_set)
        call(0xA016F9EC, 9)
        r.check(seq() == [] and m.rd(stock.NV_CUR, SZ) == p_set, "C4->C4 is a no-op")

        # slot save (Custom Mode Setting "Yes") / boot reload
        for cursor in range(5):
            m.wr(CUSTOM_STATE, struct.pack(">HH", cursor, 0))
            cur = pat(0x80 + cursor * 8, 1 + cursor % 4)
            m.wr(stock.NV_CUR, cur)
            others = [slot(i) for i in range(5) if i != cursor]
            call(0xA0059A4A, 4, 0x12)
            r.check(slot(cursor) == cur and [slot(i) for i in range(5) if i != cursor] == others and stray(nv, csr) == [],
                    f"save cursor {cursor} -> slot {cursor} only")
        for mode in range(11):
            m.w8(stock.NV_EXPO, mode)
            m.wr(stock.NV_CUR, p_set)
            call(b.sym("expo_boot_reload"))
            i = SLOT_OF.get(mode)
            r.check(m.rd(stock.NV_CUR, SZ) == (slot(i) if i is not None else p_set) and stray(nv, csr) == [],
                    f"boot reload mode {mode} -> slot {i}")

        # slot compare via the patched 0xA0167618
        for i in range(5):
            m.w8(stock.NV_EXPO, (5, 6, 7, 9, 10)[i])
            m.wr(stock.NV_CUR, slot(i))
            same = call(0xA0167618, i)
            d = bytearray(slot(i))
            d[0x1C] ^= 1
            m.wr(stock.NV_CUR, bytes(d))
            diff = call(0xA0167618, i)
            r.check(same == 1 and diff == 0 and stray() == [], f"compare slot {i}: same={same} diff={diff}")

        # LV widget 0x2E: visibility, icons, still-mode apply
        for mode in range(11):
            m.w8(stock.NV_EXPO, mode)
            m.w16(0xA07FA63E, 0)
            m.run(0xA016A7FC, 0xA016A81A, count=10_000, r4=0x1234)
            vis = m.u16(0xA07FA63E) == 0x1234
            r.eq(vis, mode in SLOT_OF, f"widget visible in mode {mode}")
        wicon = {5: 0xB7, 6: 0xB8, 7: 0xB9, 9: widget_icon + si4, 10: widget_icon + si5}
        for mode in range(11):
            m.w8(stock.NV_EXPO, mode)
            i = SLOT_OF.get(mode)
            for modified in (False, True) if i is not None else (False,):
                cur = bytearray(slot(i) if i is not None else p_set)
                if modified:
                    cur[0x1C] ^= 1
                m.wr(stock.NV_CUR, bytes(cur))
                call(0xA016BAE4)
                ids = [c[1][0] for c in calls if c[0] == "icon"]
                exp = [wicon[mode]] + ([0x2F] if modified else []) if i is not None else []
                r.check(ids[1:] == exp and stray() == [], f"widget draw mode {mode} modified={modified}: icons {[hex(x) for x in ids]}")
        for mode in range(11):
            m.w8(stock.NV_EXPO, mode)
            call(0xA016987A)
            r.eq([c[0] for c in calls if c[0] in ("still", "movie")], ["movie"] if mode == 4 else [] if mode == 8 else ["still"],
                 f"still-mode apply switch, mode {mode}")

        # exposure mode menu 0x05
        t = call(stock.GET_MENU_TABLE, 5)
        r.eq(t, b.sym("m_5"), "Get_Menu_Table(0x05) -> m_5")
        rows = []
        for k in range(16):
            w = struct.unpack(">8I", m.rd(t + 32 * k, 32))
            if not any(w):
                break
            rows.append(w)
        st = [struct.unpack(">8I", img.rd(0xA07EC8AC + 32 * k, 32)) for k in range(8)]
        r.check(rows[:8] == st, "menu 0x05 rows 0..7 = stock")
        c3 = st[7]
        for k, (si, v, sid) in enumerate(((si4, 9, "SID_C4"), (si5, 10, "SID_C5"))):
            exp = (c3[0], 0x01000000 | (cat["ICON_LABEL_L"] + si), (mode_icon + si) << 16 | (cat["ICON_LABEL_S"] + si),
                   (cat[sid] - 1) << 16, v, c3[5], c3[6], c3[7])
            r.eq(rows[8 + k] if len(rows) > 8 + k else None, exp, f"menu 0x05 row {8 + k} (mode {v})")
        r.eq(len(rows), 10, "menu 0x05 row count")

        # MODE screen 23
        ms = b.sym("mode_strip")
        cells = [struct.unpack(">IHH", m.rd(ms + 4 + 8 * k, 8)) for k in range(11)]
        exp_cells = [(4, 0xAF, 0), (0, 0xB0, 0), (1, 0xB1, 0), (2, 0xB2, 0), (3, 0xB3, 0), (5, 0xB4, 0), (6, 0xB5, 0),
                     (7, 0xB6, 0), (9, mode_icon + si4, 0), (10, mode_icon + si5, 0), (0, 0, 0)]
        r.eq(cells, exp_cells, "MODE strip cells")
        m.w8(stock.NV_EXPO, 9)
        call(0xA00521DC)
        xs = [c[1][1] for c in calls if c[0] == "blit"]
        r.eq([c[1][0] for c in calls if c[0] == "icon"], [c[1] for c in exp_cells[:10]], "MODE draw fetches 10 icons in ring order")
        r.check(xs == [24 + 60 * k for k in range(10)] and max(xs) + 51 <= 640, f"MODE icon x {xs}")
        r.eq(rects, [(20 + 60 * 8, 0x1B0, 60, 0x2E)], "MODE highlight on the C4 cell")
        r.eq(stray(), [], "MODE draw: no stray writes")

        def key(k: int, cursor: int, mode: int) -> tuple[int, list[int]]:
            m.w16(ms, cursor)
            m.w8(stock.NV_EXPO, mode)
            m.w8(SHADOW, mode)
            m.wr(stock.NV_CUR, slot(4) if mode == 10 else p_set)
            call(0xA00522AA, k)
            return m.u16(ms), [c[1][0] for c in calls if c[0] == "prog_set"]

        r.eq(key(0x40, 7, 7), (8, [9]), "MODE key 0x40: C3 -> C4")
        r.eq(key(0x40, 8, 9), (9, [10]), "MODE key 0x40: C4 -> C5")
        r.eq(key(0x40, 9, 10), (0, [4]), "MODE key 0x40: C5 -> Movie (wrap)")
        r.eq(key(0x20, 0, 4), (9, [10]), "MODE key 0x20: Movie -> C5 (wrap)")
        r.eq(stray(nv, csr, sh, (ms, ms + 2)), [], "MODE keys: no stray writes")
        m.w8(stock.NV_EXPO, 10)
        m.w16(ms, 0)
        writes.clear()
        m.run(0xA005238E, 0xA00523B2, count=10_000)
        r.check(m.u16(ms) == 9 and stray((ms, ms + 2)) == [], f"MODE open: cursor search finds C5 -> {m.u16(ms)}")

        # Custom Mode Setting screen 31
        icons5 = (0xB4, 0xB5, 0xB6, mode_icon + si4, mode_icon + si5)
        r.eq(struct.unpack(">5H", img.rd(b.sym("custom_icons"), 10)), icons5, "Custom Mode Setting icon table")
        for cursor in range(5):
            m.wr(CUSTOM_STATE, struct.pack(">HH", cursor, 0))
            call(0xA0059790)
            ids = [c[1][0] for c in calls if c[0] == "icon"]
            bl = [c[1][1] for c in calls if c[0] == "blit"]
            ok = (rects == [(20 + 120 * cursor, 0xBB, 120, 0x38)] and ids == [0x96, 0x97, *icons5]
                  and bl == [1 + 120 * cursor, 146 + 120 * cursor] + [54 + 120 * k for k in range(5)]
                  and min(bl) >= 0 and max(bl[:2]) + 13 <= 640 and max(bl[2:]) + 51 <= 640 and stray() == [])
            r.check(ok, f"Custom Mode Setting cursor {cursor}: box {rects} blit x {bl}")
        m.wr(CUSTOM_STATE, struct.pack(">HH", 4, 1))
        call(0xA0059790)
        r.eq([c[1][0] for c in calls if c[0] == "icon"], [mode_icon + si5], "confirm view shows the C5 icon")
