"""m6m suite: payload behaviour in the built C78FRM.bin and the DP2MVxxx.bin package, driven by m6m.syms + catalog.h."""

from __future__ import annotations

import re
import struct
from dataclasses import dataclass
from itertools import product

from tools.re import stock, ui
from tools.re.emu import SCRATCH, STACK, EmuError, Machine, outside
from tools.re.image import BSS, END, FPGA, IPL_LEN

from .harness import Build, Nor, Report

FILM_MODE0 = 9
NV_RESERVED = (stock.NV_BLOCK + 0x04, stock.NV_BLOCK + 0x14)  # zero-default bytes free for payload settings
LV_DIRTY = (0xA07B1398, 0xA07B139C)  # LV refresh flags, set by the paper setters
LV_CMCM, LV_MONO_Q7, LV_KNOTS = 1, 2, 4
PAPER_NV = {1: "color_paper", 2: "bw_paper", 3: "lut_slot"}
PAGE_RESET, PAGE_LIST = 0xA0058A0A, 0xA07F230E  # menu entry copies the page template (state + 0x14) to the live list
SID0 = 672
QS_SLOT_MENUS = range(0xE5, 0xED)
# slot getters called by QS_SLOT_ROWS(page, slot)
QS_SLOT_GET = {0xA01684FE: (0, 0), 0xA0168504: (0, 1), 0xA016850A: (0, 2), 0xA0168510: (0, 3),
               0xA0168516: (1, 0), 0xA016851C: (1, 1), 0xA0168522: (1, 2), 0xA0168528: (1, 3)}
# maker note writer: tag buffer setup + color mode switch (r7 = ctx + 0x80, cm at +0xE) up to the common tail
MN_SWITCH_SETUP, MN_SWITCH_DONE = 0xA0148CE4, 0xA0148D2E
MN_TAG_3D, MN_TAG_59 = 0xA07F7D78, 0xA07F8320
# playback parsers: the store of an unmatched name (r0 = 0) -> next insn; string reg, record reg (cm at +0x17)
PB_STORES = ((0xA0147938, 0xA014793C, "r7", "r4"), (0xA0132E9A, 0xA0132E9E, "r6", "r5"))


@dataclass(frozen=True)
class Film:
    cm: int
    packed: int
    paper_cls: int
    auto_paper: int
    mono: int


def s32(v: int) -> int:
    return v - (1 << 32) if v & 0x80000000 else v


def films(b: Build) -> list[Film]:
    return [Film(FILM_MODE0 + i, *f) for i, f in enumerate(b.gen.films)]


def menu_map(b: Build) -> dict[int, int]:
    """Generated menus registered by id (`m_<hex id>`): id -> rows."""
    return {int(n[2:], 16): b.sym(n) for n in b.gen.menus if re.fullmatch(r"m_[0-9a-f]+", n)}


def packed_of(f: Film | None, cm: int, nv: dict[str, int]) -> int:
    if f is None:
        return cm
    if not f.paper_cls:
        return f.packed
    p = nv.get(PAPER_NV[f.paper_cls], 0)
    return f.packed + (p - 1 if 1 <= p <= 8 else f.auto_paper)


def expand_knots(k: bytes) -> bytes:
    """18 knots at x = 0..128 step 16, 192..384 step 64, 512..1023 step 128 -> 1024 B curve."""
    out, i = [], 0
    for sh, cnt in ((4, 8), (6, 4), (7, 5)):
        for _ in range(cnt):
            y0, dy = k[i], k[i + 1] - k[i]
            out += [(y0 + ((dy * s + (1 << sh >> 1)) >> sh)) & 0xFF for s in range(1 << sh)]
            i += 1
    return bytes(out)


def lv_matrix(cmcm: tuple[int, ...], flags: int, cc: list[int]) -> tuple[int, ...]:
    """B2Y matrix the CMCM hook programs: film q10 x live q7 CCMatrix (or direct q7 mono weights), BT.601 Y row."""
    m = [0] * 9 + [0x4D, 0x96, 0x1D, 0x200, 0]
    for i, j in product(range(3), range(3)):
        if flags & LV_MONO_Q7:
            v = cmcm[i * 3 + j]
        else:
            v = (cmcm[i * 3] * cc[j] + cmcm[i * 3 + 1] * cc[3 + j] + cmcm[i * 3 + 2] * cc[6 + j] + 512) >> 10
            v = max(-2047, min(2047, v))
        m[i * 3 + j] = v & 0xFFF
    return tuple(m)


def elf_section(raw: bytes, want: bytes) -> bytes:
    shoff = struct.unpack_from(">I", raw, 0x20)[0]
    shentsize, shnum, shstrndx = struct.unpack_from(">3H", raw, 0x2E)
    sh = [struct.unpack_from(">10I", raw, shoff + i * shentsize) for i in range(shnum)]
    strtab = raw[sh[shstrndx][4] : sh[shstrndx][4] + sh[shstrndx][5]]
    for name_off, _, _, _, off, size, *_ in sh:
        if strtab[name_off : strtab.find(b"\0", name_off)] == want:
            return raw[off : off + size]
    raise KeyError(want)


def check_invariants(b: Build, r: Report) -> None:
    r.check(b.img.c78[:IPL_LEN] == b.stock.c78[:IPL_LEN], "IPL is stock")
    r.check(b.img.rd(FPGA[0], FPGA[1] - FPGA[0]) == b.stock.rd(FPGA[0], FPGA[1] - FPGA[0]), "FPGA bitstream is stock")
    r.check(b.img.rd(BSS, END - BSS) == b.stock.rd(BSS, END - BSS), ".bss tail is stock")
    r.eq(b.img.u32(0xA004D3D0), 0xA07EEFCC, "shared literal 0xA004D3D0 intact")
    r.check(b.img.rd(0xA005D924, 0xA005F1A0 - 0xA005D924) == b.stock.rd(0xA005D924, 0xA005F1A0 - 0xA005D924), "live screens 50..55 intact")


def check_hooks(b: Build, r: Report) -> None:
    raw = elf_section((b.dir / "m6m/m6m.elf").read_bytes(), b".dp2m.hooks")
    hooks = list(struct.iter_unpack(">4I", raw))
    r.check(len(hooks) > 0 and len(raw) == 16 * len(hooks), f".dp2m.hooks has {len(hooks)} records")
    bad_stock, bad_img = [], []
    for kind, site, target, expect in hooks:
        st_ok = (b.stock.bl_target(site) == (expect & ~1) if kind == 1 else
                 b.stock.u32(site) == expect if kind in (2, 3) else
                 kind == 4 and b.stock.u16(site) == expect)
        im_ok = (b.img.bl_target(site) == (target & ~1) and (b.img.u16(site + 2) & 0xF800) == 0xF800 if kind in (1, 2) else
                 b.img.u32(site) == target if kind == 3 else
                 kind == 4 and b.img.u16(site) == (target & 0xFFFF))
        if not st_ok:
            bad_stock.append((kind, hex(site)))
        if not im_ok:
            bad_img.append((kind, hex(site)))
    r.eq(bad_stock, [], f".dp2m.hooks expect values match stock across all {len(hooks)} sites")
    r.eq(bad_img, [], f".dp2m.hooks patches applied in C78FRM.bin across all {len(hooks)} sites")
    ret0 = b.sym("dp2m_ret0") | 1
    doom_scr = (b.sym("doom_screen") | 1) if "doom_screen" in b.syms else ret0
    dead = range(0xA07DEBAC, 0xA07DEBF0, 4)
    r.check(all(0xA0059D50 <= (b.stock.u32(s) & ~1) < 0xA005D924 and b.img.u32(s) == (doom_scr if s == 0xA07DEBAC else ret0) for s in dead),
            "retired factory screen vtable slots 33..49 -> doom_screen / dp2m_ret0")
    r.check(b.img.rd(0xA07DEB28, 0x84) == b.stock.rd(0xA07DEB28, 0x84)
            and b.img.rd(0xA07DEBF0, 0x1C) == b.stock.rd(0xA07DEBF0, 0x1C), "live screen vtable slots 0..32 & 50..55 intact")
    m = Machine(b.img)
    r.eq(m.call(b.sym("dp2m_ret0")), 0, "dp2m_ret0 returns 0")


def check_menus(b: Build, r: Report) -> None:
    m, ms = Machine(b.img), Machine(b.stock)
    m.load_nv_defaults()
    ms.load_nv_defaults()
    gen = menu_map(b)
    film_page, paper, paper0, qs = b.cat["MENU_FILM_PAGE"], b.cat["MENU_PAPER"], b.cat["MENU_PAPER_CLS0"], b.cat["MENU_QS_COPY"]
    r.check(film_page in gen, f"Film page {film_page:#x} in the generated menu map")
    names = {s.va: n for n, s in b.syms.items()}
    for mid, rows in sorted(gen.items()):
        r.eq(ui.menu_table(m, mid), rows, f"Get_Menu_Table({mid:#x}) -> {names.get(rows, hex(rows))}")
    for name, rows in b.gen.menus.items():
        got = [row.w for row in ui.rows(b.img, b.sym(name))]
        exp = [tuple(b.value(f) for f in row) for row in rows]
        bad = next((k for k in range(max(len(got), len(exp))) if got[k : k + 1] != exp[k : k + 1]), None)
        r.check(bad is None, f"{name}: {len(exp)} rows = catalog.c" + (f" (row {bad} differs)" if bad is not None else ""))
    subs = {row.id: ui.menu_table(m, row.id) for row in ui.rows(m, gen[film_page])}
    r.check(all(subs.values()), f"Film page submenus resolve {[hex(k) for k in subs]}")
    exp_pages = [0x0E, 0x0F, film_page, *range(0x10, 0x18)]
    r.eq(list(struct.unpack(">11H", b.img.rd(0xA07F22E6, 22))), exp_pages, "menu page template (state + 0x14) = 11 pages")
    r.eq(list(struct.unpack(">11H", b.img.rd(PAGE_LIST, 22))), exp_pages, "menu live page list (state + 0x3C) = 11 pages")
    m.wr(PAGE_LIST, bytes(22))
    m.call(PAGE_RESET)
    r.eq(list(struct.unpack(">11H", m.rd(PAGE_LIST, 22))), exp_pages, "menu page list after the entry reset 0xA0058A0A")
    pg = b.sym("page_group")
    pg_sites = (0xA005811C, 0xA0058144, 0xA0058204, 0xA00582C6, 0xA0058308, 0xA00584E8)
    r.check(all(b.img.bl_target(s) == pg for s in pg_sites), "all 6 menu page_group call sites hooked")
    r.eq([m.call(pg, p) for p in range(11)], [0] * 5 + [1] * 2 + [2] * 4, "page_group(0..10) -> shoot 0..4, playback 5..6, setup 7..10")
    tab_geom = (b.img.u16(0xA005844A), b.img.u16(0xA0058454), *(b.img.u16(s) for s in (0xA005868A, 0xA00586A0, 0xA005871C, 0xA0058734)),
                b.img.u16(0xA04F3100), b.img.u16(0xA04F3104), b.img.u16(0xA04F3110), b.img.u16(0xA04F3114),
                b.img.u16(0xA04F3118), b.img.u16(0xA04F311C), b.img.u16(0xA04F3120), b.img.u16(0xA04F3124))
    r.eq(tab_geom, (0x3034, 0x2D0B, 0x210A, 0x210A, 0x210A, 0x210A, 34, 48, 32, 260, 292, 104, 396, 208),
         "11-tab strip stride 52 px, wrap 10, and 5/2/4 group spans")
    pin_got = []
    for expo, pages in ((8, (0, 4, 5, 6, 7, 10)), (1, (0, 2, 5, 6, 10))):
        m.w8(stock.NV_EXPO, expo)
        for p in pages:
            m.w16(SCRATCH + 4, p)
            m.run(0xA00587F8, 0xA005882E, r4=SCRATCH)
            pin_got.append(m.u16(SCRATCH + 4))
    r.eq(pin_got, [5, 5, 5, 6, 5, 5, 0, 2, 0, 0, 10], "playback pins menu to pages 5..6; shooting resets 5..6 -> 0")
    m.load_nv_defaults()

    fl = films(b)
    for cls in range(4):
        cm = next((f.cm for f in fl if f.paper_cls == cls), 0)
        m.w8(stock.NV_CM, cm)
        r.eq(ui.menu_table(m, paper), gen[paper0 + cls], f"Get_Menu_Table({paper:#x}) for cm {cm} -> paper class {cls}")
        r.eq(ui.menu_table(m, paper | qs), gen[(paper0 + cls) | qs], f"Get_Menu_Table({paper | qs:#x}) for cm {cm} -> QS paper class {cls}")
    m.load_nv_defaults()

    strs = b.gen.strings
    for mid in sorted(i for i in gen if i & qs):
        bad = [row.sid for row in ui.rows(m, gen[mid])
               if row.qs_icon != b.cat["ICON_LABEL_L"] + (k := row.qs_icon & 0x3FF) or row.strip_icon != b.cat["ICON_LABEL_S"] + k
               or strs[k] != (ui.string(m, row.sid) if row.sid < SID0 else strs[row.sid - SID0])]
        r.eq(bad, [], f"QS copy {mid:#x}: value label icons match their text")
    for mid in QS_SLOT_MENUS:
        items = [row for row in ui.rows(m, gen[mid]) if row.value & qs]
        page, slot = QS_SLOT_GET[items[0].get & ~1]
        got = []
        for row in items:
            m.call(row.set & ~1, row.value)
            got.append(m.call(stock.QS_SLOT_ROWS, page, slot) == ui.menu_table(m, row.value) == gen.get(row.value, gen.get(paper0 | qs)))
        r.check(len(items) == 5 and all(got), f"QS slot menu {mid:#x} (page {page} slot {slot}): {len(items)} film items open their QS copies")
    m.load_nv_defaults()

    iso = {name: b.sym(name) for name in ("m_iso_1ev", "m_iso_3ev") if name in b.syms}
    for step, name in enumerate(("m_iso_1ev", "m_iso_3ev")):
        if name in iso:
            m.w8(stock.NV_ISO_STEP, step)
            r.eq(ui.menu_table(m, stock.MENU_ISO), iso[name], f"Get_Menu_Table({stock.MENU_ISO:#x}) with ISO step {step} -> {name}")
    m.load_nv_defaults()
    st, bt = ui.menu_tables(ms), ui.menu_tables(m)
    changed = [i for i in sorted(set(st) | set(bt)) if i not in gen and i != paper and bt.get(i) not in iso.values() and st.get(i) != bt.get(i)]
    r.eq(changed, [], f"other menu ids resolve as stock ({len(st)} stock ids)")


def check_strings(b: Build, r: Report) -> None:
    m, ms = Machine(b.img), Machine(b.stock)
    m.load_nv_defaults()
    ms.load_nv_defaults()
    strs = b.gen.strings
    r.eq(b.img.u32(b.sym("dp2m_string_count")), len(strs), "dp2m_string_count = catalog.c")
    sid0, title = stock.STOCK_SIDS, b.cat["SID_PAPER_TITLE"]
    bad = []
    for i, s in enumerate(strs):
        exp = strs[i + 1] if sid0 + i == title else s  # NV defaults: stock color mode -> paper class 0
        bad += [(sid0 + i, lang) for lang in (0, 1, 16) if ui.string(m, sid0 + i, lang) != exp]
    r.eq(bad, [], f"UI string lookup of all {len(strs)} dp2m strings (languages 0, 1, 16)")
    fl = films(b)
    for cls in range(4):
        cm = next((f.cm for f in fl if f.paper_cls == cls), 0)
        m.w8(stock.NV_CM, cm)
        r.eq(ui.string(m, title), strs[title - sid0 + 1 + cls], f"paper title for cm {cm} (class {cls})")
    m.load_nv_defaults()
    diff = []
    for sid, lang in product(range(sid0), (0, 1)):
        try:
            exp = ui.string(ms, sid, lang)
        except EmuError:
            continue
        if ui.string(m, sid, lang) != exp:
            diff.append((sid, lang))
    r.eq(diff, [], f"stock sids 0..{sid0 - 1} unchanged (languages 0, 1)")


def check_settings(b: Build, r: Report) -> dict[str, int]:
    """Setter/getter pairs: own NVRAM byte, value round trip, clamping, persistence through the stock flush + load."""
    m = Machine(b.img)
    m.load_nv_defaults()
    values: dict[int, set[int]] = {}
    for rows in menu_map(b).values():
        for row in ui.rows(b.img, rows):
            values.setdefault(row.get & ~1, set()).add(row.value)
    names = sorted(n[7:] for n in b.syms if n.startswith("nv_get_") and n != "nv_get_none")
    addrs, vals = {}, {}
    for name in names:
        get, put = b.sym(f"nv_get_{name}"), b.sym(f"nv_set_{name}")
        vs = sorted(values.get(get, ()))
        if not r.check(len(vs) > 1, f"nv {name}: menu values {vs}"):
            continue
        before = m.rd(stock.NV_BLOCK, stock.NV_LEN)
        with m.writes(STACK, LV_DIRTY) as log:
            m.call(put, vs[-1])
        after = m.rd(stock.NV_BLOCK, stock.NV_LEN)
        changed = [stock.NV_BLOCK + i for i in range(stock.NV_LEN) if before[i] != after[i]]
        addr = changed[0] if len(changed) == 1 else None
        ok = addr is not None and NV_RESERVED[0] <= addr < NV_RESERVED[1] and outside(log, (addr, addr + 1)) == []
        r.check(ok, f"nv_set_{name} writes only NVRAM {addr and hex(addr)} (reserved +0x04..+0x13)")
        if addr is None:
            continue
        rt = []
        for v in vs:
            m.call(put, v)
            if m.call(get) != v:
                rt.append(v)
        m.w8(addr, vs[-1] + 1)
        r.check(not rt and m.call(get) == 0, f"nv {name}: {vs[0]}..{vs[-1]} round-trip, {vs[-1] + 1} reads as 0")
        addrs[name], vals[name] = addr, vs[-1]
    r.eq(len(set(addrs.values())), len(addrs), "settings use distinct NVRAM bytes")

    nor = Nor(m)
    for name, v in vals.items():
        m.call(b.sym(f"nv_set_{name}"), v)
    m.call(stock.NV_FLUSH)
    off, n, buf = nor.writes[-1] if nor.writes else (0, 0, b"")
    rec = {name: buf[0x18 + a - stock.NV_BLOCK] for name, a in addrs.items()} if off == 8 else {}
    r.eq(rec, vals, "stock flush writes the settings into the NOR record (block at NOR 0x20)")
    nor.data[0:4] = struct.pack(">I", 0x20120918)  # record magic, kept outside the flushed range
    nor.data[off : off + n] = buf
    m.wr(stock.NV_BLOCK, bytes(stock.NV_PERSIST))
    m.call(stock.NV_LOAD, stock.NV_BLOCK)
    r.eq({name: m.call(b.sym(f"nv_get_{name}")) for name in vals}, vals, "settings survive NOR flush + boot load")
    return addrs


def check_bracketing(b: Build, r: Report) -> None:
    m = Machine(b.img)
    for idx in (1, 3):
        m.w16(stock.BKT_STEP_IDX, idx)
        step = s32(b.img.u32(stock.BKT_STEP_TBL + 4 * idx))
        for order, ev in ((0, [0, 1, -1]), (4, [0, 2, 1, -1, -2]), (6, [2, 1, 0, -1, -2]), (7, [-2, -1, 0, 1, 2])):
            m.w8(stock.NV_BKT_ORD, order)
            got = (m.call(stock.BKT_COUNT), [s32(m.call(stock.BKT_EV_Q16, i)) for i in range(len(ev))])
            r.eq(got, (len(ev), [e * step for e in ev]), f"bracketing order {order} step {step / 65536:.3f} EV")


def check_min_shutter(b: Build, r: Report, nv: int) -> None:
    m = Machine(b.img)
    tv = b.gen.tv_min
    a_bad, p_bad = [], []
    for idx in (*range(7), 7, 0xFF):
        m.w8(nv, idx)
        m.run(stock.AUTOISO_A, stock.AUTOISO_A + 14, r0=0, r1=0x12345678)
        if (m.reg("r0"), m.reg("r1")) != (tv[idx if idx <= 6 else 0], 0x12345678):
            a_bad.append(idx)
        for site in stock.AUTOISO_P:
            m.run(site, site + 4, r0=0)
            if m.reg("r0") != (tv[idx] + 0x30000 if 1 <= idx <= 6 else 0x90000):
                p_bad.append((site, idx))
    r.eq(a_bad, [], "ISO Auto min shutter, A mode: table value, idx > 6 -> Auto, r1 kept")
    r.eq(p_bad, [], "ISO Auto min shutter, P mode (both sites): table value + 3 EV, Auto -> stock 9.0")


def check_ipc(b: Build, r: Report, addrs: dict[str, int]) -> None:
    m = Machine(b.img)
    m.load_nv_defaults()
    cmd, meta = SCRATCH + 0x100, SCRATCH + 0x200
    fl = {f.cm: f for f in films(b)}
    regs_bad: list[tuple] = []

    def pack(cm: int, cs: int, **nv: int) -> tuple[int, int]:
        for k, v in nv.items():
            m.w8(addrs[k], v)
        m.wr(meta + 0x1D, bytes([cs, cm]))
        m.w32(STACK[1] + 0x14, meta)
        m.wr(cmd, bytes(8))
        m.run(stock.IPC_CMD20, stock.IPC_CMD20 + 6, r0=0x55, r1=0x3F800000, r4=cmd)
        if (m.reg("r0"), m.reg("r1")) != (0, 0x3F800000):
            regs_bad.append((cm, cs, nv))
        return m.u8(cmd + 5), m.u8(cmd + 6)

    zero = {k: 0 for k in addrs if k in ("engine", "grain", "halation", "fill", *PAPER_NV.values())}
    bad = [cm for cm in range(FILM_MODE0) if pack(cm, 0, **zero)[0] != cm]
    r.eq(bad, [], "cmd[5] = stock color modes 0..8 unchanged")
    for cls in range(4):
        fs = [f for f in fl.values() if f.paper_cls == cls]
        papers = range(10) if cls else (0, 3)
        bad = []
        for f, p in product(fs, papers):
            nv = {**zero, **({PAPER_NV[cls]: p} if cls else {})}
            if pack(f.cm, 0, **nv)[0] != packed_of(f, f.cm, nv):
                bad.append((f.cm, p))
        r.eq(bad, [], f"cmd[5] packed mode, paper class {cls}: {len(fs)} films x papers {papers[0]}..{papers[-1]}")
    f = next(iter(fl.values()))
    bad = []
    for e, g, h, fill, cs in product((0, 1), (0, 1), (0, 1), range(4), range(4)):
        got = pack(f.cm, cs, **{**zero, "engine": e, "grain": g, "halation": h, "fill": fill})[1]
        if got != cs | (e | g << 1 | h << 2) << 2 | fill << 5:
            bad.append((e, g, h, fill, cs, got))
    r.eq(bad, [], "cmd[6] = colorspace | (3d | grain<<1 | halation<<2) << 2 | fill << 5 (128 combos)")
    r.eq(regs_bad, [], "IPC hook keeps live r1 (1.0f) and returns r0 = 0")


def check_lv(b: Build, r: Report) -> None:
    for site, name in ((stock.LV_GAMMA_SITE, "lv_gamma_hook"), (stock.LV_MONO_SITE, "lv_mono_hook"), (stock.LV_CMCM_SITE, "lv_cmcm_hook")):
        r.eq(b.img.bl_target(site), b.sym(name), f"BL @ {site:#x} -> {name}")
    m = Machine(b.img)
    m.load_nv_defaults()
    calls: list[tuple] = []
    m.stub(b.sym("stock_b2y_set_gamma"), on=lambda a: calls.append(("gamma", a[1], m.rd(a[1], a[2]))))
    m.stub(b.sym("stock_lv_mono"), on=lambda a: calls.append(("mono", a[0], a[1])))
    m.stub(b.sym("stock_lv_cmcm"), on=lambda a: calls.append(("cmcm", a[0], a[1])))
    m.stub(b.sym("stock_b2y_set_mat"), on=lambda a: calls.append(("mat", struct.unpack(">14H", m.rd(a[0], 28)))))
    cc = {7: [96, -20, 5, -30, 120, 8, 2, -15, 110], 1: [130, -40, 10, -25, 140, -12, 0, -30, 150]}
    for mode, getter in ((7, "stock_get_lv_cc7"), (1, "stock_get_lv_ccn")):
        raw = bytes(10) + b"".join(struct.pack(">H", (v & 0xFFF) | 0xA000) for v in cc[mode]) + bytes(8)
        m.stub(b.sym(getter), on=lambda a, raw=raw: m.wr(a[0], raw))
    lk, fl = b.gen.looks, films(b)

    def look(f: Film) -> tuple[tuple[int, ...], bytes, int]:
        return lk[packed_of(f, f.cm, {}) - FILM_MODE0]

    def run(fn: str, *args: int) -> list[tuple]:
        calls.clear()
        m.call(b.sym(fn), *args)
        return list(calls)

    lut = SCRATCH + 0x1000
    stock_lut = bytes(range(256)) * 4
    m.wr(lut, stock_lut)
    gbuf = b.sym("gamma_buf")
    kf = next(f for f in fl if look(f)[2] & LV_KNOTS)
    curve = expand_knots(look(kf)[1])
    m.w8(stock.NV_CM, 0)
    r.eq(run("lv_gamma_hook", 0, lut, 0x400), [("gamma", lut, stock_lut)], "gamma hook, stock color mode: stock LUT")
    m.w8(stock.NV_CM, kf.cm)
    r.eq(run("lv_gamma_hook", 0, lut, 0x400), [("gamma", gbuf, curve)], f"gamma hook, film cm {kf.cm}: expanded film knots")
    m.w8(stock.NV_CM, 0)
    run("lv_gamma_hook", 0, lut, 0x400)
    mono_cm = 2 if kf.mono else 3
    r.eq(run("lv_mono_hook", 1, kf.cm), [("mono", 1, mono_cm), ("gamma", gbuf, curve)], f"color mode 0 -> {kf.cm}: mono routing + gamma re-applied")
    r.eq(run("lv_mono_hook", 1, kf.cm), [("mono", 1, mono_cm)], "same color mode: gamma not re-applied")
    r.eq(run("lv_mono_hook", 1, 0), [("mono", 1, 0), ("gamma", lut, stock_lut)], f"color mode {kf.cm} -> 0: stock gamma restored")

    r.eq(run("lv_cmcm_hook", 1, 0), [("cmcm", 1, 0)], "CMCM hook, stock color mode: stock path")
    cf = next(f for f in fl if look(f)[2] & LV_CMCM and not look(f)[2] & LV_MONO_Q7)
    for mode in (7, 1):
        exp = lv_matrix(look(cf)[0], look(cf)[2], cc[mode])
        r.eq(run("lv_cmcm_hook", mode, cf.cm), [("mat", exp)], f"CMCM hook, film cm {cf.cm}, LV mode {mode}: q10 film x live q7")
    mf = next((f for f in fl if look(f)[2] & LV_MONO_Q7), None)
    if mf:
        exp = lv_matrix(look(mf)[0], look(mf)[2], cc[1])
        r.eq(run("lv_cmcm_hook", 1, mf.cm), [("mat", exp)], f"CMCM hook, mono film cm {mf.cm}: direct q7 weights")


def check_ycap(b: Build, r: Report) -> None:
    m = Machine(b.img)
    lp = b.syms["lv_planes"]
    r.check(not stock.NV_BLOCK <= lp.va < stock.NV_BLOCK + stock.NV_LEN, f"lv_planes @ {lp.va:#x} outside the NVRAM block")
    site = stock.LV_YCAP + 2
    lit = b.stock.u32(((site + 4) & ~3) + (b.stock.u16(site) & 0xFF) * 4)
    planes = (0xA2300000, 0xA2360000, 0xA2390000)
    m.wr(SCRATCH, struct.pack(">3I", *planes))
    f0 = m.u32(lp.va + 12)
    with m.writes(STACK) as log:
        for _ in range(3):
            m.run(stock.LV_YCAP, stock.LV_YCAP + 6, r0=SCRATCH)
    got = (m.reg("r5"), m.reg("r7"), m.rd(lp.va, 12), m.u32(lp.va + 12))
    r.eq(got, (lit, SCRATCH, struct.pack(">3I", *planes), f0 + 3), "LV Y-capture hook: planes + frame count, stock r5/r7")
    r.eq(outside(log, (lp.va, lp.va + lp.size)), [], "LV Y-capture hook writes only lv_planes (never NVRAM)")


def check_updater(b: Build, r: Report) -> tuple[int, int]:
    maj, mi = (int(x) for x in b.pkg.version.split("."))
    r.eq(b.img.rd(stock.VERSION_IMM, 4), bytes([0x22, maj, 0x23, mi]), f"'Current version' immediates = {maj}.{mi:02}")
    for va in stock.UPDATER_BANNERS:
        old, new = b.stock.cstr(va), b.img.cstr(va)
        slot = (len(old) + 4) & ~3
        ok = "Debug Write" in old and bool(new) and new != old and b.img.rd(va, slot) == new.encode().ljust(slot, b"\0")
        r.check(ok and b.img.rd(va + slot, 4) == b.stock.rd(va + slot, 4), f"updater banner {va:#x}: {old!r} -> {new!r}")
    return maj, mi


def check_package(b: Build, r: Report, maj: int, mi: int) -> None:
    p, s = b.pkg, b.stock.pkg
    r.eq(b.pkg_name, f"DP2MV{maj}{mi:02}.bin", "package file name matches the version")
    r.check(len(set(p.versions)) == 1 and bool(re.fullmatch(r"\d+\.\d\d", p.version)), f"header versions {p.versions}")
    r.check(bool(re.fullmatch(r"20\d{6}", p.date)), f"header date {p.date}")
    r.eq((len(p.raw), p.flags, p.c78_off), (len(s.raw), s.flags, s.c78_off), "size, stage flags, M6M segment offset = stock")
    fields = {*range(0x10, 0x20), *range(0x30, 0x58)}
    r.eq([i for i in range(0x80) if i not in fields and p.raw[i] != s.raw[i]], [], "header bytes outside version/date = stock")
    r.check(p.segment("bridge") == s.segment("bridge"), "bridge M6M segment is stock")
    r.check(p.segment("m5") == (b.dir / "Firm.bin").read_bytes(), "M5 segment = Firm.bin")
    r.check(p.segment("m6m") == b.img.c78, "M6M segment = C78FRM.bin")


def check_makernote(b: Build, r: Report) -> None:
    """Color mode switch of the maker note writer, then both playback parser stores, for stock modes and films."""
    m = Machine(b.img)
    sp, rec, txt = STACK[1] - 0x200, SCRATCH, SCRATCH + 0x100
    stock_names = {1: "Sepia", 2: "Monochrome", 3: "Standard", 4: "Vivid", 0: "Standard"}
    fl = films(b)
    bad = []
    for cm in [0, 1, 2, 3, 4] + [f.cm for f in fl]:
        m.w8(rec + 0x8E, cm)
        m.run(MN_SWITCH_SETUP, MN_SWITCH_DONE, sp=sp, r7=rec + 0x80)
        tag3d, tag59 = m.rd(MN_TAG_3D, 11), m.rd(MN_TAG_59, 15)
        if cm < FILM_MODE0:
            ok = tag3d.startswith(stock_names[cm].encode()) and tag59.startswith(stock_names[cm].encode())
        else:
            name = b.gen.strings[cm - FILM_MODE0].encode()[:14]
            ok = tag3d == f"Film {cm - FILM_MODE0:02d}".encode().ljust(11, b"\0") and tag59 == name.ljust(15, b"\0")
        if not ok:
            bad.append((cm, tag3d, tag59))
            continue
        for site, until, rs, rr in PB_STORES:
            m.wr(txt, tag3d + b"\0")
            m.w8(rec + 0x17, 0xEE)
            m.run(site, until, sp=sp, r0=0, **{rs: txt, rr: rec})
            if m.u8(rec + 0x17) != (cm if cm >= FILM_MODE0 else 0):
                bad.append((cm, hex(site), m.u8(rec + 0x17)))
    r.eq(bad, [], f"maker note: stock modes keep their names, {len(fl)} films write 'Film NN' + name and parse back in JPG & X3F playback")


def check_hud(b: Build, r: Report, peakcol_addr: int, peaksens_addr: int, zebralvl_addr: int) -> None:
    wget, gdraw = b.sym("grid_widget_get"), b.sym("grid_draw_hook")
    r.eq(b.img.u32(0xA07FA4A4), wget | 1, "LV widget 0x14 getter -> grid_widget_get")
    r.check(b.img.bl_target(0xA016C708) == gdraw and b.img.bl_target(0xA004C5AE) == gdraw, "both stock_grid_draw call sites -> grid_draw_hook")

    m = Machine(b.img)
    m.load_nv_defaults()
    st = {"scr": 0x15, "lv": 1, "ax": 0, "ay": 64, "stock": []}
    m.stub(b.sym("stock_ui_screen_id"), on=lambda _: st["scr"])
    m.stub(b.sym("stock_lv_active"), on=lambda _: st["lv"])
    m.stub(b.sym("stock_accel_x"), on=lambda _: st["ax"] & 0xFF)
    m.stub(b.sym("stock_accel_y"), on=lambda _: st["ay"] & 0xFF)
    m.stub(b.sym("stock_grid_draw"), on=lambda a: st["stock"].append(a[0]))

    # Guard & stock delegation checks
    m.w8(stock.NV_GRID, 3)
    r.eq(m.call(wget), 3, "grid_widget_get passes stock grid 3 through")
    m.call(gdraw, 0x1234)
    r.eq(st["stock"], [0x1234], "grid_draw_hook delegates stock grids 0..6 to stock_grid_draw")

    m.w8(stock.NV_GRID, 7)
    for scr, lv in ((0x1B, 1), (0x15, 0)):
        st["scr"], st["lv"] = scr, lv
        with m.writes(STACK) as log:
            k = m.call(wget)
            m.call(gdraw, SCRATCH)
        r.check(k == 7 and outside(log) == [], f"HUD inactive when screen={scr:#x} lv_active={lv}: key=m, zero ARGB writes")
    st["scr"], st["lv"] = 0x15, 1

    # Dirty-key semantics: tilt + deadband + portrait on level/mask, fcnt >> 1 on frame overlays
    lp = b.sym("lv_planes")
    k0 = m.call(wget)
    st["ax"] = 1
    k_dead = m.call(wget)
    st["ax"] = 20
    k_tilt = m.call(wget)
    st["ax"], st["ay"] = 64, 0
    k_port = m.call(wget)
    r.check(k0 == k_dead and k_tilt != k0 and ((k_port >> 8) & 1) == 1, "grid_widget_get tilt: deadband |slope|<=1, landscape vs portrait")
    st["ax"], st["ay"] = 0, 64
    fkeys = []
    for mode in (11, 12, 13, 14, 17):
        m.w8(stock.NV_GRID, mode)
        for fc in (4, 5, 6):
            m.w32(lp + 12, fc)
            fkeys.append(m.call(wget))
    r.eq(fkeys, [v for mode in (11, 12, 13, 14, 17) for v in (mode << 16 | 2, mode << 16 | 2, mode << 16 | 3)],
         "grid_widget_get modes 11..14 & 17 key on fcnt >> 1")

    # Null/unaligned plane guard for per-frame modes
    m.wr(lp, struct.pack(">4I", 0, 0, 0, 0))
    m.w8(stock.NV_GRID, 11)
    with m.writes(STACK) as log:
        m.call(gdraw, SCRATCH)
    r.check(len(outside(log, (b.sym("grid_draw_hook"), END))) <= 1 and all(a != stock.ARGB_FB for a, _ in log),
            "grid_draw_hook skips peaking/zebras when lv_planes.y is unset")

    # Seed synthetic 640x424 LV planes: vertical step edge at x=160, bright highlight at (40..80, 40..80)
    yv, cbv, crv = 0xA2400000, 0xA2450000, 0xA2480000
    y_plane = bytearray([80] * (640 * 424))
    for y in range(424):
        row = y * 640
        for x in range(160, 200):
            y_plane[row + x] = 200
        if 40 <= y < 80:
            for x in range(40, 80):
                y_plane[row + x] = 252
    m.wr(yv, bytes(y_plane))
    m.wr(cbv, bytes([110]) * (320 * 424))
    m.wr(crv, bytes([160]) * (320 * 424))
    m.wr(lp, struct.pack(">4I", yv, cbv, crv, 4))

    def px(x: int, y: int) -> int:
        return m.u16(stock.ARGB_FB + (y * 640 + x) * 2)

    def draw(mode: int) -> None:
        m.w8(stock.NV_GRID, mode)
        m.call(gdraw, SCRATCH)

    # Mode 7: Electronic Level (level green -> tilted white -> portrait)
    st["ax"], st["ay"] = 0, 64
    draw(7)
    r.check(px(212, 211) == 0x1F3E and px(319, 211) == 0x1F3E and px(10, 10) == 0, "grid 7 level: green horizon & center tick")
    st["ax"], st["ay"] = 20, 64
    draw(7)
    r.eq(px(212, 211), 0xFFFE, "grid 7 tilted: white horizon")
    st["ax"], st["ay"] = 64, 0
    draw(7)
    r.eq(px(319, 112), 0x1F3E, "grid 7 portrait: vertical end-cap")
    st["ax"], st["ay"] = 0, 64

    # Modes 8, 9, 10, 15, 16: Aspect Masks + clean transition between bar sizes
    for mode, vert, alpha, s in ((8, 0, 10, 74), (9, 0, 10, 31), (10, 1, 10, 107), (15, 1, 9, 70), (16, 1, 9, 37)):
        draw(mode)
        ok = (px(s - 1, 10) == alpha and px(s, 10) == 0xFFFC and px(640 - s - 1, 10) == 0xFFFC and px(s + 4, 10) == 0
              if vert else px(10, s - 1) == alpha and px(10, s) == 0xFFFC and px(10, 424 - s - 1) == 0xFFFC and px(10, s + 4) == 0)
        r.check(ok, f"grid {mode} aspect mask ({'vert' if vert else 'horiz'} {s}px): matte, 1px white guide, clean interior")

    # Modes 11 & 12: Focus Peaking (all 4 colors + 2px top/bottom clear) and Peaking + Level
    m.w16(stock.ARGB_FB, 0xDEAD)
    m.w16(stock.ARGB_FB + (423 * 640 + 10) * 2, 0xDEAD)
    got_cols = []
    for c in range(4):
        m.w8(peakcol_addr, c)
        draw(11)
        got_cols.append(px(160, 120))
    r.eq(got_cols, [0xF11E, 0xFE1E, 0xFFFE, 0x111E], "grid 11 focus peaking: edge detected across all 4 peaking colors")
    r.check(px(0, 0) == 0 and px(10, 423) == 0 and px(300, 120) == 0, "grid 11 focus peaking: 2px border clear & flat background = 0")
    m.w8(peakcol_addr, 0)
    # Runtime peaking sensitivity: Med(44), High(24), Low(72) & Auto MF-magnify boost
    soft_y = bytearray([80] * (640 * 424))
    for y in range(424):
        row = y * 640
        soft_y[row + 160 : row + 200] = bytes([110] * 40)
        soft_y[row + 320 : row + 360] = bytes([136] * 40)
    m.wr(yv, bytes(soft_y))
    m.w8(0xA0939A53, 0)
    m.w8(peaksens_addr, 0)
    draw(11)
    med_ok = px(160, 120) == 0 and px(320, 120) == 0xF11E
    m.w8(peaksens_addr, 1)
    draw(11)
    high_ok = px(160, 120) == 0xF11E and px(320, 120) == 0xF11E
    m.w8(peaksens_addr, 2)
    draw(11)
    low_ok = px(160, 120) == 0 and px(320, 120) == 0
    m.w8(peaksens_addr, 3)
    m.w8(0xA0939A53, 0)
    draw(11)
    auto_1x = px(160, 120) == 0 and px(320, 120) == 0xF11E
    m.w8(0xA0939A53, 1)
    draw(11)
    auto_mf = px(160, 120) == 0xF11E and px(320, 120) == 0xF11E
    m.w8(0xA0939A53, 0)
    m.w8(peaksens_addr, 0)
    r.check(med_ok and high_ok and low_ok and auto_1x and auto_mf,
            "grid 11 peaking sensitivity: Med(44)/High(24)/Low(72) & Auto MF-magnify boost")
    m.wr(yv, bytes(y_plane))

    draw(12)
    r.check(px(160, 120) == 0xF11E and px(319, 211) == 0x1F3E, "grid 12 peaking + level: combines edge overlay and level")

    # Mode 13: Zebras (threshold & phase shift)
    m.w32(lp + 12, 0)
    draw(13)
    z0 = px(48, 48)
    m.w32(lp + 12, 2)
    draw(13)
    z2 = px(48, 48)
    r.check(z0 in (0xF40E, 0x111A) and z2 in (0xF40E, 0x111A) and z0 != z2 and px(300, 200) == 0,
            "grid 13 zebras: highlights >= ZEBRA_THRESH striped, phase shifts on fcnt, midtones = 0")

    # Runtime zebra thresholds & Dual Highlight+Shadow clipping
    z_y = bytearray([80] * (640 * 424))
    for y in range(40, 80):
        row = y * 640
        z_y[row + 40 : row + 80] = bytes([235] * 40)
        z_y[row + 120 : row + 160] = bytes([246] * 40)
        z_y[row + 200 : row + 240] = bytes([253] * 40)
        z_y[row + 280 : row + 320] = bytes([6] * 40)
    m.wr(yv, bytes(z_y))
    m.w32(lp + 12, 0)
    m.w8(zebralvl_addr, 1)
    draw(13)
    z90_ok = px(48, 48) != 0 and px(288, 48) == 0
    m.w8(zebralvl_addr, 2)
    draw(13)
    z100_ok = px(128, 48) == 0 and px(208, 48) != 0
    m.w8(zebralvl_addr, 3)
    draw(13)
    zh, zl0, zl1 = px(208, 48), px(288, 48), px(292, 48)
    zdual_ok = zh in (0xF40E, 0x111A) and {zl0, zl1} == {0x1DFE, 0x111A} and px(400, 200) == 0
    m.w8(zebralvl_addr, 0)
    r.check(z90_ok and z100_ok and zdual_ok, "grid 13 zebra levels: 90% Safe, 100% Clip & Dual Hi(orange)+Lo(cyan)")

    # Mode 17: Foveon False Color (IRE) 6 zones + vertical IRE scale bar
    fc_y = bytearray([80] * (640 * 424))
    for y in range(40, 80):
        row = y * 640
        fc_y[row + 20 : row + 60] = bytes([6] * 40)
        fc_y[row + 80 : row + 120] = bytes([20] * 40)
        fc_y[row + 140 : row + 180] = bytes([116] * 40)
        fc_y[row + 200 : row + 240] = bytes([156] * 40)
        fc_y[row + 260 : row + 300] = bytes([232] * 40)
        fc_y[row + 320 : row + 360] = bytes([250] * 40)
    m.wr(yv, bytes(fc_y))
    draw(17)
    fc_zones = [px(30, 50), px(90, 50), px(150, 50), px(210, 50), px(270, 50), px(330, 50), px(400, 200), px(626, 124), px(626, 284)]
    r.eq(fc_zones, [0x61FD, 0x17FB, 0x1E3C, 0xF7BC, 0xFE1C, 0xF11E, 0x0000, 0xF11F, 0x61FF],
         "grid 17 false color: 6 IRE zones, transparent midtones & right-edge legend bar")
    m.wr(yv, bytes(y_plane))

    # Mode 14: RGB Histogram + clip indicator
    draw(14)
    hist_box = [px(x, y) for y in range(360, 408) for x in range(496, 624)]
    r.check(px(492, 354) == 0xBBBC and any((p & 0xF) == 14 and p >> 4 != 0 for p in hist_box), "grid 14 RGB histogram: frame & bars drawn")
    m.wr(yv, bytes([255]) * (640 * 424))
    draw(14)
    r.eq(px(625, 360), 0xF11F, "grid 14 RGB histogram: highlight clip bar")

    # Mode transition cleanup: Hist/Peaking -> Level -> Stock Off (0)
    draw(7)
    r.check(px(48, 48) == 0 and px(160, 120) == 0 and px(500, 360) == 0 and px(319, 211) == 0x1F3E,
            "HUD transition 14 -> 7 clears outer overlay plane before drawing level")
    draw(0)
    r.check(not any(m.rd(stock.ARGB_FB, 640 * 424 * 2)), "HUD transition 7 -> 0 clears entire 640x424 RGBA4444 plane")


def run(b: Build, r: Report) -> None:
    check_invariants(b, r)
    check_hooks(b, r)
    check_menus(b, r)
    check_strings(b, r)
    addrs = check_settings(b, r)
    check_bracketing(b, r)
    check_min_shutter(b, r, addrs["minshutter"])
    check_ipc(b, r, addrs)
    check_makernote(b, r)
    check_lv(b, r)
    check_ycap(b, r)
    check_hud(b, r, addrs["peakcol"], addrs["peaksens"], addrs["zebralvl"])
    check_package(b, r, *check_updater(b, r))
